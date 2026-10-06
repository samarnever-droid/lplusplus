use std::path::{Path, PathBuf};

use keel::{InvokeOutcome, invoke_from};

fn workdir(name: &str) -> PathBuf {
    let scratch =
        std::env::temp_dir().join(format!("keel_metadata_{}_{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&scratch);
    let directory = scratch.join("project");
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

fn invoke(directory: &Path, arguments: &[&str]) -> InvokeOutcome {
    invoke_from(
        std::iter::once("keel".to_string()).chain(arguments.iter().map(|value| value.to_string())),
        directory,
    )
}

#[test]
fn metadata_is_deterministic_and_reports_manifest_and_lock_counts() {
    let root = workdir("single");
    assert_eq!(invoke(&root, &["init"]), InvokeOutcome::Success);
    assert_eq!(
        invoke(&root, &["add", "example@^1.2"]),
        InvokeOutcome::Success
    );
    std::fs::write(root.join("Keel.lock"), "version = 2\npackages = []\n").unwrap();

    let workspace = lpp_pm::Workspace::discover(&root).unwrap();
    let first = keel::commands::metadata::render(&workspace).unwrap();
    let second = keel::commands::metadata::render(&workspace).unwrap();
    assert_eq!(first, second);
    assert!(first.contains("virtual_workspace = false\n"), "{first}");
    assert!(first.contains("workspace_members = 1\n"), "{first}");
    assert!(first.contains("locked_packages = 0\n"), "{first}");
    let package_name = root.file_name().unwrap().to_string_lossy();
    assert!(
        first.contains(&format!("name = {package_name}\n")),
        "{first}"
    );
    assert!(first.contains("version = 0.1.0\n"), "{first}");
    assert!(first.contains("dependencies = 1\n"), "{first}");
    assert!(
        first.contains(&format!("member.0.name = {package_name}\n")),
        "{first}"
    );

    assert_eq!(invoke(&root, &["metadata"]), InvokeOutcome::Success);
    let listed = keel::commands::list::render(&workspace);
    assert!(
        listed.contains(&format!("{package_name} 0.1.0\n")),
        "{listed}"
    );
    assert!(listed.contains("  example ^1.2 [registry]\n"), "{listed}");
    assert_eq!(invoke(&root, &["list"]), InvokeOutcome::Success);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn metadata_rejects_an_invalid_lockfile_instead_of_guessing() {
    let root = workdir("bad-lock");
    assert_eq!(invoke(&root, &["init"]), InvokeOutcome::Success);
    std::fs::write(root.join("Keel.lock"), "version = 99\npackages = []\n").unwrap();

    let outcome = invoke(&root, &["metadata"]);
    assert!(
        matches!(outcome, InvokeOutcome::Failure(message) if message.contains("unsupported lock format"))
    );
    let _ = std::fs::remove_dir_all(root);
}
