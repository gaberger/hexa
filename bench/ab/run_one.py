#!/usr/bin/env python3
"""Run one task, in one arm, on one model, and write one evidence record.

    python3 -I bench/ab/run_one.py --task header-safe-url --arm A --model <model id> --run 1

Arms (same prompt text in every arm):
  A  the agent alone: `claude -p` in a fresh workspace
  B  hexa: `hexa do run` in the same workspace, with the same gate, on the same model
  C  arm A plus the project's ADR-rules.toml pasted into the prompt, with no gates
  0  self-test, no agent: the untouched starting code, which every oracle must fail

The oracle is never written into the agent's workspace. It is applied afterwards
to a copy. Every `claude` call in every arm goes through bin/claude, which pins
the model and meters tokens and dollars.
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent  # this hexa checkout
SHIM_DIR = HERE / "bin"
DEFAULT_FIXTURES = os.environ.get("HEXA_AB_FIXTURES", str(REPO / "docs/benchmarks/fixtures"))
DEFAULT_RULES = os.environ.get("HEXA_AB_RULES", str(REPO / "hexa-cli/assets/templates/ADR-rules.toml"))
HEXA = os.environ.get("HEXA_AB_HEXA", str(REPO / "target/release/hexa"))
# Models allowed to serve calls besides the pinned one, e.g. the CLI's own helper
# model for titles and summaries. Empty by default: any other model voids the run.
HELPER_MODELS = [m for m in os.environ.get("HEXA_AB_HELPER_MODELS", "").split(",") if m]
GRADES = ["F", "D", "C", "B", "A", "A+"]
# Strings whose appearance in an agent transcript means it went looking for the answer.
LEAK_MARKERS = ["benchmarks/fixtures", "oracle.test", "setup_files"]


def sh(cmd, cwd, timeout=None, env=None):
    t0 = time.time()
    try:
        p = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=timeout, env=env,
                           stdin=subprocess.DEVNULL)
        return p.returncode, p.stdout, p.stderr, time.time() - t0, False
    except subprocess.TimeoutExpired as e:
        out = e.stdout.decode() if isinstance(e.stdout, bytes) else (e.stdout or "")
        return 124, out, "timeout", time.time() - t0, True


def load_task(fixtures, task_id):
    fx = json.loads((Path(fixtures) / f"{task_id}.json").read_text())
    target = fx["target_file"]
    oracle_files = {k: v for k, v in fx["oracle"]["setup_files"].items() if k != target}
    test_file = str(Path(target).with_name("subject.test.ts"))
    return {
        "id": fx["id"],
        "family": "change",
        # The fixtures are lone files with no layers: hexa grades every one F 0,
        # so the architecture gate does not apply to this family.
        "shape_applies": False,
        "instruction": fx["instruction"],
        "target": target,
        "start": fx["oracle"]["setup_files"][target],
        "test_file": test_file,
        "gate": f"bun test {test_file}",
        "oracle_files": oracle_files,
        "oracle_cmd": fx["oracle"]["command"],
    }


def prompt_for(task, arm, rules_text):
    p = (f"{task['instruction']}\n\n"
         f"Edit {task['target']}. You may add or change tests in {task['test_file']}.\n"
         f"You are done when this command exits 0: {task['gate']}\n")
    if arm == "C":
        p += ("\nFollow these architecture rules:\n```toml\n" + rules_text + "\n```\n")
    return p


def bun_counts(text):
    pa = re.search(r"(\d+) pass", text)
    fa = re.search(r"(\d+) fail", text)
    return (int(pa.group(1)) if pa else 0), (int(fa.group(1)) if fa else 0)


def lizard_metrics(ws, paths):
    paths = [p for p in paths if (ws / p).exists()]
    if not paths:
        return {"nloc": 0, "functions": 0, "ccn_mean": None, "ccn_max": None, "fn_len_max": None}
    code, out, _, _, _ = sh([sys.executable, "-m", "lizard", "--csv"] + paths, cwd=ws)
    fns = []
    for line in out.splitlines():
        cols = [c.strip('"') for c in line.split(",")]
        if len(cols) >= 4 and cols[0].isdigit():
            fns.append((int(cols[0]), int(cols[1]), int(cols[4]) if len(cols) > 4 and cols[4].isdigit() else 0))
    nloc_file = 0
    for p in paths:  # file NLOC: non-blank, non-comment-only lines
        for l in (ws / p).read_text(errors="replace").splitlines():
            s = l.strip()
            if s and not s.startswith("//") and not s.startswith("*") and not s.startswith("/*"):
                nloc_file += 1
    ccns = [f[1] for f in fns]
    return {
        "nloc": nloc_file,
        "functions": len(fns),
        "ccn_mean": round(sum(ccns) / len(ccns), 2) if ccns else None,
        "ccn_max": max(ccns) if ccns else None,
        "fn_len_max": max(f[0] for f in fns) if fns else None,
    }


def grade_of(ws):
    code, out, _, _, _ = sh([HEXA, "analyze", ".", "--json"], cwd=ws, timeout=300)
    try:
        j = json.loads(out)
    except ValueError:
        return {"grade": None, "score": None, "violations": None, "gradeable": False}
    v = j.get("violations")
    nviol = len(v) if isinstance(v, list) else v
    return {"grade": j.get("grade"), "score": j.get("score"), "violations": nviol,
            "gradeable": bool(j.get("layer_inventory"))}


def run_transcripts(run_root):
    """This run's own agent transcripts: the CLI files each session under a folder
    named for its working directory, and every agent here works under run_root.
    Other sessions' transcripts (including whoever launched the bench) are not read."""
    projects = Path.home() / ".claude" / "projects"
    slug = re.sub(r"[^A-Za-z0-9]", "-", str(run_root))
    if not projects.exists():
        return []
    return [p for d in projects.iterdir() if d.is_dir() and d.name.startswith(slug)
            for p in d.rglob("*.jsonl")]


