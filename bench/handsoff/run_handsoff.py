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
                        A finished change goes through a merge queue: rebase
                        onto the latest main, rerun the gate and
                        `hexa analyze . --grade A`, and fast-forward main only
                        if both pass. A conflict or a failed re-gate sends the
                        task back a round.

Two agents, chosen per task with `agent` in task.toml:

  hexa (default)  the runner calls `hexa do run` / `hexa build` directly; a
                  round is one call, ready when it exits 0.
  claude-code     the runner launches `claude -p` headless in the worktree
                  with the task, its gate and the rule "use hexa verbs; done
                  means the gate exits 0 and hexa analyze --grade A passes".
                  A round is one session, capped by max_turns and
                  spend_cap_usd (--max-budget-usd); cost and tokens come from
                  its JSON result. The change is ready only if the gate exits
                  0 afterwards. Set CLAUDE_BIN to use another binary.

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
import argparse, hashlib, json, os, queue, shlex, shutil, subprocess, sys
import threading, time, tomllib
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

HEXA = os.environ.get("HEXA_BIN", "hexa")
CLAUDE = os.environ.get("CLAUDE_BIN", "claude")
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


def agent_kind(t):
    return t.get("agent", "hexa")


def claude_prompt(t, note=None):
    """The task, its gate, and the rule, as one headless prompt."""
    q = shlex.quote
    p = (f"{t['prompt']}\n\n"
         f"The gate for this task is: {t['gate']}\n\n"
         "Use hexa verbs. First record the gate with "
         f"`hexa loop gate {q(t['gate'])}`, then make the change with `hexa do run` "
         "or `hexa build`. Done means the gate exits 0 and "
         "`hexa analyze . --grade A` passes. Do not edit the gate's tests or any "
         "locked file.")
    if note:
        p += (f"\n\nYour previous attempt was sent back ({note}). The tree now "
              "includes everything that has landed on main since you started.")
    return p


def claude_cmd(t, note=None):
    """`claude -p`, headless. Flags checked against `claude --help` (2.1.318);
    --max-turns is accepted but not listed there."""
    q = shlex.quote
    return (f"{CLAUDE} -p {q(claude_prompt(t, note))} --output-format json"
            f" --max-turns {int(t.get('max_turns', 40))}"
            f" --max-budget-usd {float(t.get('spend_cap_usd', 2.0))}"
            " --permission-mode dontAsk --no-session-persistence")


def claude_usage(out):
    """Cost, tokens, turns and outcome from the session's JSON result."""
    for line in reversed(out.splitlines()):
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            j = json.loads(line)
        except ValueError:
            continue
        u = j.get("usage") or {}
        tokens = sum(int(u.get(k) or 0) for k in (
            "input_tokens", "output_tokens",
            "cache_creation_input_tokens", "cache_read_input_tokens"))
        return {"cost_usd": j.get("total_cost_usd"), "tokens": tokens,
                "turns": j.get("num_turns"), "subtype": j.get("subtype"),
                "is_error": j.get("is_error")}
    return {"cost_usd": None, "tokens": None, "turns": None,
            "subtype": "no_json", "is_error": True}


def run_round(t, wt, timeout, note=None):
    """One agent round in `wt`. Returns exit code, output, and whether the
    change is ready to land.

    hexa:        one `do run` / `build`; ready when it exits 0 (it commits).
    claude-code: one headless session; ready only if the gate exits 0
                 afterwards, whatever the session reported. Work the session
                 left uncommitted is committed so the queue has a change.
    """
    if agent_kind(t) != "claude-code":
        code, out = sh(agent_cmd(t), wt, timeout=timeout)
        return {"code": code, "out": out, "ready": code == 0}
    code, out = sh(claude_cmd(t, note), wt, timeout=timeout)
    usage = claude_usage(out)
    g, gout = sh(t["gate"], wt, timeout=1800)
    ready = g == 0
    if ready:
        _, dirty = sh("git status --porcelain", wt)
        if dirty.strip():
            sh("git add -A && git commit -qm 'bench: work the session left uncommitted'", wt)
    return {"code": code, "out": f"{out}\n--- gate exit {g} ---\n{gout}",
            "ready": ready, "usage": usage}


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


