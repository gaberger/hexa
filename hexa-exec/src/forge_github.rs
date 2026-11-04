//! GitHub through the `gh` CLI and its own authentication (ADR-2611031914 §1, §7). `gh` is the CLI's path, a recording stub in tests.
//!
//! Always `Command`, never a shell. Every call names `--repo`; a value is
//! passed as `--flag=value`, so a title starting with `-` cannot be read as a
//! flag; a body goes through a 0600 file, never argv, so `ps` cannot show it.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hexa_core::ports::forge::{marker_is_last_line, ForgeError, ForgeTarget, IssueDraft, IssueRef, PrDraft, PrRef};
use serde_json::Value;

use crate::ports::{Forge, ForgeStatus};

/// How long one `gh` call may run before it is killed.
const GH_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone)]
struct Listed {
    number: u64,
    url: String,
    open: bool,
    body: String,
}

/// GitHub through the `gh` CLI and its own authentication (ADR-2611031914 §1, §7). `gh` is the CLI's path, a recording stub in tests.
pub struct ForgeGithub {
    gh: PathBuf,
    owner: String,
    repo: String,
    host: String,
    listing: Mutex<Option<Vec<Listed>>>,
}

/// What a finished `gh` call left.
struct Ran {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// A body file: 0600, created fresh, removed when dropped.
struct BodyFile(PathBuf);

impl BodyFile {
    fn new(body: &str) -> Result<BodyFile, ForgeError> {
        use std::io::Write;
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let path = std::env::temp_dir().join(format!("hexa-body-{}-{nanos}.md", std::process::id()));
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&path).map_err(|e| ForgeError::Refused(format!("could not write the body file: {e}")))?;
        f.write_all(body.as_bytes()).map_err(|e| ForgeError::Refused(format!("could not write the body file: {e}")))?;
        Ok(BodyFile(path))
    }
}

impl Drop for BodyFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

impl ForgeGithub {
    pub fn new(target: &ForgeTarget, gh: &Path) -> ForgeGithub {
        ForgeGithub {
            gh: gh.to_path_buf(),
            owner: target.owner.clone(),
            repo: target.repo.clone(),
            host: target.host.clone(),
            listing: Mutex::new(None),
        }
    }

    /// `--repo`'s value: `o/r`, or `host/o/r` off github.com.
    fn repo_arg(&self) -> String {
        if self.host == "github.com" {
            format!("{}/{}", self.owner, self.repo)
        } else {
            format!("{}/{}/{}", self.host, self.owner, self.repo)
        }
    }

    fn scrub(text: &str) -> String {
        let mut t: String = text.chars().take(500).collect();
        for var in ["GH_TOKEN", "GITHUB_TOKEN", "GH_ENTERPRISE_TOKEN"] {
            if let Ok(v) = std::env::var(var) {
                if !v.is_empty() {
                    t = t.replace(&v, "<redacted>");
                }
            }
        }
        t.trim().to_string()
    }

    fn run(&self, args: &[String]) -> Result<Ran, ForgeError> {
        let mut cmd = Command::new(&self.gh);
        cmd.args(args)
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .env("NO_COLOR", "1")
            .env("GH_PAGER", "cat")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if self.host != "github.com" {
            cmd.env("GH_HOST", &self.host);
        }
        let mut child = cmd.spawn().map_err(|_| ForgeError::Unreachable(format!("gh not found at {}", self.gh.display())))?;
        let mut out = child.stdout.take();
        let mut err = child.stderr.take();
        let t_out = std::thread::spawn(move || {
            let mut s = String::new();
            if let Some(o) = out.as_mut() {
                let _ = o.read_to_string(&mut s);
            }
            s
        });
        let t_err = std::thread::spawn(move || {
            let mut s = String::new();
            if let Some(e) = err.as_mut() {
                let _ = e.read_to_string(&mut s);
            }
            s
        });
        let start = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break s,
                Ok(None) if start.elapsed() > GH_TIMEOUT => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(ForgeError::Unreachable(format!("gh did not finish within {}s", GH_TIMEOUT.as_secs())));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(e) => return Err(ForgeError::Unreachable(format!("gh: {e}"))),
            }
        };
        Ok(Ran { code: status.code(), stdout: t_out.join().unwrap_or_default(), stderr: t_err.join().unwrap_or_default() })
    }

    /// Run `gh`, and map a failure to the error it is.
    fn ok(&self, args: &[String]) -> Result<String, ForgeError> {
        let r = self.run(args)?;
        if r.code == Some(0) {
            return Ok(r.stdout);
        }
        if r.code == Some(4) || r.stderr.contains("gh auth login") {
            return Err(ForgeError::Unauthorized(format!("gh is not authenticated for {}", self.host)));
        }
        let msg = Self::scrub(&r.stderr);
        Err(ForgeError::Refused(if msg.is_empty() { format!("gh exited with {:?}", r.code) } else { msg }))
    }

    fn json(text: &str) -> Result<Value, ForgeError> {
        serde_json::from_str(text.trim()).map_err(|_| ForgeError::Refused("gh printed JSON hexa could not read".into()))
    }

    fn listed(v: &Value) -> Option<Listed> {
        if v.get("pull_request").is_some_and(|p| !p.is_null()) {
            return None;
        }
        let url = v.get("html_url").or_else(|| v.get("url"))?.as_str()?.to_string();
        Some(Listed {
            number: v.get("number")?.as_u64()?,
            url,
            open: v.get("state").and_then(Value::as_str).map(|s| s.eq_ignore_ascii_case("open")).unwrap_or(true),
            body: v.get("body").and_then(Value::as_str).unwrap_or("").to_string(),
        })
    }

    fn lowest_with_marker(list: &[Listed], key: &str) -> Option<IssueRef> {
        list.iter()
            .filter(|l| marker_is_last_line(&l.body, key))
            .min_by_key(|l| l.number)
            .map(|l| IssueRef { number: l.number, url: l.url.clone(), open: l.open })
    }

    /// Every issue, open and closed, through `gh api --paginate`: the proof.
    fn list_all(&self) -> Result<Vec<Listed>, ForgeError> {
        let path = format!("repos/{}/{}/issues?state=all&per_page=100", self.owner, self.repo);
        let mut args = vec!["api".to_string(), "--paginate".to_string()];
        if self.host != "github.com" {
            args.push(format!("--hostname={}", self.host));
        }
        args.push(path);
        let out = self.ok(&args)?;
        let mut all = Vec::new();
        for page in serde_json::Deserializer::from_str(&out).into_iter::<Value>() {
            match page {
                Ok(Value::Array(items)) => all.extend(items.iter().filter_map(Self::listed)),
                _ => return Err(ForgeError::Refused("gh api did not list issues; nothing was opened".into())),
            }
        }
        Ok(all)
    }

    /// The URL a `create` printed (its last line) and the number it ends in.
    fn created(out: &str) -> Result<(u64, String), ForgeError> {
        let url = out.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_string();
        let number = url.rsplit('/').next().and_then(|n| n.parse().ok());
        match number {
            Some(n) => Ok((n, url)),
            None => Err(ForgeError::Refused("gh created something but did not say where".into())),
        }
    }
}