def leak_markers(run_root):
    """Oracle material named in this run's transcripts, outside the prompt."""
    found = set()
    for t in run_transcripts(run_root):
        body = t.read_text(errors="replace")
        found |= {m for m in LEAK_MARKERS if m in body}
    return sorted(found)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--task", required=True)
    ap.add_argument("--arm", required=True, choices=["0", "A", "B", "C"])
    ap.add_argument("--model", required=True)
    ap.add_argument("--run", type=int, default=1)
    ap.add_argument("--timeout", type=int, default=900, help="wall-clock cap per run, seconds")
    ap.add_argument("--fixtures", default=DEFAULT_FIXTURES)
    ap.add_argument("--rules", default=DEFAULT_RULES)
    ap.add_argument("--out", default=str(HERE / "runs"))
    ap.add_argument("--apply", help="arm 0 only: copy this file over the target first (positive control)")
    a = ap.parse_args()

    task = load_task(a.fixtures, a.task)
    rules_text = Path(a.rules).read_text()
    root = Path(tempfile.mkdtemp(prefix=f"hexa-ab-{a.task}-{a.arm}-"))
    ws = root / "ws"
    ws.mkdir()
    (ws / task["target"]).parent.mkdir(parents=True, exist_ok=True)
    (ws / task["target"]).write_text(task["start"])
    for c in (["git", "init", "-q"], ["git", "add", "-A"],
              ["git", "-c", "user.name=ab", "-c", "user.email=ab@local", "commit", "-qm", "base"]):
        sh(c, cwd=ws)
    base = sh(["git", "rev-parse", "HEAD"], cwd=ws)[1].strip()

    log = root / "claude-calls.jsonl"
    env = dict(os.environ)
    env.update({"PATH": f"{SHIM_DIR}:{env['PATH']}", "HEXA_AB_LOG": str(log), "HEXA_AB_MODEL": a.model,
                "HEXA_CLAUDE_BINARY": str(SHIM_DIR / "claude")})  # hexa's frontier path, too
    prompt = prompt_for(task, a.arm, rules_text)

    t0 = time.time()
    if a.arm == "0":  # self-test: no agent; the untouched start must FAIL the oracle
        if a.apply:
            shutil.copy(a.apply, ws / task["target"])
        cmd = ["true"]
    elif a.arm in ("A", "C"):
        cmd = ["claude", "-p", "--model", a.model, prompt]  # bin/claude adds the sandbox
    else:
        cmd = [HEXA, "do", "run", prompt, "--file", task["target"], "--evidence", task["gate"],
               "--model", a.model, "--also", task["test_file"]]
    code, out, err, wall, timed_out = sh(cmd, cwd=ws, timeout=a.timeout, env=env)

    # Own gate, as the agent left it.
    g_code, g_out, g_err, _, _ = sh(["bash", "-lc", task["gate"]], cwd=ws, timeout=120)
    own_pass = g_code == 0

    # Oracle, on a copy the agent never touched.
    oc = root / "oracle"
    shutil.copytree(ws, oc, ignore=shutil.ignore_patterns(".git"))
    for rel, body in task["oracle_files"].items():
        (oc / rel).parent.mkdir(parents=True, exist_ok=True)
        (oc / rel).write_text(body)
    o_code, o_out, o_err, _, _ = sh(["bash", "-lc", task["oracle_cmd"]], cwd=oc, timeout=120)
    o_pass, o_fail = bun_counts(o_out + o_err)

    # Size and complexity of what was left behind.
    sh(["git", "add", "-A"], cwd=ws)
    numstat = sh(["git", "diff", "--cached", "--numstat", base], cwd=ws)[1]
    added = removed = files_changed = 0
    for line in numstat.splitlines():
        parts = line.split("\t")
        if len(parts) == 3 and parts[0].isdigit():
            added += int(parts[0]); removed += int(parts[1]); files_changed += 1
    src = lizard_metrics(ws, [task["target"]])
    tests = lizard_metrics(ws, [task["test_file"]])
    shape = grade_of(ws)
    if not task["shape_applies"]:
        shape["gradeable"] = False

    # Cost, and whether the model stayed pinned.
    calls = [json.loads(l) for l in log.read_text().splitlines()] if log.exists() else []
    models_used = sorted({m for c in calls for m in c["models_used"]})
    cost = {
        "calls": len(calls),
        "unmetered_calls": sum(1 for c in calls if not c["metered"]),
        "input_tokens": sum(c["input_tokens"] for c in calls),
        "output_tokens": sum(c["output_tokens"] for c in calls),
        "cache_read_tokens": sum(c["cache_read_tokens"] for c in calls),
        "cache_write_tokens": sum(c["cache_write_tokens"] for c in calls),
        "usd": round(sum(c["cost_usd"] or 0 for c in calls), 4),
        "wall_s": round(wall, 1),
        "permission_denials": sum(c.get("permission_denials", 0) for c in calls),
    }

    leaks = leak_markers(root)

    void = []
    if any(c["model_mismatch"] for c in calls):
        void.append("a call asked for a different model")
    others = [m for m in models_used if m != a.model and m not in HELPER_MODELS]
    if others:
        void.append(f"other models served calls: {others}")
    if leaks:
        void.append(f"transcript touched oracle material: {leaks}")
    if any(not c.get("sandboxed") for c in calls):
        void.append("a call ran outside the sandbox")
    if calls and cost["unmetered_calls"]:
        void.append("unmetered calls")
    if not calls and a.arm != "0":
        void.append("no model call was recorded")

    oracle_ok = o_code == 0 and o_fail == 0 and o_pass > 0
    grade_ok = (shape["grade"] in GRADES and GRADES.index(shape["grade"]) >= GRADES.index("A")) if shape["gradeable"] else None
    viol_ok = (shape["violations"] == 0) if shape["gradeable"] else None
    shape_ok = None if not shape["gradeable"] else (grade_ok and viol_ok)
    rec = {
        "task": task["id"], "family": task["family"], "arm": a.arm, "model": a.model, "run": a.run,
        "started": t0, "timed_out": timed_out, "agent_exit": code,
        "accuracy": {
            "oracle_pass": oracle_ok, "oracle_tests_passed": o_pass, "oracle_tests_failed": o_fail,
            "own_gate_pass": own_pass,
            "false_green_correctness": own_pass and not oracle_ok,
            "false_green_shape": own_pass and shape_ok is False,
            "accepted_correct": oracle_ok and shape_ok is not False,
        },
        "shape": shape,
        "loc": {"src_nloc": src["nloc"], "test_nloc": tests["nloc"],
                "test_to_src": round(tests["nloc"] / src["nloc"], 2) if src["nloc"] else None,
                "lines_added": added, "lines_removed": removed, "files_changed": files_changed},
        "complexity": {"functions": src["functions"], "ccn_mean": src["ccn_mean"],
                       "ccn_max": src["ccn_max"], "fn_len_max": src["fn_len_max"]},
        "cost": cost,
        "models_used": models_used,
        "void": void,
        "workspace": str(root),
    }
    if HELPER_MODELS:
        rec["helper_models_allowed"] = HELPER_MODELS
    out_dir = Path(a.out) / a.model / a.arm
    out_dir.mkdir(parents=True, exist_ok=True)
    path = out_dir / f"{task['id']}--r{a.run}.json"
    path.write_text(json.dumps(rec, indent=2))
    (root / "agent-stdout.txt").write_text(out)
    (root / "agent-stderr.txt").write_text(err)
    print(f"{a.model:<22} {a.arm} {task['id']:<20} r{a.run}  oracle={'PASS' if oracle_ok else 'FAIL'} "
          f"own={'PASS' if own_pass else 'FAIL'}  ${cost['usd']:.3f}  {cost['wall_s']}s"
          f"{'  VOID: ' + '; '.join(void) if void else ''}")


if __name__ == "__main__":
    main()
