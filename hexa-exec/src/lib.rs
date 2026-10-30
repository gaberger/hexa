//! hexa-exec — the single-agent execution loop (ADR-2606071340 P1).
//!
//! The ReAct tool-use loop (`direct_react`), the single-shot executor
//! (`direct_exec`), per-run git-worktree isolation (`direct_workspace`,
//! ADR-2606071323), transcript compression, the tool-use protocol
//! (`simple_agent`), and the curated guarded `tools` library. Depends only on
//! hexa-core (ports/types), hexa-graph (code-graph context), and hexa-git
//! (worktree/commit) — no daemon coupling, so the agent loop is reusable
//! outside hexa-nexus.

// `#[async_trait]` marks each method `#[must_use]` and returns a boxed future, which is
// already must-use; clippy 1.99's `double_must_use` flags the pair in the expansion.
#![allow(clippy::double_must_use)]

pub mod local_store;
pub mod provenance;
pub mod adversarial;
pub mod compress;
pub mod direct_exec;
pub mod direct_react;
pub mod do_task;
pub mod frontier;
pub mod git_history;
pub mod git_worktrees;
pub mod ports;
pub mod direct_workspace;
pub mod resource_governor;
pub mod simple_agent;
pub mod spec_author;
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
    let ledger: Arc<dyn ports::ToolLedger> = Arc::new(local_store::LocalLedger);
    let mut reg = tool_registry::ToolRegistry::new();
    reg.register(Arc::new(cargo_check::CargoCheck));
    reg.register(Arc::new(dep_audit::DepAudit));
    reg.register(Arc::new(repo_grep::RepoGrep));
    reg.register(Arc::new(repo_read::RepoRead));
    reg.register(Arc::new(secret_scan::SecretScan));
    reg.register(Arc::new(web_search::WebSearch));
    reg.register(Arc::new(adr_draft::AdrDraft::new(ledger.clone())));
    reg.register(Arc::new(spec_draft::SpecDraft::new(ledger.clone())));
    reg.register(Arc::new(code_patch::CodePatch::new(ledger.clone())));
    reg.register(Arc::new(cost_meter::CostMeter::new(ledger.clone())));
    reg.register(Arc::new(workplan_emit::WorkplanEmit::new(ledger.clone())));
    reg.register(Arc::new(adr_status_set::AdrStatusSet::new(ledger)));
    reg.register(Arc::new(workspace_boundary_check::WorkspaceBoundaryCheck));
    reg.register(Arc::new(escalate_to_operator::EscalateToOperator::new(Arc::new(telegram_notifier::TelegramNotifier::from_env()))));
    reg.register(Arc::new(typescript_check::TypescriptCheck));
    reg.register(Arc::new(memory_search::MemorySearch::new(Arc::new(memory()))));
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

/// The branch history harden's batch bisection moves, on git (ADR-2610160756 §4).
pub fn default_git_history() -> std::sync::Arc<dyn ports::GitHistory> {
    std::sync::Arc::new(git_history::GitHistoryCli)
}

/// Run one direct task on the default dependencies.
/// See [`do_task::execute_direct_with`].
pub async fn execute_direct(task: direct_exec::DirectTask) -> direct_exec::DirectResult {
    do_task::execute_direct_with(&default_deps(), task).await
}

/// Draft a spec from an ADR and its ports, on the registry-backed backends.
/// See [`spec_author::draft`] (ADR-2610221540 §2).
pub async fn draft_spec(model: &str, root: &std::path::Path, adr_id: &str) -> Result<spec_author::Drafted, String> {
    spec_author::draft(&hexa_infer::wiring::RegistryBackends, model, root, adr_id).await
}

/// Generate an accepted spec's tests, red, on the registry-backed backends.
/// See [`spec_author::generate_tests`] (ADR-2610221540 §5).
pub async fn generate_spec_tests(
    model: &str,
    root: &std::path::Path,
    spec: &hexa_core::ports::spec::Spec,
    krate: &std::path::Path,
    path_env: &str,
) -> Result<spec_author::Generated, String> {
    spec_author::generate_tests(&hexa_infer::wiring::RegistryBackends, model, root, spec, krate, path_env).await
}

/// The memory store, wired: `memory.jsonl` files under the project or `~/.hexa`.
pub fn memory() -> impl ports::MemoryStore {
    local_store::LocalMemory
}

/// [`ports::SpendLog`] on hexa-infer's spend log and the project's budget.
/// Wiring, so it lives at the crate root: the frontier adapter holds the
/// port, and only this file names the crate behind it (ADR-2610271649 §5).
struct InferSpend;

impl ports::SpendLog for InferSpend {
    fn check_budget(&self) -> Result<(), String> {
        hexa_infer::spend_budget_check()
    }
    fn record_frontier(&self, reply: &serde_json::Value, input_tokens: u64, output_tokens: u64, cost_usd: Option<f64>, source: &str) {
        let model = hexa_infer::spend::model_from_frontier_json(reply);
        hexa_infer::spend::record_with(&model, input_tokens, output_tokens, cost_usd, source);
    }
}

/// The frontier agent, wired: the operator's logged-in `claude` CLI.
pub fn frontier_agent() -> std::sync::Arc<dyn ports::Frontier> {
    std::sync::Arc::new(frontier::ClaudeCli::new(std::sync::Arc::new(InferSpend)))
}

/// The gap log, wired: `~/.hexa/gaps.jsonl` (ADR-2610301500).
pub fn gap_log() -> impl ports::GapLog {
    local_store::LocalGaps
}

/// The run log, wired: the local runs file.
pub fn run_log() -> impl ports::RunLog {
    local_store::LocalRuns
}

/// Where a build's receipt goes, wired: `PROVENANCE.md` in the target.
pub fn provenance_store() -> impl ports::Provenance {
    provenance::ProvenanceFile
}
