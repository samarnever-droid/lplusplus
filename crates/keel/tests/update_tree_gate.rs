//! Gate: `keel update` + `keel tree` (contract: docs/rewrite/UPDATE_TREE.md).

use std::path::Path;
use std::process::Command;

use lpp_pm::index::{IndexEntry, VersionEntry};
use lpp_pm::Registry;

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
    let d = std::env::temp_dir().join(format!("lpp-keel-ut-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn ve(version: &str, yanked: bool, deps: &[(&str, &str)]) -> VersionEntry {
    let artifact = format!("artifact-{version}");
    VersionEntry {
        version: version.into(),
        deps: deps
            .iter()
            .map(|(n, r)| lpp_pm::index::DepSpec {
                name: n.to_string(),
                req: r.to_string(),
                optional: false,
                features: vec![],
            })
            .collect(),
        features: Default::default(),
        checksum: lpp_pm::ContentAddress::of_bytes(artifact.as_bytes()).to_string(),
        targets: vec![],
        yanked,
    }
}

/// Seed a bare git registry with `math` + `stats`; return the bare path.
fn seeded_registry(root: &Path) -> std::path::PathBuf {
    let bare = root.join("remote.git");
    git(root, &["init", "--bare", "--initial-branch=main", bare.to_str().unwrap()]);
    let work = root.join("work");
    git(root, &["init", "-b", "main", work.to_str().unwrap()]);
    git(&work, &["config", "user.name", "Seeder"]);
    git(&work, &["config", "user.email", "s@example.com"]);
    git(&work, &["commit", "--allow-empty", "-m", "init"]);
    git(&work, &["push", bare.to_str().unwrap(), "HEAD:main"]);

    let pub_dir = root.join("publisher");
    let reg = Registry::new(bare.to_string_lossy().as_ref(), &pub_dir);
    reg.sync().unwrap();
    git(&pub_dir, &["config", "user.name", "P"]);
    git(&pub_dir, &["config", "user.email", "p@example.com"]);
    // math 1.0.0 (no deps); stats 0.1.0 (deps: math ^1)
    reg.publish(
        &IndexEntry {
            name: "math".into(),
            versions: vec![ve("1.0.0", false, &[])],
        },
        b"math 1.0.0",
        "seed math",
    )
    .unwrap();
    reg.publish(
        &IndexEntry {
            name: "stats".into(),
            versions: vec![ve("0.1.0", false, &[("math", "^1")])],
        },
        b"stats 0.1.0",
        "seed stats",
    )
    .unwrap();
    reg.push().unwrap();
    bare
}

/// Republish one package's full version list (upsert) and push.
fn republish(root: &Path, bare: &Path, entry: &IndexEntry) {
    let reg = Registry::new(bare.to_string_lossy().as_ref(), root.join("republisher"));
    reg.sync().unwrap();
    git(&root.join("republisher"), &["config", "user.name", "R"]);
    git(&root.join("republisher"), &["config", "user.email", "r@example.com"]);
    reg.publish(entry, b"artifact", "bump").unwrap();
    reg.push().unwrap();
}

/// A project depending on `math ^1` and `stats ~0.1`.
fn make_proj(root: &Path) -> std::path::PathBuf {
    let proj = root.join("proj");
    std::fs::create_dir_all(proj.join("src")).unwrap();
    std::fs::write(
        proj.join("Keel.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nmath = \"^1\"\nstats = \">=0.1\"\n",
    )
    .unwrap();
    std::fs::write(proj.join("src/main.lpp"), "def main():\n    print(1)\n").unwrap();
    proj
}

fn lock_map(proj: &Path) -> std::collections::BTreeMap<String, String> {
    let lock = lpp_pm::Lock::parse(&std::fs::read_to_string(proj.join("Keel.lock")).unwrap())
        .unwrap();
    lock.packages
        .into_iter()
        .map(|p| (p.name, p.version))
        .collect()
}

#[test]
fn update_all_bumps_everything_and_is_idempotent() {
    let root = temp("all");
    let bare = seeded_registry(&root);
    let proj = make_proj(&root);
    let reg = Registry::new(bare.to_string_lossy().as_ref(), root.join("keel-clone"));

    // Baseline: fetch resolves to the only versions.
    keel::commands::registry::fetch_all(&reg, &proj).unwrap();
    let m = lock_map(&proj);
    assert_eq!(m.get("math").map(String::as_str), Some("1.0.0"));
    assert_eq!(m.get("stats").map(String::as_str), Some("0.1.0"));

    // Publish newer versions.
    republish(
        &root,
        &bare,
        &IndexEntry {
            name: "math".into(),
            versions: vec![ve("1.0.0", false, &[]), ve("1.1.0", false, &[])],
        },
    );
    republish(
        &root,
        &bare,
        &IndexEntry {
            name: "stats".into(),
            versions: vec![
                ve("0.1.0", false, &[("math", "^1")]),
                ve("0.2.0", false, &[("math", "^1")]),
            ],
        },
    );

    // `keel update` → both bump.
    let reg2 = Registry::new(bare.to_string_lossy().as_ref(), root.join("keel-clone"));
    keel::commands::update::update(Some(&reg2), &proj, None).unwrap();
    let m = lock_map(&proj);
    assert_eq!(m.get("math").map(String::as_str), Some("1.1.0"));
    assert_eq!(m.get("stats").map(String::as_str), Some("0.2.0"));

    // Second update → no-op (lock unchanged).
    let before = std::fs::read_to_string(proj.join("Keel.lock")).unwrap();
    keel::commands::update::update(Some(&reg2), &proj, None).unwrap();
    let after = std::fs::read_to_string(proj.join("Keel.lock")).unwrap();
    assert_eq!(before, after, "idempotent update must not rewrite the lock");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn update_single_pins_everything_else() {
    let root = temp("single");
    let bare = seeded_registry(&root);
    let proj = make_proj(&root);
    let reg = Registry::new(bare.to_string_lossy().as_ref(), root.join("keel-clone"));
    keel::commands::registry::fetch_all(&reg, &proj).unwrap();

    // Newer versions of BOTH.
    republish(
        &root,
        &bare,
        &IndexEntry {
            name: "math".into(),
            versions: vec![ve("1.0.0", false, &[]), ve("1.2.0", false, &[])],
        },
    );
    republish(
        &root,
        &bare,
        &IndexEntry {
            name: "stats".into(),
            versions: vec![
                ve("0.1.0", false, &[("math", "^1")]),
                ve("0.3.0", false, &[("math", "^1")]),
            ],
        },
    );

    // `keel update math` → math bumps, stats stays pinned.
    let reg2 = Registry::new(bare.to_string_lossy().as_ref(), root.join("keel-clone"));
    keel::commands::update::update(Some(&reg2), &proj, Some("math")).unwrap();
    let m = lock_map(&proj);
    assert_eq!(m.get("math").map(String::as_str), Some("1.2.0"));
    assert_eq!(
        m.get("stats").map(String::as_str),
        Some("0.1.0"),
        "stats must stay pinned at its locked version"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn update_never_picks_yanked_versions() {
    let root = temp("yanked");
    let bare = seeded_registry(&root);
    let proj = make_proj(&root);
    let reg = Registry::new(bare.to_string_lossy().as_ref(), root.join("keel-clone"));
    keel::commands::registry::fetch_all(&reg, &proj).unwrap();

    // Add math 1.1.0, then yank it; update must land on 1.0.0 (fresh clone
    // each time, so the lock's stale 1.1.0 pick never lingers).
    republish(
        &root,
        &bare,
        &IndexEntry {
            name: "math".into(),
            versions: vec![ve("1.0.0", false, &[]), ve("1.1.0", true, &[])],
        },
    );
    let reg2 = Registry::new(bare.to_string_lossy().as_ref(), root.join("clone2"));
    keel::commands::update::update(Some(&reg2), &proj, None).unwrap();
    let m = lock_map(&proj);
    assert_eq!(
        m.get("math").map(String::as_str),
        Some("1.0.0"),
        "yanked 1.1.0 must not be offered"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn update_single_with_gone_pinned_version_is_e6022() {
    let root = temp("gone");
    let bare = seeded_registry(&root);
    let proj = make_proj(&root);
    let reg = Registry::new(bare.to_string_lossy().as_ref(), root.join("keel-clone"));
    keel::commands::registry::fetch_all(&reg, &proj).unwrap();

    // Yank the locked math 1.0.0, publish stats 0.2.0, then single-update
    // stats: the pinned math is gone → E6022.
    republish(
        &root,
        &bare,
        &IndexEntry {
            name: "math".into(),
            versions: vec![ve("1.0.0", true, &[])],
        },
    );
    republish(
        &root,
        &bare,
        &IndexEntry {
            name: "stats".into(),
            versions: vec![
                ve("0.1.0", false, &[("math", "^1")]),
                ve("0.2.0", false, &[("math", "^1")]),
            ],
        },
    );
    let reg2 = Registry::new(bare.to_string_lossy().as_ref(), root.join("clone2"));
    let err = keel::commands::update::update(Some(&reg2), &proj, Some("stats"))
        .unwrap_err();
    assert!(
        err.contains("E6022") && err.contains("math") && err.contains("1.0.0"),
        "expected E6022 for the gone pinned math, got: {err}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn update_offline_is_an_idempotent_noop() {
    let root = temp("offline");
    let bare = seeded_registry(&root);
    let proj = make_proj(&root);
    let reg = Registry::new(bare.to_string_lossy().as_ref(), root.join("keel-clone"));
    keel::commands::registry::fetch_all(&reg, &proj).unwrap();
    let before = std::fs::read_to_string(proj.join("Keel.lock")).unwrap();

    // No registry configured → locked versions offered back as-is.
    keel::commands::update::update(None, &proj, None).unwrap();
    let after = std::fs::read_to_string(proj.join("Keel.lock")).unwrap();
    assert_eq!(before, after, "offline update must be a no-op");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn tree_renders_the_locked_graph() {
    let root = temp("tree");
    let bare = seeded_registry(&root);
    let proj = make_proj(&root);
    let reg = Registry::new(bare.to_string_lossy().as_ref(), root.join("keel-clone"));
    keel::commands::registry::fetch_all(&reg, &proj).unwrap();

    let out = keel::commands::tree::render(&proj, None).unwrap();
    assert!(out.contains("app v0.1.0"), "member header missing:\n{out}");
    assert!(
        out.contains("math v1.0.0 (registry)"),
        "math node missing:\n{out}"
    );
    assert!(
        out.contains("stats v0.1.0 (registry)"),
        "stats node missing:\n{out}"
    );
    // stats depends on math: the transitive edge appears under stats.
    let stats_pos = out.find("stats v0.1.0").unwrap();
    let tail = &out[stats_pos..];
    assert!(
        tail.contains("math v1.0.0 (registry)"),
        "transitive math edge missing under stats:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn tree_without_lock_marks_registry_deps_unlocked() {
    let root = temp("unlocked");
    let _bare = seeded_registry(&root);
    let proj = make_proj(&root);

    let out = keel::commands::tree::render(&proj, None).unwrap();
    assert!(
        out.contains("math v? (^1) (unlocked)"),
        "unlocked math node missing:\n{out}"
    );
    assert!(
        out.contains("run `keel fetch` to build Keel.lock"),
        "unlock hint missing:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// CLI wiring: `keel update` + `keel tree` through the real binary, with an
/// isolated XDG cache (parallel-safe) and the registry via env.
#[test]
fn binary_wiring_update_and_tree() {
    let root = temp("bin");
    let bare = seeded_registry(&root);
    let proj = make_proj(&root);

    let xdg = root.join("xdg");
    std::fs::create_dir_all(&xdg).unwrap();
    let keel_bin = env!("CARGO_BIN_EXE_keel");

    // fetch first (builds Keel.lock at 1.0.0), THEN publish the bump.
    let out = Command::new(keel_bin)
        .arg("fetch")
        .env("XDG_CACHE_HOME", &xdg)
        .env("KEEL_REGISTRY", bare.to_string_lossy().as_ref())
        .current_dir(&proj)
        .output()
        .unwrap();
    assert!(out.status.success(), "fetch: {}", String::from_utf8_lossy(&out.stderr));

    republish(
        &root,
        &bare,
        &IndexEntry {
            name: "math".into(),
            versions: vec![ve("1.0.0", false, &[]), ve("1.5.0", false, &[])],
        },
    );

    // update → diff table on stdout
    let out = Command::new(keel_bin)
        .arg("update")
        .env("XDG_CACHE_HOME", &xdg)
        .env("KEEL_REGISTRY", bare.to_string_lossy().as_ref())
        .current_dir(&proj)
        .output()
        .unwrap();
    assert!(out.status.success(), "update: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("updated Keel.lock"), "diff table missing:\n{stdout}");
    assert!(stdout.contains("1.0.0") && stdout.contains("1.5.0"), "bump row missing:\n{stdout}");

    // tree → rendered graph on stdout
    let out = Command::new(keel_bin)
        .arg("tree")
        .env("XDG_CACHE_HOME", &xdg)
        .env("KEEL_REGISTRY", bare.to_string_lossy().as_ref())
        .current_dir(&proj)
        .output()
        .unwrap();
    assert!(out.status.success(), "tree: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("app v0.1.0"), "tree header missing:\n{stdout}");
    assert!(
        stdout.contains("math v1.5.0 (registry)"),
        "tree must show the updated version:\n{stdout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
