"""Markdown table from assistantd --eval reports: python3 eval_table.py report.json..."""
import json
import sys

reports = [json.load(open(p)) for p in sys.argv[1:]]
reports.sort(key=lambda r: -r["passed"])
names = [s["name"] for s in reports[0]["scenarios"]] if reports else []
print("| Model | Passed | s/request |")
print("|---|---|---|")
for r in reports:
    print(f"| `{r['model']}` | **{r['passed']}/{r['total']}** | {r['seconds'] / max(r['total'], 1):.1f} |")
print()
print("| Scenario | " + " | ".join(f"`{r['model']}`" for r in reports) + " |")
print("|---|" + "---|" * len(reports))
for n in names:
    cells = []
    for r in reports:
        s = next((s for s in r["scenarios"] if s["name"] == n), None)
        cells.append("—" if s is None else ("✅" if s["passed"] else "❌"))
    print(f"| {n} | " + " | ".join(cells) + " |")
fails = [(r["model"], s) for r in reports for s in r["scenarios"] if not s["passed"]]
if fails:
    print("\n<details><summary>Why they failed</summary>\n")
    for model, s in fails:
        calls = " → ".join(c["tool"] for c in s["calls"]) or "no tools"
        print(f"- `{model}` **{s['name']}**: {s['why']} ({calls})")
    print("\n</details>")
