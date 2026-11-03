#!/usr/bin/env python3
"""Recheck runs voided for oracle leakage, against each run's own transcripts.

    python3 -I bench/ab/recheck_leaks.py [runs_dir]

Before commit 'bench/ab: leak check reads only the run's own transcripts', the
leak check read every transcript changed during a run, including other
sessions' (such as the one that launched the bench), which name the fixtures.
This rereads only the transcripts under the run's own workspace. A void is
lifted only when they hold no oracle material, and the record keeps the old
reason under `void_lifted`, so nothing is silently rewritten.
"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import run_one  # noqa: E402

PREFIX = "transcript touched oracle material"


def main():
    runs = Path(sys.argv[1] if len(sys.argv) > 1 else Path(__file__).parent / "runs")
    lifted = kept = 0
    for p in sorted(runs.glob("*/*/*.json")):
        r = json.loads(p.read_text())
        leak = [v for v in r.get("void", []) if v.startswith(PREFIX)]
        if not leak:
            continue
        own = run_one.leak_markers(r["workspace"])
        n = len(run_one.run_transcripts(r["workspace"]))
        if own or n == 0:
            kept += 1
            print(f"kept    {p.relative_to(runs)}: own transcripts {n}, markers {own}")
            continue
        r["void"] = [v for v in r["void"] if not v.startswith(PREFIX)]
        r["void_lifted"] = {"reason": leak, "why": f"the run's own {n} transcript(s) hold no oracle material; "
                            "the old check also read other sessions' transcripts"}
        p.write_text(json.dumps(r, indent=2))
        lifted += 1
        print(f"lifted  {p.relative_to(runs)}")
    print(f"{lifted} lifted, {kept} kept")


if __name__ == "__main__":
    main()
