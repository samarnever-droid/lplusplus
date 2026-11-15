//! Production integration test suite for `lpp-pm` (v0.3.0).
//!
//! Covers:
//! - SemVer parsing, ordering, pre-release precedence, caret/tilde/compound ranges
//! - Trust-boundary validation (`validation::*`)
//! - Backtracking dependency resolution, deep transitive chains, diamonds, cycles, workspace union
//! - Git-backed `Registry` (`sync`, `publish`, `push`, `lookup`, `fetch`, `fetch_locked`, yanked versions)
//! - `Keel.lock` v2 roundtrips, v1 compatibility, tampering & invariant rejections, atomic writes
//! - `Workspace::discover` (`Keel.toml` + legacy `lpp.toml` fallback + `.lpp_packages` isolation) & `build_plan`
//! - Fingerprint & Delta transitive invalidation DAGs, multi-target & lock folding, store persistence
//! - `ContentAddress`, `DiskBlobStore` fanout/corruption detection, and `KvCache` eviction/rehydration

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};

use lpp_pm::delta::{Delta, diff, invalidate, mirror_to_kv};
use lpp_pm::fingerprint::{
    FP_STORE_VERSION, FingerprintStore, compute_member_fps_with_lock, dep_key_graph, hash_bytes,
    hash_sources, lpp_identity,
};
use lpp_pm::index::{DepSpec, IndexEntry, VersionEntry, index_path, try_index_path};
use lpp_pm::lock::{LOCK_VERSION, Lock, LockedPkg};
use lpp_pm::manifest::Manifest;
use lpp_pm::resolve::{Candidate, Pkg, resolve, resolve_workspace};
use lpp_pm::semver::{Req, Version};
use lpp_pm::{
    BlobStore, ContentAddress, DiskBlobStore, InMemoryKv, KvBackendKind, KvCache, PmError,
    Registry, Workspace, create_kv, validation,
};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let seq = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("lpp-pm-v03-{tag}-{}-{seq}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap_or(dir)
}

fn ver(s: &str) -> Version {
    Version::parse(s).unwrap_or_else(|| panic!("expected valid version: {s}"))
}

fn req(s: &str) -> Req {
    Req::parse(s).unwrap_or_else(|| panic!("expected valid requirement: {s}"))
}

fn cand(version: &str, deps: &[(&str, &str)]) -> Candidate {
    Candidate {
        version: ver(version),
        checksum: Some(ContentAddress::of_bytes(version.as_bytes()).to_string()),
        source: "registry".into(),
        deps: deps
            .iter()
            .map(|(n, r)| ((*n).to_string(), req(r)))
            .collect(),
    }
}

fn root_pkg(name: &str, version: &str, deps: &[(&str, &str)]) -> Pkg {
    Pkg {
        name: name.into(),
        version: ver(version),
        checksum: None,
        source: "root".into(),
        deps: deps
            .iter()
            .map(|(n, r)| ((*n).to_string(), req(r)))
            .collect(),
    }
}

fn write_pkg(dir: &Path, manifest_name: &str, name: &str, version: &str, deps: &str) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let body = format!("[package]\nname = \"{name}\"\nversion = \"{version}\"\n{deps}\n");
    std::fs::write(dir.join(manifest_name), body).unwrap();
    std::fs::write(
        dir.join("src/main.lpp"),
        format!("def main():\n    print_str(\"{name}\")\n"),
    )
    .unwrap();
}

