use std::process::Command;

fn main() {
    // Tell Cargo to recompile when any embedded asset changes.
    // Without this, rust-embed embeds stale content after asset edits.
    println!("cargo:rerun-if-changed=assets/");
    provenance();
    hexa_ledger();
}

/// hexa's own decisions, which the files it ships cite (ADR-2610281552 §1):
/// the ids of `docs/adrs/` in this source tree. Without that directory (a
/// build from a published crate), none.
fn hexa_ledger() {
    // The ledger and its `historical/` part, as `hexa adr doctor` reads them.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/adrs");
    let dirs = [dir.clone(), dir.join("historical")];
    let mut names: Vec<String> = Vec::new();
    for d in &dirs {
        println!("cargo:rerun-if-changed={}", d.display());
        names.extend(
            std::fs::read_dir(d).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()),
        );
    }
    let ids = hexa_core::ports::project_floor::adr_ids(&names);
    let body = format!(
        "/// hexa's own ADR ids, embedded at build time.\npub const HEXA_ADR_IDS: &[&str] = &[{}];\n",
        ids.iter().map(|i| format!("{i:?}")).collect::<Vec<_>>().join(", ")
    );
    let out = std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("hexa_adr_ids.rs");
    std::fs::write(out, body).expect("write hexa_adr_ids.rs");
}

/// The commit this binary is built from, and whether the tree was dirty
/// (ADR-2610270853 §1). Outside a git tree nothing is recorded.
fn provenance() {
    let git = |args: &[&str]| -> Option<String> {
        let out = Command::new("git").args(args).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let Some(commit) = git(&["rev-parse", "HEAD"]).filter(|c| !c.is_empty()) else {
        println!("cargo:rustc-env=HEXA_BUILD_COMMIT=");
        println!("cargo:rustc-env=HEXA_BUILD_DIRTY=");
        println!("cargo:rustc-env=HEXA_VERSION_LINE={version}");
        return;
    };
    // A new commit, checkout or staged change rebuilds the record.
    if let Some(dir) = git(&["rev-parse", "--absolute-git-dir"]) {
        println!("cargo:rerun-if-changed={dir}/HEAD");
        println!("cargo:rerun-if-changed={dir}/index");
        if let Some(head_ref) = git(&["symbolic-ref", "-q", "HEAD"]) {
            println!("cargo:rerun-if-changed={dir}/{head_ref}");
        }
    }
    let dirty = git(&["status", "--porcelain"]).is_some_and(|s| !s.is_empty());
    let short = &commit[..commit.len().min(7)];
    let mark = if dirty { "-dirty" } else { "" };
    println!("cargo:rustc-env=HEXA_BUILD_COMMIT={commit}");
    println!("cargo:rustc-env=HEXA_BUILD_DIRTY={}", if dirty { "1" } else { "" });
    println!("cargo:rustc-env=HEXA_VERSION_LINE={version} ({short}{mark})");
}
