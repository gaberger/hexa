---
id: ADR-2609241707
status: accepted
date: 2026-09-24
---
# ADR-2609241707: A grade cannot exceed the code it could classify

**Status:** Accepted
**Date:** 2026-09-24
**Drivers:** hexa graded itself **A+, 100/100** while 118 of its 144 Rust files had no layer and no import between its crates was an edge. Once every file was classified and every crate edge resolved, the same tree graded **C, 70** with three real violations. The A+ was a claim about roughly a fifth of the code, printed as a claim about all of it. Asked in review: *how do we ensure other projects are strictly managed this way, if hexa can't keep its own code straight?*

## Context

ADR-2609221700 decided that a scan which read no files produces no grade. It left the neighbouring case open: a scan which read every file but could place only some of them in a layer.

An edge is checked only when both of its files have a layer. A file the classifier cannot place is `Unknown`, and every import into or out of it is skipped. So the score is computed over the classified part of the tree and silent about the rest, and nothing in the output said how large the rest was. A project organised by crate, by feature, or by any folder names other than `domain/`, `ports/`, `adapters/` could — and hexa did — receive A+ with most of its code unexamined.

The instrument now has what it needs to close this: a project can declare the layer of any path (`analyze.layers`, ADR in commit `342a903`), a crate named for its layer is that layer, and one classifier serves every mode. What was missing is the grade refusing to overstate its own reach.

## Decision

1. **Coverage is part of the grade.** Coverage is the share of the files the grade reads that have a layer (anything but `Unknown`). Build scripts (`build.rs`) are not architecture and are not counted either way.

2. **The score cannot exceed coverage.** The score is `min(formula, ceiling)`, where the ceiling is the coverage percentage rounded down.

3. **A+ requires full coverage.** Below 100%, the ceiling is also at most 94 — the top of the A band — so one unclassified file is enough to withhold A+, whatever the rest of the tree looks like.

4. **The unclassified files are named.** The text report lists them and `--json` carries them under `coverage.unclassified`, each one a fix: declare its layer in `.hexa/project.json`, or move it. A deduction a reader cannot trace to a file is a number to argue with (ADR-2609140020).

## Consequences

- **Projects outside the folder convention grade lower until they declare their layers.** That is the point: their previous grade described code the grade had not checked. The fix is one config entry per directory, not a restructure.
- **hexa is unaffected today**, because every file it grades is classified — which is what made its real grade visible.
- **The formula gains a term.** `explain` and the printed formula say so, so a score below the other deductions is attributable.
- **This does not make the classification right.** A project can declare a file into the wrong layer. Coverage says the grade *looked*; the layer map, reviewed like any config, says what it looked *for*.

## Implementation

- `hexa-analysis`: coverage computed over the graded file set with the project's `LayerMap`; `ArchAnalysisResult` carries it; the ceiling applied after the formula.
- `hexa-cli analyze`: coverage printed with the grade, named files listed, `coverage` in `--json`, formula text updated.

**Gate**, written first: `cargo test -p hexa-cli --test a_grade_cannot_exceed_what_it_classified` — a fully classified project can grade A+ (control); one unclassified file withholds A+ and is named; half-unclassified caps the score at 50; declaring the layer restores A+; `build.rs` is not counted; hexa's own coverage is 100%.

## References

- ADR-2609221700: a grade is a claim about code that was read. This is that rule applied to code that was read but could not be placed.
- ADR-2609140020: a grade's deductions are traceable to files.
- ADR-2609160100: a rubric is tested against known poles.
