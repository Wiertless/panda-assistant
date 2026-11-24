"""Read FastFlowLM .q4nx files (a safetensors-style container)."""
import json, struct, numpy as np

DT = {"BF16": np.uint16, "F32": np.float32, "I8": np.int8, "U8": np.uint8, "F16": np.float16}

def header(path):
    f = open(path, "rb"); n = struct.unpack("<Q", f.read(8))[0]
    return json.loads(f.read(n)), 8 + n

def tensor(path, name):
    h, base = header(path); v = h[name]; a, b = v["data_offsets"]
    with open(path, "rb") as f:
        f.seek(base + a); raw = f.read(b - a)
    return np.frombuffer(raw, DT[v["dtype"]]).reshape(v["shape"]), raw

def bf16(u16):
    return (u16.astype(np.uint32) << 16).view(np.float32)
