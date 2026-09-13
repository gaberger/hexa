//! `hexa memory` — the lessons the agent loop reads before it starts work.
//!
//! Backed by `~/.hexa/memory.jsonl` through `hexa_exec::local_store`
//! (ADR-2608241500). It was the `hexflo_memory` SpacetimeDB table, reached
//! through the daemon over HTTP — so writing down a lesson needed a database
//! and a control plane to be up.
//!
//! Key prefixes are conventional, not enforced: `lesson:` (don't repeat
//! this), `gap:` (known issue), `project:` (in-flight context), `decision:`
//! (a recorded choice). `hexa-exec`'s context assembly reads the same feed, so
//! a lesson stored here reaches the next `hexa do`.
//!
//! # What was removed
//!
//! `sync-check` and `validate` both existed to prove that two agents on two
//! hosts saw the same row through SpacetimeDB. There is one agent and one
//! machine now, so the property they asserted is not a property any more.

use clap::Subcommand;
use colored::Colorize;

use hexa_exec::local_store;

#[derive(Subcommand)]
pub enum MemoryAction {
    /// Store a key-value pair
    Store {
        /// Key name, e.g. `lesson:trace-consumers`
        key: String,
        /// Value to store
        value: String,
    },
    /// Retrieve a value by key
    Get {
        /// Key name
        key: String,
    },
    /// Search stored memory by substring, over keys and values
    Search {
        /// Search query
        query: String,
    },
    /// List every stored entry
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Delete an entry by key
    Delete {
        /// Key name
        key: String,
    },
}

/// Entries listed by `hexa memory list`.
const LIST_LIMIT: usize = 500;

pub async fn run(action: MemoryAction) -> anyhow::Result<()> {
    match action {
        MemoryAction::Store { key, value } => {
            local_store::memory_put(&key, &value).map_err(|e| anyhow::anyhow!(e))?;
            println!("{} Memory stored", "\u{2b21}".green());
            println!("  Key:   {}", key.bold());
            println!("  Value: {} bytes", value.len());
        }
        MemoryAction::Get { key } => match local_store::memory_get(&key) {
            Some(value) => {
                println!("{} Memory lookup", "\u{2b21}".cyan());
                println!("  Key:   {}", key.bold());
                println!("  Value: {}", value);
            }
            None => {
                // Keys are namespaced, `lesson:quadratic-cost-walks`. A bare
                // namespace names every entry under it.
                let prefix = format!("{}:", key.trim_end_matches(':'));
                let under: Vec<(String, String)> = local_store::memory_entries(LIST_LIMIT)
                    .into_iter()
                    .filter(|(k, _)| k.starts_with(&prefix))
                    .collect();
                if under.is_empty() {
                    println!("{} Key '{}' not found", "\u{2b21}".yellow(), key);
                } else {
                    println!("{} {} entries under '{}'", "\u{2b21}".cyan(), under.len(), prefix.bold());
                    for (k, v) in under {
                        println!("  {}\n    {}", k.bold(), v);
                    }
                }
            }
        },
        MemoryAction::Search { query } => {
            let results = local_store::memory_search(&query);
            if results.is_empty() {
                println!("{} No results for '{}'", "\u{2b21}".dimmed(), query);
                return Ok(());
            }
            println!(
                "{} Memory search: '{}' ({} results)",
                "\u{2b21}".cyan(),
                query.bold(),
                results.len()
            );
            println!();
            for (key, value) in &results {
                println!("  {}", key.bold());
                for line in value.lines() {
                    println!("    {line}");
                }
            }
        }
        MemoryAction::List { json } => {
            let all = local_store::memory_entries(LIST_LIMIT);
            if json {
                let rows: Vec<_> = all
                    .iter()
                    .map(|(k, v)| serde_json::json!({ "key": k, "value": v }))
                    .collect();
                println!("{}", serde_json::to_string_pretty(&rows)?);
                return Ok(());
            }
            if all.is_empty() {
                println!("{} No memory entries yet", "\u{2b21}".dimmed());
                return Ok(());
            }
            println!("{} Memory ({} entries)", "\u{2b21}".cyan(), all.len());
            println!();
            // The whole value, on its own lines. A list that clips every
            // lesson at 57 characters is a list nobody can read.
            for (key, value) in &all {
                println!("  {}", key.bold());
                for line in value.lines() {
                    println!("    {line}");
                }
                println!();
            }
        }
        MemoryAction::Delete { key } => {
            if local_store::memory_delete(&key).map_err(|e| anyhow::anyhow!(e))? {
                println!("{} Deleted '{}'", "\u{2b21}".green(), key.bold());
            } else {
                println!("{} Key '{}' not found", "\u{2b21}".yellow(), key);
            }
        }
    }
    Ok(())
}

