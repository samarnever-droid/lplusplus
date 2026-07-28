#[path = "frontend/ast.rs"]
mod ast;
mod builtins;
mod config;
mod diagnostics;
#[path = "backend/cranelift/mod.rs"]
pub mod cranelift_backend;
#[path = "analysis/escape.rs"]
mod escape;
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
#[path = "analysis/typecheck.rs"]
mod typecheck;

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

/// Delegate a PM command to the self-hosted PM binary.
/// ALL PM commands route here. If the self-hosted PM is unavailable or
/// signals `__DELEGATE__`, the Rust PM takes over.
fn run_self_hosted_pm(args: &[String]) {
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("help");

    if cmd == "create" || cmd == "dev" || cmd == "lreact" || args.iter().any(|a| a == "web" || a == "--release") {
        pm::run_command(args);
        return;
    }

    let pm_bin = match bootstrap_self_hosted_pm() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[L++] Self-hosted PM unavailable: {e}");
            eprintln!("[L++] Falling back to built-in Rust PM.");
            pm::run_command(args);
            return;
        }
    };

    // Build owned env strings (avoid borrow issues)
    let mut child = std::process::Command::new(&pm_bin);
    child.env("LPP_PM_CMD", cmd);

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
        "remove" | "search" | "install" => {
            if let Some(a1) = args.get(1) {
                child.env("LPP_PM_ARG1", a1.as_str());
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
                pm::run_command(args);
                return;
            }

            if !out.status.success() {
                pm::run_command(args);
            }
        }
        Err(e) => {
            eprintln!("[L++] Failed to run self-hosted PM: {e}. Falling back.");
            pm::run_command(args);
        }
    }
}



