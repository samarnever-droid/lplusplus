//! Gate: the semver matcher, the dependency resolver, and the `Keel.lock` model.

use lpp_pm::lock::Lock;
use lpp_pm::resolve::{Candidate, Pkg, resolve};
use lpp_pm::semver::{Req, Version};

fn v(s: &str) -> Version {
    Version::parse(s).unwrap()
}
fn req(s: &str) -> Req {
    Req::parse(s).unwrap()
}
fn cand(ver: &str, deps: &[(&str, &str)]) -> Candidate {
    Candidate {
        version: v(ver),
        checksum: Some(lpp_pm::ContentAddress::of_bytes(ver.as_bytes()).to_string()),
        source: "registry".into(),
        deps: deps.iter().map(|(n, r)| (n.to_string(), req(r))).collect(),
    }
}
fn root(name: &str, deps: &[(&str, &str)]) -> Pkg {
    Pkg {
        name: name.into(),
        version: v("0.1.0"),
        checksum: None,
        source: "root".into(),
        deps: deps.iter().map(|(n, r)| (n.to_string(), req(r))).collect(),
    }
}

// --- semver ---

#[test]
fn lock_rejects_unverified_registry_and_unsafe_path_entries() {
    let missing_checksum = r#"
version = 2
[[package]]
name = "math"
version = "1.0.0"
source = "registry"
"#;
    assert!(Lock::parse(missing_checksum).is_err());

    let unsafe_path = r#"
version = 2
[[package]]
name = "helper"
version = "1.0.0"
source = "path:../../escape"
"#;
    assert!(Lock::parse(unsafe_path).is_err());
}

#[test]
fn concurrent_atomic_lock_writers_never_collide_or_leave_partial_toml() {
    let directory = std::env::temp_dir().join(format!("lpp-lock-atomic-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let path = std::sync::Arc::new(directory.join("Keel.lock"));
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let mut writers = Vec::new();
    for index in 0..8 {
        let path = path.clone();
        let barrier = barrier.clone();
        writers.push(std::thread::spawn(move || {
            let lock = Lock {
                lock_version: lpp_pm::lock::LOCK_VERSION,
                registry: Some(format!("registry-{index}")),
                packages: vec![lpp_pm::LockedPkg {
                    name: "app".into(),
                    version: "1.0.0".into(),
                    source: "root".into(),
                    checksum: None,
                    deps: vec![],
                    dep_reqs: Default::default(),
                }],
            };
            barrier.wait();
            lock.save_atomic(&path).unwrap();
        }));
    }
    for writer in writers {
        writer.join().unwrap();
    }
    let document = std::fs::read_to_string(path.as_ref()).unwrap();
    Lock::parse(&document).unwrap();
    assert!(std::fs::read_dir(&directory).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".tmp-")
    }));
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn semver_versions_order_and_parse() {
    assert_eq!(v("1.2.3"), Version::new(1, 2, 3));
    assert_eq!(v("1"), Version::new(1, 0, 0));
    assert_eq!(v("1.2"), Version::new(1, 2, 0));
    assert!(v("1.2.3") < v("1.2.4") && v("1.2.9") < v("1.10.0"));
    assert!(Version::parse("1.2.3.4").is_none());
    assert!(Version::parse("abc").is_none());
}

#[test]
fn semver_requirement_matching() {
    // caret (including the 0.x special cases)
    assert!(req("^1.2.3").matches(&v("1.9.0")));
    assert!(!req("^1.2.3").matches(&v("2.0.0")));
    assert!(req("^0.2.3").matches(&v("0.2.9")));
    assert!(!req("^0.2.3").matches(&v("0.3.0")));
    // bare version == caret
    assert!(req("1").matches(&v("1.5.0")) && !req("1").matches(&v("2.0.0")));
    // tilde
    assert!(req("~1.2.3").matches(&v("1.2.9")));
    assert!(!req("~1.2.3").matches(&v("1.3.0")));
    // exact + comparators + any
    assert!(req("=1.2.3").matches(&v("1.2.3")) && !req("=1.2.3").matches(&v("1.2.4")));
    assert!(req(">=1.2.0").matches(&v("1.2.0")) && req(">1.2.0").matches(&v("1.2.1")));
    assert!(req("<2.0.0").matches(&v("1.9.9")) && !req("<2.0.0").matches(&v("2.0.0")));
    assert!(req("*").matches(&v("9.9.9")));
}

// --- resolver ---

