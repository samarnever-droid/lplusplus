#[path = "frontend/ast.rs"]
mod ast;
mod builtins;
mod config;
mod diagnostics;
#[path = "backend/cranelift/mod.rs"]
pub mod cranelift_backend;
#[path = "backend/llvm.rs"]
mod llvm_backend;
#[path = "backend/wasm.rs"]
mod wasm_backend;
#[path = "analysis/cyclebreak.rs"]
mod cyclebreak;
#[path = "analysis/monomorph.rs"]
mod monomorph;
#[path = "frontend/lexer.rs"]
mod lexer;
#[path = "mir/mod.rs"]
pub mod mir;
#[path = "frontend/parser.rs"]
mod parser;
mod pm;
#[path = "analysis/semantic.rs"]
mod semantic;
#[path = "analysis/types.rs"]
mod types;
#[path = "analysis/typecheck.rs"]
mod typecheck;
mod target;
#[path = "analysis/type_facts.rs"]
mod type_facts;
#[path = "analysis/layout.rs"]
mod layout;
pub mod linker;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn resolve_pm_source() -> Option<PathBuf> {
    for var in &["LPP_HOME", "LPP_DIR"] {
        if let Ok(val) = env::var(var) {
            let candidate = PathBuf::from(val).join("pm/src/main.lpp");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    if let Ok(exe_path) = env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let candidates = [
                exe_dir.join("pm/src/main.lpp"),
                exe_dir.join("../pm/src/main.lpp"),
                exe_dir.join("../../pm/src/main.lpp"),
                exe_dir.join("../../../pm/src/main.lpp"),
            ];
            for c in &candidates {
                if c.exists() {
                    return Some(c.clone());
                }
            }
        }
    }

    if let Ok(home) = env::var("HOME").or_else(|_| env::var("USERPROFILE")) {
        let home_pm = PathBuf::from(home).join(".lpp/pm/src/main.lpp");
        if home_pm.exists() {
            return Some(home_pm);
        }
    }

    let cwd_pm = PathBuf::from("pm/src/main.lpp");
    if cwd_pm.exists() {
        return Some(cwd_pm);
    }

    None
}

fn resolve_runtime_source_for_bootstrap(pm_main: &Path) -> Option<PathBuf> {
    if let Some(root) = pm_main.parent().and_then(|p| p.parent()).and_then(|p| p.parent()) {
        let rt = root.join("lpp_runtime.c");
        if rt.exists() {
            return Some(rt);
        }
    }

    for var in &["LPP_HOME", "LPP_DIR"] {
        if let Ok(val) = env::var(var) {
            let rt = PathBuf::from(val).join("lpp_runtime.c");
            if rt.exists() {
                return Some(rt);
            }
        }
    }

    if let Ok(exe_path) = env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let candidates = [
                exe_dir.join("lpp_runtime.c"),
                exe_dir.join("../lpp_runtime.c"),
                exe_dir.join("../../lpp_runtime.c"),
                exe_dir.join("../../../lpp_runtime.c"),
            ];
            for c in &candidates {
                if c.exists() {
                    return Some(c.clone());
                }
            }
        }
    }

    if let Ok(home) = env::var("HOME").or_else(|_| env::var("USERPROFILE")) {
        let home_rt = PathBuf::from(home).join(".lpp/lpp_runtime.c");
        if home_rt.exists() {
            return Some(home_rt);
        }
    }

    let cwd_rt = PathBuf::from("lpp_runtime.c");
    if cwd_rt.exists() {
        return Some(cwd_rt);
    }

    None
}

fn resolve_pm_cache_dir() -> PathBuf {
    if let Ok(var) = env::var("LPP_HOME").or_else(|_| env::var("LPP_DIR")) {
        return PathBuf::from(var).join("cache");
    }
    if let Ok(home) = env::var("HOME").or_else(|_| env::var("USERPROFILE")) {
        return PathBuf::from(home).join(".lpp").join("cache");
    }
    env::temp_dir().join(".lpp_cache")
}

/// Bootstrap the self-hosted L++ PM: compile pm/src/main.lpp → cached binary.
/// Returns the path to the cached PM binary, or an error string.
fn bootstrap_self_hosted_pm() -> Result<PathBuf, String> {
    let lpp_bin = env::current_exe()
        .map_err(|e| format!("cannot locate lpp binary: {e}"))?;

    let pm_main = resolve_pm_source()
        .ok_or_else(|| "cannot locate pm/src/main.lpp".to_string())?;

    let cache_dir = resolve_pm_cache_dir();
    let _ = fs::create_dir_all(&cache_dir);

    let pm_bin = cache_dir.join(format!("lpp-pm{}", env::consts::EXE_SUFFIX));

    // Check if already built and up-to-date
    if pm_bin.exists() && pm_main.exists() {
        let bin_meta = fs::metadata(&pm_bin).ok();
        let src_meta = fs::metadata(&pm_main).ok();
        if let (Some(b), Some(s)) = (bin_meta, src_meta) {
            if let (Ok(bt), Ok(st)) = (b.modified(), s.modified()) {
                if bt >= st {
                    return Ok(pm_bin);
                }
            }
        }
    }

    eprintln!("[L++] Bootstrapping self-hosted PM...");

    // Compile pm/src/main.lpp → pm_obj
    let status = std::process::Command::new(&lpp_bin)
        .env("LPP_AOT", "1")
        .env("LPP_AOT_ONLY", "1")
        .env("BENCHMARK", "1")
        .arg(&pm_main)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        .status()
        .map_err(|e| format!("failed to spawn lpp compiler: {e}"))?;

    if !status.success() {
        return Err("self-hosted PM compilation failed".to_string());
    }

    let obj_ext = if cfg!(target_os = "windows") { "obj" } else { "o" };
    let pm_obj = pm_main.with_extension(obj_ext);
    if !pm_obj.exists() {
        return Err(format!("{} not generated", pm_obj.display()));
    }

    let mut link_ok = false;

    let lpp_link_bin = lpp_bin
        .parent()
        .map(|dir| dir.join(format!("lpp-link{}", env::consts::EXE_SUFFIX)))
        .unwrap_or_else(|| PathBuf::from(format!("lpp-link{}", env::consts::EXE_SUFFIX)));

    if lpp_link_bin.exists() {
        if let Some(runtime_src) = resolve_runtime_source_for_bootstrap(&pm_main) {
            let runtime_min_name = if cfg!(target_os = "windows") { "lpp_runtime_min.obj" } else { "lpp_runtime_min.o" };
            let lib_dir = runtime_src.parent().unwrap_or_else(|| Path::new("."));
            let runtime_min_obj = lib_dir.join(runtime_min_name);
            if runtime_min_obj.exists() {
                let mut link_cmd = std::process::Command::new(&lpp_link_bin);
                if cfg!(target_os = "windows") {
                    link_cmd.arg("pe");
                } else if cfg!(target_os = "macos") {
                    link_cmd.arg("macho");
                }
                link_cmd
                    .arg(&pm_obj)
                    .arg(&runtime_min_obj)
                    .arg("-o")
                    .arg(&pm_bin);

                if let Ok(st) = link_cmd
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::inherit())
                    .status()
                {
                    if st.success() {
                        link_ok = true;
                    }
                }
            }
        }
    }

    if !link_ok {
        #[cfg(windows)]
        pm::load_msvc_env();

        if pm::host_link_binary(&pm_obj, &pm_bin, &[]).is_ok() {
            link_ok = true;
        }
    }

    let _ = fs::remove_file(&pm_obj);

    if !link_ok {
        return Err("linking self-hosted PM failed".to_string());
    }

    Ok(pm_bin)
}

