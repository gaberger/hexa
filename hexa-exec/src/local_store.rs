//! Local, file-backed durability — the daemon's tables as JSONL under `~/.hexa`.
//!
//! Four things reached SpacetimeDB from inside the agent loop: the proposal queue
//! (`proposed_action_open`, five tools), the agent-run feed (`record_agent_run` plus a startup
//! hydrate), token spend, and memory. All of it over HTTP to a database the daemon owned — so the
//! loop needed a database up to draft an ADR.
//!
//! JSONL, not SQLite. These are append-only feeds read newest-first and bounded; a line per record
//! is the whole data model. It adds no dependency, survives a partial write (a torn last line is
//! skipped, not a corrupt file), and can be read with `tail`. If any of these grows a query beyond
//! "last N", that is the moment to reach for a real database — not before.
//!
//! # Two homes
//!
//! The observability feeds — proposals, agent runs, inference spend — are per *user*: they
//! describe the machine and what it has run, and nothing in them means something different in
//! another repository.
//!
//! **Memory is per project** (`<project>/.hexa/memory.jsonl`), because its contents *do*:
//! `adr:0007:why` names a different decision in every repository that has an ADR 7, and a
//! `lesson:` learned in one codebase is not evidence about another. Keys carry no project
//! namespace, so a shared file made those collisions silent rather than loud, and
//! `memory_search` hands the result to the model during grounding. The user store
//! (`~/.hexa/memory.jsonl`) remains for entries that genuinely are cross-project, reached
//! explicitly with [`MemoryScope::Shared`] — never as a fallback, which is the bleed again.

use crate::ports::MemoryScope;
use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// `~/.hexa`, or `$HEXA_HOME`. Created on demand.
fn hexa_home() -> PathBuf {
    std::env::var("HEXA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".hexa")
        })
}

/// Append one record. Best-effort by design, and the reason is the same one the STDB persist had:
/// the feed is observability, and losing a line must never fail the work that produced it.
fn append(name: &str, record: &Value) -> Result<(), String> {
    append_in(&hexa_home(), name, record)
}

/// The primitive, with the directory passed in.
///
/// Tests drive THIS, not the `HEXA_HOME` env var. Two tests that each `set_var` the same name race
/// in cargo's thread pool and clobber one another — a bug this file's first draft had, and the same
/// one that makes `hexa-agent`'s safe_file_writer suite flaky. An injected path cannot race.
fn append_in(dir: &std::path::Path, name: &str, record: &Value) -> Result<(), String> {
    let path = dir.join(name);
    if let Some(dir) = path.parent() {
        create_dir_all(dir).map_err(|e| format!("{}: {}", dir.display(), e))?;
    }
    let mut line = serde_json::to_string(record).map_err(|e| e.to_string())?;
    line.push('\n');
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("{}: {}", path.display(), e))?;
    f.write_all(line.as_bytes()).map_err(|e| e.to_string())
}

/// Read the last `limit` records, newest first. A line that does not parse is SKIPPED rather than
/// failing the read: a torn final line from a killed process must not hide the history behind it.
fn read_tail(name: &str, limit: usize) -> Vec<Value> {
    read_tail_in(&hexa_home(), name, limit)
}

fn read_tail_in(dir: &std::path::Path, name: &str, limit: usize) -> Vec<Value> {
    let path = dir.join(name);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return Vec::new(), // absent feed is an empty feed, not an error
    };
    let mut out: Vec<Value> = text
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect();
    out.reverse();
    out.truncate(limit);
    out
}

// ── the proposal queue ────────────────────────────────────────────────────────

const PROPOSALS: &str = "proposals.jsonl";

/// Open a proposed action — a change awaiting a human.
///
/// Was `proposed_action_open` on the hexflo-coordination module. The id was assigned by the
/// database; here it is the wall-clock microsecond, which is monotonic enough to order a feed one
/// process appends to and needs no coordination to allocate.
fn propose_id() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

pub fn propose_action(kind: &str, payload: &str, source: &str) -> Result<u64, String> {
    let id = propose_id();
    append(
        PROPOSALS,
        &serde_json::json!({
            "id": id,
            "kind": kind,
            "payload": payload,
            "source": source,
            "status": "open",
            "ts": chrono::Utc::now().to_rfc3339(),
        }),
    )?;
    Ok(id)
}

// ── the agent-run feed ────────────────────────────────────────────────────────

const RUNS: &str = "agent-runs.jsonl";

