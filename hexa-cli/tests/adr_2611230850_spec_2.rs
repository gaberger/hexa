#![allow(unused, clippy::all)]
/// S01: --lang python writes the whole Python counter layout, with the package named after the project
#[test]
fn s01_python_scaffold_writes_complete_layout() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("proj");
    std::fs::create_dir_all(&dir).expect("create dir");

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_hexa"))
        .arg("init")
        .arg(&dir)
        .args([
            "--name",
            "my-app",
            "--scaffold",
            "--lang",
            "python",
            "--no-claude-md",
        ])
        .output()
        .expect("run hexa");

    assert!(
        out.status.success(),
        "hexa init failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let expected = [
        "pyproject.toml",
        "my_app/__init__.py",
        "my_app/domain/count.py",
        "my_app/ports/store.py",
        "my_app/usecases/increment.py",
        "my_app/adapters/secondary/memory.py",
        "my_app/composition_root.py",
        "tests/__init__.py",
        "tests/test_increment.py",
    ];

    for rel in expected {
        assert!(
            dir.join(rel).is_file(),
            "expected file {rel} is missing from the Python scaffold"
        );
    }
}
