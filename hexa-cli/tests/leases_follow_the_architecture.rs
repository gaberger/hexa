//! ADR-2611160750 (leases follow the architecture), through the built binary,
//! against temp repositories with a hexagonal layout:
//!
//! 1. `--lease usecases/billing` names a layer slice and is recorded as the
//!    paths it resolves to; a plain path is recorded as before; a slice that
//!    matches nothing is refused.
//! 2. A lease on ports/ or domain/ is a contract lease: refused while a live
//!    lease covers a file that imports it, naming the consumer task and file.
//!    A use-case or adapter lease records the port and domain files it imports
//!    as `frozen`.
//! 3. `merge-order` sorts finished tasks domain → ports → use cases →
//!    adapters → composition root; ties keep finish order.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn hexa(home: &Path, repo: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(args)
        .current_dir(repo)
        .env("HOME", home)
        .env("HEXA_HOME", home.join(".hexa"))
        .env("CLAUDE_PROJECT_DIR", repo)
        .env_remove("HEXA_TASK")
        .env_remove("HEXA_INTERNAL")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap()
}

fn json(o: &Output) -> serde_json::Value {
    serde_json::from_slice(&o.stdout).unwrap_or_else(|e| {
        panic!(
            "not JSON ({e}): {}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        )
    })
}

fn git(dir: &Path, args: &[&str]) {
    let o = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@x"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
}

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// A temp repo with `files`, committed. Returns (tmp guard, home, repo).
fn repo_with(files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let repo = std::fs::canonicalize(tmp.path()).unwrap().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    for (rel, text) in files {
        write(&repo, rel, text);
    }
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "init"]);
    (tmp, home, repo)
}

