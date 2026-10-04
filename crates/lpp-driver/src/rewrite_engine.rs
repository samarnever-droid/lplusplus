//! Stage-1 rewrite engine — the new `compile.rs` pipeline behind the
//! `CompilerEngine` contract, selectable with `LPP_ENGINE=rewrite`.
//!
//! The v1 compiler (`LegacyEngine`) stays the default. This engine implements
//! the *single-file source* path of the CLI — `lpp <file.lpp> [--backend …]
//! [-o out] [--run | --check | --emit-object]` — end to end through the rewrite
//! pipeline (HIR -> typed HIR -> MIR -> Cranelift/WASM/LLVM -> link against the
//! Rust runtime cdylib proven by the 6B.3e drop-in capstone).
//!
//! Package-manager subcommands (`build`/`run`/`test` against `lpp.toml`,
//! `install`, `config`, `setup`, …) are *not* reimplemented here yet; they defer
//! to v1 with a clear message and a distinct exit code. Closing that gap — and
//! the language-feature parity behind it — is the work tracked after cutover.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::compile::{
    BackendChoice, CompileError, build_executable, compile_entry, default_runtime_lib_dir,
};
use crate::{CompilerEngine, DriverContext, DriverOutcome, DriverRequest};

/// The rewrite compiler engine (stage 1 of the driver cutover).
pub struct RewriteEngine;

impl CompilerEngine for RewriteEngine {
    fn name(&self) -> &'static str {
        "rewrite"
    }

    fn execute(&self, request: &DriverRequest, _context: &mut DriverContext) -> DriverOutcome {
        DriverOutcome::from_exit_code(rewrite_main(
            request.arguments(),
            request.working_directory(),
        ))
    }
}

/// What the source path should do with the compiled module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Compile + link to an executable (the default).
    Build,
    /// Compile + link + run, propagating the program's exit code.
    Run,
    /// Compile to object only and report success (no link, no write).
    Check,
    /// Compile to object only and write the object/module file.
    EmitObject,
}

/// Subcommands owned by the package manager / toolchain rather than the
/// single-file source path. The rewrite engine defers these to v1.
const PM_COMMANDS: &[&str] = &[
    "init",
    "create",
    "new",
    "install",
    "add",
    "remove",
    "update",
    "search",
    "list",
    "tree",
    "metadata",
    "clean",
    "outdated",
    "version",
    "publish",
    "login",
    "upgrade",
    "self-update",
    "update-self",
    "workspace",
    "bench",
    "config",
    "setup",
    "toolchain",
    "help",
    "dev",
    "lreact",
];

/// Exit code used when the rewrite engine deliberately defers to v1.
const EXIT_DEFER: i32 = 2;

