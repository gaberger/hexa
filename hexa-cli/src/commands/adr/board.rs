//! `hexa adr board`: every ADR, how far each is built, who is on it, and where
//! other checkouts differ (ADR-2611151030).
//!
//! The rules are the domain's (`ports::adr_board`). This file reads the
//! checkouts and the task ledger, and prints. It writes nothing except the
//! page `--html` names.

use std::collections::HashSet;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;

use colored::Colorize;
use hexa_core::ports::adr_board::{analyze, summarize, Analysis, State, Summary};
use hexa_core::ports::ledger::{Ledger, LedgerError};
use serde_json::{json, Value};
use tabled::Tabled;

use super::{collect_adrs, extract_adr_id, extract_title, find_adr_dir, gate_command, parse_adr_status};
use crate::commands::task_ledger;
use crate::fmt::{truncate, HexTable};

/// One ADR as one checkout holds it.
struct Entry {
    id: String,
    /// The file's name, to match against `git status`.
    file: String,
    title: String,
    content: String,
    analysis: Analysis,
    has_gate: bool,
}

/// Where ADRs were read from.
struct Source {
    path: PathBuf,
    /// A branch name, `(detached)`, or `--also` for a directory the operator named.
    label: String,
    adrs: Option<usize>,
    note: Option<String>,
}

/// What another checkout says about an ADR the current one also has (or lacks).
struct Elsewhere {
    path: PathBuf,
    label: String,
    analysis: Analysis,
    kind: &'static str,
    uncommitted: bool,
}

/// One repository's board.
struct Board {
    root: PathBuf,
    rows: Vec<Row>,
    summary: Summary,
    sources: Vec<Source>,
    warnings: Vec<String>,
}

pub async fn run(
    json: bool,
    text: bool,
    html: Option<PathBuf>,
    also: Vec<PathBuf>,
    products: Vec<PathBuf>,
    strict: bool,
) -> anyhow::Result<()> {
    let as_json = json || (!text && !std::io::stdout().is_terminal());
    let Some(adr_dir) = find_adr_dir() else {
        return refuse(as_json, no_adrs("there is no docs/adrs/ directory here or above"));
    };
    let main = match board_of(&adr_dir, &also).await {
        Ok(b) => b,
        Err(why) => return refuse(as_json, no_adrs(&why)),
    };

    // Another product is its own board; one that cannot be read is named, not dropped.
    let mut others: Vec<(PathBuf, Result<Board, String>)> = Vec::new();
    for dir in &products {
        let adrs = dir.join("docs").join("adrs");
        let b = if adrs.is_dir() {
            board_of(&adrs, &[]).await
        } else {
            Err(format!("{} has no docs/adrs/", dir.display()))
        };
        others.push((dir.clone(), b));
    }
    let generated = task_ledger::now();

    if let Some(file) = &html {
        let page = render_html(&main, &others, &generated);
        std::fs::write(file, page).map_err(|e| anyhow::anyhow!("cannot write {}: {e}", file.display()))?;
        if !as_json {
            println!("{} wrote {}", "\u{2b21}".cyan(), file.display());
        }
    }

    if as_json {
        let mut v = board_json(&main);
        v["generated"] = json!(generated);
        v["products"] = json!(others
            .iter()
            .map(|(dir, b)| match b {
                Ok(b) => {
                    let mut o = board_json(b);
                    o["path"] = json!(dir.display().to_string());
                    o
                }
                Err(why) => json!({"ok": false, "path": dir.display().to_string(), "error": why}),
            })
            .collect::<Vec<_>>());
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        print_text(&main, None);
        for (dir, b) in &others {
            match b {
                Ok(b) => print_text(b, Some(dir)),
                Err(why) => println!("\n  {} {}: {}", "\u{26a0}".yellow(), dir.display(), why),
            }
        }
    }

    let drift = main.summary.drift + others.iter().filter_map(|(_, b)| b.as_ref().ok()).map(|b| b.summary.drift).sum::<usize>();
    if strict && drift > 0 {
        crate::commands::events::exit(1);
    }
    Ok(())
}

