#[path = "frontend/ast.rs"]
mod ast;
mod builtins;
mod config;
#[path = "backend/cranelift/mod.rs"]
pub mod cranelift_backend;
#[path = "analysis/escape.rs"]
mod escape;
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

    // Link with lpp-link direct native linker
    let lpp_link_bin = lpp_bin
        .parent()
        .map(|dir| dir.join(format!("lpp-link{}", env::consts::EXE_SUFFIX)))
        .unwrap_or_else(|| PathBuf::from(format!("lpp-link{}", env::consts::EXE_SUFFIX)));

    let runtime_src = resolve_runtime_source_for_bootstrap(&pm_main)
        .ok_or_else(|| "lpp_runtime.c not found".to_string())?;

    let runtime_min_name = if cfg!(target_os = "windows") { "lpp_runtime_min.obj" } else { "lpp_runtime_min.o" };
    let lib_dir = runtime_src.parent().unwrap_or_else(|| Path::new("."));
    let runtime_min_obj = lib_dir.join(runtime_min_name);

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

    let link_status = link_cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        .status()
        .map_err(|e| format!("failed to link PM binary via lpp-link: {e}"))?;

    let _ = fs::remove_file(&pm_obj);

    if !link_status.success() {
        return Err("linking self-hosted PM failed".to_string());
    }

    Ok(pm_bin)
}

