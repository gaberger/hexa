#!/usr/bin/env python3
"""Hands-off agent workflow runner for hexa.

For every task in <bench>/<task>/task.toml, run the agent K times with no
human involved. A run ends in a landed change or an escalation (rounds, spend
or time ran out). The grader (grade_handsoff.py) scores the results afterwards
in a separate process.

Two modes:

  sequential (default)  each task alone, in a fresh copy of its repo; the
                        change lands when hexa commits on a green gate.
  --parallel N          tasks that share a repo form a batch and run N at a
                        time, one git worktree per agent off a shared main.
                        A green change lands by fast-forward; an agent that
                        started from an older main is reset and sent back a
                        round.

Uses these hexa verbs, checked against their clap definitions in hexa-cli
(src/main.rs, commands/direct.rs, build.rs, spend_cmd.rs), which `--help`
prints:
  hexa analyze . --grade A          exits 1 below the letter
  hexa do run "<prompt>" --file <f> --evidence "<gate>" [--also <f>]...
                                    exits 0 only when evidence passes and it commits
  hexa build "<prompt>" --target <dir> --gate "<gate>"
  hexa spend --json                 all.cost_usd and token totals
Set HEXA_BIN to point at another binary (the self-test uses a stub).

Usage:
  python3 run_handsoff.py --bench bench/handsoff/tasks --out runs/seq --runs 3
  python3 run_handsoff.py --bench bench/handsoff/tasks --out runs/par --runs 3 --parallel 4
  HANDSOFF_NOTIFY="<cmd>" ...   escalation hook, called with the payload path
"""
import argparse, hashlib, json, os, shlex, shutil, subprocess, sys
import threading, time, tomllib
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

HEXA = os.environ.get("HEXA_BIN", "hexa")
for k, v in (("GIT_AUTHOR_NAME", "handsoff-bench"), ("GIT_COMMITTER_NAME", "handsoff-bench"),
             ("GIT_AUTHOR_EMAIL", "bench@handsoff.local"),
             ("GIT_COMMITTER_EMAIL", "bench@handsoff.local")):
    os.environ.setdefault(k, v)


def sh(cmd, cwd, timeout=None):
    """Run a shell command, return (exit_code, combined_output)."""
    try:
        p = subprocess.run(cmd, cwd=cwd, shell=True, capture_output=True,
                           text=True, timeout=timeout)
        return p.returncode, p.stdout + p.stderr
    except subprocess.TimeoutExpired as e:
        out = (e.stdout or b"") + (e.stderr or b"")
        if isinstance(out, bytes):
            out = out.decode(errors="replace")
        return 124, out + "\n[timeout]"


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def hash_locked(work, locked):
    """Hash every locked file (globs allowed)."""
    hashes = {}
    for pattern in locked:
        for p in sorted(Path(work).glob(pattern)):
            if p.is_file():
                hashes[str(p.relative_to(work))] = sha256(p)
    return hashes


def spend(cwd):
    """All-time totals from `hexa spend --json`: (cost_usd, tokens, raw).

    A local model reports no cost, so cost stays 0 for local ladders; tokens
    still move. None when the call fails or the shape is unexpected.
    """
    code, out = sh(f"{HEXA} spend --json", cwd, timeout=60)
    try:
        a = json.loads(out[out.index("{"):])["all"]
        return float(a["cost_usd"]), int(a["input_tokens"]) + int(a["output_tokens"]), out
    except (ValueError, KeyError, TypeError):
        return None, None, out


def delta(a, b):
    return round(b - a, 4) if a is not None and b is not None else None


def git_head(work):
    code, out = sh("git rev-parse HEAD", work)
    return out.strip() if code == 0 else None


def init_repo(src, dest):
    """Copy a task's starting tree into dest as a git repo on branch main."""
    if dest.exists():
        shutil.rmtree(dest)
    shutil.copytree(src, dest)  # holdout/ lives beside repo/ and is never copied
    if not (dest / ".git").exists():
        sh("git init -q && git add -A && git commit -qm floor", dest)
    sh("git checkout -q -B main", dest)


