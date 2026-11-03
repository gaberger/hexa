#!/usr/bin/env python3
"""Turn runs/<model>/<arm>/*.json into evidence per model.

    python3 -I report.py [runs_dir] [--json out.json] [--md out.md]

For each model, each arm: every rubric metric, over every launched run.
Voided runs are listed and left out of the rates, never silently dropped.
For each model: B−A and C−A on accepted-correct, paired by task, with a 95% CI
from a bootstrap that resamples tasks and then runs within each task (runs of
one task are correlated, so resampling runs alone would make the CI too narrow).
"""
import json
import random
import statistics
import sys
from collections import defaultdict
from pathlib import Path

BOOT = 2000
SEED = 7


def load(runs_dir):
    recs = []
    for p in sorted(Path(runs_dir).glob("*/*/*.json")):
        r = json.loads(p.read_text())
        if r["arm"] == "0":
            continue
        recs.append(r)
    return recs


def rate(xs):
    return sum(1 for x in xs if x) / len(xs) if xs else None


def med(xs):
    xs = [x for x in xs if x is not None]
    return statistics.median(xs) if xs else None


def summarize(rs):
    ok = [r for r in rs if not r["void"]]
    acc = [r["accuracy"] for r in ok]
    accepted = [r for r in ok if r["accuracy"]["accepted_correct"]]
    usd = sum(r["cost"]["usd"] for r in ok)
    tok = sum(r["cost"]["input_tokens"] + r["cost"]["output_tokens"]
              + r["cost"]["cache_read_tokens"] + r["cost"]["cache_write_tokens"] for r in ok)
    grades = defaultdict(int)
    for r in ok:
        if r["shape"]["gradeable"]:
            grades[r["shape"]["grade"]] += 1
    return {
        "runs_launched": len(rs),
        "runs_void": len(rs) - len(ok),
        "accuracy": {
            "accepted_correct": rate([a["accepted_correct"] for a in acc]),
            "oracle_pass": rate([a["oracle_pass"] for a in acc]),
            "own_gate_pass": rate([a["own_gate_pass"] for a in acc]),
            "false_green_correctness": rate([a["false_green_correctness"] for a in acc]),
            "false_green_shape": rate([a["false_green_shape"] for a in acc]),
            "timeouts": rate([r["timed_out"] for r in ok]),
        },
        "shape": {
            "grade_distribution": dict(grades) or None,
            "violations_median": med([r["shape"]["violations"] for r in ok if r["shape"]["gradeable"]]),
        },
        "cost": {
            "usd_total": round(usd, 4),
            "usd_median_per_run": med([r["cost"]["usd"] for r in ok]),
            "tokens_median_per_run": med([r["cost"]["input_tokens"] + r["cost"]["output_tokens"]
                                          + r["cost"]["cache_read_tokens"] + r["cost"]["cache_write_tokens"] for r in ok]),
            "wall_s_median_per_run": med([r["cost"]["wall_s"] for r in ok]),
            "usd_per_accepted_correct": round(usd / len(accepted), 4) if accepted else None,
            "tokens_per_accepted_correct": round(tok / len(accepted)) if accepted else None,
        },
        "loc": {
            "src_nloc_median": med([r["loc"]["src_nloc"] for r in ok]),
            "test_nloc_median": med([r["loc"]["test_nloc"] for r in ok]),
            "test_to_src_median": med([r["loc"]["test_to_src"] for r in ok]),
            "lines_changed_median": med([r["loc"]["lines_added"] + r["loc"]["lines_removed"] for r in ok]),
        },
        "complexity": {
            "ccn_mean_median": med([r["complexity"]["ccn_mean"] for r in ok]),
            "ccn_max_median": med([r["complexity"]["ccn_max"] for r in ok]),
            "fn_len_max_median": med([r["complexity"]["fn_len_max"] for r in ok]),
        },
        "void_reasons": sorted({v for r in rs for v in r["void"]}),
    }


def paired_diff(rs, arm_x, arm_y, rng):
    """accepted-correct(arm_x) - accepted-correct(arm_y), over tasks both arms ran."""
    by = defaultdict(lambda: defaultdict(list))
    for r in rs:
        if not r["void"]:
            by[r["task"]][r["arm"]].append(1 if r["accuracy"]["accepted_correct"] else 0)
    tasks = [t for t in by if by[t][arm_x] and by[t][arm_y]]
    if not tasks:
        return None

    def est(sample):
        return statistics.mean(statistics.mean(xs) - statistics.mean(ys) for xs, ys in sample)

    point = est([(by[t][arm_x], by[t][arm_y]) for t in tasks])
    boots = []
    for _ in range(BOOT):
        sample = []
        for t in rng.choices(tasks, k=len(tasks)):
            xs, ys = by[t][arm_x], by[t][arm_y]
            sample.append((rng.choices(xs, k=len(xs)), rng.choices(ys, k=len(ys))))
        boots.append(est(sample))
    boots.sort()
    return {"tasks": len(tasks), "diff_points": round(100 * point, 1),
            "ci95_points": [round(100 * boots[int(0.025 * BOOT)], 1), round(100 * boots[int(0.975 * BOOT) - 1], 1)]}


