//! Production integration test suite for `keel` (v0.3.0).
//!
//! Covers:
//! - `new` / `init` / `add` / `remove` / `version` (`major`/`minor`/`patch`/`set`)
//! - `tree` / `why` / `outdated` / `verify` / `update` / `install` / `fetch` / `search`
//! - `clean` / `cache` (`path`/`stats`/`clean`) / `metadata` / `list`
//! - `migrate` (`--dry-run`, `--keep-legacy`, workspace + lockfile upgrades)
//! - `workspace` (`members`, `list`, `graph`, `build`, `test`, incremental & selective)
//! - `archive` (`pack`, `unpack`, `tree_hash`, exclusions, path-traversal & link rejections)

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use flate2::Compression;
use flate2::write::GzEncoder;
use tar::{Builder, EntryType, Header};

use common::{FakeMode, install_fake_lpp, normalize_slashes};
use keel::{InvokeOutcome, invoke_from};
use lpp_pm::Registry;
use lpp_pm::index::{DepSpec, IndexEntry, VersionEntry};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn temp_dir(tag: &str) -> PathBuf {
    let seq = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("keel-v03-{tag}-{}-{seq}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap_or(dir)
}

fn invoke(dir: &Path, args: &[&str]) -> InvokeOutcome {
    invoke_from(
        std::iter::once("keel".to_string()).chain(args.iter().map(|s| (*s).to_string())),
        dir,
    )
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

fn make_ver_entry(
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
        checksum: lpp_pm::ContentAddress::of_bytes(artifact).to_string(),
        targets: vec!["host".into()],
        yanked,
    }
}

fn setup_seeded_registry(root: &Path) -> PathBuf {
    let bare = root.join("remote.git");
    run_git(
        root,
        &[
            "init",
            "--bare",
            "--initial-branch=main",
            bare.to_str().unwrap(),
        ],
    );
    let seed = root.join("seed-init");
    run_git(root, &["init", "-b", "main", seed.to_str().unwrap()]);
    run_git(&seed, &["config", "user.name", "Seeder"]);
    run_git(&seed, &["config", "user.email", "seeder@lpp.invalid"]);
    run_git(&seed, &["commit", "--allow-empty", "-m", "seed init"]);
    run_git(&seed, &["push", bare.to_str().unwrap(), "HEAD:main"]);
    let _ = std::fs::remove_dir_all(&seed);

    let pub_dir = root.join("publisher");
    let reg = Registry::new(bare.to_string_lossy().as_ref(), &pub_dir);
    reg.sync().unwrap();
    run_git(&pub_dir, &["config", "user.name", "Publisher"]);
    run_git(&pub_dir, &["config", "user.email", "pub@lpp.invalid"]);

    let packages: &[(&str, &[(&str, bool, &[u8], &[(&str, &str)])])] = &[
        ("bytes", &[("1.0.0", false, b"bytes-1.0.0", &[])]),
        (
            "json",
            &[
                ("1.0.0", false, b"json-1.0.0", &[]),
                ("1.1.0", false, b"json-1.1.0", &[]),
            ],
        ),
        (
            "http",
            &[("1.0.0", false, b"http-1.0.0", &[("bytes", "^1.0")])],
        ),
        (
            "web",
            &[(
                "1.0.0",
                false,
                b"web-1.0.0",
                &[("http", "^1.0"), ("json", "^1.0")],
            )],
        ),
        (
            "utils",
            &[("1.0.0", false, b"utils-1.0.0", &[("json", "~1.0")])],
        ),
    ];

    for (pkg_name, vers) in packages {
        let mut versions = Vec::new();
        for (v, yanked, bytes, deps) in *vers {
            versions.push(make_ver_entry(v, *yanked, bytes, deps));
            // Publish progressively so each artifact blob is committed to git
            reg.publish(
                &IndexEntry {
                    name: (*pkg_name).to_string(),
                    versions: versions.clone(),
                },
                bytes,
                &format!("publish {pkg_name} {v}"),
            )
            .unwrap();
        }
    }
    reg.push().unwrap();
    bare
}

fn republish_entry(
    root: &Path,
    bare: &Path,
    name: &str,
    versions: Vec<VersionEntry>,
    latest_artifact: &[u8],
) {
    let pub_dir = root.join(format!(
        "repub-{}",
        NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let reg = Registry::new(bare.to_string_lossy().as_ref(), &pub_dir);
    reg.sync().unwrap();
    run_git(&pub_dir, &["config", "user.name", "Republisher"]);
    run_git(&pub_dir, &["config", "user.email", "repub@lpp.invalid"]);
    reg.publish(
        &IndexEntry {
            name: name.to_string(),
            versions,
        },
        latest_artifact,
        &format!("update {name}"),
    )
    .unwrap();
    reg.push().unwrap();
}

fn make_diamond_workspace(root: &Path) {
    std::fs::create_dir_all(root).unwrap();
    std::fs::write(
        root.join("Keel.toml"),
        "[workspace]\nmembers = [\"crates/*\", \"apps/cli\"]\n",
    )
    .unwrap();
    let specs = [
        ("crates/core", "core", "1.0.0", ""),
        (
            "crates/net",
            "net",
            "1.0.0",
            "[dependencies]\ncore = { path = \"../core\" }\n",
        ),
        (
            "crates/store",
            "store",
            "1.0.0",
            "[dependencies]\ncore = { path = \"../core\" }\n",
        ),
        (
            "apps/cli",
            "cli",
            "1.0.0",
            "[dependencies]\nnet = { path = \"../../crates/net\" }\nstore = { path = \"../../crates/store\" }\n",
        ),
    ];
    for (rel, name, version, deps) in specs {
        let dir = root.join(rel);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Keel.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"{version}\"\nedition = \"2024\"\n{deps}"
            ),
        )
        .unwrap();
        std::fs::write(
            dir.join("src/main.lpp"),
            format!("def main():\n    print_str(\"{name}\")\n"),
        )
        .unwrap();
    }
}

fn parse_outdated_status(rendered: &str, package: &str) -> Option<String> {
    for line in rendered.lines() {
        let cells: Vec<&str> = line
            .split('|')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if cells.len() >= 4 && cells[0] == package {
            return Some(cells[3].to_string());
        }
    }
    None
}

// ============================================================================
// 1. Project Lifecycle: new / init / add / remove / version
// ============================================================================

#[test]
fn project_new_init_add_remove_and_legacy_guard() {
    let root = temp_dir("proj-lifecycle");

    // `keel new` and `keel create` alias
    assert_eq!(invoke(&root, &["new", "alpha-svc"]), InvokeOutcome::Success);
    assert_eq!(
        invoke(&root, &["create", "beta-svc"]),
        InvokeOutcome::Success
    );
    assert!(root.join("alpha-svc/Keel.toml").is_file());
    assert!(root.join("alpha-svc/src/main.lpp").is_file());
    assert!(root.join("beta-svc/Keel.toml").is_file());

    // Duplicate project name and invalid names fail
    assert!(matches!(
        invoke(&root, &["new", "alpha-svc"]),
        InvokeOutcome::Failure(_)
    ));
    assert!(matches!(
        invoke(&root, &["new", "Bad_Name"]),
        InvokeOutcome::Failure(_)
    ));

    // `add` with bare name and `name@req`, plus duplicate/invalid rejections
    let alpha = root.join("alpha-svc");
    assert_eq!(
        invoke(&alpha, &["add", "json@^1.0"]),
        InvokeOutcome::Success
    );
    assert_eq!(invoke(&alpha, &["add", "bytes"]), InvokeOutcome::Success);
    assert!(matches!(
        invoke(&alpha, &["add", "json@^2.0"]),
        InvokeOutcome::Failure(_)
    ));
    assert!(matches!(
        invoke(&alpha, &["add", "bad@^not-semver"]),
        InvokeOutcome::Failure(_)
    ));

    let manifest = lpp_pm::manifest::Manifest::parse(
        &std::fs::read_to_string(alpha.join("Keel.toml")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest.dependencies["json"].version(), "^1.0");
    assert_eq!(manifest.dependencies["bytes"].version(), "*");

    // `remove` existing and non-existing deps
    assert_eq!(invoke(&alpha, &["remove", "bytes"]), InvokeOutcome::Success);
    assert!(matches!(
        invoke(&alpha, &["remove", "bytes"]),
        InvokeOutcome::Failure(_)
    ));
    let after_rm = lpp_pm::manifest::Manifest::parse(
        &std::fs::read_to_string(alpha.join("Keel.toml")).unwrap(),
    )
    .unwrap();
    assert!(!after_rm.dependencies.contains_key("bytes"));
    assert!(after_rm.dependencies.contains_key("json"));

    // `init` preserves existing user code and refuses invalid directory names
    let custom = root.join("custom-proj");
    std::fs::create_dir_all(custom.join("src")).unwrap();
    std::fs::write(custom.join("src/main.lpp"), "def main():\n    print(777)\n").unwrap();
    assert_eq!(invoke(&custom, &["init"]), InvokeOutcome::Success);
    assert!(
        std::fs::read_to_string(custom.join("src/main.lpp"))
            .unwrap()
            .contains("777")
    );
    assert_eq!(invoke(&custom, &["init"]), InvokeOutcome::Success);

    let bad_dir = root.join("Invalid_Dir_Name");
    std::fs::create_dir_all(&bad_dir).unwrap();
    assert!(matches!(
        invoke(&bad_dir, &["init"]),
        InvokeOutcome::Failure(_)
    ));

    // Legacy `lpp.toml` without `Keel.toml` directs user to `migrate` on `add`
    let legacy = root.join("legacy-dir");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(
        legacy.join("lpp.toml"),
        "[package]\nname = \"legacy\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    assert!(matches!(
        invoke(&legacy, &["add", "json"]),
        InvokeOutcome::Failure(msg) if msg.contains("migrate")
    ));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn version_major_minor_patch_set_and_lock_constraint_protection() {
    let root = temp_dir("version-suite");

    // 1. Standalone project full bump progression: 0.1.0 -> 0.1.1 -> 0.2.0 -> 1.0.0 -> 3.5.0
    let solo = root.join("solo-app");
    assert_eq!(invoke(&root, &["new", "solo-app"]), InvokeOutcome::Success);
    assert_eq!(invoke(&solo, &["version"]), InvokeOutcome::Success);
    assert_eq!(
        invoke(&solo, &["version", "bump", "patch"]),
        InvokeOutcome::Success
    );
    assert_eq!(
        invoke(&solo, &["version", "bump", "minor"]),
        InvokeOutcome::Success
    );
    assert_eq!(
        invoke(&solo, &["version", "bump", "major"]),
        InvokeOutcome::Success
    );
    let m1 = lpp_pm::manifest::Manifest::parse(
        &std::fs::read_to_string(solo.join("Keel.toml")).unwrap(),
    )
    .unwrap();
    assert_eq!(m1.version(), "1.0.0");

    assert_eq!(
        invoke(&solo, &["version", "set", "v3.5"]),
        InvokeOutcome::Success
    );
    let m2 = lpp_pm::manifest::Manifest::parse(
        &std::fs::read_to_string(solo.join("Keel.toml")).unwrap(),
    )
    .unwrap();
    assert_eq!(m2.version(), "3.5.0");

    assert!(matches!(
        invoke(&solo, &["version", "set", "invalid-ver"]),
        InvokeOutcome::Failure(_)
    ));

    // 2. Workspace member bump that violates a sibling's locked `dep_reqs` must abort cleanly
    let ws = root.join("ws");
    std::fs::create_dir_all(ws.join("lib/src")).unwrap();
    std::fs::create_dir_all(ws.join("app/src")).unwrap();
    std::fs::write(
        ws.join("Keel.toml"),
        "[workspace]\nmembers = [\"lib\", \"app\"]\n",
    )
    .unwrap();
    std::fs::write(
        ws.join("lib/Keel.toml"),
        "[package]\nname = \"lib\"\nversion = \"1.2.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    std::fs::write(
        ws.join("app/Keel.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\nlib = { path = \"../lib\", version = \"^1.0\" }\n",
    )
    .unwrap();
    std::fs::write(
        ws.join("Keel.lock"),
        concat!(
            "version = 2\n\n",
            "[[packages]]\nname = \"app\"\nversion = \"0.1.0\"\nsource = \"root\"\ndeps = [\"lib\"]\n",
            "[packages.dep_reqs]\nlib = \"^1.0\"\n\n",
            "[[packages]]\nname = \"lib\"\nversion = \"1.2.0\"\nsource = \"path\"\ndeps = []\n",
        ),
    )
    .unwrap();

    // Minor bump (1.2.0 -> 1.3.0) satisfies ^1.0 and updates Keel.lock
    assert_eq!(
        invoke(&ws.join("lib"), &["version", "bump", "minor"]),
        InvokeOutcome::Success
    );
    let lock_after_minor =
        lpp_pm::Lock::parse(&std::fs::read_to_string(ws.join("Keel.lock")).unwrap()).unwrap();
    assert_eq!(lock_after_minor.package("lib").unwrap().version, "1.3.0");

    // Major bump (1.3.0 -> 2.0.0) violates app's locked `lib = "^1.0"`, leaving manifest untouched
    let err = invoke(&ws.join("lib"), &["version", "bump", "major"]);
    assert!(matches!(err, InvokeOutcome::Failure(_)), "{err:?}");
    let lib_manifest = lpp_pm::manifest::Manifest::parse(
        &std::fs::read_to_string(ws.join("lib/Keel.toml")).unwrap(),
    )
    .unwrap();
    assert_eq!(lib_manifest.version(), "1.3.0");

    let _ = std::fs::remove_dir_all(&root);
}

// ============================================================================
// 2. Registry Workflows: tree / why / outdated / update / verify / install
// ============================================================================

#[test]
fn tree_why_outdated_update_verify_and_install_end_to_end() {
    let root = temp_dir("reg-workflows");
    let bare = setup_seeded_registry(&root);
    let reg_url = bare.to_string_lossy().to_string();

    // Workspace with two members: `api` (depends on `web`) and `worker` (depends on `utils` and `api`)
    let ws = root.join("workspace");
    std::fs::create_dir_all(ws.join("crates/api/src")).unwrap();
    std::fs::create_dir_all(ws.join("crates/worker/src")).unwrap();
    std::fs::write(
        ws.join("Keel.toml"),
        "[workspace]\nmembers = [\"crates/*\"]\n",
    )
    .unwrap();
    std::fs::write(
        ws.join("crates/api/Keel.toml"),
        "[package]\nname = \"api\"\nversion = \"0.1.0\"\n[dependencies]\nweb = \"^1.0\"\n",
    )
    .unwrap();
    std::fs::write(
        ws.join("crates/api/src/main.lpp"),
        "def main():\n    print(1)\n",
    )
    .unwrap();
    std::fs::write(
        ws.join("crates/worker/Keel.toml"),
        "[package]\nname = \"worker\"\nversion = \"0.1.0\"\n[dependencies]\napi = { path = \"../api\" }\nutils = \"^1.0\"\n",
    )
    .unwrap();
    std::fs::write(
        ws.join("crates/worker/src/main.lpp"),
        "def main():\n    print(2)\n",
    )
    .unwrap();

    // Before fetch, `tree` renders unlocked dependencies cleanly and `why` reports E6023
    let unlocked_tree = keel::commands::tree::render(&ws, None).unwrap();
    assert!(unlocked_tree.contains("(unlocked)"), "{unlocked_tree}");
    assert!(
        keel::commands::diagnostics::why(&ws, "bytes")
            .unwrap_err()
            .contains("E6023")
    );

    // Fetch all & resolve workspace lockfile
    let reg = Registry::new(&reg_url, root.join("client-reg-1"));
    keel::commands::registry::fetch_all(&reg, &ws).unwrap();
    let lock =
        lpp_pm::Lock::parse(&std::fs::read_to_string(ws.join("Keel.lock")).unwrap()).unwrap();
    // utils requires `json ~1.0` and web requires `json ^1.0` -> unified on 1.0.0? Wait: 1.0.0 satisfies ~1.0, 1.1.0 does not!
    assert_eq!(lock.package("json").unwrap().version, "1.0.0");
    assert_eq!(lock.package("bytes").unwrap().version, "1.0.0");

    // `tree` full and filtered by member
    let full_tree = keel::commands::tree::render(&ws, None).unwrap();
    assert!(full_tree.contains("api v0.1.0"), "{full_tree}");
    assert!(full_tree.contains("web v1.0.0 (registry)"), "{full_tree}");
    assert!(full_tree.contains("bytes v1.0.0 (registry)"), "{full_tree}");
    assert!(full_tree.contains("api v0.1.0 (member)"), "{full_tree}");

    let api_only_tree = keel::commands::tree::render(&ws, Some("api")).unwrap();
    assert!(api_only_tree.contains("api v0.1.0"));
    assert!(!api_only_tree.contains("worker v0.1.0"));
    assert!(keel::commands::tree::render(&ws, Some("ghost")).is_err());

    // `why` succeeds on transitive dependency `bytes` and `json`, fails E6024 on unknown pkg
    keel::commands::diagnostics::why(&ws, "bytes").unwrap();
    keel::commands::diagnostics::why(&ws, "json").unwrap();
    assert!(
        keel::commands::diagnostics::why(&ws, "nonexistent")
            .unwrap_err()
            .contains("E6024")
    );

    // `verify` reports all 5 registry packages OK and skips 2 workspace members
    let reg_v = Registry::new(&reg_url, root.join("client-reg-verify"));
    let (verify_ok_txt, ok) = keel::commands::diagnostics::verify_render(&reg_v, &ws).unwrap();
    assert!(ok, "{verify_ok_txt}");
    assert!(
        verify_ok_txt.contains("5 package(s) verified OK"),
        "{verify_ok_txt}"
    );
    assert!(
        verify_ok_txt.contains("2 path/member package(s) skipped"),
        "{verify_ok_txt}"
    );

    // Publish json 1.0.5 (compatible with ~1.0) and bytes 1.2.0 + bytes 2.0.0 (major incompatible)
    republish_entry(
        &root,
        &bare,
        "json",
        vec![
            make_ver_entry("1.0.0", false, b"json-1.0.0", &[]),
            make_ver_entry("1.0.5", false, b"json-1.0.5", &[]),
            make_ver_entry("1.1.0", false, b"json-1.1.0", &[]),
        ],
        b"json-1.0.5",
    );
    republish_entry(
        &root,
        &bare,
        "bytes",
        vec![
            make_ver_entry("1.0.0", false, b"bytes-1.0.0", &[]),
            make_ver_entry("1.2.0", false, b"bytes-1.2.0", &[]),
            make_ver_entry("2.0.0", false, b"bytes-2.0.0", &[]),
        ],
        b"bytes-2.0.0",
    );
    // Ensure bytes 1.2.0 blob is also published in bare repo
    republish_entry(
        &root,
        &bare,
        "bytes",
        vec![
            make_ver_entry("1.0.0", false, b"bytes-1.0.0", &[]),
            make_ver_entry("1.2.0", false, b"bytes-1.2.0", &[]),
            make_ver_entry("2.0.0", false, b"bytes-2.0.0", &[]),
        ],
        b"bytes-1.2.0",
    );

    // `outdated` detects compatible updates for both `json` (1.0.5) and `bytes` (1.2.0, NOT 2.0.0)
    let reg_out = Registry::new(&reg_url, root.join("client-reg-outdated"));
    let outdated = keel::commands::diagnostics::outdated_render(Some(&reg_out), &ws, None).unwrap();
    assert_eq!(
        parse_outdated_status(&outdated, "json").as_deref(),
        Some("update available")
    );
    assert_eq!(
        parse_outdated_status(&outdated, "bytes").as_deref(),
        Some("update available")
    );
    assert!(
        !outdated.contains("2.0.0"),
        "incompatible major leaked: {outdated}"
    );

    // Targeted `keel update json` updates ONLY json to 1.0.5, keeping bytes pinned at 1.0.0
    let reg_upd = Registry::new(&reg_url, root.join("client-reg-upd"));
    keel::commands::update::update(Some(&reg_upd), &ws, Some("json")).unwrap();
    let lock_after_json =
        lpp_pm::Lock::parse(&std::fs::read_to_string(ws.join("Keel.lock")).unwrap()).unwrap();
    assert_eq!(lock_after_json.package("json").unwrap().version, "1.0.5");
    assert_eq!(lock_after_json.package("bytes").unwrap().version, "1.0.0");

    // Full `keel update` updates bytes to 1.2.0
    keel::commands::update::update(Some(&reg_upd), &ws, None).unwrap();
    let lock_after_all =
        lpp_pm::Lock::parse(&std::fs::read_to_string(ws.join("Keel.lock")).unwrap()).unwrap();
    assert_eq!(lock_after_all.package("bytes").unwrap().version, "1.2.0");

    // Offline `keel update` is idempotent on already locked packages
    keel::commands::update::update(None, &ws, None).unwrap();
    let lock_offline =
        lpp_pm::Lock::parse(&std::fs::read_to_string(ws.join("Keel.lock")).unwrap()).unwrap();
    assert_eq!(lock_offline, lock_after_all);

    // `keel install` succeeds idempotently against the synced registry
    let reg_inst = Registry::new(&reg_url, root.join("client-reg-install"));
    keel::commands::registry::install(Some(&reg_inst), &ws).unwrap();

    // `keel fetch` and `keel search` via CLI invoke_from
    assert_eq!(
        invoke(&ws, &["--registry", &reg_url, "search", "web"]),
        InvokeOutcome::Success
    );
    assert_eq!(
        invoke(&ws, &["--registry", &reg_url, "fetch", "bytes@1.0.0"]),
        InvokeOutcome::Success
    );

    // Tampering a blob in the shared blob store causes `verify` to detect ChecksumMismatch
    let tamper_blobs = root.join("tampered-blobs");
    let tamper_clone = root.join("client-reg-tamper");
    let reg_tamper = Registry::new(&reg_url, &tamper_clone).with_blob_store(&tamper_blobs);
    reg_tamper.sync().unwrap();
    let bytes_sum = lock_after_all.checksum("bytes").unwrap();
    // Seed the shared blob store, then corrupt the cached blob file on disk
    let (ok_bytes, _) = reg_tamper.fetch("bytes", "1.2.0").unwrap();
    assert_eq!(ok_bytes, b"bytes-1.2.0");
    let cached_blob = tamper_blobs
        .join("blobs")
        .join(&bytes_sum[0..2])
        .join(&bytes_sum[2..]);
    std::fs::write(&cached_blob, b"evil-bytes").unwrap();
    let (verify_fail_txt, fail_ok) =
        keel::commands::diagnostics::verify_render(&reg_tamper, &ws).unwrap();
    assert!(!fail_ok, "tampered blob should fail verification");
    assert!(verify_fail_txt.contains("mismatch"), "{verify_fail_txt}");

    // `keel verify` command wrapper returns Err on mismatch
    assert!(keel::commands::diagnostics::verify(&reg_tamper, &ws).is_err());

    let _ = std::fs::remove_dir_all(&root);
}

// ============================================================================
// 3. Clean, Cache, Metadata, and List
// ============================================================================

#[test]
fn clean_cache_metadata_and_list_workflows() {
    let root = temp_dir("clean-cache-meta");

    // Workspace setup with 2 members and staged `.lpp_packages`
    let ws = root.join("ws");
    make_diamond_workspace(&ws);
    std::fs::create_dir_all(ws.join("target/debug")).unwrap();
    std::fs::write(ws.join("target/debug/cli"), b"bin").unwrap();
    std::fs::create_dir_all(ws.join(".lpp_packages/json")).unwrap();
    std::fs::write(ws.join(".lpp_packages/json/lpp.toml"), "staged").unwrap();

    // `metadata` and `list` on workspace
    let discovered = lpp_pm::Workspace::discover(&ws).unwrap();
    let meta = keel::commands::metadata::render(&discovered).unwrap();
    assert!(meta.contains("virtual_workspace = true\n"), "{meta}");
    assert!(meta.contains("workspace_members = 4\n"), "{meta}");
    assert!(meta.contains("member.0.name = core\n"), "{meta}");
    assert_eq!(invoke(&ws, &["metadata"]), InvokeOutcome::Success);

    let listed = keel::commands::list::render(&discovered);
    assert!(listed.contains("cli 1.0.0\n"), "{listed}");
    assert!(listed.contains("net * [path:../../crates/net]"), "{listed}");
    assert!(
        listed.contains("store * [path:../../crates/store]"),
        "{listed}"
    );
    assert_eq!(invoke(&ws, &["list"]), InvokeOutcome::Success);

    // `clean` deletes target/ while preserving `.lpp_packages`
    assert_eq!(invoke(&ws, &["clean"]), InvokeOutcome::Success);
    assert!(!ws.join("target").exists());
    assert!(ws.join(".lpp_packages/json/lpp.toml").is_file());
    assert_eq!(invoke(&ws, &["clean"]), InvokeOutcome::Success);

    // `cache path` / `cache stats` / `cache clean` under ENV_LOCK using isolated XDG_CACHE_HOME
    let xdg_root = root.join("xdg-cache");
    let keel_cache = xdg_root.join("keel");
    std::fs::create_dir_all(keel_cache.join("content/blobs/ab")).unwrap();
    std::fs::write(keel_cache.join("content/blobs/ab/cdef"), b"cached-blob").unwrap();
    std::fs::create_dir_all(keel_cache.join("registries/sample")).unwrap();

    {
        let _guard = ENV_LOCK.lock().unwrap();
        let prev = std::env::var_os("XDG_CACHE_HOME");
        unsafe {
            std::env::set_var("XDG_CACHE_HOME", &xdg_root);
        }

        assert_eq!(keel::cache_dir(), keel_cache);
        assert_eq!(invoke(&ws, &["cache", "path"]), InvokeOutcome::Success);
        assert_eq!(
            invoke(&ws, &["cache", "stats", "--cache-backend", "memory"]),
            InvokeOutcome::Success
        );
        assert!(keel_cache.exists());
        assert_eq!(invoke(&ws, &["cache", "clean"]), InvokeOutcome::Success);
        assert!(!keel_cache.exists());
        // Second clean on empty cache succeeds cleanly
        assert_eq!(invoke(&ws, &["cache", "clean"]), InvokeOutcome::Success);

        match prev {
            Some(v) => unsafe { std::env::set_var("XDG_CACHE_HOME", v) },
            None => unsafe { std::env::remove_var("XDG_CACHE_HOME") },
        }
    }

    let _ = std::fs::remove_dir_all(&root);
}

// ============================================================================
// 4. `keel migrate` Complex Manifests & Lockfiles
// ============================================================================

#[test]
fn migrate_complex_manifest_lockfile_and_workspace_upgrades() {
    let root = temp_dir("migrate-complex");

    // 1. Complex single package with legacy `author`, `entry`, features, targets, comments, and `lpp.lock`
    let proj = root.join("complex-app");
    std::fs::create_dir_all(proj.join("src")).unwrap();
    std::fs::write(proj.join("src/main.lpp"), "def main():\n    print(42)\n").unwrap();
    let legacy_toml = r#"# App header comment
[package]
name = "complex-app"
version = "0.2.0"
author = "Samar"
entry = "src/main.lpp"
description = "Complex legacy package"
license = "MIT OR Apache-2.0"

[dependencies]
json = "^1.0"
simd = { version = "2", optional = true, features = ["avx2"] }

[features]
default = ["simd"]

[targets]
supported = ["host", "wasm32-wasi"]

[tool.custom]
owner = "platform"
"#;
    std::fs::write(proj.join("lpp.toml"), legacy_toml).unwrap();

    let json_sum = lpp_pm::ContentAddress::of_bytes(b"json-1.2.0").to_string();
    let legacy_lock = format!(
        r#"lock_version = 2
registry = "https://github.com/samarnever-droid/llppregistry.git"

[[package]]
name = "complex-app"
version = "0.2.0"
source = "root"
dependencies = ["json"]

[[package]]
name = "json"
version = "1.2.0"
source = "registry"
checksum = "{json_sum}"
dependencies = []
"#
    );
    std::fs::write(proj.join("lpp.lock"), &legacy_lock).unwrap();

    // Dry-run does not write Keel.toml or Keel.lock
    assert_eq!(
        invoke(&proj, &["migrate", "--dry-run"]),
        InvokeOutcome::Success
    );
    assert!(!proj.join("Keel.toml").exists());
    assert!(!proj.join("Keel.lock").exists());

    // Keep-legacy writes Keel.toml + Keel.lock and preserves legacy files
    assert_eq!(
        invoke(&proj, &["migrate", "--keep-legacy"]),
        InvokeOutcome::Success
    );
    assert!(proj.join("Keel.toml").is_file());
    assert!(proj.join("Keel.lock").is_file());
    assert!(proj.join("lpp.toml").is_file());
    assert!(proj.join("lpp.lock").is_file());

    let keel_toml_text = std::fs::read_to_string(proj.join("Keel.toml")).unwrap();
    assert!(
        keel_toml_text.contains("# App header comment"),
        "{keel_toml_text}"
    );
    assert!(
        keel_toml_text.contains("edition = \"2024\""),
        "{keel_toml_text}"
    );
    assert!(
        keel_toml_text.contains("authors = [\"Samar\"]"),
        "{keel_toml_text}"
    );
    assert!(
        !keel_toml_text.contains("entry = \"src/main.lpp\""),
        "{keel_toml_text}"
    );
    assert!(keel_toml_text.contains("[tool.custom]"), "{keel_toml_text}");
    let parsed_manifest = lpp_pm::manifest::Manifest::parse(&keel_toml_text).unwrap();
    assert_eq!(parsed_manifest.dependencies["json"].version(), "^1.0");
    assert!(parsed_manifest.dependencies["simd"].optional());
    assert_eq!(
        parsed_manifest.targets.as_ref().unwrap().supported,
        vec!["host", "wasm32-wasi"]
    );

    let migrated_lock =
        lpp_pm::Lock::parse(&std::fs::read_to_string(proj.join("Keel.lock")).unwrap()).unwrap();
    assert_eq!(migrated_lock.lock_version, lpp_pm::lock::LOCK_VERSION);
    assert_eq!(
        migrated_lock.package("complex-app").unwrap().deps,
        vec!["json"]
    );
    assert_eq!(migrated_lock.checksum("json"), Some(json_sum.as_str()));

    // Running migrate while both exist fails cleanly
    assert!(matches!(
        invoke(&proj, &["migrate"]),
        InvokeOutcome::Failure(msg) if msg.contains("both")
    ));

    // Clean migration removes lpp.toml and lpp.lock
    std::fs::remove_file(proj.join("Keel.toml")).unwrap();
    std::fs::remove_file(proj.join("Keel.lock")).unwrap();
    assert_eq!(invoke(&proj, &["migrate"]), InvokeOutcome::Success);
    assert!(!proj.join("lpp.toml").exists());
    assert!(!proj.join("lpp.lock").exists());
    assert!(proj.join("Keel.toml").is_file());
    assert!(proj.join("Keel.lock").is_file());

    // 2. Multi-member workspace clean migration
    let ws = root.join("legacy-ws");
    std::fs::create_dir_all(ws.join("libs/math/src")).unwrap();
    std::fs::create_dir_all(ws.join("apps/web/src")).unwrap();
    std::fs::write(
        ws.join("lpp.toml"),
        "[workspace]\nmembers = [\"libs/*\", \"apps/*\"]\n",
    )
    .unwrap();
    std::fs::write(
        ws.join("libs/math/lpp.toml"),
        "[package]\nname = \"math\"\nversion = \"0.1.0\"\nauthor = \"Team\"\n",
    )
    .unwrap();
    std::fs::write(
        ws.join("apps/web/lpp.toml"),
        "[package]\nname = \"web\"\nversion = \"0.1.0\"\n[dependencies]\nmath = { path = \"../../libs/math\" }\n",
    )
    .unwrap();
    assert_eq!(invoke(&ws, &["migrate"]), InvokeOutcome::Success);
    assert!(ws.join("Keel.toml").is_file());
    assert!(ws.join("libs/math/Keel.toml").is_file());
    assert!(ws.join("apps/web/Keel.toml").is_file());
    assert!(!ws.join("lpp.toml").exists());
    assert!(!ws.join("libs/math/lpp.toml").exists());
    assert!(!ws.join("apps/web/lpp.toml").exists());

    let discovered = lpp_pm::Workspace::discover(&ws).unwrap();
    assert_eq!(discovered.build_plan().unwrap().len(), 2);

    let _ = std::fs::remove_dir_all(&root);
}

// ============================================================================
// 5. Workspace Members, Graph, Build (Parallel/Incremental/Selective) & Test
// ============================================================================

#[test]
fn workspace_members_graph_incremental_selective_build_and_tests() {
    let root = temp_dir("ws-build-test");
    let ws = root.join("monorepo");
    make_diamond_workspace(&ws);

    // `workspace members`, `workspace list`, `workspace graph` via CLI
    assert_eq!(
        invoke(&ws, &["workspace", "members"]),
        InvokeOutcome::Success
    );
    assert_eq!(invoke(&ws, &["workspace", "list"]), InvokeOutcome::Success);
    assert_eq!(invoke(&ws, &["workspace", "graph"]), InvokeOutcome::Success);

    let discovered = keel::commands::workspace::discover(&ws).unwrap();
    assert!(keel::commands::workspace::require_member(&discovered, "net").is_ok());
    assert!(keel::commands::workspace::require_member(&discovered, "nope").is_err());
    let graph_text = keel::commands::workspace::render_graph(&discovered);
    assert!(graph_text.contains("cli -> net, store"), "{graph_text}");
    assert!(graph_text.contains("core -> (none)"), "{graph_text}");

    // 1. Incremental build with fake lpp compiler
    let log = root.join("inc.log");
    let fake_inc = install_fake_lpp(&root, FakeMode::Incremental, &log);
    let kv = Some(std::sync::Arc::new(std::sync::Mutex::new(
        lpp_pm::create_kv(lpp_pm::KvBackendKind::Auto).unwrap(),
    )));

    keel::commands::build::build_incremental(&ws, fake_inc.to_str().unwrap(), kv.clone(), None)
        .unwrap();
    let first_log = std::fs::read_to_string(&log).unwrap();
    assert_eq!(
        first_log.lines().count(),
        4,
        "all 4 members built once:\n{first_log}"
    );

    // Second incremental build with no source changes -> 0 new invocations!
    keel::commands::build::build_incremental(&ws, fake_inc.to_str().unwrap(), kv.clone(), None)
        .unwrap();
    let second_log = std::fs::read_to_string(&log).unwrap();
    assert_eq!(
        second_log.lines().count(),
        4,
        "unchanged workspace must skip all rebuilds:\n{second_log}"
    );

    // Mutate `crates/net` ONLY -> `net` and its dependent `cli` rebuild; `core` and `store` stay cached!
    std::fs::write(
        ws.join("crates/net/src/main.lpp"),
        "def main():\n    print_str(\"net-v2\")\n",
    )
    .unwrap();
    keel::commands::build::build_incremental(&ws, fake_inc.to_str().unwrap(), kv.clone(), None)
        .unwrap();
    let third_log = std::fs::read_to_string(&log).unwrap();
    let new_lines: Vec<&str> = third_log.lines().skip(4).collect();
    assert_eq!(
        new_lines.len(),
        2,
        "only net + cli should rebuild:\n{third_log}"
    );
    let norm_new = normalize_slashes(&new_lines.join("\n"));
    assert!(norm_new.contains("crates/net/src/main.lpp"), "{norm_new}");
    assert!(norm_new.contains("apps/cli/src/main.lpp"), "{norm_new}");

    // 2. Selective `build_incremental_selected` for `net` after clean builds only `core` + `net`
    keel::commands::clean::clean(&ws).unwrap();
    let sel_log = root.join("sel.log");
    let fake_sel = install_fake_lpp(&root, FakeMode::Incremental, &sel_log);
    keel::commands::build::build_incremental_selected(
        &ws,
        fake_sel.to_str().unwrap(),
        kv,
        None,
        Some("net"),
    )
    .unwrap();
    let sel_lines = normalize_slashes(&std::fs::read_to_string(&sel_log).unwrap());
    assert_eq!(sel_lines.lines().count(), 2, "{sel_lines}");
    assert!(sel_lines.contains("crates/core/src/main.lpp"));
    assert!(sel_lines.contains("crates/net/src/main.lpp"));
    assert!(!sel_lines.contains("crates/store/src/main.lpp"));
    assert!(!sel_lines.contains("apps/cli/src/main.lpp"));

    // 3. Parallel layer overlap verification for `net` and `store`
    let marks_log = root.join("marks.log");
    let fake_slow = install_fake_lpp(&root, FakeMode::SlowMarks, &marks_log);
    keel::commands::build::build(
        &ws,
        fake_slow.to_str().unwrap(),
        &|m, cwd, l, r| keel::commands::build::run_lpp_jobs(m, cwd, l, r),
        None,
    )
    .unwrap();
    let marks = normalize_slashes(&std::fs::read_to_string(&marks_log).unwrap());
    let start_net = marks
        .lines()
        .position(|l| l.ends_with("crates/net/src/main.lpp") && l.starts_with("START"))
        .unwrap();
    let end_net = marks
        .lines()
        .position(|l| l.ends_with("crates/net/src/main.lpp") && l.starts_with("END"))
        .unwrap();
    let start_store = marks
        .lines()
        .position(|l| l.ends_with("crates/store/src/main.lpp") && l.starts_with("START"))
        .unwrap();
    let end_store = marks
        .lines()
        .position(|l| l.ends_with("crates/store/src/main.lpp") && l.starts_with("END"))
        .unwrap();
    assert!(
        start_net.max(start_store) < end_net.min(end_store),
        "net and store must run concurrently:\n{marks}"
    );

    // 4. `workspace test` across all members and selective member, plus failure reporting
    std::fs::create_dir_all(ws.join("crates/core/tests")).unwrap();
    std::fs::create_dir_all(ws.join("crates/net/tests")).unwrap();
    std::fs::write(
        ws.join("crates/core/tests/t_core.lpp"),
        "def main():\n    return\n",
    )
    .unwrap();
    std::fs::write(
        ws.join("crates/net/tests/t_net.lpp"),
        "def main():\n    return\n",
    )
    .unwrap();

    let test_log = root.join("test.log");
    let fake_test = install_fake_lpp(&root, FakeMode::TestRunner, &test_log);
    keel::commands::test::test_run_selected(&ws, fake_test.to_str().unwrap(), None, None).unwrap();
    let ran_all = normalize_slashes(&std::fs::read_to_string(&test_log).unwrap());
    assert!(ran_all.contains("crates/core/tests/t_core.lpp"));
    assert!(ran_all.contains("crates/net/tests/t_net.lpp"));

    std::fs::write(
        ws.join("crates/net/tests/t_net_fail.lpp"),
        "def main():\n    return\n",
    )
    .unwrap();
    // Selected `core` still passes
    keel::commands::test::test_run_selected(&ws, fake_test.to_str().unwrap(), None, Some("core"))
        .unwrap();
    // Selected `net` fails with summary
    let fail_err = keel::commands::test::test_run_selected(
        &ws,
        fake_test.to_str().unwrap(),
        None,
        Some("net"),
    )
    .unwrap_err();
    assert!(fail_err.contains("t_net_fail.lpp"), "{fail_err}");
    assert!(fail_err.contains("1 passed, 1 failed"), "{fail_err}");

    let _ = std::fs::remove_dir_all(&root);
}

// ============================================================================
// 6. Archive Creation, Determinism, Exclusions & Path-Traversal Rejection
// ============================================================================

fn build_hostile_tar_gz(entries: &[(&[u8], EntryType, Option<&str>, &[u8])]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut archive = Builder::new(&mut encoder);
        for (raw_path, entry_type, link_target, body) in entries {
            let mut header = Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            header.set_entry_type(*entry_type);
            if let Some(target) = link_target {
                header.set_link_name(target).unwrap();
            }
            let raw = header.as_mut_bytes();
            raw[..100].fill(0);
            let len = raw_path.len().min(100);
            raw[..len].copy_from_slice(&raw_path[..len]);
            header.set_cksum();
            archive.append(&header, *body).unwrap();
        }
        archive.finish().unwrap();
    }
    encoder.finish().unwrap()
}

#[test]
fn archive_pack_exclusions_tree_hash_and_security_rejections() {
    let root = temp_dir("archive-suite");
    let pkg = root.join("pkg");
    std::fs::create_dir_all(pkg.join("src/sub")).unwrap();
    std::fs::create_dir_all(pkg.join(".git")).unwrap();
    std::fs::create_dir_all(pkg.join("target/debug")).unwrap();
    std::fs::create_dir_all(pkg.join(".lpp_packages/dep")).unwrap();

    std::fs::write(
        pkg.join("Keel.toml"),
        "[package]\nname = \"demo\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    std::fs::write(pkg.join("src/main.lpp"), "def main():\n    print(1)\n").unwrap();
    std::fs::write(
        pkg.join("src/sub/mod.lpp"),
        "def x() -> Int:\n    return 2\n",
    )
    .unwrap();

    // Sensitive & build-generated paths that MUST be excluded from archives
    std::fs::write(pkg.join(".git/config"), "secret").unwrap();
    std::fs::write(pkg.join("target/debug/out"), "bin").unwrap();
    std::fs::write(pkg.join(".lpp_packages/dep/lpp.toml"), "staged").unwrap();
    std::fs::write(pkg.join("Keel.lock"), "version = 2\npackages = []\n").unwrap();
    std::fs::write(pkg.join(".env"), "API_KEY=secret").unwrap();
    std::fs::write(pkg.join(".env.production"), "TOKEN=secret").unwrap();

    let packed_a = keel::commands::archive::pack(&pkg).unwrap();
    let packed_b = keel::commands::archive::pack(&pkg).unwrap();
    assert_eq!(packed_a, packed_b, "pack must be byte-deterministic");

    let dest = root.join("unpacked");
    keel::commands::archive::unpack(&packed_a, &dest).unwrap();
    assert!(dest.join("Keel.toml").is_file());
    assert!(dest.join("src/main.lpp").is_file());
    assert!(dest.join("src/sub/mod.lpp").is_file());
    assert!(!dest.join(".git").exists());
    assert!(!dest.join("target").exists());
    assert!(!dest.join(".lpp_packages").exists());
    assert!(!dest.join("Keel.lock").exists());
    assert!(!dest.join(".env").exists());
    assert!(!dest.join(".env.production").exists());

    // `tree_hash` ignores generated staging metadata (`lpp.toml` and `.keel-stage`)
    let hash_clean = keel::commands::archive::tree_hash(&dest).unwrap();
    std::fs::write(dest.join("lpp.toml"), "[package]\nname = \"demo\"\n").unwrap();
    std::fs::write(dest.join(".keel-stage"), "staged\n").unwrap();
    let hash_with_stage = keel::commands::archive::tree_hash(&dest).unwrap();
    assert_eq!(hash_clean, hash_with_stage);

    // Modifying a real source file changes `tree_hash`
    std::fs::write(dest.join("src/main.lpp"), "def main():\n    print(99)\n").unwrap();
    let hash_changed = keel::commands::archive::tree_hash(&dest).unwrap();
    assert_ne!(hash_clean, hash_changed);

    // Hostile archives that MUST be rejected without writing outside destination
    let parent_traversal =
        build_hostile_tar_gz(&[(b"../escape.txt", EntryType::Regular, None, b"pwned")]);
    assert!(
        keel::commands::archive::unpack(&parent_traversal, &root.join("out1"))
            .unwrap_err()
            .contains("unsafe archive path")
    );
    assert!(!root.join("escape.txt").exists());

    let nested_traversal =
        build_hostile_tar_gz(&[(b"src/../../escape2.txt", EntryType::Regular, None, b"pwned")]);
    assert!(
        keel::commands::archive::unpack(&nested_traversal, &root.join("out2"))
            .unwrap_err()
            .contains("unsafe archive path")
    );
    assert!(!root.join("escape2.txt").exists());

    let absolute_path =
        build_hostile_tar_gz(&[(b"/etc/passwd", EntryType::Regular, None, b"root")]);
    assert!(
        keel::commands::archive::unpack(&absolute_path, &root.join("out3"))
            .unwrap_err()
            .contains("unsafe archive path")
    );

    let duplicate_entry = build_hostile_tar_gz(&[
        (b"src/main.lpp", EntryType::Regular, None, b"a"),
        (b"src/main.lpp", EntryType::Regular, None, b"b"),
    ]);
    assert!(
        keel::commands::archive::unpack(&duplicate_entry, &root.join("out4"))
            .unwrap_err()
            .contains("duplicate archive path")
    );

    let symlink_entry =
        build_hostile_tar_gz(&[(b"src/link", EntryType::Symlink, Some("/etc/passwd"), b"")]);
    assert!(
        keel::commands::archive::unpack(&symlink_entry, &root.join("out5"))
            .unwrap_err()
            .contains("unsupported archive entry type")
    );

    let hardlink_entry =
        build_hostile_tar_gz(&[(b"src/hard", EntryType::Link, Some("src/main.lpp"), b"")]);
    assert!(
        keel::commands::archive::unpack(&hardlink_entry, &root.join("out6"))
            .unwrap_err()
            .contains("unsupported archive entry type")
    );

    // Pack rejects package missing Keel.toml or src/, and rejects symlinks inside package
    let no_src = root.join("no-src");
    std::fs::create_dir_all(&no_src).unwrap();
    std::fs::write(
        no_src.join("Keel.toml"),
        "[package]\nname = \"x\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    assert!(keel::commands::archive::pack(&no_src).is_err());

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(pkg.join("Keel.toml"), pkg.join("src/sym.lpp")).unwrap();
        let sym_err = keel::commands::archive::pack(&pkg).unwrap_err();
        assert!(sym_err.contains("symlink"), "{sym_err}");
    }

    let _ = std::fs::remove_dir_all(&root);
}
