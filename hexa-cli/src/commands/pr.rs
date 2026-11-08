//! `hexa pr open`: a pull request for the current branch, once it is on the
//! remote at HEAD (ADR-2611031914 §5). hexa never pushes: the only git
//! network command here is `ls-remote`, and a branch that is missing from the
//! remote, or at another commit, is refused with "push it first".

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use clap::Args;
use colored::Colorize;
use hexa_core::ports::forge::{pr_body, PrDraft};
use hexa_exec::forge_filing::{self, Opened};

use super::forge::{blocking, front, git};

#[derive(Args, Debug)]
pub struct PrOpenArgs {
    /// The branch to merge into (default: the remote's default branch)
    #[arg(long)]
    pub base: Option<String>,
    /// Open it as a draft
    #[arg(long)]
    pub draft: bool,
    /// A file whose contents replace the generated body
    #[arg(long)]
    pub body: Option<PathBuf>,
    /// Open it. Without this, print it and send nothing
    #[arg(long)]
    pub send: bool,
}

#[derive(clap::Subcommand, Debug)]
pub enum PrAction {
    /// Open a pull request for the current, pushed branch. Prints it and sends nothing without --send
    Open(PrOpenArgs),
}

pub async fn run(action: PrAction) -> anyhow::Result<()> {
    match action {
        PrAction::Open(a) => blocking(move || open_pr_flow(&a, None, None)).await,
    }
}

/// How long one `git ls-remote` may take.
const LS_REMOTE_TIMEOUT: Duration = Duration::from_secs(30);

/// `git ls-remote <args>` (the caller names `origin`, after any flag), never
/// prompting, killed after 30 s.
fn ls_remote(root: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.arg("ls-remote").args(args).current_dir(root).env("GIT_TERMINAL_PROMPT", "0");
    if std::env::var_os("GIT_SSH_COMMAND").is_none() {
        cmd.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut out = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut out, &mut s);
        s
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if start.elapsed() > LS_REMOTE_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => return None,
        }
    };
    let text = reader.join().ok()?;
    status.success().then_some(text)
}

/// The commit `ref_name` is at on the remote: an exact ref match only.
fn remote_sha(root: &Path, ref_name: &str) -> Option<String> {
    let out = ls_remote(root, &["origin", ref_name])?;
    out.lines().find_map(|l| {
        let (sha, r) = l.split_once('\t')?;
        (r.trim() == ref_name).then(|| sha.trim().to_string())
    })
}

/// The remote's default branch, from its symbolic HEAD.
fn remote_default(root: &Path) -> Option<String> {
    let out = ls_remote(root, &["--symref", "origin", "HEAD"])?;
    out.lines().find_map(|l| l.strip_prefix("ref: ")?.split('\t').next()?.strip_prefix("refs/heads/").map(str::to_string))
}

/// The ADR a commit subject names whose file is in `docs/adrs`, its gate, and
/// the loop stage of the one session for it. Anything unsure is `None`.
fn evidence(root: &Path, commits: &[String]) -> (Option<String>, Option<String>, Option<String>) {
    let adrs = root.join("docs").join("adrs");
    let file_of = |id: &str| -> Option<PathBuf> {
        std::fs::read_dir(&adrs).ok()?.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(id) && n.ends_with(".md"))
        })
    };
    let ids = commits.iter().flat_map(|c| {
        c.match_indices("ADR-")
            .filter_map(|(i, _)| {
                let digits = c.get(i + 4..i + 14)?;
                let after = c[i + 14..].chars().next();
                (digits.chars().all(|d| d.is_ascii_digit()) && !after.is_some_and(|a| a.is_ascii_digit()))
                    .then(|| format!("ADR-{digits}"))
            })
            .collect::<Vec<_>>()
    });
    let Some((adr, path)) = ids.into_iter().find_map(|id| file_of(&id).map(|p| (id, p))) else {
        return (None, None, None);
    };
    let gate = std::fs::read_to_string(&path).ok().and_then(|md| super::adr::gate_command(&md));
    let last = std::fs::read_to_string(root.join(".hexa").join("loop.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| {
            let sessions = v.get("sessions")?.as_object()?;
            let mine: Vec<&serde_json::Value> = sessions
                .values()
                .filter(|s| {
                    s.get("adr").and_then(|a| a.as_str()) == Some(adr.as_str())
                        || (gate.is_some() && s.get("gate").and_then(|g| g.as_str()) == gate.as_deref())
                })
                .collect();
            match mine.as_slice() {
                [one] => one.get("stage").and_then(|s| s.as_str()).map(|s| format!("loop stage `{s}`")),
                _ => None,
            }
        });
    (Some(adr), gate, last)
}

