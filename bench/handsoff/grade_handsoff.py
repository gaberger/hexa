#!/usr/bin/env python3
"""Grade hands-off runs produced by run_handsoff.py.

Runs in its own process, after the agents are done. For each run it re-runs
the gate on a clean copy of the graded tree, copies in the hidden holdouts,
runs them, re-checks the architecture grade, re-hashes locked files and runs
the regression suite. Sequential runs are graded on their own tree; parallel
runs on the batch's integrated main, so a change that passed alone but clashes
with another lands as wrong-but-green. Each run gets exactly one outcome:

  correct           gate 0, holdouts pass, grade A, locks unchanged, no regressions
  wrong_but_green   landed, but the gate, holdouts or suite fail on the graded tree
  misshapen         behaviour correct but grade below A
  tampered          a locked file changed (fails regardless of tests)
  escalated         nothing landed and the notifier fired
  silent_failure    nothing landed and no notification (harness bug)
  invalid_floor     starting tree failed the grade; task excluded

Multi-agent metrics, from parallel runs:

  conflict rate        rebase conflicts / merge-queue submissions
  re-gate failure rate failed re-gates / clean rebases
  wrong-but-green delta  parallel rate - sequential rate, on tasks run both ways
  throughput           landed changes per hour of wall time, per mode

Usage:
  python3 grade_handsoff.py --runs runs            # globs runs/**/result.json
"""
import argparse, csv, json, shutil, statistics, tomllib
from collections import Counter, defaultdict
from pathlib import Path

import importlib.util
_spec = importlib.util.spec_from_file_location(
    "runner", Path(__file__).with_name("run_handsoff.py"))
runner = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(runner)
sh, hash_locked, HEXA = runner.sh, runner.hash_locked, runner.HEXA


def grade_run(task_dir, t, run_dir, r):
    if r.get("outcome_hint") == "invalid_floor":
        return "invalid_floor", {}
    tree = Path(r["grade_dir"])

    # Tampering is checked on the graded tree before anything else.
    if hash_locked(tree, t.get("locked", [])) != r.get("locked_before", {}):
        return "tampered", {}

    if not r.get("landed"):
        return ("escalated" if r.get("notified") else "silent_failure"), {}

    # Clean grading copy: graded tree + this task's hidden holdouts.
    g = run_dir / "grade"
    if g.exists():
        shutil.rmtree(g)
    shutil.copytree(tree, g, ignore=shutil.ignore_patterns("target", "node_modules"))
    if (task_dir / "holdout").exists():
        shutil.copytree(task_dir / "holdout", g, dirs_exist_ok=True)

    checks = {}
    checks["gate"], _ = sh(t["gate"], g, timeout=1800)
    checks["holdout"], hold_log = sh(t.get("holdout_cmd", t["gate"]), g, timeout=1800)
    checks["suite"], _ = sh(t.get("suite_cmd", t["gate"]), g, timeout=1800)
    checks["grade"], grade_log = sh(f"{HEXA} analyze . --grade A", g, timeout=600)
    (run_dir / "holdout.log").write_text(hold_log)
    (run_dir / "grade.log").write_text(grade_log)
    shutil.rmtree(g, ignore_errors=True)

    if checks["gate"] != 0 or checks["holdout"] != 0 or checks["suite"] != 0:
        return "wrong_but_green", checks
    if checks["grade"] != 0:
        return "misshapen", checks
    return "correct", checks


def pct(n, d):
    return f"{100 * n / d:.0f}% ({n}/{d})" if d else "n/a"


def rate(n, d):
    return n / d if d else None


def med_p90(rows, field):
    vals = sorted(x[field] for x in rows if isinstance(x.get(field), (int, float)))
    if not vals:
        return "n/a"
    p90 = vals[min(len(vals) - 1, int(0.9 * len(vals)))]
    return f"median {statistics.median(vals):g}, p90 {p90:g}"


def count(rows, outcome):
    return sum(1 for x in rows if x["outcome"] == outcome)


def landed_of(rows):
    return [x for x in rows if x["outcome"] in ("correct", "wrong_but_green", "misshapen")]


