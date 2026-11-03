#!/usr/bin/env python3
"""Blank-slate A/B: build a URL shortener from a behaviour-only spec, then change it.

    python3 -I bench/ab/greenfield/run_greenfield.py --arm A --model <model id> --run 1

Arms:
  A  no hexa at all. `claude -p` gets SPEC.md and gate.sh in an empty folder and
     chooses its own design. Nothing it reads names an architecture.
  B  the same SPEC.md, built through `hexa scaffold --lang ts --grade A`, which
     brings hexa's architecture and its two gates.

Round 2 is the same in both arms: the agent alone gets CHANGE.md (swap file
storage for SQLite) in the codebase its arm produced. It tests the design, not
the tool: what the change costs and how much of the code it touches.

Judged with design-neutral instruments only: the black-box gate, the hidden
defect probes in bench/probes, tokens and dollars, lines of code, cyclomatic
complexity, and the change's blast radius. hexa's grade is recorded for
information and is never a pass criterion here.
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
AB = HERE.parent
REPO = AB.parent.parent
SHIM_DIR = AB / "bin"
HEXA = os.environ.get("HEXA_AB_HEXA", str(REPO / "target/release/hexa"))
GATE = REPO / "bench/gate.sh"
PROBES = sorted((REPO / "bench/probes").glob("p*.sh"))
# The same tools in both arms: hexa's TypeScript skeleton builds with npm, tsc and node.
BUILD_TOOLS = ("Edit,Write,Bash(bun:*),Bash(npm:*),Bash(npx:*),Bash(node:*),Bash(./gate.sh:*),"
               "Bash(bash gate.sh:*),Bash(mkdir:*),Bash(ls:*)")


def sh(cmd, cwd, timeout=None, env=None):
    t0 = time.time()
    try:
        p = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=timeout, env=env,
                           stdin=subprocess.DEVNULL)
        return p.returncode, p.stdout, p.stderr, time.time() - t0, False
    except subprocess.TimeoutExpired as e:
        out = e.stdout.decode() if isinstance(e.stdout, bytes) else (e.stdout or "")
        return 124, out, "timeout", time.time() - t0, True


def git(ws, *args):
    return sh(["git", "-c", "user.name=ab", "-c", "user.email=ab@local", *args], cwd=ws)[1]


def snapshot(ws, msg):
    git(ws, "add", "-A")
    git(ws, "commit", "-qm", msg, "--allow-empty")
    return git(ws, "rev-parse", "HEAD").strip()


def calls_in(log):
    if not log.exists():
        return []
    return [json.loads(l) for l in log.read_text().splitlines() if l.strip()]


def cost_of(calls, wall):
    return {
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


def source_files(app):
    """TypeScript the program is made of: under src/, not tests, not dependencies."""
    out = []
    for p in sorted((app / "src").rglob("*.ts")) if (app / "src").exists() else []:
        rel = str(p.relative_to(app))
        if "node_modules" in rel or re.search(r"(\.test|\.spec)\.ts$|(^|/)(test|tests|__tests__)/", rel):
            continue
        out.append(rel)
    return out


def test_files(app):
    return [str(p.relative_to(app)) for p in sorted(app.rglob("*.ts"))
            if "node_modules" not in str(p) and re.search(r"(\.test|\.spec)\.ts$|(^|/)(test|tests|__tests__)/", str(p.relative_to(app)))]


def nloc(app, files):
    n = 0
    for f in files:
        for l in (app / f).read_text(errors="replace").splitlines():
            s = l.strip()
            if s and not s.startswith(("//", "*", "/*")):
                n += 1
    return n


def complexity(app, files):
    if not files:
        return {"functions": 0, "ccn_mean": None, "ccn_max": None, "fn_len_max": None}
    _, out, _, _, _ = sh([sys.executable, "-m", "lizard", "--csv", *files], cwd=app)
    fns = []
    for line in out.splitlines():
        c = [x.strip('"') for x in line.split(",")]
        if len(c) > 4 and c[0].isdigit():
            fns.append((int(c[0]), int(c[1])))
    ccn = [f[1] for f in fns]
    return {"functions": len(fns), "ccn_mean": round(sum(ccn) / len(ccn), 2) if ccn else None,
            "ccn_max": max(ccn) if ccn else None, "fn_len_max": max(f[0] for f in fns) if fns else None}


def measure(app, env):
    """Everything about the program as it stands, by instruments that ignore its design."""
    g_code, g_out, g_err, _, _ = sh(["bash", str(GATE), str(app)], cwd=app, timeout=300, env=env)
    probes = {}
    for p in PROBES:
        code, out, _, _, _ = sh(["bash", str(p), str(app)], cwd=app, timeout=180, env=env)
        probes[p.stem] = {0: "pass", 1: "defect", 2: "skip"}.get(code, f"exit {code}")
    t_code, t_out, t_err, _, _ = sh(["bun", "test"], cwd=app, timeout=300, env=env)
    has_tests = bool(test_files(app))
    _, a_out, _, _, _ = sh([HEXA, "analyze", ".", "--json"], cwd=app, timeout=300)
    try:
        aj = json.loads(a_out)
        v = aj.get("violations")
        grade = {"grade": aj.get("grade"), "score": aj.get("score"),
                 "violations": len(v) if isinstance(v, list) else v}
    except ValueError:
        grade = {"grade": None, "score": None, "violations": None}
    src, tests = source_files(app), test_files(app)
    return {
        "gate_pass": g_code == 0,
        "gate_failures": [l.strip() for l in (g_out + g_err).splitlines() if l.strip().startswith("FAIL")][:10],
        "probes": probes,
        "defects": sum(1 for v in probes.values() if v == "defect"),
        "own_tests": {"present": has_tests, "pass": (t_code == 0) if has_tests else None},
        "src_files": len(src), "src_nloc": nloc(app, src),
        "test_files": len(tests), "test_nloc": nloc(app, tests),
        "complexity": complexity(app, src),
        "hexa_grade_info_only": grade,
    }


def blast_radius(ws, base, app_rel):
    num = git(ws, "diff", "--numstat", base, "--", app_rel)
    added = removed = 0
    files = []
    for line in num.splitlines():
        parts = line.split("\t")
        if len(parts) == 3 and parts[0].isdigit():
            added += int(parts[0]); removed += int(parts[1]); files.append(parts[2])
    src_files = [f for f in files if f.startswith(f"{app_rel}/src/") and f.endswith(".ts")
                 and not re.search(r"(\.test|\.spec)\.ts$", f)]
    return {"lines_added": added, "lines_removed": removed, "files_touched": len(files),
            "src_files_touched": len(src_files), "files": files}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--arm", required=True, choices=["A", "B"])
    ap.add_argument("--model", required=True)
    ap.add_argument("--run", type=int, default=1)
    ap.add_argument("--timeout", type=int, default=3600, help="wall-clock cap per round, seconds")
    ap.add_argument("--out", default=str(HERE / "runs"))
    a = ap.parse_args()

    root = Path(tempfile.mkdtemp(prefix=f"hexa-ab-greenfield-{a.arm}-"))
    ws = root / "ws"
    ws.mkdir()
    app_rel = "app"
    spec = (HERE / "SPEC.md").read_text()
    change = (HERE / "CHANGE.md").read_text()
    (ws / ".gitignore").write_text("node_modules/\ndist/\n")
    git(ws, "init", "-q")
    base0 = snapshot(ws, "empty")

    def env_for(log, tools):
        env = dict(os.environ)
        env.update({"PATH": f"{SHIM_DIR}:{env['PATH']}", "HEXA_AB_LOG": str(log),
                    "HEXA_AB_MODEL": a.model, "HEXA_CLAUDE_BINARY": str(SHIM_DIR / "claude"),
                    "HEXA_AB_ALLOWED_TOOLS": tools})
        return env

    # ── Round 1: build ──────────────────────────────────────────────────────
    log1 = root / "calls-build.jsonl"
    app = ws / app_rel
    app.mkdir()
    shutil.copy(GATE, app / "gate.sh")
    (app / "SPEC.md").write_text(spec)
    if a.arm == "A":
        env1 = env_for(log1, BUILD_TOOLS)
        prompt = spec + "\nThe spec is also in SPEC.md, and gate.sh is in this directory.\n"
        cmd, cwd = ["claude", "-p", "--model", a.model, prompt], app
    else:
        env1 = env_for(log1, BUILD_TOOLS + f",Bash({HEXA}:*)")
        desc = spec + "\nSPEC.md and gate.sh are already in the project directory. Do not edit gate.sh.\n"
        cmd = [HEXA, "scaffold", desc, "--target", app_rel, "--lang", "ts", "--grade", "A"]
        cwd = ws
    code1, out1, err1, wall1, to1 = sh(cmd, cwd=cwd, timeout=a.timeout, env=env1)
    (root / "build-stdout.txt").write_text(out1)
    (root / "build-stderr.txt").write_text(err1)
    app.mkdir(exist_ok=True)
    shutil.copy(GATE, app / "gate.sh")  # the same gate in both arms, whatever the build did
    (app / "SPEC.md").write_text(spec)
    base1 = snapshot(ws, "built")
    m1 = measure(app, env1)
    calls1 = calls_in(log1)

    # ── Round 2: the change, by the agent alone in both arms ───────────────
    log2 = root / "calls-change.jsonl"
    env2 = env_for(log2, BUILD_TOOLS)
    (app / "CHANGE.md").write_text(change)
    base2 = snapshot(ws, "change request")
    code2, out2, err2, wall2, to2 = sh(
        ["claude", "-p", "--model", a.model,
         change + "\nThe change request is also in CHANGE.md. The original spec is in SPEC.md.\n"],
        cwd=app, timeout=a.timeout, env=env2)
    (root / "change-stdout.txt").write_text(out2)
    (root / "change-stderr.txt").write_text(err2)
    shutil.copy(GATE, app / "gate.sh")  # never the agent's edit of it
    m2 = measure(app, env2)
    br = blast_radius(ws, base2, app_rel)
    snapshot(ws, "changed")
    calls2 = calls_in(log2)
    total_src = max(m1["src_nloc"], 1)

    sqlite_used = any("bun:sqlite" in (app / f).read_text(errors="replace") for f in source_files(app))
    all_calls = calls1 + calls2
    models_used = sorted({m for c in all_calls for m in c["models_used"]})
    void = []
    if any(c["model_mismatch"] for c in all_calls):
        void.append("a call asked for a different model")
    others = [m for m in models_used if m != a.model]
    if others:
        void.append(f"other models served calls: {others}")
    if any(not c.get("sandboxed") for c in all_calls):
        void.append("a call ran outside the sandbox")
    if any(not c["metered"] for c in all_calls):
        void.append("unmetered calls")
    if not calls1:
        void.append("no model call in the build")

    rec = {
        "task": "url-shortener", "family": "greenfield", "arm": a.arm, "model": a.model, "run": a.run,
        "build": {"agent_exit": code1, "timed_out": to1, "cost": cost_of(calls1, wall1), **m1},
        "change": {"agent_exit": code2, "timed_out": to2, "cost": cost_of(calls2, wall2), **m2,
                   "uses_bun_sqlite": sqlite_used, "blast_radius": br,
                   "share_of_src_changed": round((br["lines_added"] + br["lines_removed"]) / total_src, 2)},
        "accepted": {
            "build": m1["gate_pass"],
            "change": m2["gate_pass"] and sqlite_used,
            "both": m1["gate_pass"] and m2["gate_pass"] and sqlite_used,
        },
        "models_used": models_used,
        "void": void,
        "workspace": str(root),
    }
    out_dir = Path(a.out) / a.model / a.arm
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / f"url-shortener--r{a.run}.json").write_text(json.dumps(rec, indent=2))
    print(f"{a.model:<22} {a.arm} r{a.run}  build gate={'PASS' if m1['gate_pass'] else 'FAIL'} "
          f"defects={m1['defects']} ${rec['build']['cost']['usd']:.2f}  |  change gate="
          f"{'PASS' if m2['gate_pass'] else 'FAIL'} sqlite={sqlite_used} lines={br['lines_added'] + br['lines_removed']} "
          f"${rec['change']['cost']['usd']:.2f}{'  VOID: ' + '; '.join(void) if void else ''}")


if __name__ == "__main__":
    main()
