//! The repository's forge: where an issue is filed and a pull request is
//! opened (ADR-2611031914). Pure: the mapping from a remote to a forge, the
//! texts hexa sends, and the scan that refuses to send a secret. The forges
//! themselves are reached through the `Forge` port in `hexa-exec`.

use serde::{Deserialize, Serialize};

/// Which forge API a host speaks (§1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ForgeKind {
    /// github.com, through the `gh` CLI and its authentication.
    Github,
    /// Forgejo or Gitea, through REST API v1 and a token variable.
    Forgejo,
}

/// A host listed in `$HEXA_HOME/forges.json` (§2): its kind and the name of
/// the variable that holds its token, never the token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgeHost {
    pub host: String,
    pub kind: ForgeKind,
    pub token_env: Option<String>,
}

/// The forge and repository a remote maps to (§2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgeTarget {
    pub kind: ForgeKind,
    pub host: String,
    pub owner: String,
    pub repo: String,
    pub token_env: Option<String>,
}

/// An issue to file, keyed by the memory key or gap it came from (§4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueDraft {
    pub key: String,
    pub title: String,
    pub body: String,
}

/// An issue on the forge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueRef {
    pub number: u64,
    pub url: String,
    pub open: bool,
}

/// A pull request to open from a pushed branch (§5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrDraft {
    pub head: String,
    pub base: String,
    pub title: String,
    pub body: String,
    pub draft: bool,
}

/// A pull request on the forge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrRef {
    pub number: u64,
    pub url: String,
}

/// Why a forge call did not do what was asked. A message never holds a
/// token's value (§6).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ForgeError {
    #[error("the forge did not answer: {0}")]
    Unreachable(String),
    #[error("the forge refused the credential: {0}")]
    Unauthorized(String),
    #[error("not found on the forge: {0}")]
    NotFound(String),
    #[error("refused: {0}")]
    Refused(String),
}

/// A remote URL taken apart: whether it is plain http, the host as the forge
/// is addressed (lowercase; the port kept for http(s) only), and the path.
struct Remote {
    plain_http: bool,
    host: String,
    path: String,
}

fn parse_remote(url: &str) -> Option<Remote> {
    let url = url.trim();
    if let Some((scheme, rest)) = url.split_once("://") {
        let scheme = scheme.to_ascii_lowercase();
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        // userinfo never survives: it may hold a password.
        let hostport = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
        let keep_port = scheme == "http" || scheme == "https";
        let host = if keep_port { hostport.to_string() } else { strip_port(hostport) };
        if host.is_empty() {
            return None;
        }
        return Some(Remote { plain_http: scheme == "http", host: host.to_ascii_lowercase(), path: path.to_string() });
    }
    // scp-like `user@host:path`: a `:` before any `/`.
    let colon = url.find(':')?;
    if url[..colon].contains('/') {
        return None;
    }
    let hostpart = &url[..colon];
    let host = hostpart.rsplit_once('@').map(|(_, h)| h).unwrap_or(hostpart);
    if host.is_empty() {
        return None;
    }
    Some(Remote { plain_http: false, host: host.to_ascii_lowercase(), path: url[colon + 1..].to_string() })
}

fn strip_port(hostport: &str) -> String {
    if let Some(rest) = hostport.strip_prefix('[') {
        return rest.split(']').next().unwrap_or("").to_string();
    }
    hostport.split(':').next().unwrap_or("").to_string()
}

/// The host without its port, for the `github.com` and loopback checks.
fn bare_host(host: &str) -> String {
    if host.starts_with('[') {
        return strip_port(host);
    }
    match host.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') && p.chars().all(|c| c.is_ascii_digit()) => h.to_string(),
        _ => host.to_string(),
    }
}

/// The host a remote names (lowercase, http(s) port kept), for saying which
/// host is unknown. Never the URL: it may carry a password.
pub fn remote_host(remote_url: &str) -> Option<String> {
    parse_remote(remote_url).map(|r| r.host)
}

/// The password in a remote's userinfo, if any: a credential the secret scan
/// must look for, and that must never be printed.
pub fn remote_password(remote_url: &str) -> Option<String> {
    let (_, rest) = remote_url.trim().split_once("://")?;
    let authority = rest.split('/').next()?;
    let (userinfo, _) = authority.rsplit_once('@')?;
    let (_, pw) = userinfo.split_once(':')?;
    let pw = urldecode(pw);
    (!pw.is_empty()).then_some(pw)
}

