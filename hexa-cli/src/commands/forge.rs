//! `hexa forge add|status`: the hosts hexa knows a forge on, and whether this
//! repository's forge answers (ADR-2611031914 §2). Also the front every forge
//! verb shares: the target from `origin`, the `gh` on PATH, the secret scan,
//! and running a verb off the async runtime.
//!
//! The target is read from the RAW `remote.origin.url`: `git remote get-url`
//! applies `insteadOf`, which can turn the forge's URL into a local path.

use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{Subcommand, ValueEnum};
use colored::Colorize;
use hexa_core::ports::forge::{
    forge_base_url, forge_for_remote, looks_secret, remote_host, remote_password, valid_env_name, ForgeHost, ForgeKind,
    ForgeTarget,
};
use hexa_exec::ports::{ForgeHosts, MemoryScope, MemoryStore};

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum KindArg {
    Github,
    Forgejo,
}

#[derive(Subcommand, Debug)]
pub enum ForgeAction {
    /// Record a forge host: its kind and the NAME of the variable holding its token. Never a token
    Add {
        /// The host as the remote names it, e.g. git.local or 127.0.0.1:3000
        host: String,
        #[arg(long, value_enum)]
        kind: KindArg,
        /// The environment variable that holds the token (Forgejo)
        #[arg(long = "token-env")]
        token_env: Option<String>,
    },
    /// This repository's forge: the target, whether it answers, whether the credential is accepted. Writes nothing
    Status,
}

pub async fn run(action: ForgeAction) -> anyhow::Result<()> {
    match action {
        ForgeAction::Add { host, kind, token_env } => add(&host, kind, token_env),
        ForgeAction::Status => blocking(status).await,
    }
}

/// Run `f` on a blocking thread. The forge adapters are synchronous, and a
/// blocking HTTP client must be built and dropped off the runtime's threads.
pub(crate) async fn blocking<T: Send + 'static>(f: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> anyhow::Result<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| anyhow::anyhow!("the forge task stopped: {e}"))?
}

fn add(host: &str, kind: KindArg, token_env: Option<String>) -> anyhow::Result<()> {
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() || host.contains("://") || host.contains('/') || host.contains('@') {
        anyhow::bail!("give the host alone, as the remote names it (e.g. git.local), not a URL");
    }
    let kind = match kind {
        KindArg::Github => ForgeKind::Github,
        KindArg::Forgejo => ForgeKind::Forgejo,
    };
    if host == "github.com" && kind != ForgeKind::Github {
        anyhow::bail!("github.com is GitHub; it needs no entry");
    }
    if let Some(name) = &token_env {
        if looks_secret(name, None).is_some() || !valid_env_name(name) {
            anyhow::bail!("--token-env takes the NAME of an environment variable ([A-Za-z_][A-Za-z0-9_]*), never a token");
        }
    }
    if kind == ForgeKind::Forgejo && token_env.is_none() {
        anyhow::bail!("a forgejo host needs --token-env <NAME>: the variable its token is read from at call time");
    }
    let hosts = hexa_exec::forge_hosts();
    hosts.upsert(ForgeHost { host: host.clone(), kind, token_env: token_env.clone() }).map_err(|e| anyhow::anyhow!(e))?;
    println!("{} {} recorded as {}", "\u{2713}".green(), host.bold(), kind_name(kind));
    if let Some(n) = token_env {
        println!("  token read from ${n} when used; never written");
    }
    println!("  {}", hosts.path().display().to_string().dimmed());
    Ok(())
}

pub(crate) fn kind_name(k: ForgeKind) -> &'static str {
    match k {
        ForgeKind::Github => "github",
        ForgeKind::Forgejo => "forgejo",
    }
}

/// `git <args>` in `dir`: stdout trimmed, or `None` when it fails.
pub(crate) fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).current_dir(dir).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// This repository's forge, from `origin` as written in the config.
pub(crate) struct Front {
    pub root: PathBuf,
    /// The raw remote: never printed, it may hold a password.
    pub remote: String,
    pub target: ForgeTarget,
}

impl Front {
    pub fn describe(&self) -> String {
        format!("{} {} {}/{}", kind_name(self.target.kind), self.target.host, self.target.owner, self.target.repo)
    }

    /// The credentials a body is scanned for: the token's value, the
    /// remote's password, and `gh`'s token variables. (name, value)
    pub fn credentials(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut env = |n: &str| {
            if let Ok(v) = std::env::var(n) {
                if !v.is_empty() {
                    out.push((n.to_string(), v));
                }
            }
        };
        match self.target.kind {
            ForgeKind::Forgejo => {
                if let Some(n) = &self.target.token_env {
                    env(n);
                }
            }
            ForgeKind::Github => {
                env("GH_TOKEN");
                env("GITHUB_TOKEN");
            }
        }
        if let Some(pw) = remote_password(&self.remote) {
            out.push(("the remote URL's password".to_string(), pw));
        }
        out
    }

