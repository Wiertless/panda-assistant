"""Convert Hugging Face Gemma 4 E2B weights (model.safetensors) into FastFlowLM's
model.q4nx (text model only). numpy only.

    python3 convert.py SRC_DIR_OR_SAFETENSORS OUT.q4nx [--quant qkx|minmax]

Layouts are reverse-engineered from FastFlowLM/Gemma4-E2B-IT-NPU2 and verified
by round trip (see NOTES.md). The *values* of FLM's Q4 scales/mins come from a
calibrated (activation-aware) quantizer we cannot reproduce, so Q4 tensors are
not byte-identical to FLM's file; everything else is.
"""
import json, os, struct, sys
import numpy as np

# ---------- I/O ----------
DT = {"BF16": np.uint16, "F32": np.float32, "I8": np.int8, "U8": np.uint8, "F16": np.float16}

class ST:
    """Lazy safetensors reader (also reads .q4nx, same container)."""
    def __init__(self, path):
        self.path = path
        with open(path, "rb") as f:
            n = struct.unpack("<Q", f.read(8))[0]
            self.h = json.loads(f.read(n))
        self.h.pop("__metadata__", None)
        self.base = 8 + n
    def shape(self, name): return self.h[name]["shape"]
    def __contains__(self, name): return name in self.h
    def rows(self, name, r0=0, r1=None):
        """Rows r0..r1 along the first axis, as the raw numpy dtype."""
        v = self.h[name]; dt = np.dtype(DT[v["dtype"]]); sh = v["shape"]
        rowsz = int(np.prod(sh[1:], dtype=np.int64)) if len(sh) > 1 else 1
        r1 = sh[0] if r1 is None else r1
        a = v["data_offsets"][0] + r0 * rowsz * dt.itemsize
        with open(self.path, "rb") as f:
            f.seek(self.base + a)
            buf = f.read((r1 - r0) * rowsz * dt.itemsize)
        return np.frombuffer(buf, dt).reshape([r1 - r0] + list(sh[1:]))
    def get(self, name): return self.rows(name)

def bf2f(u): return (np.asarray(u, np.uint16).astype(np.uint32) << 16).view(np.float32)
def f2bf(x):  # float32 -> bf16 bits, round to nearest even
    x = np.ascontiguousarray(x, np.float32).view(np.uint32)
    return ((x + 0x7FFF + ((x >> 16) & 1)) >> 16).astype(np.uint16)

# ---------- Q4_1 (linear weights, lm_head) ----------
# FLM tensor I8 [N/32 * K/256, 5120]: one row per tile of 32 output rows x 256
# input columns, tiles ordered row-tile major (tile = rt*(K/256) + kt).
# Row bytes: [0:512) 256 BF16 scales, [512:1024) 256 BF16 mins, [1024:5120) nibbles.
# Group g = j*32 + r (j = 32-column group 0..7, r = row 0..31); w = scale*q + min.
# Nibble n (low nibble of byte n//2 first) holds r = (n>>12)*16 + (n&15),
# j = (n>>9)&7, i = (n>>4)&31 (column inside the group).
_n = np.arange(8192)
NIB_SRC = (((_n >> 12) * 16 + (_n & 15)) * 256 + ((_n >> 9) & 7) * 32 + ((_n >> 4) & 31))

