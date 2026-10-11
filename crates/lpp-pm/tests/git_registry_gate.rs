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
    let dir = std::env::temp_dir().join(format!("lpp-git-registry-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Seed a bare git repo (a "remote") with an empty initial commit on `main`.
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
        .publish(
            &entry("math", "1.0.0", &checksum),
            artifact,
            "publish math 1.0.0",
        )
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
    reg.publish(&entry("math", "1.0.0", &checksum), artifact, "init")
        .unwrap();
    reg.push().unwrap();

    // Consumer clones, then the blob is silently corrupted.
    let con_dir = root.join("consumer");
    let creg = Registry::new(bare.to_string_lossy().as_ref(), &con_dir);
    creg.sync().unwrap();
    std::fs::write(con_dir.join("blob").join(&checksum), b"EVIL tampered bytes").unwrap();
    let err = creg.fetch("math", "1.0.0").unwrap_err();
    assert!(
        matches!(err, lpp_pm::PmError::ChecksumMismatch { .. }),
        "got {err}"
    );
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
    assert!(
        matches!(err, lpp_pm::PmError::PackageNotFound(_)),
        "got {err}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn registry_remote_rejects_option_injection_and_insecure_transports() {
    for bad in [
        "",
        "   ",
        "-oProxyCommand=evil",
        "--upload-pack=evil",
        "ext::sh -c touch% /tmp/pwn",
        "EXT::sh -c evil",
        "fd::1",
        "http://registry.lplusplus.bond/repo.git",
        "git://github.com/samarnever-droid/llppregistry.git",
        "ftp://example.com/repo.git",
        "ftps://example.com/repo.git",
        "https://github.com/samarnever-droid/llppregistry.git\n--upload-pack=evil",
        "https://github.com/samarnever-droid/llppregistry.git\0evil",
    ] {
        assert!(
            lpp_pm::validation::registry_remote(bad).is_err(),
            "expected remote {bad:?} to be rejected"
        );
    }

    assert_eq!(
        lpp_pm::validation::registry_remote(
            "  https://github.com/samarnever-droid/llppregistry.git  "
        )
        .unwrap(),
        "https://github.com/samarnever-droid/llppregistry.git"
    );
}

#[test]
fn publish_updates_public_aggregate_index_and_ignores_gitkeep() {
    let root = temp("agg");
    let bare = seed_bare(&root);

    // Seed the remote with index/.gitkeep and registry/index.json containing an existing package.
    let seed = root.join("seed-agg");
    git(
        &root,
        &["clone", bare.to_str().unwrap(), seed.to_str().unwrap()],
    );
    git(&seed, &["config", "user.name", "Seeder"]);
    git(&seed, &["config", "user.email", "seeder@example.com"]);
    std::fs::create_dir_all(seed.join("index")).unwrap();
    std::fs::create_dir_all(seed.join("blob")).unwrap();
    std::fs::create_dir_all(seed.join("registry")).unwrap();
    std::fs::write(seed.join("index/.gitkeep"), "").unwrap();
    std::fs::write(seed.join("blob/.gitkeep"), "").unwrap();
    std::fs::write(
        seed.join("registry/index.json"),
        r#"{
  "packages": {
    "lpp-json": {
      "description": "Fast JSON parser",
      "name": "lpp-json",
      "version": "1.2.0"
    }
  },
  "registry": {
    "name": "L++ Official Package Registry",
    "package_count": 1,
    "repository": "samarnever-droid/llppregistry",
    "source_of_truth": "git",
    "url": "https://registry.lplusplus.bond",
    "version": "3.0.0"
  }
}
"#,
    )
    .unwrap();
    git(&seed, &["add", "."]);
    git(&seed, &["commit", "-m", "seed aggregate index"]);
    git(&seed, &["push", "origin", "HEAD:main"]);

    let pub_dir = root.join("publisher");
    let reg = Registry::new(bare.to_string_lossy().as_ref(), &pub_dir);
    reg.sync().unwrap();
    git(&pub_dir, &["config", "user.name", "Publisher"]);
    git(&pub_dir, &["config", "user.email", "pub@example.com"]);

    // Verify list() ignores .gitkeep when no sparse packages exist yet.
    assert!(reg.list().unwrap().is_empty());

    let artifact = b"lpp-semver archive bytes";
    let checksum = lpp_pm::ContentAddress::of_bytes(artifact).to_string();
    reg.publish(
        &entry("lpp-semver", "0.1.0", &checksum),
        artifact,
        "publish lpp-semver 0.1.0",
    )
    .unwrap();
    reg.push().unwrap();

    // Verify both sparse index and aggregate registry/index.json were updated and pushed.
    let verify_dir = root.join("verify");
    let vreg = Registry::new(bare.to_string_lossy().as_ref(), &verify_dir);
    vreg.sync().unwrap();
    let listed = vreg.list().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "lpp-semver");

    let agg_raw = std::fs::read_to_string(verify_dir.join("registry/index.json")).unwrap();
    let agg: serde_json::Value = serde_json::from_str(&agg_raw).unwrap();
    assert_eq!(agg["registry"]["package_count"], 2);
    assert_eq!(agg["packages"]["lpp-json"]["version"], "1.2.0");
    assert_eq!(agg["packages"]["lpp-semver"]["version"], "0.1.0");
    assert_eq!(
        agg["packages"]["lpp-semver"]["versions"]["0.1.0"]["download_url"],
        format!("https://registry.lplusplus.bond/blob/{checksum}")
    );

    let _ = std::fs::remove_dir_all(&root);
}
