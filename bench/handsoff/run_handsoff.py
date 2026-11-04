#!/usr/bin/env python3
"""Hands-off agent workflow runner for hexa.

For every task in <bench>/<task>/task.toml, run the agent K times with no
human involved. A run ends in a landed change or an escalation (rounds, spend
or time ran out). The grader (grade_handsoff.py) scores the results afterwards
in a separate process.

Each task runs alone, in a fresh copy of its repo; the change lands when
hexa commits on a green gate.

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
  HANDSOFF_NOTIFY="<cmd>" ...   escalation hook, called with the payload path
"""
import argparse, hashlib, json, os, shlex, shutil, subprocess, sys
import time, tomllib
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


# ---------------------------------------------------------------------- main

def load_tasks(bench, only):
    out = []
    for task_dir in sorted(p.parent for p in Path(bench).glob("*/task.toml")):
        t = tomllib.loads((task_dir / "task.toml").read_text())
        t.setdefault("id", task_dir.name)
        if not only or t["id"] in only:
            out.append((task_dir, t))
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bench", default="bench/handsoff/tasks")
    ap.add_argument("--out", default="runs")
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--only", nargs="*", help="task ids to run")
    a = ap.parse_args()

    tasks = load_tasks(a.bench, a.only)
    if not tasks:
        sys.exit(f"no tasks under {a.bench}")
    out = Path(a.out)
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


if __name__ == "__main__":
    main()
