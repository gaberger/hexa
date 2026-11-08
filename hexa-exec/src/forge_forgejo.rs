//! Forgejo or Gitea through the REST API v1, with the token read from `target.token_env` at call time (ADR-2611031914 §1, §7). `base_url` is `https://<host>` in use and a local test server in tests; a missing or empty token variable is `Unauthorized` naming the variable.
//!
//! The adapter holds the variable's NAME, never its value: each request reads
//! it again, sends it as a sensitive header, and scrubs it from every message.
//! Redirects are not followed, so the token cannot follow one to another
//! origin. Every listing pages to the end or fails closed: absence is proven,
//! never assumed from a short page.

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

use hexa_core::ports::forge::{marker_is_last_line, ForgeError, ForgeTarget, IssueDraft, IssueRef, PrDraft, PrRef};
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::header::{HeaderValue, AUTHORIZATION};
use serde_json::Value;

use crate::ports::{Forge, ForgeStatus};

/// Items asked for per page. Gitea caps it at `MAX_RESPONSE_ITEMS`, so a page
/// shorter than this proves nothing.
const PAGE: usize = 50;
/// The most pages a listing reads before it gives up, and refuses.
const MAX_PAGES: u64 = 1_000;

/// One issue as listed, kept so a second lookup in the same run is free.
#[derive(Clone)]
struct Listed {
    number: u64,
    url: String,
    open: bool,
    body: String,
}

/// Forgejo or Gitea through the REST API v1, with the token read from `target.token_env` at call time (ADR-2611031914 §1, §7). `base_url` is `https://<host>` in use and a local test server in tests; a missing or empty token variable is `Unauthorized` naming the variable.
pub struct ForgeForgejo {
    api: String,
    owner: String,
    repo: String,
    host: String,
    token_env: String,
    client: Client,
    listing: Mutex<Option<Vec<Listed>>>,
}

/// What a page of a listing tells the pager.
enum Step<T> {
    Done(T),
    More,
}