def caps(t, parallel=False):
    """(max rounds, cost cap USD, token cap or None, deadline).

    For claude-code a round is a whole session, already capped inside by
    max_turns and --max-budget-usd, so rounds count sessions: one by default,
    two in a batch so the merge queue can send a change back.
    """
    if agent_kind(t) == "claude-code":
        rounds = int(t.get("max_sessions", 2 if parallel else 1))
    else:
        rounds = int(t.get("max_rounds", 3))
    return (rounds, float(t.get("spend_cap_usd", 2.0)),
            t.get("token_cap"), time.time() + 60 * float(t.get("timeout_min", 30)))


def add_usage(acc, r):
    """Accumulate a claude-code round's cost and tokens into acc."""
    u = r.get("usage")
    if not u:
        return
    acc["sessions"].append(u)
    for k, field in (("cost", "cost_usd"), ("tokens", "tokens")):
        if u.get(field) is not None:
            acc[k] = (acc[k] or 0) + u[field]


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
    claude = agent_kind(t) == "claude-code"
    cost0, tok0, _ = (None, None, "") if claude else spend(work)
    acc = {"cost": None, "tokens": None, "sessions": []}
    max_rounds, cost_cap, token_cap, deadline = caps(t)
    start, rounds, last_out, success, stop = time.time(), [], "", False, "rounds"

    for i in range(max_rounds):
        remaining = deadline - time.time()
        if remaining <= 0:
            stop = "time"
            break
        r = run_round(t, work, remaining)
        last_out = r["out"]
        rounds.append(r["code"])
        add_usage(acc, r)
        (run_dir / f"round{i + 1}.log").write_text(last_out)
        if r["ready"]:
            success = True
            break
        if claude:
            cost, tok = acc["cost"], acc["tokens"]
        else:
            c, k, _ = spend(work)
            cost, tok = delta(cost0, c), delta(tok0, k)
        if (cost or 0) >= cost_cap or (token_cap and (tok or 0) >= token_cap):
            stop = "spend"
            break

    if claude:
        spend_usd, tokens, spend_raw = acc["cost"], acc["tokens"], ""
    else:
        cost1, tok1, spend_raw = spend(work)
        spend_usd, tokens = delta(cost0, cost1), delta(tok0, tok1)
    head1 = git_head(work)
    wall = round(time.time() - start, 1)
    result.update(
        agent=agent_kind(t), sessions=acc["sessions"],
        rounds=rounds, success=success, head0=head0, head1=head1,
        committed=bool(head1 and head1 != head0), landed=success,
        landed_at_s=wall if success else None, wall_s=wall,
        spend_usd=spend_usd, tokens=tokens,
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

class EventLog:
    """Append-only JSONL log of merge-queue events, shared by a batch's agents."""

    def __init__(self, path, t0):
        self.path, self.t0, self.lock = path, t0, threading.Lock()

    def __call__(self, task, event, **kw):
        rec = {"t_s": round(time.time() - self.t0, 3), "task": task, "event": event, **kw}
        with self.lock, open(self.path, "a") as f:
            f.write(json.dumps(rec) + "\n")


class MergeQueue:
    """Serialises landings onto main, first come first served.

    A finished change is rebased onto the latest main, then the gate and the
    architecture grade are rerun on the rebased tree. Main is fast-forwarded
    only if both pass. A conflict resets the agent's worktree to main; a failed
    re-gate leaves the rebased tree in place. Either way the agent gets the
    task back for another round.
    """

    def __init__(self, main, log):
        self.main, self.log, self.q = main, log, queue.Queue()
        threading.Thread(target=self._serve, daemon=True).start()

    def submit(self, t, wt):
        done, box = threading.Event(), {}
        self.log(t["id"], "submitted")
        self.q.put((t, wt, done, box))
        done.wait()
        return box

    def _serve(self):
        while True:
            t, wt, done, box = self.q.get()
            try:
                box.update(self._land(t, wt))
            except Exception as e:  # a queue that dies silently would hang every agent
                box.update(status="error", detail=repr(e))
                self.log(t["id"], "queue_error", detail=repr(e))
            done.set()

    def _land(self, t, wt):
        tid = t["id"]
        code, out = sh("git rebase main", wt, timeout=300)
        if code != 0:
            sh("git rebase --abort", wt)
            sh("git reset -q --hard main && git clean -fdq", wt)
            self.log(tid, "conflict", detail=out[-500:])
            return {"status": "conflict"}
        self.log(tid, "rebased")
        g, _ = sh(t["gate"], wt, timeout=1800)
        a, _ = sh(f"{HEXA} analyze . --grade A", wt, timeout=600)
        if g != 0 or a != 0:
            failed = "gate" if g != 0 else "grade"
            self.log(tid, "regate_failed", failed=failed)
            return {"status": "regate_failed", "failed": failed}
        branch = f"agent/{tid}"
        code, out = sh(f"git merge -q --ff-only {branch}", self.main, timeout=300)
        if code != 0:  # main moved under us; treat like a conflict
            sh("git reset -q --hard main && git clean -fdq", wt)
            self.log(tid, "conflict", detail="fast-forward refused: " + out[-300:])
            return {"status": "conflict"}
        self.log(tid, "landed", head=git_head(self.main))
        return {"status": "landed", "at_s": round(time.time() - self.log.t0, 3)}


def run_agent(task_dir, t, wt, run_dir, mq, log):
    """One agent's life in a batch: rounds of hexa, each green one through the queue."""
    result = base_result(task_dir, t, "parallel", run_dir.parent.name)
    # hexa's spend log is global, so concurrent agents can't be told apart:
    # rounds and time cap each agent; spend is recorded per batch.
    max_rounds, _cost_cap, _token_cap, deadline = caps(t, parallel=True)
    rounds, conflicts, regates, landed_at, last_out, stop = [], 0, 0, None, "", "rounds"
    acc, note = {"cost": None, "tokens": None, "sessions": []}, None
    start = time.time()
    while len(rounds) < max_rounds:
        remaining = deadline - time.time()
        if remaining <= 0:
            stop = "time"
            break
        r = run_round(t, wt, remaining, note)
        last_out = r["out"]
        rounds.append(r["code"])
        add_usage(acc, r)
        (run_dir / f"round{len(rounds)}.log").write_text(last_out)
        log(t["id"], "round", round=len(rounds), exit=r["code"], ready=r["ready"])
        if not r["ready"]:
            note = "the gate did not exit 0"
            continue
        verdict = mq.submit(t, wt)
        if verdict.get("status") == "landed":
            landed_at = verdict["at_s"]
            break
        if verdict.get("status") == "conflict":
            conflicts += 1
            note = "a rebase conflict with main; your worktree was reset to main"
        elif verdict.get("status") == "regate_failed":
            regates += 1
            note = f"after rebasing onto main, the {verdict.get('failed')} check failed"
        else:
            stop = "queue_error"
            break
        log(t["id"], "sent_back", reason=verdict.get("status"))
    landed = landed_at is not None
    result.update(agent=agent_kind(t), sessions=acc["sessions"],
                  rounds=rounds, success=landed, landed=landed, committed=landed,
                  landed_at_s=landed_at, wall_s=round(time.time() - start, 1),
                  conflicts=conflicts, regate_failures=regates,
                  # claude-code reports its own cost; hexa's spend log is global
                  spend_usd=acc["cost"], tokens=acc["tokens"],
                  stop_reason=None if landed else stop)
    if not landed:
        log(t["id"], "escalated", reason=stop)
    finish(run_dir, t, result, last_out, stop, landed)
    return result


def run_batch(name, members, batch_dir, workers):
    """Run one batch: shared main, a worktree per agent, a merge queue in front of main."""
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
    log = EventLog(batch_dir / "events.jsonl", t_start)
    mq = MergeQueue(main, log)
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
        r = run_agent(task_dir, t, wt, rd, mq, log)
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
        "spend_usd": delta(cost0, cost1), "tokens": delta(tok0, tok1),  # hexa's log
        "agent_spend_usd": round(sum(r["spend_usd"] or 0 for r in results), 4),
        "agent_tokens": sum(r["tokens"] or 0 for r in results),
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
                    help="run each batch N agents at a time through a merge queue")
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