    /// Refuse if the title or body looks like it holds a secret. Names the
    /// place, never the text.
    pub fn scan(&self, title: &str, body: &str) -> anyhow::Result<()> {
        let creds = self.credentials();
        if let Some(why) = scan_text(title, &creds) {
            anyhow::bail!("refusing to send: the title looks like it holds a credential{why}; nothing was sent");
        }
        if let Some((line, why)) = scan_lines(body, &creds) {
            anyhow::bail!("refusing to send: line {line} of the body looks like it holds a credential{why}; nothing was sent");
        }
        Ok(())
    }

    /// The adapter, built here and only after every local check passed.
    pub fn forge(&self) -> anyhow::Result<Box<dyn hexa_exec::ports::ForgeBoth>> {
        let base = forge_base_url(&self.remote, &self.target).map_err(|e| anyhow::anyhow!(e))?;
        hexa_exec::forge(&self.target, &base, &gh_path()).map_err(|e| anyhow::anyhow!("{e}"))
    }
}

fn scan_text(text: &str, creds: &[(String, String)]) -> Option<String> {
    scan_lines(text, creds).map(|(_, why)| why)
}

/// The first line holding a credential shape across every credential, and
/// whether a credential's value (by name) is what matched.
fn scan_lines(text: &str, creds: &[(String, String)]) -> Option<(usize, String)> {
    let mut best: Option<(usize, String)> = looks_secret(text, None).map(|l| (l, String::new()));
    for (name, value) in creds {
        if let Some(l) = looks_secret(text, Some(value)) {
            if best.as_ref().is_none_or(|(b, _)| l < *b) {
                best = Some((l, format!(" (it matches the value of {name})")));
            }
        }
    }
    best
}

/// The `gh` the PATH names now, else plain `gh` (which then fails to start).
pub(crate) fn gh_path() -> PathBuf {
    std::env::var_os("PATH")
        .and_then(|p| std::env::split_paths(&p).map(|d| d.join("gh")).find(|c| c.is_file()))
        .unwrap_or_else(|| PathBuf::from("gh"))
}

/// The repository root and its forge, or why there is none.
pub(crate) fn front() -> anyhow::Result<Front> {
    let cwd = std::env::current_dir()?;
    let root = git(&cwd, &["rev-parse", "--show-toplevel"])
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("not in a git repository"))?;
    let remote = git(&root, &["config", "--get", "remote.origin.url"])
        .filter(|r| !r.is_empty())
        .ok_or_else(|| anyhow::anyhow!("this repository has no `origin` remote; the forge follows it"))?;
    let hosts = hexa_exec::forge_hosts().list().map_err(|e| anyhow::anyhow!(e))?;
    match forge_for_remote(&remote, &hosts) {
        Some(target) => Ok(Front { root, remote, target }),
        None => {
            let host = remote_host(&remote).unwrap_or_else(|| "the origin host".into());
            anyhow::bail!(
                "no forge is known for {host}. Add it with:\n  hexa forge add {host} --kind forgejo --token-env <VAR>\n(or `--kind github` for a GitHub host)"
            )
        }
    }
}

fn status() -> anyhow::Result<()> {
    let f = front()?;
    println!("{} {}", "\u{2b21} forge".cyan().bold(), f.describe());
    match &f.target.token_env {
        Some(n) => {
            let set = std::env::var(n).map(|v| !v.is_empty()).unwrap_or(false);
            println!("  token variable: {n} ({})", if set { "set" } else { "unset" });
        }
        None => println!("  credential: gh's own authentication"),
    }
    println!("  memory: {}", hexa_exec::memory().path(MemoryScope::Project).display());
    let forge = match f.forge() {
        Ok(forge) => forge,
        Err(e) => anyhow::bail!("{e:#}"),
    };
    let mut failed = false;
    match forge.answers() {
        Ok(v) => println!("  {} answers: {v}", "\u{2713}".green()),
        Err(e) => {
            failed = true;
            println!("  {} does not answer: {e}", "\u{2717}".red());
        }
    }
    match forge.credential() {
        Ok(v) => println!("  {} credential accepted: {v}", "\u{2713}".green()),
        Err(e) => {
            failed = true;
            println!("  {} credential: {e}", "\u{2717}".red());
        }
    }
    drop(forge);
    if failed {
        anyhow::bail!("the forge is not ready to send to");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_takes_the_lowest_line_across_credentials() {
        let creds = vec![("A".to_string(), "alpha-secret".to_string()), ("B".to_string(), "beta-secret".to_string())];
        let text = "x\nbeta-secret\nalpha-secret\nkey sk-abcdefghijklmnop";
        let (line, why) = scan_lines(text, &creds).unwrap();
        assert_eq!(line, 2);
        assert!(why.contains('B') && !why.contains("beta-secret"));
        assert_eq!(scan_lines("plain prose", &creds), None);
    }
}
