use std::process::Command;

#[test]
fn cli_routes_the_v1_engine_through_the_typed_driver() {
    let output = Command::new(env!("CARGO_BIN_EXE_lpp"))
        .arg("--version")
        .output()
        .expect("run lpp --version");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("L++ v{} (rewrite engine)\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}

/// Regression: `print("a", b)` used to slip through both type checkers
/// (the old `Any`-param arity exemption) and crash the Cranelift verifier
/// with a cryptic "mismatched argument count". It must now be rejected at
/// type-check time with a clear diagnostic.
#[test]
fn multi_arg_print_is_rejected_with_a_clear_error() {
    let dir = std::env::temp_dir().join(format!("lpp-print-arity-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("make temp dir");
    let file = dir.join("main.lpp");
    std::fs::write(&file, "def main():\n    print(\"sum =\", 2 + 2)\n").expect("write repro");

    let output = Command::new(env!("CARGO_BIN_EXE_lpp"))
        .arg(&file)
        .arg("--check")
        .output()
        .expect("run lpp --check");
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        !output.status.success(),
        "wrong-arity print must not type-check"
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("ArityMismatch")
            || combined.contains("arity mismatch")
            || combined.contains("print expects 1 arguments"),
        "expected the arity diagnostic, got:\n{combined}"
    );
}
