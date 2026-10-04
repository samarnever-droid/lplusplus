//! Gate: `keel new` / `init` / `add` / `remove` — the project lifecycle.

fn temp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("lpp-keel-proj-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn new_scaffolds_a_project() {
    let root = temp("new");
    let dir = keel::commands::project::new_project("mylib", &root).unwrap();
    assert!(dir.join("Keel.toml").exists());
    assert!(dir.join("src/main.lpp").exists());
    let toml = std::fs::read_to_string(dir.join("Keel.toml")).unwrap();
    assert!(toml.contains("name = \"mylib\""), "{toml}");
    let m = lpp_pm::manifest::Manifest::parse(&toml).unwrap();
    assert_eq!(m.name(), "mylib");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn new_rejects_bad_names() {
    let root = temp("badname");
    assert!(keel::commands::project::new_project("Bad Name", &root).is_err());
    assert!(keel::commands::project::new_project("UPPER", &root).is_err());
    assert!(keel::commands::project::new_project("", &root).is_err());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn init_is_idempotent_and_non_overwriting() {
    let root = temp("init");
    let proj = root.join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    keel::commands::project::init(&proj).unwrap();
    assert!(proj.join("Keel.toml").exists());
    assert!(proj.join("src/main.lpp").exists());
    // second init: no error, and it must NOT overwrite the user's source
    std::fs::write(proj.join("src/main.lpp"), "fn main() { CUSTOM }\n").unwrap();
    keel::commands::project::init(&proj).unwrap();
    let main = std::fs::read_to_string(proj.join("src/main.lpp")).unwrap();
    assert!(main.contains("CUSTOM"), "init must not overwrite: {main}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn add_and_remove_round_trip_the_manifest() {
    let root = temp("addremove");
    let proj = root.join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    keel::commands::project::init(&proj).unwrap();

    keel::commands::project::add_dep(&proj, "math", "^1.0").unwrap();
    keel::commands::project::add_dep(&proj, "simdlib", "2").unwrap();
    let m = lpp_pm::manifest::Manifest::parse(
        &std::fs::read_to_string(proj.join("Keel.toml")).unwrap(),
    )
    .unwrap();
    assert!(m.dependencies.contains_key("math"));
    assert!(m.dependencies.contains_key("simdlib"));
    assert_eq!(m.dependencies["math"].version(), "^1.0");

    keel::commands::project::remove_dep(&proj, "math").unwrap();
    let m2 = lpp_pm::manifest::Manifest::parse(
        &std::fs::read_to_string(proj.join("Keel.toml")).unwrap(),
    )
    .unwrap();
    assert!(!m2.dependencies.contains_key("math"));
    assert!(m2.dependencies.contains_key("simdlib"));
    assert!(keel::commands::project::remove_dep(&proj, "nope").is_err());
    let _ = std::fs::remove_dir_all(&root);
}
