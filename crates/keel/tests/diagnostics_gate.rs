//! Gate: `keel outdated` + `keel why` (contract: docs/rewrite/DIAGNOSTICS.md).

use std::path::Path;
use std::process::Command;

use lpp_pm::Registry;
use lpp_pm::index::{DepSpec, IndexEntry, VersionEntry};

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
    let d = std::env::temp_dir().join(format!("lpp-keel-dx-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn ve(version: &str, yanked: bool, deps: &[(&str, &str)]) -> VersionEntry {
    VersionEntry {
        version: version.into(),
        deps: deps
            .iter()
            .map(|(n, r)| DepSpec {
                name: n.to_string(),
                req: r.to_string(),
                optional: false,
                features: vec![],
            })
            .collect(),
        features: Default::default(),
        checksum: lpp_pm::ContentAddress::of_bytes(format!("artifact-{version}").as_bytes())
            .to_string(),
        targets: vec![],
        yanked,
    }
}

fn seeded_registry(root: &Path) -> std::path::PathBuf {
    let bare = root.join("remote.git");
    git(
        root,
        &[
            "init",
            "--bare",
            "--initial-branch=main",
            bare.to_str().unwrap(),
        ],
    );
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
    // Artifacts are "artifact-{version}" so every blob lands at
    // blob/<the checksum its index entry promises> (honest registry).
    reg.publish(
        &IndexEntry {
            name: "math".into(),
            versions: vec![ve("1.0.0", false, &[])],
        },
        b"artifact-1.0.0",
        "seed math",
    )
    .unwrap();
    reg.publish(
        &IndexEntry {
            name: "stats".into(),
            versions: vec![ve("0.1.0", false, &[("math", "^1")])],
        },
        b"artifact-0.1.0",
        "seed stats",
    )
    .unwrap();
    reg.push().unwrap();
    bare
}

/// Republish a full version list (upsert). The artifact is the newest
/// version's honest bytes; older versions' blobs persist from earlier
/// publishes.
fn republish(root: &Path, bare: &Path, entry: &IndexEntry) {
    let newest = entry
        .versions
        .iter()
        .max_by(|a, b| a.version.cmp(&b.version))
        .map(|v| v.version.clone())
        .expect("non-empty entry");
    let artifact = format!("artifact-{newest}").into_bytes();
    let reg = Registry::new(bare.to_string_lossy().as_ref(), root.join("republisher"));
    reg.sync().unwrap();
    git(&root.join("republisher"), &["config", "user.name", "R"]);
    git(
        &root.join("republisher"),
        &["config", "user.email", "r@example.com"],
    );
    reg.publish(entry, &artifact, "bump").unwrap();
    reg.push().unwrap();
}

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

/// `outdated` rows parsed from the rendered table (name → status).
fn outdated_status(out: &str, name: &str) -> Option<String> {
    for line in out.lines() {
        let cells: Vec<&str> = line
            .split('|')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if cells.len() >= 4 && cells[0] == name {
            return Some(cells[3].to_string());
        }
    }
    None
}

#[test]
fn outdated_reports_updates_and_yanks() {
    let root = temp("out");
    let bare = seeded_registry(&root);
    let proj = make_proj(&root);
    let reg = Registry::new(bare.to_string_lossy().as_ref(), root.join("clone"));
    keel::commands::registry::fetch_all(&reg, &proj).unwrap();
    let url = bare.to_string_lossy().to_string();

    // 1. Baseline: both up to date.
    let r = Registry::new(url.as_str(), root.join("c2"));
    let out = keel::commands::diagnostics::outdated_render(Some(&r), &proj, None).unwrap();
    assert_eq!(
        outdated_status(&out, "math"),
        Some("up to date".into()),
        "{out}"
    );
    assert_eq!(
        outdated_status(&out, "stats"),
        Some("up to date".into()),
        "{out}"
    );
    assert!(!out.contains("need attention"), "{out}");

    // 2. Publish math 1.2.0 → update available (latest 1.2.0).
    republish(
        &root,
        &bare,
        &IndexEntry {
            name: "math".into(),
            versions: vec![ve("1.0.0", false, &[]), ve("1.2.0", false, &[])],
        },
    );
    let r = Registry::new(url.as_str(), root.join("c3"));
    let out = keel::commands::diagnostics::outdated_render(Some(&r), &proj, None).unwrap();
    assert_eq!(
        outdated_status(&out, "math"),
        Some("update available".into()),
        "{out}"
    );
    assert!(out.contains("1.2.0"), "latest column missing:\n{out}");
    assert!(out.contains("1 package(s) need attention"), "{out}");

    // 3. Yank math 1.2.0 → back to up to date (1.0.0 is latest non-yanked).
    republish(
        &root,
        &bare,
        &IndexEntry {
            name: "math".into(),
            versions: vec![ve("1.0.0", false, &[]), ve("1.2.0", true, &[])],
        },
    );
    let r = Registry::new(url.as_str(), root.join("c4"));
    let out = keel::commands::diagnostics::outdated_render(Some(&r), &proj, None).unwrap();
    assert_eq!(
        outdated_status(&out, "math"),
        Some("up to date".into()),
        "{out}"
    );

    // 4. Yank the locked math 1.0.0 → yanked (filter works too).
    republish(
        &root,
        &bare,
        &IndexEntry {
            name: "math".into(),
            versions: vec![ve("1.0.0", true, &[]), ve("1.2.0", false, &[])],
        },
    );
    let r = Registry::new(url.as_str(), root.join("c5"));
    let out = keel::commands::diagnostics::outdated_render(Some(&r), &proj, Some("math")).unwrap();
    assert_eq!(
        outdated_status(&out, "math"),
        Some("yanked".into()),
        "{out}"
    );
    assert!(
        !out.lines().any(|l| l.contains("stats")),
        "filter leaked stats:\n{out}"
    );

    // 5. Filter for a name not in the lock → E6024.
    let r = Registry::new(url.as_str(), root.join("c6"));
    let err =
        keel::commands::diagnostics::outdated_render(Some(&r), &proj, Some("ghost")).unwrap_err();
    assert!(err.contains("E6024") && err.contains("ghost"), "{err}");

    // 6. No registry configured, lock has registry packages → clean error.
    let out = keel::commands::diagnostics::outdated_render(None, &proj, None);
    assert!(out.is_err(), "offline with registry deps must error");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn verify_passes_on_an_honest_registry() {
    let root = temp("verify-ok");
    let bare = seeded_registry(&root);
    let proj = make_proj(&root);
    let reg = Registry::new(bare.to_string_lossy().as_ref(), root.join("clone"));
    keel::commands::registry::fetch_all(&reg, &proj).unwrap();

    let r = Registry::new(bare.to_string_lossy().as_ref(), root.join("vclone"));
    keel::commands::diagnostics::verify(&r, &proj).unwrap(); // all ok → Ok(())
    let _ = std::fs::remove_dir_all(&root);
}

/// Raw-registry tamper: rewrite math's index entry to point at a bogus
/// checksum (no such blob). `keel verify` must report `missing` and fail.
#[test]
fn verify_catches_a_missing_blob() {
    let root = temp("verify-miss");
    let bare = seeded_registry(&root);
    let proj = make_proj(&root);
    let reg = Registry::new(bare.to_string_lossy().as_ref(), root.join("clone"));
    keel::commands::registry::fetch_all(&reg, &proj).unwrap();

    let attacker = Registry::new(bare.to_string_lossy().as_ref(), root.join("attacker"));
    attacker.sync().unwrap();
    git(&root.join("attacker"), &["config", "user.name", "A"]);
    git(
        &root.join("attacker"),
        &["config", "user.email", "a@example.com"],
    );
    // Forged checksum: no blob is (or ever was) stored under it.
    let forged = lpp_pm::ContentAddress::of_bytes(b"nonexistent-bytes").to_string();
    attacker
        .publish(
            &IndexEntry {
                name: "math".into(),
                versions: vec![VersionEntry {
                    version: "1.0.0".into(),
                    deps: vec![],
                    features: Default::default(),
                    checksum: forged,
                    targets: vec![],
                    yanked: false,
                }],
            },
            b"attacker artifact",
            "tamper: bogus checksum",
        )
        .unwrap();
    attacker.push().unwrap();

    let r = Registry::new(bare.to_string_lossy().as_ref(), root.join("vclone"));
    let err = keel::commands::diagnostics::verify(&r, &proj).unwrap_err();
    assert!(
        err.to_lowercase().contains("fail"),
        "verify must fail on a broken registry: {err}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Tamper where a blob EXISTS at the rewritten checksum but the bytes are
/// NOT what the lockfile promised → `mismatch` (the attack a plain fetch
/// would miss).
#[test]
fn verify_catches_a_tampered_checksum() {
    let root = temp("verify-tamper");
    let bare = seeded_registry(&root);
    let proj = make_proj(&root);
    let reg = Registry::new(bare.to_string_lossy().as_ref(), root.join("clone"));
    keel::commands::registry::fetch_all(&reg, &proj).unwrap();

    // Attacker publishes a NEW artifact under math 1.0.0 (raw registry
    // access — the keel merge guard is a client-side convenience, not the
    // trust boundary). The index now points math 1.0.0 at the attacker's
    // blob, which self-verifies against the (tampered) index.
    let attacker = Registry::new(bare.to_string_lossy().as_ref(), root.join("attacker"));
    attacker.sync().unwrap();
    git(&root.join("attacker"), &["config", "user.name", "A"]);
    git(
        &root.join("attacker"),
        &["config", "user.email", "a@example.com"],
    );
    let evil = b"attacker-controlled math bytes";
    let evil_sum = lpp_pm::ContentAddress::of_bytes(evil).to_string();
    attacker
        .publish(
            &IndexEntry {
                name: "math".into(),
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
            "tamper: replace math 1.0.0",
        )
        .unwrap();
    attacker.push().unwrap();

    let r = Registry::new(bare.to_string_lossy().as_ref(), root.join("vclone"));
    let err = keel::commands::diagnostics::verify(&r, &proj).unwrap_err();
    assert!(
        err.to_lowercase().contains("fail"),
        "verify must fail on a tampered checksum: {err}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn verify_without_lock_is_e6023() {
    let root = temp("verify-nolock");
    let bare = seeded_registry(&root);
    let proj = make_proj(&root); // no fetch → no Keel.lock
    let r = Registry::new(bare.to_string_lossy().as_ref(), root.join("clone"));
    let err = keel::commands::diagnostics::verify(&r, &proj).unwrap_err();
    assert!(err.contains("E6023"), "{err}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn outdated_without_lock_is_e6023() {
    let root = temp("nolock");
    let bare = seeded_registry(&root);
    let proj = make_proj(&root); // no fetch → no Keel.lock
    let r = Registry::new(bare.to_string_lossy().as_ref(), root.join("clone"));
    let err = keel::commands::diagnostics::outdated(Some(&r), &proj, None).unwrap_err();
    assert!(err.contains("E6023"), "expected E6023, got: {err}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn why_shows_the_dependency_chains() {
    let root = temp("why");
    let bare = seeded_registry(&root);
    let proj = make_proj(&root);
    let reg = Registry::new(bare.to_string_lossy().as_ref(), root.join("clone"));
    keel::commands::registry::fetch_all(&reg, &proj).unwrap();

    // Capture why's stdout via the binary (render path prints).
    let keel_bin = env!("CARGO_BIN_EXE_keel");
    let xdg = root.join("xdg");
    std::fs::create_dir_all(&xdg).unwrap();

    // math is a direct dep of app AND a dep of stats (transitive chain).
    let out = Command::new(keel_bin)
        .arg("why")
        .arg("math")
        .env("XDG_CACHE_HOME", &xdg)
        .env("KEEL_REGISTRY", bare.to_string_lossy().as_ref())
        .current_dir(&proj)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "why: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("why: math v1.0.0 (registry)"), "{stdout}");
    assert!(
        stdout.contains("math ← app (direct)"),
        "direct chain missing:\n{stdout}"
    );
    assert!(
        stdout.contains("math ← stats ← app"),
        "transitive chain missing:\n{stdout}"
    );

    // stats is only a direct dep.
    let out = Command::new(keel_bin)
        .arg("why")
        .arg("stats")
        .env("XDG_CACHE_HOME", &xdg)
        .env("KEEL_REGISTRY", bare.to_string_lossy().as_ref())
        .current_dir(&proj)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("stats ← app (direct)"), "{stdout}");

    // Unknown package → E6024.
    let out = Command::new(keel_bin)
        .arg("why")
        .arg("ghost")
        .env("XDG_CACHE_HOME", &xdg)
        .current_dir(&proj)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("E6024"), "{stderr}");

    let _ = std::fs::remove_dir_all(&root);
}
