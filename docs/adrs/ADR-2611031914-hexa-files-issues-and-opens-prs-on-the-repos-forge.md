# ADR-2611031914: hexa files issues and opens pull requests on the repository's forge

**Status:** Accepted
**Date:** 2026-11-03
**Drivers:** On 2026-11-02, working in strata, eight hexa bugs were found and recorded with `hexa memory store bug:…`: the red check accepting stub tests, `hexa do` committing untracked files, foreign ADR ids in installed assets, and five more. Each sits in one project's `.hexa/memory.jsonl`, seen by the next session there and by nobody else. Both pull requests of the day (#30, #31) were opened by hand with `gh pr create` and hand-written bodies, and `hexa gap pr <id>` prints a pull request it cannot open. strata lives on `git.local`, a Forgejo 16 server, where nothing in hexa can open anything at all. The owner: integrate hexa with the repository's forge to file issues and create pull requests, on GitHub and Forgejo, through an explicit, deduplicated verb.

## Context

hexa already writes the text. `hexa gap pr <id>` builds a pull request from a gap (`pr_text` in `hexa-core/src/domain/guide.rs`). `hexa memory store` keeps a recorded bug under a stable key in `hexa-exec/src/ports.rs`'s `MemoryStore`. `hexa insight punch-list` finds follow-ups "routed nowhere". What is missing is the last step, and its absence has a cost: a bug recorded in strata's memory about hexa never reaches hexa's tracker.

Two forges are in use. hexa is on GitHub, where `gh` is installed and authenticated. strata is on `git.local`, Forgejo `16.0.6+gitea-1.22.0`, whose REST API v1 is Gitea's: `GET /api/v1/repos/{owner}/{repo}/issues`, `POST …/issues`, `GET …/pulls`, `POST …/pulls`, with `Authorization: token <t>`.

Three properties matter more than coverage:

* **Nothing leaves the machine by accident.** An issue or a pull request is published, indexed and mailed to watchers the moment it is created. hexa's existing rule for outward text is that it prints and opens nothing (`gap pr`); this keeps that as the default.
* **Filing twice files once.** Retrying a sync must not open duplicates. The key the bug was recorded under is the identity.
* **A secret is never sent or stored.** Memory values are free text written during work, and can hold a token pasted into a note. hexa's existing rule for credentials is a named environment variable, resolved when used and never written (`Endpoint::resolve_secret` in `hexa-infer/src/endpoint.rs`).

## Decision

