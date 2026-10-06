//! Gate: `keel test` runs every `tests/*.lpp` with `lpp <file> --run` and
//! reports a pass/fail table (non-zero exit from the test = FAIL).

use std::path::Path;

#[cfg(unix)]
fn make_executable(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(not(unix))]
fn make_executable(_p: &Path) {}

fn temp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("lpp-keel-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A fake `lpp` that records its arguments and FAILS (exit 3, stderr message)
/// when the file being run contains "fail" — simulating a failed assertion.
fn fake_lpp(root: &Path) -> std::path::PathBuf {
    let p = root.join("lpp-fake");
    std::fs::write(
        &p,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> {}\n\ncase \"$(basename \"$1\")\" in *fail*) echo \"assertion failed: 1 != 2\" >&2; exit 3;; esac\nexit 0\n",
            root.join("args.txt").display()
        ),
    )
    .unwrap();
    make_executable(&p);
    p
}

fn project(root: &Path) -> std::path::PathBuf {
    let proj = root.join("proj");
    std::fs::create_dir_all(proj.join("src")).unwrap();
    std::fs::write(
        proj.join("Keel.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(proj.join("src/main.lpp"), "fn main() {}\n").unwrap();
    proj
}

#[test]
fn no_tests_dir_is_ok() {
    let root = temp("none");
    let fake = fake_lpp(&root);
    let proj = project(&root);

    let res = keel::commands::test::test_run(&proj, fake.to_str().unwrap(), None);
    assert!(res.is_ok(), "no tests should be a clean pass: {res:?}");
    assert!(
        !root.join("args.txt").exists(),
        "lpp must not be invoked when there are no test files"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn all_tests_pass_and_only_lpp_files_run() {
    let root = temp("pass");
    let fake = fake_lpp(&root);
    let proj = project(&root);
    std::fs::create_dir_all(proj.join("tests")).unwrap();
    std::fs::write(proj.join("tests/alpha.lpp"), "fn main() {}\n").unwrap();
    std::fs::write(proj.join("tests/beta.lpp"), "fn main() {}\n").unwrap();
    std::fs::write(proj.join("tests/README.md"), "not a test\n").unwrap();

    let res = keel::commands::test::test_run(&proj, fake.to_str().unwrap(), None);
    assert!(res.is_ok(), "all-pass suite should succeed: {res:?}");

    let args = std::fs::read_to_string(root.join("args.txt")).unwrap();
    assert_eq!(
        args.matches("--run").count(),
        2,
        "one --run per .lpp test file:\n{args}"
    );
    assert!(args.contains("tests/alpha.lpp"), "{args}");
    assert!(args.contains("tests/beta.lpp"), "{args}");
    assert!(
        !args.contains("README"),
        "non-.lpp files must be ignored:\n{args}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn selected_workspace_test_runs_only_the_named_members_tests() {
    let root = temp("selected");
    let fake = fake_lpp(&root);
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        workspace.join("Keel.toml"),
        "[workspace]\nmembers = [\"first\", \"second\"]\n",
    )
    .unwrap();
    for package in ["first", "second"] {
        let directory = workspace.join(package);
        std::fs::create_dir_all(directory.join("tests")).unwrap();
        std::fs::write(
            directory.join("Keel.toml"),
            format!("[package]\nname = \"{package}\"\nversion = \"0.1.0\"\n"),
        )
        .unwrap();
        std::fs::write(
            directory.join(format!("tests/{package}.lpp")),
            "fn main() {}\n",
        )
        .unwrap();
    }

    let result = keel::commands::test::test_run_selected(
        &workspace,
        fake.to_str().unwrap(),
        None,
        Some("second"),
    );
    assert!(result.is_ok(), "selected tests should pass: {result:?}");
    let arguments = std::fs::read_to_string(root.join("args.txt")).unwrap();
    assert!(arguments.contains("second/tests/second.lpp"), "{arguments}");
    assert!(!arguments.contains("first/tests/first.lpp"), "{arguments}");

    let missing = keel::commands::test::test_run_selected(
        &workspace,
        fake.to_str().unwrap(),
        None,
        Some("missing"),
    );
    assert!(
        missing
            .unwrap_err()
            .contains("workspace member not found: missing")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn failing_test_fails_the_suite() {
    let root = temp("fail");
    let fake = fake_lpp(&root);
    let proj = project(&root);
    std::fs::create_dir_all(proj.join("tests")).unwrap();
    std::fs::write(proj.join("tests/good.lpp"), "fn main() {}\n").unwrap();
    std::fs::write(proj.join("tests/bad_fail.lpp"), "fn main() {}\n").unwrap();

    let res = keel::commands::test::test_run(&proj, fake.to_str().unwrap(), None);
    assert!(res.is_err(), "a failing test must fail the suite: {res:?}");
    let err = res.unwrap_err();
    assert!(
        err.contains("bad_fail.lpp"),
        "error names the failing test: {err}"
    );
    assert!(
        err.contains("1 passed"),
        "error reports the pass count: {err}"
    );
    assert!(
        err.contains("1 failed"),
        "error reports the fail count: {err}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
