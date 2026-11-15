"""Markdown table from assistantd --eval reports: python3 eval_table.py report.json..."""
import json
import sys

reports = [json.load(open(p)) for p in sys.argv[1:]]
for r in reports:
    v = r.get("variant", "baseline")
    r["label"] = r["model"] if v == "baseline" else f"{r['model']} · {v}"
reports.sort(key=lambda r: (-r["passed"], r["seconds"]))
names = []
for r in reports:
    names += [s["name"] for s in r["scenarios"] if s["name"] not in names]
print("| Model | Passed | s/request |")
print("|---|---|---|")
for r in reports:
    print(f"| `{r['label']}` | **{r['passed']}/{r['total']}** ({100 * r['passed'] / max(r['total'], 1):.0f}%) | {r['seconds'] / max(len(r['scenarios']), 1):.1f} |")
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