def quant_q4_1(X, mode="qkx"):
    """X [G,32] float32 -> (scale bf16 bits, min bf16 bits, q uint8 [G,32])."""
    mi = np.minimum(X.min(1), 0); ma = X.max(1)
    rng = ma - mi
    if mode == "minmax":
        s, m = rng / 15, mi
    else:  # ggml make_qkx2_quants(nmax=15, weights=1, rmin=-1, rdelta=0.1, nstep=20)
        with np.errstate(all="ignore"):
            n = X.shape[1]; sx = X.sum(1)
            s = rng / 15; m = mi.copy()
            L = np.clip(np.rint(np.where(rng > 0, 15 / rng, 0)[:, None] * (X - mi[:, None])), 0, 15)
            best = ((s[:, None] * L + m[:, None] - X) ** 2).sum(1)
            for st in range(21):
                isc = np.where(rng > 0, (-1 + 0.1 * st + 15) / rng, 0)
                La = np.clip(np.rint(isc[:, None] * (X - mi[:, None])), 0, 15)
                sl = La.sum(1); sl2 = (La * La).sum(1); sxl = (La * X).sum(1)
                D = n * sl2 - sl * sl
                ts = (n * sxl - sx * sl) / D; tm = (sl2 * sx - sl * sxl) / D
                pos = tm > 0
                ts = np.where(pos, sxl / sl2, ts); tm = np.where(pos, 0, tm)
                err = ((ts[:, None] * La + tm[:, None] - X) ** 2).sum(1)
                ok = (D > 0) & (err < best)
                best = np.where(ok, err, best); s = np.where(ok, ts, s); m = np.where(ok, tm, m)
    sb, mb = f2bf(s), f2bf(m)
    sf, mf = bf2f(sb), bf2f(mb)
    with np.errstate(all="ignore"):
        q = np.where(sf[:, None] > 0, np.rint((X - mf[:, None]) / sf[:, None]), 0)
    return sb, mb, np.clip(q, 0, 15).astype(np.uint8)

def pack_q4_1_tiles(sb, mb, q):
    """sb, mb [T,32 rows,8 groups]; q [T,32,256] -> uint8 [T,5120]."""
    T = q.shape[0]
    out = np.empty((T, 5120), np.uint8)
    out[:, :512] = np.ascontiguousarray(sb.transpose(0, 2, 1)).reshape(T, 256).view(np.uint8).reshape(T, 512)
    out[:, 512:1024] = np.ascontiguousarray(mb.transpose(0, 2, 1)).reshape(T, 256).view(np.uint8).reshape(T, 512)
    nib = q.reshape(T, 8192)[:, NIB_SRC]
    out[:, 1024:] = nib[:, 0::2] | (nib[:, 1::2] << 4)
    return out

def unpack_q4_1_tiles(b):
    """inverse of pack: uint8 [T,5120] -> sb, mb [T,32,8], q [T,32,256]."""
    T = b.shape[0]
    sb = np.ascontiguousarray(b[:, :512]).view(np.uint16).reshape(T, 8, 32).transpose(0, 2, 1)
    mb = np.ascontiguousarray(b[:, 512:1024]).view(np.uint16).reshape(T, 8, 32).transpose(0, 2, 1)
    nb = b[:, 1024:]
    nib = np.stack([nb & 15, nb >> 4], -1).reshape(T, 8192)
    q = np.empty((T, 8192), np.uint8); q[:, NIB_SRC] = nib
    return sb, mb, q.reshape(T, 32, 256)