/// Persist one agent run. Best-effort: the in-memory ring is the fast path, this is the copy that
/// survives a restart. Never fails a recorder.
pub fn persist_run(run: &Value) {
    if let Err(e) = append(RUNS, run) {
        tracing::debug!(error = %e, "agent-run persist failed (non-fatal)");
    }
}

/// Newest-first runs, for hydrating the in-memory feed at startup.
pub fn recent_runs(limit: usize) -> Vec<Value> {
    read_tail(RUNS, limit)
}

// ── token spend ───────────────────────────────────────────────────────────────

const SPEND: &str = "inference-log.jsonl";

// ── memory ────────────────────────────────────────────────────────────────────

const MEMORY: &str = "memory.jsonl";


/// The directory a scope's `memory.jsonl` lives in. Internal: callers outside
/// the crate want the file, which is [`memory_path`].
fn memory_dir(scope: MemoryScope) -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    resolve_memory_dir(scope, &|k| std::env::var(k).ok().filter(|v| !v.is_empty()), &cwd)
}

/// The file a scope's entries are read from and appended to — what `hexa memory`
/// prints so the scope is never a guess.
pub fn memory_path(scope: MemoryScope) -> PathBuf {
    memory_dir(scope).join(MEMORY)
}

/// Scope resolution, with the environment and the working directory passed in.
///
/// Tests drive THIS (ADR-2609131749): two tests that each `set_var("HOME", …)` race in cargo's
/// thread pool and clobber one another, and `current_dir` is process-wide state with the same
/// problem.
///
/// Precedence, both scopes:
/// 1. `$HEXA_HOME` — an explicit override of where hexa keeps state, and explicit wins.
/// 2. `Shared` → `$HOME/.hexa`.
/// 3. `Project` → the nearest ancestor of `$HEXA_PROJECT_ROOT` (else the working directory)
///    that holds a `.hexa/` directory. The ascent stops after `$HOME`: above it there is no
///    project, only other people's directories.
/// 4. No project → `$HOME/.hexa`. A directory that was never scaffolded behaves as it did
///    before, which is what makes `hexa memory` outside a repository still work.
fn resolve_memory_dir(
    scope: MemoryScope,
    env: &dyn Fn(&str) -> Option<String>,
    cwd: &Path,
) -> PathBuf {
    if let Some(explicit) = env("HEXA_HOME") {
        return PathBuf::from(explicit);
    }
    let user_home = env("HOME").map(PathBuf::from);
    let user_store = || {
        user_home.clone().unwrap_or_else(|| PathBuf::from(".")).join(".hexa")
    };
    if scope == MemoryScope::Shared {
        return user_store();
    }
    let start = env("HEXA_PROJECT_ROOT").map(PathBuf::from).unwrap_or_else(|| cwd.to_path_buf());
    let mut dir: Option<&Path> = Some(start.as_path());
    while let Some(d) = dir {
        if d.join(".hexa").is_dir() {
            return d.join(".hexa");
        }
        if user_home.as_deref() == Some(d) {
            break; // the user's home is the top of the search, not a project
        }
        dir = d.parent();
    }
    user_store()
}

/// How far back a memory read scans before it gives up on finding a key.
///
/// The feed is append-only, so a rewrite is a newer line rather than an edit.
/// A few hundred short entries is the whole realistic corpus.
const MEMORY_SCAN: usize = 5_000;

/// `(key, value)` pairs from this project's memory feed, newest first, bounded.
///
/// Was `SELECT key, value FROM hexflo_memory` over SpacetimeDB's HTTP SQL endpoint. Rows missing
/// either field are skipped rather than defaulted — a lesson with no text is not a lesson.
///
/// **Newest wins.** A rewritten key appends rather than replacing, so the same
/// key can appear many times; only its most recent line is returned. Without
/// that, `hexa memory store` on an existing key would leave the loop reading
/// both the old lesson and the new one.
pub fn memory_entries(limit: usize) -> Vec<(String, String)> {
    memory_entries_scoped(MemoryScope::Project, limit)
}

/// The same read, against a named scope.
pub fn memory_entries_scoped(scope: MemoryScope, limit: usize) -> Vec<(String, String)> {
    memory_entries_in(&memory_dir(scope), limit)
}