#[test]
fn resolves_the_highest_matching_version() {
    let r = resolve(&root("app", &[("math", "^1.0")]), &|name| match name {
        "math" => Some(vec![
            cand("1.0.0", &[]),
            cand("1.4.2", &[]),
            cand("2.0.0", &[]),
        ]),
        _ => None,
    })
    .unwrap();
    assert_eq!(r.get("math").unwrap().version, v("1.4.2")); // highest 1.x, not 2.0.0
}

#[test]
fn resolves_transitive_deps() {
    let r = resolve(&root("app", &[("a", "^1.0")]), &|name| match name {
        "a" => Some(vec![cand("1.0.0", &[("b", "^2.0")])]),
        "b" => Some(vec![cand("2.1.0", &[])]),
        _ => None,
    })
    .unwrap();
    assert!(r.get("a").is_some() && r.get("b").is_some());
    assert_eq!(r.get("b").unwrap().version, v("2.1.0"));
}

#[test]
fn shared_compatible_dep_is_resolved_once() {
    // a and c both need b ^1 -> exactly one b.
    let r = resolve(
        &root("app", &[("a", "^1.0"), ("c", "^1.0")]),
        &|name| match name {
            "a" => Some(vec![cand("1.0.0", &[("b", "^1.0")])]),
            "c" => Some(vec![cand("1.0.0", &[("b", "^1.0")])]),
            "b" => Some(vec![cand("1.5.0", &[])]),
            _ => None,
        },
    )
    .unwrap();
    assert_eq!(r.get("b").unwrap().version, v("1.5.0"));
}

#[test]
fn conflicting_requirements_error() {
    // a needs b ^1, c needs b ^2 -> incompatible.
    let e = resolve(
        &root("app", &[("a", "^1.0"), ("c", "^1.0")]),
        &|name| match name {
            "a" => Some(vec![cand("1.0.0", &[("b", "^1.0")])]),
            "c" => Some(vec![cand("1.0.0", &[("b", "^2.0")])]),
            "b" => Some(vec![cand("1.5.0", &[]), cand("2.0.0", &[])]),
            _ => None,
        },
    )
    .unwrap_err();
    assert!(
        matches!(e, lpp_pm::PmError::ResolveConflict { .. }),
        "got {e}"
    );
}

#[test]
fn cycles_do_not_loop_forever() {
    // a -> b -> a
    let r = resolve(&root("app", &[("a", "^1.0")]), &|name| match name {
        "a" => Some(vec![cand("1.0.0", &[("b", "^1.0")])]),
        "b" => Some(vec![cand("1.0.0", &[("a", "^1.0")])]),
        _ => None,
    })
    .unwrap();
    assert!(r.get("a").is_some() && r.get("b").is_some());
}

#[test]
fn backtracks_from_a_high_version_when_a_later_constraint_requires_lower() {
    let resolved = resolve(
        &root("app", &[("a", "*"), ("c", "*")]),
        &|name| match name {
            "a" => Some(vec![
                cand("1.0.0", &[("b", "^1")]),
                cand("2.0.0", &[("b", "^2")]),
            ]),
            "c" => Some(vec![cand("1.0.0", &[("b", "^1")])]),
            "b" => Some(vec![cand("1.5.0", &[]), cand("2.5.0", &[])]),
            _ => None,
        },
    )
    .unwrap();
    assert_eq!(resolved.get("a").unwrap().version, v("1.0.0"));
    assert_eq!(resolved.get("b").unwrap().version, v("1.5.0"));
}

#[test]
fn unknown_package_is_a_typed_error() {
    let e = resolve(&root("app", &[("ghost", "^1.0")]), &|_| None).unwrap_err();
    assert!(
        matches!(e, lpp_pm::PmError::NoMatchingVersion { .. }),
        "got {e}"
    );
}

// --- lock ---

#[test]
fn lock_round_trips_through_toml() {
    let resolved = resolve(&root("app", &[("math", "^1.0")]), &|name| match name {
        "math" => Some(vec![cand("1.4.2", &[])]),
        _ => None,
    })
    .unwrap();
    let lock = Lock::from_resolved(&resolved);
    let toml = lock.to_toml().unwrap();
    let back = Lock::parse(&toml).unwrap();
    assert_eq!(lock, back);
    assert_eq!(
        back.checksum("math"),
        Some(
            lpp_pm::ContentAddress::of_bytes(b"1.4.2")
                .to_string()
                .as_str()
        )
    );
    assert!(toml.contains("version = 2"));
}
