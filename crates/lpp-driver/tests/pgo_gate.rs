use std::path::{Path, PathBuf};
use std::process::Command;

use lpp_common::OptimizationLevel;
use lpp_driver::{
    BackendChoice, CompileOptions, CompilerSession, DriverRequest, RewriteEngine,
    build_executable_configured, compile_entry_with_options, runtime_library_path,
};

fn workdir(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("lpp_pgo_{}_{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

fn build(entry: &Path, output: &Path, options: CompileOptions) {
    let runtime = runtime_library_path().expect("the test runtime cdylib is built");
    build_executable_configured(
        entry,
        "pgo_gate",
        BackendChoice::Cranelift,
        options,
        output,
        runtime.parent().unwrap(),
        Some("cc"),
    )
    .unwrap();
}

fn run(executable: &Path, profile: &Path) -> String {
    let output = Command::new(executable)
        .env("LPP_PGO_PROFILE", profile)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "program failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn cli_check(directory: &Path, entry: &Path, profile: &Path) -> i32 {
    let request = DriverRequest::from_args(
        [
            "lpp".to_string(),
            entry.to_string_lossy().into_owned(),
            "--pgo-use".to_string(),
            profile.to_string_lossy().into_owned(),
            "--check".to_string(),
        ],
        directory,
    )
    .unwrap();
    CompilerSession::new(RewriteEngine)
        .execute(&request)
        .exit_code()
}

#[test]
fn pgo_profile_roundtrips_and_guides_hot_callee_inlining() {
    let directory = workdir("roundtrip");
    let entry = directory.join("main.lpp");
    let profile = directory.join("sample.pgo");
    let generated_executable = directory.join("generate");
    let optimized_executable = directory.join("use-profile");
    std::fs::write(
        &entry,
        concat!(
            "def hot(value: Int) -> Int:\n",
            "    return value + 1\n",
            "def main() -> Int:\n",
            "    print_int(hot(41))\n",
            "    return 0\n",
        ),
    )
    .unwrap();

    let mut generate = CompileOptions::for_backend(BackendChoice::Cranelift);
    generate.optimization = OptimizationLevel::O2;
    generate.pgo_generate = true;
    build(&entry, &generated_executable, generate);
    assert_eq!(run(&generated_executable, &profile), "42\n");

    let profile_text = std::fs::read_to_string(&profile).unwrap();
    assert!(profile_text.starts_with("LPP-PGO-1 "), "{profile_text}");
    assert!(
        profile_text
            .lines()
            .skip(1)
            .any(|line| !line.trim().is_empty())
    );

    let mut use_profile = CompileOptions::for_backend(BackendChoice::Cranelift);
    use_profile.optimization = OptimizationLevel::O2;
    use_profile.pgo_use = Some(profile.clone());
    build(&entry, &optimized_executable, use_profile.clone());
    assert_eq!(run(&optimized_executable, &profile), "42\n");
    assert_eq!(cli_check(&directory, &entry, &profile), 0);

    // A profile cannot silently cross a source/MIR change.
    std::fs::write(
        &entry,
        concat!(
            "def hot(value: Int) -> Int:\n",
            "    return value + 2\n",
            "def main() -> Int:\n",
            "    print_int(hot(41))\n",
            "    return 0\n",
        ),
    )
    .unwrap();
    let error =
        compile_entry_with_options(&entry, "pgo_gate", BackendChoice::Cranelift, use_profile)
            .unwrap_err();
    assert!(
        error.to_string().contains("fingerprint mismatch"),
        "unexpected error: {error}"
    );
    let _ = std::fs::remove_dir_all(directory);
}