impl Forge for ForgeGithub {
    fn find_issue(&self, key: &str) -> Result<Option<IssueRef>, ForgeError> {
        if let Some(cached) = self.listing.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            return Ok(Self::lowest_with_marker(cached, key));
        }
        let hint = self.ok(&[
            "issue".into(),
            "list".into(),
            format!("--repo={}", self.repo_arg()),
            "--state=all".into(),
            format!("--search=\"hexa-key: {key}\" in:body"),
            "--json=number,url,state,body".into(),
            "--limit=100".into(),
        ])?;
        if let Ok(Value::Array(items)) = Self::json(&hint) {
            let listed: Vec<Listed> = items.iter().filter_map(Self::listed).collect();
            if let Some(hit) = Self::lowest_with_marker(&listed, key) {
                return Ok(Some(hit));
            }
        }
        let all = self.list_all()?;
        let found = Self::lowest_with_marker(&all, key);
        *self.listing.lock().unwrap_or_else(|e| e.into_inner()) = Some(all);
        Ok(found)
    }

    fn open_issue(&self, d: &IssueDraft) -> Result<IssueRef, ForgeError> {
        let body = BodyFile::new(&d.body)?;
        let out = self.ok(&[
            "issue".into(),
            "create".into(),
            format!("--repo={}", self.repo_arg()),
            format!("--title={}", d.title),
            "--body-file".into(),
            body.0.display().to_string(),
        ]);
        // A failed create's outcome is unknown: forget the cached listing.
        let out = out.inspect_err(|_| *self.listing.lock().unwrap_or_else(|e| e.into_inner()) = None)?;
        let (number, url) = Self::created(&out)?;
        let r = IssueRef { number, url: url.clone(), open: true };
        if let Some(cached) = self.listing.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            cached.push(Listed { number, url, open: true, body: d.body.clone() });
        }
        Ok(r)
    }

    fn find_pr(&self, head: &str) -> Result<Option<PrRef>, ForgeError> {
        let out = self.ok(&[
            "pr".into(),
            "list".into(),
            format!("--repo={}", self.repo_arg()),
            format!("--head={head}"),
            "--state=open".into(),
            "--json=number,url,headRefName,headRepositoryOwner".into(),
        ])?;
        let items = match Self::json(&out)? {
            Value::Array(a) => a,
            _ => return Err(ForgeError::Refused("gh pr list did not print a list".into())),
        };
        let hit = items.iter().find(|p| {
            let same_ref = p.get("headRefName").and_then(Value::as_str) == Some(head);
            let owner = p.get("headRepositoryOwner").and_then(|o| o.get("login")).and_then(Value::as_str);
            same_ref && owner.is_none_or(|o| o.eq_ignore_ascii_case(&self.owner))
        });
        Ok(hit.and_then(|p| Some(PrRef { number: p.get("number")?.as_u64()?, url: p.get("url")?.as_str()?.to_string() })))
    }

    fn open_pr(&self, d: &PrDraft) -> Result<PrRef, ForgeError> {
        let body = BodyFile::new(&d.body)?;
        // `--head` always: gh never offers to push.
        let mut args: Vec<String> = vec![
            "pr".into(),
            "create".into(),
            "--repo".into(),
            self.repo_arg(),
            format!("--head={}", d.head),
            format!("--base={}", d.base),
            format!("--title={}", d.title),
            "--body-file".into(),
            body.0.display().to_string(),
        ];
        if d.draft {
            args.push("--draft".into());
        }
        let out = self.ok(&args)?;
        let (number, url) = Self::created(&out)?;
        Ok(PrRef { number, url })
    }
}

impl ForgeStatus for ForgeGithub {
    fn answers(&self) -> Result<String, ForgeError> {
        let out = self.ok(&["repo".into(), "view".into(), self.repo_arg(), "--json=name".into()])?;
        Ok(Self::json(&out).ok().and_then(|v| v.get("name").and_then(Value::as_str).map(str::to_string)).unwrap_or_else(|| "answered".into()))
    }

    fn credential(&self) -> Result<String, ForgeError> {
        self.ok(&["auth".into(), "status".into(), format!("--hostname={}", self.host)])?;
        Ok("accepted".into())
    }
}
