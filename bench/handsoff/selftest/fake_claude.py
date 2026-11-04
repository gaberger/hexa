#!/usr/bin/env python3
"""A scripted stand-in for `claude -p`, used only by bench/handsoff/selftest.py.

It reads the task's directives from the first paragraph of the prompt (the
same `write <path>=<content>` / `sleep <s>` language as fake_hexa.py), applies
them, and leaves the edit uncommitted, the way a session that edits files
directly would. The runner must then run the gate and commit the work itself.

Before doing anything it checks what the runner is supposed to guarantee, and
exits 2 if any of it is missing, so a wiring regression fails the self-test:

- the headless flags: -p, --output-format json, --max-turns,
  --max-budget-usd, --permission-mode dontAsk
- the prompt carries the gate and the rule
- .claude/settings.local.json exists and denies edits to .claude/

It prints a result in the shape of `claude -p --output-format json`, costing
$0.01 a session, so the runner's cost and token accounting can be checked.
"""
import json, sys, time
from pathlib import Path


def fail(msg):
    print(f"fake_claude: {msg}", file=sys.stderr)
    sys.exit(2)


def main(a):
    if "-p" not in a:
        fail("not run headless (-p)")
    prompt = a[a.index("-p") + 1]
    need = {"--output-format": "json", "--permission-mode": "dontAsk"}
    for flag, value in need.items():
        if flag not in a or a[a.index(flag) + 1] != value:
            fail(f"missing {flag} {value}")
    for flag in ("--max-turns", "--max-budget-usd"):
        if flag not in a:
            fail(f"missing {flag}")
    if "The gate for this task is:" not in prompt or "hexa analyze . --grade A" not in prompt:
        fail("prompt lacks the gate or the rule")
    settings = Path(".claude/settings.local.json")
    if not settings.exists() or "Edit(/.claude/**)" not in settings.read_text():
        fail("no deny rules in .claude/settings.local.json")

    for d in (p.strip() for p in prompt.split("\n\n", 1)[0].split(";")):
        if d.startswith("write "):
            path, _, content = d[len("write "):].partition("=")
            Path(path).parent.mkdir(parents=True, exist_ok=True)
            Path(path).write_text(content + "\n")
        elif d.startswith("sleep "):
            time.sleep(float(d[len("sleep "):]))

    print(json.dumps({
        "type": "result", "subtype": "success", "is_error": False, "num_turns": 3,
        "total_cost_usd": 0.01,
        "usage": {"input_tokens": 100, "output_tokens": 50,
                  "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0},
    }))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