fn main() {
    let mut args: Vec<String> = env::args().collect();

    // The CLI has two intentionally separate modes:
    // - package commands (`build`, `run`, `test`, …) operate on lpp.toml;
    // - source commands (`check file.lpp`, `emit file.lpp`) operate on one file.
    let mut source_check_command = false;
    let mut is_emit_cmd = false;
    if args.len() > 2 && args[1] == "emit" {
        is_emit_cmd = true;
        args.remove(1);
    } else if args.len() > 2 && args[1] == "check" && args[2].ends_with(".lpp") {
        source_check_command = true;
        args.remove(1);
    }

    // Handle config command
    if args.len() > 1 && args[1] == "config" {
        if args.len() > 2 && args[2] == "set" && args.len() > 4 && args[3] == "linker" {
            let mut cfg = config::LppConfig::load_or_create();
            let val = &args[4];
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
        } else {
            let cfg = config::LppConfig::load_or_create();
            cfg.print_summary();
        }
        return;
    }

    if args.len() > 1 {
        let first_arg = &args[1];
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
            || first_arg == "help"
            || first_arg == "bench"
        {
            run_self_hosted_pm(&args[1..]);
            return;
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
    let mut cli_target: Option<String> = None;

    while idx < args.len() {
        let arg = &args[idx];
        if arg == "--version" || arg == "-v" {
            println!("L++ Compiler v4.5.0 (Pure Native AOT)");
            return;
        } else if arg == "--help" || arg == "-h" {
            println!("L++ (L Plus Plus) v4.5.0 — Pure Native Compiler & Toolchain");
            println!("Cranelift AOT backend, 9 MIR optimization passes, direct ELF/PE/Mach-O linker");
            println!();
            println!("Usage: lpp <file.lpp> [options]");
            println!("       lpp <command> [args]");
            println!();
            println!("Compilation:");
            println!("  lpp <file.lpp>             Compile to native executable (direct lpp-link)");
            println!("  lpp <file.lpp> --emit-obj  Emit native object file only (.o / .obj)");
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
            println!("  outdated         Show dependencies without pinned versions");
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
            println!("  --dump-escape    Dump escape analysis classifications");
            println!("  --dump-mir       Dump Mid-level IR (MIR)");
            println!();
            println!("Linker:");
            println!("  --linker direct  Use lpp-link (no external tools needed)");
            println!("  --linker host    Use system cc/cl.exe (required for FFI/extern)");
            println!();
            println!("Configuration:");
            println!("  config                         Show config (~/.lpp/config.json)");
            println!("  config set linker <value>      Set default linker (direct|host|auto)");
            println!();
            println!("Options:");
            println!("  -v, --version    Show version");
            println!("  -h, --help       Show this help");
            println!();
            println!("Language Features (v4.5.0):");
            println!("  Functions, default params, closures, threads");
            println!("  Structs, enums, match with bindings");
            println!("  Generics: def foo[T](x: T) -> T");
            println!("  Traits:   trait Name / impl Trait for Type (static + dynamic dispatch)");
            println!("  FFI:      extern \"C\" link \"SDL2\" (call any C library)");
            println!("  Try:      result? operator for error propagation");
            println!("  Builtins: 100+ (strings, lists, maps, files, network, JSON, GUI)");
            println!("  Ownership: ARC + escape analysis, cycle rejection");
            println!();
            println!("Environment:");
            println!("  BENCHMARK=1           Print JSON timings instead of descriptive output");
            println!("  LPP_AOT_OPT=speed    Set Cranelift optimization level (none|speed|speed_and_size)");
            return;
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
        } else if arg == "--checkall" {
            check_all = true;
        } else if arg == "--fix" {
            do_fix = true;
        } else if arg == "--emit-object" || arg == "--aot" {
            emit_object = true;
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
        let mut p = 0usize;
        let mut all_fails: Vec<String> = Vec::new();
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
            return;
        }
        all_files.sort();
        eprintln!("[L++] --checkall: checking {} file(s)...", all_files.len());
        let ta = Instant::now();
        let mut fixed_files_count = 0usize;
        for fpath in &all_files {
            let mut input = match fs::read_to_string(fpath) {
                Ok(c) => c, Err(e) => { all_fails.push(format!("{}:1:1: read: {}", fpath.display(), e)); continue; }
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
                    if do_fix {
                        if fs::write(fpath, &new_src).is_ok() {
                            fixed_files_count += 1;
                            println!("[L++] Fixed {}:{}:{}: {} [{}]", fpath.display(), line, col, stage, desc);
                            p += 1;
                            continue;
                        }
                    }
                    all_fails.push(format!("{}:{}:{}: {}: {} [suggestion: {}]", fpath.display(), line, col, stage, msg, desc));
                } else {
                    all_fails.push(format!("{}:{}:{}: {}: {}", fpath.display(), line, col, stage, msg));
                }
            } else {
                p += 1;
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
        return;
    }

    let filename = match filename {
        Some(f) => f,
        None => {
            eprintln!("[L++] Error: No input file specified.");
            eprintln!("Usage: lpp [file.lpp] [options]");
            return;
        }
    };

    let total_start = Instant::now();

    let io_start = Instant::now();
    let input = match fs::read_to_string(filename) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("Failed to read {}: {}", filename, e);
            return;
        }
    };
    let io_time = io_start.elapsed();

    let lex_start = Instant::now();
    let mut lexer = lexer::Lexer::new(&input);
    let tokens = match lexer.tokenize() {
        Ok(tokens) => tokens,
        Err(e) => {
            eprint!("{}", diagnostics::render_error_string(&filename, &input, diagnostics::DiagnosticKind::Lexer, &e));
            return;
        }
    };
    let lex_time = lex_start.elapsed();

    let parse_start = Instant::now();
    let mut parser = parser::Parser::new(tokens);
    let mut ast = match parser.parse() {
        Ok(ast) => ast,
        Err(e) => {
            eprint!("{}", diagnostics::render_error_string(&filename, &input, diagnostics::DiagnosticKind::Syntax, &e));
            return;
        }
    };
    let parse_time = parse_start.elapsed();

    let file_path = std::path::Path::new(&filename);
    let base_dir = file_path.parent().unwrap_or(std::path::Path::new("."));
    let mut imported_files = std::collections::HashSet::new();
    if let Err(e) = resolve_local_imports(&mut ast.declarations, &mut imported_files, base_dir) {
        eprint!("{}", diagnostics::render_error_string(&filename, &input, diagnostics::DiagnosticKind::Import, &e));
        return;
    }

    if let Err(e) = monomorph::Monomorphizer::process_program(&mut ast) {
        eprint!("{}", diagnostics::render_error_string(&filename, &input, diagnostics::DiagnosticKind::Semantic, &e));
        return;
    }

    let sem_start = Instant::now();
    let mut resolver = semantic::Resolver::new();
    if let Err(e) = resolver.resolve_program(&mut ast) {
        eprint!("{}", diagnostics::render_error_string(&filename, &input, diagnostics::DiagnosticKind::Semantic, &e));
        return;
    }
    let sem_time = sem_start.elapsed();

    let ty_start = Instant::now();
    let mut type_table = {
        let mut type_checker = typecheck::TypeChecker::new(&mut resolver.table);
        if let Err(e) = type_checker.check_program(&ast) {
            eprint!("{}", diagnostics::render_error_string(&filename, &input, diagnostics::DiagnosticKind::Type, &e));
            return;
        }
        type_checker.type_table
    };
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
        return;
    }

    #[allow(unused_assignments)]
    let mut mir_time = std::time::Duration::ZERO;
    let esc_start = Instant::now();
    match escape::EscapeAnalyzer::analyze(&ast, &resolver.table, &type_table) {
        Ok(storage) => {
            let esc_time = esc_start.elapsed();
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
            if dump_escape {
                println!("--- Storage Classification Map ---");
                for (id, class) in &storage {
                    let binding = &resolver.table.bindings[id.0];
                    println!("  Binding '{}' -> {:?}", binding.name, class);
                }
            }

            let mir_start = Instant::now();
            let mut mir_ctx = mir::lower::MirLowerCtx::new(&resolver.table, &mut type_table, &ast);
            let mut mir_program = match mir_ctx.lower_program(&ast) {
                Ok(program) => program,
                Err(e) => {
                    eprintln!("MIR lowering error: {}", e);
                    return;
                }
            };
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
            mir::pass_arc::run_arc_insertion_pass(&mut mir_program, &storage);

            if dump_mir {
                println!("--- Generated MIR ---");
                println!("{}", mir_program);
            }
            mir_time = mir_start.elapsed();

            // L++ 2.0 Pure Native Cranelift AOT Backend
            let aot_start = Instant::now();
            let has_extern_decl = ast
                .declarations
                .iter()
                .any(|d| matches!(d, crate::ast::TopLevel::Extern(_)));
            let obj_bytes = match cranelift_backend::compiler::AotCompiler::compile_with_options(
                &mir_program,
                &type_table,
                has_extern_decl,
            ) {
                Ok(bytes) => bytes,
                Err(e) => {
                    eprintln!("[L++] Cranelift AOT compilation error: {}", e);
                    return;
                }
            };
            let aot_time = aot_start.elapsed();

            let ext = if cfg!(target_os = "windows") { "obj" } else { "o" };
            let obj_path = filename.replace(".lpp", &format!(".{}", ext));
            if let Err(e) = fs::write(&obj_path, &obj_bytes) {
                eprintln!("Failed to write object file {}: {}", obj_path, e);
                return;
            }

            let total_time = total_start.elapsed();

            if check_only {
                return;
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
                        esc_time.as_secs_f64(),
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
                    println!("[L++] Native Cranelift object emitted at {}", obj_path);
                    println!("Time: {:.1} ms", total_time.as_secs_f64() * 1000.0);
                }
                return;
            }

            // Direct Native Executable Link via lpp-link
            let exe_ext = std::env::consts::EXE_SUFFIX;
            let exe_path = filename.replace(".lpp", exe_ext);

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
            let has_extern = ast.declarations.iter().any(|d| matches!(d, crate::ast::TopLevel::Extern(_)));
            let env_linker = env::var("LPP_LINKER").ok();
            let effective_linker = cli_linker.or(env_linker);
            let use_host = effective_linker.as_deref() == Some("host")
                || (effective_linker.as_deref() != Some("direct") && (has_extern || !link_libs.is_empty()));

            let link_result = if use_host {
                #[cfg(windows)]
                pm::load_msvc_env();
                pm::host_link_binary(Path::new(&obj_path), Path::new(&exe_path), &link_libs)
            } else {
                pm::direct_link_binary(Path::new(&obj_path), Path::new(&exe_path))
            };
            if let Err(e) = link_result {
                eprintln!("[L++] Native Link Error: {}", e);
                return;
            }
            let _ = fs::remove_file(&obj_path);

            if env::var("BENCHMARK").is_ok() {
                println!(
                    "TIMING_JSON: {{\"io\": {}, \"lex\": {}, \"parse\": {}, \"semantic\": {}, \"typecheck\": {}, \"escape\": {}, \"mir\": {}, \"aot\": {}, \"total\": {}}}",
                    io_time.as_secs_f64(),
                    lex_time.as_secs_f64(),
                    parse_time.as_secs_f64(),
                    sem_time.as_secs_f64(),
                    ty_time.as_secs_f64(),
                    esc_time.as_secs_f64(),
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
                println!("L++ v4.2.2 (Pure Native Executable)\n");
                println!("Compiled and linked native binary: {}", exe_path);
                println!("Time: {:.1} ms", total_time.as_secs_f64() * 1000.0);
            }
        }
        Err(e) => {
            eprintln!("Escape Analysis error: {}", e);
            return;
        }
    }
}