/// Delegate a PM command to the self-hosted PM binary.
/// ALL PM commands route here. If the self-hosted PM is unavailable or
/// signals `__DELEGATE__`, the Rust PM takes over.
fn run_self_hosted_pm(args: &[String]) {
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("help");

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
                let rest: Vec<&str> = args.iter().skip(1).map(|s| s.as_str()).collect();
                child.env("LPP_PM_ARGS", rest.join("\x1f"));
            }
        }
        "remove" | "search" => {
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
    let mut emit_object = is_emit_cmd || env::var("LPP_AOT").is_ok() || env::var("LPP_AOT_ONLY").is_ok();

    for arg in args.iter().skip(1) {
        if arg == "--version" || arg == "-v" {
            println!("L++ Compiler v2.0.0 (Pure Native AOT)");
            return;
        } else if arg == "--help" || arg == "-h" {
            println!("L++ (L Plus Plus) Pure Native Compiler, Cranelift AOT & Direct Linker Toolchain v2.0.0");
            println!("Usage: lpp [command] [options]");
            println!("\nCommands (Package Manager):");
            println!("  new <name>       Create a new L++ package");
            println!("  init <name>      Initialize a project in the current directory");
            println!("  install          Resolve and install dependencies");
            println!("  add <name>       Add a dependency to lpp.toml");
            println!("  remove <name>    Remove a dependency from lpp.toml");
            println!("  update           Refresh dependencies and rewrite lpp.lock");
            println!("  search <query>   Search the package registry");
            println!("  list             List direct dependencies from lpp.toml");
            println!("  tree             Print dependency tree/lockfile view");
            println!("  metadata         Print package metadata summary");
            println!("  outdated         Show dependencies without pinned versions");
            println!("  clean            Remove build output and generated artifacts");
            println!("  check            Check the project for compilation errors");
            println!("  build            Build project into a native binary via direct lpp-link");
            println!("  run              Compile and run the project native executable");
            println!("  test             Compile and run tests inside tests/");
            println!("\nSource Commands:");
            println!("  lpp <file.lpp>               Compile L++ source into direct native executable");
            println!("  lpp check <file.lpp>         Type-check one file; emit no artifacts");
            println!("  lpp emit <file.lpp> --aot    Emit Cranelift native object file (.o / .obj)");
            println!("\nOptions (Compiler):");
            println!("  -v, --version    Show L++ compiler version");
            println!("  -h, --help       Show this help menu");
            println!("  --check          Check a single file without compiling");
            println!("  --dump-ast       Dump the Abstract Syntax Tree");
            println!("  --dump-symbols   Dump the resolved symbol table");
            println!("  --dump-types     Dump the typechecker type table");
            println!("  --dump-escape    Dump the escape analysis classifications");
            println!("  --dump-mir       Dump the generated Mid-level IR (MIR)");
            println!("  --linker direct  Use lpp-link (no external compiler needed)");
            println!("  --linker host    Use system cc/cl.exe linker");
            println!("\nConfiguration:");
            println!("  config                       Show current config (~/.lpp/config.json)");
            println!("  config set linker <value>    Set default linker (direct|host|auto)");
            println!("\nEnvironment Variables:");
            println!("  BENCHMARK=1      Suppress descriptive text and print sub-millisecond JSON timings");
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
        } else if arg == "--emit-object" || arg == "--aot" {
            emit_object = true;
        } else if arg == "--linker" {
            // Handled below after arg loop
        } else if !arg.starts_with('-') {
            filename = Some(arg.as_str());
        }
    }

    // Parse --linker <value> (needs look-ahead)
    let mut cli_linker: Option<String> = None;
    for i in 1..args.len() {
        if args[i] == "--linker" && i + 1 < args.len() {
            cli_linker = Some(args[i + 1].clone());
        }
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
        for fpath in &all_files {
            let input = match fs::read_to_string(fpath) {
                Ok(c) => c, Err(e) => { all_fails.push(format!("{}: read: {}", fpath.display(), e)); continue; }
            };
            let mut l = lexer::Lexer::new(&input);
            let tokens = match l.tokenize() {
                Ok(t) => t, Err(e) => { all_fails.push(format!("{}: lex: {}", fpath.display(), e)); continue; }
            };
            let mut par = parser::Parser::new(tokens);
            let mut ast = match par.parse() {
                Ok(a) => a, Err(e) => { all_fails.push(format!("{}: syntax: {}", fpath.display(), e)); continue; }
            };
            let base = fpath.parent().unwrap_or(Path::new("."));
            let mut imp = std::collections::HashSet::new();
            if let Err(e) = resolve_local_imports(&mut ast.declarations, &mut imp, base) {
                all_fails.push(format!("{}: import: {}", fpath.display(), e)); continue;
            }
            let mut res = semantic::Resolver::new();
            if let Err(e) = res.resolve_program(&mut ast) {
                all_fails.push(format!("{}: semantic: {}", fpath.display(), e)); continue;
            }
            let mut tc = typecheck::TypeChecker::new(&mut res.table);
            if let Err(e) = tc.check_program(&ast) {
                all_fails.push(format!("{}: type: {}", fpath.display(), e)); continue;
            }
            p += 1;
        }
        let el = ta.elapsed();
        if all_fails.is_empty() {
            println!("[L++] --checkall: OK — {} file(s) passed in {:.1} ms", p, el.as_secs_f64() * 1000.0);
        } else {
            eprintln!("[L++] --checkall: {} passed, {} FAILED:", p, all_fails.len());
            for f in &all_fails { eprintln!("  {}", f); }
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
            eprintln!("Lexer Error in '{}':\n  {}", filename, e);
            return;
        }
    };
    let lex_time = lex_start.elapsed();

    let parse_start = Instant::now();
    let mut parser = parser::Parser::new(tokens);
    let mut ast = match parser.parse() {
        Ok(ast) => ast,
        Err(e) => {
            eprintln!("Syntax Error in '{}':\n  {}", filename, e);
            return;
        }
    };
    let parse_time = parse_start.elapsed();

    let file_path = std::path::Path::new(&filename);
    let base_dir = file_path.parent().unwrap_or(std::path::Path::new("."));
    let mut imported_files = std::collections::HashSet::new();
    if let Err(e) = resolve_local_imports(&mut ast.declarations, &mut imported_files, base_dir) {
        eprintln!("Import Error in '{}':\n  {}", filename, e);
        return;
    }

    let sem_start = Instant::now();
    let mut resolver = semantic::Resolver::new();
    if let Err(e) = resolver.resolve_program(&mut ast) {
        eprintln!("Semantic Error in '{}':\n  {}", filename, e);
        return;
    }
    let sem_time = sem_start.elapsed();

    let ty_start = Instant::now();
    let mut type_table = {
        let mut type_checker = typecheck::TypeChecker::new(&mut resolver.table);
        if let Err(e) = type_checker.check_program(&ast) {
            eprintln!("Type Error in '{}':\n  {}", filename, e);
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
            let mut mir_ctx = mir::lower::MirLowerCtx::new(&resolver.table, &mut type_table);
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
            let obj_bytes = match cranelift_backend::compiler::AotCompiler::compile(&mir_program, &type_table) {
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

            if let Err(e) = pm::direct_link_binary(Path::new(&obj_path), Path::new(&exe_path)) {
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
                println!("L++ v2.0.0 (Pure Native Executable)\n");
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

fn resolve_local_imports(
    declarations: &mut Vec<ast::TopLevel>,
    imported_files: &mut std::collections::HashSet<String>,
    base_dir: &std::path::Path,
) -> Result<(), String> {
    let mut new_decls = Vec::new();
    let mut imports_to_process = Vec::new();

    for decl in declarations.iter() {
        if let ast::TopLevel::Import(import_kind) = decl {
            let (path, _items) = match import_kind {
                ast::ImportKind::Module { path, .. } => (path.clone(), None),
                ast::ImportKind::Selective { path, items } => (path.clone(), Some(items.clone())),
            };
            // Convert dotted path to filesystem path: ["utils", "math"] → "utils/math"
            let module = path.join("/");
            let module_name = path.last().cloned().unwrap_or_default();
            if module_name != "json" && !imported_files.contains(&module) {
                imports_to_process.push(module);
            }
        }
    }

    for module in imports_to_process {
        imported_files.insert(module.clone());
        // module is "math" or "utils/math" for dotted paths
        let leaf_name = module.split('/').last().unwrap_or(&module);
        let mut filepath = base_dir.join(format!("{}.lpp", module));
        if !filepath.exists() {
            // Check in .lpp_packages/leaf/leaf.lpp
            let pkg_path = std::path::Path::new(".lpp_packages")
                .join(leaf_name)
                .join(format!("{}.lpp", leaf_name));
            if pkg_path.exists() {
                filepath = pkg_path;
            } else {
                // Check in .lpp_packages/leaf/src/leaf.lpp
                let pkg_src_path = std::path::Path::new(".lpp_packages")
                    .join(leaf_name)
                    .join("src")
                    .join(format!("{}.lpp", leaf_name));
                if pkg_src_path.exists() {
                    filepath = pkg_src_path;
                } else {
                    return Err(format!(
                        "Imported library file '{}' not found in local directory or .lpp_packages",
                        module
                    ));
                }
            }
        }
        let content = std::fs::read_to_string(&filepath)
            .map_err(|e| format!("Failed to read library '{}': {}", filepath.display(), e))?;

        let mut lex = lexer::Lexer::new(&content);
        let tokens = lex.tokenize()?;
        let mut par = parser::Parser::new(tokens);
        let mut lib_ast = par.parse()?;

        // Recursively resolve imports of the library using its own base directory
        let lib_base_dir = filepath.parent().unwrap_or(std::path::Path::new("."));
        resolve_local_imports(&mut lib_ast.declarations, imported_files, lib_base_dir)?;

        new_decls.extend(lib_ast.declarations);
    }

    declarations.extend(new_decls);
    Ok(())
}
