"""Convert, then compare our model.q4nx with FastFlowLM's, tensor by tensor.

    python3 verify.py SRC_DIR FLM_model.q4nx OURS.q4nx [--skip-convert] [--only REGEX] [--quant qkx|minmax]

Writes OURS.q4nx with convert.py (unless --skip-convert), then for every
tensor prints EXACT or how it differs. For Q4 tensors it also re-packs FLM's own
(scale, min, nibbles) with our packer (layout round trip) and gives the
reconstruction RMSE against the source weights for FLM's file and for ours.
Large tensors are compared in row chunks (fits in ~4 GB RAM).
"""
import json, os, re, sys
import numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import convert as C

P = "model.language_model."

def src_name(name):
    return P + "embed_tokens.weight" if name == "lm_head.weight" else P + name[len("model."):]

def main():
    a = sys.argv[1:]
    opt = lambda k: (a[a.index(k) + 1] if k in a else None)
    only, quant = opt("--only"), opt("--quant")
    pos = [x for i, x in enumerate(a) if not x.startswith("--") and not (i and a[i - 1] in ("--only", "--quant"))]
    src_dir, flm_path, out = pos[:3]
    if quant: C.MODE = quant
    if "--skip-convert" not in a:
        C.convert(src_dir, out, log=lambda s: None)
    cfg = json.load(open(os.path.join(src_dir, "config.json")))
    src = C.ST(os.path.join(src_dir, "model.safetensors"))
    flm, ours = C.ST(flm_path), C.ST(out)
    hdr, _ = C.build_header(C.plan(src, cfg))
    with open(flm_path, "rb") as f: fh = f.read(len(hdr))
    with open(out, "rb") as f: oh = f.read(len(hdr))
    print("header:", "EXACT" if fh == hdr == oh else "DIFFERENT", f"({len(hdr)} bytes);",
          "file size", "same" if os.path.getsize(out) == os.path.getsize(flm_path) else "DIFFERENT")
    print("tensor set:", "same" if set(ours.h) == set(flm.h) else "DIFFERENT")
    n_exact = n = 0
    for name in ours.h:
        if only and not re.search(only, name): continue
        n += 1
        dt, shape = flm.h[name]["dtype"], flm.h[name]["shape"]
        R = shape[0]
        if dt != "I8":
            m, t = ours.get(name), flm.get(name)
            if np.array_equal(m, t): n_exact += 1; print(f"EXACT  {name}"); continue
            if dt == "BF16":
                d = np.abs(C.bf2f(m) - C.bf2f(t))
                print(f"DIFF   {name}: max abs err {d.max():.3g}, {np.mean(d > 0):.4%} differ")
            else:
                r = np.abs(m / np.where(t == 0, 1, t) - 1)
                print(f"DIFF   {name}: {np.mean(m == t):.2%} equal, rel diff median {np.median(r):.3g} max {r.max():.3g}")
            continue
        if name in ("model.embed_tokens.weight", "model.per_layer_token_embd.weight"):
            eq = steq = 0; emax = smax = 0.0; esum = 0.0
            for r0 in range(0, R, 8192):
                r1 = min(R, r0 + 8192)
                m, t = ours.rows(name, r0, r1), flm.rows(name, r0, r1)
                sm, st = ours.rows(name + ".scale", r0, r1), flm.rows(name + ".scale", r0, r1)
                eq += int((m == t).sum())
                k = t.shape[1]
                dm = m.astype(np.float32).reshape(r1 - r0, -1, 32) * sm[..., None]
                dtt = t.astype(np.float32).reshape(r1 - r0, -1, 32) * st[..., None]
                e = np.abs(dm - dtt); emax = max(emax, float(e.max())); esum += float(e.sum())
                smax = max(smax, float((e / np.where(st == 0, 1, st)[..., None]).max()))
            tot = R * shape[1]
            same = eq == tot and np.array_equal(ours.get(name + ".scale"), flm.get(name + ".scale"))
            if same: n_exact += 1; print(f"EXACT  {name}"); continue
            print(f"DIFF   {name}: int8 equal {eq/tot:.2%}; dequant |ours-FLM| max {emax:.3g} ({smax:.1f} FLM int8 steps), mean {esum/tot:.3g}")
            continue
        # Q4_1 tiles: rows of FLM tensor are tiles; walk 32*CH source rows at a time
        sn = src_name(name); N, K = src.shape(sn); nk = K // 256
        CH = 1024; beq = 0; seq = meq = qeq = 0; rt_ok = True; dmax = 0.0; ssf = ssm = 0.0; ng = 0
        for s0 in range(0, N, CH):
            s1 = min(N, s0 + CH); t0, t1 = s0 // 32 * nk, s1 // 32 * nk
            tb, mb = flm.rows(name, t0, t1).view(np.uint8), ours.rows(name, t0, t1).view(np.uint8)
            beq += int((tb == mb).sum())
            a1, b1, c1 = C.unpack_q4_1_tiles(tb); a2, b2, c2 = C.unpack_q4_1_tiles(mb)
            rt_ok &= np.array_equal(C.pack_q4_1_tiles(a1, b1, c1), tb)
            seq += int((a1 == a2).sum()); meq += int((b1 == b2).sum()); qeq += int((c1 == c2).sum()); ng += a1.size
            W = C.bf2f(src.rows(sn, s0, s1))
            Dt, Dm = C.dequant_q4_tiles(tb, s1 - s0, K), C.dequant_q4_tiles(mb, s1 - s0, K)
            dmax = max(dmax, float(np.abs(Dt - Dm).max()))
            ssf += float(((Dt - W) ** 2).sum()); ssm += float(((Dm - W) ** 2).sum())
        tot = R * 5120
        if beq == tot: n_exact += 1; print(f"EXACT  {name}"); continue
        print(f"DIFF   {name}: layout round trip {'EXACT' if rt_ok else 'FAIL'}; bytes equal {beq/tot:.2%};"
              f" scales equal {seq/ng:.2%}, mins equal {meq/ng:.2%}, nibbles equal {qeq/(ng*32):.2%};"
              f" |ours-FLM| max {dmax:.3g}; RMSE vs source: FLM {np.sqrt(ssf/(N*K)):.4g} ours {np.sqrt(ssm/(N*K)):.4g}")
    print(f"\n{n_exact}/{n} tensors byte-identical")

if __name__ == "__main__":
    main()
