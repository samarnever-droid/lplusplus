#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch() -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("lpp_rewrite_installed_gate_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

fn copy(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
    std::fs::copy(source, destination).unwrap();
}

#[test]
fn installed_bin_lib_layout_discovers_links_and_loads_the_rewrite_runtime() {
    let root = scratch();
    let prefix = root.join("toolchain");
    let installed_lpp = prefix.join("bin/lpp");
    copy(Path::new(env!("CARGO_BIN_EXE_lpp")), &installed_lpp);

    let runtime = lpp_driver::runtime_library_path()
        .expect("build lpp-runtime before this installed-layout gate");
    copy(
        &runtime,
        &prefix
            .join("lib")
            .join(lpp_driver::runtime_library_filename()),
    );

    let project = root.join("project");
    let home = root.join("home");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        project.join("installed.lpp"),
        "def main():\n    print_int(42)\n",
    )
    .unwrap();

    let output = Command::new(&installed_lpp)
        .current_dir(&project)
        .env("HOME", &home)
        .env("LPP_ENGINE", "rewrite")
        .args(["run", "installed.lpp"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line.trim() == "42"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(!project.join("installed.o").exists());

    let _ = std::fs::remove_dir_all(root);
}
