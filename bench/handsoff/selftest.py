#!/usr/bin/env python3
"""Is the hands-off rubric fit to judge anything?

Builds a throwaway bench of toy tasks, runs them sequentially and in a
parallel batch against fake_hexa.py (no model, no network), grades both, and
checks that every outcome and every multi-agent metric comes out as designed.
A grader that cannot report wrong-but-green, tampering, a conflict or a failed
re-gate on tasks built to produce them is not fit to grade real runs.

Exits 0 when every expectation holds, 1 otherwise.

  python3 bench/handsoff/selftest.py
"""
import csv, os, subprocess, sys, tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
FAKE = HERE / "selftest" / "fake_hexa.py"

# id: (prompt directives, gate, holdout, locked, solvable)
TASKS = {
    "ok":     ("write a.txt=ok", "grep -q ok a.txt", None, [], True),
    "wbg":    ("write w.txt=wrong", "test -f w.txt", "grep -q right w.txt", [], True),
    "shape":  ("write s.txt=x; write BAD_SHAPE=1", "test -f s.txt", None, [], True),
    "tamper": ("write lock.txt=hacked; write t.txt=1", "test -f t.txt", None, ["lock.txt"], True),
    "stuck":  ("write x.txt=1", "false", None, [], False),
    # c1 lands first; c2 then conflicts on shared.txt, is sent back, and lands
    # over c1, so c1 is correct alone but wrong-but-green once integrated.
    "c1":     ("write shared.txt=one", "grep -q one shared.txt", None, [], True),
    "c2":     ("sleep 0.6; write shared.txt=two", "grep -q two shared.txt", None, [], True),
    # r2 lands breaker.txt first; r1 rebases cleanly, fails its re-gate, and
    # cannot recover, so it escalates in the batch but is correct alone.
    "r1":     ("sleep 0.6; write r.txt=1", "test -f r.txt && ! test -f breaker.txt", None, [], True),
    "r2":     ("write breaker.txt=1", "test -f breaker.txt", None, [], True),
}

EXPECT = {
    "sequential": {"ok": "correct", "wbg": "wrong_but_green", "shape": "misshapen",
                   "tamper": "tampered", "stuck": "escalated", "c1": "correct",
                   "c2": "correct", "r1": "correct", "r2": "correct"},
    # In a batch the merge queue gates the grade, so the misshapen change
    # never lands and escalates instead.
    "parallel":   {"ok": "correct", "wbg": "wrong_but_green", "shape": "escalated",
                   "tamper": "tampered", "stuck": "escalated", "c1": "wrong_but_green",
                   "c2": "correct", "r1": "escalated", "r2": "correct"},
}


def build_bench(root):
    repo = root / "_repo"
    repo.mkdir(parents=True)
    (repo / "shared.txt").write_text("base\n")
    (repo / "lock.txt").write_text("locked\n")
    for tid, (prompt, gate, holdout, locked, solvable) in TASKS.items():
        d = root / tid
        d.mkdir()
        lines = [f'id = "{tid}"', 'category = "selftest"', f"solvable = {str(solvable).lower()}",
                 'mode = "do"', 'file = "a.txt"', 'repo = "../_repo"',
                 f"prompt = {prompt!r}".replace("'", '"'), f"gate = {gate!r}".replace("'", '"'),
                 f"locked = {locked!r}".replace("'", '"'),
                 "max_rounds = 2", "timeout_min = 1"]
        if holdout:
            lines.append(f"holdout_cmd = {holdout!r}".replace("'", '"'))
        (d / "task.toml").write_text("\n".join(lines) + "\n")


def run(args, env, cwd):
    p = subprocess.run([sys.executable, *args], cwd=cwd, env=env,
                       capture_output=True, text=True)
    if p.returncode != 0:
        print(p.stdout, p.stderr, sep="\n")
        sys.exit(f"selftest: {args[0]} exited {p.returncode}")
    return p.stdout


def main():
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        bench, runs = tmp / "tasks", tmp / "runs"
        build_bench(bench)
        env = {**os.environ, "HEXA_BIN": f"{sys.executable} {FAKE}",
               "HANDSOFF_NOTIFY": "true"}
        run([str(HERE / "run_handsoff.py"), "--bench", str(bench),
             "--out", str(runs / "seq"), "--runs", "1"], env, tmp)
        run([str(HERE / "run_handsoff.py"), "--bench", str(bench),
             "--out", str(runs / "par"), "--runs", "1", "--parallel", "4"], env, tmp)
        report = run([str(HERE / "grade_handsoff.py"), "--runs", str(runs),
                      "--csv", str(tmp / "results.csv")], env, tmp)

        got = {(r["mode"], r["task"]): r for r in csv.DictReader(open(tmp / "results.csv"))}
        failures = []
        for mode, want in EXPECT.items():
            for tid, outcome in want.items():
                o = got.get((mode, tid), {}).get("outcome")
                mark = "ok  " if o == outcome else "FAIL"
                if o != outcome:
                    failures.append(f"{mode}/{tid}: want {outcome}, got {o}")
                print(f"{mark} {mode:10s} {tid:7s} {outcome:16s} got {o}")

        par = [r for (m, _), r in got.items() if m == "parallel"]
        conflicts = sum(int(r["conflicts"]) for r in par)
        regates = sum(int(r["regate_failures"]) for r in par)
        checks = [
            ("c2 hit a rebase conflict", int(got[("parallel", "c2")]["conflicts"]) >= 1),
            ("r1 failed a re-gate after rebase", int(got[("parallel", "r1")]["regate_failures"]) >= 1),
            ("shape was refused by the queue's grade", int(got[("parallel", "shape")]["regate_failures"]) >= 1),
            ("report shows a positive wrong-but-green delta", "wrong-but-green delta:   +" in report),
            ("report shows both throughputs", "throughput, landed/hour: sequential" in report),
        ]
        for name, ok in checks:
            print(f"{'ok  ' if ok else 'FAIL'} {name}")
            if not ok:
                failures.append(name)
        print(f"\nconflicts={conflicts} regate_failures={regates}")
        if failures:
            print("\n" + report)
            print("selftest FAILED:\n  " + "\n  ".join(failures))
            return 1
        print("selftest passed: the rubric discriminates every outcome")
        return 0


if __name__ == "__main__":
    sys.exit(main())