def agent_cmd(t):
    q = shlex.quote
    if t.get("mode", "do") == "do":
        # `do run` commits only --file and each --also; any other file it
        # touches is restored, so multi-file tasks list them in `also`.
        also = "".join(f" --also {q(p)}" for p in t.get("also", []))
        return (f"{HEXA} do run {q(t['prompt'])} --file {q(t['file'])}"
                f" --evidence {q(t['gate'])}{also}")
    return f"{HEXA} build {q(t['prompt'])} --target {q(t.get('target', '.'))} --gate {q(t['gate'])}"


def escalate(run_dir, payload):
    """Write the payload and fire the notifier if configured. True if it reported success."""
    path = run_dir / "escalation.json"
    path.write_text(json.dumps(payload, indent=2))
    hook = os.environ.get("HANDSOFF_NOTIFY")
    if not hook:
        return False
    code, out = sh(f"{hook} {shlex.quote(str(path))}", run_dir, timeout=60)
    (run_dir / "notify.log").write_text(out)
    return code == 0


def base_result(task_dir, t, mode, run_name):
    return {"task": t["id"], "category": t["category"], "solvable": t.get("solvable", True),
            "mode": mode, "run": run_name, "task_dir": str(task_dir.resolve())}


def caps(t):
    """(max_rounds, cost cap USD, token cap or None, deadline)."""
    return (int(t.get("max_rounds", 3)), float(t.get("spend_cap_usd", 2.0)),
            t.get("token_cap"), time.time() + 60 * float(t.get("timeout_min", 30)))


# ---------------------------------------------------------------- sequential

def run_sequential(task_dir, t, run_dir):
    work = run_dir / "work"
    init_repo(task_dir / t.get("repo", "repo"), work)
    result = base_result(task_dir, t, "sequential", run_dir.name)

    code, out = sh(f"{HEXA} analyze . --grade A", work, timeout=300)
    if code != 0:
        result.update(outcome_hint="invalid_floor", floor_log=out[-4000:])
        return result

    result["locked_before"] = hash_locked(work, t.get("locked", []))
    head0 = git_head(work)
    cost0, tok0, _ = spend(work)
    max_rounds, cost_cap, token_cap, deadline = caps(t)
    start, rounds, last_out, success, stop = time.time(), [], "", False, "rounds"

    for i in range(max_rounds):
        remaining = deadline - time.time()
        if remaining <= 0:
            stop = "time"
            break
        code, last_out = sh(agent_cmd(t), work, timeout=remaining)
        rounds.append(code)
        (run_dir / f"round{i + 1}.log").write_text(last_out)
        if code == 0:
            success = True
            break
        cost, tok, _ = spend(work)
        if (delta(cost0, cost) or 0) >= cost_cap or \
                (token_cap and (delta(tok0, tok) or 0) >= token_cap):
            stop = "spend"
            break

    cost1, tok1, spend_raw = spend(work)
    head1 = git_head(work)
    wall = round(time.time() - start, 1)
    result.update(
        rounds=rounds, success=success, head0=head0, head1=head1,
        committed=bool(head1 and head1 != head0), landed=success,
        landed_at_s=wall if success else None, wall_s=wall,
        spend_usd=delta(cost0, cost1), tokens=delta(tok0, tok1),
        spend_raw=spend_raw[-2000:], stop_reason=None if success else stop,
        conflicts=0, regate_failures=0, grade_dir=str(work.resolve()),
    )
    if result["committed"]:
        _, diff = sh(f"git diff {head0} {head1}", work)
        (run_dir / "final.diff").write_text(diff)
    finish(run_dir, t, result, last_out, stop, success)
    return result


