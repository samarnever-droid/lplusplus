//! Gate: `keel build` incremental builds (fingerprint skip) and
//! `keel cache clean` (docs/rewrite/DELTA.md).

use std::path::Path;

#[cfg(unix)]
fn make_executable(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(not(unix))]
fn make_executable(_p: &Path) {}

fn temp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("lpp-keel-incr-{tag}-{}", std::process::id()));
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

/// A fake `lpp`: records every invocation and creates its `-o` artifact.
fn fake_lpp(root: &Path) -> std::path::PathBuf {
    let p = root.join("lpp-fake");
    std::fs::write(
        &p,
        format!(
            "#!/bin/sh\necho \"INVOKE $*\" >> {}\n\nout=\"\"\nprev=\"\"\nfor a in \"$@\"; do\n  if [ \"$prev\" = \"-o\" ]; then out=\"$a\"; fi\n  prev=\"$a\"\ndone\nif [ -n \"$out\" ]; then mkdir -p \"$(dirname \"$out\")\" && echo built > \"$out\"; fi\n",
            root.join("args.txt").display()
        ),
    )
    .unwrap();
    make_executable(&p);
    p
}

fn build(root: &Path, fake: &Path) {
    keel::commands::build::build_incremental(root, fake.to_str().unwrap(), None, None)
        .expect("build should succeed");
}

fn invocation_count(root: &Path) -> usize {
    match std::fs::read_to_string(root.join("args.txt")) {
        Ok(s) => s.lines().filter(|l| l.starts_with("INVOKE")).count(),
        Err(_) => 0,
    }
}

#[test]
fn unchanged_workspace_is_fully_cached_on_rebuild() {
    let root = temp("cache");
    diamond(&root);
    let fake = fake_lpp(&root);

    build(&root, &fake);
    let first = invocation_count(&root);
    assert_eq!(first, 4, "first build compiles all 4 members");
    assert!(root.join("target").join("app").exists(), "artifact exists");

    // Nothing changed: rebuild invokes lpp ZERO times.
    build(&root, &fake);
    assert_eq!(
        invocation_count(&root),
        first,
        "unchanged workspace must not re-invoke lpp"
    );
    // The durable store exists at the shared workspace target/.
    assert!(
        root.join("target/.keel/fingerprints.toml").exists(),
        "fingerprint store must be durable"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn deleting_an_artifact_forces_only_that_rebuild() {
    let root = temp("art");
    diamond(&root);
    let fake = fake_lpp(&root);

    build(&root, &fake);
    let after_first = invocation_count(&root);

    // Simulate a broken/missing artifact: fingerprint matches, file gone.
    std::fs::remove_file(root.join("target").join("b")).unwrap();
    build(&root, &fake);
    assert_eq!(
        invocation_count(&root),
        after_first + 1,
        "only the missing artifact's member re-builds"
    );
    assert!(root.join("target").join("b").exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn changing_a_leaf_rebuilds_its_whole_dependent_cone() {
    let root = temp("cone");
    diamond(&root);
    let fake = fake_lpp(&root);

    build(&root, &fake);
    let base = invocation_count(&root);

    // Change the leaf d: d, b, c AND app must rebuild (4).
    std::fs::write(root.join("crates/d/src/main.lpp"), "fn main() { changed }\n").unwrap();
    build(&root, &fake);
    assert_eq!(
        invocation_count(&root),
        base + 4,
        "d's dependent cone is the whole diamond"
    );

    // Now change only app: exactly 1 rebuild.
    std::fs::write(root.join("apps/app/src/main.lpp"), "fn main() { changed2 }\n").unwrap();
    build(&root, &fake);
    assert_eq!(
        invocation_count(&root),
        base + 5,
        "only app rebuilds when only app changes"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn cache_clean_removes_the_global_cache() {
    // Isolate the cache dir for this test (no other keel test reads it).
    let cache = temp("xdg");
    // SAFETY: tests in this binary run in one process and no other test
    // touches XDG_CACHE_HOME.
    unsafe {
        std::env::set_var("XDG_CACHE_HOME", &cache);
    }
    let keel_cache = cache.join("keel");
    std::fs::create_dir_all(keel_cache.join("registry")).unwrap();
    std::fs::create_dir_all(keel_cache.join("blob")).unwrap();
    std::fs::write(keel_cache.join("registry").join("index.json"), b"{}").unwrap();
    std::fs::write(keel_cache.join("blob").join("abc"), b"blob-bytes").unwrap();

    let res = keel::commands::cache::run(&keel::cli::CacheAction::Clean, keel::cli::CacheBackend::Auto);
    unsafe {
        std::env::remove_var("XDG_CACHE_HOME");
    }
    assert!(res.is_ok(), "cache clean should succeed: {res:?}");
    assert!(!keel_cache.exists(), "the global cache dir must be removed");
    let _ = std::fs::remove_dir_all(&cache);
}
