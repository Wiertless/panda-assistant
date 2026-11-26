"""Convert Hugging Face Qwen3.5-9B weights (e.g. an abliterated fine-tune) into
FastFlowLM's model.q4nx for FLM 1.0.13's Qwen3.5-9B-NPU2 kernels.

Layout, reverse-engineered from FLM's own Qwen3.5-9B-NPU2 model.q4nx (2026-11-26):
- Container: FLM's header copied verbatim (same names, shapes, dtypes, offsets).
- Tiles: a matrix W [N, K] becomes [N/32, K/256, row] tiles of 32 rows x 256 cols.
  Inside a tile, value n (n = col*32 + row) and per-group entry p (p = group*32 +
  row, group = 32 columns) are column-major.
- Q4 (4736 B/tile, Q4_K with 8-bit sub-scales): [0:256) sc u8, [256:512) mn u8,
  [512:4608) 4-bit q (low nibble first), [4608:4672) d bf16 x32 rows,
  [4672:4736) dmin bf16 x32 (negative). w = d*sc*q + dmin*mn.
- Q8 (8704 B/tile): [0:512) scale bf16 per group, [512:8704) int8. w = s*q.
- q_proj (full-attention layers): HF's per-head [query, gate] rows reordered to
  [all queries, all gates].
- BF16: norms rne(1+w) (ssm_norm as is), conv1d [8192,1,4] -> [4,8192],
  alpha/beta also kept as plain bf16; F32: ssm_a = -exp(A_log), ssm_dt.bias.

Memory: one chunk of rows at a time, under 100 MB of its own (file cache aside).
Usage: convert_qwen.py HF_DIR FLM_MODEL OUT_FILE
  FLM_MODEL: FLM's own Qwen3.5-9B-NPU2 model.q4nx, or a copy of its first 8+N
  bytes (the header). Only the header is read: names, shapes and offsets.
See QWEN.md.
"""
import json, struct, sys, time
import numpy as np


def bf2f(u): return (np.asarray(u, np.uint16).astype(np.uint32) << 16).view(np.float32)
def f2bf(x):  # float32 -> bf16 bits, round to nearest even
    x = np.ascontiguousarray(x, np.float32).view(np.uint32)
    return ((x + 0x7FFF + ((x >> 16) & 1)) >> 16).astype(np.uint16)

TILE_Q4, TILE_Q8 = 4736, 8704
CHUNK = 16  # row tiles (512 rows) per step


class ST:
    def __init__(self, path):
        self.f = open(path, 'rb'); n = struct.unpack('<Q', self.f.read(8))[0]
        self.h = json.loads(self.f.read(n)); self.h.pop('__metadata__', None); self.base = 8 + n
    def rows_bits(self, name, r0, r1):
        t = self.h[name]; s, _ = t['data_offsets']; shape = t['shape']
        it = {'BF16': 2, 'F32': 4}[t['dtype']]; row = int(np.prod(shape[1:])) * it
        self.f.seek(self.base + s + r0 * row)
        a = np.frombuffer(self.f.read((r1 - r0) * row), {'BF16': np.uint16, 'F32': np.float32}[t['dtype']])
        return a.reshape([r1 - r0] + shape[1:]), t['dtype']
    def f32(self, name, r0=0, r1=None):
        r1 = self.h[name]['shape'][0] if r1 is None else r1
        a, dt = self.rows_bits(name, r0, r1)
        return bf2f(a) if dt == 'BF16' else a.astype(np.float32)
    def bits(self, name):
        a, dt = self.rows_bits(name, 0, self.h[name]['shape'][0]); assert dt == 'BF16', name; return a


class HF:
    def __init__(self, d):
        self.d = d; self.idx = json.load(open(d + '/model.safetensors.index.json'))['weight_map']; self.open = {}
    def __call__(self, name):
        full = name if name.startswith('lm_head') else 'model.language_model.' + name
        fn = self.idx[full]
        if fn not in self.open: self.open[fn] = ST(self.d + '/' + fn)
        return self.open[fn], full


