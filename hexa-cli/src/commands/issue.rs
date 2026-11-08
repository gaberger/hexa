//! `hexa issue file|sync`: a recorded key becomes one issue on the
//! repository's forge, deduplicated by the marker in its body
//! (ADR-2611031914 §3, §4, §6).
//!
//! Without `--send` nothing is sent: a dry run never builds an adapter, so it
//! cannot. With it, the order is fixed: the key is checked, the secret scan
//! runs, the target lock is taken, and only then is the forge asked.

use std::path::Path;
use std::time::Duration;

use clap::Subcommand;
use colored::Colorize;
use hexa_core::ports::forge::{issue_from_memory, valid_key, IssueDraft};
use hexa_exec::forge_filing::{self, FileError, Filed};
use hexa_exec::ports::{MemoryScope, MemoryStore, TargetLock};

use super::forge::{blocking, front, Front};

/// How long `--send` waits for another hexa filing on the same target.
const LOCK_WAIT: Duration = Duration::from_secs(120);
/// How many memory entries `sync` reads in its one snapshot.
const SNAPSHOT: usize = 5_000;

#[derive(Subcommand, Debug)]
pub enum IssueAction {
    /// File one recorded key (`bug:<x>`, `gap:<id>`) as an issue. Prints it and sends nothing without --send
    File {
        key: String,
        #[arg(long)]
        send: bool,
    },
    /// File every `bug:` and `gap:` key (or those under --prefix) that has no `issue:` entry. Sends nothing without --send
    Sync {
        #[arg(long)]
        prefix: Option<String>,
        #[arg(long)]
        send: bool,
    },
}

pub async fn run(action: IssueAction) -> anyhow::Result<()> {
    match action {
        IssueAction::File { key, send } => blocking(move || file(&key, send)).await,
        IssueAction::Sync { prefix, send } => blocking(move || sync(prefix.as_deref(), send)).await,
    }
}

/// A key's text: its memory value, else for `gap:<id>` the gap's `what`.
fn value_of(key: &str) -> Option<String> {
    if let Some(v) = hexa_exec::memory().get(MemoryScope::Project, key) {
        return Some(v);
    }
    let id = key.strip_prefix("gap:")?;
    hexa_core::ports::guide::find(id)
        .or_else(|| super::gap::read().1.into_iter().rev().find(|g| g.id == id))
        .map(|g| g.what)
}

fn draft_for(f: &Front, key: &str, value: &str) -> IssueDraft {
    issue_from_memory(key, value, &super::loop_cmd::project_name(&f.root))
}

fn print_draft(f: &Front, d: &IssueDraft) {
    println!("  target: {}", f.describe());
    println!("  memory: {}", hexa_exec::memory().path(MemoryScope::Project).display());
    println!("  title:  {}", d.title.bold());
    println!("  body:");
    for l in d.body.lines() {
        println!("    {l}");
    }
}

fn file(key: &str, send: bool) -> anyhow::Result<()> {
    if !valid_key(key) {
        anyhow::bail!("`{key}` is not a key hexa files: it needs a `:` and only [A-Za-z0-9:_./-], with no `--`");
    }
    let f = front()?;
    let value = value_of(key).ok_or_else(|| anyhow::anyhow!("no memory key `{key}` (and no gap by that id); `hexa memory store {key} \"…\"` records one"))?;
    let d = draft_for(&f, key, &value);
    f.scan(&d.title, &d.body)?;
    let link = hexa_exec::memory().get(MemoryScope::Project, &forge_filing::link_key(key));
    if !send {
        println!("{} {} (dry run: nothing sent)", "\u{2b21} issue".cyan().bold(), key.bold());
        print_draft(&f, &d);
        if let Some(url) = link {
            println!("  recorded as filed: {url}");
        }
        println!("\n  `hexa issue file {key} --send` files it.");
        return Ok(());
    }
    let forge = f.forge()?;
    let _guard = hexa_exec::target_lock().acquire(&f.target, LOCK_WAIT).map_err(|e| anyhow::anyhow!(e))?;
    let r = forge_filing::file_issue(forge.as_ref(), &hexa_exec::memory(), &d);
    drop(forge);
    report(key, r).map(|_| ())
}

