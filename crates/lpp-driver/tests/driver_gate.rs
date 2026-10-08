//! Phase 7 driver gate — the real engine compiles an L++ source file all the
//! way to a runnable native executable on the proven Rust runtime.
//!
//! Two slices of the end-to-end pipeline are pinned here:
//! 1. **source -> object.** `compile_entry` reads a `.lpp` file from disk
//!    (`OsFileSystem`), runs HIR -> typed -> MIR -> Cranelift, and yields a
//!    non-empty `X86_64` object that exports `main`.
//! 2. **source -> executable -> run.** `build_executable` links that object
//!    against the Rust runtime cdylib (the 6B.3e drop-in artifact); running
//!    the executable produces the expected stdout and exits 0.

use std::path::{Path, PathBuf};
use std::process::Command;

use lpp_driver::{
    BackendChoice, CompilerSession, DriverRequest, RewriteEngine, Target, build_executable,
    compile_entry, default_runtime_lib_dir,
};

/// Ensure the host runtime cdylib exists (building it on demand) and return
/// its directory. `cargo test` does not build that artifact unless something
/// depends on it; cargo has released the package lock by the time test
/// binaries run, so this nested build is safe.
fn runtime_lib_dir() -> PathBuf {
    let dir = default_runtime_lib_dir();
    let runtime = if cfg!(target_os = "windows") {
        "lpp_runtime.dll"
    } else if cfg!(target_os = "macos") {
        "liblpp_runtime.dylib"
    } else {
        "liblpp_runtime.so"
    };
    if !dir.join(runtime).exists() {
        let workspace = dir.parent().and_then(Path::parent).expect("workspace root");
        let out = Command::new("cargo")
            .args(["build", "-p", "lpp-runtime"])
            .current_dir(workspace)
            .output()
            .expect("cargo build -p lpp-runtime");
        assert!(
            out.status.success(),
            "runtime build failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    dir
}

fn workdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lpp_driver_{}_{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn compiles_an_entry_to_an_object() {
    let dir = workdir("object");
    let entry = dir.join("main.lpp");
    std::fs::write(
        &entry,
        "def main() -> Int:\n    print_int(6 * 7)\n    return 0\n",
    )
    .unwrap();

    let module = compile_entry(&entry, "objtest", BackendChoice::Cranelift)
        .expect("compile_entry must succeed");
    assert_eq!(module.target, Target::host());
    assert!(!module.object.is_empty(), "object must be non-empty");
    assert!(
        module
            .exported_symbols
            .iter()
            .any(|symbol| symbol == "main"),
        "object must export main; got {:?}",
        module.exported_symbols
    );
}

#[test]
fn compiles_and_runs_a_native_executable() {
    let dir = workdir("exe");
    let entry = dir.join("main.lpp");
    std::fs::write(
        &entry,
        "def main() -> Int:\n    print_int(6 * 7)\n    print_str(\"driver_ok\")\n    return 0\n",
    )
    .unwrap();
    let exe = if cfg!(target_os = "windows") {
        dir.join("program.exe")
    } else {
        dir.join("program")
    };

    build_executable(
        &entry,
        "exetest",
        BackendChoice::Cranelift,
        &exe,
        &runtime_lib_dir(),
    )
    .expect("build_executable must succeed");

    let run = Command::new(&exe).output().expect("run the executable");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    assert_eq!(run.status.code(), Some(0), "exe exited non-zero:\n{stdout}");
    assert_eq!(stdout.replace("\r\n", "\n"), "42\ndriver_ok\n", "unexpected program output");
}

/// Cutover stage 1: `RewriteEngine` drives the same pipeline through the
/// `CompilerEngine` contract. `lpp run <file.lpp>` with `LPP_ENGINE=rewrite`
/// compiles, links, and runs, returning exit code 0 to the session and leaving
/// the executable beside the source.
#[test]
fn rewrite_engine_runs_a_program_through_the_driver_contract() {
    let dir = workdir("engine");
    let entry = dir.join("main.lpp");
    std::fs::write(
        &entry,
        "def main() -> Int:\n    print_int(6 * 7)\n    print_str(\"driver_ok\")\n    return 0\n",
    )
    .unwrap();
    let _ = runtime_lib_dir(); // ensure liblpp_runtime.so exists

    let request = DriverRequest::from_args(
        [
            "lpp".to_string(),
            "run".to_string(),
            entry.to_string_lossy().into_owned(),
        ],
        dir.as_path(),
    )
    .expect("a request with an executable and a source file is valid");

    let mut session = CompilerSession::new(RewriteEngine);
    assert_eq!(session.engine_name(), "rewrite");
    let outcome = session.execute(&request);
    assert_eq!(
        outcome.exit_code(),
        0,
        "rewrite engine `run` should compile, link, and run cleanly"
    );
    let exe_name = if cfg!(target_os = "windows") {
        "main.exe"
    } else {
        "main"
    };
    assert!(
        dir.join(exe_name).exists(),
        "the engine should have built the executable next to the source"
    );
}

/// v1 struct-constructor parity: `Box()` with zero arguments zero-initializes
/// every primitive field (Int -> 0), matching the v1 oracle. Pins the vertical
/// fix across the type checker (zero-argument constructor arity) and MIR
/// (zero-value synthesis per concrete field type).
#[test]
fn zero_arg_struct_constructor_zero_initializes_fields() {
    let dir = workdir("zeroinit");
    let entry = dir.join("main.lpp");
    std::fs::write(
        &entry,
        "struct Box:\n    value: Int\n\ndef main() -> Int:\n    b := Box()\n    print_int(b.value)\n    return 0\n",
    )
    .unwrap();
    let exe = if cfg!(target_os = "windows") {
        dir.join("program.exe")
    } else {
        dir.join("program")
    };

    build_executable(
        &entry,
        "zeroinit",
        BackendChoice::Cranelift,
        &exe,
        &runtime_lib_dir(),
    )
    .expect("zero-argument struct construction must compile and link");

    let run = Command::new(&exe).output().expect("run the executable");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    assert_eq!(run.status.code(), Some(0), "exe exited non-zero:\n{stdout}");
    assert_eq!(stdout.replace("\r\n", "\n"), "0\n", "Box() should zero-initialize `value` to 0");
}
