//! Gate: slice 11 — every compile command (build/check/run/test) stages
//! path + registry deps; registry artifacts are verified against the lock.
//! Contract: DEP_LINKING.md (slice 11 section).

use std::path::Path;
use std::process::Command;

use lpp_pm::index::{IndexEntry, VersionEntry};
use lpp_pm::Registry;

#[cfg(unix)]
fn make_executable(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(not(unix))]
fn make_executable(_p: &Path) {}

fn temp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("lpp-keel-stage-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A fake `lpp` that records its cwd + args for every invocation.
fn fake_lpp(root: &Path) -> std::path::PathBuf {
    let p = root.join("lpp-fake");
    std::fs::write(
        &p,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$(pwd)|$@\" >> {}\n",
            root.join("calls.txt").display()
        ),
    )
    .unwrap();
    make_executable(&p);
    p
}

fn git(cwd: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git must be available on PATH");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Empty bare registry (no packages yet).
fn empty_registry(root: &Path) -> std::path::PathBuf {
    let bare = root.join("remote.git");
    git(root, &["init", "--bare", "--initial-branch=main", bare.to_str().unwrap()]);
    let work = root.join("work");
    git(root, &["init", "-b", "main", work.to_str().unwrap()]);
    git(&work, &["config", "user.name", "S"]);
    git(&work, &["config", "user.email", "s@example.com"]);
    git(&work, &["commit", "--allow-empty", "-m", "init"]);
    git(&work, &["push", bare.to_str().unwrap(), "HEAD:main"]);
    bare
}

/// Publish a real package artifact (tar.gz of a Keel.toml + src/lib.lpp)
/// with a consistent checksum.
fn publish_pkg(root: &Path, bare: &Path, name: &str, version: &str, body: &str) {
    let pkgdir = root.join(format!("pkg-{name}-{version}"));
    std::fs::create_dir_all(pkgdir.join("src")).unwrap();
    std::fs::write(
        pkgdir.join("Keel.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"{version}\"\n"),
    )
    .unwrap();
    std::fs::write(pkgdir.join("src/lib.lpp"), body).unwrap();
    let tar_path = root.join(format!("{name}-{version}.tar.gz"));
    let out = Command::new("tar")
        .args(["-czf", tar_path.to_str().unwrap(), "-C", pkgdir.to_str().unwrap(), "."])
        .output()
        .unwrap();
    assert!(out.status.success(), "tar: {}", String::from_utf8_lossy(&out.stderr));
    let bytes = std::fs::read(&tar_path).unwrap();
    let checksum = lpp_pm::ContentAddress::of_bytes(&bytes).to_string();

    let reg = Registry::new(bare.to_string_lossy().as_ref(), root.join(format!("pub-{name}")));
    reg.sync().unwrap();
    git(&root.join(format!("pub-{name}")), &["config", "user.name", "P"]);
    git(&root.join(format!("pub-{name}")), &["config", "user.email", "p@example.com"]);
    reg.publish(
        &IndexEntry {
            name: name.into(),
            versions: vec![VersionEntry {
                version: version.into(),
                deps: vec![],
                features: Default::default(),
                checksum,
                targets: vec![],
                yanked: false,
            }],
        },
        &bytes,
        "publish",
    )
    .unwrap();
    reg.push().unwrap();
}

/// A monorepo: member `a` depends on path member `b`.
fn monorepo_path(root: &Path) -> std::path::PathBuf {
    std::fs::write(
        root.join("Keel.toml"),
        "[workspace]\nmembers = [\"crates/*\"]\n",
    )
    .unwrap();
    for (name, deps) in [
        (
            "a",
            "[dependencies]\nb = { path = \"../b\" }\n",
        ),
        ("b", ""),
    ] {
        let d = root.join("crates").join(name);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(
            d.join("Keel.toml"),
            format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n\n{deps}"),
        )
        .unwrap();
        std::fs::write(
            d.join("src/main.lpp"),
            "def main():\n    print(1)\n",
        )
        .unwrap();
    }
    root.to_path_buf()
}

#[test]
fn run_stages_path_deps_without_a_prior_build() {
    let root = temp("run-stage");
    monorepo_path(&root);
    let fake = fake_lpp(&root);
    assert!(!root.join(".lpp_packages").exists(), "no stale staging");

    // Run member `a` (which depends on `b`) — from a FRESH workspace, no
    // prior `keel build`.
    keel::commands::build::run(&root.join("crates/a"), fake.to_str().unwrap(), None).unwrap();

    assert!(
        root.join(".lpp_packages/b/lpp.toml").is_file(),
        "b must be staged for `keel run`"
    );
    assert!(
        root.join(".lpp_packages/b/src/main.lpp").exists(),
        "staged src must resolve"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn check_stages_path_deps() {
    let root = temp("check-stage");
    monorepo_path(&root);
    let fake = fake_lpp(&root);

    keel::commands::build::check(&root.join("crates/a"), fake.to_str().unwrap(), None).unwrap();

    assert!(
        root.join(".lpp_packages/b/lpp.toml").is_file(),
        "b must be staged for `keel check`"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn registry_dep_is_staged_and_verifies_against_the_lock() {
    let root = temp("reg-stage");
    let bare = empty_registry(&root);
    publish_pkg(&root, &bare, "mathx", "1.0.0", "def answer():\n    return 42\n");

    // app depends on mathx (registry dep); fetch locks it.
    let app = root.join("app");
    std::fs::create_dir_all(app.join("src")).unwrap();
    std::fs::write(
        app.join("Keel.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nmathx = \"^1\"\n",
    )
    .unwrap();
    std::fs::write(app.join("src/main.lpp"), "def main():\n    print(mathx.answer())\n").unwrap();

    let reg = Registry::new(bare.to_string_lossy().as_ref(), root.join("clone"));
    keel::commands::registry::fetch_all(&reg, &app).unwrap();
    let lock = std::fs::read_to_string(app.join("Keel.lock")).unwrap();
    assert!(lock.contains("mathx"), "{lock}");

    let fake = fake_lpp(&root);
    let reg2 = Registry::new(bare.to_string_lossy().as_ref(), root.join("c2"));
    keel::commands::build::run(&app, fake.to_str().unwrap(), Some(&reg2))
        .expect("run must succeed and stage the registry dep");

    // Staged at the workspace root (= `app`, a standalone project).
    let staged_src = app.join(".lpp_packages/mathx/src/lib.lpp");
    assert!(staged_src.is_file(), "registry dep must be staged");
    let body = std::fs::read_to_string(&staged_src).unwrap();
    assert!(body.contains("return 42"), "staged source must be the real artifact: {body}");
    let doc = std::fs::read_to_string(app.join(".lpp_packages/mathx/lpp.toml")).unwrap();
    assert!(doc.contains("managed = \"keel\""), "{doc}");
    assert!(doc.contains("source = \"registry\""), "{doc}");
    assert!(doc.contains("checksum ="), "{doc}");

    // Idempotent: a second run is fine (no re-extraction needed).
    let reg3 = Registry::new(bare.to_string_lossy().as_ref(), root.join("c3"));
    keel::commands::build::run(&app, fake.to_str().unwrap(), Some(&reg3)).unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn tampered_registry_is_refused_at_build_time() {
    let root = temp("reg-tamper");
    let bare = empty_registry(&root);
    publish_pkg(&root, &bare, "mathx", "1.0.0", "def answer():\n    return 42\n");

    let app = root.join("app");
    std::fs::create_dir_all(app.join("src")).unwrap();
    std::fs::write(
        app.join("Keel.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nmathx = \"^1\"\n",
    )
    .unwrap();
    std::fs::write(app.join("src/main.lpp"), "def main():\n    print(mathx.answer())\n").unwrap();

    let reg = Registry::new(bare.to_string_lossy().as_ref(), root.join("clone"));
    keel::commands::registry::fetch_all(&reg, &app).unwrap();

    // Attacker (raw registry access) replaces mathx 1.0.0 with evil bytes.
    let evil = b"def answer():\n    return 999\n";
    let evil_sum = lpp_pm::ContentAddress::of_bytes(evil).to_string();
    let attacker = Registry::new(bare.to_string_lossy().as_ref(), root.join("attacker"));
    attacker.sync().unwrap();
    git(&root.join("attacker"), &["config", "user.name", "A"]);
    git(&root.join("attacker"), &["config", "user.email", "a@example.com"]);
    attacker
        .publish(
            &IndexEntry {
                name: "mathx".into(),
                versions: vec![VersionEntry {
                    version: "1.0.0".into(),
                    deps: vec![],
                    features: Default::default(),
                    checksum: evil_sum,
                    targets: vec![],
                    yanked: false,
                }],
            },
            evil,
            "tamper",
        )
        .unwrap();
    attacker.push().unwrap();

    // Force re-staging (the staging manifest would otherwise be current).
    let _ = std::fs::remove_dir_all(app.join(".lpp_packages"));
    let fake = fake_lpp(&root);
    let reg2 = Registry::new(bare.to_string_lossy().as_ref(), root.join("c2"));
    let err = keel::commands::build::check(&app, fake.to_str().unwrap(), Some(&reg2))
        .expect_err("tampered registry must be refused");
    assert!(
        err.contains("E6009"),
        "expected a checksum-mismatch refusal, got: {err}"
    );
    // The evil bytes must NOT have been staged.
    assert!(
        !app.join(".lpp_packages/mathx/src/lib.lpp").exists(),
        "tampered artifact must not be staged"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn registry_dep_without_a_registry_is_a_clean_error() {
    let root = temp("reg-noreg");
    let bare = empty_registry(&root);
    publish_pkg(&root, &bare, "mathx", "1.0.0", "def answer():\n    return 42\n");

    let app = root.join("app");
    std::fs::create_dir_all(app.join("src")).unwrap();
    std::fs::write(
        app.join("Keel.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nmathx = \"^1\"\n",
    )
    .unwrap();
    std::fs::write(app.join("src/main.lpp"), "def main():\n    print(1)\n").unwrap();

    let reg = Registry::new(bare.to_string_lossy().as_ref(), root.join("clone"));
    keel::commands::registry::fetch_all(&reg, &app).unwrap();

    let fake = fake_lpp(&root);
    let err = keel::commands::build::check(&app, fake.to_str().unwrap(), None).unwrap_err();
    assert!(
        err.contains("no registry configured"),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn registry_dep_without_a_lock_is_a_clean_error() {
    let root = temp("reg-nolock");
    let bare = empty_registry(&root);
    publish_pkg(&root, &bare, "mathx", "1.0.0", "def answer():\n    return 42\n");

    let app = root.join("app");
    std::fs::create_dir_all(app.join("src")).unwrap();
    std::fs::write(
        app.join("Keel.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nmathx = \"^1\"\n",
    )
    .unwrap();
    std::fs::write(app.join("src/main.lpp"), "def main():\n    print(1)\n").unwrap();
    // No `keel fetch` → no Keel.lock.

    let fake = fake_lpp(&root);
    let reg = Registry::new(bare.to_string_lossy().as_ref(), root.join("clone"));
    let err = keel::commands::build::check(&app, fake.to_str().unwrap(), Some(&reg)).unwrap_err();
    assert!(err.contains("keel fetch"), "{err}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_run_stages_deps_and_runs_from_the_workspace_root() {
    let root = temp("test-cwd");
    monorepo_path(&root);
    // Add a test file to member `a`.
    let tests = root.join("crates/a/tests");
    std::fs::create_dir_all(&tests).unwrap();
    std::fs::write(tests.join("t1.lpp"), "def main():\n    print(1)\n").unwrap();

    let fake = fake_lpp(&root);
    keel::commands::test::test_run(&root, fake.to_str().unwrap(), None).unwrap();

    // The test ran with cwd = the workspace root (so .lpp_packages and the
    // shared target/ resolve), and `b` was staged.
    let calls = std::fs::read_to_string(root.join("calls.txt")).unwrap();
    let line = calls.lines().find(|l| l.contains("t1.lpp")).unwrap_or("");
    let cwd = line.split('|').next().unwrap_or("");
    assert_eq!(
        cwd,
        root.canonicalize().unwrap().to_string_lossy().as_ref(),
        "test must run from the workspace root: {line}"
    );
    assert!(root.join(".lpp_packages/b/lpp.toml").is_file(), "deps must be staged");
    let _ = std::fs::remove_dir_all(&root);
}