/// Print one key's outcome. `Ok(true)` when the forge was asked and answered.
fn report(key: &str, r: Result<Filed, FileError>) -> anyhow::Result<Filed> {
    match r {
        Ok(Filed::Found(url)) => {
            println!("  {} {key}: already filed, nothing opened: {url}", "\u{2713}".green());
            Ok(Filed::Found(url))
        }
        Ok(Filed::Opened(url)) => {
            println!("  {} {key}: filed {url}", "\u{2713}".green());
            Ok(Filed::Opened(url))
        }
        Err(FileError::Forge(e)) => anyhow::bail!("{key}: {e}"),
        Err(FileError::Unconfirmed(e)) => anyhow::bail!("{key}: {e}; nothing was confirmed created; rerunning is safe"),
        Err(FileError::NotRecorded { filed, why }) => {
            anyhow::bail!("{key}: on the forge at {}, but the link was not recorded ({why}); rerun to record it", filed.url())
        }
    }
}

/// Whether `path` lies inside `root`, both resolved.
fn inside(path: &Path, root: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let dir = path.parent().unwrap_or(path);
    canon(dir).starts_with(canon(root))
}

fn sync(prefix: Option<&str>, send: bool) -> anyhow::Result<()> {
    let f = front()?;
    let mem = hexa_exec::memory();
    let store = mem.path(MemoryScope::Project);
    let entries = mem.entries(MemoryScope::Project, SNAPSHOT);
    let keys = forge_filing::unfiled_keys(&entries, prefix);
    let value = |k: &str| entries.iter().find(|(e, _)| e == k).map(|(_, v)| v.clone()).unwrap_or_default();

    println!("{} {} unfiled key(s) on {}", "\u{2b21} issue sync".cyan().bold(), keys.len(), f.describe());
    println!("  memory: {}", store.display());
    if !send {
        for k in &keys {
            println!("  {}  {}", k.bold(), draft_for(&f, k, &value(k)).title);
        }
        println!("  Gaps that are only in hexa's own list are not synced; `hexa issue file gap:<id>` files one.");
        if !keys.is_empty() {
            println!("\n  `hexa issue sync --send` files them (dry run: nothing sent).");
        }
        return Ok(());
    }
    // Publishing a shared store would file keys from every project.
    if std::env::var_os("HEXA_HOME").is_some() || !inside(&store, &f.root) {
        anyhow::bail!(
            "refusing to sync from {}: it is not this repository's own store, and this would publish keys from a shared store. \
             File keys one at a time with `hexa issue file <key> --send`",
            store.display()
        );
    }
    if keys.is_empty() {
        return Ok(());
    }
    let forge = f.forge()?;
    let _guard = hexa_exec::target_lock().acquire(&f.target, LOCK_WAIT).map_err(|e| anyhow::anyhow!(e))?;
    let (mut filed, mut found, mut refused) = (0, 0, 0);
    let mut abort: Option<anyhow::Error> = None;
    for k in &keys {
        if mem.get(MemoryScope::Project, &forge_filing::link_key(k)).is_some() {
            continue; // another hexa recorded it while this one waited
        }
        let d = draft_for(&f, k, &value(k));
        if let Err(e) = f.scan(&d.title, &d.body) {
            println!("  {} {k}: {e}", "\u{2717}".red());
            refused += 1;
            continue;
        }
        match report(k, forge_filing::file_issue(forge.as_ref(), &mem, &d)) {
            Ok(Filed::Opened(_)) => filed += 1,
            Ok(Filed::Found(_)) => found += 1,
            Err(e) => {
                abort = Some(e);
                break;
            }
        }
    }
    drop(forge);
    println!("  {filed} filed, {found} found already, {refused} refused");
    if let Some(e) = abort {
        return Err(e.context("sync stopped; nothing after this key was sent"));
    }
    if refused > 0 {
        anyhow::bail!("{refused} key(s) were refused");
    }
    Ok(())
}
