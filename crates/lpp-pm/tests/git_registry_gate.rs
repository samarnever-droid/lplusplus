//! Gate: the git-backed registry round-trip (clone → fetch → verify → publish → push).

use std::path::Path;
use std::process::Command;

use lpp_pm::index::{IndexEntry, VersionEntry};
use lpp_pm::registry::Registry;

/// Run `git <args>` in `cwd`, panicking on failure (test helper).
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

/// A fresh, unique temp dir for a test.
fn temp(tag: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("lpp-git-registry-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Seed a bare git repo (a "remote") with an empty initial commit on `main`.
fn seed_bare(dir: &Path) -> std::path::PathBuf {
    let bare = dir.join("remote.git");
    git(dir, &["init", "--bare", "--initial-branch=main", bare.to_str().unwrap()]);
    let work = dir.join("seed");
    git(dir, &["init", "-b", "main", work.to_str().unwrap()]);
    git(&work, &["config", "user.name", "Seeder"]);
    git(&work, &["config", "user.email", "seeder@example.com"]);
    git(&work, &["commit", "--allow-empty", "-m", "init"]);
    git(&work, &["push", bare.to_str().unwrap(), "HEAD:main"]);
    let _ = std::fs::remove_dir_all(&work);
    bare
}

fn entry(name: &str, version: &str, checksum: &str) -> IndexEntry {
    IndexEntry {
        name: name.to_string(),
        versions: vec![VersionEntry {
            version: version.to_string(),
            deps: vec![],
            features: Default::default(),
            checksum: checksum.to_string(),
            targets: vec![],
            yanked: false,
        }],
    }
}

#[test]
fn publish_then_clone_fetch_round_trip_and_verifies_sha() {
    let root = temp("rt");
    let bare = seed_bare(&root);

    // Publisher: clone the (empty) registry, publish a package, push.
    let pub_dir = root.join("publisher");
    let reg = Registry::new(bare.to_string_lossy().as_ref(), &pub_dir);
    reg.sync().unwrap();
    git(&pub_dir, &["config", "user.name", "Publisher"]);
    git(&pub_dir, &["config", "user.email", "pub@example.com"]);

    let artifact = b"fn add(a: i32, b: i32) -> i32 { a + b }";
    let checksum = lpp_pm::ContentAddress::of_bytes(artifact).to_string();
    let sum = reg
        .publish(&entry("math", "1.0.0", &checksum), artifact, "publish math 1.0.0")
        .unwrap();
    assert_eq!(sum, checksum);
    reg.push().unwrap();

    // Consumer: fresh clone → offline fetch → sha verified.
    let con_dir = root.join("consumer");
    let creg = Registry::new(bare.to_string_lossy().as_ref(), &con_dir);
    creg.sync().unwrap();
    let (bytes, v) = creg.fetch("math", "1.0.0").unwrap();
    assert_eq!(bytes, artifact);
    assert_eq!(v.checksum, checksum);

    // Re-sync (an existing clone) fast-forwards instead of re-cloning.
    creg.sync().unwrap();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fetch_rejects_a_tampered_blob() {
    let root = temp("tamper");
    let bare = seed_bare(&root);
    let pub_dir = root.join("publisher");
    let reg = Registry::new(bare.to_string_lossy().as_ref(), &pub_dir);
    reg.sync().unwrap();
    git(&pub_dir, &["config", "user.name", "P"]);
    git(&pub_dir, &["config", "user.email", "p@example.com"]);
    let artifact = b"original artifact bytes";
    let checksum = lpp_pm::ContentAddress::of_bytes(artifact).to_string();
    reg.publish(&entry("math", "1.0.0", &checksum), artifact, "init").unwrap();
    reg.push().unwrap();

    // Consumer clones, then the blob is silently corrupted.
    let con_dir = root.join("consumer");
    let creg = Registry::new(bare.to_string_lossy().as_ref(), &con_dir);
    creg.sync().unwrap();
    std::fs::write(con_dir.join("blob").join(&checksum), b"EVIL tampered bytes").unwrap();
    let err = creg.fetch("math", "1.0.0").unwrap_err();
    assert!(matches!(err, lpp_pm::PmError::ChecksumMismatch { .. }), "got {err}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn lookup_missing_package_is_typed_not_found() {
    let root = temp("missing");
    let bare = seed_bare(&root);
    let con_dir = root.join("consumer");
    let creg = Registry::new(bare.to_string_lossy().as_ref(), &con_dir);
    creg.sync().unwrap();
    let err = creg.lookup("nope").unwrap_err();
    assert!(matches!(err, lpp_pm::PmError::PackageNotFound(_)), "got {err}");
    let _ = std::fs::remove_dir_all(&root);
}
