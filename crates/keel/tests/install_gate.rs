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
fn install_requires_registry_configuration_only_for_registry_dependencies() {
    let root = project("registry-required");
    assert_eq!(invoke(&root, &["init"]), InvokeOutcome::Success);
    assert_eq!(
        invoke(&root, &["add", "external@^1"]),
        InvokeOutcome::Success
    );
    assert!(matches!(
        invoke(&root, &["install"]),
        InvokeOutcome::Failure(message) if message.contains("registry dependencies require")
    ));
    assert!(!root.join("Keel.lock").exists());
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}
