//! Gate: `keel build` over a monorepo — topological order over the
//! path-dep DAG, CONCURRENT builds within a layer, one `Keel.lock` for the
//! whole workspace (docs/rewrite/WORKSPACE.md).

use std::path::Path;
use std::process::Command;

#[cfg(unix)]
fn make_executable(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(not(unix))]
fn make_executable(_p: &Path) {}

fn temp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("lpp-keel-ws-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::remove_file(d.join("Keel.lock")).ok();
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn git(cwd: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git must be available on PATH");
    assert!(out.status.success(), "git {args:?} failed");
}

/// A bare git repo (a "remote") with an empty initial commit on `main`.
fn seed_bare(dir: &Path) -> std::path::PathBuf {
    let bare = dir.join("remote.git");
    git(
        dir,
        &[
            "init",
            "--bare",
            "--initial-branch=main",
            bare.to_str().unwrap(),
        ],
    );
    let work = dir.join("seed");
    git(dir, &["init", "-b", "main", work.to_str().unwrap()]);
    git(&work, &["config", "user.name", "Seeder"]);
    git(&work, &["config", "user.email", "seeder@example.com"]);
    git(&work, &["commit", "--allow-empty", "-m", "init"]);
    git(&work, &["push", bare.to_str().unwrap(), "HEAD:main"]);
    let _ = std::fs::remove_dir_all(&work);
    bare
}

fn pkg_manifest(name: &str, deps: &str) -> String {
    format!("[package]\nname = \"{name}\"\nversion = \"1.0.0\"\n{deps}\n")
}

/// The diamond: app → (b, c) → d, in a virtual workspace.
fn diamond(root: &Path) {
    std::fs::write(
        root.join("Keel.toml"),
        "[workspace]\nmembers = [\"crates/*\", \"apps/app\"]\n",
    )
    .unwrap();
    for (dir, name, deps) in [
        ("crates/d", "d", ""),
        ("crates/b", "b", "[dependencies]\nd = { path = \"../d\" }"),
        ("crates/c", "c", "[dependencies]\nd = { path = \"../d\" }"),
        (
            "apps/app",
            "app",
            "[dependencies]\nb = { path = \"../../crates/b\" }\nc = { path = \"../../crates/c\" }",
        ),
    ] {
        let p = root.join(dir);
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(p.join("Keel.toml"), pkg_manifest(name, deps)).unwrap();
        std::fs::write(p.join("src/main.lpp"), "fn main() {}\n").unwrap();
    }
}

/// A fake `lpp` that appends one line per invocation.
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

/// A fake `lpp` that records START/END around a 0.5s "compile".
fn slow_fake_lpp(root: &Path) -> std::path::PathBuf {
    let p = root.join("lpp-fake");
    std::fs::write(
        &p,
        format!(
            "#!/bin/sh\necho \"START $1\" >> {}\nsleep 0.5\necho \"END $1\" >> {}\n",
            root.join("marks.txt").display(),
            root.join("marks.txt").display()
        ),
    )
    .unwrap();
    make_executable(&p);
    p
}

#[test]
fn build_runs_dependencies_before_dependents() {
    let root = temp("order");
    diamond(&root);
    let fake = fake_lpp(&root);

    let res = keel::commands::build::build(
        &root,
        fake.to_str().unwrap(),
        &|m, cwd, l, r| keel::commands::build::run_lpp_jobs(m, cwd, l, r),
        None,
    );
    assert!(res.is_ok(), "build should succeed: {res:?}");

    let args = std::fs::read_to_string(root.join("args.txt")).unwrap();
    let line = |member: &str| {
        args.lines()
            .position(|l| l.contains(&format!("{member}/src/main.lpp")))
            .unwrap_or_else(|| panic!("no invocation for {member}:\n{args}"))
    };
    assert!(line("d") < line("b"), "d must build before b:\n{args}");
    assert!(line("d") < line("c"), "d must build before c:\n{args}");
    assert!(line("b") < line("app"), "b must build before app:\n{args}");
    assert!(line("c") < line("app"), "c must build before app:\n{args}");
    // Multi-member workspaces share the root target/.
    assert!(
        args.contains(&format!("{}/target/app", root.display())),
        "shared root target/ expected:\n{args}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn independent_members_build_concurrently_within_a_layer() {
    let root = temp("parallel");
    diamond(&root);
    let fake = slow_fake_lpp(&root);

    let res = keel::commands::build::build(
        &root,
        fake.to_str().unwrap(),
        &|m, cwd, l, r| keel::commands::build::run_lpp_jobs(m, cwd, l, r),
        None,
    );
    assert!(res.is_ok(), "build should succeed: {res:?}");

    let marks = std::fs::read_to_string(root.join("marks.txt")).unwrap();
    let pos = |needle: &str| {
        marks
            .lines()
            .position(|l| l == needle)
            .unwrap_or_else(|| panic!("missing mark '{needle}':\n{marks}"))
    };
    let start_b = pos(&format!(
        "START {root}/crates/b/src/main.lpp",
        root = root.display()
    ));
    let end_b = pos(&format!(
        "END {root}/crates/b/src/main.lpp",
        root = root.display()
    ));
    let start_c = pos(&format!(
        "START {root}/crates/c/src/main.lpp",
        root = root.display()
    ));
    let end_c = pos(&format!(
        "END {root}/crates/c/src/main.lpp",
        root = root.display()
    ));
    // b and c are independent (same layer): their "compile" windows overlap.
    assert!(
        start_b.min(start_c) < end_b.min(end_c) && start_b.max(start_c) < end_b.min(end_c),
        "b and c must overlap in time (same layer):\n{marks}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fetch_all_writes_one_lock_for_the_whole_workspace() {
    let root = temp("lock");
    diamond(&root);
    let bare = seed_bare(&root);
    let reg = lpp_pm::Registry::new(
        bare.to_string_lossy().as_ref(),
        &root.join("registry-cache"),
    );

    let res = keel::commands::registry::fetch_all(&reg, &root.join("apps/app"));
    assert!(res.is_ok(), "fetch_all should succeed: {res:?}");

    // ONE lock at the workspace root (not in the sub-member dir).
    let lock_path = root.join("Keel.lock");
    assert!(
        lock_path.exists(),
        "Keel.lock must be at the workspace root"
    );
    assert!(!root.join("apps/app/Keel.lock").exists());
    let lock = lpp_pm::Lock::parse(&std::fs::read_to_string(&lock_path).unwrap()).unwrap();
    let names: Vec<&str> = lock.packages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["app", "b", "c", "d"], "all members in the lock");
    // Path members carry no checksum.
    assert!(lock.packages.iter().all(|p| p.checksum.is_none()));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn workspace_members_and_graph_are_deterministic() {
    let root = temp("inspection");
    diamond(&root);
    let workspace = keel::commands::workspace::discover(&root).unwrap();

    let members = keel::commands::workspace::render_members(&workspace);
    assert!(members.starts_with(&format!("Workspace: {}\n", root.display())));
    assert!(members.contains("  app @ 1.0.0 (apps/app)"), "{members}");
    assert!(members.contains("  b @ 1.0.0 (crates/b)"), "{members}");
    assert!(members.contains("  c @ 1.0.0 (crates/c)"), "{members}");
    assert!(members.contains("  d @ 1.0.0 (crates/d)"), "{members}");

    let graph = keel::commands::workspace::render_graph(&workspace);
    assert!(graph.contains("  app -> b, c"), "{graph}");
    assert!(graph.contains("  b -> d"), "{graph}");
    assert!(graph.contains("  c -> d"), "{graph}");
    assert!(graph.contains("  d -> (none)"), "{graph}");
    assert_eq!(graph, keel::commands::workspace::render_graph(&workspace));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_cyclic_workspace_fails_the_build_with_e6016() {
    let root = temp("cycle");
    std::fs::write(
        root.join("Keel.toml"),
        "[workspace]\nmembers = [\"a\", \"b\"]\n",
    )
    .unwrap();
    for (d, deps) in [
        ("a", "[dependencies]\nb = { path = \"../b\" }"),
        ("b", "[dependencies]\na = { path = \"../a\" }"),
    ] {
        let p = root.join(d);
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(p.join("Keel.toml"), pkg_manifest(d, deps)).unwrap();
        std::fs::write(p.join("src/main.lpp"), "fn main() {}\n").unwrap();
    }
    let fake = fake_lpp(&root);
    let res = keel::commands::build::build(
        &root,
        fake.to_str().unwrap(),
        &|m, cwd, l, r| keel::commands::build::run_lpp_jobs(m, cwd, l, r),
        None,
    );
    let err = res.expect_err("cycle must fail");
    assert!(err.contains("E6016"), "typed cycle error: {err}");
    let _ = std::fs::remove_dir_all(&root);
}