fn urldecode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The forge an `origin` URL (https or ssh form) maps to (§2): `github.com`
/// is GitHub, a host in `hosts` is its recorded kind, anything else `None`.
pub fn forge_for_remote(remote_url: &str, hosts: &[ForgeHost]) -> Option<ForgeTarget> {
    let r = parse_remote(remote_url)?;
    let path = r.path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path).trim_end_matches('/');
    let mut segs = path.split('/');
    let (owner, repo) = match (segs.next(), segs.next(), segs.next()) {
        (Some(o), Some(p), None) if !o.is_empty() && !p.is_empty() => (o.to_string(), p.to_string()),
        _ => return None,
    };
    let bare = bare_host(&r.host);
    if bare == "github.com" {
        return Some(ForgeTarget { kind: ForgeKind::Github, host: "github.com".into(), owner, repo, token_env: None });
    }
    let listed = hosts
        .iter()
        .find(|h| h.host.eq_ignore_ascii_case(&r.host))
        .or_else(|| hosts.iter().find(|h| h.host.eq_ignore_ascii_case(&bare)))?;
    Some(ForgeTarget { kind: listed.kind, host: r.host, owner, repo, token_env: listed.token_env.clone() })
}

/// Where the forge's API is reached for a remote: `https://<host>`, or plain
/// `http://<host>` only for a loopback host reached by an http remote. A token
/// is never sent over plain http to anything else.
pub fn forge_base_url(remote_url: &str, target: &ForgeTarget) -> Result<String, String> {
    let plain = parse_remote(remote_url).map(|r| r.plain_http).unwrap_or(false);
    if !plain {
        return Ok(format!("https://{}", target.host));
    }
    let bare = bare_host(&target.host);
    if matches!(bare.as_str(), "127.0.0.1" | "::1" | "localhost") {
        Ok(format!("http://{}", target.host))
    } else {
        Err(format!("refusing to send a token over plain http to {}", target.host))
    }
}

/// Whether `key` may be filed: `[A-Za-z0-9:_./-]+`, a `:`, and no `--` (which
/// would end the HTML comment the marker is).
pub fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.contains(':')
        && !key.contains("--")
        && key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '.' | '/' | '-'))
}

