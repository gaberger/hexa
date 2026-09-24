//! Do one task: pick the loop, run it, record the run.
//!
//! The dispatcher sat in `direct_exec`, beside the single-shot loop it can
//! choose, and so `direct_exec` imported `direct_react` while `direct_react`
//! imported `direct_exec`'s helpers — a cycle. It is the one use case that
//! stands on both loops, so it stands above them.

use crate::direct_exec::{self, DirectResult, DirectTask, ExecDeps};
use crate::direct_react;

pub async fn execute_direct_with(deps: &ExecDeps, task: DirectTask) -> DirectResult {
    let started = std::time::Instant::now();
    let started_at = chrono::Utc::now().to_rfc3339();
    let Some(model) = direct_exec::resolve_model(&task) else {
        return DirectResult::err(direct_exec::NO_MODEL_CONFIGURED.to_string());
    };

    // Default (ADR-2606071XXX): the multi-step ReAct tool-use loop — the agent
    // explores (grep/read/cargo_check) before editing. `--fast` keeps the
    // single-shot path (read → one edit → evidence → retry) for trivial edits.
    if task.fast {
        let result = direct_exec::execute_direct_inner(deps, task.clone()).await;
        direct_exec::record_run(&*deps.runs, started_at, &task, &model, &result, started.elapsed().as_millis() as u64);
        result
    } else {
        // Evidence-gated best-of-N across candidate models (ADR-2606072044): try
        // each in order, commit the first that passes. Single-model configs resolve
        // to a one-element list, so this is a no-op for them.
        let (result, steps, used_model) = direct_react::react_execute_best_of_n(deps, task.clone()).await;
        direct_exec::record_react_run(
            &*deps.runs,
            started_at,
            &task,
            &used_model,
            result.ok,
            result.evidence_passed,
            result.committed.clone(),
            steps,
            started.elapsed().as_millis() as u64,
            result.error.clone(),
        );
        result
    }
}