impl ForgeForgejo {
    pub fn new(target: &ForgeTarget, base_url: &str) -> Result<ForgeForgejo, ForgeError> {
        let token_env = target
            .token_env
            .clone()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| ForgeError::Unauthorized(format!("{} has no token variable; `hexa forge add {} --kind forgejo --token-env <VAR>`", target.host, target.host)))?;
        match std::env::var(&token_env) {
            Ok(v) if !v.is_empty() => {}
            _ => return Err(ForgeError::Unauthorized(format!("{token_env} is not set"))),
        }
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| ForgeError::Unreachable(format!("{}: could not build an HTTP client", target.host)))?;
        Ok(ForgeForgejo {
            api: format!("{}/api/v1", base_url.trim_end_matches('/')),
            owner: target.owner.clone(),
            repo: target.repo.clone(),
            host: target.host.clone(),
            token_env,
            client,
            listing: Mutex::new(None),
        })
    }

    fn token(&self) -> Result<String, ForgeError> {
        match std::env::var(&self.token_env) {
            Ok(v) if !v.is_empty() => Ok(v),
            _ => Err(ForgeError::Unauthorized(format!("{} is not set", self.token_env))),
        }
    }

    /// `msg` with the token's current value replaced: no message leaves here holding it.
    fn scrub(&self, msg: String) -> String {
        match std::env::var(&self.token_env) {
            Ok(v) if !v.is_empty() => msg.replace(&v, "<redacted>"),
            _ => msg,
        }
    }

    fn repo_path(&self) -> String {
        format!("{}/repos/{}/{}", self.api, urlencoding::encode(&self.owner), urlencoding::encode(&self.repo))
    }

    /// Send one request with the token; the status, `X-Total-Count`, and the body.
    fn send(&self, req: RequestBuilder) -> Result<(u16, Option<usize>, String), ForgeError> {
        let mut auth = HeaderValue::from_str(&format!("token {}", self.token()?))
            .map_err(|_| ForgeError::Unauthorized(format!("{} holds characters a header cannot carry", self.token_env)))?;
        auth.set_sensitive(true);
        let resp = req.header(AUTHORIZATION, auth).send().map_err(|e| {
            let kind = if e.is_timeout() {
                "timed out"
            } else if refused_certificate(&e) {
                // A forge behind a private CA answers `curl` (the system store
                // trusts the CA) but not a client that does not read it; saying
                // "could not connect" sent the owner to the network instead.
                "its TLS certificate is not trusted; add the issuer's CA to the system trust store"
            } else if e.is_connect() {
                "could not connect"
            } else {
                "the request failed"
            };
            ForgeError::Unreachable(format!("{}: {kind}", self.host))
        })?;
        let status = resp.status().as_u16();
        let total = resp.headers().get("x-total-count").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse().ok());
        let text = resp.text().map_err(|_| ForgeError::Unreachable(format!("{}: the response was cut off", self.host)))?;
        self.check(status, &text)?;
        Ok((status, total, text))
    }

    /// The error a status maps to, if it is one.
    fn check(&self, status: u16, text: &str) -> Result<(), ForgeError> {
        let server_msg = || {
            let m = serde_json::from_str::<Value>(text)
                .ok()
                .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::to_string))
                .unwrap_or_else(|| format!("HTTP {status}"));
            self.scrub(m.chars().take(500).collect())
        };
        match status {
            200..=299 => Ok(()),
            300..=399 => Err(ForgeError::Refused(format!("{} answered with a redirect (HTTP {status}); it was not followed", self.host))),
            401 | 403 => Err(ForgeError::Unauthorized(format!("{} was not accepted by {}", self.token_env, self.host))),
            404 => Err(ForgeError::NotFound(format!("{}/{} on {}, or the token cannot see it", self.owner, self.repo, self.host))),
            500..=599 => Err(ForgeError::Unreachable(format!("{}: HTTP {status}", self.host))),
            _ => Err(ForgeError::Refused(server_msg())),
        }
    }

    fn json(&self, text: &str) -> Result<Value, ForgeError> {
        serde_json::from_str(text).map_err(|_| ForgeError::Refused(format!("{} sent JSON hexa could not read", self.host)))
    }

    /// Page through `url` (which already has its query) until a page is
    /// empty or `X-Total-Count` is reached; never because a page was short.
    /// `on_page` may end the walk early. A repeated page, or the cap, refuses:
    /// the listing is unproven.
    fn pages<T>(&self, url: &str, mut on_page: impl FnMut(&[Value]) -> Step<T>) -> Result<Option<T>, ForgeError> {
        let mut seen: HashSet<u64> = HashSet::new();
        for page in 1..=MAX_PAGES {
            let req = self.client.get(format!("{url}&limit={PAGE}&page={page}"));
            let (_, total, text) = self.send(req)?;
            let items = match self.json(&text)? {
                Value::Array(a) => a,
                _ => return Err(ForgeError::Refused(format!("{} sent a listing that is not a list", self.host))),
            };
            if items.is_empty() {
                return Ok(None);
            }
            if let Step::Done(t) = on_page(&items) {
                return Ok(Some(t));
            }
            let before = seen.len();
            seen.extend(items.iter().filter_map(|i| i.get("number").and_then(Value::as_u64)));
            if total.is_some_and(|t| seen.len() >= t) {
                return Ok(None);
            }
            // A full page with nothing new: the server is not paging, so the
            // pages after it were never seen. Absence is unproven.
            if seen.len() == before {
                return Err(ForgeError::Refused(format!(
                    "{} repeated a page of the listing; it could not be proven complete, so nothing was opened",
                    self.host
                )));
            }
        }
        Err(ForgeError::Refused(format!("could not finish listing on {}; nothing was opened", self.host)))
    }

    fn listed(v: &Value) -> Option<Listed> {
        if v.get("pull_request").is_some_and(|p| !p.is_null()) {
            return None;
        }
        Some(Listed {
            number: v.get("number")?.as_u64()?,
            url: v.get("html_url")?.as_str()?.to_string(),
            open: v.get("state").and_then(Value::as_str).map(|s| s.eq_ignore_ascii_case("open")).unwrap_or(true),
            body: v.get("body").and_then(Value::as_str).unwrap_or("").to_string(),
        })
    }

    /// Every issue, open and closed, as listed: the whole listing.
    fn list_issues(&self, query: &str) -> Result<Vec<Listed>, ForgeError> {
        let mut all: Vec<Listed> = Vec::new();
        let url = format!("{}/issues?type=issues&state=all{query}", self.repo_path());
        self.pages::<()>(&url, |items| {
            all.extend(items.iter().filter_map(Self::listed));
            Step::More
        })?;
        Ok(all)
    }

    fn lowest_with_marker(list: &[Listed], key: &str) -> Option<IssueRef> {
        list.iter()
            .filter(|l| marker_is_last_line(&l.body, key))
            .min_by_key(|l| l.number)
            .map(|l| IssueRef { number: l.number, url: l.url.clone(), open: l.open })
    }

    fn get_json(&self, url: &str) -> Result<Value, ForgeError> {
        let (_, _, text) = self.send(self.client.get(url))?;
        self.json(&text)
    }
}

