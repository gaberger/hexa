//! Which hexa verb a plain-English request names (ADR-2610271308). Pure: the
//! caller confirms and runs what is routed.

/// One routed command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    /// For `Hexa`, the arguments after `hexa`; for `Shell`, the whole command.
    pub command: String,
    /// It changes or deletes something, so it needs confirmation even with `--yes`.
    pub destructive: bool,
    /// One line saying what it will do.
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Routed {
    /// A hexa verb: run as `hexa <command>`.
    Hexa(Route),
    /// A command that is not hexa's, such as `git status`.
    Shell(Route),
    /// Nothing in the table matches: say so, never guess.
    None,
}

/// Every hexa command `route` can produce. An argument is written as
/// `<placeholder>`; the check runs the words before it with `--help`.
const HEXA_COMMANDS: &[&str] = &[
    "analyze .",
    "loop",
    "go",
    "adr gates",
    "doctor",
    "self-update --check",
    "harden <path>",
    "memory search <words>",
    "dev worktree list",
    "dev worktree cleanup",
    "status",
    "docs check",
    "plan list",
    "plan reconcile --all --update",
    "config inference list",
    "config inference test --all",
];

/// Every hexa command the table can route to, so each can be checked against
/// the CLI.
pub fn hexa_commands() -> Vec<String> {
    HEXA_COMMANDS.iter().map(|c| c.to_string()).collect()
}

/// A whitespace-separated word of the request: where it sits in the original
/// text, and its lowercase form with surrounding punctuation dropped.
struct Word<'a> {
    start: usize,
    raw: &'a str,
    key: String,
}

fn words(text: &str) -> Vec<Word<'_>> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(i),
            (true, Some(s)) => {
                let raw = &text[s..i];
                let key = raw
                    .trim_matches(|c: char| !c.is_alphanumeric() && c != '-')
                    .to_lowercase();
                out.push(Word { start: s, raw, key });
                start = None;
            }
            _ => {}
        }
    }
    out
}

fn hexa(command: impl Into<String>, destructive: bool, description: impl Into<String>) -> Routed {
    Routed::Hexa(Route {
        command: command.into(),
        destructive,
        description: description.into(),
    })
}

fn shell(command: &str, description: &str) -> Routed {
    Routed::Shell(Route {
        command: command.into(),
        destructive: false,
        description: description.into(),
    })
}

/// A word that names a path: it has a separator or an extension, so "it" or
/// "everything" is never taken for one.
fn path_like(raw: &str) -> Option<&str> {
    let p = raw.trim_end_matches([',', ';', ':', '!', '?']);
    (p.contains('/') || p.contains('.')).then_some(p)
}

