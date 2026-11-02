//! ADR-2611020815 §5: the commands that run work record their evidence through
//! `record_run_gate`, never by writing `"gate"` through `update_loop`, which
//! merges blindly and replaced the loop's gate (strata, 2026-11-02).

#[test]
fn run_gate_no_caller_writes_gate_through_update_loop() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands");
    let mut checked = 0;
    for f in ["direct.rs", "build.rs"] {
        let text = std::fs::read_to_string(root.join(f)).unwrap_or_else(|e| panic!("{f}: {e}"));
        checked += 1;
        for (n, line) in text.lines().enumerate() {
            if line.contains("update_loop") {
                // The patch may span lines; read the call up to its closing `);`.
                let call: String = text.lines().skip(n).take(6).collect::<Vec<_>>().join(" ");
                let call = &call[..call.find(");").map_or(call.len(), |i| i + 2)];
                assert!(
                    !call.contains("\"gate\""),
                    "{f}:{}: writes the gate through update_loop: {call}",
                    n + 1
                );
            }
        }
        assert!(
            text.contains("note_run_gate") || text.contains("record_run_gate"),
            "{f} does not record its gate through note_run_gate/record_run_gate"
        );
    }
    assert_eq!(checked, 2, "vacuous: no caller was read");
}