fn find_stdlib_module(clean_module: &str, leaf_name: &str) -> Option<PathBuf> {
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
        let local_path = base_dir.join(format!("{}.lpp", module));
        if local_path.exists() {
            return Ok(local_path);
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
    base_dir: &std::path::Path,
) -> Result<(), String> {
    let mut new_decls = Vec::new();
    let mut imports_to_process = Vec::new();
    let mut module_of_decl: Vec<(String, String)> = Vec::new();

    for decl in declarations.iter() {
        if let ast::TopLevel::Import(import_kind) = decl {
            let (path, _items) = match import_kind {
                ast::ImportKind::Module { path, .. } => (path.clone(), None),
                ast::ImportKind::Selective { path, items } => (path.clone(), Some(items.clone())),
            };
            let module = path.join("/");
            let module_name = path.last().cloned().unwrap_or_default();
            if module_name != "json" && !imported_files.contains(&module) {
                imports_to_process.push(module);
            }
        }
    }

    for module in imports_to_process {
        let filepath = resolve_module_filepath(&module, base_dir)?;
        let canonical_key = filepath.canonicalize().unwrap_or_else(|_| filepath.clone()).to_string_lossy().to_string();

        if imported_files.contains(&canonical_key) || imported_files.contains(&module) {
            continue;
        }
        imported_files.insert(canonical_key.clone());
        imported_files.insert(module.clone());

        let content = std::fs::read_to_string(&filepath)
            .map_err(|e| format!("Failed to read library '{}': {}", filepath.display(), e))?;

        let mut lex = lexer::Lexer::new(&content);
        let tokens = lex.tokenize()?;
        let mut par = parser::Parser::new(tokens);
        let mut lib_ast = par.parse()?;

        // Recursively resolve imports of the library using its own base directory
        let lib_base_dir = filepath.parent().unwrap_or(std::path::Path::new("."));
        resolve_local_imports(&mut lib_ast.declarations, imported_files, lib_base_dir)?;

        // Record which module each declaration came from so that a name
        // collision can name both sides.
        for decl in &lib_ast.declarations {
            if let Some(name) = declaration_name(decl) {
                module_of_decl.push((name, module.clone()));
            }
        }

        new_decls.extend(lib_ast.declarations);
    }

    declarations.extend(new_decls);

    // Imported modules are flattened into one global declaration list, so two
    // modules defining the same function name would silently collapse into a
    // single symbol: `a.shared()` and `b.shared()` would both call whichever
    // was linked last, with no diagnostic. Detect that here and fail loudly.
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