def finish(run_dir, t, result, last_out, stop, success):
    result["escalated"] = not success
    result["notified"] = False
    if not success:
        result["notified"] = escalate(run_dir, {
            "task": t["id"], "prompt": t["prompt"], "gate": t["gate"],
            "stop_reason": stop, "rounds": result.get("rounds"),
            "conflicts": result.get("conflicts"), "regate_failures": result.get("regate_failures"),
            "last_output_tail": last_out[-3000:], "spend_usd": result.get("spend_usd"),
        })


# ------------------------------------------------------------------ parallel

class Lander:
    """Lands a finished change on main by fast-forward, one at a time.

    Main is advanced only when the agent's branch already contains it, so the
    first agent to finish lands and anyone who started from an older main is
    refused. A refused agent's worktree is reset to the new main and the task
    goes back a round.
    """

    def __init__(self, main, t0):
        self.main, self.t0, self.lock = main, t0, threading.Lock()

    def submit(self, t, wt):
        with self.lock:
            code, _ = sh(f"git merge -q --ff-only agent/{t['id']}", self.main, timeout=300)
            if code != 0:
                sh("git reset -q --hard main && git clean -fdq", wt)
                return {"status": "conflict"}
            return {"status": "landed", "at_s": round(time.time() - self.t0, 3)}


def run_agent(task_dir, t, wt, run_dir, mq):
    """One agent's life in a batch: rounds of hexa, each green one offered to main."""
    result = base_result(task_dir, t, "parallel", run_dir.parent.name)
    # hexa's spend log is global, so concurrent agents can't be told apart:
    # rounds and time cap each agent; spend is recorded per batch.
    max_rounds, _cost_cap, _token_cap, deadline = caps(t)
    rounds, conflicts, regates, landed_at, last_out, stop = [], 0, 0, None, "", "rounds"
    start = time.time()
    while len(rounds) < max_rounds:
        remaining = deadline - time.time()
        if remaining <= 0:
            stop = "time"
            break
        code, last_out = sh(agent_cmd(t), wt, timeout=remaining)
        rounds.append(code)
        (run_dir / f"round{len(rounds)}.log").write_text(last_out)
        if code != 0:
            continue
        verdict = mq.submit(t, wt)
        if verdict.get("status") == "landed":
            landed_at = verdict["at_s"]
            break
        if verdict.get("status") == "conflict":
            conflicts += 1
    landed = landed_at is not None
    result.update(rounds=rounds, success=landed, landed=landed, committed=landed,
                  landed_at_s=landed_at, wall_s=round(time.time() - start, 1),
                  conflicts=conflicts, regate_failures=regates, spend_usd=None,
                  stop_reason=None if landed else stop)
    finish(run_dir, t, result, last_out, stop, landed)
    return result


def run_batch(name, members, batch_dir, workers):
    """Run one batch: shared main, a worktree per agent, one landing at a time."""
    if batch_dir.exists():
        shutil.rmtree(batch_dir)
    batch_dir.mkdir(parents=True)
    main = batch_dir / "main"
    task_dir0, t0_ = members[0]
    init_repo(task_dir0 / t0_.get("repo", "repo"), main)

    code, out = sh(f"{HEXA} analyze . --grade A", main, timeout=300)
    if code != 0:
        for task_dir, t in members:
            rd = batch_dir / t["id"]
            rd.mkdir()
            r = base_result(task_dir, t, "parallel", batch_dir.name)
            r.update(outcome_hint="invalid_floor", floor_log=out[-4000:])
            (rd / "result.json").write_text(json.dumps(r, indent=2))
        return

    t_start = time.time()
    mq = Lander(main, t_start)
    cost0, tok0, _ = spend(main)
    jobs = []
    for task_dir, t in members:
        rd = batch_dir / t["id"]
        rd.mkdir()
        wt = batch_dir / "wt" / t["id"]
        sh(f"git worktree add -q -b agent/{t['id']} {shlex.quote(str(wt.resolve()))} main", main)
        locked = hash_locked(main, t.get("locked", []))
        jobs.append((task_dir, t, wt, rd, locked))

    def one(job):
        task_dir, t, wt, rd, locked = job
        r = run_agent(task_dir, t, wt, rd, mq)
        r["locked_before"] = locked
        r["grade_dir"] = str(main.resolve())  # graded on the integrated main
        r["batch"] = name
        (rd / "result.json").write_text(json.dumps(r, indent=2))
        return r

    with ThreadPoolExecutor(max_workers=workers) as ex:
        results = list(ex.map(one, jobs))
    cost1, tok1, _ = spend(main)
    (batch_dir / "batch.json").write_text(json.dumps({
        "batch": name, "run": batch_dir.name, "workers": workers,
        "tasks": [r["task"] for r in results], "wall_s": round(time.time() - t_start, 1),
        "landed": sum(r["landed"] for r in results),
        "spend_usd": delta(cost0, cost1), "tokens": delta(tok0, tok1),
        "main": str(main.resolve()), "head": git_head(main),
    }, indent=2))