fn make_fake_compiler(dir: &Path, name: &str, body: &str) -> PathBuf {
    let bin = dir.join(name);
    std::fs::write(&bin, body.as_bytes()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    bin
}

fn run_git(cwd: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git must be installed");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn seed_bare_registry_repo(root: &Path) -> PathBuf {
    let bare = root.join("bare-registry.git");
    run_git(
        root,
        &[
            "init",
            "--bare",
            "--initial-branch=main",
            bare.to_str().unwrap(),
        ],
    );
    let seed = root.join("seed-work");
    run_git(root, &["init", "-b", "main", seed.to_str().unwrap()]);
    run_git(&seed, &["config", "user.name", "Test Seeder"]);
    run_git(&seed, &["config", "user.email", "seeder@lpp.invalid"]);
    run_git(&seed, &["commit", "--allow-empty", "-m", "initial commit"]);
    run_git(&seed, &["push", bare.to_str().unwrap(), "HEAD:main"]);
    let _ = std::fs::remove_dir_all(&seed);
    bare
}

fn version_entry(
    version: &str,
    yanked: bool,
    artifact: &[u8],
    deps: &[(&str, &str)],
) -> VersionEntry {
    VersionEntry {
        version: version.into(),
        deps: deps
            .iter()
            .map(|(n, r)| DepSpec {
                name: (*n).to_string(),
                req: (*r).to_string(),
                optional: false,
                features: Vec::new(),
            })
            .collect(),
        features: BTreeMap::new(),
        checksum: ContentAddress::of_bytes(artifact).to_string(),
        targets: vec!["host".into()],
        yanked,
    }
}

// ============================================================================
// 1. SemVer Parsing, Precedence, and Range Matching Matrix
// ============================================================================

#[test]
fn semver_parsing_normalization_and_invalid_forms() {
    assert_eq!(ver("1.2.3"), Version::new(1, 2, 3));
    assert_eq!(ver("v1.2.3"), Version::new(1, 2, 3));
    assert_eq!(ver("  v2  "), Version::new(2, 0, 0));
    assert_eq!(ver("3.4"), Version::new(3, 4, 0));
    assert_eq!(ver("v0.9"), Version::new(0, 9, 0));
    assert_eq!(ver("0"), Version::new(0, 0, 0));

    let pre = ver("1.0.0-alpha.1+build.42");
    assert_eq!(pre.to_string(), "1.0.0-alpha.1+build.42");
    assert_eq!(pre.as_semver().pre.as_str(), "alpha.1");
    assert_eq!(pre.as_semver().build.as_str(), "build.42");

    for bad in [
        "", "   ", "v", "1.2.3.4", "1..2", "01.2.3", "1.02.3", "-1.0.0", "1.2-beta", "abc", "1.2.x",
    ] {
        assert!(
            Version::parse(bad).is_none(),
            "version '{bad}' should be rejected"
        );
    }
}

#[test]
fn semver_prerelease_and_numeric_precedence_chain() {
    // Canonical SemVer 2.0.0 precedence ladder
    let ladder = [
        "0.0.1",
        "0.1.0-alpha",
        "0.1.0",
        "1.0.0-1",
        "1.0.0-alpha",
        "1.0.0-alpha.1",
        "1.0.0-alpha.beta",
        "1.0.0-beta",
        "1.0.0-beta.2",
        "1.0.0-beta.11",
        "1.0.0-rc.1",
        "1.0.0",
        "1.0.1",
        "1.2.9",
        "1.10.0",
        "2.0.0",
    ];
    for window in ladder.windows(2) {
        let lo = ver(window[0]);
        let hi = ver(window[1]);
        assert!(lo < hi, "expected {lo} < {hi}");
        assert!(hi > lo, "expected {hi} > {lo}");
    }

    // Build metadata does not alter SemVer precedence (`cmp_precedence`),
    // while `Version` structural `Ord` still provides a deterministic total order.
    let b1 = ver("1.0.0+001");
    let b2 = ver("1.0.0+20130313144700");
    assert_eq!(
        b1.as_semver().cmp_precedence(b2.as_semver()),
        std::cmp::Ordering::Equal
    );
    assert!(req("=1.0.0").matches(&b1));
    assert!(req("=1.0.0").matches(&b2));
}

#[test]
fn semver_caret_tilde_wildcard_and_compound_matrix() {
    let cases: &[(&str, &[&str], &[&str])] = &[
        // Caret standard (>=1.0.0)
        (
            "^1.2.3",
            &["1.2.3", "1.2.4", "1.9.99"],
            &["1.2.2", "2.0.0-alpha.1", "2.0.0", "0.9.9"],
        ),
        // Caret 0.y.z (minor is breaking)
        (
            "^0.2.3",
            &["0.2.3", "0.2.9"],
            &["0.2.2", "0.3.0-alpha.1", "0.3.0", "1.0.0"],
        ),
        // Caret 0.0.z (patch is pinned)
        ("^0.0.3", &["0.0.3"], &["0.0.2", "0.0.4", "0.1.0"]),
        // Caret shorthand
        ("^1", &["1.0.0", "1.8.0"], &["0.9.9", "2.0.0"]),
        ("^0.2", &["0.2.0", "0.2.5"], &["0.1.9", "0.3.0"]),
        ("1.2", &["1.2.0", "1.9.1"], &["1.1.9", "2.0.0"]),
        // Caret with pre-release
        (
            "^1.2.3-alpha.2",
            &["1.2.3-alpha.2", "1.2.3-beta.1", "1.2.3", "1.4.0"],
            &["1.2.3-alpha.1", "1.3.0-alpha.1", "2.0.0"],
        ),
        // Tilde
        (
            "~1.2.3",
            &["1.2.3", "1.2.99"],
            &["1.2.2", "1.3.0-alpha.1", "1.3.0", "2.0.0"],
        ),
        ("~1.2", &["1.2.0", "1.2.8"], &["1.1.9", "1.3.0"]),
        ("~1", &["1.0.0", "1.9.0"], &["0.9.0", "2.0.0"]),
        ("~0.2.3", &["0.2.3", "0.2.9"], &["0.2.2", "0.3.0"]),
        // Exact and inequalities
        ("=1.2.3", &["1.2.3"], &["1.2.2", "1.2.4", "1.2.3-rc.1"]),
        (
            ">=1.2.0",
            &["1.2.0", "1.5.0", "9.0.0"],
            &["1.1.9", "1.2.0-rc.1"],
        ),
        (">1.2.0", &["1.2.1", "2.0.0"], &["1.2.0", "1.1.9"]),
        ("<=1.2.0", &["0.0.1", "1.2.0"], &["1.2.1", "2.0.0"]),
        ("<2.0.0", &["0.1.0", "1.9.9"], &["2.0.0", "2.0.0-alpha.1"]),
        // Wildcards
        ("*", &["0.0.1", "1.0.0", "99.0.0"], &[]),
        ("", &["0.0.1", "4.5.6"], &[]),
        ("1.*", &["1.0.0", "1.9.5"], &["0.9.9", "2.0.0"]),
        ("1.2.*", &["1.2.0", "1.2.99"], &["1.1.9", "1.3.0"]),
        // Compound comma-separated constraints
        (
            ">=1.2.0, <2.0.0",
            &["1.2.0", "1.5.3", "1.9.9"],
            &["1.1.9", "2.0.0", "2.0.0-alpha.1"],
        ),
        (
            ">=0.4.2, <=0.6.1",
            &["0.4.2", "0.5.0", "0.6.1"],
            &["0.4.1", "0.6.2", "1.0.0"],
        ),
        (
            "^1.2, <1.4.0",
            &["1.2.0", "1.3.9"],
            &["1.1.9", "1.4.0", "2.0.0"],
        ),
    ];

    for (spec, should_match, should_reject) in cases {
        let r = req(spec);
        for candidate in *should_match {
            assert!(
                r.matches(&ver(candidate)),
                "requirement '{spec}' should match '{candidate}'"
            );
        }
        for candidate in *should_reject {
            assert!(
                !r.matches(&ver(candidate)),
                "requirement '{spec}' should reject '{candidate}'"
            );
        }
    }

    for invalid_req in [">>1.0.0", "^", "~", ">=1.0, ", "1.0.0 || 2.0.0", "invalid"] {
        assert!(
            Req::parse(invalid_req).is_none(),
            "requirement '{invalid_req}' should fail parsing"
        );
    }
    assert_eq!(req("*").to_string(), "*");
    assert_eq!(req("").to_string(), "*");
    assert_eq!(req("^1.2.3").to_string(), "^1.2.3");
}

// ============================================================================
// 2. Trust-Boundary Validation
// ============================================================================

#[test]
fn validation_enforces_package_names_checksums_and_remote_security() {
    for good_name in ["a", "0", "lpp-json", "math2-core", &"x".repeat(64)] {
        assert!(
            validation::package_name(good_name).is_ok(),
            "{good_name} should be valid"
        );
    }
    for bad_name in [
        "",
        "-leading-hyphen",
        "Upper",
        "under_score",
        "has space",
        "dot.name",
        "slash/name",
        &"x".repeat(65),
    ] {
        assert!(
            matches!(
                validation::package_name(bad_name),
                Err(PmError::InvalidPackageName(_))
            ),
            "{bad_name} should be rejected"
        );
    }

    let valid_hash = ContentAddress::of_bytes(b"payload").to_string();
    assert!(validation::checksum(&valid_hash).is_ok());
    assert!(matches!(
        validation::checksum(&valid_hash.to_ascii_uppercase()),
        Err(PmError::InvalidAddress(_))
    ));
    assert!(matches!(
        validation::checksum("deadbeef"),
        Err(PmError::InvalidAddress(_))
    ));

    // Security rules for git registry remotes
    for ok_remote in [
        "https://github.com/samarnever-droid/llppregistry.git",
        "ssh://git@github.com/org/reg.git",
        "file:///tmp/reg.git",
        "  /var/lib/lpp/registry.git  ",
    ] {
        assert!(
            validation::registry_remote(ok_remote).is_ok(),
            "remote '{ok_remote}' should be accepted"
        );
    }
    for bad_remote in [
        "",
        "   ",
        "--upload-pack=evil",
        "-oProxyCommand=evil",
        "https://example.com/reg\n.git",
        "https://example.com/reg\x00.git",
        "ext::sh -c id",
        "fd::123",
        "http://insecure.example.com/reg.git",
        "git://unauthenticated.example.com/reg.git",
        "ftp://files.example.com/reg.git",
        "ftps://files.example.com/reg.git",
    ] {
        assert!(
            matches!(
                validation::registry_remote(bad_remote),
                Err(PmError::Git(_))
            ),
            "remote '{bad_remote}' should be rejected"
        );
    }
}

// ============================================================================
// 3. Deep Transitive, Backtracking, Diamond & Workspace Resolution
// ============================================================================

#[test]
fn resolver_handles_deep_transitive_chain_and_multi_level_backtracking() {
    // 6-level transitive chain: app -> a -> b -> c -> d -> e -> f
    let chain = resolve(
        &root_pkg("app", "0.1.0", &[("a", "^1.0")]),
        &|name| match name {
            "a" => Some(vec![cand("1.2.0", &[("b", "^2.0")])]),
            "b" => Some(vec![cand("2.3.0", &[("c", "~1.4.0")])]),
            "c" => Some(vec![cand("1.4.9", &[("d", ">=0.5.0, <0.8.0")])]),
            "d" => Some(vec![cand("0.7.2", &[("e", "^0.2.0")])]),
            "e" => Some(vec![cand("0.2.5", &[("f", "=3.1.0")])]),
            "f" => Some(vec![cand("3.1.0", &[]), cand("3.2.0", &[])]),
            _ => None,
        },
    )
    .unwrap();
    assert_eq!(chain.len(), 7);
    assert_eq!(chain.get("f").unwrap().version, ver("3.1.0"));
    assert_eq!(chain.get("d").unwrap().version, ver("0.7.2"));

    // Multi-level backtracking:
    // app depends on `alpha *` and `anchor *`.
    // alpha 3.0.0 -> beta ^3.0 -> gamma ^3.0 (incompatible with anchor -> gamma ^1.0)
    // alpha 2.0.0 -> beta ^2.0 -> gamma ^2.0 (also incompatible with anchor -> gamma ^1.0)
    // alpha 1.5.0 -> beta ^1.0 -> gamma >=1.0.0, <1.5.0 (compatible with anchor -> gamma >=1.2.0, <2.0.0)
    let backtracked = resolve(
        &root_pkg("app", "0.1.0", &[("alpha", "*"), ("anchor", "*")]),
        &|name| match name {
            "alpha" => Some(vec![
                cand("3.0.0", &[("beta", "^3.0")]),
                cand("2.0.0", &[("beta", "^2.0")]),
                cand("1.5.0", &[("beta", "^1.0")]),
            ]),
            "anchor" => Some(vec![cand("1.0.0", &[("gamma", ">=1.2.0, <2.0.0")])]),
            "beta" => Some(vec![
                cand("3.1.0", &[("gamma", "^3.0")]),
                cand("2.1.0", &[("gamma", "^2.0")]),
                cand("1.3.0", &[("gamma", ">=1.0.0, <1.5.0")]),
            ]),
            "gamma" => Some(vec![
                cand("3.0.0", &[]),
                cand("2.0.0", &[]),
                cand("1.6.0", &[]),
                cand("1.4.2", &[]),
                cand("1.1.0", &[]),
            ]),
            _ => None,
        },
    )
    .unwrap();
    assert_eq!(backtracked.get("alpha").unwrap().version, ver("1.5.0"));
    assert_eq!(backtracked.get("beta").unwrap().version, ver("1.3.0"));
    assert_eq!(backtracked.get("gamma").unwrap().version, ver("1.4.2"));
}

#[test]
fn resolver_diamond_unification_conflicts_cycles_and_prereleases() {
    // Asymmetric diamond: app -> (left, right), both -> (mid, util) -> leaf
    let diamond = resolve(
        &root_pkg("app", "0.2.0", &[("left", "^1.0"), ("right", "^1.0")]),
        &|name| match name {
            "left" => Some(vec![cand("1.1.0", &[("mid", "^1.0"), ("util", ">=1.1.0")])]),
            "right" => Some(vec![cand(
                "1.2.0",
                &[("mid", ">=1.2.0, <1.5.0"), ("util", "<1.4.0")],
            )]),
            "mid" => Some(vec![
                cand("1.6.0", &[("leaf", "^2.0")]),
                cand("1.4.0", &[("leaf", "^1.0")]),
                cand("1.1.0", &[("leaf", "^1.0")]),
            ]),
            "util" => Some(vec![
                cand("1.5.0", &[("leaf", "^1.0")]),
                cand("1.3.2", &[("leaf", ">=1.2.0, <1.4.0")]),
                cand("1.0.0", &[("leaf", "^1.0")]),
            ]),
            "leaf" => Some(vec![
                cand("2.0.0", &[]),
                cand("1.5.0", &[]),
                cand("1.3.5", &[]),
            ]),
            _ => None,
        },
    )
    .unwrap();
    assert_eq!(diamond.get("mid").unwrap().version, ver("1.4.0"));
    assert_eq!(diamond.get("util").unwrap().version, ver("1.3.2"));
    assert_eq!(diamond.get("leaf").unwrap().version, ver("1.3.5"));

    // Unresolvable diamond conflict -> ResolveConflict
    let conflict = resolve(
        &root_pkg("app", "0.1.0", &[("a", "^1.0"), ("b", "^1.0")]),
        &|name| match name {
            "a" => Some(vec![cand("1.0.0", &[("shared", "^1.0")])]),
            "b" => Some(vec![cand("1.0.0", &[("shared", "^2.0")])]),
            "shared" => Some(vec![cand("1.9.0", &[]), cand("2.1.0", &[])]),
            _ => None,
        },
    )
    .unwrap_err();
    assert!(
        matches!(conflict, PmError::ResolveConflict { .. }),
        "{conflict}"
    );

    // Cycle a -> b -> c -> a terminates cleanly
    let cyclic = resolve(
        &root_pkg("app", "0.1.0", &[("a", "^1.0")]),
        &|name| match name {
            "a" => Some(vec![cand("1.0.0", &[("b", "^1.0")])]),
            "b" => Some(vec![cand("1.0.0", &[("c", "^1.0")])]),
            "c" => Some(vec![cand("1.0.0", &[("a", "^1.0")])]),
            _ => None,
        },
    )
    .unwrap();
    assert_eq!(cyclic.len(), 4);

    // Pre-release versions are not matched by stable ^1.0 unless opted in
    let pre_res = resolve(
        &root_pkg("app", "0.1.0", &[("lib", "^1.0")]),
        &|name| match name {
            "lib" => Some(vec![
                cand("1.2.0", &[]),
                cand("1.3.0-alpha.1", &[]),
                cand("2.0.0-rc.1", &[]),
            ]),
            _ => None,
        },
    )
    .unwrap();
    assert_eq!(pre_res.get("lib").unwrap().version, ver("1.2.0"));

    // Missing package -> NoMatchingVersion
    let missing = resolve(&root_pkg("app", "0.1.0", &[("ghost", "^1.0")]), &|_| None).unwrap_err();
    assert!(
        matches!(missing, PmError::NoMatchingVersion { .. }),
        "{missing}"
    );
}

#[test]
fn resolve_workspace_unifies_three_roots_and_rejects_duplicate_roots() {
    let available = |name: &str| match name {
        "json" => Some(vec![
            cand("1.0.0", &[("core", "^1.0")]),
            cand("1.3.0", &[("core", ">=1.2.0, <1.5.0")]),
            cand("1.6.0", &[("core", "^1.5")]),
        ]),
        "core" => Some(vec![
            cand("1.1.0", &[]),
            cand("1.4.0", &[]),
            cand("1.5.2", &[]),
        ]),
        _ => None,
    };
    let roots = vec![
        root_pkg("svc-a", "0.1.0", &[("json", "^1.0")]),
        root_pkg("svc-b", "0.1.0", &[("json", ">=1.2.0, <1.6.0")]),
        root_pkg("cli", "0.1.0", &[("core", "<1.5.0")]),
    ];
    let resolved = resolve_workspace(&roots, &available).unwrap();
    assert_eq!(resolved.get("json").unwrap().version, ver("1.3.0"));
    assert_eq!(resolved.get("core").unwrap().version, ver("1.4.0"));
    assert!(resolved.get("<workspace>").is_none());

    // Duplicate workspace root names must fail cleanly
    let dup_roots = vec![root_pkg("dup", "1.0.0", &[]), root_pkg("dup", "2.0.0", &[])];
    let dup_err = resolve_workspace(&dup_roots, &available).unwrap_err();
    assert!(
        matches!(dup_err, PmError::ResolveConflict { .. }),
        "{dup_err}"
    );
}

// ============================================================================
// 4. Git-Backed Registry, Sparse Index, and Yanked Version Handling
// ============================================================================

#[test]
fn sparse_index_paths_and_entry_validation() {
    assert_eq!(try_index_path("a").unwrap(), "1/a");
    assert_eq!(try_index_path("ab").unwrap(), "2/ab");
    assert_eq!(try_index_path("lpp").unwrap(), "3/l/pp");
    assert_eq!(try_index_path("lpp-json").unwrap(), "lp/p-/lpp-json");
    assert_eq!(index_path("lpp-math"), "lp/p-/lpp-math");
    assert!(matches!(
        try_index_path("BAD"),
        Err(PmError::InvalidPackageName(_))
    ));

    let valid_entry = IndexEntry {
        name: "lpp-json".into(),
        versions: vec![
            version_entry("1.0.0", false, b"json-1.0.0", &[("lpp-core", "^1.0")]),
            version_entry("1.1.0", true, b"json-1.1.0", &[]),
        ],
    };
    valid_entry.validate().unwrap();
    assert!(
        valid_entry
            .download_url("https://registry.lplusplus.bond", "1.0.0")
            .unwrap()
            .starts_with("https://registry.lplusplus.bond/blob/")
    );
    assert!(
        valid_entry
            .download_url("https://registry.lplusplus.bond", "9.9.9")
            .is_none()
    );

    // Duplicate version rejected
    let dup_entry = IndexEntry {
        name: "lpp-json".into(),
        versions: vec![
            version_entry("1.0.0", false, b"a", &[]),
            version_entry("1.0.0", false, b"b", &[]),
        ],
    };
    assert!(matches!(dup_entry.validate(), Err(PmError::IndexParse(_))));

    // Bad checksum rejected
    let mut bad_sum = valid_entry.clone();
    bad_sum.versions[0].checksum = "not-a-sha256".into();
    assert!(matches!(
        bad_sum.validate(),
        Err(PmError::InvalidAddress(_))
    ));
}

#[test]
fn registry_publish_sync_fetch_yank_and_corruption_detection() {
    let root = temp_dir("registry");
    let bare = seed_bare_registry_repo(&root);
    let remote = bare.to_string_lossy().to_string();

    let pub_dir = root.join("publisher");
    let pub_reg = Registry::new(&remote, &pub_dir);
    pub_reg.sync().unwrap();
    run_git(&pub_dir, &["config", "user.name", "Publisher"]);
    run_git(&pub_dir, &["config", "user.email", "pub@lpp.invalid"]);

    let v1_bytes = b"pkg-math-1.0.0-tarball";
    let v2_bytes = b"pkg-math-1.1.0-yanked-tarball";
    let v1_entry = version_entry("1.0.0", false, v1_bytes, &[]);
    let v2_entry = version_entry("1.1.0", true, v2_bytes, &[]);

    // Publish v1.0.0 first so its blob lands, then republish with v1.1.0 yanked
    let c1 = pub_reg
        .publish(
            &IndexEntry {
                name: "math".into(),
                versions: vec![v1_entry.clone()],
            },
            v1_bytes,
            "publish math 1.0.0",
        )
        .unwrap();
    assert_eq!(c1, v1_entry.checksum);

    let c2 = pub_reg
        .publish(
            &IndexEntry {
                name: "math".into(),
                versions: vec![v1_entry, v2_entry.clone()],
            },
            v2_bytes,
            "publish math 1.1.0 (yanked)",
        )
        .unwrap();
    assert_eq!(c2, v2_entry.checksum);

    // Mismatched checksum not in IndexEntry must be rejected before committing
    let bad_pub = pub_reg.publish(
        &IndexEntry {
            name: "other".into(),
            versions: vec![version_entry("0.1.0", false, b"expected-bytes", &[])],
        },
        b"different-bytes",
        "bad publish",
    );
    assert!(matches!(bad_pub, Err(PmError::IndexParse(_))));

    pub_reg.push().unwrap();

    // Consumer clones into fresh local cache with shared DiskBlobStore
    let consumer_cache = root.join("consumer-reg");
    let blob_root = root.join("shared-blobs");
    let reg = Registry::new(&remote, &consumer_cache).with_blob_store(&blob_root);
    reg.sync().unwrap();

    let found = reg.lookup("math").unwrap();
    assert_eq!(found.versions.len(), 2);
    assert_eq!(reg.list().unwrap().len(), 1);
    assert_eq!(reg.search("MAT").unwrap().len(), 1);
    assert!(reg.search("nonexistent").unwrap().is_empty());

    // Regular fetch succeeds for non-yanked 1.0.0 and populates DiskBlobStore
    let (fetched, ve1) = reg.fetch("math", "1.0.0").unwrap();
    assert_eq!(fetched, v1_bytes);
    assert!(!ve1.yanked);
    let store = DiskBlobStore::open(&blob_root).unwrap();
    let addr1 = ContentAddress::try_new(&c1).unwrap();
    assert!(store.contains(&addr1));

    // Regular fetch refuses yanked 1.1.0; fetch_locked permits it for pinned lockfiles
    let yanked_err = reg.fetch("math", "1.1.0").unwrap_err();
    assert!(
        matches!(yanked_err, PmError::VersionNotFound { .. }),
        "{yanked_err}"
    );
    let (locked_bytes, ve2) = reg.fetch_locked("math", "1.1.0").unwrap();
    assert_eq!(locked_bytes, v2_bytes);
    assert!(ve2.yanked);

    // Tampering the blob in the shared store triggers ChecksumMismatch
    let blob_file = blob_root.join("blobs").join(&c1[0..2]).join(&c1[2..]);
    std::fs::write(&blob_file, b"tampered-artifact").unwrap();
    let tamper_err = reg.fetch("math", "1.0.0").unwrap_err();
    assert!(
        matches!(tamper_err, PmError::ChecksumMismatch { .. }),
        "{tamper_err}"
    );

    // Origin mismatch on existing clone must fail
    let wrong_origin = Registry::new(
        root.join("other-bare.git").to_string_lossy().as_ref(),
        &consumer_cache,
    );
    assert!(matches!(wrong_origin.sync(), Err(PmError::Git(_))));

    let _ = std::fs::remove_dir_all(&root);
}

// ============================================================================
// 5. Lockfile v2 Roundtrip, v1 Compatibility, Tampering & Concurrency
// ============================================================================

#[test]
fn lockfile_roundtrip_v1_compat_and_tampering_rejections() {
    let resolved = resolve(
        &root_pkg("app", "0.3.0", &[("a", "^1.0"), ("b", "^2.0")]),
        &|name| match name {
            "a" => Some(vec![cand("1.2.0", &[("b", ">=2.1.0, <3.0.0")])]),
            "b" => Some(vec![cand("2.4.0", &[])]),
            _ => None,
        },
    )
    .unwrap();

    let mut lock = Lock::from_resolved(&resolved);
    lock.registry = Some("https://github.com/samarnever-droid/llppregistry.git".into());
    assert_eq!(lock.lock_version, LOCK_VERSION);
    // Packages are sorted deterministically by name
    let names: Vec<&str> = lock.packages.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["a", "app", "b"]);
    assert_eq!(
        lock.package("a")
            .unwrap()
            .dep_reqs
            .get("b")
            .map(String::as_str),
        Some(">=2.1.0, <3.0.0")
    );

    let toml_text = lock.to_toml().unwrap();
    assert!(toml_text.contains("version = 2"));
    let parsed = Lock::parse(&toml_text).unwrap();
    assert_eq!(parsed, lock);
    assert_eq!(
        parsed.checksum("b"),
        Some(ContentAddress::of_bytes(b"2.4.0").as_str())
    );
    assert!(parsed.checksum("app").is_none());

    // Lock format v1 remains parseable for legacy upgrades
    let v1_toml = toml_text.replacen("version = 2", "version = 1", 1);
    assert_eq!(Lock::parse(&v1_toml).unwrap().lock_version, 1);

    // Unsupported versions rejected
    for bad_ver in [0, 3, 99] {
        let bad_toml = toml_text.replacen("version = 2", &format!("version = {bad_ver}"), 1);
        assert!(matches!(Lock::parse(&bad_toml), Err(PmError::LockParse(_))));
    }

    let valid_sum = ContentAddress::of_bytes(b"ok").to_string();

    // Tampering scenarios that must fail Lock::validate
    let invalid_cases: Vec<(&str, Lock)> = vec![
        (
            "registry missing checksum",
            Lock {
                lock_version: 2,
                registry: None,
                packages: vec![LockedPkg {
                    name: "math".into(),
                    version: "1.0.0".into(),
                    source: "registry".into(),
                    checksum: None,
                    deps: vec![],
                    dep_reqs: BTreeMap::new(),
                }],
            },
        ),
        (
            "local root has checksum",
            Lock {
                lock_version: 2,
                registry: None,
                packages: vec![LockedPkg {
                    name: "app".into(),
                    version: "1.0.0".into(),
                    source: "root".into(),
                    checksum: Some(valid_sum.clone()),
                    deps: vec![],
                    dep_reqs: BTreeMap::new(),
                }],
            },
        ),
        (
            "unknown source",
            Lock {
                lock_version: 2,
                registry: None,
                packages: vec![LockedPkg {
                    name: "app".into(),
                    version: "1.0.0".into(),
                    source: "git+https://evil".into(),
                    checksum: None,
                    deps: vec![],
                    dep_reqs: BTreeMap::new(),
                }],
            },
        ),
        (
            "duplicate package",
            Lock {
                lock_version: 2,
                registry: None,
                packages: vec![
                    LockedPkg {
                        name: "app".into(),
                        version: "1.0.0".into(),
                        source: "root".into(),
                        checksum: None,
                        deps: vec![],
                        dep_reqs: BTreeMap::new(),
                    },
                    LockedPkg {
                        name: "app".into(),
                        version: "1.0.0".into(),
                        source: "path".into(),
                        checksum: None,
                        deps: vec![],
                        dep_reqs: BTreeMap::new(),
                    },
                ],
            },
        ),
        (
            "dangling dependency edge",
            Lock {
                lock_version: 2,
                registry: None,
                packages: vec![LockedPkg {
                    name: "app".into(),
                    version: "1.0.0".into(),
                    source: "root".into(),
                    checksum: None,
                    deps: vec!["missing-dep".into()],
                    dep_reqs: BTreeMap::new(),
                }],
            },
        ),
        (
            "undeclared dep_reqs key",
            Lock {
                lock_version: 2,
                registry: None,
                packages: vec![LockedPkg {
                    name: "app".into(),
                    version: "1.0.0".into(),
                    source: "root".into(),
                    checksum: None,
                    deps: vec![],
                    dep_reqs: [("ghost".into(), "^1.0".into())].into_iter().collect(),
                }],
            },
        ),
        (
            "edge requirement violated by selected version",
            Lock {
                lock_version: 2,
                registry: None,
                packages: vec![
                    LockedPkg {
                        name: "app".into(),
                        version: "1.0.0".into(),
                        source: "root".into(),
                        checksum: None,
                        deps: vec!["lib".into()],
                        dep_reqs: [("lib".into(), "^2.0".into())].into_iter().collect(),
                    },
                    LockedPkg {
                        name: "lib".into(),
                        version: "1.5.0".into(),
                        source: "registry".into(),
                        checksum: Some(valid_sum),
                        deps: vec![],
                        dep_reqs: BTreeMap::new(),
                    },
                ],
            },
        ),
    ];

    for (label, tampered) in invalid_cases {
        let err = tampered.validate().unwrap_err();
        assert!(
            matches!(err, PmError::LockParse(_)),
            "{label}: expected LockParse, got {err}"
        );
    }
}

