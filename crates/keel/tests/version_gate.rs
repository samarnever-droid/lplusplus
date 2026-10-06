use std::path::{Path, PathBuf};

use keel::{InvokeOutcome, invoke_from};

fn workdir(name: &str) -> PathBuf {
    let scratch =
        std::env::temp_dir().join(format!("keel_version_{}_{}", std::process::id(), name));
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
fn version_show_set_and_bump_preserve_manifest_and_update_lock() {
    let root = workdir("lifecycle");
    assert_eq!(invoke(&root, &["init"]), InvokeOutcome::Success);
    let name = root.file_name().unwrap().to_string_lossy();
    let manifest_path = root.join("Keel.toml");
    let source = std::fs::read_to_string(&manifest_path).unwrap();
    std::fs::write(
        &manifest_path,
        source.replace("[package]", "# preserved comment\n[package]"),
    )
    .unwrap();
    std::fs::write(
        root.join("Keel.lock"),
        format!(
            "version = 2\n\n[[packages]]\nname = \"{name}\"\nversion = \"0.1.0\"\nsource = \"root\"\ndeps = []\n"
        ),
    )
    .unwrap();

    assert_eq!(invoke(&root, &["version"]), InvokeOutcome::Success);
    assert_eq!(
        invoke(&root, &["version", "bump", "patch"]),
        InvokeOutcome::Success
    );
    let bumped = std::fs::read_to_string(&manifest_path).unwrap();
    assert!(bumped.contains("# preserved comment"), "{bumped}");
    assert!(bumped.contains("version = \"0.1.1\""), "{bumped}");
    let lock =
        lpp_pm::Lock::parse(&std::fs::read_to_string(root.join("Keel.lock")).unwrap()).unwrap();
    assert_eq!(lock.package(&name).unwrap().version, "0.1.1");

    assert_eq!(
        invoke(&root, &["version", "set", "v2.3"]),
        InvokeOutcome::Success
    );
    let set = std::fs::read_to_string(&manifest_path).unwrap();
    assert!(set.contains("version = \"2.3.0\""), "{set}");

    let before = set;
    assert!(matches!(
        invoke(&root, &["version", "set", "not-semver"]),
        InvokeOutcome::Failure(_)
    ));
    assert_eq!(std::fs::read_to_string(&manifest_path).unwrap(), before);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn virtual_workspace_requires_a_member_directory() {
    let root = workdir("virtual");
    let member = root.join("packages/app");
    let library = root.join("packages/library");
    std::fs::create_dir_all(member.join("src")).unwrap();
    std::fs::create_dir_all(library.join("src")).unwrap();
    std::fs::write(
        root.join("Keel.toml"),
        "[workspace]\nmembers = [\"packages/*\"]\n",
    )
    .unwrap();
    std::fs::write(
        member.join("Keel.toml"),
        "[package]\nname = \"app\"\nversion = \"1.2.3\"\nedition = \"2024\"\n",
    )
    .unwrap();
    std::fs::write(member.join("src/main.lpp"), "def main():\n    print(1)\n").unwrap();
    std::fs::write(
        library.join("Keel.toml"),
        "[package]\nname = \"library\"\nversion = \"0.5.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    std::fs::write(
        library.join("src/lib.lpp"),
        "def value() -> Int:\n    return 1\n",
    )
    .unwrap();

    assert!(matches!(
        invoke(&root, &["version"]),
        InvokeOutcome::Failure(message) if message.contains("virtual")
    ));
    assert_eq!(
        invoke(&member, &["version", "bump", "minor"]),
        InvokeOutcome::Success
    );
    let manifest = std::fs::read_to_string(member.join("Keel.toml")).unwrap();
    assert!(manifest.contains("version = \"1.3.0\""), "{manifest}");
    let _ = std::fs::remove_dir_all(root);
}
