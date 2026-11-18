"""Markdown table from assistantd --eval reports: python3 eval_table.py report.json..."""
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from eval_verdict import merge  # noqa: E402

reports = merge([json.load(open(p)) for p in sys.argv[1:]])
for r in reports:
    v = r.get("variant", "baseline")
    r["label"] = r["model"] if v == "baseline" else f"{r['model']} · {v}"
    if r.get("partial"):
        r["label"] += " (cut off)"
reports.sort(key=lambda r: (-r["passed"], r["seconds"]))
names = []
for r in reports:
    names += [s["name"] for s in r["scenarios"] if s["name"] not in names]
print("| Model | Passed | Scenarios fully passed | Not offered by routing | s/request | Model calls/request | Prompt tokens/request |")
print("|---|---|---|---|---|---|---|")
for r in reports:
    clusters = f"{r['clusters_passed']}/{r['clusters']}" if "clusters" in r else "—"
    cost = f"{r['model_calls']:.1f} | {r['prompt_tokens']:.0f}" if "model_calls" in r else "— | —"
    print(f"| `{r['label']}` | **{r['passed']}/{r['total']}** ({100 * r['passed'] / max(r['total'], 1):.0f}%) | {clusters} "
          f"| {r.get('unrouted', '—')} | {r['seconds'] / max(len(r['scenarios']), 1):.1f} | {cost} |")
# What one variant fixed and broke against another (same model): totals hide offsetting changes.
by_model = {}
for r in reports:
    by_model.setdefault(r["model"], []).append(r)
for model, rs in by_model.items():
    base = next((r for r in rs if r.get("variant", "baseline") == "baseline"), None)
    for r in rs:
        if base is None or r is base:
            continue
        ok = lambda rep, n: next((s["passed"] for s in rep["scenarios"] if s["name"] == n), None)
        fixed = [s["name"] for s in r["scenarios"] if s["passed"] and ok(base, s["name"]) is False]
        broke = [s["name"] for s in r["scenarios"] if not s["passed"] and ok(base, s["name"]) is True]
        print(f"\n`{r['label']}` vs baseline: fixed {len(fixed)} ({', '.join(fixed) or '—'}); broke {len(broke)} ({', '.join(broke) or '—'})")
caps = sorted({c for r in reports for c in r.get("capabilities", {})})
if caps:
    print()
    print("| Capability | " + " | ".join(f"`{r['label']}`" for r in reports) + " |")
    print("|---|" + "---|" * len(reports))
    for c in caps:
        cells = []
        for r in reports:
            v = r.get("capabilities", {}).get(c)
            cells.append("—" if not v else f"{v['passed']}/{v['total']}")
        print(f"| {c} | " + " | ".join(cells) + " |")
print()
print("| Scenario | " + " | ".join(f"`{r['label']}`" for r in reports) + " |")
print("|---|" + "---|" * len(reports))
for n in names:
    cells = []
    for r in reports:
        s = next((s for s in r["scenarios"] if s["name"] == n), None)
        if s is None:
            cells.append("—")
        elif s.get("runs", 1) > 1 and 0 < s.get("passed_runs", 0) < s["runs"]:
            cells.append(f"{s['passed_runs']}/{s['runs']}")
        else:
            cells.append("✅" if s.get("passed_runs", int(s["passed"])) > 0 else "❌")
    print(f"| {n} | " + " | ".join(cells) + " |")
fails = [(r["label"], s) for r in reports for s in r["scenarios"] if not s["passed"]]
if fails:
    print("\n<details><summary>Why they failed</summary>\n")
    for label, s in fails:
        calls = " → ".join(c["tool"] for c in s["calls"]) or "no tools"
        print(f"- `{label}` **{s['name']}**: {s['why']} ({calls})")
    print("\n</details>")
