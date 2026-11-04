#!/usr/bin/env python3
"""Hands-off agent workflow runner for hexa.

For every task in bench/handsoff/<task>/task.toml, run the agent K times in a
fresh copy of the repo with no human involved. A run ends in a commit (gate
exited 0) or an escalation (rounds, spend or time ran out). The grader
(grade_handsoff.py) scores the results afterwards in a separate process.

Assumes the hexa verbs shown in its README:
  hexa analyze . --grade A
  hexa do run "<prompt>" --file <f> --evidence "<gate>"
  hexa build "<prompt>" --target <dir> --gate "<gate>"
  hexa spend
Check them against `hexa --help` before the first run.

Usage:
  python3 run_handsoff.py --bench bench/handsoff --out runs --runs 3
  HANDSOFF_NOTIFY="./notify.sh" python3 run_handsoff.py ...   # escalation hook
"""
import argparse, hashlib, json, os, re, shutil, subprocess, sys, time, tomllib
from pathlib import Path


def sh(cmd, cwd, timeout=None):
    """Run a shell command, return (exit_code, combined_output)."""
    try:
        p = subprocess.run(cmd, cwd=cwd, shell=True, capture_output=True,
                           text=True, timeout=timeout)
        return p.returncode, p.stdout + p.stderr
    except subprocess.TimeoutExpired as e:
        out = (e.stdout or "") + (e.stderr or "")
        if isinstance(out, bytes):
            out = out.decode(errors="replace")
        return 124, out + "\n[timeout]"


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        h.update(f.read())
    return h.hexdigest()


def hash_locked(work, locked):
    """Hash every locked file (globs allowed)."""
    hashes = {}
    for pattern in locked:
        for p in sorted(work.glob(pattern)):
            if p.is_file():
                hashes[str(p.relative_to(work))] = sha256(p)
    return hashes


def spend_usd(work):
    """Best-effort read of total spend from `hexa spend` (format unverified)."""
    code, out = sh("hexa spend", work, timeout=60)
    m = re.search(r"\$\s*([0-9]+(?:\.[0-9]+)?)", out)
    return (float(m.group(1)) if m else None), out


def git_head(work):
    code, out = sh("git rev-parse HEAD", work)
    return out.strip() if code == 0 else None


def prepare(task_dir, work):
    if work.exists():
        shutil.rmtree(work)
    shutil.copytree(task_dir / "repo", work)  # holdout/ is never copied
    if not (work / ".git").exists():
        sh("git init -q && git add -A && git -c user.name=bench "
           "-c user.email=bench@local commit -qm floor", work)


def agent_cmd(t):
    gate = t["gate"].replace('"', '\\"')
    prompt = t["prompt"].replace('"', '\\"')
    if t.get("mode", "do") == "do":
        return f'hexa do run "{prompt}" --file {t["file"]} --evidence "{gate}"'
    return f'hexa build "{prompt}" --target {t.get("target", ".")} --gate "{gate}"'


def escalate(run_dir, t, payload):
    """Fire the notifier if configured. Returns True if it reported success."""
    (run_dir / "escalation.json").write_text(json.dumps(payload, indent=2))
    hook = os.environ.get("HANDSOFF_NOTIFY")
    if not hook:
        return False
    code, out = sh(f'{hook} "{run_dir / "escalation.json"}"', run_dir, timeout=60)
    (run_dir / "notify.log").write_text(out)
    return code == 0


def run_once(task_dir, t, run_dir):
    work = run_dir / "work"
    prepare(task_dir, work)
    result = {"task": t["id"], "category": t["category"],
              "solvable": t.get("solvable", True), "run": run_dir.name}

    code, out = sh("hexa analyze . --grade A", work, timeout=300)
    if code != 0:
        result.update(outcome_hint="invalid_floor", floor_log=out[-4000:])
        return result

    result["locked_before"] = hash_locked(work, t.get("locked", []))
    head0 = git_head(work)
    spend0, _ = spend_usd(work)
    max_rounds = int(t.get("max_rounds", 3))
    cap = float(t.get("spend_cap_usd", 2.0))
    deadline = time.time() + 60 * float(t.get("timeout_min", 30))
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
        now, _ = spend_usd(work)
        if spend0 is not None and now is not None and now - spend0 >= cap:
            stop = "spend"
            break

    spend1, spend_raw = spend_usd(work)
    head1 = git_head(work)
    result.update(
        rounds=rounds, success=success, head0=head0, head1=head1,
        committed=bool(head1 and head1 != head0),
        wall_s=round(time.time() - start, 1),
        spend_usd=(round(spend1 - spend0, 4)
                   if spend0 is not None and spend1 is not None else None),
        spend_raw=spend_raw[-2000:], stop_reason=None if success else stop,
    )
    if result["committed"]:
        _, diff = sh(f"git diff {head0} {head1}", work)
        (run_dir / "final.diff").write_text(diff)

    if not success:
        result["escalated"] = True
        result["notified"] = escalate(run_dir, t, {
            "task": t["id"], "prompt": t["prompt"], "gate": t["gate"],
            "stop_reason": stop, "rounds": rounds,
            "last_output_tail": last_out[-3000:], "spend_usd": result["spend_usd"],
            "loop_json": str(work / ".hexa" / "loop.json"),
        })
    else:
        result["escalated"] = False
        result["notified"] = False
    return result


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bench", default="bench/handsoff")
    ap.add_argument("--out", default="runs")
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--only", nargs="*", help="task ids to run")
    a = ap.parse_args()

    bench, out = Path(a.bench), Path(a.out)
    tasks = sorted(p.parent for p in bench.glob("*/task.toml"))
    if not tasks:
        sys.exit(f"no tasks under {bench}")
    for task_dir in tasks:
        t = tomllib.loads((task_dir / "task.toml").read_text())
        t.setdefault("id", task_dir.name)
        if a.only and t["id"] not in a.only:
            continue
        for k in range(1, a.runs + 1):
            run_dir = out / t["id"] / f"run{k}"
            run_dir.mkdir(parents=True, exist_ok=True)
            print(f"[{t['id']} run{k}] ...", flush=True)
            r = run_once(task_dir, t, run_dir)
            (run_dir / "result.json").write_text(json.dumps(r, indent=2))
            state = ("invalid floor" if r.get("outcome_hint") else
                     "committed" if r.get("committed") else "escalated")
            print(f"[{t['id']} run{k}] {state}", flush=True)


if __name__ == "__main__":
    main()
