//! Python, where Rust, Go and TypeScript were already handled: `gate coverage`,
//! `doctor` and `dev validate`.

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn hexa(root: &Path, args: &[&str]) -> (bool, String) {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(args)
        .current_dir(root)
        .env("HOME", home.path())
        .env("HEXA_HOME", home.path().join(".hexa"))
        .env("NO_COLOR", "1")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::Builder::new().prefix("hexaproj-").tempdir().unwrap();
    write(dir.path(), "pyproject.toml", "[project]\nname = \"p\"\nversion = \"0.1.0\"\n");
    dir
}

const GATE: &str = "python3 -c 'import pkg.zq_used'";

#[test]
fn gate_coverage_measures_python_modules() {
    let dir = project();
    let root = dir.path();
    write(root, "pkg/__init__.py", "\"\"\"A package.\"\"\"\n# nothing to run\n");
    write(root, "pkg/zq_used.py", "VALUE = 1\n");
    write(root, "pkg/zq_unused.py", "VALUE = 2\n");
    for test_file in ["test_zq.py", "zq_test.py", "conftest.py", "tests/zq_helper.py"] {
        write(root, test_file, "VALUE = 3\n");
    }
    // The composition root, found by its Python name: wiring, not a subject.
    write(root, "src/main.py", "VALUE = 4\n");
    for installed in [".venv/lib/zq_dep.py", ".venv/lib/zq_dep.rs", "venv/zq_other.go", "pkg/__pycache__/zq.py", "site-packages/zq_site.py"] {
        write(root, installed, "x\n");
    }

    let (ok, out) = hexa(root, &["gate", "coverage", ".", "--gate", GATE, "--uncounted"]);
    assert!(!ok, "a module the gate never needs fails the run:\n{out}");
    assert!(out.contains("1 of 2 runtime modules are verified by nothing"), "{out}");
    assert!(out.contains("pkg/zq_unused.py"), "{out}");
    assert!(out.contains("pkg/zq_used.py"), "{out}");
    for skipped in ["__init__", "test_zq", "zq_test", "conftest", "zq_helper", "zq_dep", "zq_other", "zq_site", "__pycache__", "src/main.py"] {
        assert!(!out.contains(skipped), "{skipped} is not a module to measure:\n{out}");
    }
    assert!(root.join("pkg/zq_unused.py").is_file(), "every deleted module is restored");
}

#[test]
fn gate_coverage_of_a_tree_it_cannot_read_does_not_pass() {
    let dir = project();
    let root = dir.path();
    write(root, "notes/readme.txt", "x\n");
    write(root, "app/Zq.java", "class Zq {}\n");
    let (ok, out) = hexa(root, &["gate", "coverage", ".", "--gate", "true", "--uncounted"]);
    assert!(!ok, "{out}");
    assert!(out.contains("no source to measure"), "{out}");
    assert!(!out.contains("0 of 0"), "{out}");
}

#[test]
fn doctor_names_a_python_project() {
    let dir = project();
    let (_, out) = hexa(dir.path(), &["doctor"]);
    assert!(out.contains("python (pyproject.toml)"), "{out}");
}

#[test]
fn dev_validate_builds_and_tests_a_python_project() {
    let dir = project();
    let root = dir.path();
    write(root, "src/domain/zq.py", "def zq() -> int:\n    return 1\n\n\n_X = zq()\n");
    // A stand-in for pytest in the project, so the run does not depend on what
    // this machine has installed: `python3 -m pytest` finds it first.
    write(root, "pytest.py", "import sys\n\nprint('zq-test-ran')\nsys.exit(1)\n");
    let (ok, out) = hexa(root, &["dev", "validate"]);
    assert!(!ok, "{out}");
    assert!(!out.contains("no build system detected"), "{out}");
    assert!(out.contains("pytest failed") && out.contains("zq-test-ran"), "a failing test fails the stage:\n{out}");
    write(root, "pytest.py", "print('1 passed')\n");
    let (_, out) = hexa(root, &["dev", "validate"]);
    assert!(!out.contains("pytest failed"), "a passing run passes the stage:\n{out}");
    assert!(out.contains("✓ tests"), "{out}");

    write(root, "src/domain/zq_broken.py", "def (:\n");
    let (_, out) = hexa(root, &["dev", "validate", "--skip-test"]);
    assert!(out.contains("python build failed") && out.contains("zq_broken"), "a module that does not compile fails the build:\n{out}");
    fs::remove_file(root.join("src/domain/zq_broken.py")).unwrap();
    for environment in [".tox/zq.py", ".nox/zq.py", "build/zq.py"] {
        write(root, environment, "def (:\n");
    }
    let (_, out) = hexa(root, &["dev", "validate", "--skip-test"]);
    assert!(!out.contains("python build failed"), "an environment's files are not the project's:\n{out}");
    assert!(!root.join("src/domain/__pycache__").exists(), "validating writes nothing into the tree");
}

/// Wiring and tooling by their Python names, wherever the package puts them.
#[test]
fn gate_coverage_skips_python_wiring_and_tooling() {
    let dir = project();
    let root = dir.path();
    write(root, "pkg/zq_used.py", "VALUE = 1\n");
    for wiring in ["app/composition_root.py", "app/__main__.py", "setup.py", "manage.py", "noxfile.py", "app/migrations/zq_0001.py"] {
        write(root, wiring, "VALUE = 2\n");
    }
    for (blank, body) in [("app/__init__.py", "\"An app.\"\n"), ("pkg/__init__.py", "r\"\"\"A package.\"\"\"\n"), ("pkg/zq_empty.py", "'Nothing.'  # yet\n")] {
        write(root, blank, body);
    }
    let (ok, out) = hexa(root, &["gate", "coverage", ".", "--gate", GATE, "--uncounted"]);
    assert!(ok, "{out}");
    assert!(out.contains("0 of 1 runtime modules are verified by nothing"), "{out}");
}

/// A deleted module comes back as it was: its mode, and a link as a link.
#[cfg(unix)]
#[test]
fn gate_coverage_restores_a_file_as_it_was() {
    use std::os::unix::fs::PermissionsExt;
    let dir = project();
    let root = dir.path();
    write(root, "pkg/zq_used.py", "VALUE = 1\n");
    write(root, "pkg/zq_script.py", "#!/usr/bin/env python3\nVALUE = 2\n");
    fs::set_permissions(root.join("pkg/zq_script.py"), fs::Permissions::from_mode(0o755)).unwrap();
    write(root, "shared/zq_real.txt", "VALUE = 3\n");
    std::os::unix::fs::symlink("../shared/zq_real.txt", root.join("pkg/zq_link.py")).unwrap();

    let (_, out) = hexa(root, &["gate", "coverage", ".", "--gate", GATE, "--uncounted"]);
    let mode = fs::metadata(root.join("pkg/zq_script.py")).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o755, "{out}");
    assert_eq!(fs::read_link(root.join("pkg/zq_link.py")).unwrap(), Path::new("../shared/zq_real.txt"), "{out}");
    assert_eq!(fs::read_to_string(root.join("shared/zq_real.txt")).unwrap(), "VALUE = 3\n");
}
