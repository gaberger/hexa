#!/usr/bin/env python3
"""Summarize the blank-slate A/B by model and arm.

    python3 -I bench/ab/greenfield/report_greenfield.py [runs_dir] [--md out.md] [--json out.json]

Every launched run is counted. Void runs are listed and left out of the
numbers. Defects are the hidden probes in bench/probes, which no arm sees.
"""
import json
import statistics
import sys
from collections import defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent


def med(xs):
    xs = [x for x in xs if x is not None]
    return statistics.median(xs) if xs else None


def mean(xs):
    xs = [x for x in xs if x is not None]
    return round(statistics.mean(xs), 2) if xs else None


def rate(xs):
    return sum(1 for x in xs if x) / len(xs) if xs else None


def tokens(c):
    return c["input_tokens"] + c["output_tokens"] + c["cache_read_tokens"] + c["cache_write_tokens"]


def summarize(rs):
    ok = [r for r in rs if not r["void"]]
    b = [r["build"] for r in ok]
    c = [r["change"] for r in ok]
    probes = sorted({p for r in ok for p in list(r["build"]["probes"]) + list(r["change"]["probes"])})
    return {
        "runs": len(rs), "void": len(rs) - len(ok),
        "build_gate_pass": rate([x["gate_pass"] for x in b]),
        "build_defects_mean": mean([x["defects"] for x in b]),
        "build_clean": rate([x["gate_pass"] and x["defects"] == 0 for x in b]),
        "change_gate_pass": rate([r["accepted"]["change"] for r in ok]),
        "change_defects_mean": mean([x["defects"] for x in c]),
        "build_usd_median": med([x["cost"]["usd"] for x in b]),
        "change_usd_median": med([x["cost"]["usd"] for x in c]),
        "total_usd": round(sum(x["cost"]["usd"] for x in b + c), 2),
        "build_tokens_median": med([tokens(x["cost"]) for x in b]),
        "build_minutes_median": round(med([x["cost"]["wall_s"] for x in b]) / 60, 1) if b else None,
        "src_files_median": med([x["src_files"] for x in b]),
        "src_nloc_median": med([x["src_nloc"] for x in b]),
        "test_nloc_median": med([x["test_nloc"] for x in b]),
        "wrote_tests": rate([x["test_nloc"] > 0 for x in b]),
        "ccn_mean_median": med([x["complexity"]["ccn_mean"] for x in b]),
        "ccn_max_median": med([x["complexity"]["ccn_max"] for x in b]),
        "change_src_files_median": med([x["blast_radius"]["src_files_touched"] for x in c]),
        "change_src_lines_median": med([x["blast_radius"]["lines_added"] + x["blast_radius"]["lines_removed"] for x in c]),
        "probe_defect_rate_build": {p: rate([r["build"]["probes"].get(p) == "defect" for r in ok]) for p in probes},
        "probe_defect_rate_change": {p: rate([r["change"]["probes"].get(p) == "defect" for r in ok]) for p in probes},
        "void_reasons": sorted({v for r in rs for v in r["void"]}),
    }


def pct(x):
    return "—" if x is None else f"{100 * x:.0f}%"


def num(x, d=0):
    if x is None:
        return "—"
    return f"{x:.{d}f}" if isinstance(x, float) else str(x)


ROWS = [
    ("runs / void", lambda s: f"{s['runs']} / {s['void']}"),
    ("**build: gate passes**", lambda s: pct(s["build_gate_pass"])),
    ("**build: hidden defects, mean**", lambda s: num(s["build_defects_mean"], 2)),
    ("build: gate passes and 0 defects", lambda s: pct(s["build_clean"])),
    ("change: gate passes on SQLite", lambda s: pct(s["change_gate_pass"])),
    ("change: hidden defects, mean", lambda s: num(s["change_defects_mean"], 2)),
    ("build $, median", lambda s: num(s["build_usd_median"], 2)),
    ("change $, median", lambda s: num(s["change_usd_median"], 2)),
    ("total $", lambda s: num(s["total_usd"], 2)),
    ("build minutes, median", lambda s: num(s["build_minutes_median"], 1)),
    ("source files, median", lambda s: num(s["src_files_median"])),
    ("source lines, median", lambda s: num(s["src_nloc_median"])),
    ("test lines, median", lambda s: num(s["test_nloc_median"])),
    ("builds that wrote tests", lambda s: pct(s["wrote_tests"])),
    ("CCN mean / max, median", lambda s: f"{num(s['ccn_mean_median'], 1)} / {num(s['ccn_max_median'])}"),
    ("change: source files touched, median", lambda s: num(s["change_src_files_median"])),
]


def markdown(ev):
    out = ["# Blank-slate A/B: by model and arm", "",
           "A = no hexa, the model designs from a behaviour-only spec. B = the same spec through `hexa scaffold`.",
           "Defects are bench/probes, hidden from both arms.", ""]
    for model, arms in list(ev["models"].items()) + [("all models", ev["pooled"])]:
        keys = sorted(arms)
        out += [f"## {model}", "", "| | " + " | ".join(keys) + " |", "|---|" + "---:|" * len(keys)]
        for label, f in ROWS:
            out.append(f"| {label} | " + " | ".join(f(arms[k]) for k in keys) + " |")
        probes = sorted({p for k in keys for p in arms[k]["probe_defect_rate_build"]})
        for p in probes:
            out.append(f"| probe {p} (build / change) | " + " | ".join(
                f"{pct(arms[k]['probe_defect_rate_build'].get(p))} / {pct(arms[k]['probe_defect_rate_change'].get(p))}"
                for k in keys) + " |")
        voids = sorted({v for k in keys for v in arms[k]["void_reasons"]})
        if voids:
            out += ["", "Void reasons: " + "; ".join(voids)]
        out.append("")
    return "\n".join(out)


def main():
    args = sys.argv[1:]
    runs = Path(args[0]) if args and not args[0].startswith("--") else HERE / "runs"
    recs = [json.loads(p.read_text()) for p in sorted(runs.glob("*/*/*.json"))]
    by = defaultdict(lambda: defaultdict(list))
    for r in recs:
        by[r["model"]][r["arm"]].append(r)
    ev = {"models": {m: {a: summarize(rs) for a, rs in sorted(arms.items())} for m, arms in sorted(by.items())},
          "pooled": {a: summarize([r for r in recs if r["arm"] == a]) for a in sorted({r["arm"] for r in recs})}}
    md = markdown(ev)
    if "--md" in args:
        Path(args[args.index("--md") + 1]).write_text(md)
    if "--json" in args:
        Path(args[args.index("--json") + 1]).write_text(json.dumps(ev, indent=2))
    print(md)


if __name__ == "__main__":
    main()