def summary(rows, label):
    solv = [x for x in rows if x["solvable"]]
    unsolv = [x for x in rows if not x["solvable"]]
    landed = landed_of(rows)
    by_task = defaultdict(list)
    for x in rows:
        by_task[x["task"]].append(x["outcome"])
    consistent = sum(1 for v in by_task.values() if len(set(v)) == 1)
    print(f"\n== {label}: {len(rows)} runs ==")
    print(f"correct rate:          {pct(count(solv, 'correct'), len(solv))}   target >= 80%")
    print(f"wrong-but-green rate:  {pct(count(landed, 'wrong_but_green'), len(landed))}   target ~0  <- deciding number")
    print(f"misshapen:             {pct(count(landed, 'misshapen'), len(landed))}   target 0")
    print(f"tampered:              {count(rows, 'tampered')}   target 0")
    print(f"escalation recall:     {pct(count(unsolv, 'escalated'), len(unsolv))}   target 100%")
    print(f"false escalations:     {pct(count(solv, 'escalated'), len(solv))}   target <= 20%")
    print(f"silent failures:       {count(rows, 'silent_failure')}   (harness bugs)")
    print(f"consistency:           {pct(consistent, len(by_task))}")
    print(f"spend per run (USD):   {med_p90(rows, 'spend_usd')}")
    print(f"wall time per run (s): {med_p90(rows, 'wall_s')}")


def wbg_rate(rows):
    landed = landed_of(rows)
    return rate(count(landed, "wrong_but_green"), len(landed))


def multi_agent(seq, par, runs_root):
    print("\n== Multi-agent ==")
    submissions = sum(x["conflicts"] + x["regate_failures"] + (1 if x["landed"] else 0)
                      for x in par)
    conflicts = sum(x["conflicts"] for x in par)
    regates = sum(x["regate_failures"] for x in par)
    print(f"merge-queue submissions: {submissions}")
    print(f"conflict rate:           {pct(conflicts, submissions)}")
    print(f"re-gate failure rate:    {pct(regates, submissions - conflicts)}")

    both = {x["task"] for x in seq} & {x["task"] for x in par}
    s = wbg_rate([x for x in seq if x["task"] in both])
    p = wbg_rate([x for x in par if x["task"] in both])
    if s is None or p is None:
        print("wrong-but-green delta:   n/a (need the same tasks run both ways)")
    else:
        print(f"wrong-but-green delta:   {100 * (p - s):+.0f} points "
              f"(parallel {100 * p:.0f}% vs sequential {100 * s:.0f}%, {len(both)} tasks)")

    seq_wall = sum(x["wall_s"] or 0 for x in seq)
    par_wall = sum(json.loads(b.read_text())["wall_s"]
                   for b in Path(runs_root).glob("**/batch.json"))
    seq_tp = rate(sum(x["landed"] for x in seq), seq_wall / 3600)
    par_tp = rate(sum(x["landed"] for x in par), par_wall / 3600)
    fmt = lambda v: f"{v:.1f}/h" if v is not None else "n/a"
    print(f"throughput, landed/hour: sequential {fmt(seq_tp)}, parallel {fmt(par_tp)}")
    print(f"landing time in batch (s): {med_p90(par, 'landed_at_s')}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", default="runs")
    ap.add_argument("--csv", default="results.csv")
    a = ap.parse_args()

    rows = []
    for res in sorted(Path(a.runs).glob("**/result.json")):
        r = json.loads(res.read_text())
        task_dir = Path(r["task_dir"])
        t = tomllib.loads((task_dir / "task.toml").read_text())
        t.setdefault("id", r["task"])
        outcome, checks = grade_run(task_dir, t, res.parent, r)
        rows.append({
            "task": r["task"], "category": r["category"], "solvable": r["solvable"],
            "mode": r.get("mode", "sequential"), "batch": r.get("batch"),
            "run": r["run"], "outcome": outcome, "landed": bool(r.get("landed")),
            "rounds": len(r.get("rounds", [])), "wall_s": r.get("wall_s"),
            "landed_at_s": r.get("landed_at_s"), "spend_usd": r.get("spend_usd"),
            "conflicts": r.get("conflicts", 0), "regate_failures": r.get("regate_failures", 0),
            "stop_reason": r.get("stop_reason"),
            **{f"exit_{k}": v for k, v in checks.items()},
        })
    if not rows:
        raise SystemExit("no results found")

    keys = sorted({k for row in rows for k in row})
    with open(a.csv, "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=keys)
        w.writeheader()
        w.writerows(rows)

    valid = [x for x in rows if x["outcome"] != "invalid_floor"]
    print(f"graded {len(valid)} runs (excluded invalid floor: {len(rows) - len(valid)})")
    seq = [x for x in valid if x["mode"] == "sequential"]
    par = [x for x in valid if x["mode"] == "parallel"]
    if seq:
        summary(seq, "Sequential")
    if par:
        summary(par, "Parallel")
        multi_agent(seq, par, a.runs)

    print("\n== By category ==")
    cats = defaultdict(Counter)
    for x in valid:
        cats[(x["mode"], x["category"])][x["outcome"]] += 1
    for (mode, cat), cnt in sorted(cats.items()):
        print(f"{mode:10s} {cat:22s} " + ", ".join(f"{k}={v}" for k, v in sorted(cnt.items())))
    print(f"\nper-run detail written to {a.csv}")
    return rows


if __name__ == "__main__":
    main()
