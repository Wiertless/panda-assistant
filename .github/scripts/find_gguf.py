"""Find a GGUF build of a model on Hugging Face: python3 find_gguf.py "Qwen3.5-4B" [Q4_K_M]

Prints "repo file". Prefers well-known quantizers and the most downloaded
repo that has a file in the wanted quantization (no vision projector files).
"""
import json
import sys
import urllib.parse
import urllib.request

TRUSTED = ["unsloth", "ggml-org", "bartowski", "lmstudio-community", "Qwen", "google"]


def get(url):
    with urllib.request.urlopen(url, timeout=30) as r:
        return json.load(r)


def main():
    name, quant = sys.argv[1], (sys.argv[2] if len(sys.argv) > 2 else "Q4_K_M")
    q = urllib.parse.urlencode({"search": name, "filter": "gguf", "sort": "downloads", "direction": "-1", "limit": "30"})
    repos = [m["id"] for m in get(f"https://huggingface.co/api/models?{q}")]
    norm = lambda s: s.lower().replace("_", "-")
    repos = [r for r in repos if norm(name) in norm(r)]
    repos.sort(key=lambda r: TRUSTED.index(r.split("/")[0]) if r.split("/")[0] in TRUSTED else len(TRUSTED))
    for repo in repos:
        files = [f["path"] for f in get(f"https://huggingface.co/api/models/{repo}/tree/main") if f.get("type") == "file"]
        hits = [f for f in files if f.endswith(".gguf") and quant.lower() in f.lower() and "mmproj" not in f.lower()]
        if hits:
            print(repo, sorted(hits, key=len)[0])
            return
    sys.exit(f"no {quant} GGUF found for {name} (looked at: {', '.join(repos) or 'nothing'})")


main()