/// Read one repository: its ADRs, the other checkouts of it, its ledger.
/// An `Err` is the reason there is nothing to show.
async fn board_of(adr_dir: &Path, also: &[PathBuf]) -> Result<Board, String> {
    let current = read_checkout(adr_dir).await?;
    if current.is_empty() {
        return Err(format!("{} holds no ADR", adr_dir.display()));
    }
    let root = adr_dir.parent().and_then(Path::parent).unwrap_or(adr_dir).to_path_buf();

    let mut warnings: Vec<String> = Vec::new();
    let mut sources = vec![Source { path: root.clone(), label: "current".into(), adrs: Some(current.len()), note: None }];

    let mut others: Vec<(PathBuf, String, Vec<Entry>)> = Vec::new();
    let mut candidates: Vec<(PathBuf, String)> = worktrees(&root);
    candidates.extend(also.iter().map(|p| (p.clone(), "--also".to_string())));
    for (path, label) in candidates {
        let dir = path.join("docs").join("adrs");
        let named = label == "--also";
        if !dir.is_dir() {
            if named {
                warnings.push(format!("--also {}: no docs/adrs/ there", path.display()));
            }
            sources.push(Source { path, label, adrs: None, note: Some("no docs/adrs/".into()) });
            continue;
        }
        match read_checkout(&dir).await {
            Ok(entries) => {
                // Another product is not another copy of this one.
                if named && !entries.iter().any(|e| current.iter().any(|c| c.id == e.id)) {
                    warnings.push(format!(
                        "--also {}: shares no ADR with this repository; if it is another product, use --product {}",
                        path.display(),
                        path.display()
                    ));
                    sources.push(Source { path, label, adrs: Some(entries.len()), note: Some("not read: no ADR in common".into()) });
                    continue;
                }
                sources.push(Source { path: path.clone(), label: label.clone(), adrs: Some(entries.len()), note: None });
                others.push((path, label, entries));
            }
            Err(e) => {
                warnings.push(format!("{}: {e}", path.display()));
                sources.push(Source { path, label, adrs: None, note: Some("unreadable".into()) });
            }
        }
    }

    let ledger = match task_ledger::read(&root) {
        Ok(l) => l,
        Err(e) => {
            warnings.push(format!("task ledger not read, so claims are unknown: {}", e.message));
            Ledger::default()
        }
    };

    let rows = build_rows(&current, &others, &ledger);
    let summary = summarize(current.iter().map(|e| &e.analysis));
    Ok(Board { root, rows, summary, sources, warnings })
}

fn board_json(b: &Board) -> Value {
    json!({
        "ok": true,
        "root": b.root.display().to_string(),
        "summary": b.summary,
        "adrs": b.rows.iter().map(Row::to_json).collect::<Vec<_>>(),
        "sources": b.sources.iter().map(|s| json!({
            "path": s.path.display().to_string(), "label": s.label, "adrs": s.adrs, "note": s.note,
        })).collect::<Vec<_>>(),
        "warnings": b.warnings,
    })
}

// ── reading ─────────────────────────────────────────────────────────────

async fn read_checkout(dir: &Path) -> Result<Vec<Entry>, String> {
    let adrs = collect_adrs(dir).await.map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    Ok(adrs
        .into_iter()
        .map(|(path, content)| {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("???").to_string();
            let status = parse_adr_status(&content).to_string();
            Entry {
                file: path.file_name().and_then(|f| f.to_str()).unwrap_or("").to_string(),
                id: extract_adr_id(&stem),
                title: extract_title(&path, &content),
                analysis: analyze(&status, &content),
                has_gate: gate_command(&content).is_some(),
                content,
            }
        })
        .collect())
}

