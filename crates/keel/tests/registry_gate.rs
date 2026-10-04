//! Gate: the keel registry command wiring (fetch / search / publish + a live fetch).

use clap::Parser;
use std::path::Path;
use std::process::Command;

use keel::cli::{Cli, Command as CliCommand};

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

fn temp(tag: &str) -> std::path::PathBuf {
    let d =
        std::env::temp_dir().join(format!("lpp-keel-reg-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Seed a bare git registry with one package (`math` 1.0.0); return its path.
fn seeded_registry(root: &Path) -> std::path::PathBuf {
    let bare = root.join("remote.git");
    git(root, &["init", "--bare", "--initial-branch=main", bare.to_str().unwrap()]);
    let work = root.join("work");
    git(root, &["init", "-b", "main", work.to_str().unwrap()]);
    git(&work, &["config", "user.name", "Seeder"]);
    git(&work, &["config", "user.email", "s@example.com"]);
    git(&work, &["commit", "--allow-empty", "-m", "init"]);
    git(&work, &["push", bare.to_str().unwrap(), "HEAD:main"]);

    // Publish `math` 1.0.0 through the Registry, then push.
    let pub_dir = root.join("publisher");
    let reg = lpp_pm::Registry::new(bare.to_string_lossy().as_ref(), &pub_dir);
    reg.sync().unwrap();
    git(&pub_dir, &["config", "user.name", "P"]);
    git(&pub_dir, &["config", "user.email", "p@example.com"]);
    let artifact = b"math package bytes";
    let checksum = lpp_pm::ContentAddress::of_bytes(artifact).to_string();
    let entry = lpp_pm::index::IndexEntry {
        name: "math".into(),
        versions: vec![lpp_pm::index::VersionEntry {
            version: "1.0.0".into(),
            deps: vec![],
            features: Default::default(),
            checksum,
            targets: vec![],
            yanked: false,
        }],
    };
    reg.publish(&entry, artifact, "seed math 1.0.0").unwrap();
    reg.push().unwrap();
    bare
}

#[test]
fn fetch_a_package_from_a_git_registry() {
    let root = temp("fetch");
    let bare = seeded_registry(&root);
    let reg = lpp_pm::Registry::new(bare.to_string_lossy().as_ref(), root.join("keel-clone"));
    let res = keel::commands::registry::fetch(&reg, "math");
    assert!(res.is_ok(), "fetch should succeed: {res:?}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fetch_a_missing_package_errors() {
    let root = temp("miss");
    let bare = seeded_registry(&root);
    let reg = lpp_pm::Registry::new(bare.to_string_lossy().as_ref(), root.join("keel-clone"));
    let res = keel::commands::registry::fetch(&reg, "doesnotexist");
    assert!(res.is_err(), "fetch of a missing package should error: {res:?}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn parses_registry_commands() {
    let cli =
        Cli::try_parse_from(["keel", "fetch", "math@1.0.0", "--registry", "https://x/y.git"])
            .unwrap();
    assert_eq!(cli.registry.as_deref(), Some("https://x/y.git"));
    match cli.command {
        CliCommand::Fetch { name } => assert_eq!(name.as_deref(), Some("math@1.0.0")),
        _ => panic!("expected fetch"),
    }
    assert!(matches!(
        Cli::try_parse_from(["keel", "search", "linear"]).unwrap().command,
        CliCommand::Search { .. }
    ));
    assert!(matches!(
        Cli::try_parse_from(["keel", "publish"]).unwrap().command,
        CliCommand::Publish
    ));
}

#[test]
fn fetch_all_resolves_the_graph_and_writes_lock() {
    let root = temp("fetchall");
    let bare = seeded_registry(&root); // publishes "math" 1.0.0
    let reg = lpp_pm::Registry::new(bare.to_string_lossy().as_ref(), root.join("keel-clone"));
    // a project that depends on math ^1.0
    let proj = root.join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(
        proj.join("Keel.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nmath = \"^1.0\"\n",
    )
    .unwrap();
    let res = keel::commands::registry::fetch_all(&reg, &proj);
    assert!(res.is_ok(), "fetch_all should succeed: {res:?}");
    let lock = std::fs::read_to_string(proj.join("Keel.lock")).unwrap();
    assert!(lock.contains("math"), "lock should contain math:\n{lock}");
    assert!(lock.contains("1.0.0"), "lock should contain math's version:\n{lock}");
    let _ = std::fs::remove_dir_all(&root);
}

fn ventry(version: &str, checksum: &str) -> lpp_pm::index::VersionEntry {
    lpp_pm::index::VersionEntry {
        version: version.into(),
        deps: vec![],
        features: Default::default(),
        checksum: checksum.into(),
        targets: vec![],
        yanked: false,
    }
}

#[test]
fn publish_merges_versions_instead_of_clobbering() {
    // First publish: no existing entry.
    let e1 = keel::commands::registry::merge_publish(
        None,
        "math",
        ventry("1.0.0", "aaa"),
    )
    .unwrap();
    assert_eq!(e1.versions.len(), 1);

    // Second publish (newer version): appends, does not drop 1.0.0.
    let e2 = keel::commands::registry::merge_publish(
        Some(&e1),
        "math",
        ventry("1.1.0", "bbb"),
    )
    .unwrap();
    let vers: Vec<&str> = e2.versions.iter().map(|v| v.version.as_str()).collect();
    assert_eq!(vers, vec!["1.0.0", "1.1.0"], "must keep old version: {vers:?}");
}

#[test]
fn publish_rejects_republish_of_same_version() {
    let e1 = keel::commands::registry::merge_publish(None, "math", ventry("1.0.0", "aaa")).unwrap();

    // Same version, different checksum → immutability conflict.
    let err = keel::commands::registry::merge_publish(Some(&e1), "math", ventry("1.0.0", "ccc"))
        .unwrap_err();
    assert!(matches!(
        err,
        lpp_pm::PmError::PublishConflict { same: false, .. }
    ));
    assert!(err.to_string().contains("E6025"), "{err}");

    // Same version, same checksum → identical republish (no-op, not an
    // immutability violation).
    let err = keel::commands::registry::merge_publish(Some(&e1), "math", ventry("1.0.0", "aaa"))
        .unwrap_err();
    assert!(matches!(
        err,
        lpp_pm::PmError::PublishConflict { same: true, .. }
    ));
}

#[test]
fn publish_command_appends_versions_end_to_end() {
    let root = temp("pube2e");
    // Empty bare registry.
    let bare = root.join("remote.git");
    git(&root, &["init", "--bare", "--initial-branch=main", bare.to_str().unwrap()]);
    let work = root.join("work");
    git(&root, &["init", "-b", "main", work.to_str().unwrap()]);
    git(&work, &["config", "user.name", "S"]);
    git(&work, &["config", "user.email", "s@example.com"]);
    git(&work, &["commit", "--allow-empty", "-m", "init"]);
    git(&work, &["push", bare.to_str().unwrap(), "HEAD:main"]);

    let xdg = root.join("xdg");
    std::fs::create_dir_all(&xdg).unwrap();
    let keel_bin = env!("CARGO_BIN_EXE_keel");

    let pub_pkg = |dir_name: &str, version: &str| {
        let dir = root.join(dir_name);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Keel.toml"),
            format!("[package]\nname = \"mathx\"\nversion = \"{version}\"\n"),
        )
        .unwrap();
        std::fs::write(dir.join("src/lib.lpp"), "def answer():\n    return 42\n").unwrap();
        dir
    };
    let publish = |dir: &std::path::PathBuf| {
        Command::new(keel_bin)
            .arg("publish")
            .env("XDG_CACHE_HOME", &xdg)
            .env("KEEL_REGISTRY", bare.to_string_lossy().as_ref())
            .current_dir(dir)
            .output()
            .unwrap()
    };

    // 1.0.0, then 1.1.0.
    let p1 = pub_pkg("pub1", "1.0.0");
    let out = publish(&p1);
    assert!(out.status.success(), "publish 1.0.0: {}", String::from_utf8_lossy(&out.stderr));
    let p2 = pub_pkg("pub2", "1.1.0");
    let out = publish(&p2);
    assert!(out.status.success(), "publish 1.1.0: {}", String::from_utf8_lossy(&out.stderr));

    // The registry index now carries BOTH versions (no clobber).
    let clone = root.join("verify");
    git(&root, &["clone", "--quiet", bare.to_str().unwrap(), clone.to_str().unwrap()]);
    let reg = lpp_pm::Registry::new(bare.to_string_lossy().as_ref(), &clone);
    let entry = reg.lookup("mathx").unwrap();
    let vers: Vec<&str> = entry.versions.iter().map(|v| v.version.as_str()).collect();
    assert_eq!(vers, vec!["1.0.0", "1.1.0"], "versions must accumulate: {vers:?}");

    // Republishing 1.1.0 is rejected (E6025 — immutable version).
    let out = publish(&p2);
    assert!(!out.status.success(), "republish must fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("E6025"), "expected E6025, got: {stderr}");

    let _ = std::fs::remove_dir_all(&root);
}
