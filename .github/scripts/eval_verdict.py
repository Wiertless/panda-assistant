"""Is a variant better than baseline, or is it noise? python3 eval_verdict.py report.json...

Paired by scenario family (a prompt and its paraphrases, "name ~2", count once:
they fail together), so only the families where the two differ say anything.
Exact sign test on those, and a bootstrap interval on the pass-rate difference.
"""
import json
import math
import random
import sys
from collections import defaultdict

ALPHA = 0.05


def family(name):
    return name.split(" ~")[0]


def rates(report):
    """Pass rate and seconds per family: {family: (rate, seconds)}."""
    acc = defaultdict(lambda: [0, 0, 0.0])
    for s in report["scenarios"]:
        f = acc[family(s["name"])]
        runs = s.get("runs", 1)
        f[0] += s.get("passed_runs", int(s["passed"]))
        f[1] += runs
        f[2] += s.get("seconds", 0.0)
    return {k: (p / max(n, 1), sec) for k, (p, n, sec) in acc.items()}


def sign_p(wins, losses):
    """Two-sided exact binomial p for wins vs losses at 50/50."""
    n = wins + losses
    if n == 0:
        return 1.0
    k = min(wins, losses)
    tail = sum(math.comb(n, i) for i in range(k + 1)) / 2**n
    return min(1.0, 2 * tail)


def runs_needed(wins, losses):
    """Discordant families an 80%-power sign test needs at the observed split, as a multiple of now."""
    n = wins + losses
    if n == 0 or wins == losses:
        return None
    p = wins / n
    need = ((1.96 * 0.5 + 0.84 * math.sqrt(p * (1 - p))) / (p - 0.5)) ** 2
    return need / n


def bootstrap(diffs, total, reps=4000, seed=1):
    """95% interval of the change in pass rate (points out of 100), resampling families."""
    rnd = random.Random(seed)
    keys = list(diffs)
    if not keys:
        return (0.0, 0.0)
    sims = sorted(
        100 * sum(diffs[rnd.choice(keys)] for _ in keys) / total for _ in range(reps)
    )
    return sims[int(0.025 * reps)], sims[int(0.975 * reps)]


def compare(base, other):
    a, b = rates(base), rates(other)
    common = [k for k in a if k in b]
    diffs = {k: b[k][0] - a[k][0] for k in common}
    wins = sum(d > 0 for d in diffs.values())
    losses = sum(d < 0 for d in diffs.values())
    p = sign_p(wins, losses)
    lo, hi = bootstrap(diffs, len(common))
    change = 100 * sum(diffs.values()) / max(len(common), 1)
    faster = sum(b[k][1] < a[k][1] for k in common)
    slower = sum(b[k][1] > a[k][1] for k in common)
    secs = (sum(b[k][1] for k in common) - sum(a[k][1] for k in common)) / max(sum(a[k][1] for k in common), 1e-9)
    if p < ALPHA:
        verdict = "**BETTER**" if wins > losses else "**WORSE**"
    else:
        more = runs_needed(wins, losses)
        verdict = "**UNDECIDED**" + (
            f" — at this split it would take about {math.ceil(more)}× the runs to tell"
            if more and more < 50 else " — no real difference"
        )
    speed_p = sign_p(faster, slower)
    speed = f"{100 * secs:+.0f}% time" + (" (real)" if speed_p < ALPHA else " (noise)")
    return (
        f"{verdict}: {change:+.1f} points (95% range {lo:+.1f} to {hi:+.1f}), "
        f"better on {wins} families, worse on {losses}, of {len(common)} (p = {p:.2f}); {speed}"
    )


def main(paths):
    reports = [json.load(open(p)) for p in paths]
    by_model = defaultdict(list)
    for r in reports:
        by_model[r["model"]].append(r)
    print("## Verdict against baseline\n")
    for model, rs in by_model.items():
        base = next((r for r in rs if r.get("variant", "baseline") == "baseline"), None)
        if base is None:
            continue
        for r in rs:
            if r is base:
                continue
            print(f"- `{model} · {r['variant']}`: {compare(base, r)}")
    print()


if __name__ == "__main__":
    if sys.argv[1:] == ["--test"]:
        assert sign_p(0, 0) == 1.0
        assert abs(sign_p(5, 3) - 0.7265625) < 1e-9
        assert sign_p(12, 1) < 0.01
        assert runs_needed(5, 3) > 5
        mk = lambda passed: {"model": "m", "variant": "v", "scenarios": [
            {"name": f"s{i}" + (" ~2" if j else ""), "passed": ok, "passed_runs": int(ok), "runs": 1, "seconds": 1.0}
            for i, ok in enumerate(passed) for j in (0, 1)]}
        base, other = mk([False] * 15 + [True] * 25), mk([True] * 40)
        assert "BETTER" in compare(base, other), compare(base, other)
        assert "UNDECIDED" in compare(base, mk([True] + [False] * 14 + [True] * 25))
        print("ok")
    else:
        main(sys.argv[1:])