/// Every git worktree except the one `root` is, as (path, branch label).
/// Paths are compared by realpath: `/home` and `/var/home` are one directory.
fn worktrees(root: &Path) -> Vec<(PathBuf, String)> {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let here = canon(root);
    let Ok(out) = Command::new("git").arg("-C").arg(root).args(["worktree", "list", "--porcelain"]).output() else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut path: Option<PathBuf> = None;
    let mut label = String::from("(detached)");
    let mut bare = false;
    let mut flush = |path: &mut Option<PathBuf>, label: &mut String, bare: &mut bool| {
        if let Some(p) = path.take() {
            if !*bare && canon(&p) != here {
                found.push((p, label.clone()));
            }
        }
        *label = "(detached)".into();
        *bare = false;
    };
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if line.is_empty() {
            flush(&mut path, &mut label, &mut bare);
        } else if let Some(p) = line.strip_prefix("worktree ") {
            path = Some(PathBuf::from(p));
        } else if let Some(b) = line.strip_prefix("branch ") {
            label = b.strip_prefix("refs/heads/").unwrap_or(b).to_string();
        } else if line == "bare" {
            bare = true;
        }
    }
    flush(&mut path, &mut label, &mut bare);
    found
}

/// File names under `docs/adrs/` with uncommitted changes in `path`.
/// Unreadable is an empty set only when git is absent; the caller treats it as "not known to be edited".
fn uncommitted_adrs(path: &Path) -> HashSet<String> {
    let Ok(out) = Command::new("git").arg("-C").arg(path).args(["status", "--porcelain", "-uall", "--", "docs/adrs"]).output() else {
        return HashSet::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.get(3..))
        .map(|p| p.trim().trim_matches('"'))
        .filter_map(|p| Path::new(p).file_name().and_then(|f| f.to_str()).map(str::to_string))
        .collect()
}

// ── joining ─────────────────────────────────────────────────────────────

struct Row {
    id: String,
    title: String,
    in_current: bool,
    analysis: Analysis,
    has_gate: bool,
    claimed_by: Vec<(String, Option<String>, String)>,
    elsewhere: Vec<Elsewhere>,
}

fn build_rows(current: &[Entry], others: &[(PathBuf, String, Vec<Entry>)], ledger: &Ledger) -> Vec<Row> {
    let claims = |id: &str| -> Vec<(String, Option<String>, String)> {
        let number = id.strip_prefix("ADR-").unwrap_or(id);
        if number.len() < 8 {
            return Vec::new();
        }
        ledger
            .tasks
            .iter()
            .filter(|t| t.status.is_active())
            .filter(|t| {
                t.name.contains(number)
                    || t.gate.as_deref().is_some_and(|g| g.contains(number))
                    || t.leases.iter().any(|l| l.contains(number))
            })
            .map(|t| (t.name.clone(), t.owner.clone(), t.status.to_string()))
            .collect()
    };

    let mut rows: Vec<Row> = current
        .iter()
        .map(|e| Row {
            id: e.id.clone(),
            title: e.title.clone(),
            in_current: true,
            analysis: e.analysis.clone(),
            has_gate: e.has_gate,
            claimed_by: claims(&e.id),
            elsewhere: Vec::new(),
        })
        .collect();

    for (path, label, entries) in others {
        let edited = uncommitted_adrs(path);
        for e in entries {
            let mine = current.iter().find(|c| c.id == e.id);
            let kind = match mine {
                Some(c) if c.analysis.status != e.analysis.status => "status",
                Some(c) if c.analysis.units.done != e.analysis.units.done || c.analysis.units.total != e.analysis.units.total => "progress",
                _ => "text",
            };
            let theirs = Elsewhere {
                path: path.clone(),
                label: label.clone(),
                analysis: e.analysis.clone(),
                kind,
                uncommitted: edited.contains(&e.file),
            };
            match rows.iter_mut().find(|r| r.id == e.id) {
                Some(row) => {
                    if mine.map(|c| c.content.as_str()) != Some(e.content.as_str()) {
                        row.elsewhere.push(theirs);
                    }
                }
                None => rows.push(Row {
                    id: e.id.clone(),
                    title: e.title.clone(),
                    in_current: false,
                    analysis: e.analysis.clone(),
                    has_gate: e.has_gate,
                    claimed_by: claims(&e.id),
                    elsewhere: vec![theirs],
                }),
            }
        }
    }
    rows
}