fn memory_entries_in(dir: &Path, limit: usize) -> Vec<(String, String)> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for v in read_tail_in(dir, MEMORY, MEMORY_SCAN) {
        let (Some(k), Some(val)) = (
            v.get("key").and_then(|x| x.as_str()),
            v.get("value").and_then(|x| x.as_str()),
        ) else {
            continue;
        };
        if v.get("deleted").and_then(|x| x.as_bool()).unwrap_or(false) {
            seen.insert(k.to_string()); // a tombstone hides older lines too
            continue;
        }
        if seen.insert(k.to_string()) {
            out.push((k.to_string(), val.to_string()));
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}

/// Write a memory entry to this project's store, superseding any earlier line with the same key.
pub fn memory_put(key: &str, value: &str) -> Result<(), String> {
    memory_put_scoped(MemoryScope::Project, key, value)
}

/// The same write, against a named scope.
pub fn memory_put_scoped(scope: MemoryScope, key: &str, value: &str) -> Result<(), String> {
    memory_put_in(&memory_dir(scope), key, value)
}

fn memory_put_in(dir: &Path, key: &str, value: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("memory key is empty".to_string());
    }
    append_in(
        dir,
        MEMORY,
        &serde_json::json!({
            "key": key,
            "value": value,
            "ts": chrono::Utc::now().to_rfc3339(),
        }),
    )
}

/// The current value for one key in this project's store, or `None`.
pub fn memory_get(key: &str) -> Option<String> {
    memory_get_scoped(MemoryScope::Project, key)
}

/// The same lookup, against a named scope. A miss is a miss: it does NOT fall
/// through to the other scope, because a silent fallback is the cross-project
/// answer this split exists to stop.
pub fn memory_get_scoped(scope: MemoryScope, key: &str) -> Option<String> {
    memory_get_in(&memory_dir(scope), key)
}

