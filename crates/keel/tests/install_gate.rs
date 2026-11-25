use std::path::{Path, PathBuf};

use keel::{InvokeOutcome, invoke_from};

fn project(name: &str) -> PathBuf {
    let scratch =
        std::env::temp_dir().join(format!("keel_install_{}_{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&scratch);
    let project = scratch.join("project");
    std::fs::create_dir_all(&project).unwrap();
    project
}

fn invoke(directory: &Path, arguments: &[&str]) -> InvokeOutcome {
    invoke_from(
        std::iter::once("keel".to_string()).chain(arguments.iter().map(|value| value.to_string())),
        directory,
    )
}

#[test]
fn install_locks_a_registry_free_workspace_without_configuration() {
    let root = project("offline");
    assert_eq!(invoke(&root, &["init"]), InvokeOutcome::Success);
    assert_eq!(invoke(&root, &["install"]), InvokeOutcome::Success);

    let lock =
        lpp_pm::Lock::parse(&std::fs::read_to_string(root.join("Keel.lock")).unwrap()).unwrap();
    assert_eq!(lock.packages.len(), 1);
    assert_eq!(lock.packages[0].name, "project");
    assert_eq!(lock.packages[0].source, "root");
    assert!(lock.registry.is_none());

    // Reinstalling is deterministic and remains offline.
    let first = std::fs::read(root.join("Keel.lock")).unwrap();
    assert_eq!(invoke(&root, &["install"]), InvokeOutcome::Success);
    assert_eq!(std::fs::read(root.join("Keel.lock")).unwrap(), first);
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

#[test]
fn install_requires_registry_when_explicitly_offline() {
    let root = project("registry-required");
    assert_eq!(invoke(&root, &["init"]), InvokeOutcome::Success);
    assert_eq!(
        invoke(&root, &["add", "external@^1"]),
        InvokeOutcome::Success
    );
    // Explicitly opt into offline mode (`KEEL_REGISTRY=offline`) — registry deps must fail cleanly.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_keel"))
        .arg("install")
        .current_dir(&root)
        .env("KEEL_REGISTRY", "offline")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("registry dependencies require"),
        "unexpected stderr: {stderr}"
    );
    assert!(!root.join("Keel.lock").exists());
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

#[test]
fn add_path_dependency_and_valid_template() {
    let root = project("path-dep");
    assert_eq!(invoke(&root, &["init"]), InvokeOutcome::Success);
    let main_src = std::fs::read_to_string(root.join("src/main.lpp")).unwrap();
    assert!(
        main_src.contains("print_str(\"Hello from L++\")"),
        "expected valid print_str template, got: {main_src}"
    );
    assert_eq!(
        invoke(&root, &["add", "local-lib", "--path", "../local-lib"]),
        InvokeOutcome::Success
    );
    let manifest_text = std::fs::read_to_string(root.join("Keel.toml")).unwrap();
    let manifest = lpp_pm::manifest::Manifest::parse(&manifest_text).unwrap();
    assert_eq!(
        manifest.dependencies["local-lib"].path(),
        Some("../local-lib")
    );
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}
