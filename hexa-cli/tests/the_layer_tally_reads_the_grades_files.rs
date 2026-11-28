//! The "Hex layers" tally lists the grade's own files (ADR-2611221646 §3):
//! the grade reads `.tsx` and `.js` (ADR-2611270420), and it reads a file in a
//! hidden directory (it skips only its listed patterns), so the tally does the same.

use std::path::Path;
use std::process::Command;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn hex_layers_block(root: &Path) -> String {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hexa"))
        .args(["analyze", "."])
        .current_dir(root)
        .env("HOME", home.path())
        .env("NO_COLOR", "1")
        .env("CLICOLOR_FORCE", "0")
        .output()
        .unwrap();
    let all = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let start = all.find("Hex layers:").unwrap_or_else(|| panic!("no Hex layers block in:\n{all}"));
    all[start..].lines().take(8).collect::<Vec<_>>().join("\n")
}

/// One `.ts` and two `.tsx` use cases, two `.js`, and a `.ts` in a hidden directory,
/// all of which the grade reads. The `.js` files were the ones it did not read until
/// ADR-2611270420.
#[test]
fn tsx_js_and_hidden_dirs_are_counted() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", "{\"name\":\"fx\",\"version\":\"0.1.0\"}\n");
    write(r, "src/usecases/a.ts", "export const a = 1;\n");
    write(r, "src/usecases/b.tsx", "export const b = 2;\n");
    write(r, "src/usecases/c.tsx", "export const c = 3;\n");
    write(r, "src/usecases/d.js", "export const d = 4;\n");
    write(r, "src/usecases/f.js", "export const f = 6;\n");
    write(r, "src/usecases/.cache/e.ts", "export const e = 5;\n");
    let block = hex_layers_block(r);
    assert!(
        block.contains("Use Cases (6 files)"),
        "expected the six .ts/.tsx/.js files the grade reads (one in a hidden folder):\n{block}"
    );
}
