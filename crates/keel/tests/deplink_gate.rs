//! Gate: path-dep staging into `.lpp_packages` (docs/rewrite/DEP_LINKING.md)
//! — `import <dep>` resolves via lpp's existing module resolver, lpp runs
//! with cwd = the workspace root, staging is idempotent and prunes only
//! Keel-managed entries.

use std::path::Path;

#[cfg(unix)]
fn make_executable(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(not(unix))]
fn make_executable(_p: &Path) {}

fn temp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("lpp-keel-deplink-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn pkg_manifest(name: &str, deps: &str) -> String {
    format!("[package]\nname = \"{name}\"\nversion = \"1.0.0\"\n{deps}\n")
}

/// A fake `lpp` that records its cwd and args per invocation.
fn fake_lpp(root: &Path) -> std::path::PathBuf {
    let p = root.join("lpp-fake");
    std::fs::write(
        &p,
        format!(
            "#!/bin/sh\necho \"PWD $(pwd)\" >> {}\necho \"ARGS $*\" >> {}\n",
            root.join("marks.txt").display(),
            root.join("marks.txt").display()
        ),
    )
    .unwrap();
    make_executable(&p);
    p
}

/// app (binary) → calc (lib) path dep.
fn app_and_calc(root: &Path) {
    std::fs::write(
        root.join("Keel.toml"),
        "[workspace]\nmembers = [\"packages/*\"]\n",
    )
    .unwrap();
    for (dir, name, deps) in [
        (
            "packages/calc",
            "calc",
            "",
        ),
        (
            "packages/app",
            "app",
            "[dependencies]\ncalc = { path = \"../calc\" }",
        ),
    ] {
        let p = root.join(dir);
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(p.join("Keel.toml"), pkg_manifest(name, deps)).unwrap();
    }
    std::fs::write(root.join("packages/calc/src/lib.lpp"), "def square(x: Int) -> Int:\n    return x * x\n").unwrap();
    std::fs::write(root.join("packages/app/src/main.lpp"), "def main():\n    print(1)\n").unwrap();
}

fn build(root: &Path, fake: &Path) {
    keel::commands::build::build(root, fake.to_str().unwrap(), &|m, cwd, l, r| {
        keel::commands::build::run_lpp_jobs(m, cwd, l, r)
    }, None)
    .expect("build should succeed");
}

#[test]
fn path_dep_is_staged_and_lpp_runs_from_the_root() {
    let root = temp("stage");
    app_and_calc(&root);
    let fake = fake_lpp(&root);

    build(&root, &fake);

    // The staged manifest: entry = the dep's lib entry, managed by keel.
    let doc = std::fs::read_to_string(root.join(".lpp_packages/calc/lpp.toml")).unwrap();
    assert!(doc.contains("entry = \"src/lib.lpp\""), "{doc}");
    assert!(doc.contains("managed = \"keel\""), "{doc}");
    // The src symlink points at the member's real src dir.
    let link = root.join(".lpp_packages/calc/src");
    #[cfg(unix)]
    assert_eq!(
        std::fs::read_link(&link).unwrap(),
        root.join("packages/calc/src")
    );
    // lpp ran with cwd = the workspace root.
    let marks = std::fs::read_to_string(root.join("marks.txt")).unwrap();
    assert!(
        marks.contains(&format!("PWD {}", root.display())),
        "lpp must run from the workspace root:\n{marks}"
    );
    // ...and the dep still builds before the dependent.
    let calc = marks.lines().position(|l| l.contains("packages/calc/src/lib.lpp")).unwrap();
    let app = marks.lines().position(|l| l.contains("packages/app/src/main.lpp")).unwrap();
    assert!(calc < app, "calc must build before app:\n{marks}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn stale_managed_entries_are_pruned_when_the_dep_goes_away() {
    let root = temp("prune");
    app_and_calc(&root);
    let fake = fake_lpp(&root);

    build(&root, &fake);
    assert!(root.join(".lpp_packages/calc").exists());

    // Remove the dep from app's manifest and rebuild.
    std::fs::write(
        root.join("packages/app/Keel.toml"),
        pkg_manifest("app", ""),
    )
    .unwrap();
    build(&root, &fake);
    assert!(
        !root.join(".lpp_packages/calc").exists(),
        "stale managed entry must be pruned"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn foreign_lpp_packages_entries_are_never_touched() {
    let root = temp("foreign");
    app_and_calc(&root);
    let fake = fake_lpp(&root);

    // A hand-rolled (or old-PM) package dir WITHOUT the keel marker.
    let foreign = root.join(".lpp_packages/foreignpkg");
    std::fs::create_dir_all(foreign.join("src")).unwrap();
    std::fs::write(
        foreign.join("lpp.toml"),
        "[package]\nname = \"foreignpkg\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(foreign.join("src/foreignpkg.lpp"), "fn f() {}\n").unwrap();

    build(&root, &fake);
    assert!(
        root.join(".lpp_packages/foreignpkg").exists(),
        "unmanaged .lpp_packages entries must survive"
    );
    assert!(root.join(".lpp_packages/calc").exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn transitive_path_deps_are_staged_too() {
    let root = temp("transitive");
    // app → mid → leaf
    std::fs::write(
        root.join("Keel.toml"),
        "[workspace]\nmembers = [\"packages/*\"]\n",
    )
    .unwrap();
    for (dir, name, deps) in [
        ("packages/leaf", "leaf", ""),
        (
            "packages/mid",
            "mid",
            "[dependencies]\nleaf = { path = \"../leaf\" }",
        ),
        (
            "packages/app",
            "app",
            "[dependencies]\nmid = { path = \"../mid\" }",
        ),
    ] {
        let p = root.join(dir);
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(p.join("Keel.toml"), pkg_manifest(name, deps)).unwrap();
        if name == "app" {
            std::fs::write(p.join("src/main.lpp"), "fn main() {}\n").unwrap();
        } else {
            std::fs::write(p.join("src/lib.lpp"), "fn f() {}\n").unwrap();
        }
    }
    let fake = fake_lpp(&root);
    build(&root, &fake);
    // Both mid (app's dep) and leaf (mid's dep) are staged — leaf is needed
    // when mid itself is compiled.
    assert!(root.join(".lpp_packages/mid/lpp.toml").exists());
    assert!(root.join(".lpp_packages/leaf/lpp.toml").exists());
    let _ = std::fs::remove_dir_all(&root);
}