def qkx_sm(X):
    """X [G,32] -> per-group scale s and min m (m <= 0), ggml make_qkx2 search (nmax 15)."""
    mi = np.minimum(X.min(1), 0); ma = X.max(1); rng = ma - mi
    with np.errstate(all='ignore'):
        n = X.shape[1]; sx = X.sum(1); s = rng / 15; m = mi.copy()
        L = np.clip(np.rint(np.where(rng > 0, 15 / rng, 0)[:, None] * (X - mi[:, None])), 0, 15)
        best = ((s[:, None] * L + m[:, None] - X) ** 2).sum(1)
        for st in range(21):
            isc = np.where(rng > 0, (-1 + 0.1 * st + 15) / rng, 0)
            La = np.clip(np.rint(isc[:, None] * (X - mi[:, None])), 0, 15)
            sl = La.sum(1); sl2 = (La * La).sum(1); sxl = (La * X).sum(1)
            D = n * sl2 - sl * sl
            ts = (n * sxl - sx * sl) / D; tm = (sl2 * sx - sl * sxl) / D
            pos = tm > 0; ts = np.where(pos, sxl / sl2, ts); tm = np.where(pos, 0, tm)
            err = ((ts[:, None] * La + tm[:, None] - X) ** 2).sum(1)
            ok = (D > 0) & (err < best)
            best = np.where(ok, err, best); s = np.where(ok, ts, s); m = np.where(ok, tm, m)
    return np.nan_to_num(s).astype(np.float32), np.nan_to_num(m).astype(np.float32)


