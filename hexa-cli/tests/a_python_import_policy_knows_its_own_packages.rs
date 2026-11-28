//! An `[[import_policy]]` over Python code (ADR-2611211324 §2): the project's
//! own packages, relative imports and the standard library are not the
//! policy's business; a denied module and an undeclared dependency are.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

#[test]
fn only_the_denied_and_the_undeclared_python_imports_are_reported() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "pyproject.toml", "[project]\nname = \"app\"\n");
    write(r, "app/__init__.py", "");
    write(r, "app/domain/__init__.py", "");
    write(r, "app/ports/__init__.py", "");
    write(r, "app/ports/repo.py", "class Repo:\n    pass\n");
    write(
        r,
        "app/domain/order.py",
        "from __future__ import annotations\nimport os\nimport socket\nimport requests\nfrom app.ports import repo\nfrom ..ports.repo import Repo\n\nclass Order:\n    pass\n",
    );
    write(
        r,
        ".hexa/ADR-rules.toml",
        "[[import_policy]]\nadr = \"ADR-0001\"\nid = \"domain-pure\"\nmessage = \"domain stays pure\"\nlayer = \"/domain/\"\ndeny = [\"socket\"]\n",
    );
    let o = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["analyze", "."])
        .current_dir(r)
        .env("HEXA_NEXUS_URL", "http://127.0.0.1:1")
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&o.stdout).to_string() + &String::from_utf8_lossy(&o.stderr);
    let sites: Vec<&str> = out.lines().filter(|l| l.contains("domain stays pure [")).collect();
    assert_eq!(sites.len(), 2, "exactly socket and requests:\n{out}");
    assert!(sites.iter().any(|l| l.contains("`socket` — denied")), "{out}");
    assert!(sites.iter().any(|l| l.contains("`requests` — not in this policy's `allow`")), "{out}");
    for own in ["`app", "`..ports", "`os`", "`__future__`"] {
        assert!(!sites.iter().any(|l| l.contains(own)), "{own} is not the policy's business:\n{out}");
    }
}
