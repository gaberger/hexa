#!/usr/bin/env python3
"""Is the hands-off rubric fit to judge anything?

Builds a throwaway bench of toy tasks, runs them sequentially and in a
parallel batch, grades both, and checks that every outcome and every
multi-agent metric comes out as designed. The whole suite runs twice: once
with the hexa agent against fake_hexa.py, once with agent = "claude-code"
against fake_claude.py (no model, no network either way).
A grader that cannot report wrong-but-green, tampering, a conflict or a failed
re-gate on tasks built to produce them is not fit to grade real runs.

Exits 0 when every expectation holds, 1 otherwise.

  python3 bench/handsoff/selftest.py
"""
import csv, json, os, subprocess, sys, tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
FAKE = HERE / "selftest" / "fake_hexa.py"
FAKE_CLAUDE = HERE / "selftest" / "fake_claude.py"
AGENTS = ("hexa", "claude-code")

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


def build_bench(root, agent):
    repo = root / "_repo"
    repo.mkdir(parents=True)
    (repo / "shared.txt").write_text("base\n")
    (repo / "lock.txt").write_text("locked\n")
    for tid, (prompt, gate, holdout, locked, solvable) in TASKS.items():
        d = root / tid
        d.mkdir()
        lines = [f'id = "{tid}"', 'category = "selftest"', f"solvable = {str(solvable).lower()}",
                 f'agent = "{agent}"', "max_turns = 5",
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


def suite(agent, tmp):
    """Run one agent through both modes, grade, and return the failures."""
    print(f"\n=== agent = {agent} ===")
    bench, runs = tmp / agent / "tasks", tmp / agent / "runs"
    build_bench(bench, agent)
    env = {**os.environ, "HEXA_BIN": f"{sys.executable} {FAKE}",
           "CLAUDE_BIN": f"{sys.executable} {FAKE_CLAUDE}", "HANDSOFF_NOTIFY": "true"}
    run([str(HERE / "run_handsoff.py"), "--bench", str(bench),
         "--out", str(runs / "seq"), "--runs", "1"], env, tmp)
    run([str(HERE / "run_handsoff.py"), "--bench", str(bench),
         "--out", str(runs / "par"), "--runs", "1", "--parallel", "4"], env, tmp)
    csv_path = tmp / agent / "results.csv"
    report = run([str(HERE / "grade_handsoff.py"), "--runs", str(runs),
                  "--csv", str(csv_path)], env, tmp)

    got = {(r["mode"], r["task"]): r for r in csv.DictReader(open(csv_path))}
    failures = []
    for mode, want in EXPECT.items():
        for tid, outcome in want.items():
            o = got.get((mode, tid), {}).get("outcome")
            if o != outcome:
                failures.append(f"{agent}/{mode}/{tid}: want {outcome}, got {o}")
            print(f"{'ok  ' if o == outcome else 'FAIL'} {mode:10s} {tid:7s} {outcome:16s} got {o}")

    checks = [
        ("c2 hit a rebase conflict", int(got[("parallel", "c2")]["conflicts"]) >= 1),
        ("r1 failed a re-gate after rebase", int(got[("parallel", "r1")]["regate_failures"]) >= 1),
        ("shape was refused by the queue's grade", int(got[("parallel", "shape")]["regate_failures"]) >= 1),
        ("report shows a positive wrong-but-green delta", "wrong-but-green delta:   +" in report),
        ("report shows both throughputs", "throughput, landed/hour: sequential" in report),
    ]
    if agent == "claude-code":
        results = [json.loads(p.read_text()) for p in runs.glob("**/result.json")]
        tamper = [r for r in results if r["task"] == "tamper"]
        ok_seq = next(r for r in results if r["task"] == "ok" and r["mode"] == "sequential")
        checks += [
            ("session cost read from claude's JSON", ok_seq.get("spend_usd") == 0.01),
            ("session tokens read from claude's JSON", ok_seq.get("tokens") == 150),
            ("locked path denied to the session",
             all("Edit(/lock.txt)" in r.get("denied", []) for r in tamper) and len(tamper) == 2),
            ("settings.local.json never committed",
             all(".claude/settings.local.json" not in
                 subprocess.run(["git", "ls-files", ".claude"], cwd=r["grade_dir"],
                                capture_output=True, text=True).stdout
                 for r in results if r.get("grade_dir"))),
            ("session left its edit uncommitted and the runner committed it",
             "bench: work the session left uncommitted" in
             subprocess.run(["git", "log", "--format=%s"], cwd=ok_seq["grade_dir"],
                            capture_output=True, text=True).stdout),
        ]
    for name, ok in checks:
        print(f"{'ok  ' if ok else 'FAIL'} {name}")
        if not ok:
            failures.append(f"{agent}: {name}")
    if failures:
        print("\n" + report)
    return failures


def main():
    with tempfile.TemporaryDirectory() as tmp:
        failures = []
        for agent in AGENTS:
            failures += suite(agent, Path(tmp))
        if failures:
            print("\nselftest FAILED:\n  " + "\n  ".join(failures))
            return 1
        print("\nselftest passed: the rubric discriminates every outcome, for both agents")
        return 0


if __name__ == "__main__":
    sys.exit(main())