/// Run a PM command. The production-compatible Rust PM is used by default;
/// setting LPP_SELF_HOSTED_PM=1 opts into the experimental pure-L++ delegate.
/// If that delegate is unavailable or signals `__DELEGATE__`, the Rust PM takes over.
fn run_self_hosted_pm(args: &[String]) -> i32 {
    // The Rust PM is the compatibility implementation and is deliberately the
    // default. The pure-L++ PM can be exercised with
    // LPP_SELF_HOSTED_PM=1, but its archive/delta backend is still experimental;
    // ordinary commands must not depend on bootstrapping a second compiler.
    if env::var("LPP_SELF_HOSTED_PM").ok().as_deref() != Some("1") {
        return pm::run_command(args);
    }

    let cmd = args.first().map(|s| s.as_str()).unwrap_or("help");

    // These commands intentionally stay in the Rust implementation.  They
    // either need to create the PM itself or launch a long-running process;
    // delegating them through a second compiler process makes error handling
    // and signal forwarding unreliable.
    if cmd == "create" || cmd == "dev" || cmd == "version" || cmd == "lreact" || args.iter().any(|a| a == "web" || a == "--release") {
        return pm::run_command(args);
    }

    let pm_bin = match bootstrap_self_hosted_pm() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[L++] Self-hosted PM unavailable: {e}");
            eprintln!("[L++] Falling back to built-in Rust PM.");
            return pm::run_command(args);
        }
    };

    // Build owned env strings (avoid borrow issues)
    let mut child = std::process::Command::new(&pm_bin);
    child.env("LPP_PM_CMD", cmd);
    child.env("LPP_PM_VERSION", env!("CARGO_PKG_VERSION"));
    // Commands launched by the self-hosted PM (for example `lpp test`) must
    // use the Rust command implementation instead of recursively bootstrapping
    // another PM process.
    child.env("LPP_PM_CHILD", "1");
    if let Ok(current_exe) = env::current_exe() {
        child.env("LPP_BIN", current_exe);
    }

    // Pass sub-arguments through env vars
    match cmd {
        "new" | "init" => {
            let name = args.get(1).map(|s| s.as_str()).unwrap_or("my_project");
            child.env("LPP_PM_NAME", name);
        }
        "add" => {
            if let Some(a1) = args.get(1) {
                child.env("LPP_PM_ARG1", a1.as_str());
                let mut i = 2;
                while i < args.len() {
                    match args[i].as_str() {
                        "--git" => {
                            if i + 1 < args.len() {
                                child.env("LPP_PM_GIT", &args[i + 1]);
                                i += 1;
                            }
                        }
                        "--branch" => {
                            if i + 1 < args.len() {
                                child.env("LPP_PM_BRANCH", &args[i + 1]);
                                i += 1;
                            }
                        }
                        "--tag" => {
                            if i + 1 < args.len() {
                                child.env("LPP_PM_TAG", &args[i + 1]);
                                i += 1;
                            }
                        }
                        "--path" => {
                            if i + 1 < args.len() {
                                child.env("LPP_PM_PATH", &args[i + 1]);
                                i += 1;
                            }
                        }
                        "--version" => {
                            if i + 1 < args.len() {
                                child.env("LPP_PM_VERSION", &args[i + 1]);
                                i += 1;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
            }
        }
        "remove" | "search" => {
            if let Some(a1) = args.get(1) {
                child.env("LPP_PM_ARG1", a1.as_str());
            }
        }
        "install" => {
            for arg in args.iter().skip(1) {
                if arg == "--offline" {
                    child.env("LPP_PM_OFFLINE", "1");
                }
                if arg == "--online" {
                    child.env("LPP_PM_ONLINE", "1");
                }
            }
        }
        "version" => {
            if let Some(a1) = args.get(1) { child.env("LPP_PM_ARG1", a1.as_str()); }
            if let Some(a2) = args.get(2) { child.env("LPP_PM_ARG2", a2.as_str()); }
            if let Some(a3) = args.get(3) { child.env("LPP_PM_ARG3", a3.as_str()); }
        }
        "workspace" => {
            if let Some(a1) = args.get(1) { child.env("LPP_PM_ARG1", a1.as_str()); }
            if let Some(a2) = args.get(2) { child.env("LPP_PM_ARG2", a2.as_str()); }
        }
        "publish" => {
            // Forward patch/minor/major and flags as ARG1, ARG2
            if let Some(a1) = args.get(1) {
                child.env("LPP_PM_ARG1", a1.as_str());
            }
            if let Some(a2) = args.get(2) {
                child.env("LPP_PM_ARG2", a2.as_str());
            }
        }
        _ => {
            if args.len() > 1 {
                let rest: Vec<&str> = args.iter().skip(1).map(|s| s.as_str()).collect();
                child.env("LPP_PM_ARGS", rest.join("\x1f"));
            }
        }
    }

    // Pass through AOT/linker settings
    for key in &["LPP_AOT", "LPP_LINKER", "BENCHMARK"] {
        if let Ok(val) = env::var(key) {
            child.env(key, val);
        }
    }

    // Ensure lpp and git are findable
    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            let existing = env::var("PATH").unwrap_or_default();
            child.env("PATH", format!("{}:{}", dir.display(), existing));
        }
    }

    let output = child
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);

            if !stdout.is_empty() {
                print!("{}", stdout);
            }
            if !stderr.is_empty() {
                eprint!("{}", stderr);
            }

            // Check for delegation signal
            if stdout.contains("__DELEGATE__") || stderr.contains("__DELEGATE__") {
                return pm::run_command(args);
            }

            // A real command failure must stay a failure.  The old code ran
            // the Rust PM again after a non-zero child exit, which duplicated
            // side effects (clone/publish/build) and often turned a failed
            // action into a successful process exit.
            if out.status.success() {
                0
            } else {
                out.status.code().unwrap_or(1)
            }
        }
        Err(e) => {
            eprintln!("[L++] Failed to run self-hosted PM: {e}. Falling back.");
            pm::run_command(args)
        }
    }
}



fn main() {
    let builder = std::thread::Builder::new()
        .name("lpp_main".to_string())
        .stack_size(32 * 1024 * 1024);

    let handle = builder.spawn(real_main).expect("failed to spawn main compiler thread");

    let code = match handle.join() {
        Ok(code) => code,
        Err(_) => {
            eprintln!("[L++] compiler thread panicked");
            101
        }
    };
    if code != 0 {
        std::process::exit(code);
    }
}