/// Whether `name` is an environment variable name: `[A-Za-z_][A-Za-z0-9_]*`.
pub fn valid_env_name(name: &str) -> bool {
    let mut cs = name.chars();
    matches!(cs.next(), Some(c) if c.is_ascii_alphabetic() || c == '_') && cs.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The marker that makes a recorded key's issue findable (§4).
pub fn key_marker(key: &str) -> String {
    format!("<!-- hexa-key: {key} -->")
}

/// Whether `body` carries `key`: its last non-empty line, trimmed, is the
/// marker exactly. A marker quoted elsewhere in a body never counts.
pub fn marker_is_last_line(body: &str, key: &str) -> bool {
    body.lines().rev().find(|l| !l.trim().is_empty()).map(|l| l.trim() == key_marker(key)).unwrap_or(false)
}

/// The first sentence of `value`: up to `.`, `!` or `?` followed by
/// whitespace or the end, or up to the first newline.
fn first_sentence(value: &str) -> String {
    let chars: Vec<char> = value.trim_start().chars().collect();
    let mut end = chars.len();
    for (i, c) in chars.iter().enumerate() {
        if *c == '\n' {
            end = i;
            break;
        }
        if matches!(c, '.' | '!' | '?') && chars.get(i + 1).is_none_or(|n| n.is_whitespace()) {
            end = i;
            break;
        }
    }
    chars[..end].iter().collect::<String>().trim().to_string()
}

const TITLE_MAX: usize = 80;

/// The issue for a memory entry (§7): title the value's first sentence, at
/// most 80 characters; body the value, the project and the marker.
pub fn issue_from_memory(key: &str, value: &str, project: &str) -> IssueDraft {
    let mut title = first_sentence(value);
    if title.chars().count() > TITLE_MAX {
        let head: Vec<char> = title.chars().take(TITLE_MAX - 1).collect();
        let cut = head.iter().rposition(|c| c.is_whitespace()).filter(|&i| i > 0).unwrap_or(head.len());
        title = head[..cut].iter().collect::<String>().trim_end().to_string();
        title.push('…');
    }
    if title.trim().is_empty() {
        title = key.to_string();
    }
    let body = format!("{value}\n\nRecorded in project `{project}` under `{key}`.\n\n{}", key_marker(key));
    IssueDraft { key: key.to_string(), title, body }
}

/// A pull request body from the commit subjects and the loop's evidence (§5).
pub fn pr_body(commits: &[String], adr: Option<&str>, gate: Option<&str>, last_result: Option<&str>) -> String {
    let mut out = String::from("## Commits\n\n");
    for c in commits {
        out.push_str(&format!("- {c}\n"));
    }
    if adr.is_some() || gate.is_some() || last_result.is_some() {
        out.push_str("\n## Evidence\n\n");
        if let Some(a) = adr {
            out.push_str(&format!("- ADR: {a}\n"));
        }
        if let Some(g) = gate {
            out.push_str(&format!("- Gate: `{g}`\n"));
        }
        if let Some(r) = last_result {
            out.push_str(&format!("- Last result: {r}\n"));
        }
    }
    out
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Prefixes of credential shapes and the fewest characters that must follow.
const PREFIXES: [(&str, usize); 5] = [("sk-", 12), ("ghp_", 20), ("gho_", 20), ("github_pat_", 20), ("glpat-", 16)];

fn line_is_secret(line: &str, token: Option<&str>) -> bool {
    if let Some(t) = token.filter(|t| !t.is_empty()) {
        if line.contains(t) {
            return true;
        }
    }
    if line.contains("-----BEGIN ") && line.contains("PRIVATE KEY-----") {
        return true;
    }
    let at_boundary = |i: usize| line[..i].chars().next_back().is_none_or(|c| !is_word(c));
    for (prefix, min) in PREFIXES {
        for (i, _) in line.match_indices(prefix) {
            if at_boundary(i) && line[i + prefix.len()..].chars().take_while(|c| is_word(*c)).count() >= min {
                return true;
            }
        }
    }
    for (i, _) in line.match_indices("AKIA") {
        let rest = &line[i + 4..];
        let body: Vec<char> = rest.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        if at_boundary(i) && body.len() == 16 && body.iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) {
            return true;
        }
    }
    false
}

/// The 1-based line of the first credential shape in `text`, or `None` (§6).
/// Only the line number is returned, so the match itself cannot leak.
pub fn looks_secret(text: &str, token: Option<&str>) -> Option<usize> {
    text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).position(|l| line_is_secret(l, token)).map(|i| i + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_words_with_prefix_inside_are_not_secrets() {
        for t in ["task-list for the week", "risk-free change", "desk-lamp-and-chair-and-more-things", "AKIAS are birds"] {
            assert_eq!(looks_secret(t, None), None, "{t}");
        }
    }

    #[test]
    fn crlf_numbers_lines_like_lf() {
        let lf = "a\nb\nkey sk-abcdefghijklmnop";
        let crlf = "a\r\nb\r\nkey sk-abcdefghijklmnop";
        assert_eq!(looks_secret(lf, None), Some(3));
        assert_eq!(looks_secret(crlf, None), Some(3));
    }

    #[test]
    fn marker_counts_only_as_last_line() {
        let quoted = format!("See {} here.\n\nmore", key_marker("bug:y"));
        assert!(!marker_is_last_line(&quoted, "bug:y"));
        let other = format!("x\n\n{}\n", key_marker("bug:x-y"));
        assert!(!marker_is_last_line(&other, "bug:x"));
        assert!(marker_is_last_line(&other, "bug:x-y"));
    }

    #[test]
    fn keys_and_env_names_are_validated() {
        assert!(valid_key("bug:red-check"));
        assert!(!valid_key("bug:a--b"));
        assert!(!valid_key("nocolon"));
        assert!(!valid_key("bug:a b"));
        assert!(valid_env_name("STRATA_TOKEN"));
        assert!(!valid_env_name("1X"));
        assert!(!valid_env_name("A-B"));
    }

    #[test]
    fn remote_port_userinfo_and_plain_http() {
        let hosts = [ForgeHost { host: "127.0.0.1:3000".into(), kind: ForgeKind::Forgejo, token_env: Some("T".into()) }];
        let t = forge_for_remote("http://u:pw@127.0.0.1:3000/me/strata.git", &hosts).unwrap();
        assert_eq!(t.host, "127.0.0.1:3000");
        assert_eq!(forge_base_url("http://127.0.0.1:3000/me/strata.git", &t).unwrap(), "http://127.0.0.1:3000");
        let lan = [ForgeHost { host: "forge.lan".into(), kind: ForgeKind::Forgejo, token_env: Some("T".into()) }];
        let t = forge_for_remote("http://forge.lan/o/r", &lan).unwrap();
        assert!(forge_base_url("http://forge.lan/o/r", &t).is_err());
        assert_eq!(forge_for_remote("ssh://git@git.local:2222/o/r.git", &lan), None);
        assert!(forge_for_remote("https://github.com/a/b/c", &[]).is_none());
        assert_eq!(remote_password("https://u:SECRET%21PW@h/o/r").as_deref(), Some("SECRET!PW"));
        assert_eq!(remote_password("https://u@h/o/r"), None);
        assert_eq!(remote_host("git@Git.Local:o/r.git").as_deref(), Some("git.local"));
    }

    #[test]
    fn title_cuts_at_a_word_and_stays_under_80() {
        let v = "word ".repeat(40);
        let d = issue_from_memory("bug:x", &v, "p");
        assert!(d.title.chars().count() <= 80);
        assert!(d.title.ends_with('…'));
        assert_eq!(issue_from_memory("bug:x", "", "p").title, "bug:x");
    }
}
