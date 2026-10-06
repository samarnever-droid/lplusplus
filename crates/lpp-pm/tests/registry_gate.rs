//! Registry + manifest exit gate.

use lpp_pm::index::{DepSpec, IndexEntry, VersionEntry, index_path};
use lpp_pm::manifest::{Dependency, Manifest};

#[test]
fn parses_a_keel_toml_manifest() {
    let doc = r#"
[package]
name = "mylib"
version = "1.2.0"
edition = "2024"
license = "MIT OR Apache-2.0"

[dependencies]
otherlib = "1"
simdlib = { version = "2", optional = true, features = ["fast"] }

[features]
default = []
simd = ["simdlib"]

[targets]
supported = ["x86_64", "aarch64", "wasm32-wasi"]
"#;
    let m = Manifest::parse(doc).unwrap();
    assert_eq!(m.name(), "mylib");
    assert_eq!(m.version(), "1.2.0");
    assert_eq!(m.package.edition, "2024");
    assert_eq!(m.dependencies.len(), 2);
    match m.dependencies.get("otherlib").unwrap() {
        Dependency::Version(v) => assert_eq!(v, "1"),
        _ => panic!("otherlib should be a version-only dep"),
    }
    match m.dependencies.get("simdlib").unwrap() {
        Dependency::Detailed {
            version,
            optional,
            features,
            ..
        } => {
            assert_eq!(version.as_deref(), Some("2"));
            assert!(*optional);
            assert_eq!(features, &vec!["fast".to_string()]);
        }
        _ => panic!("simdlib should be detailed"),
    }
    assert_eq!(
        m.features.get("simd").unwrap(),
        &vec!["simdlib".to_string()]
    );
    assert_eq!(
        m.targets.as_ref().unwrap().supported,
        vec!["x86_64", "aarch64", "wasm32-wasi"]
    );
}

#[test]
fn manifest_parse_error_is_typed() {
    // No [package] section → parse error, not a panic.
    let err = Manifest::parse("[dependencies]\nx = \"1\"").unwrap_err();
    assert!(matches!(err, lpp_pm::PmError::ManifestParse(_)));
}

#[test]
fn manifest_rejects_malformed_names_versions_and_requirements() {
    for document in [
        "[package]\nname = \"../escape\"\nversion = \"1.0.0\"\n",
        "[package]\nname = \"safe\"\nversion = \"not-semver\"\n",
        "[package]\nname = \"safe\"\nversion = \"1.0.0\"\n[dependencies]\ndep = \"definitely not a requirement\"\n",
    ] {
        assert!(Manifest::parse(document).is_err(), "accepted {document:?}");
    }
}

#[test]
fn index_path_matches_the_sparse_layout() {
    assert_eq!(index_path("a"), "1/a");
    assert_eq!(index_path("ab"), "2/ab");
    assert_eq!(index_path("abc"), "3/a/bc");
    assert_eq!(index_path("abcd"), "ab/cd/abcd");
    assert_eq!(index_path("serde"), "se/rd/serde");
}

#[test]
fn index_entry_round_trips_and_builds_download_urls() {
    let e = IndexEntry {
        name: "mylib".into(),
        versions: vec![VersionEntry {
            version: "1.2.0".into(),
            deps: vec![DepSpec {
                name: "otherlib".into(),
                req: "^1".into(),
                optional: false,
                features: vec![],
            }],
            features: Default::default(),
            checksum: "abc123".into(),
            targets: vec!["x86_64".into()],
            yanked: false,
        }],
    };
    let json = serde_json::to_string(&e).unwrap();
    let back: IndexEntry = serde_json::from_str(&json).unwrap();
    assert_eq!(back, e);
    assert_eq!(
        e.download_url("https://registry.lplusplus.bond", "1.2.0"),
        Some("https://registry.lplusplus.bond/blob/abc123".to_string())
    );
    assert_eq!(
        e.download_url("https://registry.lplusplus.bond", "9.9.9"),
        None
    );
}