#[test]
fn lock_concurrent_atomic_saves_produce_valid_toml_with_no_temp_leftovers() {
    let dir = temp_dir("lock-concurrent");
    let lock_path = Arc::new(dir.join("Keel.lock"));
    let barrier = Arc::new(Barrier::new(12));
    let mut handles = Vec::new();

    for i in 0..12 {
        let path = Arc::clone(&lock_path);
        let gate = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            let lock = Lock {
                lock_version: LOCK_VERSION,
                registry: Some(format!("https://reg.example/{i}")),
                packages: vec![
                    LockedPkg {
                        name: "root".into(),
                        version: "1.0.0".into(),
                        source: "root".into(),
                        checksum: None,
                        deps: vec!["dep".into()],
                        dep_reqs: [("dep".into(), "^1.0".into())].into_iter().collect(),
                    },
                    LockedPkg {
                        name: "dep".into(),
                        version: format!("1.{i}.0"),
                        source: "registry".into(),
                        checksum: Some(
                            ContentAddress::of_bytes(format!("dep-{i}").as_bytes()).to_string(),
                        ),
                        deps: vec![],
                        dep_reqs: BTreeMap::new(),
                    },
                ],
            };
            gate.wait();
            lock.save_atomic(&path).unwrap();
        }));
    }

    for h in handles {
        h.join().unwrap();
    }
    let text = std::fs::read_to_string(lock_path.as_ref()).unwrap();
    let final_lock = Lock::parse(&text).unwrap();
    assert_eq!(final_lock.packages.len(), 2);
    for entry in std::fs::read_dir(&dir).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        assert!(!name.contains(".tmp-"), "leftover temp file: {name}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ============================================================================
// 6. Manifest Validation, Workspace Discovery, Legacy Fallback & `.lpp_packages`
// ============================================================================

#[test]
fn manifest_parse_and_validation_rules() {
    let doc = r#"
[package]
name = "engine"
version = "0.3.0"
edition = "2024"
license = "MIT OR Apache-2.0"
description = "Core runtime"

[dependencies]
math = "^1.2"
local-core = { path = "../core" }
simd = { version = "2", optional = true, features = ["avx2"] }

[features]
default = ["simd"]
full = ["simd"]

[targets]
supported = ["host", "wasm32-wasi"]
"#;
    let m = Manifest::parse(doc).unwrap();
    assert_eq!(m.name(), "engine");
    assert_eq!(m.version(), "0.3.0");
    assert_eq!(m.dependencies["math"].version(), "^1.2");
    assert_eq!(m.dependencies["local-core"].path(), Some("../core"));
    assert_eq!(m.dependencies["local-core"].version(), "*");
    assert!(m.dependencies["simd"].optional());
    assert_eq!(m.dependencies["simd"].features(), &["avx2"]);
    assert_eq!(
        m.targets.as_ref().unwrap().supported,
        vec!["host", "wasm32-wasi"]
    );
    let roundtrip = Manifest::parse(&m.to_toml().unwrap()).unwrap();
    assert_eq!(roundtrip, m);

    for bad_manifest in [
        "[package]\nname = \"app\"\nversion = \"1.0.0\"\nedition = \"  \"\n",
        "[package]\nname = \"app\"\nversion = \"1.0.0\"\n[dependencies]\na = { path = \"  \" }\n",
        "[package]\nname = \"app\"\nversion = \"1.0.0\"\n[dependencies]\na = { git = \"https://example.com/a.git\" }\n",
        "[package]\nname = \"app\"\nversion = \"1.0.0\"\n[dependencies]\na = { path = \"../a\", git = \"https://example.com/a.git\" }\n",
        "[package]\nname = \"app\"\nversion = \"1.0.0\"\n[features]\n\"\" = [\"x\"]\n",
        "[package]\nname = \"app\"\nversion = \"1.0.0\"\n[features]\nfast = [\"\"]\n",
    ] {
        assert!(
            matches!(
                Manifest::parse(bad_manifest),
                Err(PmError::ManifestParse(_))
            ),
            "manifest should fail validation:\n{bad_manifest}"
        );
    }
}

#[test]
fn workspace_discover_keel_toml_legacy_lpp_toml_and_lpp_packages_isolation() {
    let root = temp_dir("discover");

    // 1. Legacy lpp.toml fallback (standalone package + nested file path)
    let legacy_solo = root.join("legacy-solo");
    write_pkg(&legacy_solo, "lpp.toml", "legacy-solo", "0.2.0", "");
    let ws_legacy = Workspace::discover(&legacy_solo.join("src/main.lpp")).unwrap();
    assert!(!ws_legacy.virtual_root);
    assert_eq!(ws_legacy.members.len(), 1);
    assert_eq!(ws_legacy.members[0].name(), "legacy-solo");
    assert_eq!(ws_legacy.out_dir(), legacy_solo.join("target"));

    // 2. When both Keel.toml and lpp.toml exist, Keel.toml wins
    write_pkg(&legacy_solo, "Keel.toml", "modern-solo", "0.9.0", "");
    let ws_modern = Workspace::discover(&legacy_solo).unwrap();
    assert_eq!(ws_modern.members[0].name(), "modern-solo");
    assert_eq!(ws_modern.members[0].manifest.version(), "0.9.0");

    // 3. `.lpp_packages` isolation: a staged dep containing only `lpp.toml` inside
    // `.lpp_packages/<dep>` must NOT be treated as a standalone workspace; discovery
    // must walk up to the enclosing workspace root.
    let staged_dep = legacy_solo.join(".lpp_packages/helper-dep");
    write_pkg(&staged_dep, "lpp.toml", "helper-dep", "1.0.0", "");
    let ws_from_staged = Workspace::discover(&staged_dep).unwrap();
    assert_eq!(ws_from_staged.root, legacy_solo);
    assert_eq!(ws_from_staged.members[0].name(), "modern-solo");

    // And a staged `.lpp_packages/<dep>/lpp.toml` with NO parent manifest fails discovery
    let orphan_staged = root.join("orphan/.lpp_packages/staged-only");
    write_pkg(&orphan_staged, "lpp.toml", "staged-only", "1.0.0", "");
    let orphan_err = Workspace::discover(&orphan_staged).unwrap_err();
    assert!(
        matches!(orphan_err, PmError::MemberNotFound { .. }),
        "{orphan_err}"
    );

    // 4. Legacy virtual workspace with mixed lpp.toml members + Keel.toml precedence
    let legacy_ws = root.join("legacy-ws");
    std::fs::create_dir_all(&legacy_ws).unwrap();
    std::fs::write(
        legacy_ws.join("lpp.toml"),
        "[workspace]\nmembers = [\"crates/*\"]\n",
    )
    .unwrap();
    write_pkg(
        &legacy_ws.join("crates/core"),
        "lpp.toml",
        "core",
        "1.0.0",
        "",
    );
    write_pkg(
        &legacy_ws.join("crates/app"),
        "lpp.toml",
        "app",
        "1.0.0",
        "[dependencies]\ncore = { path = \"../core\" }\n",
    );
    let loaded_legacy_ws = Workspace::discover(&legacy_ws.join("crates/app/src")).unwrap();
    assert!(loaded_legacy_ws.virtual_root);
    assert_eq!(loaded_legacy_ws.members.len(), 2);
    assert_eq!(loaded_legacy_ws.out_dir(), legacy_ws.join("target"));

    // 5. Mixed root-package + members workspace
    let mixed_ws = root.join("mixed-ws");
    write_pkg(
        &mixed_ws,
        "Keel.toml",
        "umbrella",
        "1.0.0",
        "[dependencies]\nsub = { path = \"crates/sub\" }\n\n[workspace]\nmembers = [\"crates/*\"]\n",
    );
    write_pkg(
        &mixed_ws.join("crates/sub"),
        "Keel.toml",
        "sub",
        "0.5.0",
        "",
    );
    let loaded_mixed = Workspace::discover(&mixed_ws).unwrap();
    assert!(!loaded_mixed.virtual_root);
    assert_eq!(
        loaded_mixed
            .members
            .iter()
            .map(|m| m.name())
            .collect::<Vec<_>>(),
        vec!["umbrella", "sub"]
    );
    let plan = loaded_mixed.build_plan().unwrap();
    assert_eq!(plan.len(), 2);
    assert_eq!(loaded_mixed.members[plan[0][0]].name(), "sub");
    assert_eq!(loaded_mixed.members[plan[1][0]].name(), "umbrella");

    // 6. Subdirectory under a virtual workspace root that is NOT a member is rejected
    let non_member = legacy_ws.join("scratch/not-a-member");
    std::fs::create_dir_all(&non_member).unwrap();
    let non_member_err = Workspace::discover(&non_member).unwrap_err();
    assert!(
        matches!(non_member_err, PmError::MemberNotFound { .. }),
        "{non_member_err}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn workspace_build_plan_handles_multi_layer_dag_and_rejects_errors() {
    let root = temp_dir("plan-dag");
    // 6-member DAG:
    // layer 0: core, log
    // layer 1: net(core, log), store(core)
    // layer 2: api(net, store)
    // layer 3: cli(api, log)
    std::fs::write(
        root.join("Keel.toml"),
        "[workspace]\nmembers = [\"crates/*\", \"apps/*\"]\n",
    )
    .unwrap();
    write_pkg(&root.join("crates/core"), "Keel.toml", "core", "1.0.0", "");
    write_pkg(&root.join("crates/log"), "Keel.toml", "log", "1.0.0", "");
    write_pkg(
        &root.join("crates/net"),
        "Keel.toml",
        "net",
        "1.0.0",
        "[dependencies]\ncore = { path = \"../core\", version = \"^1.0\" }\nlog = { path = \"../log\" }\n",
    );
    write_pkg(
        &root.join("crates/store"),
        "Keel.toml",
        "store",
        "1.0.0",
        "[dependencies]\ncore = { path = \"../core\" }\n",
    );
    write_pkg(
        &root.join("crates/api"),
        "Keel.toml",
        "api",
        "1.0.0",
        "[dependencies]\nnet = { path = \"../net\" }\nstore = { path = \"../store\" }\n",
    );
    write_pkg(
        &root.join("apps/cli"),
        "Keel.toml",
        "cli",
        "1.0.0",
        "[dependencies]\napi = { path = \"../../crates/api\" }\nlog = { path = \"../../crates/log\" }\n",
    );

    let ws = Workspace::discover(&root).unwrap();
    let plan = ws.build_plan().unwrap();
    let layers: Vec<Vec<&str>> = plan
        .iter()
        .map(|layer| layer.iter().map(|&idx| ws.members[idx].name()).collect())
        .collect();
    assert_eq!(
        layers,
        vec![
            vec!["core", "log"],
            vec!["net", "store"],
            vec!["api"],
            vec!["cli"],
        ]
    );

    // Path dep pointing to a member with a different package name -> PathDepOutsideWorkspace
    let wrong_name = temp_dir("wrong-target-name");
    std::fs::write(
        wrong_name.join("Keel.toml"),
        "[workspace]\nmembers = [\"a\", \"b\"]\n",
    )
    .unwrap();
    write_pkg(&wrong_name.join("a"), "Keel.toml", "a", "1.0.0", "");
    write_pkg(
        &wrong_name.join("b"),
        "Keel.toml",
        "b",
        "1.0.0",
        "[dependencies]\nwrong = { path = \"../a\" }\n",
    );
    let err_name = Workspace::discover(&wrong_name)
        .unwrap()
        .build_plan()
        .unwrap_err();
    assert!(
        matches!(err_name, PmError::PathDepOutsideWorkspace { .. }),
        "{err_name}"
    );

    // Path dep whose declared version constraint mismatches target member's version -> ResolveConflict
    let ver_mismatch = temp_dir("path-ver-mismatch");
    std::fs::write(
        ver_mismatch.join("Keel.toml"),
        "[workspace]\nmembers = [\"a\", \"b\"]\n",
    )
    .unwrap();
    write_pkg(&ver_mismatch.join("a"), "Keel.toml", "a", "1.0.0", "");
    write_pkg(
        &ver_mismatch.join("b"),
        "Keel.toml",
        "b",
        "1.0.0",
        "[dependencies]\na = { path = \"../a\", version = \"^2.0\" }\n",
    );
    let err_ver = Workspace::discover(&ver_mismatch)
        .unwrap()
        .build_plan()
        .unwrap_err();
    assert!(
        matches!(err_ver, PmError::ResolveConflict { .. }),
        "{err_ver}"
    );

    // 3-member cycle a -> b -> c -> a -> WorkspaceCycle
    let cyc = temp_dir("3cycle");
    std::fs::write(
        cyc.join("Keel.toml"),
        "[workspace]\nmembers = [\"a\", \"b\", \"c\"]\n",
    )
    .unwrap();
    write_pkg(
        &cyc.join("a"),
        "Keel.toml",
        "a",
        "1.0.0",
        "[dependencies]\nb = { path = \"../b\" }\n",
    );
    write_pkg(
        &cyc.join("b"),
        "Keel.toml",
        "b",
        "1.0.0",
        "[dependencies]\nc = { path = \"../c\" }\n",
    );
    write_pkg(
        &cyc.join("c"),
        "Keel.toml",
        "c",
        "1.0.0",
        "[dependencies]\na = { path = \"../a\" }\n",
    );
    let err_cyc = Workspace::discover(&cyc).unwrap().build_plan().unwrap_err();
    assert!(
        matches!(err_cyc, PmError::WorkspaceCycle { .. }),
        "{err_cyc}"
    );

    // Glob escaping workspace root -> BadMemberPattern
    let escape_parent = temp_dir("escape-glob");
    let escape = escape_parent.join("ws");
    let outside = escape_parent.join("outside");
    std::fs::create_dir_all(&escape).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(
        escape.join("Keel.toml"),
        "[workspace]\nmembers = [\"../outside/*\"]\n",
    )
    .unwrap();
    assert!(matches!(
        Workspace::discover(&escape),
        Err(PmError::BadMemberPattern(_))
    ));

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&wrong_name);
    let _ = std::fs::remove_dir_all(&ver_mismatch);
    let _ = std::fs::remove_dir_all(&cyc);
    let _ = std::fs::remove_dir_all(&escape_parent);
}

// ============================================================================
// 7. Fingerprint & Delta Invalidation DAGs + Store Persistence
// ============================================================================

#[test]
fn hash_sources_filtering_and_multi_target_lock_fingerprints() {
    let root = temp_dir("fp-targets");
    let compiler = make_fake_compiler(&root, "lpp-fake", "#!/bin/sh\nexit 0\n");

    std::fs::write(
        root.join("Keel.toml"),
        "[workspace]\nmembers = [\"core\", \"app\"]\n",
    )
    .unwrap();

    // core supports both host and wasm32-wasi
    write_pkg(
        &root.join("core"),
        "Keel.toml",
        "core",
        "1.0.0",
        "[targets]\nsupported = [\"host\", \"wasm32-wasi\"]\n",
    );
    // Excluded directories inside core must NOT influence hash_sources
    for ignored in ["target", ".git", ".lpp_packages", "tests", "examples"] {
        let dir = root.join("core").join(ignored);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("noise.lpp"), "def noise():\n    return\n").unwrap();
    }
    std::fs::write(root.join("core/src/notes.md"), "not an lpp file").unwrap();
    let core_hashes = hash_sources(&root.join("core"));
    assert_eq!(
        core_hashes.keys().cloned().collect::<Vec<_>>(),
        vec!["src/main.lpp"]
    );

    // app depends on core (path), optional `opt-a` enabled by default feature, and disabled `opt-b`
    write_pkg(
        &root.join("app"),
        "Keel.toml",
        "app",
        "0.1.0",
        concat!(
            "[dependencies]\n",
            "core = { path = \"../core\" }\n",
            "opt-a = { version = \"^1.0\", optional = true }\n",
            "opt-b = { version = \"^1.0\", optional = true }\n",
            "[features]\n",
            "default = [\"opt-a\"]\n",
            "[targets]\n",
            "supported = [\"host\", \"wasm32-wasi\"]\n",
        ),
    );

    let ws = Workspace::discover(&root).unwrap();
    let sum_a1 = ContentAddress::of_bytes(b"opt-a-1.0.0").to_string();
    let sum_a2 = ContentAddress::of_bytes(b"opt-a-1.1.0").to_string();
    let lock_v1 = Lock {
        lock_version: LOCK_VERSION,
        registry: None,
        packages: vec![LockedPkg {
            name: "opt-a".into(),
            version: "1.0.0".into(),
            source: "registry".into(),
            checksum: Some(sum_a1),
            deps: vec![],
            dep_reqs: BTreeMap::new(),
        }],
    };
    let lock_v2 = Lock {
        lock_version: LOCK_VERSION,
        registry: None,
        packages: vec![LockedPkg {
            name: "opt-a".into(),
            version: "1.1.0".into(),
            source: "registry".into(),
            checksum: Some(sum_a2),
            deps: vec![],
            dep_reqs: BTreeMap::new(),
        }],
    };

    let lpp_path = compiler.to_str().unwrap();
    let fps1 = compute_member_fps_with_lock(&ws, lpp_path, Some(&lock_v1)).unwrap();
    assert_eq!(fps1.len(), 4);
    assert_ne!(fps1["core|host"], fps1["core|wasm32-wasi"]);
    assert_ne!(fps1["app|host"], fps1["app|wasm32-wasi"]);

    // Updating locked registry dependency opt-a invalidates app|*, leaving core|* untouched
    let fps2 = compute_member_fps_with_lock(&ws, lpp_path, Some(&lock_v2)).unwrap();
    assert_eq!(fps1["core|host"], fps2["core|host"]);
    assert_eq!(fps1["core|wasm32-wasi"], fps2["core|wasm32-wasi"]);
    assert_ne!(fps1["app|host"], fps2["app|host"]);
    assert_ne!(fps1["app|wasm32-wasi"], fps2["app|wasm32-wasi"]);

    // Missing required registry entry in lockfile must return LockParse error
    let empty_lock = Lock {
        lock_version: LOCK_VERSION,
        registry: None,
        packages: vec![],
    };
    let err = compute_member_fps_with_lock(&ws, lpp_path, Some(&empty_lock)).unwrap_err();
    assert!(matches!(err, PmError::LockParse(_)), "{err}");

    // Verify dep_key_graph produces target-aligned edges
    let graph = dep_key_graph(&ws);
    assert_eq!(graph["app|host"], vec!["core|host"]);
    assert_eq!(graph["app|wasm32-wasi"], vec!["core|wasm32-wasi"]);
    assert!(graph["core|host"].is_empty());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fingerprint_store_versioning_delta_invalidation_and_kv_mirror() {
    let root = temp_dir("fp-delta");
    let store_path = root.join("target/.keel/fingerprints.toml");

    // 1. Store persists FP_STORE_VERSION and artifact hashes
    let mut store = FingerprintStore::load(&store_path);
    assert!(store.fps().is_empty());
    store.upsert_artifact("leaf|host", "fp-leaf-1", Some("art-leaf".into()));
    store.upsert_artifact("mid|host", "fp-mid-1", Some("art-mid".into()));
    store.upsert_artifact("top|host", "fp-top-1", None);
    store.upsert("side|host", "fp-side-1");
    store.save().unwrap();

    let reloaded = FingerprintStore::load(&store_path);
    assert_eq!(reloaded.file.version, FP_STORE_VERSION);
    assert_eq!(
        reloaded.get("leaf|host").unwrap().artifact_hash.as_deref(),
        Some("art-leaf")
    );
    assert!(!reloaded.get("leaf|host").unwrap().built_at.is_empty());

    // Older store version (v1) loads as cold (empty)
    let mut stale_file = reloaded.file.clone();
    stale_file.version = 1;
    std::fs::write(&store_path, stale_file.to_toml().unwrap()).unwrap();
    assert!(FingerprintStore::load(&store_path).fps().is_empty());

    // 2. Delta & transitive invalidation over a branched DAG
    let graph: BTreeMap<String, Vec<String>> = [
        ("leaf|host".into(), vec![]),
        ("mid-a|host".into(), vec!["leaf|host".into()]),
        ("mid-b|host".into(), vec!["leaf|host".into()]),
        (
            "top|host".into(),
            vec!["mid-a|host".into(), "mid-b|host".into()],
        ),
        ("isolated|host".into(), vec![]),
    ]
    .into_iter()
    .collect();

    let stored: BTreeMap<String, String> = [
        ("leaf|host".into(), "1".into()),
        ("mid-a|host".into(), "1".into()),
        ("mid-b|host".into(), "1".into()),
        ("top|host".into(), "1".into()),
        ("isolated|host".into(), "1".into()),
        ("obsolete|host".into(), "1".into()),
    ]
    .into_iter()
    .collect();
    let current: BTreeMap<String, String> = [
        ("leaf|host".into(), "2".into()),
        ("mid-a|host".into(), "1".into()),
        ("mid-b|host".into(), "1".into()),
        ("top|host".into(), "1".into()),
        ("isolated|host".into(), "1".into()),
        ("added|host".into(), "1".into()),
    ]
    .into_iter()
    .collect();

    let d: Delta = diff(&stored, &current);
    assert!(!d.is_empty());
    assert_eq!(d.changed, vec!["leaf|host"]);
    assert_eq!(d.added, vec!["added|host"]);
    assert_eq!(d.removed, vec!["obsolete|host"]);
    assert_eq!(
        d.members(),
        ["added", "leaf", "obsolete"]
            .into_iter()
            .map(str::to_string)
            .collect::<BTreeSet<_>>()
    );

    let rebuild = invalidate(&d, &graph);
    for must_rebuild in [
        "leaf|host",
        "mid-a|host",
        "mid-b|host",
        "top|host",
        "added|host",
        "obsolete|host",
    ] {
        assert!(rebuild.contains(must_rebuild), "missing {must_rebuild}");
    }
    assert!(!rebuild.contains("isolated|host"));

    // Mirror current fingerprints into hot KV cache
    let mut kv = InMemoryKv::new();
    mirror_to_kv(&mut kv, &current);
    assert_eq!(kv.len(), current.len());
    assert_eq!(kv.get("fp/leaf|host").as_deref(), Some(b"2".as_slice()));

    // Missing compiler binary identity degrades gracefully to `#missing`
    let missing_id = lpp_identity(&root.join("no-such-compiler").to_string_lossy());
    assert!(missing_id.ends_with("#missing"), "{missing_id}");
    assert_eq!(
        hash_bytes(b"abc"),
        ContentAddress::of_bytes(b"abc").as_str()
    );

    let _ = std::fs::remove_dir_all(&root);
}

// ============================================================================
// 8. Content Addressing, DiskBlobStore Persistence/Corruption & KV Cache Eviction
// ============================================================================

#[test]
fn disk_blob_store_fanout_persistence_and_corruption_detection() {
    let root = temp_dir("blob-store");
    let payload_a = b"compiled-object-module-alpha";
    let payload_b = b"compiled-object-module-beta";

    let addr_a;
    let addr_b;
    {
        let store = DiskBlobStore::open(&root).unwrap();
        assert_eq!(store.root(), root.as_path());
        addr_a = store.insert(payload_a).unwrap();
        addr_b = store.insert(payload_b).unwrap();
        assert_ne!(addr_a, addr_b);
        // Idempotent re-insert returns the same address
        assert_eq!(store.insert(payload_a).unwrap(), addr_a);

        // Two-character fanout layout: <root>/blobs/<xx>/<rest>
        let s = addr_a.as_str();
        let expected_path = root.join("blobs").join(&s[0..2]).join(&s[2..]);
        assert!(
            expected_path.is_file(),
            "missing fanout file at {expected_path:?}"
        );
    }

    // Re-open store from disk: data persists cleanly
    let reopened = DiskBlobStore::open(&root).unwrap();
    assert!(reopened.contains(&addr_a));
    assert_eq!(reopened.fetch(&addr_a).unwrap(), payload_a);
    assert_eq!(reopened.fetch(&addr_b).unwrap(), payload_b);

    // Corrupt blob_a on disk and verify both fetch() and insert() detect ChecksumMismatch
    let sa = addr_a.as_str();
    let path_a = root.join("blobs").join(&sa[0..2]).join(&sa[2..]);
    std::fs::write(&path_a, b"corrupted-bytes").unwrap();
    assert!(matches!(
        reopened.fetch(&addr_a),
        Err(PmError::ChecksumMismatch { .. })
    ));
    assert!(matches!(
        reopened.insert(payload_a),
        Err(PmError::ChecksumMismatch { .. })
    ));

    // ContentAddress boundary checks
    assert!(ContentAddress::try_new(&"0".repeat(64)).is_ok());
    assert!(ContentAddress::try_new(&"0".repeat(63)).is_err());
    assert!(ContentAddress::try_new(&"0".repeat(65)).is_err());
    assert!(ContentAddress::try_new(&format!("{}g", "0".repeat(63))).is_err());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn kv_cache_eviction_accounting_and_durable_blob_rehydration() {
    let root = temp_dir("kv-rehydrate");
    let store = DiskBlobStore::open(&root).unwrap();
    let mut kv = create_kv(KvBackendKind::Auto).unwrap();
    assert!(kv.is_empty());
    assert_eq!(kv.stats().hit_ratio(), 0.0);

    // Insert 32 blobs into durable DiskBlobStore and index their addresses in hot KV cache
    let mut keys = Vec::new();
    for i in 0..32u32 {
        let data = format!("artifact-payload-{i:02}");
        let addr = store.insert(data.as_bytes()).unwrap();
        let key = format!("pkg:item-{i:02}");
        kv.set(&key, addr.as_str().as_bytes());
        keys.push((key, addr, data));
    }
    assert_eq!(kv.len(), 32);

    // Deterministic lexicographical key order
    let listed = kv.keys();
    let mut sorted = listed.clone();
    sorted.sort();
    assert_eq!(listed, sorted);

    // Evict even-indexed entries from hot KV cache
    for (idx, (key, _, _)) in keys.iter().enumerate() {
        if idx % 2 == 0 {
            assert!(kv.del(key));
            assert!(!kv.del(key));
        }
    }
    assert_eq!(kv.len(), 16);

    // Verify misses for evicted keys, hits for retained keys, then rehydrate from DiskBlobStore
    for (idx, (key, addr, raw)) in keys.iter().enumerate() {
        if idx % 2 == 0 {
            assert!(kv.get(key).is_none());
            // Durable store still owns the blob; rehydrate hot index
            let blob = store.fetch(addr).unwrap();
            assert_eq!(blob, raw.as_bytes());
            kv.set(key, addr.as_str().as_bytes());
        } else {
            let cached = kv.get(key).unwrap();
            assert_eq!(cached, addr.as_str().as_bytes());
        }
    }
    assert_eq!(kv.len(), 32);
    let stats = kv.stats();
    assert_eq!(stats.hits, 16);
    assert_eq!(stats.misses, 16);
    assert_eq!(stats.entries, 32);
    assert!((stats.hit_ratio() - 0.5).abs() < 1e-12);

    let _ = std::fs::remove_dir_all(&root);
}