/// The route for `text`, or `Routed::None`.
pub fn route(text: &str) -> Routed {
    let ws = words(text);
    let has = |k: &str| ws.iter().any(|w| w.key == k);
    let starts = |k: &str| ws.iter().any(|w| w.key.starts_with(k));
    let pos = |k: &str| ws.iter().position(|w| w.key == k);

    // Hardening a path. The path comes from the request or there is no route.
    let hunt = ws
        .iter()
        .position(|w| w.key == "harden" || w.key == "bug-hunt")
        .or_else(|| {
            ws.windows(2)
                .position(|p| {
                    (p[0].key == "bug" && p[1].key == "hunt")
                        || (p[0].key == "hunt" && p[1].key.starts_with("bug"))
                })
                .map(|i| i + 1)
        });
    if let Some(i) = hunt {
        return match ws[i + 1..].iter().find_map(|w| path_like(w.raw)) {
            Some(p) => hexa(
                format!("harden {p}"),
                true,
                format!("Hunt for bugs in {p}, verify each finding, fix each under the gate"),
            ),
            None => Routed::None,
        };
    }

    // Searching memory: the query is the rest of the request.
    let after = match (pos("search"), pos("memory")) {
        (Some(s), Some(m)) if m == s + 1 || s == m + 1 => Some(s.max(m) + 1),
        _ => pos("recall").map(|r| r + 1),
    };
    if let Some(mut i) = after {
        while ws
            .get(i)
            .is_some_and(|w| matches!(w.key.as_str(), "for" | "about" | "on"))
        {
            i += 1;
        }
        return match ws.get(i) {
            Some(w) => {
                let q = text[w.start..].trim();
                hexa(
                    format!("memory search {q}"),
                    false,
                    format!("Search what was learned for \"{q}\""),
                )
            }
            None => Routed::None,
        };
    }

    if starts("worktree") {
        return if has("clean") || has("cleanup") || has("remove") || has("delete") || has("prune") {
            hexa(
                "dev worktree cleanup",
                true,
                "Remove worktrees whose branches are merged into main",
            )
        } else {
            hexa("dev worktree list", false, "List every active worktree")
        };
    }

    // Git state is git's, not hexa's.
    if starts("commit") || (has("git") && (has("log") || has("history"))) {
        return shell("git log --oneline -20", "Show the last 20 commits");
    }
    if has("diff") || (has("git") && (has("changed") || has("changes"))) {
        return shell("git diff --stat", "Show what changed and is not committed");
    }
    if has("git") {
        return shell("git status", "Show git status");
    }

    if has("self-update")
        || ((has("update") || has("upgrade")) && has("hexa"))
        || (has("up") && has("to") && has("date") && has("hexa"))
    {
        return hexa(
            "self-update --check",
            false,
            "Check whether a newer hexa release exists",
        );
    }
    if has("doctor") || has("install") || has("installation") || has("setup") {
        return hexa(
            "doctor",
            false,
            "Check the install, the config and this project",
        );
    }
    if (starts("gate")
        && (starts("decision") || has("adr") || has("adrs") || has("every") || has("all")))
        || (has("adr") && starts("gate"))
    {
        return hexa("adr gates", false, "Run every ADR's gate");
    }
    if starts("architect") || has("analyze") || has("analyse") || starts("boundar") || has("grade")
    {
        return hexa("analyze .", false, "Grade the architecture of this project");
    }
    if has("loop") || (has("work") && has("stand")) || has("progress") {
        return hexa(
            "loop",
            false,
            "Show where the work stands, in this session and every other",
        );
    }
    if has("next") || (ws.len() == 1 && has("go")) {
        return hexa("go", false, "Suggest the next action");
    }

    if starts("calibrat") || (starts("test") && has("inference")) {
        return hexa(
            "config inference test --all",
            false,
            "Test every registered inference provider",
        );
    }
    if (has("list") || has("show")) && (starts("provider") || has("inference")) {
        return hexa("config inference list", false, "List inference providers");
    }
    if has("reconcile") {
        return hexa(
            "plan reconcile --all --update",
            true,
            "Reconcile every workplan's steps with the code, and write the result",
        );
    }
    if (has("list") || has("show")) && (starts("plan") || starts("workplan")) {
        return hexa("plan list", false, "List the workplans");
    }
    if has("docs") || has("documentation") {
        return hexa(
            "docs check",
            false,
            "Check the docs for freshness and terminology",
        );
    }
    if (ws.len() == 1 && has("status")) || (has("project") && has("status")) {
        return hexa("status", false, "Show project, config and git state");
    }

    if has("rebuild") || (has("build") && has("release")) {
        return shell(
            "cargo build -p hexa-cli --release",
            "Build hexa in release mode",
        );
    }
    if starts("vulnerab") || has("security") || has("dependabot") {
        return shell("cargo audit", "Scan dependencies for known vulnerabilities");
    }
    if has("tests") || (has("run") && has("test")) {
        return shell("cargo test --workspace", "Run every test in the workspace");
    }
    if has("list") && (starts("file") || has("directory") || has("dir")) {
        return shell("ls -la", "List files in this directory");
    }

    Routed::None
}