impl Row {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "title": self.title,
            "in_current": self.in_current,
            "status": self.analysis.status,
            "state": self.analysis.state,
            "units": self.analysis.units,
            "drift": self.analysis.drift,
            "gate": if self.has_gate { "recorded" } else { "none" },
            "claimed_by": self.claimed_by.iter().map(|(task, owner, status)| json!({
                "task": task, "owner": owner, "status": status,
            })).collect::<Vec<_>>(),
            "elsewhere": self.elsewhere.iter().map(|e| json!({
                "worktree": e.path.display().to_string(),
                "branch": e.label,
                "status": e.analysis.status,
                "state": e.analysis.state,
                "done": e.analysis.units.done,
                "total": e.analysis.units.total,
                "kind": e.kind,
                "uncommitted": e.uncommitted,
                "differs": true,
            })).collect::<Vec<_>>(),
        })
    }

    /// Where a row sits on the page: work in flight first, finished last.
    fn rank(&self) -> u8 {
        if !self.in_current {
            return 5;
        }
        match self.analysis.state {
            State::Building => 0,
            State::Accepted => 1,
            State::Proposed => 2,
            State::Built => 3,
            State::Retired => 4,
            State::Unknown => 4,
        }
    }

    fn notes(&self) -> Vec<String> {
        let mut n: Vec<String> = self.analysis.drift.iter().map(|d| format!("drift: {d}")).collect();
        for (_, owner, status) in &self.claimed_by {
            n.push(format!("{status} by {}", owner.as_deref().unwrap_or("(no owner)")));
        }
        // A text-only difference that is committed is a branch that is behind or ahead, not a collision.
        for e in &self.elsewhere {
            let tail = if e.uncommitted { " (uncommitted)" } else { "" };
            match (e.kind, e.uncommitted) {
                ("text", false) => {}
                ("text", true) => n.push(format!("{}: edited{tail}", e.label)),
                _ => n.push(format!(
                    "{}: {} {}/{}{tail}",
                    e.label, e.analysis.status, e.analysis.units.done, e.analysis.units.total
                )),
            }
        }
        if !self.in_current {
            n.push("not in this checkout".into());
        }
        n
    }
}

fn bar(percent: usize) -> String {
    let filled = (percent * 20) / 100;
    format!("{}{}", "\u{2588}".repeat(filled), "\u{2591}".repeat(20 - filled))
}

// ── text ────────────────────────────────────────────────────────────────

#[derive(Tabled)]
struct TextRow {
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "State")]
    state: String,
    #[tabled(rename = "Built")]
    built: String,
    #[tabled(rename = "Gate")]
    gate: String,
    #[tabled(rename = "Notes")]
    notes: String,
    #[tabled(rename = "Title")]
    title: String,
}

fn print_text(b: &Board, product: Option<&Path>) {
    let (rows, summary, warnings) = (&b.rows, &b.summary, &b.warnings);
    match product {
        None => println!("{} ADR board", "\u{2b21}".cyan()),
        Some(dir) => println!("\n{} ADR board: {}", "\u{2b21}".cyan(), dir.display()),
    }
    println!();
    println!(
        "  {} {} of {} units built \u{b7} {}%",
        bar(summary.percent),
        summary.built,
        summary.units,
        summary.percent
    );
    let states: Vec<String> = summary.by_state.iter().map(|(k, v)| format!("{v} {k}")).collect();
    println!("  {} ADRs: {}", summary.adrs, states.join(", "));
    if summary.drift > 0 {
        println!("  {}", format!("{} ADR(s) with a status behind the evidence", summary.drift).yellow());
    }
    println!();

    let mut order: Vec<&Row> = rows.iter().collect();
    order.sort_by_key(|r| (r.rank(), r.id.clone()));
    let table: Vec<TextRow> = order
        .into_iter()
        .map(|r| TextRow {
            id: r.id.clone(),
            state: r.analysis.state.name().to_string(),
            built: format!("{}/{}", r.analysis.units.done, r.analysis.units.total),
            gate: if r.has_gate { "yes".into() } else { "\u{2014}".into() },
            notes: r.notes().join("; "),
            title: truncate(r.title.trim_start_matches(&format!("{}: ", r.id)), 48),
        })
        .collect();
    println!("{}", HexTable::render(&table));

    if !warnings.is_empty() {
        println!();
        for w in warnings {
            println!("  {} {}", "\u{26a0}".yellow(), w);
        }
    }
}

