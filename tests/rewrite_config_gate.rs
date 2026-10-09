use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch() -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("lpp_rewrite_config_gate_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

fn lpp(home: &Path, working_directory: &Path, arguments: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_lpp"));
    cmd.current_dir(working_directory)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("LPP_ENGINE", "rewrite")
        .env("LPP_BENCH_BIN", env!("CARGO_BIN_EXE_lpp"))
        .env("LPP_UPDATE_LATEST_TAG", "v0.1.0")
        .env("LPP_LLVM_CC", env!("CARGO_BIN_EXE_lpp"));
    if let Some(runtime) = lpp_driver::runtime_library_path() {
        cmd.env("LPP_RUNTIME_LIB", runtime);
    }
    cmd.args(arguments).output().unwrap()
}

#[test]
#[cfg_attr(
    target_os = "windows",
    ignore = "MSVC environment is exercised by the Windows driver smoke gate"
)]
fn rewrite_config_is_persisted_validated_and_used_as_the_backend_default() {
    let root = scratch();
    if lpp_driver::runtime_library_path().is_none() {
        let _ = Command::new(env!("CARGO"))
            .args(["build", "--locked", "-p", "lpp-runtime"])
            .output();
    }
    let home = root.join("home");
    let project = root.join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("main.lpp"),
        "def main() -> Int:\n    return 0\n",
    )
    .unwrap();

    let invalid_engine = Command::new(env!("CARGO_BIN_EXE_lpp"))
        .current_dir(&project)
        .env("HOME", &home)
        .env("LPP_ENGINE", "mystery")
        .arg("--version")
        .output()
        .unwrap();
    assert_eq!(invalid_engine.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&invalid_engine.stderr).contains("legacy` or `rewrite"));

    let version = lpp(&home, &project, &["--version"]);
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        "L++ v0.1.0 (rewrite engine)"
    );

    let bench = lpp(&home, &project, &["bench", "--version"]);
    assert!(bench.status.success());
    assert_eq!(
        String::from_utf8_lossy(&bench.stdout).trim(),
        "L++ v0.1.0 (rewrite engine)"
    );

    let update_policy = lpp(&home, &project, &["upgrade"]);
    assert_eq!(update_policy.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&update_policy.stderr).contains("disabled"));
    for alias in ["upgrade", "self-update", "update-self"] {
        let check = lpp(&home, &project, &[alias, "--check"]);
        assert!(
            check.status.success(),
            "{alias}: {}",
            String::from_utf8_lossy(&check.stderr)
        );
        assert!(
            String::from_utf8_lossy(&check.stdout).contains("latest production build"),
            "{}",
            String::from_utf8_lossy(&check.stdout)
        );
    }

    let retired_login = lpp(&home, &project, &["login", "secret-token"]);
    assert_eq!(retired_login.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&retired_login.stderr).contains("retired"));
    assert!(!home.join(".lpp/credentials").exists());
    let lreact_help = lpp(&home, &project, &["lreact", "help"]);
    assert!(lreact_help.status.success());
    let retired_scaffold = lpp(&home, &project, &["lreact", "create", "web-app"]);
    assert_eq!(retired_scaffold.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&retired_scaffold.stderr).contains("retired"));

    for command in [vec!["setup", "llvm"], vec!["toolchain", "install", "llvm"]] {
        let setup = lpp(&home, &project, &command);
        assert!(
            setup.status.success(),
            "{command:?}: {}",
            String::from_utf8_lossy(&setup.stderr)
        );
        assert!(
            String::from_utf8_lossy(&setup.stdout).contains("LLVM compiler configured"),
            "{}",
            String::from_utf8_lossy(&setup.stdout)
        );
    }

    let init = lpp(&home, &project, &["init"]);
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );
    let dev = lpp(&home, &project, &["dev"]);
    assert!(
        dev.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&dev.stdout),
        String::from_utf8_lossy(&dev.stderr)
    );

    let set = lpp(&home, &project, &["config", "set", "backend", "wasm"]);
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );
    let config_path = home.join(".lpp/config.json");
    let configured = std::fs::read_to_string(&config_path).unwrap();
    assert!(configured.contains("\"backend\": \"wasm\""), "{configured}");

    let check = lpp(&home, &project, &["main.lpp", "--check"]);
    assert!(
        check.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(
        String::from_utf8_lossy(&check.stdout).contains("bytes of wasm"),
        "{}",
        String::from_utf8_lossy(&check.stdout)
    );

    let before_invalid = std::fs::read(&config_path).unwrap();
    let invalid = lpp(&home, &project, &["config", "set", "linker", "mystery"]);
    assert!(!invalid.status.success());
    assert_eq!(std::fs::read(&config_path).unwrap(), before_invalid);

    let summary = lpp(&home, &project, &["config"]);
    assert!(summary.status.success());
    assert!(
        String::from_utf8_lossy(&summary.stdout).contains("Backend:     wasm"),
        "{}",
        String::from_utf8_lossy(&summary.stdout)
    );

    let _ = std::fs::remove_dir_all(root);
}