/// Text that would notify people or close issues once sent.
fn mentions(text: &str) -> bool {
    let mut prev = ' ';
    for (i, c) in text.char_indices() {
        if (c == '@' || c == '#') && !prev.is_alphanumeric() {
            let next = text[i + 1..].chars().next();
            if next.is_some_and(|n| if c == '@' { n.is_alphanumeric() } else { n.is_ascii_digit() }) {
                return true;
            }
        }
        prev = c;
    }
    false
}

/// `hexa pr open`'s rules, shared with `hexa gap pr --send`: `title` and
/// `body` replace the generated ones when given.
pub(crate) fn open_pr_flow(a: &PrOpenArgs, title: Option<String>, body: Option<String>) -> anyhow::Result<()> {
    let f = front()?;
    let root = f.root.clone();
    let head = git(&root, &["symbolic-ref", "--short", "-q", "HEAD"])
        .filter(|b| !b.is_empty())
        .ok_or_else(|| anyhow::anyhow!("HEAD is detached; check out the branch the pull request is from"))?;

    // On the remote at HEAD, or refused: hexa never pushes.
    let remote_ref = match (git(&root, &["config", &format!("branch.{head}.remote")]), git(&root, &["config", &format!("branch.{head}.merge")])) {
        (Some(r), Some(m)) if r == "origin" && m.starts_with("refs/heads/") => m,
        _ => format!("refs/heads/{head}"),
    };
    let head_sha = git(&root, &["rev-parse", "HEAD"]).ok_or_else(|| anyhow::anyhow!("HEAD has no commit"))?;
    match remote_sha(&root, &remote_ref) {
        Some(sha) if sha == head_sha => {}
        Some(_) => anyhow::bail!("`{head}` is on origin at another commit; push `{head}` first: hexa never pushes"),
        None => anyhow::bail!("`{head}` is not on origin; push `{head}` first: hexa never pushes"),
    }
    let head_name = remote_ref.strip_prefix("refs/heads/").unwrap_or(&head).to_string();

    let base = match a.base.clone().or_else(|| remote_default(&root)) {
        Some(b) => b,
        None => anyhow::bail!("could not read origin's default branch; pass --base <branch>"),
    };
    if base == head_name {
        anyhow::bail!("the branch `{head_name}` is the base itself; check out the branch the pull request is from, or pass --base");
    }
    let base_sha = remote_sha(&root, &format!("refs/heads/{base}"))
        .ok_or_else(|| anyhow::anyhow!("`{base}` is not a branch on origin; pass --base <branch>"))?;
    let from = if git(&root, &["cat-file", "-e", &format!("{base_sha}^{{commit}}")]).is_some() {
        base_sha
    } else {
        git(&root, &["rev-parse", "--verify", "-q", &format!("origin/{base}")])
            .ok_or_else(|| anyhow::anyhow!("origin's `{base}` is not here; fetch `{base}` first"))?
    };
    let log = git(&root, &["log", "--reverse", "--format=%s", &format!("{from}..HEAD")]).unwrap_or_default();
    let commits: Vec<String> = log.lines().map(str::to_string).filter(|l| !l.is_empty()).collect();
    if commits.is_empty() {
        anyhow::bail!("`{head_name}` has no commits that `{base}` lacks; there is nothing to open");
    }

    let title = title.unwrap_or_else(|| commits[0].clone());
    let body = match (&a.body, body) {
        (Some(path), _) => std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?,
        (None, Some(b)) => b,
        (None, None) => {
            let (adr, gate, last) = evidence(&root, &commits);
            pr_body(&commits, adr.as_deref(), gate.as_deref(), last.as_deref())
        }
    };
    f.scan(&title, &body)?;
    let d = PrDraft { head: head_name.clone(), base: base.clone(), title, body, draft: a.draft };

    if !a.send {
        println!("{} (dry run: nothing sent)", "\u{2b21} pr".cyan().bold());
        println!("  target: {}", f.describe());
        println!("  {} -> {}{}", d.head.bold(), d.base.bold(), if d.draft { " (draft)" } else { "" });
        println!("  title:  {}", d.title.bold());
        println!("  body:");
        for l in d.body.lines() {
            println!("    {l}");
        }
        if mentions(&d.title) || mentions(&d.body) {
            println!("  {} the text holds an @mention or #number: sending would notify people or link (or close) issues", "!".yellow());
        }
        println!("\n  add --send to open it.");
        return Ok(());
    }
    let forge = f.forge()?;
    let r = forge_filing::open_pr(forge.as_ref(), &d);
    drop(forge);
    match r.map_err(|e| anyhow::anyhow!("{e}"))? {
        Opened::Existing(url) => println!("  {} a pull request from `{}` is open already, nothing opened: {url}", "\u{2713}".green(), d.head),
        Opened::Created(url) => println!("  {} opened {url}", "\u{2713}".green()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mentions_find_people_and_issue_numbers_only() {
        assert!(mentions("thanks @alice"));
        assert!(mentions("Fixes #12"));
        assert!(!mentions("a@b.c email-ish and C# and #tag"));
    }
}
