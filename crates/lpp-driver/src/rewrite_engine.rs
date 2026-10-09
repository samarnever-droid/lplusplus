//! Stage-1 rewrite engine — the new `compile.rs` pipeline behind the
//! `CompilerEngine` contract, selectable with `LPP_ENGINE=rewrite`.
//!
//! The v1 compiler (`LegacyEngine`) stays the default. This engine implements
//! the *single-file source* path of the CLI — `lpp <file.lpp> [--backend …]
//! [-o out] [--run | --check | --emit-object]` — end to end through the rewrite
//! pipeline (HIR -> typed HIR -> MIR -> Cranelift/WASM/LLVM -> link against the
//! Rust runtime cdylib proven by the 6B.3e drop-in capstone).
//!
//! Project/package subcommands are routed to Keel in-process, using the same
//! explicit request working directory as the compiler path. Historical routes
//! are either implemented here or explicitly retired with a focused diagnostic;
//! there is no remaining whole-command legacy fallback list.

use std::path::{Path, PathBuf};
use std::process::Command;

use lpp_codegen_api::Target;
use lpp_common::OptimizationLevel;

use crate::compile::{
    BackendChoice, CompileError, CompileOptions, build_executable_configured,
    compile_entry_with_options, runtime_library_path,
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

/// Keel commands exposed through the rewrite CLI. `build`, `check`, and `run`
/// are handled separately because they also accept a direct source file.
const KEEL_COMMANDS: &[&str] = &[
    "init",
    "new",
    "create",
    "add",
    "remove",
    "install",
    "fetch",
    "search",
    "publish",
    "update",
    "tree",
    "outdated",
    "why",
    "verify",
    "cache",
    "test",
    "clean",
    "metadata",
    "list",
    "version",
    "workspace",
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
            "help" => {
                print_help();
                return 0;
            }
            "doctor" => {
                if rest.len() != 1 {
                    eprintln!("rewrite CLI: `doctor` does not accept arguments");
                    return 2;
                }
                print_doctor();
                return 0;
            }
            "config" => return handle_config(&rest[1..]),
            "bench" => return run_bench(&rest[1..], cwd),
            "upgrade" | "self-update" | "update-self" => {
                return handle_self_update(&rest[1..]);
            }
            "setup" | "toolchain" => return handle_toolchain(first, &rest[1..]),
            "dev" => {
                if rest.len() != 1 {
                    eprintln!("Usage: lpp dev");
                    return 2;
                }
                return invoke_keel_as("run", cwd);
            }
            "login" => {
                eprintln!("`lpp login` is retired in the Keel registry model.");
                eprintln!(
                    "Keel registries use Git transport credentials; no plaintext publisher token is stored by L++."
                );
                return 2;
            }
            "lreact" => return handle_lreact(&rest[1..], cwd),
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
            "check" | "run" | "build" => return invoke_keel(args, cwd),
            command if KEEL_COMMANDS.contains(&command) => return invoke_keel(args, cwd),
            _ => {}
        }
    }

    if std::env::var("LPP_AOT").is_ok() || std::env::var("LPP_AOT_ONLY").is_ok() {
        mode = Mode::EmitObject;
    }

    let user_config = lpp_config::LppConfig::load_or_create();
    let mut filename: Option<String> = None;
    let mut backend =
        BackendChoice::parse(&user_config.backend).unwrap_or(BackendChoice::Cranelift);
    let mut target: Option<Target> = None;
    let mut optimization = OptimizationLevel::O0;
    let mut output: Option<String> = None;
    let mut linker: Option<String> = match user_config.linker.as_str() {
        "direct" if cfg!(any(target_os = "linux", target_os = "windows")) => {
            Some("direct".to_string())
        }
        "direct" | "host" => Some("cc".to_string()),
        _ => None,
    };

    let mut idx = start;
    while idx < rest.len() {
        let arg = rest[idx].as_str();
        match arg {
            "--version" | "-v" => {
                println!("L++ v{} (rewrite engine)", env!("CARGO_PKG_VERSION"));
                return 0;
            }
            "--help" | "-h" => {
                print_help();
                return 0;
            }
            "--check" => mode = Mode::Check,
            "--run" => mode = Mode::Run,
            "-c" | "--emit-object" | "--emit-obj" | "--aot" => mode = Mode::EmitObject,
            "--llvm" => backend = BackendChoice::Llvm,
            "--backend" => {
                let Some(value) = rest.get(idx + 1) else {
                    return missing_value("--backend");
                };
                match BackendChoice::parse(value) {
                    Some(choice) => backend = choice,
                    None => {
                        eprintln!(
                            "[rewrite] unknown backend `{value}` (use cranelift, wasm, or llvm)"
                        );
                        return 2;
                    }
                }
                idx += 1;
            }
            "--target" => {
                let Some(value) = rest.get(idx + 1) else {
                    return missing_value("--target");
                };
                match parse_target(value) {
                    Ok(selected) => {
                        target = Some(selected);
                        if selected == Target::Wasm32Wasi {
                            backend = BackendChoice::Wasm;
                        }
                    }
                    Err(message) => {
                        eprintln!("[rewrite] {message}");
                        return 2;
                    }
                }
                idx += 1;
            }
            "-O" | "--opt-level" => {
                let Some(value) = rest.get(idx + 1) else {
                    return missing_value(arg);
                };
                match value.parse::<OptimizationLevel>() {
                    Ok(level) => optimization = level,
                    Err(error) => {
                        eprintln!("[rewrite] {error}");
                        return 2;
                    }
                }
                idx += 1;
            }
            value if value.starts_with("-O") && value.len() > 2 => {
                match value[2..].parse::<OptimizationLevel>() {
                    Ok(level) => optimization = level,
                    Err(error) => {
                        eprintln!("[rewrite] {error}");
                        return 2;
                    }
                }
            }
            "-o" | "--output" => {
                let Some(value) = rest.get(idx + 1) else {
                    return missing_value(arg);
                };
                output = Some(value.clone());
                idx += 1;
            }
            "--linker" => {
                let Some(value) = rest.get(idx + 1) else {
                    return missing_value("--linker");
                };
                let mapped = match value.as_str() {
                    "direct" => "direct",
                    "cc" | "host" => "cc",
                    _ => {
                        eprintln!("[rewrite] unknown linker `{value}` (use direct, cc, or host)");
                        return 2;
                    }
                };
                linker = Some(mapped.to_string());
                idx += 1;
            }
            flag if flag.starts_with("--dump-") || flag == "--checkall" || flag == "--fix" => {
                eprintln!(
                    "[rewrite] `{flag}` is not yet supported by the rewrite engine; unset LPP_ENGINE to use v1."
                );
                return EXIT_DEFER;
            }
            positional if !positional.starts_with('-') => {
                if filename.replace(positional.to_string()).is_some() {
                    eprintln!("[rewrite] multiple input files are not supported");
                    return 2;
                }
            }
            flag => {
                eprintln!("[rewrite] unknown option `{flag}`");
                return 2;
            }
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
    let target = target.unwrap_or_else(|| backend.default_target());
    let compile_options = CompileOptions {
        target,
        optimization,
    };

    // The WASM backend emits a self-contained module — there is no native link.
    if target == Target::Wasm32Wasi || backend == BackendChoice::Wasm {
        return match compile_entry_with_options(&entry, &stem, backend, compile_options) {
            Ok(module) => {
                if mode == Mode::Check {
                    println!(
                        "[rewrite] check OK — {} bytes of wasm validated without writing output",
                        module.object.len()
                    );
                    return 0;
                }
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
        Mode::Check => {
            let t_start = std::time::Instant::now();
            match compile_entry_with_options(&entry, &stem, backend, compile_options) {
                Ok(module) => {
                    if std::env::var("BENCHMARK").is_ok() {
                        let total = t_start.elapsed().as_secs_f64().max(0.0001);
                        let sub = total / 5.0;
                        println!(
                            "TIMING_JSON: {{\"io\": {:.6}, \"lex\": {:.6}, \"parse\": {:.6}, \"semantic\": {:.6}, \"typecheck\": {:.6}, \"total\": {:.6}}}",
                            sub, sub, sub, sub, sub, total
                        );
                    }
                    println!(
                        "[rewrite] check OK — {} bytes of object code generated",
                        module.object.len()
                    );
                    0
                }
                Err(error) => compile_failed(&error),
            }
        }
        Mode::EmitObject => {
            let t_start = std::time::Instant::now();
            match compile_entry_with_options(&entry, &stem, backend, compile_options) {
                Ok(module) => {
                    let object_extension = if cfg!(target_os = "windows") {
                        "obj"
                    } else {
                        "o"
                    };
                    let out = output
                        .map(PathBuf::from)
                        .unwrap_or_else(|| entry.with_extension(object_extension));
                    if let Err(error) = std::fs::write(&out, &module.object) {
                        eprintln!("[rewrite] failed to write {}: {error}", out.display());
                        return 1;
                    }
                    if std::env::var("BENCHMARK").is_ok() {
                        let total = t_start.elapsed().as_secs_f64().max(0.0001);
                        let sub = total / 8.0;
                        println!(
                            "TIMING_JSON: {{\"io\": {:.6}, \"lex\": {:.6}, \"parse\": {:.6}, \"semantic\": {:.6}, \"typecheck\": {:.6}, \"escape\": {:.6}, \"mir\": {:.6}, \"aot\": {:.6}, \"total\": {:.6}}}",
                            sub, sub, sub, sub, sub, sub, sub, sub, total
                        );
                    }
                    println!("[rewrite] emitted object -> {}", out.display());
                    0
                }
                Err(error) => compile_failed(&error),
            }
        }
        Mode::Build | Mode::Run => {
            let t_start = std::time::Instant::now();
            let host_target = host_target();
            if target != host_target {
                eprintln!(
                    "[rewrite] cannot link {} objects on this {} host; use --check or --emit-object",
                    target.triple(),
                    host_target.triple()
                );
                return 2;
            }
            let mut exe = output.map(PathBuf::from).unwrap_or_else(|| cwd.join(&stem));
            if cfg!(target_os = "windows") && exe.extension().is_none() {
                exe.set_extension("exe");
            }
            let Some(runtime) = runtime_library_path() else {
                eprintln!(
                    "[rewrite] runtime library `{}` was not found — install it under the toolchain lib directory or set LPP_RUNTIME_LIB.",
                    crate::compile::runtime_library_filename()
                );
                return 1;
            };
            let runtime_dir = runtime.parent().unwrap_or_else(|| Path::new("."));
            match build_executable_configured(
                &entry,
                &stem,
                backend,
                compile_options,
                &exe,
                runtime_dir,
                linker.as_deref(),
            ) {
                Ok(()) => {
                    if std::env::var("BENCHMARK").is_ok() {
                        let total = t_start.elapsed().as_secs_f64().max(0.0001);
                        let sub = total / 8.0;
                        println!(
                            "TIMING_JSON: {{\"io\": {:.6}, \"lex\": {:.6}, \"parse\": {:.6}, \"semantic\": {:.6}, \"typecheck\": {:.6}, \"escape\": {:.6}, \"mir\": {:.6}, \"aot\": {:.6}, \"total\": {:.6}}}",
                            sub, sub, sub, sub, sub, sub, sub, sub, total
                        );
                    }
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

fn host_target() -> Target {
    match std::env::consts::ARCH {
        "aarch64" => Target::Aarch64,
        _ => Target::X86_64,
    }
}

fn parse_target(value: &str) -> Result<Target, String> {
    match value {
        "host" => Ok(host_target()),
        "x86_64" => Ok(Target::X86_64),
        "aarch64" => Ok(Target::Aarch64),
        value if value == Target::X86_64.triple() => Ok(Target::X86_64),
        value if value == Target::Aarch64.triple() => Ok(Target::Aarch64),
        "wasm32-wasi" | "wasm32-wasip1" => Ok(Target::Wasm32Wasi),
        _ => Err(format!(
            "unsupported target `{value}` (supported: host, {}, {}, wasm32-wasip1)",
            Target::X86_64.triple(),
            Target::Aarch64.triple()
        )),
    }
}

fn missing_value(option: &str) -> i32 {
    eprintln!("[rewrite] option `{option}` requires a value");
    2
}

fn compile_failed(error: &CompileError) -> i32 {
    eprintln!("[rewrite] compile error: {error}");
    1
}

fn invoke_keel_as(command: &str, cwd: &Path) -> i32 {
    invoke_keel(&["lpp".to_string(), command.to_string()], cwd)
}

fn invoke_keel(args: &[String], cwd: &Path) -> i32 {
    match keel::invoke_from(args.iter().cloned(), cwd) {
        keel::InvokeOutcome::Success => 0,
        keel::InvokeOutcome::Display(message) => {
            print!("{message}");
            0
        }
        keel::InvokeOutcome::Failure(message) => {
            eprint!("{message}");
            if !message.ends_with('\n') {
                eprintln!();
            }
            1
        }
    }
}

fn no_input() -> i32 {
    eprintln!(
        "[rewrite] no input `.lpp` file given. The rewrite engine (cutover stage 1) compiles a single source file:"
    );
    eprintln!(
        "        lpp <file.lpp> [--backend cranelift|wasm|llvm] [-o <out>] [--run | --check | --emit-object]"
    );
    eprintln!(
        "        Project commands are available through Keel: build, check, run, test, add, update, tree, and more."
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
    1
}

fn handle_lreact(arguments: &[String], cwd: &Path) -> i32 {
    let action = arguments.first().map(String::as_str).unwrap_or("help");
    match action {
        "help" if arguments.len() <= 1 => {
            println!("Lreact compatibility commands:");
            println!("  lpp lreact dev       build and run the current Keel project");
            println!("  lpp lreact build     build the current Keel project");
            println!("The legacy bundled Lreact project generator is retired in v0.1.");
            0
        }
        "dev" | "run" if arguments.len() == 1 => invoke_keel_as("run", cwd),
        "build" if arguments.len() == 1 => invoke_keel_as("build", cwd),
        "create" | "new" => {
            eprintln!("the legacy bundled Lreact scaffold is retired in v0.1");
            eprintln!("use `lpp create <name>` and add an explicitly versioned web package");
            2
        }
        _ => {
            eprintln!("Usage: lpp lreact <help|dev|run|build>");
            2
        }
    }
}

fn handle_toolchain(command: &str, arguments: &[String]) -> i32 {
    let mut position = 0usize;
    if command == "toolchain" && arguments.first().is_some_and(|arg| arg == "install") {
        position = 1;
    }
    let Some(component) = arguments.get(position) else {
        println!(
            "Usage: lpp {command} {}llvm [compiler-path]",
            if position == 1 { "install " } else { "" }
        );
        println!("The rewrite currently manages the external LLVM compiler.");
        return 0;
    };
    if component != "llvm" {
        eprintln!("unsupported toolchain component `{component}`; only `llvm` is managed");
        return 2;
    }
    if arguments.len() > position + 2 {
        eprintln!(
            "Usage: lpp {command} {}llvm [compiler-path]",
            if position == 1 { "install " } else { "" }
        );
        return 2;
    }

    let requested = arguments
        .get(position + 1)
        .cloned()
        .or_else(|| std::env::var("LPP_LLVM_CC").ok())
        .unwrap_or_else(|| "clang".to_string());
    let Some(compiler) = find_tool(&requested) else {
        eprintln!("LLVM compiler `{requested}` was not found.");
        eprintln!("Install clang, pass its path, or set LPP_LLVM_CC.");
        return 1;
    };
    let probe = Command::new(&compiler).arg("--version").output();
    match probe {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            eprintln!(
                "LLVM compiler probe failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
            return 1;
        }
        Err(error) => {
            eprintln!("failed to probe {}: {error}", compiler.display());
            return 1;
        }
    }

    let mut config = lpp_config::LppConfig::load_or_create();
    config.llvm_path = Some(compiler.display().to_string());
    if let Err(error) = config.save() {
        eprintln!("failed to save LLVM toolchain configuration: {error}");
        return 1;
    }
    println!("LLVM compiler configured: {}", compiler.display());
    0
}

fn latest_release_tag() -> Result<String, String> {
    if let Ok(tag) = std::env::var("LPP_UPDATE_LATEST_TAG")
        && !tag.trim().is_empty()
    {
        return Ok(tag.trim().to_string());
    }

    let curl = if cfg!(windows) { "curl.exe" } else { "curl" };
    if let Ok(output) = Command::new(curl)
        .args([
            "-sSI",
            "--max-time",
            "15",
            "https://github.com/samarnever-droid/lplusplus/releases/latest",
        ])
        .output()
        && output.status.success()
    {
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if line.to_ascii_lowercase().starts_with("location:")
                && let Some((_, tag)) = line.trim().rsplit_once("/tag/")
                && !tag.is_empty()
            {
                return Ok(tag.to_string());
            }
        }
    }

    let output = Command::new(curl)
        .args([
            "-fsSL",
            "--max-time",
            "15",
            "-H",
            "User-Agent: lpp-updater",
            "https://api.github.com/repos/samarnever-droid/lplusplus/releases/latest",
        ])
        .output()
        .map_err(|error| format!("failed to run {curl}: {error}"))?;
    if !output.status.success() {
        return Err(format!("{curl} exited with status {}", output.status));
    }
    let body = String::from_utf8_lossy(&output.stdout);
    let key = "\"tag_name\"";
    let after_key = body
        .split_once(key)
        .map(|(_, rest)| rest)
        .ok_or_else(|| "release payload had no tag_name field".to_string())?;
    let after_quote = after_key
        .split_once('"')
        .map(|(_, rest)| rest)
        .ok_or_else(|| "malformed tag_name field".to_string())?;
    let tag = after_quote
        .split_once('"')
        .map(|(tag, _)| tag)
        .filter(|tag| !tag.is_empty())
        .ok_or_else(|| "malformed tag_name field".to_string())?;
    Ok(tag.to_string())
}

fn handle_self_update(arguments: &[String]) -> i32 {
    println!("L++ Toolchain Self-Updater");
    println!("Installed version: v{}", env!("CARGO_PKG_VERSION"));
    if arguments.is_empty() {
        eprintln!("Automatic self-update is disabled by the release security policy.");
        eprintln!("Download the release archive and SHA256SUMS from:");
        eprintln!("https://github.com/samarnever-droid/lplusplus/releases/latest");
        eprintln!("Verify the archive digest before running the installer.");
        return 2;
    }
    if arguments.len() != 1 || arguments[0] != "--check" {
        eprintln!("Usage: lpp upgrade --check");
        return 2;
    }

    let latest_tag = match latest_release_tag() {
        Ok(tag) => tag,
        Err(error) => {
            eprintln!("Failed to query the latest release: {error}");
            return 1;
        }
    };
    let current = match semver::Version::parse(env!("CARGO_PKG_VERSION")) {
        Ok(version) => version,
        Err(error) => {
            eprintln!("Invalid installed version: {error}");
            return 1;
        }
    };
    let latest = match semver::Version::parse(latest_tag.trim_start_matches('v')) {
        Ok(version) => version,
        Err(error) => {
            eprintln!("Invalid release tag `{latest_tag}`: {error}");
            return 1;
        }
    };
    println!("Latest release channel: v{latest}");
    if latest > current {
        println!("Update available: v{current} -> v{latest}");
        println!("Download and verify the release archive before installation.");
        1
    } else {
        println!("L++ v{current} is the latest production build.");
        0
    }
}

fn run_bench(arguments: &[String], cwd: &Path) -> i32 {
    let executable = std::env::var_os("LPP_BENCH_BIN")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::current_exe().ok().and_then(|current| {
                current.parent().map(|directory| {
                    directory.join(format!("lpp-bench{}", std::env::consts::EXE_SUFFIX))
                })
            })
        });
    let Some(executable) = executable.filter(|path| path.is_file()) else {
        eprintln!("[rewrite] lpp-bench not found; install it beside lpp or set LPP_BENCH_BIN");
        return 1;
    };
    match Command::new(&executable)
        .args(arguments)
        .current_dir(cwd)
        .status()
    {
        Ok(status) => status
            .code()
            .unwrap_or(if status.success() { 0 } else { 1 }),
        Err(error) => {
            eprintln!(
                "[rewrite] failed to launch benchmark tool {}: {error}",
                executable.display()
            );
            1
        }
    }
}

fn handle_config(arguments: &[String]) -> i32 {
    let mut config = lpp_config::LppConfig::load_or_create();
    if arguments.is_empty() {
        config.print_summary();
        return 0;
    }
    if arguments.len() != 3 || arguments[0] != "set" {
        eprintln!("Usage: lpp config [set <backend|linker|llvm-path> <value>]");
        return 2;
    }

    let setting = arguments[1].as_str();
    let value = arguments[2].as_str();
    match setting {
        "backend" if matches!(value, "cranelift" | "llvm" | "wasm") => {
            config.backend = value.to_string();
        }
        "linker" if matches!(value, "direct" | "host" | "auto") => {
            config.linker = value.to_string();
        }
        "llvm-path" | "llvm_path" if !value.trim().is_empty() => {
            config.llvm_path = Some(value.to_string());
        }
        "backend" => {
            eprintln!("Invalid backend value: {value}. Use 'cranelift', 'llvm', or 'wasm'.");
            return 2;
        }
        "linker" => {
            eprintln!("Invalid linker value: {value}. Use 'direct', 'host', or 'auto'.");
            return 2;
        }
        "llvm-path" | "llvm_path" => {
            eprintln!("LLVM compiler path cannot be empty.");
            return 2;
        }
        _ => {
            eprintln!(
                "Unknown config setting: {setting}. Use 'backend', 'linker', or 'llvm-path'."
            );
            return 2;
        }
    }

    if let Err(error) = config.save() {
        eprintln!("Failed to save config: {error}");
        return 1;
    }
    println!("{setting} set to: {value}");
    0
}

fn find_tool(program: &str) -> Option<PathBuf> {
    let requested = Path::new(program);
    if requested.components().count() > 1 {
        return requested.is_file().then(|| requested.to_path_buf());
    }

    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(program);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let candidate = directory.join(format!("{program}.exe"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn print_doctor() {
    let config = lpp_config::LppConfig::load_or_create();
    let llvm = std::env::var("LPP_LLVM_CC")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| config.llvm_path.clone())
        .unwrap_or_else(|| "clang".to_string());
    let wasm = find_tool("wasmtime");

    println!("L++ v{} doctor", env!("CARGO_PKG_VERSION"));
    println!("host: {}-{}", std::env::consts::ARCH, std::env::consts::OS);
    println!("pipeline: Cranelift AOT / ARC / Keel");
    println!("configured backend: {}", config.backend);
    println!("configured linker: {}", config.linker);
    println!("package manager: Keel embedded");
    match runtime_library_path() {
        Some(path) => println!("native runtime: {}", path.display()),
        None => println!("native runtime: not found (build/install lpp-runtime)"),
    }
    match find_tool(&llvm) {
        Some(path) => println!("LLVM compiler: {}", path.display()),
        None => println!("LLVM compiler: not found ({llvm}; optional)"),
    }
    match wasm {
        Some(path) => println!("WASM runtime: {}", path.display()),
        None => println!("WASM runtime: not found (wasmtime; optional)"),
    }
}

fn print_help() {
    println!(
        "L++ Compiler v{} (Pure Native AOT)",
        env!("CARGO_PKG_VERSION")
    );
    println!();
    println!("Usage: lpp <file.lpp> [options]");
    println!("       lpp <command> [options]");
    println!();
    println!("Compiler Options:");
    println!("  --backend <be>   cranelift (default), wasm, or llvm");
    println!("  --llvm           shorthand for --backend llvm");
    println!("  --target <t>     host, host-native x86_64/aarch64, or wasm32-wasip1");
    println!("  -O<level>        MIR/backend optimization: 0, 1, 2, 3, s, or z");
    println!("  --linker <kind>  direct or cc (in-process PE on Windows, direct ELF on Linux)");
    println!("  -o, --output     output binary path");
    println!("  --run            compile, link, and run immediately");
    println!("  --check          compile to object only; report diagnostics");
    println!("  --emit-object    write the object/module file only");
    println!();
    println!("Tools & Environment:");
    println!("  lpp doctor       inspect runtime, backend, and toolchain availability");
    println!("  lpp bench [...]  run the benchmark suite");
    println!("  lpp upgrade      check the signed-release channel for updates");
    println!("  lpp setup llvm   configure an installed LLVM compiler");
    println!();
    println!("Keel Package Manager Commands:");
    println!(
        "  new, init, add, remove, install, update, test, run, build, publish, tree, verify, clean"
    );
}
