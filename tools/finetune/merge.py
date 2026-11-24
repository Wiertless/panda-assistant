"""Merge a LoRA adapter (from train.py) into Gemma 4 E2B's weights, on the laptop.

    python3 merge.py BASE/model.safetensors ADAPTER_DIR OUT/model.safetensors

The merged model has the same tensors, shapes and dtypes as the base, so the
output is a copy of the base file (a reflink on btrfs: no space, no time) with
each LoRA target overwritten in place by W + (B @ A) * alpha / r, one tensor at
a time: a few hundred MB of memory, not the 10 GB the model takes. numpy only.
"""

import json
import shutil
import struct
import subprocess
import sys
from pathlib import Path

import numpy as np

DT = {"BF16": np.uint16, "F16": np.float16, "F32": np.float32}


def header(path):
    with open(path, "rb") as f:
        n = struct.unpack("<Q", f.read(8))[0]
        h = json.loads(f.read(n))
    h.pop("__metadata__", None)
    return h, 8 + n


def read(path, h, base, name):
    v = h[name]
    a, b = v["data_offsets"]
    with open(path, "rb") as f:
        f.seek(base + a)
        raw = f.read(b - a)
    x = np.frombuffer(raw, DT[v["dtype"]]).reshape(v["shape"])
    if v["dtype"] == "BF16":
        return (x.astype(np.uint32) << 16).view(np.float32)
    return x.astype(np.float32)


def to_bf16(x):
    """float32 -> BF16 bits, rounding to nearest even (as torch does)."""
    u = np.ascontiguousarray(x, np.float32).view(np.uint32)
    return ((u + 0x7FFF + ((u >> 16) & 1)) >> 16).astype(np.uint16)


def base_name(module, names):
    """The base tensor a LoRA module changes. PEFT names follow how transformers
    loaded the model (with or without "language_model"); the base file's don't."""
    m = module.removeprefix("base_model.model.")
    for cand in (m, m.replace("model.layers.", "model.language_model.layers."),
                 m.replace("model.model.", "model.")):
        if cand + ".weight" in names:
            return cand + ".weight"
    raise KeyError(f"no base tensor for {module}")


def merge(base_path, adapter_dir, out_path):
    adapter_dir = Path(adapter_dir)
    cfg = json.loads((adapter_dir / "adapter_config.json").read_text())
    scale = cfg["lora_alpha"] / cfg["r"]
    apath = adapter_dir / "adapter_model.safetensors"
    ah, abase = header(apath)
    bh, bbase = header(base_path)
    out_path = Path(out_path)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(["cp", "--reflink=auto", str(base_path), str(out_path)], check=True)
    modules = sorted({k.split(".lora_")[0] for k in ah if ".lora_A" in k})
    with open(out_path, "r+b") as f:
        for i, mod in enumerate(modules):
            name = base_name(mod, bh)
            a = read(apath, ah, abase, next(k for k in ah if k.startswith(mod + ".lora_A")))
            b = read(apath, ah, abase, next(k for k in ah if k.startswith(mod + ".lora_B")))
            w = read(base_path, bh, bbase, name)
            assert w.shape == (b.shape[0], a.shape[1]), f"{name}: {w.shape} vs {b.shape}@{a.shape}"
            assert bh[name]["dtype"] == "BF16", f"{name} is {bh[name]['dtype']}"
            merged = to_bf16(w + (b @ a) * scale)
            off = bh[name]["data_offsets"][0]
            f.seek(bbase + off)
            f.write(merged.tobytes())
            if i % 50 == 0:
                print(f"{i + 1}/{len(modules)} {name}", flush=True)
    print(f"merged {len(modules)} tensors (scale {scale}) -> {out_path}")
    # The converter reads config.json next to the weights.
    for extra in ("config.json",):
        src = Path(base_path).parent / extra
        if src.exists():
            shutil.copy(src, out_path.parent / extra)


def self_check():
    """BF16 rounding and the name mapping, without a model."""
    x = np.array([1.0, -2.5, 3.14159, 1e-8], np.float32)
    back = (to_bf16(x).astype(np.uint32) << 16).view(np.float32)
    assert np.allclose(back, x, rtol=1 / 128), back
    names = {"model.language_model.layers.0.self_attn.q_proj.weight": 1}
    assert base_name("base_model.model.model.layers.0.self_attn.q_proj", names) in names
    assert base_name("base_model.model.model.language_model.layers.0.self_attn.q_proj", names) in names


if __name__ == "__main__":
    self_check()
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    merge(*sys.argv[1:])