/// A Rust crate laid out by layer.
fn rust_repo() -> (tempfile::TempDir, PathBuf, PathBuf) {
    repo_with(&[
        ("Cargo.toml", "[package]\nname = \"billing-app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        ("src/lib.rs", "pub mod domain;\npub mod ports;\npub mod usecases;\npub mod adapters;\n"),
        ("src/domain/mod.rs", "pub mod invoice;\n"),
        ("src/domain/invoice.rs", "pub struct Invoice;\n"),
        ("src/ports/mod.rs", "pub mod billing;\n"),
        (
            "src/ports/billing.rs",
            "use crate::domain::invoice::Invoice;\npub trait Billing { fn charge(&self, i: Invoice); }\n",
        ),
        ("src/usecases/mod.rs", "pub mod billing;\npub mod reports;\n"),
        ("src/usecases/billing/mod.rs", "pub mod charge;\n"),
        (
            "src/usecases/billing/charge.rs",
            "use crate::ports::billing::Billing;\nuse crate::domain::invoice::Invoice;\npub fn charge(b: &dyn Billing) { b.charge(Invoice) }\n",
        ),
        ("src/usecases/reports/mod.rs", "pub fn report() {}\n"),
        ("src/adapters/mod.rs", "pub mod primary;\npub mod secondary;\n"),
        ("src/adapters/secondary/mod.rs", "pub mod postgres;\n"),
        ("src/adapters/secondary/postgres/mod.rs", "pub mod store;\n"),
        (
            "src/adapters/secondary/postgres/store.rs",
            "use crate::{ports::billing::Billing, domain::invoice::Invoice};\npub struct Pg;\nimpl Billing for Pg { fn charge(&self, _i: Invoice) {} }\n",
        ),
        ("src/adapters/primary/mod.rs", "pub mod cli;\n"),
        ("src/adapters/primary/cli/mod.rs", "use crate::usecases::billing::charge;\npub fn run() { let _ = charge::charge; }\n"),
    ])
}

fn register(home: &Path, repo: &Path, name: &str, leases: &[&str], extra: &[&str]) -> Output {
    let mut args = vec!["agent", "register", "--task", name];
    for l in leases {
        args.push("--lease");
        args.push(l);
    }
    args.extend_from_slice(extra);
    args.push("--json");
    hexa(home, repo, &args)
}

fn ok(o: &Output) -> serde_json::Value {
    let v = json(o);
    assert!(
        o.status.success() && v["ok"] == true,
        "expected success: {v}\n{}",
        String::from_utf8_lossy(&o.stderr)
    );
    v
}

fn refused(o: &Output, code: &str) -> serde_json::Value {
    let v = json(o);
    assert!(!o.status.success(), "expected a refusal ({code}): {v}");
    assert_eq!(v["code"], code, "{v}");
    v
}

fn strs(v: &serde_json::Value) -> Vec<String> {
    v.as_array()
        .unwrap_or_else(|| panic!("not an array: {v}"))
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect()
}

// ── 1. layer slices ──────────────────────────────────────────────────────────

#[test]
fn a_layer_slice_resolves_to_its_files_a_plain_path_is_unchanged_and_an_empty_slice_is_refused() {
    let (_t, home, repo) = rust_repo();

    let v = ok(&register(&home, &repo, "uc", &["usecases/billing"], &[]));
    assert_eq!(strs(&v["leases"]), vec!["src/usecases/billing"], "{v}");

    let v = ok(&register(
        &home,
        &repo,
        "pg",
        &["adapters/secondary/postgres"],
        &["--dry-run"],
    ));
    assert_eq!(
        strs(&v["would_register"]["leases"]),
        vec!["src/adapters/secondary/postgres"],
        "{v}"
    );

    // A plain path is recorded as it was given, whether it exists or not.
    let v = ok(&register(
        &home,
        &repo,
        "reports",
        &["src/usecases/reports/"],
        &[],
    ));
    assert_eq!(strs(&v["leases"]), vec!["src/usecases/reports"], "{v}");
    let v = ok(&register(&home, &repo, "new", &["src/nowhere/"], &[]));
    assert_eq!(strs(&v["leases"]), vec!["src/nowhere"], "{v}");

    // A slice naming no file is refused, and says so.
    let v = refused(
        &register(&home, &repo, "none", &["usecases/shipping"], &[]),
        "lease_slice_empty",
    );
    assert_eq!(v["field"], "lease");
    assert!(
        v["message"].as_str().unwrap().contains("usecases/shipping"),
        "{v}"
    );
    assert!(!v["hint"].as_str().unwrap().is_empty());

    // The slice lease binds what it resolved to: a second slice over the same files overlaps.
    refused(
        &register(&home, &repo, "again", &["usecases/billing/charge"], &[]),
        "lease_overlap",
    );
}

// ── 2. contract leases ───────────────────────────────────────────────────────

#[test]
fn a_contract_lease_is_refused_while_a_live_lease_imports_it_naming_the_consumer() {
    let (_t, home, repo) = rust_repo();
    let uc = ok(&register(
        &home,
        &repo,
        "charge",
        &["usecases/billing"],
        &[],
    ));
    let uc_id = uc["id"].as_str().unwrap().to_string();

    for contract in ["src/ports/billing.rs", "ports/billing", "domain"] {
        let v = refused(
            &register(&home, &repo, "port", &[contract], &[]),
            "lease_contract_in_use",
        );
        let msg = v["message"].as_str().unwrap();
        assert!(
            msg.contains(&uc_id),
            "names the consumer task ({contract}): {v}"
        );
        assert!(
            msg.contains("src/usecases/billing/charge.rs"),
            "names the consumer file ({contract}): {v}"
        );
        assert_eq!(v["field"], "lease");
    }
    // A port file nobody under a live lease imports is free.
    ok(&register(
        &home,
        &repo,
        "mod",
        &["src/ports/mod.rs"],
        &["--dry-run"],
    ));

    // Once the consumer is released, the contract can be leased.
    ok(&hexa(
        &home,
        &repo,
        &["agent", "release", "--id", &uc_id, "--json"],
    ));
    ok(&register(
        &home,
        &repo,
        "port",
        &["src/ports/billing.rs"],
        &[],
    ));
}

#[test]
fn an_outer_lease_records_the_contracts_it_imports_as_frozen_and_waits_for_a_live_contract_lease() {
    let (_t, home, repo) = rust_repo();

    let v = ok(&register(
        &home,
        &repo,
        "charge",
        &["usecases/billing"],
        &[],
    ));
    let mut frozen = strs(&v["frozen"]);
    frozen.sort();
    assert_eq!(
        frozen,
        vec!["src/domain/invoice.rs", "src/ports/billing.rs"],
        "{v}"
    );
    let listed = json(&hexa(&home, &repo, &["agent", "list", "--json"]));
    assert_eq!(
        listed["tasks"][0]["frozen"], v["frozen"],
        "the ledger stores it: {listed}"
    );

    // A grouped `use crate::{…}` is read too, and adapters do not conflict with each other.
    let v = ok(&register(
        &home,
        &repo,
        "pg",
        &["adapters/secondary/postgres"],
        &[],
    ));
    let mut frozen = strs(&v["frozen"]);
    frozen.sort();
    assert_eq!(
        frozen,
        vec!["src/domain/invoice.rs", "src/ports/billing.rs"],
        "{v}"
    );
    let v = ok(&register(
        &home,
        &repo,
        "cli",
        &["adapters/primary/cli"],
        &[],
    ));
    assert!(
        strs(&v["frozen"]).is_empty(),
        "the cli imports a use case, not a contract: {v}"
    );

    // The other direction: a live contract lease holds back a consumer that would freeze it.
    let (_t2, home2, repo2) = rust_repo();
    let port = ok(&register(
        &home2,
        &repo2,
        "port",
        &["src/ports/billing.rs"],
        &[],
    ));
    let v = refused(
        &register(&home2, &repo2, "charge", &["usecases/billing"], &[]),
        "lease_contract_held",
    );
    assert!(
        v["message"]
            .as_str()
            .unwrap()
            .contains(port["id"].as_str().unwrap()),
        "{v}"
    );
}

#[test]
fn typescript_relative_imports_count_as_consumers() {
    let (_t, home, repo) = repo_with(&[
        ("package.json", "{\"name\":\"billing\",\"type\":\"module\"}\n"),
        ("src/domain/invoice.ts", "export interface Invoice { total: number }\n"),
        ("src/ports/billing.ts", "import type { Invoice } from '../domain/invoice.js';\nexport interface Billing { charge(i: Invoice): void }\n"),
        ("src/usecases/billing/charge.ts", "import { Billing } from '../../ports/billing.js';\nexport const charge = (b: Billing) => b;\n"),
        ("src/adapters/secondary/postgres/store.ts", "import type { Billing } from \"../../../ports/billing.js\";\nexport class Pg {}\n"),
    ]);
    let uc = ok(&register(
        &home,
        &repo,
        "charge",
        &["usecases/billing"],
        &[],
    ));
    assert_eq!(strs(&uc["frozen"]), vec!["src/ports/billing.ts"], "{uc}");
    let v = refused(
        &register(&home, &repo, "port", &["ports/billing"], &[]),
        "lease_contract_in_use",
    );
    assert!(
        v["message"]
            .as_str()
            .unwrap()
            .contains("src/usecases/billing/charge.ts"),
        "{v}"
    );
}

#[test]
fn python_absolute_and_relative_imports_count_as_consumers() {
    let (_t, home, repo) = repo_with(&[
        ("pyproject.toml", "[project]\nname = \"billing\"\nversion = \"0.1.0\"\n"),
        ("src/billing/__init__.py", ""),
        ("src/billing/domain/__init__.py", ""),
        ("src/billing/domain/invoice.py", "class Invoice:\n    total: int = 0\n"),
        ("src/billing/ports/__init__.py", ""),
        ("src/billing/ports/billing.py", "from billing.domain.invoice import Invoice\n\n\nclass Billing:\n    def charge(self, i: Invoice) -> None: ...\n"),
        ("src/billing/usecases/__init__.py", ""),
        ("src/billing/usecases/charge.py", "import billing.ports.billing as ports\n\n\ndef charge(b: ports.Billing):\n    return b\n"),
        ("src/billing/adapters/secondary/postgres/store.py", "from ....ports.billing import Billing\n\n\nclass Pg(Billing): ...\n"),
        ("src/billing/adapters/secondary/memory/store.py", "from billing.ports import billing\n\n\nclass Mem(billing.Billing): ...\n"),
    ]);
    // absolute `import a.b.c as x`
    let uc = ok(&register(
        &home,
        &repo,
        "charge",
        &["src/billing/usecases/charge.py"],
        &[],
    ));
    assert_eq!(
        strs(&uc["frozen"]),
        vec!["src/billing/ports/billing.py"],
        "{uc}"
    );
    // relative `from ....ports.billing import X`
    let pg = ok(&register(
        &home,
        &repo,
        "pg",
        &["src/billing/adapters/secondary/postgres"],
        &[],
    ));
    assert_eq!(
        strs(&pg["frozen"]),
        vec!["src/billing/ports/billing.py"],
        "{pg}"
    );
    // `from pkg.ports import billing` names a submodule, not the package
    let mem = ok(&register(
        &home,
        &repo,
        "mem",
        &["src/billing/adapters/secondary/memory"],
        &[],
    ));
    assert_eq!(
        strs(&mem["frozen"]),
        vec!["src/billing/ports/billing.py"],
        "{mem}"
    );
    // a contract lease is refused while a live lease imports it
    let v = refused(
        &register(&home, &repo, "port", &["src/billing/ports/billing.py"], &[]),
        "lease_contract_in_use",
    );
    assert!(
        v["message"]
            .as_str()
            .unwrap()
            .contains("src/billing/usecases/charge.py"),
        "{v}"
    );
}

#[test]
fn go_package_imports_count_as_consumers() {
    let (_t, home, repo) = repo_with(&[
        ("go.mod", "module example.com/billing\n\ngo 1.22\n"),
        ("domain/invoice.go", "package domain\n\ntype Invoice struct{ Total int }\n"),
        ("ports/billing.go", "package ports\n\nimport \"example.com/billing/domain\"\n\ntype Billing interface{ Charge(domain.Invoice) }\n"),
        ("ports/billing_test.go", "package ports\n"),
        ("usecases/charge.go", "package usecases\n\nimport (\n\t\"fmt\"\n\tp \"example.com/billing/ports\"\n)\n\nfunc Charge(b p.Billing) { fmt.Println(b) }\n"),
        ("adapters/secondary/pg/store.go", "package pg\n\nimport \"example.com/billing/ports\"\n\ntype Pg struct{ ports.Billing }\n"),
    ]);
    // a package import freezes the package's non-test files; std and foreign imports are not repo files
    let uc = ok(&register(&home, &repo, "charge", &["usecases/charge.go"], &[]));
    assert_eq!(strs(&uc["frozen"]), vec!["ports/billing.go"], "{uc}");
    let pg = ok(&register(&home, &repo, "pg", &["adapters/secondary/pg"], &[]));
    assert_eq!(strs(&pg["frozen"]), vec!["ports/billing.go"], "{pg}");
    let v = refused(&register(&home, &repo, "port", &["ports/billing.go"], &[]), "lease_contract_in_use");
    assert!(v["message"].as_str().unwrap().contains("usecases/charge.go"), "{v}");
}

// ── 3. merge order ───────────────────────────────────────────────────────────

#[test]
fn merge_order_goes_domain_ports_usecases_adapters_composition_root_and_ties_keep_finish_order() {
    let (_t, home, repo) = rust_repo();
    let wt = |n: &str| repo.parent().unwrap().join(n).display().to_string();
    let head = String::from_utf8(
        std::process::Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .current_dir(&repo)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();
    // Finished outermost first, so finish order alone would be exactly backwards.
    let plan: [(&str, &str); 6] = [
        ("root", "src/lib.rs"),
        ("pg", "adapters/secondary/postgres"),
        ("cli", "adapters/primary/cli"),
        ("uc", "usecases/billing"),
        ("port", "src/ports/billing.rs"),
        ("dom", "domain"),
    ];
    let mut ids = std::collections::HashMap::new();
    for (i, (name, lease)) in plan.iter().enumerate() {
        if i > 0 {
            // Finish times have one-second resolution.
            std::thread::sleep(std::time::Duration::from_millis(1100));
        }
        // `passed` is earned: a gate that passes, and a commit that exists.
        let gate = "echo 'test result: ok. 1 passed; 0 failed; 0 ignored'";
        let v = ok(&register(
            &home,
            &repo,
            name,
            &[lease],
            &["--worktree", &wt(name), "--gate", gate],
        ));
        let id = v["id"].as_str().unwrap().to_string();
        ok(&hexa(
            &home,
            &repo,
            &[
                "agent",
                "report",
                "--id",
                &id,
                "--outcome",
                "passed",
                "--commit",
                &head,
                "--json",
            ],
        ));
        ids.insert(id, *name);
    }
    let order = json(&hexa(
        &home,
        &repo,
        &["dev", "worktree", "merge-order", "--json"],
    ));
    let names: Vec<&str> = order["order"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| ids[t["id"].as_str().unwrap()])
        .collect();
    assert_eq!(
        names,
        vec!["dom", "port", "uc", "pg", "cli", "root"],
        "{order}"
    );
    assert_eq!(order["order"][0]["layer"], "domain", "{order}");
}
