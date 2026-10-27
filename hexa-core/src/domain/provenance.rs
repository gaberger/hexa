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
pub fn staleness(build: &Build, checkout: Option<&Checkout>, latest_release: Option<&str>) -> Vec<Staleness> {
    let _ = (build, checkout, latest_release);
    unimplemented!("ADR-2610270853: built to its sealed spec")
}

/// The one line a session or `hexa doctor` prints, or `None` when current.
pub fn advice(s: &Staleness) -> Option<String> {
    let _ = s;
    unimplemented!("ADR-2610270853: built to its sealed spec")
}
