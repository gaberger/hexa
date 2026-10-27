//! Which source a hexa binary was built from, and whether it is behind
//! (ADR-2610270853). Pure: the caller reads git and the release feed.

/// What the binary knows about its own build, embedded at compile time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Build {
    pub version: String,
    /// The full commit the binary was built from; `None` when it was not
    /// built from a git tree.
    pub commit: Option<String>,
    /// The tree had uncommitted changes when it was built.
    pub dirty: bool,
}

/// The git tree hexa is running in, as far as it relates to the build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkout {
    pub head: String,
    /// The build commit exists in this repository.
    pub knows_build: bool,
    /// The build commit is an ancestor of `head`.
    pub build_is_ancestor: bool,
    /// Commits from the build to `head` that change a file the binary is
    /// built from (crate sources, manifests, `Cargo.lock`, embedded assets).
    pub source_commits_behind: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Staleness {
    Current,
    /// Built from an earlier commit of this checkout's source.
    BehindCheckout { commits: u64 },
    /// Built from a commit this checkout has but is not on its branch.
    OffBranch,
    /// A published release is newer than the build's version.
    BehindRelease { latest: String },
    /// Nothing to compare against: say so, never claim `Current`.
    Unknown { why: String },
}

/// Judge the build against the checkout, then against the newest release.
///
/// Every finding is returned (§7). `Current` only when a comparison was
/// made and found nothing; `Unknown` when none could be made (§5).
pub fn staleness(build: &Build, checkout: Option<&Checkout>, latest_release: Option<&str>) -> Vec<Staleness> {
    let mut out = Vec::new();
    let mut compared = false;
    if let (Some(_), Some(c)) = (&build.commit, checkout) {
        // A repository without the build commit is not this binary's source (§3).
        if c.knows_build {
            compared = true;
            if !c.build_is_ancestor {
                out.push(Staleness::OffBranch);
            } else if c.source_commits_behind > 0 {
                out.push(Staleness::BehindCheckout { commits: c.source_commits_behind });
            }
        }
    }
    if let Some(latest) = latest_release {
        if let (Some(have), Some(want)) = (version(&build.version), version(latest)) {
            compared = true;
            if want > have {
                out.push(Staleness::BehindRelease { latest: latest.trim_start_matches('v').to_string() });
            }
        }
    }
    if !compared {
        out.push(Staleness::Unknown { why: why_unknown(build, checkout, latest_release) });
    }
    if out.is_empty() {
        out.push(Staleness::Current);
    }
    out
}

/// The one line a session or `hexa doctor` prints, or `None` when current.
/// `Unknown` gives none: only `hexa doctor` prints it, as itself (§6).
pub fn advice(s: &Staleness) -> Option<String> {
    match s {
        Staleness::Current | Staleness::Unknown { .. } => None,
        Staleness::BehindCheckout { commits } => Some(format!(
            "this hexa is {commits} source commit{} behind this checkout: cargo build -p hexa-cli --release",
            if *commits == 1 { "" } else { "s" }
        )),
        Staleness::OffBranch => {
            Some("this hexa was built from a commit that is not on this branch".to_string())
        }
        Staleness::BehindRelease { latest } => {
            Some(format!("hexa {latest} is released and this one is older: hexa self-update"))
        }
    }
}

/// `X.Y.Z` or `vX.Y.Z`, compared numerically (ADR-2609151700).
fn version(s: &str) -> Option<(u64, u64, u64)> {
    let mut parts = s.trim().trim_start_matches('v').split('.');
    let v = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
    parts.next().is_none().then_some(v)
}

fn why_unknown(build: &Build, checkout: Option<&Checkout>, latest_release: Option<&str>) -> String {
    let source = match (&build.commit, checkout) {
        (None, _) => "this binary records no build commit",
        (Some(_), None) => "not run in a git checkout",
        (Some(_), Some(_)) => "this checkout does not hold the build commit",
    };
    let release = match latest_release {
        None => "no release answer is cached".to_string(),
        Some(r) => format!("the release feed's answer {r:?} is not a version"),
    };
    format!("{source}; {release}")
}