fn real_main() -> i32 {
    let mut args: Vec<String> = env::args().collect();

    // The CLI has two intentionally separate modes:
    // - package commands (`build`, `run`, `test`, …) operate on lpp.toml;
    // - source commands (`check file.lpp`, `emit file.lpp`) operate on one file.
    let mut source_check_command = false;
    let mut is_emit_cmd = false;
    let mut source_run_command = false;
    if args.len() > 2 && args[1] == "emit" {
        is_emit_cmd = true;
        args.remove(1);
    } else if args.len() > 2 && args[1] == "check" && args[2].ends_with(".lpp") {
        source_check_command = true;
        args.remove(1);
    } else if args.len() > 2 && args[1] == "run" && (args[2].ends_with(".lpp") || Path::new(&args[2]).exists()) {
        source_run_command = true;
        args.remove(1);
    }

    // Handle config command
    if args.len() > 1 && args[1] == "config" {
        if args.len() > 2 && args[2] == "set" && args.len() > 4 {
            let mut cfg = config::LppConfig::load_or_create();
            let setting = &args[3];
            let val = &args[4];
            if setting == "linker" {
                if val == "direct" || val == "host" || val == "auto" {
                    cfg.linker = val.clone();
                    if let Err(e) = cfg.save() {
                        eprintln!("Failed to save config: {e}");
                        std::process::exit(1);
                    }
                    println!("Linker set to: {val}");
                } else {
                    eprintln!("Invalid linker value: {val}. Use 'direct', 'host', or 'auto'.");
                    std::process::exit(1);
                }
            } else if setting == "backend" {
                if val == "cranelift" || val == "llvm" || val == "wasm" {
                    cfg.backend = val.clone();
                    if let Err(e) = cfg.save() {
                        eprintln!("Failed to save config: {e}");
                        std::process::exit(1);
                    }
                    println!("Backend set to: {val}");
                } else {
                    eprintln!("Invalid backend value: {val}. Use 'cranelift', 'llvm', or 'wasm'.");
                    std::process::exit(1);
                }
            } else if setting == "llvm-path" || setting == "llvm_path" {
                cfg.llvm_path = Some(val.clone());
                if let Err(e) = cfg.save() {
                    eprintln!("Failed to save config: {e}");
                    std::process::exit(1);
                }
                println!("LLVM compiler path set to: {val}");
            } else {
                eprintln!("Unknown config setting: {setting}. Use 'linker', 'backend', or 'llvm-path'.");
                std::process::exit(1);
            }
        } else {
            let cfg = config::LppConfig::load_or_create();
            cfg.print_summary();
        }
        return 0;
    }

    if args.len() > 1 {
        let first_arg = &args[1];
        if env::var_os("LPP_PM_CHILD").is_some() {
            return pm::run_command(&args[1..]);
        }
        if first_arg == "init"
            || first_arg == "create"
            || first_arg == "lreact"
            || first_arg == "dev"
            || first_arg == "install"
            || first_arg == "add"
            || first_arg == "remove"
            || first_arg == "update"
            || first_arg == "check"
            || first_arg == "build"
            || first_arg == "run"
            || first_arg == "test"
            || first_arg == "new"
            || first_arg == "search"
            || first_arg == "list"
            || first_arg == "tree"
            || first_arg == "metadata"
            || first_arg == "clean"
            || first_arg == "outdated"
            || first_arg == "version"
            || first_arg == "publish"
            || first_arg == "workspace"
            || first_arg == "help"
            || first_arg == "bench"
        {
            return run_self_hosted_pm(&args[1..]);
        }
    }

    let mut filename = None;
    let mut dump_ast = false;
    let mut dump_symbols = false;
    let mut dump_types = false;
    let mut dump_escape = false;
    let mut dump_mir = false;
    let mut check_only = source_check_command;
    let mut check_all = false;
    let mut do_fix = false;
    let mut emit_object = is_emit_cmd || env::var("LPP_AOT").is_ok() || env::var("LPP_AOT_ONLY").is_ok();

    let mut idx = 1;
    let mut cli_linker: Option<String> = None;
    let config_obj_init = config::LppConfig::load_or_create();
    let mut backend = config_obj_init.backend.clone();
    let mut cli_target: Option<String> = None;

    while idx < args.len() {
        let arg = &args[idx];
        if arg == "--version" || arg == "-v" {
            println!("L++ Compiler v{} (Pure Native AOT)", env!("CARGO_PKG_VERSION"));
            return 0;
        } else if arg == "--list-targets" {
            println!("L++ supported target triples (Android / Termux / Linux):");
            println!("  aarch64-linux-android        Android arm64 & Termux 64-bit");
            println!("  armv7-linux-androideabi      Android arm32");
            println!("  arm-linux-androideabi        Android arm (v7)");
            println!("  i686-linux-android           Android x86");
            println!("  x86_64-linux-android         Android x86_64");
            println!("  aarch64-unknown-linux-gnu    Generic arm64 Linux");
            println!("  x86_64-unknown-linux-gnu     Generic x86_64 Linux");
            println!("  riscv64gc-unknown-linux-gnu  Generic riscv64 Linux");
            println!();
            println!("WebAssembly targets (wasm backend, no linker needed):");
            println!("  wasm32-wasi                  WebAssembly module with WASI imports (.wasm)");
            println!("  wasm32-wasip1                Alias profile for wasm32-wasi");
            println!("  wasm32-unknown-unknown       Bare module (WASI imports still emitted)");
            println!();
            println!("Use --target <triple> to cross-compile. The Cranelift backend");
            println!("must be built with the matching arch feature (default: all-arch).");
            return 0;
        } else if arg == "--help" || arg == "-h" {
            println!("L++ (L Plus Plus) v{} — Pure Native Compiler & Toolchain", env!("CARGO_PKG_VERSION"));
            println!("Cranelift AOT backend, 9 MIR optimization passes, direct ELF/PE/Mach-O linker");
            println!();
            println!("Usage: lpp <file.lpp> [options]");
            println!("       lpp <command> [args]");
            println!();
            println!("Compilation:");
            println!("  lpp <file.lpp>             Compile to native executable (direct lpp-link)");
            println!("  lpp <file.lpp> --emit-obj  Emit native object file only (.o / .obj)");
            println!("  lpp <file.lpp> --backend llvm  Use the optional LLVM object backend");
            println!("  lpp <file.lpp> --target wasm32-wasi  Emit a WebAssembly module (.wasm)");
            println!("  lpp <file.lpp> --check     Type-check without compiling");
            println!("  lpp --checkall             Check all .lpp files in current directory");
            println!();
            println!("Package Manager:");
            println!("  new <name>       Create a new L++ package");
            println!("  init <name>      Initialize in current directory");
            println!("  install          Resolve and install dependencies");
            println!("  add <name>       Add a dependency to lpp.toml");
            println!("  remove <name>    Remove a dependency");
            println!("  update           Refresh dependencies and rewrite lpp.lock");
            println!("  search <query>   Search the package registry");
            println!("  list             List direct dependencies");
            println!("  tree             Print dependency tree");
            println!("  metadata         Print package metadata");
            println!("  outdated         Show unpinned or incompatible dependencies");
            println!("  version          Show package version");
            println!("  version set <v>  Set package version (SemVer)");
            println!("  version bump     Bump patch/minor/major version");
            println!("  clean            Remove build output and artifacts");
            println!("  check            Check project for errors");
            println!("  build            Build project to native binary");
            println!("  run              Compile and run project");
            println!("  test             Run tests in tests/ directory");
            println!("  bench            Run benchmarks");
            println!("  publish          Publish package to registry");
            println!();
            println!("Debug & Inspection:");
            println!("  --dump-ast       Dump Abstract Syntax Tree");
            println!("  --dump-symbols   Dump resolved symbol table");
            println!("  --dump-types     Dump type checker output");
            println!("  --dump-escape    Dump MIR escape/storage classifications");
            println!("  --dump-mir       Dump Mid-level IR (MIR)");
            println!();
            println!("Linker:");
            println!("  --linker direct  Use lpp-link (no external tools needed)");
            println!("  --linker host    Use system cc/cl.exe (required for FFI/extern)");
            println!();
            println!("Target:");
            println!("  --target <triple>  Emit for a target triple instead of the host");
            println!("                     (e.g. aarch64-linux-android, armv7-linux-androideabi,");
            println!("                     i686-linux-android, aarch64-unknown-linux-gnu,");
            println!("                     wasm32-wasi for a WebAssembly module)");
            println!("  --backend wasm     Emit a .wasm module (implies wasm32-wasi)");
            println!("  --list-targets     List known Android/Termux/WebAssembly target triples");
            println!();
            println!("Configuration:");
            println!("  config                         Show config (~/.lpp/config.json)");
            println!("  config set linker <value>      Set default linker (direct|host|auto)");
            println!();
            println!("Options:");
            println!("  -v, --version    Show version");
            println!("  -h, --help       Show this help");
            println!();
            println!("Language Features (v{}):", env!("CARGO_PKG_VERSION"));
            println!("  Functions, default params, closures, threads");
            println!("  Structs, enums, match with bindings");
            println!("  Experimental: tuples, typed rest lists, borrowed slices, async/.await");
            println!("  Generics: def foo[T](x: T) -> T");
            println!("  Traits:   trait Name / impl Trait for Type (static + dynamic dispatch)");
            println!("  FFI:      extern \"C\" link \"SDL2\" (call any C library)");
            println!("  Try:      result? operator for error propagation");
            println!("  Builtins: 100+ (strings, lists, maps, files, network, JSON, GUI)");
            println!("  Ownership: MIR escape solver + ARC/stack/Arena + static cycle breaking");
            println!();
            println!("Environment:");
            println!("  BENCHMARK=1           Print JSON timings instead of descriptive output");
            println!("  LPP_AOT_OPT=speed     Set Cranelift optimization level (none|speed|speed_and_size)");
            println!("  LPP_SELF_HOSTED_PM=1  Opt into the experimental pure-L++ package manager");
            return 0;
        } else if arg == "--dump-ast" {
            dump_ast = true;
        } else if arg == "--dump-symbols" {
            dump_symbols = true;
        } else if arg == "--dump-types" {
            dump_types = true;
        } else if arg == "--dump-escape" {
            dump_escape = true;
        } else if arg == "--dump-mir" {
            dump_mir = true;
        } else if arg == "--check" {
            check_only = true;
        } else if arg == "--run" {
            source_run_command = true;
        } else if arg == "--checkall" {
            check_all = true;
        } else if arg == "--fix" {
            do_fix = true;
        } else if arg == "--emit-object" || arg == "--aot" {
            emit_object = true;
        } else if arg == "--backend" {
            if idx + 1 < args.len() {
                backend = args[idx + 1].clone();
                idx += 1;
            }
        } else if arg == "--linker" {
            if idx + 1 < args.len() {
                cli_linker = Some(args[idx + 1].clone());
                idx += 1;
            }
        } else if arg == "--target" {
            if idx + 1 < args.len() {
                cli_target = Some(args[idx + 1].clone());
                idx += 1;
            }
        } else if !arg.starts_with('-') {
            filename = Some(arg.as_str());
        }
        idx += 1;
    }

    if check_all {
        // Scan current directory recursively for .lpp files and type-check all
        let mut all_files: Vec<PathBuf> = Vec::new();
        fn walk(base: &Path, files: &mut Vec<PathBuf>) {
            if let Ok(entries) = fs::read_dir(base) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_dir() {
                        let name = p.file_name().unwrap_or_default().to_string_lossy();
                        if name.starts_with('.') || name == "target" || name == "LppData" || name == "node_modules" {
                            continue;
                        }
                        walk(&p, files);
                    } else if p.extension().map_or(false, |e| e == "lpp") {
                        files.push(p);
                    }
                }
            }
        }
        walk(Path::new("."), &mut all_files);
        if all_files.is_empty() {
            eprintln!("[L++] No .lpp files found in project.");
            return 1;
        }
        all_files.sort();
        eprintln!("[L++] --checkall: checking {} file(s)...", all_files.len());
        let ta = Instant::now();

        enum CheckResult {
            Passed,
            Fixed(String), // log msg
            Failed(String), // log msg
        }

        let num_threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        let files_arc = std::sync::Arc::new(all_files);
        let do_fix_val = do_fix;

        let results: Vec<CheckResult> = if files_arc.len() <= 1 || num_threads <= 1 {
            // Single-threaded path for tiny workloads
            files_arc.iter().map(|fpath| {
                let input = match fs::read_to_string(fpath) {
                    Ok(c) => c,
                    Err(e) => return CheckResult::Failed(format!("{}:1:1: read: {}", fpath.display(), e)),
                };
                let mut err_tuple: Option<(&'static str, String)> = None;
                let mut l = lexer::Lexer::new(&input);
                match l.tokenize() {
                    Ok(t) => {
                        let mut par = parser::Parser::new(t);
                        match par.parse() {
                            Ok(mut ast) => {
                                let base = fpath.parent().unwrap_or(Path::new("."));
                                let mut imp = std::collections::HashSet::new();
                                if let Err(e) = resolve_local_imports(&mut ast.declarations, &mut imp, base) {
                                    err_tuple = Some(("import", e));
                                } else if let Err(e) = monomorph::Monomorphizer::process_program(&mut ast) {
                                    err_tuple = Some(("monomorph", e));
                                } else {
                                    let mut res = semantic::Resolver::new();
                                    if let Err(e) = res.resolve_program(&mut ast) {
                                        err_tuple = Some(("semantic", e));
                                    } else {
                                        let mut tc = typecheck::TypeChecker::new(&mut res.table);
                                        if let Err(e) = tc.check_program(&ast) {
                                            err_tuple = Some(("type", e));
                                        }
                                    }
                                }
                            }
                            Err(e) => { err_tuple = Some(("syntax", e)); }
                        }
                    }
                    Err(e) => { err_tuple = Some(("lex", e)); }
                }

                if let Some((stage, raw_err)) = err_tuple {
                    let (line, col, msg) = diagnostics::parse_line_col_message_with_source(&raw_err, &input);
                    let auto_fix = diagnostics::try_auto_fix(&input, &raw_err);
                    if let Some((new_src, desc)) = auto_fix {
                        if do_fix_val {
                            if fs::write(fpath, &new_src).is_ok() {
                                return CheckResult::Fixed(format!("[L++] Fixed {}:{}:{}: {} [{}]", fpath.display(), line, col, stage, desc));
                            }
                        }
                        CheckResult::Failed(format!("{}:{}:{}: {}: {} [suggestion: {}]", fpath.display(), line, col, stage, msg, desc))
                    } else {
                        CheckResult::Failed(format!("{}:{}:{}: {}: {}", fpath.display(), line, col, stage, msg))
                    }
                } else {
                    CheckResult::Passed
                }
            }).collect()
        } else {
            // Parallel worker pool
            let chunk_size = (files_arc.len() + num_threads - 1) / num_threads;
            let mut handles = Vec::new();

            for thread_idx in 0..num_threads {
                let files = std::sync::Arc::clone(&files_arc);
                let start = thread_idx * chunk_size;
                if start >= files.len() {
                    break;
                }
                let end = (start + chunk_size).min(files.len());

                handles.push(std::thread::spawn(move || {
                    let mut local_res = Vec::with_capacity(end - start);
                    for fpath in &files[start..end] {
                        let input = match fs::read_to_string(fpath) {
                            Ok(c) => c,
                            Err(e) => {
                                local_res.push(CheckResult::Failed(format!("{}:1:1: read: {}", fpath.display(), e)));
                                continue;
                            }
                        };
                        let mut err_tuple: Option<(&'static str, String)> = None;
                        let mut l = lexer::Lexer::new(&input);
                        match l.tokenize() {
                            Ok(t) => {
                                let mut par = parser::Parser::new(t);
                                match par.parse() {
                                    Ok(mut ast) => {
                                        let base = fpath.parent().unwrap_or(Path::new("."));
                                        let mut imp = std::collections::HashSet::new();
                                        if let Err(e) = resolve_local_imports(&mut ast.declarations, &mut imp, base) {
                                            err_tuple = Some(("import", e));
                                        } else if let Err(e) = monomorph::Monomorphizer::process_program(&mut ast) {
                                            err_tuple = Some(("monomorph", e));
                                        } else {
                                            let mut res = semantic::Resolver::new();
                                            if let Err(e) = res.resolve_program(&mut ast) {
                                                err_tuple = Some(("semantic", e));
                                            } else {
                                                let mut tc = typecheck::TypeChecker::new(&mut res.table);
                                                if let Err(e) = tc.check_program(&ast) {
                                                    err_tuple = Some(("type", e));
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => { err_tuple = Some(("syntax", e)); }
                                }
                            }
                            Err(e) => { err_tuple = Some(("lex", e)); }
                        }

                        if let Some((stage, raw_err)) = err_tuple {
                            let (line, col, msg) = diagnostics::parse_line_col_message_with_source(&raw_err, &input);
                            let auto_fix = diagnostics::try_auto_fix(&input, &raw_err);
                            if let Some((new_src, desc)) = auto_fix {
                                if do_fix_val {
                                    if fs::write(fpath, &new_src).is_ok() {
                                        local_res.push(CheckResult::Fixed(format!("[L++] Fixed {}:{}:{}: {} [{}]", fpath.display(), line, col, stage, desc)));
                                        continue;
                                    }
                                }
                                local_res.push(CheckResult::Failed(format!("{}:{}:{}: {}: {} [suggestion: {}]", fpath.display(), line, col, stage, msg, desc)));
                            } else {
                                local_res.push(CheckResult::Failed(format!("{}:{}:{}: {}: {}", fpath.display(), line, col, stage, msg)));
                            }
                        } else {
                            local_res.push(CheckResult::Passed);
                        }
                    }
                    local_res
                }));
            }

            let mut all_res = Vec::with_capacity(files_arc.len());
            for handle in handles {
                if let Ok(res) = handle.join() {
                    all_res.extend(res);
                }
            }
            all_res
        };

        let mut p = 0usize;
        let mut fixed_files_count = 0usize;
        let mut all_fails: Vec<String> = Vec::new();

        for res in results {
            match res {
                CheckResult::Passed => p += 1,
                CheckResult::Fixed(msg) => {
                    println!("{}", msg);
                    fixed_files_count += 1;
                    p += 1;
                }
                CheckResult::Failed(msg) => all_fails.push(msg),
            }
        }

        let el = ta.elapsed();
        if all_fails.is_empty() {
            println!("[L++] --checkall: OK — {} file(s) passed in {:.1} ms", p, el.as_secs_f64() * 1000.0);
            if do_fix && fixed_files_count > 0 {
                println!("[L++] --fix: Automatically repaired {} file(s).", fixed_files_count);
            }
        } else {
            eprintln!("[L++] --checkall: {} passed, {} FAILED:", p, all_fails.len());
            for f in &all_fails { eprintln!("  {}", f); }
            if do_fix && fixed_files_count > 0 {
                eprintln!("[L++] --fix: Automatically repaired {} file(s).", fixed_files_count);
            }
        }
        return if all_fails.is_empty() { 0 } else { 1 };
    }

    let filename = match filename {
        Some(f) => f,
        None => {
            eprintln!("[L++] Error: No input file specified.");
            eprintln!("Usage: lpp [file.lpp] [options]");
            return 1;
        }
    };

    let total_start = Instant::now();

    let io_start = Instant::now();
    let input = match fs::read_to_string(filename) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("Failed to read {}: {}", filename, e);
            return 1;
        }
    };
    let io_time = io_start.elapsed();

    let lex_start = Instant::now();
    let mut lexer = lexer::Lexer::new(&input);
    let tokens = match lexer.tokenize() {
        Ok(tokens) => tokens,
        Err(e) => {
            eprint!("{}", diagnostics::render_error_string(&filename, &input, diagnostics::DiagnosticKind::Lexer, &e));
            return 1;
        }
    };
    let lex_time = lex_start.elapsed();

    let parse_start = Instant::now();
    let mut parser = parser::Parser::new(tokens);
    let mut ast = match parser.parse() {
        Ok(ast) => ast,
        Err(e) => {
            eprint!("{}", diagnostics::render_error_string(&filename, &input, diagnostics::DiagnosticKind::Syntax, &e));
            return 1;
        }
    };
    let parse_time = parse_start.elapsed();

    let file_path = std::path::Path::new(&filename);
    let base_dir = file_path.parent().unwrap_or(std::path::Path::new("."));
    let mut imported_files = std::collections::HashSet::new();
    if let Err(e) = resolve_local_imports(&mut ast.declarations, &mut imported_files, base_dir) {
        eprint!("{}", diagnostics::render_error_string(&filename, &input, diagnostics::DiagnosticKind::Import, &e));
        return 1;
    }

    if let Err(e) = monomorph::Monomorphizer::process_program(&mut ast) {
        eprint!("{}", diagnostics::render_error_string(&filename, &input, diagnostics::DiagnosticKind::Semantic, &e));
        return 1;
    }

    let sem_start = Instant::now();
    let mut resolver = semantic::Resolver::new();
    if let Err(e) = resolver.resolve_program(&mut ast) {
        eprint!("{}", diagnostics::render_error_string(&filename, &input, diagnostics::DiagnosticKind::Semantic, &e));
        return 1;
    }
    let sem_time = sem_start.elapsed();

    let ty_start = Instant::now();
    let mut type_table;
    let trait_impls_for_cycles;
    {
        let mut type_checker = typecheck::TypeChecker::new(&mut resolver.table);
        if let Err(e) = type_checker.check_program(&ast) {
            eprint!("{}", diagnostics::render_error_string(&filename, &input, diagnostics::DiagnosticKind::Type, &e));
            return 1;
        }
        trait_impls_for_cycles = type_checker.trait_impls.clone();
        type_table = type_checker.type_table;
    }
    let ty_time = ty_start.elapsed();

    if check_only {
        let total_time = total_start.elapsed();
        if env::var("BENCHMARK").is_ok() {
            println!(
                "TIMING_JSON: {{\"io\": {}, \"lex\": {}, \"parse\": {}, \"semantic\": {}, \"typecheck\": {}, \"total\": {}}}",
                io_time.as_secs_f64(),
                lex_time.as_secs_f64(),
                parse_time.as_secs_f64(),
                sem_time.as_secs_f64(),
                ty_time.as_secs_f64(),
                total_time.as_secs_f64()
            );
        } else {
            println!("L++ check: OK");
            println!("Time: {:.1} ms", total_time.as_secs_f64() * 1000.0);
        }
        return 0;
    }

    #[allow(unused_assignments)]
    let mut mir_time = std::time::Duration::ZERO;
    // Ownership is solved once over the lowered MIR. The old AST walker is no
    // longer a code-generation input; keeping one source of truth prevents a
    // missed AST form from disagreeing with the exhaustive MIR analysis.
    let escape_start = Instant::now();
    if dump_ast {
        println!("--- Abstract Syntax Tree ---");
        println!("{:#?}", ast);
    }
    if dump_symbols {
        println!("--- Symbol Table ---");
        println!("{:#?}", resolver.table);
    }
    if dump_types {
        println!("--- Type Table ---");
        println!("{:#?}", type_table);
    }

    let mir_start = Instant::now();
    // Mark every struct that sits on an ownership cycle (mutual / indirect
    // cycles included) as arena-allocated BEFORE MIR lowering, because the
    // arena-vs-ARC choice is made there from `is_self_referential`. This
    // extends the arena lifetime guarantee that direct self-referential
    // structs already enjoy to mutual cycles (e.g. Parent<->Child), so a weak
    // (cycle-broken) field read cannot dangle while a node in the same region
    // is still live. See analysis::cyclebreak::mark_cyclic_structs.
    cyclebreak::mark_cyclic_structs(&mut type_table);
    let mut mir_ctx = mir::lower::MirLowerCtx::new(&resolver.table, &mut type_table, &ast);
    let mut mir_program = match mir_ctx.lower_program(&ast) {
        Ok(program) => program,
        Err(e) => {
            eprintln!("MIR lowering error: {}", e);
            return 1;
        }
    };
    if let Err(error) = mir::validate_borrows::validate(&mir_program) {
        eprintln!("{}", error);
        return 1;
    }
    // C-Speed Project: simplify only scalar/copy MIR before ARC so
    // no retain/release or ownership edge can be optimized away.
    mir::pass_peephole::run(&mut mir_program);
    // Propagate constant integers through basic blocks before
    // inlining — constant addresses/offsets unlock further folding.
    mir::pass_constprop::run(&mut mir_program);
    // Inline only scalar straight-line direct calls; ownership-bearing
    // functions remain opaque so ARC semantics cannot be altered.
    mir::pass_inline::run(&mut mir_program);
    // Straight-line scalar dead stores are removed only after folding
    // and inlining, before ownership instrumentation.
    mir::pass_dce::run(&mut mir_program);
    // Copy propagation: _tmp = _a + _b; _a = _tmp → _a = _a + _b
    // Eliminates extra register moves in tight loops.
    mir::pass_copyprop::run(&mut mir_program);
    // Strength reduction: x % power_of_2 → x & (power_of_2 - 1)
    // Avoids expensive idiv on x86 for common modulo patterns.
    mir::pass_strength::run(&mut mir_program);
    // Fuses a trailing comparison temporary with its branch to avoid
    // setcc/test materialization in hot native loops.
    mir::pass_branch::run(&mut mir_program);
    // Break every ownership cycle statically before ARC insertion, so
    // the owning subgraph the pass reasons about is acyclic. See
    // analysis::cyclebreak for the proof.
    let ownership_graph = cyclebreak::break_cycles_with_traits(&type_table, &trait_impls_for_cycles);
    let weak_fields = ownership_graph.weak_fields();
    // Value-by-default. This is where the escape classification finally
    // reaches codegen: a struct that provably cannot outlive its frame
    // is moved to a stack slot, losing its header, its allocator call
    // and its retain/release traffic.
    //
    // It must run BEFORE ARC insertion. A promoted local has no ARC
    // header, so it must never enter `arc_locals` and never be handed to
    // retain/release; running first means the ARC pass simply never sees
    // it as an owner.
    //
    // One escape fact for the whole program, computed once over MIR and
    // read by every consumer that needs it. This replaces the private
    // use-scan that pass_escape used to carry: three partial answers to
    // the same question is how the double-free and the nondeterministic
    // release order happened.
    let escape_facts = mir::escape_solver::solve(&mir_program);
    if dump_escape {
        println!("--- MIR Ownership Facts ---");
        let mut ordered_functions: Vec<_> = mir_program.functions.values().collect();
        ordered_functions.sort_by_key(|function| function.id.0);
        for function in ordered_functions {
            println!("  fn {}:", function.name);
            if let Some(facts) = escape_facts.functions.get(&function.id) {
                for local in &function.locals {
                    let name = local.debug_name.as_deref().unwrap_or("<anon>");
                    let storage = facts
                        .locals
                        .get(local.id.0)
                        .copied()
                        .unwrap_or(mir::escape_solver::Storage::Owned);
                    println!(
                        "    _{} ({}) : {:?}",
                        local.id.0, name, storage
                    );
                }
            }
        }
    }
    let escape_stats = mir::pass_escape::run(&mut mir_program, &escape_facts, &type_table);
    if dump_escape {
        println!("--- MIR Ownership Summary ---");
        println!(
            "  stack-promoted {} of {} candidate managed locals",
            escape_stats.promoted, escape_stats.considered
        );
    }
    mir::pass_arc::run_arc_insertion_pass_with_weak(&mut mir_program, &weak_fields);
    // Values handed to a thread and never touched again are moved, not
    // shared: drop the refcount pair that only existed to model a
    // second owner that never overlaps the first.
    mir::pass_moveout::run(&mut mir_program);

    if dump_mir {
        println!("--- Generated MIR ---");
        println!("{}", mir_program);
    }
    let escape_time = escape_start.elapsed();
    mir_time = mir_start.elapsed();

    // L++ 2.0 Pure Native Cranelift AOT Backend
    // Resolve the optional --target triple into a validated spec. The host is
    // used when none is given; an Android/Termux triple selects a non-host ISA
    // and influences runtime/cc selection.
    //
    // WebAssembly triples take a dedicated route: they bypass the native
    // object/link pipeline and emit a single `.wasm` module from the wasm
    // backend, so no native target spec / linker is involved.
    let wasm_target = match cli_target.as_deref().map(str::trim) {
        Some(t) if crate::target::is_wasm_triple_str(t) => true,
        Some(t) if t.starts_with("wasm32") || t.starts_with("wasm64") => {
            eprintln!(
                "[L++] Target Error: unsupported WebAssembly triple '{}'; expected wasm32-wasi, wasm32-wasip1, or wasm32-unknown-unknown",
                t
            );
            return 1;
        }
        _ => false,
    };
    let target_spec = if wasm_target {
        crate::target::TargetSpec::host()
    } else {
        match &cli_target {
            Some(t) => match crate::target::TargetSpec::from_triple_str(t) {
                Ok(spec) => spec,
                Err(e) => {
                    eprintln!("[L++] Target Error: {}", e);
                    return 1;
                }
            },
            None => crate::target::TargetSpec::host(),
        }
    };
    if let Some(_t) = &cli_target {
        eprintln!(
            "[L++] targeting {}",
            if wasm_target {
                format!("{} (WebAssembly backend)", cli_target.as_deref().unwrap_or_default().trim())
            } else {
                target_spec.to_string()
            }
        );
    }
    if wasm_target {
        backend = "wasm".to_string();
    }

    let aot_start = Instant::now();
    let has_extern_decl = ast
        .declarations
        .iter()
        .any(|d| matches!(d, crate::ast::TopLevel::Extern(_)));
    let backend_result = match backend.as_str() {
        "cranelift" => {
            let target_flag = target_spec.raw.as_deref();
            cranelift_backend::compiler::AotCompiler::compile_with_options_target(
                &mir_program,
                &type_table,
                has_extern_decl,
                &weak_fields,
                target_flag,
            )
        }
        "llvm" => llvm_backend::compile(&mir_program, &type_table, &weak_fields),
        "wasm" => wasm_backend::compile(&mir_program, &type_table, &weak_fields),
        other => Err(format!(
            "unknown backend '{}'; expected cranelift, llvm, or wasm",
            other
        )),
    };
    let obj_bytes = match backend_result {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("[L++] {} backend compilation error: {}", backend, e);
            return 1;
        }
    };
    let aot_time = aot_start.elapsed();

    let ext = if backend == "wasm" {
        "wasm"
    } else if cfg!(target_os = "windows") {
        "obj"
    } else {
        "o"
    };
    let exe_ext = std::env::consts::EXE_SUFFIX;

    let (obj_path, exe_path) = if source_run_command {
        let temp_dir = env::temp_dir();
        let stem = std::path::Path::new(&filename)
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        let pid = std::process::id();
        let obj_p = temp_dir.join(format!("lpp_{}_{}.{}", stem, pid, ext));
        let exe_p = temp_dir.join(format!("lpp_{}_{}{}", stem, pid, exe_ext));
        (obj_p, exe_p)
    } else {
        let path = std::path::Path::new(&filename);
        let obj_p = path.with_extension(ext);
        let exe_p = path.with_extension(exe_ext.trim_start_matches('.'));
        (obj_p, exe_p)
    };

    if let Err(e) = fs::write(&obj_path, &obj_bytes) {
        eprintln!("Failed to write object file {}: {}", obj_path.display(), e);
        return 1;
    }

    let total_time = total_start.elapsed();

    if check_only {
        let _ = fs::remove_file(&obj_path);
        return 0;
    }

    // WebAssembly output: the module is final — there is no link step.
    if backend == "wasm" {
        if !dump_ast && !dump_symbols && !dump_types && !dump_escape && !dump_mir {
            println!(
                "[L++] WebAssembly module (wasm32-wasi) emitted at {}",
                obj_path.display()
            );
            if !source_run_command {
                println!("        run with: wasmtime {}", obj_path.display());
            }
            println!("Time: {:.1} ms", total_time.as_secs_f64() * 1000.0);
        }
        if source_run_command {
            let runtime = env::var("LPP_WASM_RUNTIME")
                .ok()
                .filter(|r| !r.trim().is_empty())
                .unwrap_or_else(|| "wasmtime".to_string());
            let status = std::process::Command::new(&runtime).arg(&obj_path).status();
            let _ = fs::remove_file(&obj_path);
            match status {
                Ok(s) => std::process::exit(s.code().unwrap_or(0)),
                Err(_) => {
                    eprintln!(
                        "[L++] module compiled, but WebAssembly runtime '{}' was not found; \
                         install wasmtime (https://wasmtime.dev/) or set LPP_WASM_RUNTIME",
                        runtime
                    );
                    std::process::exit(1);
                }
            }
        }
        return 0;
    }

    if emit_object {
        if env::var("BENCHMARK").is_ok() {
            println!(
                "TIMING_JSON: {{\"io\": {}, \"lex\": {}, \"parse\": {}, \"semantic\": {}, \"typecheck\": {}, \"escape\": {}, \"mir\": {}, \"aot\": {}, \"total\": {}}}",
                io_time.as_secs_f64(),
                lex_time.as_secs_f64(),
                parse_time.as_secs_f64(),
                sem_time.as_secs_f64(),
                ty_time.as_secs_f64(),
                escape_time.as_secs_f64(),
                mir_time.as_secs_f64(),
                aot_time.as_secs_f64(),
                total_time.as_secs_f64()
            );
        } else if !dump_ast
            && !dump_symbols
            && !dump_types
            && !dump_escape
            && !dump_mir
        {
            println!("[L++] Native Cranelift object emitted at {}", obj_path.display());
            println!("Time: {:.1} ms", total_time.as_secs_f64() * 1000.0);
        }
        return 0;
    }

    // Collect FFI link libraries from extern blocks
    let mut link_libs: Vec<String> = Vec::new();
    for decl in &ast.declarations {
        if let crate::ast::TopLevel::Extern(ext) = decl {
            if let Some(ref lib) = ext.link_lib {
                if !link_libs.contains(lib) {
                    link_libs.push(lib.clone());
                }
            }
        }
    }

    // Check if any extern blocks or explicit host libraries exist (FFI/host linking required)
    let config_obj = config::LppConfig::load_or_create();
    let has_extern = ast.declarations.iter().any(|d| matches!(d, crate::ast::TopLevel::Extern(_)));
    let env_linker = env::var("LPP_LINKER").ok();
    let effective_linker = cli_linker.or(env_linker);
    let use_host = effective_linker.as_deref() == Some("host")
        || (effective_linker.as_deref() != Some("direct") && (has_extern || !link_libs.is_empty()))
        || (effective_linker.is_none() && !config_obj.use_direct_linker());

    // A failed link must not leave an executable from an earlier build that
    // looks successful to a subsequent action.
    let _ = fs::remove_file(&exe_path);
    let forced_direct = effective_linker.as_deref() == Some("direct");
    let mut link_result = if use_host {
        #[cfg(windows)]
        pm::load_msvc_env();
        pm::host_link_binary_target(
            &obj_path,
            &exe_path,
            &link_libs,
            target_spec.raw.as_deref(),
        )
    } else {
        pm::direct_link_binary(&obj_path, &exe_path)
    };
    // An auto-selected direct link that hits a feature outside lpp-link's
    // verified subset should not strand the user: fall back to the host
    // linker.  Explicitly forced direct links still fail loudly.
    if link_result.is_err() && !use_host && !forced_direct && config_obj.system.has_cc {
        if let Err(e) = &link_result {
            eprintln!("[L++] direct linker failed: {e}");
        }
        eprintln!("[L++] falling back to the host linker...");
        #[cfg(windows)]
        pm::load_msvc_env();
        link_result = pm::host_link_binary_target(
            &obj_path,
            &exe_path,
            &link_libs,
            target_spec.raw.as_deref(),
        );
    }
    if let Err(e) = link_result {
        eprintln!("[L++] Native Link Error: {}", e);
        let _ = fs::remove_file(&obj_path);
        return 1;
    }
    let _ = fs::remove_file(&obj_path);

    if source_run_command {
        let status = std::process::Command::new(&exe_path).status();
        let _ = fs::remove_file(&exe_path);
        match status {
            Ok(s) => std::process::exit(s.code().unwrap_or(0)),
            Err(e) => {
                eprintln!("[L++] Execution failed: {}", e);
                std::process::exit(1);
            }
        }
    }

    if env::var("BENCHMARK").is_ok() {
        println!(
            "TIMING_JSON: {{\"io\": {}, \"lex\": {}, \"parse\": {}, \"semantic\": {}, \"typecheck\": {}, \"escape\": {}, \"mir\": {}, \"aot\": {}, \"total\": {}}}",
            io_time.as_secs_f64(),
            lex_time.as_secs_f64(),
            parse_time.as_secs_f64(),
            sem_time.as_secs_f64(),
            ty_time.as_secs_f64(),
            escape_time.as_secs_f64(),
            mir_time.as_secs_f64(),
            aot_time.as_secs_f64(),
            total_time.as_secs_f64()
        );
    } else if !dump_ast
        && !dump_symbols
        && !dump_types
        && !dump_escape
        && !dump_mir
    {
        println!("L++ v{} (Pure Native Executable)\n", env!("CARGO_PKG_VERSION"));
        println!("Compiled and linked native binary: {}", exe_path.display());
        println!("Time: {:.1} ms", total_time.as_secs_f64() * 1000.0);
    }
    0
}

fn find_stdlib_module(clean_module: &str, leaf_name: &str) -> Option<PathBuf> {
    for var in &["LPP_HOME", "LPP_DIR"] {
        if let Ok(val) = std::env::var(var) {
            let home_dir = std::path::Path::new(&val);
            let candidates = [
                home_dir.join(format!("stdlib/{}.lpp", clean_module)),
                home_dir.join(format!("stdlib/{}.lpp", leaf_name)),
            ];
            for c in candidates {
                if c.exists() {
                    return Some(c);
                }
            }
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        let exe_dir = exe.parent().unwrap_or(std::path::Path::new("."));
        let candidates = [
            exe_dir.join(format!("../stdlib/{}.lpp", clean_module)),
            exe_dir.join(format!("../../stdlib/{}.lpp", clean_module)),
            exe_dir.join(format!("stdlib/{}.lpp", clean_module)),
            exe_dir.join(format!("../stdlib/{}.lpp", leaf_name)),
            exe_dir.join(format!("../../stdlib/{}.lpp", leaf_name)),
            exe_dir.join(format!("stdlib/{}.lpp", leaf_name)),
            std::path::PathBuf::from(format!("stdlib/{}.lpp", clean_module)),
            std::path::PathBuf::from(format!("stdlib/{}.lpp", leaf_name)),
        ];
        for c in candidates {
            if c.exists() {
                return Some(c);
            }
        }
    }
    None
}

fn resolve_module_filepath(module: &str, base_dir: &std::path::Path) -> Result<PathBuf, String> {
    let clean_module = module.strip_prefix("stdlib/").unwrap_or(module);
    let leaf_name = clean_module.split('/').last().unwrap_or(clean_module);

    let is_explicit_stdlib = module.starts_with("stdlib/") || module.starts_with("stdlib.");

    // Core shipped standard library modules
    let is_known_stdlib = matches!(
        leaf_name,
        "math" | "strings" | "collections" | "gui" | "convert" | "assert" | "result"
        | "algo" | "sort" | "testing" | "env" | "args" | "path" | "http" | "hash"
        | "uuid" | "io" | "csv" | "base64" | "list_util" | "map_str" | "color" | "config"
        | "log" | "time_util" | "regex_lite"
    );

    // 1. Check local file in project base_dir (unless explicitly prefixed with stdlib.)
    if !is_explicit_stdlib {
        let local_exact = base_dir.join(format!("{}.lpp", module));
        if local_exact.exists() {
            return Ok(local_exact);
        }
        let local_src = base_dir.join("src").join(format!("{}.lpp", module));
        if local_src.exists() {
            return Ok(local_src);
        }
    }

    // 2. Standard Library protection: stdlib modules take precedence over third-party packages
    if is_explicit_stdlib || is_known_stdlib {
        if let Some(stdlib_file) = find_stdlib_module(clean_module, leaf_name) {
            return Ok(stdlib_file);
        }
    }

    // 3. Check third-party package dependencies in .lpp_packages/
    let parts: Vec<&str> = clean_module.split(|c| c == '/' || c == '.').collect();
    let pkg_name = parts[0];
    let pkg_dir = std::path::Path::new(".lpp_packages").join(pkg_name);
    if pkg_dir.exists() {
        if parts.len() > 1 {
            let sub_path = parts[1..].join("/");
            let candidates = [
                pkg_dir.join(format!("{}.lpp", sub_path)),
                pkg_dir.join("src").join(format!("{}.lpp", sub_path)),
            ];
            for c in &candidates {
                if c.exists() {
                    return Ok(c.clone());
                }
            }
        }

        // Check root and subfolder (e.g. .lpp_packages/sqlite/packages/sqlite)
        let search_dirs = [
            pkg_dir.clone(),
            pkg_dir.join("packages").join(pkg_name),
            pkg_dir.join(pkg_name),
        ];

        for s_dir in &search_dirs {
            if !s_dir.exists() { continue; }

            let manifest_path_json = s_dir.join("lpp.json");
            let manifest_path_toml = s_dir.join("lpp.toml");
            let parsed_pkg = if manifest_path_json.exists() {
                std::fs::read_to_string(&manifest_path_json)
                    .ok()
                    .and_then(|c| pm::parse_json_manifest(&c).ok())
            } else if manifest_path_toml.exists() {
                std::fs::read_to_string(&manifest_path_toml)
                    .ok()
                    .and_then(|c| pm::parse_toml(&c).ok())
            } else {
                None
            };

            if let Some(pkg) = parsed_pkg {
                if let Some(entry) = pkg.entry {
                    let custom_entry = s_dir.join(entry);
                    if custom_entry.exists() {
                        return Ok(custom_entry);
                    }
                }
            }

            let candidates = [
                s_dir.join(format!("{}.lpp", pkg_name)),
                s_dir.join("src").join(format!("{}.lpp", pkg_name)),
                s_dir.join("src").join("main.lpp"),
                s_dir.join("main.lpp"),
                s_dir.join("src").join("sqlite.lpp"),
            ];
            for c in &candidates {
                if c.exists() {
                    return Ok(c.clone());
                }
            }
        }
    }

    // 4. Fallback check stdlib if not checked earlier
    if let Some(stdlib_file) = find_stdlib_module(clean_module, leaf_name) {
        return Ok(stdlib_file);
    }

    Err(format!(
        "Imported module '{}' not found in:\n  - {}\n  - stdlib/{}.lpp\n  - .lpp_packages/{}/{}.lpp",
        module,
        base_dir.join(format!("{}.lpp", module)).display(),
        leaf_name,
        leaf_name, leaf_name
    ))
}

fn resolve_local_imports(
    declarations: &mut Vec<ast::TopLevel>,
    imported_files: &mut std::collections::HashSet<String>,
    root_base_dir: &std::path::Path,
) -> Result<(), String> {
    use std::collections::VecDeque;

    let mut new_decls = Vec::new();
    let mut module_of_decl: Vec<(String, String)> = Vec::new();
    let mut worklist: VecDeque<(String, PathBuf)> = VecDeque::new();

    // Collect initial imports from entry file
    for decl in declarations.iter() {
        if let ast::TopLevel::Import(import_kind) = decl {
            let path = match import_kind {
                ast::ImportKind::Module { path, .. } => path.clone(),
                ast::ImportKind::Selective { path, .. } => path.clone(),
            };
            let module = path.join("/");
            let module_name = path.last().cloned().unwrap_or_default();
            if module_name != "json" {
                worklist.push_back((module, root_base_dir.to_path_buf()));
            }
        }
    }

    while let Some((module, base_dir)) = worklist.pop_front() {
        let filepath = match resolve_module_filepath(&module, &base_dir) {
            Ok(fp) => fp,
            Err(e) => return Err(e),
        };

        let raw_canonical = filepath
            .canonicalize()
            .unwrap_or_else(|_| filepath.clone())
            .to_string_lossy()
            .to_string();
        let canonical_key = raw_canonical.trim_start_matches(r"\\?\").to_lowercase();
        let mod_key = module.to_lowercase();

        if imported_files.contains(&canonical_key) || imported_files.contains(&mod_key) {
            continue;
        }
        imported_files.insert(canonical_key);
        imported_files.insert(mod_key);

        let content = match std::fs::read_to_string(&filepath) {
            Ok(c) => c,
            Err(e) => return Err(format!("Failed to read library '{}': {}", filepath.display(), e)),
        };

        let mut lex = lexer::Lexer::new(&content);
        let tokens = lex.tokenize()?;
        let mut par = parser::Parser::new(tokens);
        let lib_ast = par.parse()?;

        let lib_base_dir = filepath.parent().unwrap_or(Path::new(".")).to_path_buf();

        for decl in &lib_ast.declarations {
            if let ast::TopLevel::Import(import_kind) = decl {
                let path = match import_kind {
                    ast::ImportKind::Module { path, .. } => path.clone(),
                    ast::ImportKind::Selective { path, .. } => path.clone(),
                };
                let sub_module = path.join("/");
                let sub_module_name = path.last().cloned().unwrap_or_default();
                // Keep transitive import declarations in the flattened
                // program.  Dropping them made qualified calls such as
                // `ieee754.fmt_float(...)` fail when ieee754 was imported by a
                // library rather than by the entry file itself.
                new_decls.push(decl.clone());
                if sub_module_name != "json" {
                    worklist.push_back((sub_module, lib_base_dir.clone()));
                }
            } else {
                if let Some(name) = declaration_name(decl) {
                    module_of_decl.push((name, module.clone()));
                }
                new_decls.push(decl.clone());
            }
        }
    }

    declarations.extend(new_decls);
    check_duplicate_declarations(declarations, &module_of_decl)?;
    Ok(())
}

/// The name a top-level declaration introduces into the global namespace.
fn declaration_name(decl: &ast::TopLevel) -> Option<String> {
    match decl {
        ast::TopLevel::Function(function) => Some(function.name.clone()),
        ast::TopLevel::Struct(def) => Some(def.name.clone()),
        ast::TopLevel::Enum(def) => Some(def.name.clone()),
        ast::TopLevel::Const { name, .. } => Some(name.clone()),
        ast::TopLevel::TypeAlias { name, .. } => Some(name.clone()),
        _ => None,
    }
}

/// Reject two declarations of the same name coming from different files.
fn check_duplicate_declarations(
    declarations: &[ast::TopLevel],
    module_of_decl: &[(String, String)],
) -> Result<(), String> {
    use std::collections::HashMap;

    // Map every imported declaration name to the module that defined it.
    let mut origin: HashMap<&str, &str> = HashMap::new();
    let mut duplicates: Vec<(String, String, String)> = Vec::new();
    for (name, module) in module_of_decl {
        if let Some(previous) = origin.get(name.as_str()) {
            if *previous != module.as_str() {
                duplicates.push((name.clone(), (*previous).to_string(), module.clone()));
            }
        } else {
            origin.insert(name.as_str(), module.as_str());
        }
    }

    let _ = declarations;

    if let Some((name, first, second)) = duplicates.first() {
        return Err(format!(
            "duplicate definition of '{}': defined in both '{}.lpp' and '{}.lpp'.\n  \
             Imported modules share one global namespace in L++, so two modules \
             cannot define the same function, struct, enum, const or type name.\n  \
             Rename one of them.",
            name, first, second
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_stdlib_with_precedence_over_packages() {
        let base_dir = Path::new("tests");
        let res = resolve_module_filepath("math", base_dir);
        assert!(res.is_ok(), "math module should resolve");
        let path = res.unwrap();
        assert!(path.to_string_lossy().contains("stdlib"), "stdlib/math.lpp should take precedence");
    }
}