def q4_tiles_from_rows(W, mode):
    """W float32 [32*R, K] -> packed uint8 [R*K/256, 5120]."""
    N, K = W.shape; nk = K // 256
    T = W.reshape(N // 32, 32, nk, 256).transpose(0, 2, 1, 3).reshape(-1, 32, 256)
    sb, mb, q = quant_q4_1(np.ascontiguousarray(T).reshape(-1, 32), mode)
    t = T.shape[0]
    return pack_q4_1_tiles(sb.reshape(t, 32, 8), mb.reshape(t, 32, 8), q.reshape(t, 32, 256))

def dequant_q4_tiles(b, N, K):
    """packed uint8 [N/32*K/256, 5120] -> float32 [N, K]."""
    sb, mb, q = unpack_q4_1_tiles(b)
    D = q.reshape(-1, 32, 8, 32) * bf2f(sb)[..., None] + bf2f(mb)[..., None]
    nk = K // 256
    return D.reshape(N // 32, nk, 32, 256).transpose(0, 2, 1, 3).reshape(N, K)

# ---------- Q8 per 32 (embeddings) ----------
def q8_rows(X):
    """X float32 [R, C] -> int8 [R, C], F32 scale [R, C/32]; scale = amax/127."""
    R, C = X.shape; G = X.reshape(R, C // 32, 32)
    s = (np.abs(G).max(-1) / np.float32(127)).astype(np.float32)
    with np.errstate(all="ignore"):
        q = np.where(s[..., None] > 0, np.rint(G / s[..., None]), 0)
    return np.clip(q, -127, 127).astype(np.int8).reshape(R, C), s

# ---------- BF16 re-tilings ----------
def tile_decode(W):   # [N, K] -> [N/32, K, 32]
    N, K = W.shape; return np.ascontiguousarray(W.reshape(N // 32, 32, K).transpose(0, 2, 1))
def tile_prefill(W):  # [N, K] -> [N/64, K/256, 64*256]
    N, K = W.shape
    return np.ascontiguousarray(W.reshape(N // 64, 64, K // 256, 256).transpose(0, 2, 1, 3)).reshape(N // 64, K // 256, 16384)

# ---------- the model ----------
P = "model.language_model."

def plan(src, cfg):
    """List of (flm_name, dtype, shape, producer) in FLM's file order."""
    tc = cfg.get("text_config", cfg)
    L = tc["num_hidden_layers"]; H = tc["hidden_size"]; Dp = tc["hidden_size_per_layer_input"]
    V = tc["vocab_size"]; Vp = tc.get("vocab_size_per_layer_input", V)
    mode = MODE
    items = []
    def bf16_copy(sname):
        return lambda: src.get(sname)
    def q4(sname):
        def prod():
            N, K = src.shape(sname)
            for r0 in range(0, N, 1024):
                W = bf2f(src.rows(sname, r0, min(N, r0 + 1024)))
                yield q4_tiles_from_rows(W, mode)
        return prod
    def q8(sname, factor):
        def prod_q():
            N = src.shape(sname)[0]
            for r0 in range(0, N, 4096):
                X = bf2f(src.rows(sname, r0, min(N, r0 + 4096))) * np.float32(factor)
                yield q8_rows(X)[0]
        def prod_s():
            N = src.shape(sname)[0]
            for r0 in range(0, N, 4096):
                X = bf2f(src.rows(sname, r0, min(N, r0 + 4096))) * np.float32(factor)
                yield q8_rows(X)[1]
        return prod_q, prod_s
    emb = P + "embed_tokens.weight"; pemb = P + "embed_tokens_per_layer.weight"
    eq, es = q8(emb, np.sqrt(np.float32(H)))
    pq, ps = q8(pemb, np.sqrt(np.float32(Dp)))
    items.append(("model.embed_tokens.weight.scale", "F32", [V, H // 32], es))
    items.append(("model.per_layer_token_embd.weight.scale", "F32", [Vp, L * Dp // 32], ps))
    for i in range(L):
        s = f"{P}layers.{i}."; d = f"model.layers.{i}."
        g = s + "per_layer_input_gate.weight"; pp = s + "per_layer_projection.weight"
        gs, ps_ = src.shape(g), src.shape(pp)
        items += [
            (d + "inp_gate.weight", "BF16", [gs[0] // 32, gs[1], 32], lambda g=g: tile_decode(src.get(g))),
            (d + "inp_gate.weight_prefill", "BF16", [gs[0] // 64, gs[1] // 256, 16384], lambda g=g: tile_prefill(src.get(g))),
            (d + "input_layernorm.weight", "BF16", [H], bf16_copy(s + "input_layernorm.weight")),
            (d + "layer_output_scale.weight", "BF16", [1], bf16_copy(s + "layer_scalar")),
            (d + "per_layer_projection.weight", "BF16", [ps_[0] // 32, ps_[1], 32], lambda pp=pp: tile_decode(src.get(pp))),
            (d + "per_layer_projection.weight_prefill", "BF16", [ps_[0] // 64, ps_[1] // 256, 16384], lambda pp=pp: tile_prefill(src.get(pp))),
            (d + "post_attention_layernorm.weight", "BF16", [H], bf16_copy(s + "post_attention_layernorm.weight")),
            (d + "post_feedforward_layernorm.weight", "BF16", [H], bf16_copy(s + "post_feedforward_layernorm.weight")),
            (d + "post_layernorm.weight", "BF16", [H], bf16_copy(s + "post_per_layer_input_norm.weight")),
            (d + "pre_feedforward_layernorm.weight", "BF16", [H], bf16_copy(s + "pre_feedforward_layernorm.weight")),
            (d + "self_attn.k_norm.weight", "BF16", src.shape(s + "self_attn.k_norm.weight"), bf16_copy(s + "self_attn.k_norm.weight")),
            (d + "self_attn.q_norm.weight", "BF16", src.shape(s + "self_attn.q_norm.weight"), bf16_copy(s + "self_attn.q_norm.weight")),
        ]
        for m in ("mlp.down_proj", "mlp.gate_proj", "mlp.up_proj", "self_attn.k_proj",
                  "self_attn.o_proj", "self_attn.q_proj", "self_attn.v_proj"):
            sn = s + m + ".weight"; N, K = src.shape(sn)
            items.append((d + m + ".weight", "I8", [N // 32 * (K // 256), 5120], q4(sn)))
    items.append(("model.norm.weight", "BF16", [H], bf16_copy(P + "norm.weight")))
    pm = P + "per_layer_model_projection.weight"; pmN, pmK = src.shape(pm)
    for i in range(L):
        items.append((f"model.per_layer_model_proj.weight_layer{i}", "BF16", [Dp // 32, pmK, 32],
                      lambda i=i: tile_decode(src.rows(pm, i * Dp, (i + 1) * Dp))))
    items.append(("model.per_layer_model_proj.weight_prefill", "BF16", [pmN // 64, pmK // 256, 16384], lambda: tile_prefill(src.get(pm))))
    items.append(("model.per_layer_proj_norm.weight", "BF16", [Dp], bf16_copy(P + "per_layer_projection_norm.weight")))
    # rope_freqs: global (full_attention) layers, proportional RoPE: rotary dims get 1,
    # the rest 1e30 (no rotation). Length = global_head_dim/2.
    gd = tc.get("global_head_dim", tc["head_dim"])
    prf = tc["rope_parameters"]["full_attention"].get("partial_rotary_factor", 1.0)
    nrot = int(gd * prf) // 2
    rope = np.full(gd // 2, 1e30, np.float32); rope[:nrot] = 1.0
    items.append(("rope_freqs.weight", "BF16", [gd // 2], lambda: f2bf(rope)))
    # I8 section
    items.append(("lm_head.weight", "I8", [V // 32 * (H // 256), 5120], q4(emb)))
    items.append(("model.embed_tokens.weight", "I8", [V, H], eq))
    # (layer tensors I8 are already listed above; sort below puts them in place)
    items.append(("model.per_layer_token_embd.weight", "I8", [Vp, L * Dp], pq))
    order = {"F32": 0, "BF16": 1, "I8": 2}
    items.sort(key=lambda x: (order[x[1]], x[0]))
    return items

MODE = "qkx"

def build_header(items):
    h = {}; off = 0
    for name, dt, shape, _ in items:
        n = int(np.prod(shape, dtype=np.int64)) * np.dtype(DT[dt]).itemsize
        h[name] = {"dtype": dt, "shape": shape, "data_offsets": [off, off + n]}; off += n
    hb = json.dumps(h, separators=(",", ":")).encode()
    hb += b" " * ((-(8 + len(hb))) % 8)
    return struct.pack("<Q", len(hb)) + hb, h

def produce(prod):
    r = prod()
    if isinstance(r, np.ndarray): yield r
    else: yield from r

def convert(src_path, out_path, only=None, log=print):
    if os.path.isdir(src_path):
        cfg = json.load(open(os.path.join(src_path, "config.json")))
        src_path = os.path.join(src_path, "model.safetensors")
    else:
        cfg = json.load(open(os.path.join(os.path.dirname(src_path), "config.json")))
    src = ST(src_path)
    items = plan(src, cfg)
    hdr, h = build_header(items)
    with open(out_path, "wb") as f:
        f.write(hdr)
        for name, dt, shape, prod in items:
            n = 0
            for chunk in produce(prod):
                b = np.ascontiguousarray(chunk).tobytes(); f.write(b); n += len(b)
            a, e = h[name]["data_offsets"]
            assert n == e - a, (name, n, e - a)
            log(f"wrote {name} {dt} {shape}")
    return out_path

if __name__ == "__main__":
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if "--quant" in sys.argv:
        MODE = sys.argv[sys.argv.index("--quant") + 1]; args.remove(MODE)
    if len(args) != 2:
        sys.exit(__doc__)
    convert(args[0], args[1])