impl Forge for ForgeForgejo {
    fn find_issue(&self, key: &str) -> Result<Option<IssueRef>, ForgeError> {
        if let Some(cached) = self.listing.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            return Ok(Self::lowest_with_marker(cached, key));
        }
        // The search index is a hint: a hit is checked against the marker.
        let hinted = self.list_issues(&format!("&q={}", urlencoding::encode(key)))?;
        if let Some(hit) = Self::lowest_with_marker(&hinted, key) {
            return Ok(Some(hit));
        }
        // The index is eventually consistent; the full listing is the proof.
        let all = self.list_issues("")?;
        let found = Self::lowest_with_marker(&all, key);
        *self.listing.lock().unwrap_or_else(|e| e.into_inner()) = Some(all);
        Ok(found)
    }

    fn open_issue(&self, d: &IssueDraft) -> Result<IssueRef, ForgeError> {
        let url = format!("{}/issues", self.repo_path());
        let sent = self.send(self.client.post(url).json(&serde_json::json!({"title": d.title, "body": d.body})));
        // The outcome of a failed create is unknown: the cached listing no
        // longer proves anything, and the next lookup asks the forge again.
        let (_, _, text) = sent.inspect_err(|_| *self.listing.lock().unwrap_or_else(|e| e.into_inner()) = None)?;
        let v = self.json(&text)?;
        let l = Self::listed(&v)
            .ok_or_else(|| ForgeError::Refused(format!("{} created an issue but did not say where", self.host)))?;
        let l = Listed { body: if l.body.is_empty() { d.body.clone() } else { l.body }, ..l };
        let r = IssueRef { number: l.number, url: l.url.clone(), open: l.open };
        if let Some(cached) = self.listing.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            cached.push(l);
        }
        Ok(r)
    }

    fn find_pr(&self, head: &str) -> Result<Option<PrRef>, ForgeError> {
        let full = format!("{}/{}", self.owner, self.repo);
        let url = format!("{}/pulls?state=open", self.repo_path());
        self.pages(&url, |items| {
            let hit = items.iter().find(|p| {
                let h = p.get("head");
                let same_ref = h.and_then(|h| h.get("ref")).and_then(Value::as_str) == Some(head);
                let repo = h.and_then(|h| h.get("repo")).and_then(|r| r.get("full_name")).and_then(Value::as_str);
                same_ref && repo.is_none_or(|r| r.eq_ignore_ascii_case(&full))
            });
            match hit.and_then(|p| Some(PrRef { number: p.get("number")?.as_u64()?, url: p.get("html_url")?.as_str()?.to_string() })) {
                Some(r) => Step::Done(r),
                None => Step::More,
            }
        })
    }

    fn open_pr(&self, d: &PrDraft) -> Result<PrRef, ForgeError> {
        // The v1 API has no draft field; Gitea reads a `WIP:` title as one.
        let title = if d.draft && !d.title.starts_with("WIP:") { format!("WIP: {}", d.title) } else { d.title.clone() };
        let url = format!("{}/pulls", self.repo_path());
        let payload = serde_json::json!({"head": d.head, "base": d.base, "title": title, "body": d.body});
        let (_, _, text) = self.send(self.client.post(url).json(&payload))?;
        let v = self.json(&text)?;
        let number = v.get("number").and_then(Value::as_u64);
        let html = v.get("html_url").and_then(Value::as_str);
        match (number, html) {
            (Some(number), Some(url)) => Ok(PrRef { number, url: url.to_string() }),
            _ => Err(ForgeError::Refused(format!("{} opened a pull request but did not say where", self.host))),
        }
    }
}

/// Whether a failed request failed on the server's TLS certificate: some error
/// in its source chain names a certificate. Only the fixed words are tested;
/// no part of the chain is returned, so nothing in it can reach a message.
fn refused_certificate(e: &reqwest::Error) -> bool {
    let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(e);
    while let Some(err) = cur {
        let s = err.to_string().to_ascii_lowercase();
        if s.contains("certificate") || s.contains("unknownissuer") {
            return true;
        }
        cur = err.source();
    }
    false
}

impl ForgeStatus for ForgeForgejo {
    fn answers(&self) -> Result<String, ForgeError> {
        let v = self.get_json(&format!("{}/version", self.api))?;
        Ok(v.get("version").and_then(Value::as_str).unwrap_or("answered").to_string())
    }

    fn credential(&self) -> Result<String, ForgeError> {
        let v = self.get_json(&format!("{}/user", self.api))?;
        Ok(v.get("login").and_then(Value::as_str).unwrap_or("accepted").to_string())
    }
}
