#!/usr/bin/env python3
"""A scripted stand-in for `hexa`, used only by bench/handsoff/selftest.py.

It lets the self-test drive every outcome the grader can assign without a
model in the loop. The task prompt is a list of directives separated by ';':

  write <path>=<content>   write a file (content has no spaces)
  sleep <seconds>          wait, to order agents in a parallel batch

Verbs it imitates, with the flags the runner uses:

  analyze . --grade A                        exit 1 if BAD_SHAPE exists, else 0
  do run "<prompt>" --file <f> --evidence "<cmd>" [--also <f>]...
  build "<prompt>" --target <dir> --gate "<cmd>"
  spend --json                               zero totals, in the real JSON shape
  init . --no-claude-md                      .hexa/ and a settings.json with a hexa hook

`do run` and `build` behave like the real verbs' contract: apply the edit, run
the gate, commit only if it exits 0, otherwise revert and exit non-zero.
"""
import json, subprocess, sys, time
from pathlib import Path


def sh(cmd):
    return subprocess.run(cmd, shell=True).returncode


def apply(prompt):
    for d in (p.strip() for p in prompt.split(";")):
        if d.startswith("write "):
            path, _, content = d[len("write "):].partition("=")
            Path(path).parent.mkdir(parents=True, exist_ok=True)
            Path(path).write_text(content + "\n")
        elif d.startswith("sleep "):
            time.sleep(float(d[len("sleep "):]))


def agent(prompt, gate):
    apply(prompt)
    if sh(gate) == 0:
        sh("git add -A && git commit -q --allow-empty -m 'hexa: task'")
        return 0
    sh("git checkout -q -- . && git clean -fdq")
    return 1


def flag(args, name):
    return args[args.index(name) + 1] if name in args else None


def main(a):
    if a[:1] == ["analyze"]:
        bad = Path("BAD_SHAPE").exists()
        print("architecture grade: " + ("D" if bad else "A"))
        return 1 if bad and "--grade" in a else 0
    if a[:1] == ["init"]:
        Path(".hexa").mkdir(exist_ok=True)
        Path(".hexa/project.json").write_text("{}\n")
        Path(".claude").mkdir(exist_ok=True)
        Path(".claude/settings.json").write_text(json.dumps({"hooks": {"SessionStart": [
            {"hooks": [{"type": "command", "command": "hexa hook session-start"}]}]}}) + "\n")
        return 0
    if a[:1] == ["spend"]:
        zero = {"calls": 0, "input_tokens": 0, "output_tokens": 0,
                "cost_usd": 0.0, "priced_calls": 0}
        print(json.dumps({"today": zero, "last_7_days": zero, "all": zero}))
        return 0
    if a[:2] == ["do", "run"]:
        return agent(a[2], flag(a, "--evidence"))
    if a[:1] == ["build"]:
        return agent(a[1], flag(a, "--gate"))
    print(f"fake_hexa: unsupported {a}", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
