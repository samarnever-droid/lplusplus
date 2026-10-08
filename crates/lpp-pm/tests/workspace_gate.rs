//! Gate: the workspace model — discovery, member loading, glob expansion,
//! the topological build plan, and union (workspace) resolution.

use std::path::Path;

use lpp_pm::{Candidate, Pkg, Req, Version, Workspace, resolve_workspace};

fn temp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("lpp-ws-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.canonicalize().unwrap_or(d)
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

fn names<'a>(layers: &'a [Vec<usize>], ws: &'a Workspace) -> Vec<Vec<&'a str>> {
    layers
        .iter()
        .map(|l| l.iter().map(|&i| ws.members[i].name()).collect())
        .collect()
}

#[test]
fn standalone_package_is_a_one_member_workspace() {
    let root = temp("solo");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("Keel.toml"), pkg_manifest("solo", "")).unwrap();

    let ws = Workspace::discover(&root).unwrap();
    assert_eq!(ws.members.len(), 1);
    assert_eq!(ws.members[0].name(), "solo");
    assert!(!ws.virtual_root);
    // A single member keeps its own target/ (no shared root target/).
    assert_eq!(ws.out_dir(), root.join("target"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn virtual_root_loads_its_members_in_declarative_order() {
    let root = temp("virtual");
    diamond(&root);
    let ws = Workspace::discover(&root).unwrap();
    assert!(ws.virtual_root);
    assert_eq!(
        ws.members.iter().map(|m| m.name()).collect::<Vec<_>>(),
        vec!["b", "c", "d", "app"] // crates/* glob (sorted) then apps/app
    );
    // Multi-member workspaces share the root target/.
    assert_eq!(ws.out_dir(), root.join("target"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn discover_walks_up_from_a_submember() {
    let root = temp("submember");
    diamond(&root);
    let ws = Workspace::discover(&root.join("apps/app")).unwrap();
    assert_eq!(ws.root, root);
    assert_eq!(ws.members.len(), 4);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn missing_member_manifest_is_typed_error() {
    let root = temp("missing");
    std::fs::write(
        root.join("Keel.toml"),
        "[workspace]\nmembers = [\"crates/ghost\"]\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("crates/ghost/src")).unwrap();
    let err = Workspace::discover(&root).unwrap_err();
    assert!(
        matches!(err, lpp_pm::PmError::MemberNotFound { .. }),
        "got {err}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn duplicate_member_names_rejected() {
    let root = temp("dup");
    std::fs::write(
        root.join("Keel.toml"),
        "[workspace]\nmembers = [\"a\", \"b\"]\n",
    )
    .unwrap();
    for d in ["a", "b"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
        std::fs::write(root.join(d).join("Keel.toml"), pkg_manifest("same", "")).unwrap();
    }
    let err = Workspace::discover(&root).unwrap_err();
    assert!(
        matches!(err, lpp_pm::PmError::DuplicateMember { .. }),
        "got {err}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn bad_member_pattern_rejected() {
    let root = temp("pattern");
    std::fs::write(
        root.join("Keel.toml"),
        "[workspace]\nmembers = [\"crates/**/x\"]\n",
    )
    .unwrap();
    let err = Workspace::discover(&root).unwrap_err();
    assert!(
        matches!(err, lpp_pm::PmError::BadMemberPattern(_)),
        "got {err}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn build_plan_orders_the_diamond_by_layers() {
    let root = temp("plan");
    diamond(&root);
    let ws = Workspace::discover(&root).unwrap();
    let plan = ws.build_plan().unwrap();
    let layers = names(&plan, &ws);
    assert_eq!(
        layers.len(),
        3,
        "deps first: [[d], [b, c], [app]]\n{layers:?}"
    );
    assert_eq!(layers[0], vec!["d"]);
    assert_eq!(layers[1], vec!["b", "c"]); // deterministic: sorted within a layer
    assert_eq!(layers[2], vec!["app"]);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn build_plan_rejects_cycles() {
    let root = temp("cycle");
    std::fs::write(
        root.join("Keel.toml"),
        "[workspace]\nmembers = [\"a\", \"b\"]\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("a")).unwrap();
    std::fs::create_dir_all(root.join("b")).unwrap();
    std::fs::write(
        root.join("a").join("Keel.toml"),
        pkg_manifest("a", "[dependencies]\nb = { path = \"../b\" }"),
    )
    .unwrap();
    std::fs::write(
        root.join("b").join("Keel.toml"),
        pkg_manifest("b", "[dependencies]\na = { path = \"../a\" }"),
    )
    .unwrap();
    let ws = Workspace::discover(&root).unwrap();
    let err = ws.build_plan().unwrap_err();
    assert!(
        matches!(err, lpp_pm::PmError::WorkspaceCycle { .. }),
        "got {err}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn path_dep_outside_the_workspace_is_rejected_at_plan_time() {
    let root = temp("outside");
    // A standalone package (its own 1-member workspace) with a path dep on
    // a SIBLING that is NOT a member.
    std::fs::create_dir_all(root.join("solo/src")).unwrap();
    std::fs::create_dir_all(root.join("sibling")).unwrap();
    std::fs::write(
        root.join("sibling").join("Keel.toml"),
        pkg_manifest("sibling", ""),
    )
    .unwrap();
    std::fs::write(
        root.join("solo").join("Keel.toml"),
        pkg_manifest(
            "solo",
            "[dependencies]\nsibling = { path = \"../sibling\" }",
        ),
    )
    .unwrap();
    let ws = Workspace::discover(&root.join("solo")).unwrap();
    let err = ws.build_plan().unwrap_err();
    assert!(
        matches!(err, lpp_pm::PmError::PathDepOutsideWorkspace { .. }),
        "got {err}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn resolve_workspace_unifies_compatible_requirements() {
    let available = |_name: &str| {
        Some(vec![
            Candidate {
                version: Version::new(1, 0, 0),
                checksum: None,
                source: "registry".into(),
                deps: vec![],
            },
            Candidate {
                version: Version::new(1, 2, 0),
                checksum: None,
                source: "registry".into(),
                deps: vec![],
            },
        ])
    };
    // Two members: one wants ^1.0, the other ^1.1 → unify on 1.2.0.
    let roots = vec![
        Pkg {
            name: "m1".into(),
            version: Version::new(1, 0, 0),
            checksum: None,
            source: "root".into(),
            deps: vec![("math".into(), Req::parse("^1.0").unwrap())],
        },
        Pkg {
            name: "m2".into(),
            version: Version::new(1, 0, 0),
            checksum: None,
            source: "path".into(),
            deps: vec![("math".into(), Req::parse("^1.1").unwrap())],
        },
    ];
    let resolved = resolve_workspace(&roots, &available).unwrap();
    assert!(resolved.get("m1").is_some());
    assert!(resolved.get("m2").is_some());
    assert_eq!(resolved.get("math").unwrap().version, Version::new(1, 2, 0));
    assert!(
        resolved.get("<workspace>").is_none(),
        "synthetic root must not leak"
    );
}

#[test]
fn path_dep_parses_without_a_version() {
    let doc = r#"
[package]
name = "app"
version = "0.1.0"

[dependencies]
math = { path = "../crates/math" }
simdlib = { version = "2", features = ["fast"] }
"#;
    let m = lpp_pm::manifest::Manifest::parse(doc).unwrap();
    assert_eq!(
        m.dependencies.get("math").unwrap().path(),
        Some("../crates/math")
    );
    // No version → any-version requirement.
    assert_eq!(m.dependencies.get("math").unwrap().version(), "*");
    assert_eq!(m.dependencies.get("simdlib").unwrap().path(), None);
    assert_eq!(m.dependencies.get("simdlib").unwrap().version(), "2");
}

#[test]
fn resolve_workspace_reports_incompatible_requirements() {
    let available = |name: &str| {
        (name == "math").then(|| {
            vec![
                Candidate {
                    version: Version::new(1, 0, 0),
                    checksum: None,
                    source: "registry".into(),
                    deps: vec![],
                },
                Candidate {
                    version: Version::new(2, 0, 0),
                    checksum: None,
                    source: "registry".into(),
                    deps: vec![],
                },
            ]
        })
    };
    let roots = vec![
        Pkg {
            name: "m1".into(),
            version: Version::new(1, 0, 0),
            checksum: None,
            source: "root".into(),
            deps: vec![("math".into(), Req::parse("^1.0").unwrap())],
        },
        Pkg {
            name: "m2".into(),
            version: Version::new(1, 0, 0),
            checksum: None,
            source: "path".into(),
            deps: vec![("math".into(), Req::parse("^2.0").unwrap())],
        },
    ];
    let err = resolve_workspace(&roots, &available).unwrap_err();
    assert!(
        matches!(err, lpp_pm::PmError::ResolveConflict { .. }),
        "got {err}"
    );
}
