---
name: hexa-status-summary
description: Provide a high-level synthesis of project progress, mapping ADRs to Specs and current architectural health.
trigger: /hexa-status-summary
---

# Project Progress Summary

## Steps

1. **Analyze Architecture**: Run `hexa analyze .` to get the current grade and boundary violation count.

2. **Map the Pipeline**:
   - Scan `docs/adrs/` for the most recent `Accepted` decisions.
   - Check `docs/specs/` to see which of those ADRs have corresponding specs.
   - Cross-reference with recent git commits to see which specs have been implemented.

3. **Synthesize State**:
   - **Completed**: Features where ADR $\rightarrow$ Spec $\rightarrow$ Code $\rightarrow$ Verified.
   - **In Progress**: Current active "Gates" (e.g., an ADR with a pending Spec).
   - **Pipeline Health**: Report the "velocity" (how many ADRs are currently in the spec/build phase).

4. **Generate Report**:
   Produce a summary using the following structure:
   - **✅ Completed**: List of high-value items shipped.
   - **🚧 In Progress**: Table of current pipeline phases (ADR $\rightarrow$ Spec $\rightarrow$ Gate $\rightarrow$ Build $\rightarrow$ Harden).
   - **⬡ Health**: Current architecture grade and any critical violations.

## CLI Shortcut
Use `hexa status` for low-level state, but use this skill for the "Developer's View" of progress.