1. **One port, two adapters.** `hexa-exec/src/ports.rs` gains `pub trait Forge: Send + Sync` (§7). `ForgeGithub` drives the `gh` CLI with the user's existing authentication; `ForgeForgejo` calls the Gitea-compatible REST API v1 with a token read from an environment variable at call time. Nothing above the port knows which one it has.
2. **The forge follows the remote.** `forge_for_remote(remote_url, hosts)` in `hexa-core/src/domain/forge.rs` maps a repository's `origin` URL (https or ssh form) to a target: `github.com` is GitHub; a host listed in `~/.hexa/forges.json` is the kind recorded there; any other host is `None`, and the verbs say which host is unknown and how to add it. `hexa forge add <host> --kind forgejo --token-env <NAME>` writes the host, kind and variable name, never a token. `hexa forge status` prints the target for this repository, whether it answers, and whether the credential is accepted, and writes nothing.
3. **Every verb is a dry run unless told to send.** `hexa issue file`, `hexa issue sync` and `hexa pr open` print the target, the title and the body, and send nothing, unless given `--send`. `hexa gap pr <id>` keeps printing; `hexa gap pr <id> --send` opens it with `hexa pr open`'s rules.
4. **A recorded key files one issue.** `hexa issue file <key>` takes a memory key (`bug:red-check-accepts-stubs`) or a gap (`gap:no-completion-model`). The body ends with the marker `<!-- hexa-key: <key> -->`. Before sending, the forge is searched for that marker over open and closed issues; when one exists, nothing is opened and its URL is printed. After sending, or on finding one, the issue's URL is stored in memory under `issue:<key>`. `hexa issue sync [--prefix <p>]` does this for every `bug:` and `gap:` key (or those under `--prefix`) that has no `issue:` entry.
5. **A pull request is opened for a pushed branch only.** `hexa pr open [--base <branch>] [--draft]` opens one for the current branch against `--base` (default: the remote's default branch). It refuses when the branch is not on the remote at the same commit, and says to push it; hexa never pushes. When the forge already has an open pull request from that branch, nothing is opened and its URL is printed. The body is built by `pr_body` from the commit subjects in `base..HEAD` and, when the loop has them, its ADR, its gate and the gate's last result; `--body <file>` replaces the generated body.
6. **A body that looks like it holds a secret is refused.** Before any send, `looks_secret(text)` scans the title and body for credential shapes (`sk-`, `ghp_`, `gho_`, `github_pat_`, `glpat-`, `AKIA` plus 16 upper-case characters, `-----BEGIN … PRIVATE KEY-----`, and the current value of the forge's token variable). A match refuses the send and names the line, without printing the match. A token value is never printed, logged or written.
7. **Public API.** Tests use only these items. Pure functions and value types in `hexa-core/src/domain/forge.rs`, re-exported by `hexa-core/src/ports/forge.rs`:
   * `pub enum ForgeKind { Github, Forgejo }`; `pub struct ForgeHost { pub host: String, pub kind: ForgeKind, pub token_env: Option<String> }`; `pub struct ForgeTarget { pub kind: ForgeKind, pub host: String, pub owner: String, pub repo: String, pub token_env: Option<String> }`.
   * `pub fn forge_for_remote(remote_url: &str, hosts: &[ForgeHost]) -> Option<ForgeTarget>`.
   * `pub fn key_marker(key: &str) -> String`, which is `<!-- hexa-key: {key} -->`.
   * `pub fn issue_from_memory(key: &str, value: &str, project: &str) -> IssueDraft`: the title is the value's first sentence, at most 80 characters; the body is the value, the project it was recorded in, and the marker.
   * `pub fn pr_body(commits: &[String], adr: Option<&str>, gate: Option<&str>, last_result: Option<&str>) -> String`.
   * `pub fn looks_secret(text: &str, token: Option<&str>) -> Option<usize>`: the 1-based line of the first match.
   * `pub struct IssueDraft { pub key: String, pub title: String, pub body: String }`, `pub struct IssueRef { pub number: u64, pub url: String, pub open: bool }`, `pub struct PrDraft { pub head: String, pub base: String, pub title: String, pub body: String, pub draft: bool }`, `pub struct PrRef { pub number: u64, pub url: String }`, `pub enum ForgeError { Unreachable(String), Unauthorized(String), NotFound(String), Refused(String) }`.
   * In `hexa-exec/src/ports.rs`: `pub trait Forge: Send + Sync { fn find_issue(&self, key: &str) -> Result<Option<IssueRef>, ForgeError>; fn open_issue(&self, d: &IssueDraft) -> Result<IssueRef, ForgeError>; fn find_pr(&self, head: &str) -> Result<Option<PrRef>, ForgeError>; fn open_pr(&self, d: &PrDraft) -> Result<PrRef, ForgeError>; }`.
   * `ForgeForgejo::new(target: &ForgeTarget, base_url: &str) -> Result<ForgeForgejo, ForgeError>` (`base_url` is `https://<host>` in use and a local test server in tests; a missing or empty token variable is `Unauthorized` naming the variable) and `ForgeGithub::new(target: &ForgeTarget, gh: &Path) -> ForgeGithub` (`gh` is the CLI's path, a recording stub in tests), in `hexa-exec/src/forge_forgejo.rs` and `hexa-exec/src/forge_github.rs`.
   * The CLI verbs `hexa forge add|status`, `hexa issue file|sync`, `hexa pr open` and `hexa gap pr --send` in `hexa-cli/src/commands/forge.rs`, `issue.rs` and `pr.rs`; the hosts file at `$HEXA_HOME/forges.json`, where `HEXA_HOME` defaults to `~/.hexa`, so a test points it at a temporary directory.

## Consequences

* A bug recorded in any project can reach the tracker of the project it is about, once, with a link back in memory.
* The pull requests hexa's own work produces get a body from the evidence hexa already holds, instead of one written from memory.
* hexa gains outward-facing verbs. The dry-run default, the refusal to push, the deduplication and the secret scan are what keep them from publishing by accident; each has a test.
* Forgejo needs a token with issue and pull-request scope in a named variable; GitHub needs only `gh auth`. `hexa forge status` is how a person checks either before sending.
* GitLab and Bitbucket are not covered; a third adapter is a new `ForgeKind` and an adapter, with no change above the port.

## Alternatives considered

* **File automatically on `hexa memory store bug:` and `hexa gap add`.** Faster, but every note becomes public the moment it is written, including half-formed ones. Rejected by the owner for an explicit verb.
* **Print a ready-to-run `gh`/`tea` command and keep credentials out of hexa.** No secret handling at all, but no deduplication, no link back into memory, and nothing for Forgejo without `tea` installed. Rejected.
* **One adapter over the REST APIs of both.** One code path, but it would need its own GitHub token where `gh` is already authenticated. Rejected for GitHub; kept for Forgejo, which has no comparable CLI installed here.

## Gate

`cargo test -p hexa-core forge && cargo test -p hexa-cli --test forge_files_issues_and_opens_prs`: the domain functions (the remote mapping in https and ssh forms, the marker, the issue and PR bodies, the secret scan); and end to end, against a local HTTP double of the Forgejo API and a recording stub `gh` on PATH, with `HEXA_HOME` in a temporary directory: a dry run sends nothing; `--send` opens one issue whose body carries the marker and records `issue:<key>`; a second `--send` finds it and opens none; `sync` files only unfiled keys; `pr open` refuses an unpushed branch, finds an existing pull request, and opens one otherwise; a body holding a token-shaped string is refused before any request; a rejected token is `Unauthorized` naming the variable, never its value.

## References

* ADR-2610301500: a dead end gives a guide and records the gap (`hexa gap`, `pr_text`)
* ADR-2610221540: the spec pipeline whose sealed tests this decision's tests will join
* ADR-2610271110: the unbuilt list may only shrink, so this decision merges with its implementation