// ── page ────────────────────────────────────────────────────────────────

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// One board as page markup: its total, its table, where it was read from.
fn html_section(title: &str, b: &Board) -> String {
    let mut order: Vec<&Row> = b.rows.iter().collect();
    order.sort_by_key(|r| (r.rank(), r.id.clone()));

    let mut body = String::new();
    for r in order {
        let phases: String = r
            .analysis
            .units
            .phases
            .iter()
            .map(|p| format!("<span class=\"ph {}\" title=\"{}\">{}</span>", if p.built { "on" } else { "off" }, esc(&p.title), esc(&p.id)))
            .collect::<Vec<_>>()
            .join(" ");
        let pct = (r.analysis.units.done * 100).checked_div(r.analysis.units.total).unwrap_or(0);
        let notes: String = r.notes().iter().map(|n| format!("<div>{}</div>", esc(n))).collect();
        body.push_str(&format!(
            "<tr class=\"{state}\"><td class=\"id\">{id}</td><td><span class=\"badge {state}\">{state}</span></td>\
             <td class=\"num\"><div class=\"meter\"><i style=\"width:{pct}%\"></i></div>{done}/{total}</td>\
             <td>{phases}</td><td>{gate}</td><td class=\"notes\">{notes}</td><td>{title}</td></tr>\n",
            state = r.analysis.state.name(),
            id = esc(&r.id),
            pct = pct,
            done = r.analysis.units.done,
            total = r.analysis.units.total,
            phases = phases,
            gate = if r.has_gate { "recorded" } else { "&mdash;" },
            notes = notes,
            title = esc(&r.title),
        ));
    }

    let s = &b.summary;
    let states: String = s
        .by_state
        .iter()
        .map(|(k, v)| format!("<span class=\"badge {k}\">{v} {k}</span> ", k = esc(k)))
        .collect();
    let src: String = b
        .sources
        .iter()
        .map(|x| {
            format!(
                "<li><b>{}</b> {} <span class=\"dim\">{}</span></li>",
                esc(&x.label),
                esc(&x.path.display().to_string()),
                match (&x.adrs, &x.note) {
                    (Some(n), Some(note)) => format!("{n} ADRs, {}", esc(note)),
                    (Some(n), None) => format!("{n} ADRs"),
                    (None, Some(note)) => esc(note),
                    _ => String::new(),
                }
            )
        })
        .collect();
    let warn: String = b.warnings.iter().map(|w| format!("<li>{}</li>", esc(w))).collect();

    format!(
        "<section><h2>{title}</h2>\n\
<div class=\"big\"><div class=\"total\">{percent}%</div><div class=\"meter\"><i style=\"width:{percent}%\"></i></div>\
<div>{built} of {units} units built over {adrs} ADRs</div><div style=\"margin-top:6px\">{states}</div></div>\n\
<div class=\"wrap\"><table><thead><tr><th>ADR</th><th>State</th><th>Built</th><th>Phases</th><th>Gate</th><th>Notes</th><th>Title</th></tr></thead><tbody>\n{body}</tbody></table></div>\n\
<details><summary class=\"dim\">Read from</summary><ul>{src}</ul></details>\n{warnblock}</section>\n",
        title = esc(title),
        percent = s.percent,
        built = s.built,
        units = s.units,
        adrs = s.adrs,
        states = states,
        body = body,
        src = src,
        warnblock = if warn.is_empty() { String::new() } else { format!("<h3>Warnings</h3><ul>{warn}</ul>\n") },
    )
}

