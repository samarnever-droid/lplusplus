use std::path::{Path, PathBuf};

use keel::{InvokeOutcome, invoke_from};

fn workdir(name: &str) -> PathBuf {
    let scratch = std::env::temp_dir().join(format!("keel_clean_{}_{}", std::process::id(), name));
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
fn clean_removes_only_workspace_build_outputs_and_is_idempotent() {
    let root = workdir("outputs");
    assert_eq!(invoke(&root, &["init"]), InvokeOutcome::Success);

    let target = root.join("target");
    std::fs::create_dir_all(target.join("debug")).unwrap();
    std::fs::write(target.join("debug/program"), b"binary").unwrap();
    let staged = root.join(".lpp_packages/example");
    std::fs::create_dir_all(&staged).unwrap();
    std::fs::write(staged.join("keep"), b"dependency").unwrap();

    assert_eq!(invoke(&root, &["clean"]), InvokeOutcome::Success);
    assert!(!target.exists());
    assert_eq!(std::fs::read(staged.join("keep")).unwrap(), b"dependency");

    assert_eq!(invoke(&root, &["clean"]), InvokeOutcome::Success);
    assert!(staged.join("keep").is_file());
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn clean_unlinks_a_target_symlink_without_following_it() {
    use std::os::unix::fs::symlink;

    let root = workdir("symlink");
    assert_eq!(invoke(&root, &["init"]), InvokeOutcome::Success);
    let victim = root.join("victim");
    std::fs::create_dir_all(&victim).unwrap();
    std::fs::write(victim.join("keep"), b"safe").unwrap();
    symlink(&victim, root.join("target")).unwrap();

    assert_eq!(invoke(&root, &["clean"]), InvokeOutcome::Success);
    assert!(!root.join("target").exists());
    assert_eq!(std::fs::read(victim.join("keep")).unwrap(), b"safe");
    let _ = std::fs::remove_dir_all(root);
}
