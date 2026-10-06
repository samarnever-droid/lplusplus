//! Gate: `keel build` / `keel check` drive the `lpp` compiler.

use std::path::Path;

#[cfg(unix)]
fn make_executable(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(not(unix))]
fn make_executable(_p: &Path) {}

fn temp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("lpp-keel-build-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A fake `lpp` that records every argument it was called with.
fn fake_lpp(root: &Path) -> std::path::PathBuf {
    let p = root.join("lpp-fake");
    std::fs::write(
        &p,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> {}\n",
            root.join("args.txt").display()
        ),
    )
    .unwrap();
    make_executable(&p);
    p
}

#[test]
fn build_invokes_lpp_for_each_target() {
    let root = temp("multi");
    let fake = fake_lpp(&root);
    let proj = root.join("proj");
    std::fs::create_dir_all(proj.join("src")).unwrap();
    std::fs::write(
        proj.join("Keel.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[targets]\nsupported = [\"x86_64-linux-gnu\", \"wasm32-wasi\"]\n",
    )
    .unwrap();
    std::fs::write(proj.join("src/main.lpp"), "fn main() {}\n").unwrap();

    let res = keel::commands::build::build(
        &proj,
        fake.to_str().unwrap(),
        &|m, cwd, l, r| keel::commands::build::run_lpp_jobs(m, cwd, l, r),
        None,
    );
    assert!(res.is_ok(), "build should succeed: {res:?}");

    let args = std::fs::read_to_string(root.join("args.txt")).unwrap();
    // two targets -> two invocations, each with the entry + its --target + -o
    assert_eq!(
        args.matches("--target").count(),
        2,
        "one --target per cross target:\n{args}"
    );
    assert!(
        args.contains("src/main.lpp"),
        "should pass the entry point:\n{args}"
    );
    assert!(
        args.contains("x86_64-linux-gnu") && args.contains("wasm32-wasi"),
        "{args}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn build_defaults_to_host_without_target_flag() {
    let root = temp("host");
    let fake = fake_lpp(&root);
    let proj = root.join("proj");
    std::fs::create_dir_all(proj.join("src")).unwrap();
    std::fs::write(
        proj.join("Keel.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(proj.join("src/main.lpp"), "fn main() {}\n").unwrap();

    let res = keel::commands::build::build(
        &proj,
        fake.to_str().unwrap(),
        &|m, cwd, l, r| keel::commands::build::run_lpp_jobs(m, cwd, l, r),
        None,
    );
    assert!(res.is_ok(), "build should succeed: {res:?}");

    let args = std::fs::read_to_string(root.join("args.txt")).unwrap();
    assert!(
        !args.contains("--target"),
        "host build passes no --target:\n{args}"
    );
    assert!(args.contains("src/main.lpp"), "{args}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn selected_workspace_build_includes_path_dependencies_but_not_unrelated_members() {
    let root = temp("selected-member");
    let fake = fake_lpp(&root);
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        workspace.join("Keel.toml"),
        "[workspace]\nmembers = [\"core\", \"app\", \"extra\"]\n",
    )
    .unwrap();
    for package in ["core", "app", "extra"] {
        let directory = workspace.join(package);
        std::fs::create_dir_all(directory.join("src")).unwrap();
        let dependencies = if package == "app" {
            "\n[dependencies]\ncore = { path = \"../core\" }\n"
        } else {
            ""
        };
        std::fs::write(
            directory.join("Keel.toml"),
            format!("[package]\nname = \"{package}\"\nversion = \"0.1.0\"\n{dependencies}"),
        )
        .unwrap();
        std::fs::write(directory.join("src/main.lpp"), "fn main() {}\n").unwrap();
    }

    let result = keel::commands::build::build_selected(
        &workspace,
        fake.to_str().unwrap(),
        &|member, cwd, lpp, rows| keel::commands::build::run_lpp_jobs(member, cwd, lpp, rows),
        None,
        Some("app"),
    );
    assert!(result.is_ok(), "selected build should succeed: {result:?}");
    let arguments = std::fs::read_to_string(root.join("args.txt")).unwrap();
    assert!(arguments.contains("core/src/main.lpp"), "{arguments}");
    assert!(arguments.contains("app/src/main.lpp"), "{arguments}");
    assert!(!arguments.contains("extra/src/main.lpp"), "{arguments}");

    let missing = keel::commands::build::build_selected(
        &workspace,
        fake.to_str().unwrap(),
        &|member, cwd, lpp, rows| keel::commands::build::run_lpp_jobs(member, cwd, lpp, rows),
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
fn check_invokes_lpp_with_check_flag() {
    let root = temp("check");
    let fake = fake_lpp(&root);
    let proj = root.join("proj");
    std::fs::create_dir_all(proj.join("src")).unwrap();
    std::fs::write(
        proj.join("Keel.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(proj.join("src/main.lpp"), "fn main() {}\n").unwrap();

    let res = keel::commands::build::check(&proj, fake.to_str().unwrap(), None);
    assert!(res.is_ok(), "check should succeed: {res:?}");

    let args = std::fs::read_to_string(root.join("args.txt")).unwrap();
    assert!(args.contains("--check"), "check passes --check:\n{args}");
    let _ = std::fs::remove_dir_all(&root);
}