fn rewrite_main(args: &[String], cwd: &Path) -> i32 {
    // `args[0]` is the executable name; the CLI surface starts at `args[1]`.
    let rest: &[String] = if args.len() > 1 { &args[1..] } else { &[] };

    let mut mode = Mode::Build;
    let mut start = 0usize;

    // A leading verb mirrors v1's `emit`/`check`/`run`/`build <file.lpp>` forms.
    if let Some(first) = rest.first() {
        let second_is_source =
            rest.len() > 1 && (rest[1].ends_with(".lpp") || cwd.join(&rest[1]).exists());
        match first.as_str() {
            "emit" => {
                mode = Mode::EmitObject;
                start = 1;
            }
            "check" if second_is_source => {
                mode = Mode::Check;
                start = 1;
            }
            "run" if second_is_source => {
                mode = Mode::Run;
                start = 1;
            }
            "build" if second_is_source => {
                mode = Mode::Build;
                start = 1;
            }
            command if PM_COMMANDS.contains(&command) => return defer(command),
            _ => {}
        }
    }

    let mut filename: Option<String> = None;
    let mut backend = BackendChoice::Cranelift;
    let mut output: Option<String> = None;

    let mut idx = start;
    while idx < rest.len() {
        let arg = rest[idx].as_str();
        match arg {
            "--version" | "-v" => {
                println!("L++ rewrite engine (driver cutover stage 1)");
                return 0;
            }
            "--help" | "-h" => {
                print_help();
                return 0;
            }
            "--check" => mode = Mode::Check,
            "--run" => mode = Mode::Run,
            "--emit-object" | "--emit-obj" | "--aot" => mode = Mode::EmitObject,
            "--llvm" => backend = BackendChoice::Llvm,
            "--backend" => {
                if idx + 1 < rest.len() {
                    match BackendChoice::parse(&rest[idx + 1]) {
                        Some(choice) => backend = choice,
                        None => {
                            eprintln!(
                                "[rewrite] unknown backend `{}` (use cranelift, wasm, or llvm)",
                                rest[idx + 1]
                            );
                            return EXIT_DEFER;
                        }
                    }
                    idx += 1;
                }
            }
            "--target" => {
                if idx + 1 < rest.len() {
                    let triple = rest[idx + 1].as_str();
                    if triple.starts_with("wasm") {
                        backend = BackendChoice::Wasm;
                    } else if !(triple.starts_with("x86_64") || triple == "host") {
                        eprintln!(
                            "[rewrite] stage 1 targets host (x86_64) and wasm32-wasi; `{triple}` needs the v1 engine."
                        );
                        return EXIT_DEFER;
                    }
                    idx += 1;
                }
            }
            "-o" | "--output" => {
                if idx + 1 < rest.len() {
                    output = Some(rest[idx + 1].clone());
                    idx += 1;
                }
            }
            // The rewrite link step always uses the host `cc`; accept and ignore
            // an explicit linker choice so the same command line still works.
            "--linker" => {
                if idx + 1 < rest.len() {
                    idx += 1;
                }
            }
            flag if flag.starts_with("--dump-") || flag == "--checkall" || flag == "--fix" => {
                eprintln!(
                    "[rewrite] `{flag}` is not yet supported by the rewrite engine; unset LPP_ENGINE to use v1."
                );
                return EXIT_DEFER;
            }
            positional if !positional.starts_with('-') => filename = Some(positional.to_string()),
            _ => {}
        }
        idx += 1;
    }

    let Some(file) = filename else {
        return no_input();
    };
    let entry = if Path::new(&file).is_absolute() {
        PathBuf::from(&file)
    } else {
        cwd.join(&file)
    };
    if !entry.exists() {
        eprintln!("[rewrite] input file not found: {}", entry.display());
        return 1;
    }
    let stem = entry
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("program")
        .to_string();

    // The WASM backend emits a self-contained module — there is no `cc` link.
    if backend == BackendChoice::Wasm {
        return match compile_entry(&entry, &stem, backend) {
            Ok(module) => {
                let out = output
                    .map(PathBuf::from)
                    .unwrap_or_else(|| cwd.join(format!("{stem}.wasm")));
                if let Err(error) = std::fs::write(&out, &module.object) {
                    eprintln!("[rewrite] failed to write {}: {error}", out.display());
                    return 1;
                }
                println!("[rewrite] emitted wasm module -> {}", out.display());
                if mode == Mode::Run { run_wasm(&out) } else { 0 }
            }
            Err(error) => compile_failed(&error),
        };
    }

    // Native backends (Cranelift default, LLVM partial).
    match mode {
        Mode::Check => match compile_entry(&entry, &stem, backend) {
            Ok(module) => {
                println!(
                    "[rewrite] check OK — {} bytes of object code generated",
                    module.object.len()
                );
                0
            }
            Err(error) => compile_failed(&error),
        },
        Mode::EmitObject => match compile_entry(&entry, &stem, backend) {
            Ok(module) => {
                let out = output
                    .map(PathBuf::from)
                    .unwrap_or_else(|| cwd.join(format!("{stem}.o")));
                if let Err(error) = std::fs::write(&out, &module.object) {
                    eprintln!("[rewrite] failed to write {}: {error}", out.display());
                    return 1;
                }
                println!("[rewrite] emitted object -> {}", out.display());
                0
            }
            Err(error) => compile_failed(&error),
        },
        Mode::Build | Mode::Run => {
            let exe = output.map(PathBuf::from).unwrap_or_else(|| cwd.join(&stem));
            let runtime_dir = default_runtime_lib_dir();
            if !runtime_dir.join("liblpp_runtime.so").exists() {
                eprintln!(
                    "[rewrite] runtime library not found in {} — build it first with `cargo build -p lpp-runtime`.",
                    runtime_dir.display()
                );
                return 1;
            }
            match build_executable(&entry, &stem, backend, &exe, &runtime_dir) {
                Ok(()) => {
                    if mode == Mode::Run {
                        match Command::new(&exe).status() {
                            Ok(status) => status.code().unwrap_or(0),
                            Err(error) => {
                                eprintln!("[rewrite] failed to run {}: {error}", exe.display());
                                1
                            }
                        }
                    } else {
                        println!("[rewrite] built -> {}", exe.display());
                        0
                    }
                }
                Err(error) => compile_failed(&error),
            }
        }
    }
}

fn compile_failed(error: &CompileError) -> i32 {
    eprintln!("[rewrite] compile error: {error}");
    1
}

fn defer(command: &str) -> i32 {
    eprintln!(
        "[rewrite] `{command}` is a package-manager command not yet implemented by the rewrite engine (cutover stage 1)."
    );
    eprintln!(
        "[rewrite] Unset LPP_ENGINE (or set LPP_ENGINE=legacy) to run it with the v1 compiler."
    );
    EXIT_DEFER
}

fn no_input() -> i32 {
    eprintln!(
        "[rewrite] no input `.lpp` file given. The rewrite engine (cutover stage 1) compiles a single source file:"
    );
    eprintln!(
        "        lpp <file.lpp> [--backend cranelift|wasm|llvm] [-o <out>] [--run | --check | --emit-object]"
    );
    eprintln!(
        "        Package commands (build/run/test against lpp.toml, install, config, …) still require v1: unset LPP_ENGINE."
    );
    EXIT_DEFER
}

fn run_wasm(module: &Path) -> i32 {
    if let Ok(status) = Command::new("wasmtime").arg(module).status() {
        return status.code().unwrap_or(0);
    }
    eprintln!(
        "[rewrite] built the wasm module but found no runtime to execute it (install wasmtime)."
    );
    eprintln!("        module: {}", module.display());
    0
}

fn print_help() {
    println!("L++ rewrite engine — driver cutover stage 1");
    println!();
    println!("Usage: lpp <file.lpp> [options]      (selected with LPP_ENGINE=rewrite)");
    println!();
    println!("  --backend <be>   cranelift (default), wasm, or llvm");
    println!("  --llvm           shorthand for --backend llvm");
    println!("  --target <t>     host (x86_64) or wasm32-wasi");
    println!("  -o, --output     output path");
    println!("  --run            compile, link, and run");
    println!("  --check          compile to object only; report success");
    println!("  --emit-object    write the object/module file only");
    println!();
    println!("Package-manager commands are not yet implemented here; unset LPP_ENGINE for v1.");
}