def pct(x):
    return "—" if x is None else f"{100 * x:.0f}%"


def num(x, d=0):
    return "—" if x is None else (f"{x:.{d}f}" if isinstance(x, float) else str(x))


FAMILY_TITLES = {
    "change": "Change existing code (the hexa fixtures)",
    "shape": "Shape traps (hexa's own examples)",
}


def markdown(ev):
    out = ["# hexa A/B evidence, by family and model", ""]
    for fam, block in ev["families"].items():
        out += [f"# {FAMILY_TITLES.get(fam, fam)}", "",
                f"Tasks: {', '.join(block['tasks'])}.", ""]
        out += markdown_models(block["models"])
    return "\n".join(out)


def markdown_models(models):
    out = []
    rows = [("runs launched / void", lambda s: f"{s['runs_launched']} / {s['runs_void']}"),
            ("**accepted-correct**", lambda s: pct(s["accuracy"]["accepted_correct"])),
            ("oracle pass", lambda s: pct(s["accuracy"]["oracle_pass"])),
            ("own gate pass", lambda s: pct(s["accuracy"]["own_gate_pass"])),
            ("false-green (correctness)", lambda s: pct(s["accuracy"]["false_green_correctness"])),
            ("false-green (shape)", lambda s: pct(s["accuracy"]["false_green_shape"])),
            ("timeouts", lambda s: pct(s["accuracy"]["timeouts"])),
            ("violations, median", lambda s: num(s["shape"]["violations_median"])),
            ("$ per run, median", lambda s: num(s["cost"]["usd_median_per_run"], 3)),
            ("tokens per run, median", lambda s: num(s["cost"]["tokens_median_per_run"])),
            ("wall s per run, median", lambda s: num(s["cost"]["wall_s_median_per_run"], 0)),
            ("$ per accepted-correct", lambda s: num(s["cost"]["usd_per_accepted_correct"], 3)),
            ("tokens per accepted-correct", lambda s: num(s["cost"]["tokens_per_accepted_correct"])),
            ("src NLOC, median", lambda s: num(s["loc"]["src_nloc_median"])),
            ("test NLOC, median", lambda s: num(s["loc"]["test_nloc_median"])),
            ("lines changed, median", lambda s: num(s["loc"]["lines_changed_median"])),
            ("CCN mean, median", lambda s: num(s["complexity"]["ccn_mean_median"], 1)),
            ("CCN max, median", lambda s: num(s["complexity"]["ccn_max_median"])),
            ("longest function, median", lambda s: num(s["complexity"]["fn_len_max_median"]))]
    for model, m in models.items():
        arms = sorted(m["arms"])
        out += [f"## {model}", "", "| metric | " + " | ".join(arms) + " |", "|---|" + "---:|" * len(arms)]
        for label, f in rows:
            out.append(f"| {label} | " + " | ".join(f(m["arms"][a]) for a in arms) + " |")
        out.append("")
        for k, d in m["paired"].items():
            if d:
                out.append(f"- **{k}** accepted-correct: {d['diff_points']:+.1f} points, "
                           f"95% CI [{d['ci95_points'][0]:+.1f}, {d['ci95_points'][1]:+.1f}], over {d['tasks']} tasks")
        voids = sorted({v for a in m["arms"].values() for v in a["void_reasons"]})
        if voids:
            out.append("- void reasons: " + "; ".join(voids))
        out.append("")
    return out


def by_model(recs, rng):
    models = {}
    for model in sorted({r["model"] for r in recs}):
        rs = [r for r in recs if r["model"] == model]
        models[model] = {
            "arms": {arm: summarize([r for r in rs if r["arm"] == arm]) for arm in sorted({r["arm"] for r in rs})},
            "paired": {"B−A": paired_diff(rs, "B", "A", rng), "C−A": paired_diff(rs, "C", "A", rng)},
        }
    return models


def main():
    args = sys.argv[1:]
    runs_dir = args[0] if args and not args[0].startswith("--") else str(Path(__file__).parent / "runs")
    recs = load(runs_dir)
    rng = random.Random(SEED)
    order = [f for f in FAMILY_TITLES if any(r["family"] == f for r in recs)]
    order += sorted({r["family"] for r in recs} - set(order))
    ev = {"runs_dir": runs_dir, "families": {}, "models": by_model(recs, rng)}
    for fam in order:
        fr = [r for r in recs if r["family"] == fam]
        ev["families"][fam] = {"title": FAMILY_TITLES.get(fam, fam),
                               "tasks": sorted({r["task"] for r in fr}),
                               "models": by_model(fr, rng)}
    md = markdown(ev)
    if "--json" in args:
        Path(args[args.index("--json") + 1]).write_text(json.dumps(ev, indent=2))
    if "--md" in args:
        Path(args[args.index("--md") + 1]).write_text(md)
    print(md)


if __name__ == "__main__":
    main()
