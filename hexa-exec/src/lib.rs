//! hexa-exec — the single-agent execution loop (ADR-2606071340 P1).
//!
//! The ReAct tool-use loop (`direct_react`), the single-shot executor
//! (`direct_exec`), per-run git-worktree isolation (`direct_workspace`,
//! ADR-2606071323), transcript compression, the tool-use protocol
//! (`simple_agent`), and the curated guarded `tools` library. Depends only on
//! hexa-core (ports/types), hexa-graph (code-graph context), and hexa-git
//! (worktree/commit) — no daemon coupling, so the agent loop is reusable
//! outside hexa-nexus.

pub mod local_store;
pub mod provenance;
pub mod adversarial;
pub mod compress;
pub mod direct_exec;
pub mod direct_react;
pub mod do_task;
pub mod frontier;
pub mod git_worktrees;
pub mod ports;
pub mod direct_workspace;
pub mod resource_governor;
pub mod simple_agent;
pub mod telegram_notifier;
pub mod tool_registry;
pub mod tools;

/// The repository the tools operate on.
///
/// `HEXA_REPO_ROOT`, else the CURRENT DIRECTORY. It used to fall back to
/// `/home/gary/hexa-intf` — one contributor's checkout, hardcoded into eleven tools. On any other
/// machine, and in any other project, every `repo_read` and `repo_grep` resolved against a path
/// that does not exist and returned `ok=false`.
///
/// That failure is invisible from the outside. The ReAct loop watched devstral-small-2:24b call
/// `repo_read` thirteen times with perfectly correct arguments, get nothing back each time, and
/// give up — which reads as a model that cannot follow through, not as a tool pointed at a
/// stranger's home directory. hexa "installs INTO a target project"; the tools have to look there.
pub fn repo_root() -> String {
    std::env::var("HEXA_REPO_ROOT").unwrap_or_else(|_| {
        std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| ".".to_string())
    })
}

/// The tools the agent loop may call — every adapter behind the [`ports::Tool`]
/// port, registered here, at the crate root, so the loop never names one.
pub fn default_tools() -> tool_registry::ToolRegistry {
    use std::sync::Arc;
    use tools::*;
    let mut reg = tool_registry::ToolRegistry::new();
    reg.register(Arc::new(cargo_check::CargoCheck));
    reg.register(Arc::new(dep_audit::DepAudit));
    reg.register(Arc::new(repo_grep::RepoGrep));
    reg.register(Arc::new(repo_read::RepoRead));
    reg.register(Arc::new(secret_scan::SecretScan));
    reg.register(Arc::new(web_search::WebSearch));
    reg.register(Arc::new(adr_draft::AdrDraft));
    reg.register(Arc::new(spec_draft::SpecDraft));
    reg.register(Arc::new(code_patch::CodePatch));
    reg.register(Arc::new(cost_meter::CostMeter));
    reg.register(Arc::new(workplan_emit::WorkplanEmit));
    reg.register(Arc::new(adr_status_set::AdrStatusSet));
    reg.register(Arc::new(workspace_boundary_check::WorkspaceBoundaryCheck));
    reg.register(Arc::new(escalate_to_operator::EscalateToOperator));
    reg.register(Arc::new(typescript_check::TypescriptCheck));
    reg.register(Arc::new(memory_search::MemorySearch));
    reg
}

/// A run's dependencies, wired: the default tools, and worktrees on git.
pub fn default_deps() -> direct_exec::ExecDeps {
    direct_exec::ExecDeps {
        tools: std::sync::Arc::new(default_tools()),
        worktrees: std::sync::Arc::new(git_worktrees::GitWorktrees),
        frontier: frontier_agent(),
        runs: std::sync::Arc::new(local_store::LocalRuns),
        memory: std::sync::Arc::new(memory()),
    }
}

/// Run one direct task on the default dependencies.
/// See [`do_task::execute_direct_with`].
pub async fn execute_direct(task: direct_exec::DirectTask) -> direct_exec::DirectResult {
    do_task::execute_direct_with(&default_deps(), task).await
}

/// The memory store, wired: `memory.jsonl` files under the project or `~/.hexa`.
pub fn memory() -> impl ports::MemoryStore {
    local_store::LocalMemory
}

/// The frontier agent, wired: the operator's logged-in `claude` CLI.
pub fn frontier_agent() -> std::sync::Arc<dyn ports::Frontier> {
    std::sync::Arc::new(frontier::ClaudeCli)
}

/// The run log, wired: the local runs file.
pub fn run_log() -> impl ports::RunLog {
    local_store::LocalRuns
}

/// Where a build's receipt goes, wired: `PROVENANCE.md` in the target.
pub fn provenance_store() -> impl ports::Provenance {
    provenance::ProvenanceFile
}