fn render_html(main: &Board, others: &[(PathBuf, Result<Board, String>)], generated: &str) -> String {
    let name = |p: &Path| p.file_name().and_then(|f| f.to_str()).unwrap_or("repository").to_string();
    let mut sections = html_section(&name(&main.root), main);
    for (dir, b) in others {
        match b {
            Ok(b) => sections.push_str(&html_section(&name(dir), b)),
            Err(why) => sections.push_str(&format!(
                "<section><h2>{}</h2><p class=\"badge unknown\">not read: {}</p></section>\n",
                esc(&name(dir)),
                esc(why)
            )),
        }
    }
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>ADR board</title>\n<style>\n\
:root{{--bg:#fbfbfa;--fg:#1c1c1a;--dim:#6b6b66;--line:#dcdcd6;--on:#2f7d4f;--mid:#a86b00;--off:#9a9a94;--bad:#b3261e}}\n\
@media (prefers-color-scheme:dark){{:root{{--bg:#161614;--fg:#ecece8;--dim:#9a9a94;--line:#34342f;--on:#58b77c;--mid:#e0a43a;--off:#6b6b66;--bad:#f2766d}}}}\n\
body{{margin:0;padding:24px 16px;background:var(--bg);color:var(--fg);font:15px/1.45 system-ui,sans-serif}}\n\
main{{max-width:1200px;margin:0 auto}}h1{{font-size:22px;margin:0 0 4px}}h2{{font-size:18px;margin:32px 0 8px}}.dim{{color:var(--dim)}}\n\
.total{{font-size:34px;font-weight:600}}.meter{{height:8px;background:var(--line);border-radius:4px;overflow:hidden;margin-bottom:4px;min-width:90px}}\n\
.meter i{{display:block;height:100%;background:var(--on)}}.big .meter{{height:14px;max-width:520px}}\n\
.wrap{{overflow-x:auto;margin-top:12px}}table{{border-collapse:collapse;width:100%;min-width:860px}}\n\
th,td{{text-align:left;vertical-align:top;padding:8px 10px;border-bottom:1px solid var(--line)}}th{{font-size:12px;text-transform:uppercase;letter-spacing:.04em;color:var(--dim)}}\n\
td.id{{font-family:ui-monospace,monospace;white-space:nowrap}}td.notes{{font-size:13px;color:var(--dim)}}\n\
.badge{{display:inline-block;padding:1px 8px;border-radius:10px;border:1px solid var(--line);font-size:12px}}\n\
.badge.building{{color:var(--mid);border-color:var(--mid)}}.badge.built{{color:var(--on);border-color:var(--on)}}.badge.unknown{{color:var(--bad);border-color:var(--bad)}}\n\
.ph{{font-family:ui-monospace,monospace;font-size:12px;padding:0 5px;border-radius:3px;border:1px solid var(--line)}}\n\
.ph.on{{color:var(--on);border-color:var(--on)}}.ph.off{{color:var(--off)}}tr.retired{{opacity:.55}}\n\
ul{{padding-left:18px}}\n</style></head><body><main>\n\
<h1>ADR board</h1><div class=\"dim\">as of {generated}</div>\n\
<p class=\"dim\">Built means the ADR says so: a <code>**Built</code> line on a phase or the header. Accepted is a decision, not a build.</p>\n\
{sections}</main></body></html>\n",
        generated = esc(generated),
        sections = sections,
    )
}

// ── refusal ─────────────────────────────────────────────────────────────

fn no_adrs(why: &str) -> LedgerError {
    LedgerError::new(
        "no_adrs",
        format!("no ADRs to show: {why}"),
        "Run from a repository that has docs/adrs/ADR-*.md; a board of zero is not a board.".into(),
        None,
        false,
    )
}

fn refuse(as_json: bool, e: LedgerError) -> anyhow::Result<()> {
    if as_json {
        let mut v = serde_json::to_value(&e)?;
        v["ok"] = json!(false);
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        eprintln!("\u{2717} {}\n  hint: {}\n  code: {}", e.message, e.hint, e.code);
    }
    crate::commands::events::exit(1);
}
