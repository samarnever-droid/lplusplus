//! Gate: fingerprint incremental builds + the Delta layer
//! (docs/rewrite/DELTA.md).

use std::collections::BTreeMap;
use std::path::Path;

use lpp_pm::delta::{diff, invalidate, Delta};
use lpp_pm::fingerprint::{
    compute_member_fps, hash_sources, lpp_identity, member_fingerprint, FingerprintStore,
    FingerprintStoreFile,
};
use lpp_pm::Workspace;

fn temp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("lpp-fp-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
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

fn fake_lpp(root: &Path) -> std::path::PathBuf {
    let p = root.join("lpp-bin");
    std::fs::write(&p, b"#! /bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    p
}

#[test]
fn fingerprint_changes_when_a_source_changes() {
    let root = temp("src");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/main.lpp"), "fn main() {}\n").unwrap();

    let s1 = hash_sources(&root);
    let fp1 =
        member_fingerprint("lpp", "host", "manifest", &s1, &BTreeMap::new());
    std::fs::write(root.join("src/main.lpp"), "fn main() { changed() }\n").unwrap();
    let s2 = hash_sources(&root);
    let fp2 =
        member_fingerprint("lpp", "host", "manifest", &s2, &BTreeMap::new());
    assert_ne!(fp1, fp2, "source change must change the fingerprint");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fingerprint_changes_when_the_compiler_identity_changes() {
    let root = temp("lpp");
    let bin = fake_lpp(&root);
    let bin_s = bin.to_string_lossy().to_string();
    let id1 = lpp_identity(&bin_s);

    // A different compiler path is a different identity...
    let other = root.join("lpp-other");
    std::fs::write(&other, b"#! /bin/sh\nexit 0\n").unwrap();
    assert_ne!(id1, lpp_identity(&other.to_string_lossy()));

    // ...and a rebuilt compiler (new mtime) is too.
    // (sleep past the filesystem's mtime tick — coarse FSs round to seconds)
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(&bin, b"#! /bin/sh\nexit 0  # rebuilt\n").unwrap();
    let id2 = lpp_identity(&bin_s);
    assert_ne!(id1, id2, "compiler rebuild must change the identity");

    let s = hash_sources(&root);
    let fp1 = member_fingerprint(&bin_s, "host", "m", &s, &BTreeMap::new());
    let fp2 = member_fingerprint(&other.to_string_lossy(), "host", "m", &s, &BTreeMap::new());
    assert_ne!(fp1, fp2);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_dep_change_propagates_to_every_transitive_dependent() {
    let root = temp("propagate");
    diamond(&root);
    let bin = fake_lpp(&root);
    let ws = Workspace::discover(&root).unwrap();
    let fps1 = compute_member_fps(&ws, bin.to_str().unwrap()).unwrap();

    // Change the LEAF (d) — d, b, c AND app must all get new fingerprints.
    std::fs::write(root.join("crates/d/src/main.lpp"), "fn main() { changed }\n").unwrap();
    let fps2 = compute_member_fps(&ws, bin.to_str().unwrap()).unwrap();

    for key in ["d|host", "b|host", "c|host", "app|host"] {
        assert_ne!(
            fps1.get(key),
            fps2.get(key),
            "{key} must change when its dep d changes"
        );
    }

    // ...but change ONLY app's own source: only app changes.
    std::fs::write(root.join("apps/app/src/main.lpp"), "fn main() { changed }\n").unwrap();
    let fps3 = compute_member_fps(&ws, bin.to_str().unwrap()).unwrap();
    assert_ne!(fps2.get("app|host"), fps3.get("app|host"));
    for key in ["d|host", "b|host", "c|host"] {
        assert_eq!(
            fps2.get(key),
            fps3.get(key),
            "{key} must NOT change when only app changes"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn store_round_trips_and_prunes_removed_members() {
    let root = temp("store");
    let path = root.join("target").join(".keel").join("fingerprints.toml");
    let mut store = FingerprintStore::load(&path);
    store.upsert("a|host", "fp1");
    store.upsert("b|host", "fp2");
    store.save().unwrap();

    let mut loaded = FingerprintStore::load(&path);
    assert_eq!(loaded.get("a|host").unwrap().fingerprint, "fp1");
    assert_eq!(loaded.get("b|host").unwrap().fingerprint, "fp2");

    // Prune to a world where b no longer exists.
    let current: BTreeMap<String, String> =
        [("a|host".to_string(), "fp1".to_string())].into_iter().collect();
    loaded.prune(&current);
    loaded.save().unwrap();
    let reloaded = FingerprintStore::load(&path);
    assert!(reloaded.get("a|host").is_some());
    assert!(reloaded.get("b|host").is_none(), "pruned member must be gone");

    // A corrupt file = cold (empty), never an error.
    std::fs::write(&path, "not [valid toml").unwrap();
    let cold = FingerprintStore::load(&path);
    assert!(cold.fps().is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn store_file_parses_and_serializes() {
    let mut f = FingerprintStoreFile::default();
    f.version = 1;
    f.entries.insert(
        "x|host".to_string(),
        lpp_pm::fingerprint::FingerprintEntry {
            fingerprint: "abc".into(),
            built_at: "2026-01-01T00:00:00Z".into(),
        },
    );
    let doc = f.to_toml().unwrap();
    let back = FingerprintStoreFile::parse(&doc).unwrap();
    assert_eq!(back, f);
}

#[test]
fn diff_and_invalidate_cover_the_transitive_closure() {
    // key → its deps.
    let graph: BTreeMap<String, Vec<String>> = [
        ("a|host".into(), vec!["b|host".into()]),
        ("b|host".into(), vec!["d|host".into()]),
        ("c|host".into(), vec!["d|host".into()]),
        ("d|host".into(), vec![]),
    ]
    .into_iter()
    .collect();

    let stored: BTreeMap<String, String> = [
        ("a|host".into(), "1".into()),
        ("b|host".into(), "1".into()),
        ("c|host".into(), "1".into()),
        ("d|host".into(), "1".into()),
        ("gone|host".into(), "1".into()),
    ]
    .into_iter()
    .collect();
    let current: BTreeMap<String, String> = [
        ("a|host".into(), "1".into()), // unchanged
        ("b|host".into(), "1".into()), // unchanged
        ("c|host".into(), "1".into()), // unchanged
        ("d|host".into(), "2".into()), // changed
        ("new|host".into(), "1".into()), // added
    ]
    .into_iter()
    .collect();

    let d = diff(&stored, &current);
    assert_eq!(d.changed, vec!["d|host"]);
    assert_eq!(d.added, vec!["new|host"]);
    assert_eq!(d.removed, vec!["gone|host"]);

    // Changing d rebuilds d, its dependents b and c, and a (via b).
    let rebuild = invalidate(&d, &graph);
    for k in ["a|host", "b|host", "c|host", "d|host"] {
        assert!(rebuild.contains(k), "{k} must be in the rebuild set");
    }

    // Precision: an addition with no dependents rebuilds only itself.
    let only_new = Delta {
        added: vec!["new|host".to_string()],
        ..Default::default()
    };
    assert_eq!(invalidate(&only_new, &graph), ["new|host".to_string()].into_iter().collect::<std::collections::BTreeSet<_>>());

    // An empty delta rebuilds nothing.
    let empty = Delta::default();
    assert!(empty.is_empty());
    assert!(invalidate(&empty, &graph).is_empty());
}