def tiles(W):
    """W [32R, K] -> [R*nk, 32, 256] in FLM tile order (row tile major)."""
    N, K = W.shape
    return np.ascontiguousarray(W.reshape(N // 32, 32, K // 256, 256).transpose(0, 2, 1, 3)).reshape(-1, 32, 256)


def q4_tiles(W):
    T = tiles(W); t = T.shape[0]
    s, m = qkx_sm(T.reshape(-1, 32)); s = s.reshape(t, 32, 8); m = m.reshape(t, 32, 8)
    db = f2bf(s.max(2) / 255); df = bf2f(db)
    dmb = f2bf(np.minimum(m.min(2), 0) / 255); dmf = bf2f(dmb)
    with np.errstate(all='ignore'):
        sc = np.clip(np.rint(np.where(df[..., None] > 0, s / df[..., None], 0)), 0, 255)
        mn = np.clip(np.rint(np.where(dmf[..., None] < 0, m / dmf[..., None], 0)), 0, 255)
        S = df[..., None] * sc; M = dmf[..., None] * mn
        q = np.where(S[..., None] > 0, np.rint((T.reshape(t, 32, 8, 32) - M[..., None]) / S[..., None]), 0)
    q = np.clip(q, 0, 15).astype(np.uint8).reshape(t, 32, 256)
    out = np.empty((t, TILE_Q4), np.uint8)
    out[:, :256] = sc.astype(np.uint8).transpose(0, 2, 1).reshape(t, 256)
    out[:, 256:512] = mn.astype(np.uint8).transpose(0, 2, 1).reshape(t, 256)
    nib = q.transpose(0, 2, 1).reshape(t, 8192)
    out[:, 512:4608] = nib[:, 0::2] | (nib[:, 1::2] << 4)
    out[:, 4608:4672] = db.view(np.uint8).reshape(t, 64)
    out[:, 4672:4736] = dmb.view(np.uint8).reshape(t, 64)
    return out


def q8_tiles(W):
    T = tiles(W); t = T.shape[0]; G = T.reshape(t, 32, 8, 32)
    sb = f2bf(np.abs(G).max(-1) / np.float32(127)); sf = bf2f(sb)
    with np.errstate(all='ignore'):
        q = np.where(sf[..., None] > 0, np.rint(G / sf[..., None]), 0)
    q = np.clip(q, -127, 127).astype(np.int8).reshape(t, 32, 256)
    out = np.empty((t, TILE_Q8), np.uint8)
    out[:, :512] = np.ascontiguousarray(sb.transpose(0, 2, 1)).reshape(t, 256).view(np.uint8).reshape(t, 512)
    out[:, 512:] = q.transpose(0, 2, 1).reshape(t, 8192).view(np.uint8)
    return out


def source(flm_name):
    """FLM tensor name -> (kind, HF name)."""
    n = flm_name.replace('model.', '', 1) if flm_name.startswith('model.') else flm_name
    if flm_name == 'lm_head.weight': return 'q8', 'lm_head.weight'
    if n == 'embed_tokens.weight': return 'bits', n
    if n == 'norm.weight': return 'norm1p', n
    L = n.split('.')[1]; rest = '.'.join(n.split('.')[2:]); p = f'layers.{L}.'
    table = {
        'input_layernorm.weight': ('norm1p', 'input_layernorm.weight'),
        'post_attention_layernorm.weight': ('norm1p', 'post_attention_layernorm.weight'),
        'self_attn.q_norm.weight': ('norm1p', 'self_attn.q_norm.weight'),
        'self_attn.k_norm.weight': ('norm1p', 'self_attn.k_norm.weight'),
        'linear_attn.ssm_norm.weight': ('norm', 'linear_attn.norm.weight'),
        'linear_attn.ssm_conv1d.weight': ('conv', 'linear_attn.conv1d.weight'),
        'linear_attn.ssm_alpha_proj.bf16.weight': ('bits', 'linear_attn.in_proj_a.weight'),
        'linear_attn.ssm_beta_proj.bf16.weight': ('bits', 'linear_attn.in_proj_b.weight'),
        'linear_attn.ssm_alpha_proj.weight': ('q8', 'linear_attn.in_proj_a.weight'),
        'linear_attn.ssm_beta_proj.weight': ('q8', 'linear_attn.in_proj_b.weight'),
        'linear_attn.ssm_a': ('ssm_a', 'linear_attn.A_log'),
        'linear_attn.ssm_dt.bias': ('f32', 'linear_attn.dt_bias'),
        'linear_attn.qkv_proj.weight': ('q4', 'linear_attn.in_proj_qkv.weight'),
        'linear_attn.ssm_out_proj.weight': ('q4', 'linear_attn.out_proj.weight'),
        # FLM files the linear layers' z projection under self_attn.gate_proj.
        'self_attn.gate_proj.weight': ('q4', 'linear_attn.in_proj_z.weight'),
    }
    if rest in table: k, h = table[rest]; return k, p + h
    if rest.startswith(('self_attn.', 'mlp.')): return 'q4', p + rest
    raise KeyError(flm_name)


HEAD = 256  # head_dim


def q_rows(N):
    """FLM row i of q_proj comes from HF row q_rows(N)[i]."""
    heads = N // (2 * HEAD); i = np.arange(N); h, w = (i % (N // 2)) // HEAD, i % HEAD
    return h * 2 * HEAD + (i >= N // 2) * HEAD + w


def produce(hf, name, info, out):
    """Write one tensor's bytes to out; returns bytes written."""
    kind, hname = source(name); st, full = hf(hname); written = 0
    def put(a): nonlocal written; b = np.ascontiguousarray(a).view(np.uint8); out.write(b.tobytes()); written += b.nbytes
    if kind in ('q4', 'q8'):
        N = st.h[full]['shape'][0]
        # HF interleaves each head's query and output gate ([q0, g0, q1, g1, ...], 256 rows
        # each); FLM wants all queries, then all gates.
        order = q_rows(N) if name.endswith('self_attn.q_proj.weight') else None
        for r0 in range(0, N, CHUNK * 32):
            r1 = min(N, r0 + CHUNK * 32)
            if order is None:
                W = st.f32(full, r0, r1)
            else:
                W = np.concatenate([st.f32(full, s, s + HEAD) for s in order[r0:r1:HEAD]])
            put(q4_tiles(W) if kind == 'q4' else q8_tiles(W))
    elif kind == 'bits':
        N = st.h[full]['shape'][0]
        for r0 in range(0, N, 8192):
            put(st.rows_bits(full, r0, min(N, r0 + 8192))[0])
    elif kind == 'norm1p': put(f2bf(1 + st.f32(full)))
    elif kind == 'norm': put(f2bf(st.f32(full)))
    elif kind == 'conv': put(np.ascontiguousarray(st.bits(full)[:, 0, :].T))
    elif kind == 'ssm_a': put((-np.exp(st.f32(full).astype(np.float64))).astype(np.float32))
    elif kind == 'f32': put(st.f32(full).astype(np.float32))
    s, e = info['data_offsets']
    assert written == e - s, f'{name}: wrote {written}, header says {e - s}'


def main(hf_dir, header_bin, out_path):
    with open(header_bin, 'rb') as f:
        n = struct.unpack('<Q', f.read(8))[0]; f.seek(0); raw = f.read(8 + n)
    h = json.loads(raw[8:8 + n])
    items = sorted(((k, v) for k, v in h.items() if k != '__metadata__'), key=lambda kv: kv[1]['data_offsets'][0])
    hf = HF(hf_dir); t0 = time.time()
    with open(out_path, 'wb') as out:
        out.write(raw[:8 + n])
        for i, (name, info) in enumerate(items):
            produce(hf, name, info, out)
            if i % 25 == 0 or info['dtype'] == 'I8' and 'lm_head' in name:
                print(f'{i + 1}/{len(items)} {name}  {time.time() - t0:.0f} s', flush=True)
    print(f'done in {time.time() - t0:.0f} s', flush=True)


if __name__ == '__main__':
    main(*sys.argv[1:4])