# ---------------------------------------------------------------------- main

def load_tasks(bench, only):
    out = []
    for task_dir in sorted(p.parent for p in Path(bench).glob("*/task.toml")):
        t = tomllib.loads((task_dir / "task.toml").read_text())
        t.setdefault("id", task_dir.name)
        if not only or t["id"] in only:
            out.append((task_dir, t))
    return out


def batches(tasks):
    """Group tasks by `batch`, else by the repo they start from."""
    groups = {}
    for task_dir, t in tasks:
        repo = (task_dir / t.get("repo", "repo")).resolve()
        key = t.get("batch") or str(repo)
        groups.setdefault(key, []).append((task_dir, t, repo))
    for key, members in groups.items():
        repos = {m[2] for m in members}
        if len(repos) > 1:
            sys.exit(f"batch {key!r} mixes starting repos: {sorted(map(str, repos))}")
    return {k: [(d, t) for d, t, _ in v] for k, v in groups.items()}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bench", default="bench/handsoff/tasks")
    ap.add_argument("--out", default="runs")
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--parallel", type=int, default=0,
                    help="run each batch N agents at a time, one worktree each")
    ap.add_argument("--only", nargs="*", help="task ids to run")
    a = ap.parse_args()

    tasks = load_tasks(a.bench, a.only)
    if not tasks:
        sys.exit(f"no tasks under {a.bench}")
    out = Path(a.out)

    if a.parallel <= 0:
        for task_dir, t in tasks:
            for k in range(1, a.runs + 1):
                run_dir = out / t["id"] / f"run{k}"
                if run_dir.exists():
                    shutil.rmtree(run_dir)
                run_dir.mkdir(parents=True)
                print(f"[{t['id']} run{k}] ...", flush=True)
                r = run_sequential(task_dir, t, run_dir)
                (run_dir / "result.json").write_text(json.dumps(r, indent=2))
                state = ("invalid floor" if r.get("outcome_hint") else
                         "landed" if r.get("landed") else "escalated")
                print(f"[{t['id']} run{k}] {state}", flush=True)
        return

    for key, members in batches(tasks).items():
        name = members[0][1].get("batch") or \
            f"{Path(key).name}-{hashlib.sha1(key.encode()).hexdigest()[:6]}"
        for k in range(1, a.runs + 1):
            batch_dir = out / name / f"run{k}"
            print(f"[batch {name} run{k}] {len(members)} tasks, {a.parallel} at a time", flush=True)
            run_batch(name, members, batch_dir, a.parallel)
            b = json.loads((batch_dir / "batch.json").read_text()) \
                if (batch_dir / "batch.json").exists() else {}
            print(f"[batch {name} run{k}] landed {b.get('landed', 0)}/{len(members)}", flush=True)


if __name__ == "__main__":
    main()