fn memory_get_in(dir: &Path, key: &str) -> Option<String> {
    memory_entries_in(dir, MEMORY_SCAN).into_iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// Entries whose key or value contains `query`, case-insensitively.
/// An empty query matches everything.
pub fn memory_search(query: &str) -> Vec<(String, String)> {
    memory_search_scoped(MemoryScope::Project, query)
}

/// The same search, against a named scope.
pub fn memory_search_scoped(scope: MemoryScope, query: &str) -> Vec<(String, String)> {
    memory_search_in(&memory_dir(scope), query)
}

fn memory_search_in(dir: &Path, query: &str) -> Vec<(String, String)> {
    let needle = query.trim().to_lowercase();
    memory_entries_in(dir, MEMORY_SCAN)
        .into_iter()
        .filter(|(k, v)| {
            needle.is_empty()
                || k.to_lowercase().contains(&needle)
                || v.to_lowercase().contains(&needle)
        })
        .collect()
}

/// Hide a key from future reads. Returns whether it was present.
///
/// A tombstone line rather than a rewrite of the file: the feed stays
/// append-only, which is what makes a torn write survivable.
pub fn memory_delete(key: &str) -> Result<bool, String> {
    memory_delete_scoped(MemoryScope::Project, key)
}

/// The same delete, against a named scope.
pub fn memory_delete_scoped(scope: MemoryScope, key: &str) -> Result<bool, String> {
    memory_delete_in(&memory_dir(scope), key)
}

fn memory_delete_in(dir: &Path, key: &str) -> Result<bool, String> {
    if memory_get_in(dir, key).is_none() {
        return Ok(false);
    }
    append_in(
        dir,
        MEMORY,
        &serde_json::json!({
            "key": key,
            "value": "",
            "deleted": true,
            "ts": chrono::Utc::now().to_rfc3339(),
        }),
    )?;
    Ok(true)
}

/// Spend rows in the shape `cost_meter` already aggregates:
/// `[group_key, input_tokens, output_tokens, cost_usd, created_at]`.
///
/// Deliberately the STDB row shape rather than a nicer one. cost_meter's aggregation — the window
/// filter, the HashMap, the cost parse — is fine and was never the problem; only where the rows
/// came from was. Keeping the shape means none of that code changes.
///
/// `cost_usd` is what the row reported, or "0". A local completion reports none and costs
/// nothing; a `claude -p` call reports its own figure and that is what is shown.
pub fn spend_rows(group_by: &str, limit: usize) -> Vec<Value> {
    read_tail(SPEND, limit)
        .into_iter()
        .map(|v| {
            let key = v
                .get(group_by)
                .and_then(|x| x.as_str())
                .or_else(|| v.get("model").and_then(|x| x.as_str()))
                .unwrap_or("unknown")
                .to_string();
            serde_json::json!([
                key,
                v.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
                v.get("output_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
                // A row that reported a cost carries it; a local completion
                // carries none, and none is "0", not a made-up figure.
                v.get("cost_usd").and_then(|x| x.as_f64()).map(|c| c.to_string()).unwrap_or_else(|| "0".to_string()),
                v.get("ts").and_then(|x| x.as_str()).unwrap_or(""),
            ])
        })
        .collect()
}

/// [`RunLog`](crate::ports::RunLog) on the local runs file.
pub struct LocalRuns;

impl crate::ports::RunLog for LocalRuns {
    fn record(&self, run: &Value) {
        persist_run(run)
    }
    fn recent(&self, limit: usize) -> Vec<Value> {
        recent_runs(limit)
    }
}

/// [`MemoryStore`](crate::ports::MemoryStore) on `memory.jsonl` files.
pub struct LocalMemory;

impl crate::ports::MemoryStore for LocalMemory {
    fn path(&self, scope: MemoryScope) -> PathBuf {
        memory_path(scope)
    }
    fn put(&self, scope: MemoryScope, key: &str, value: &str) -> Result<(), String> {
        memory_put_scoped(scope, key, value)
    }
    fn get(&self, scope: MemoryScope, key: &str) -> Option<String> {
        memory_get_scoped(scope, key)
    }
    fn entries(&self, scope: MemoryScope, limit: usize) -> Vec<(String, String)> {
        memory_entries_scoped(scope, limit)
    }
    fn search(&self, scope: MemoryScope, query: &str) -> Vec<(String, String)> {
        memory_search_scoped(scope, query)
    }
    fn delete(&self, scope: MemoryScope, key: &str) -> Result<bool, String> {
        memory_delete_scoped(scope, key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory per test. No env var, so nothing races: cargo runs these in parallel and
    /// two tests that both `set_var("HEXA_HOME", …)` overwrite each other's answer.
    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn a_proposal_round_trips_and_carries_its_source() {
        let d = dir();
        append_in(
            d.path(),
            PROPOSALS,
            &serde_json::json!({ "id": 1u64, "kind": "file_write", "source": "tool:adr_draft", "status": "open" }),
        )
        .unwrap();
        let all = read_tail_in(d.path(), PROPOSALS, 10);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0]["kind"], "file_write");
        assert_eq!(all[0]["source"], "tool:adr_draft");
        assert_eq!(all[0]["status"], "open");
    }

    #[test]
    fn proposal_ids_are_monotonic_so_a_feed_orders_without_a_database() {
        let a = propose_id();
        let b = propose_id();
        assert!(b >= a, "ids must not go backwards: {} then {}", a, b);
        assert!(a > 0, "id should be a real timestamp, not zero");
    }

    #[test]
    fn reads_are_newest_first_and_bounded() {
        let d = dir();
        for i in 0..5 {
            append_in(d.path(), RUNS, &serde_json::json!({ "n": i })).unwrap();
        }
        let got = read_tail_in(d.path(), RUNS, 3);
        assert_eq!(got.len(), 3, "limit is honoured");
        assert_eq!(got[0]["n"], 4, "newest first");
    }

    #[test]
    fn a_torn_last_line_does_not_hide_the_history_behind_it() {
        // A process killed mid-append leaves half a line. Failing the whole read there would lose
        // every earlier record too — so the bad line is skipped, not fatal.
        let d = dir();
        append_in(d.path(), RUNS, &serde_json::json!({ "n": 1 })).unwrap();
        let mut f = OpenOptions::new().append(true).open(d.path().join(RUNS)).unwrap();
        f.write_all(b"{\"n\": 2, \"tr").unwrap();
        let got = read_tail_in(d.path(), RUNS, 10);
        assert_eq!(got.len(), 1, "the intact record survives");
        assert_eq!(got[0]["n"], 1);
    }

    #[test]
    fn an_absent_feed_is_empty_not_an_error() {
        let d = dir();
        assert!(read_tail_in(d.path(), RUNS, 10).is_empty());
    }

    // ── memory scope ──────────────────────────────────────────────────────────

    /// An environment built from pairs, so a test never writes the process's own
    /// (ADR-2609131749).
    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let owned: Vec<(String, String)> =
            pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k: &str| owned.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())
    }

    #[test]
    fn a_project_resolves_to_its_own_store_not_the_users() {
        let d = dir();
        let home = d.path().join("home");
        let proj = home.join("work/alpha");
        create_dir_all(proj.join(".hexa")).unwrap();
        let env = env_of(&[("HOME", home.to_str().unwrap())]);

        assert_eq!(
            resolve_memory_dir(MemoryScope::Project, &env, &proj),
            proj.join(".hexa")
        );
        assert_eq!(
            resolve_memory_dir(MemoryScope::Shared, &env, &proj),
            home.join(".hexa"),
            "the shared scope stays the user store, from anywhere"
        );
    }

    #[test]
    fn the_project_root_is_found_by_walking_up_from_the_working_directory() {
        let d = dir();
        let home = d.path().join("home");
        let proj = home.join("work/alpha");
        let deep = proj.join("crates/hexa-exec/src");
        create_dir_all(&deep).unwrap();
        create_dir_all(proj.join(".hexa")).unwrap();
        let env = env_of(&[("HOME", home.to_str().unwrap())]);

        assert_eq!(
            resolve_memory_dir(MemoryScope::Project, &env, &deep),
            proj.join(".hexa"),
            "a nested directory belongs to the project above it"
        );
    }

    #[test]
    fn outside_a_project_the_store_is_the_users() {
        let d = dir();
        let home = d.path().join("home");
        let loose = home.join("scratch");
        create_dir_all(&loose).unwrap();
        let env = env_of(&[("HOME", home.to_str().unwrap())]);

        assert_eq!(
            resolve_memory_dir(MemoryScope::Project, &env, &loose),
            home.join(".hexa"),
            "a directory that was never scaffolded keeps the old behaviour"
        );
    }

    #[test]
    fn the_ascent_stops_at_home_so_a_stray_dot_hexa_above_it_is_not_a_project() {
        // `/tmp/.hexa` left by somebody else's run must not capture every
        // directory under it, or the scope is decided by a neighbour.
        let d = dir();
        create_dir_all(d.path().join(".hexa")).unwrap();
        let home = d.path().join("home");
        let loose = home.join("scratch");
        create_dir_all(&loose).unwrap();
        let env = env_of(&[("HOME", home.to_str().unwrap())]);

        assert_eq!(
            resolve_memory_dir(MemoryScope::Project, &env, &loose),
            home.join(".hexa")
        );
    }

    #[test]
    fn hexa_home_overrides_both_scopes_because_explicit_wins() {
        let d = dir();
        let proj = d.path().join("alpha");
        create_dir_all(proj.join(".hexa")).unwrap();
        let over = d.path().join("elsewhere");
        let env = env_of(&[("HOME", d.path().to_str().unwrap()), ("HEXA_HOME", over.to_str().unwrap())]);

        assert_eq!(resolve_memory_dir(MemoryScope::Project, &env, &proj), over);
        assert_eq!(resolve_memory_dir(MemoryScope::Shared, &env, &proj), over);
    }

    #[test]
    fn hexa_project_root_names_the_project_when_the_cwd_is_elsewhere() {
        let d = dir();
        let home = d.path().join("home");
        let proj = home.join("alpha");
        let away = home.join("beta");
        create_dir_all(proj.join(".hexa")).unwrap();
        create_dir_all(&away).unwrap();
        let env = env_of(&[
            ("HOME", home.to_str().unwrap()),
            ("HEXA_PROJECT_ROOT", proj.to_str().unwrap()),
        ]);

        assert_eq!(resolve_memory_dir(MemoryScope::Project, &env, &away), proj.join(".hexa"));
    }

    #[test]
    fn two_stores_do_not_see_each_others_entries() {
        let a = dir();
        let b = dir();
        memory_put_in(a.path(), "adr:0007:why", "Postgres over SQLite").unwrap();

        assert_eq!(
            memory_get_in(a.path(), "adr:0007:why").as_deref(),
            Some("Postgres over SQLite")
        );
        assert_eq!(
            memory_get_in(b.path(), "adr:0007:why"),
            None,
            "the same key in another project is a different question"
        );
        assert!(memory_entries_in(b.path(), 10).is_empty());
        assert!(memory_search_in(b.path(), "Postgres").is_empty());
    }

    #[test]
    fn newest_wins_and_a_tombstone_hides_the_key_within_one_store() {
        let d = dir();
        memory_put_in(d.path(), "lesson:x", "first").unwrap();
        memory_put_in(d.path(), "lesson:x", "second").unwrap();
        assert_eq!(memory_get_in(d.path(), "lesson:x").as_deref(), Some("second"));

        assert!(memory_delete_in(d.path(), "lesson:x").unwrap());
        assert_eq!(memory_get_in(d.path(), "lesson:x"), None);
        assert!(memory_entries_in(d.path(), 10).is_empty());
        assert!(!memory_delete_in(d.path(), "lesson:x").unwrap(), "a second delete is a miss");
    }

    #[test]
    fn an_empty_key_is_refused() {
        let d = dir();
        assert!(memory_put_in(d.path(), "   ", "value").is_err());
    }
}
