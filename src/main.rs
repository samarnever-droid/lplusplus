#[path = "frontend/ast.rs"]
mod ast;
mod builtins;
#[path = "frontend/lexer.rs"]
mod lexer;
#[path = "frontend/parser.rs"]
mod parser;
#[path = "analysis/semantic.rs"]
mod semantic;
#[path = "analysis/typecheck.rs"]
mod typecheck;
#[path = "analysis/escape.rs"]
mod escape;
#[path = "backend/codegen.rs"]
mod codegen;
#[path = "backend/c_runtime_headers.rs"]
mod c_runtime_headers;
#[path = "backend/cranelift/mod.rs"]
pub mod cranelift_backend;
#[path = "mir/mod.rs"]
pub mod mir;
mod pm;

use std::fs;
use std::env;
use std::time::Instant;

fn main() {
    let mut args: Vec<String> = env::args().collect();
    
    // The CLI has two intentionally separate modes:
    // - package commands (`build`, `run`, `test`, …) operate on lpp.toml;
    // - source commands (`check file.lpp`, `emit file.lpp`) operate on one file.
    let mut explicit_emit = false;
    let mut source_check_command = false;
    if args.len() > 2 && args[1] == "emit" {
        explicit_emit = true;
        args.remove(1);
    } else if args.len() > 2 && args[1] == "check" && args[2].ends_with(".lpp") {
        source_check_command = true;
        args.remove(1);
    }

    if args.len() > 1 {
        let first_arg = &args[1];
        if first_arg == "init" || first_arg == "install" || first_arg == "add" ||
           first_arg == "remove" || first_arg == "update" || first_arg == "check" ||
           first_arg == "build" || first_arg == "run" || first_arg == "test" ||
           first_arg == "new" || first_arg == "search" || first_arg == "list" ||
           first_arg == "tree" || first_arg == "metadata" || first_arg == "clean" ||
           first_arg == "outdated" || first_arg == "help" {
            pm::run_command(&args[1..]);
            return;
        }
    }
    
    let mut filename = None;
    let mut dump_ast = false;
    let mut dump_symbols = false;
    let mut dump_types = false;
    let mut dump_escape = false;
    let mut dump_mir = false;
    let mut dump_c = false;
    let mut check_only = source_check_command;
    let mut emit_object = false;
    
    for arg in args.iter().skip(1) {
        if arg == "--version" || arg == "-v" {
            println!("L++ Compiler v0.1.2");
            return;
        } else if arg == "--help" || arg == "-h" {
            println!("L++ (L Plus Plus) Compiler, Codegen Backend & Package Manager");
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
            println!("  build            Build project into a native binary");
            println!("  run              Compile and run the project binary");
            println!("  test             Compile and run tests inside tests/");
            println!("\nSource Commands:");
            println!("  lpp check <file.lpp>          Type-check one file; emit no artifacts");
            println!("  lpp emit <file.lpp>           Emit C source next to the input file");
            println!("  lpp emit <file.lpp> --aot     Emit C source and a Cranelift object file");
            println!("  lpp <file.lpp>                Legacy source invocation; emits C with guidance");
            println!("\nOptions (Compiler):");
            println!("  -v, --version    Show L++ compiler version");
            println!("  -h, --help       Show this help menu");
            println!("  --check          Check a single file without compiling");
            println!("  --dump-ast       Dump the Abstract Syntax Tree");
            println!("  --dump-symbols   Dump the resolved symbol table");
            println!("  --dump-types     Dump the typechecker type table");
            println!("  --dump-escape    Dump the escape analysis classifications");
            println!("  --dump-mir       Dump the generated Mid-level IR (MIR)");
            println!("  --dump-c         Dump the generated transpiled C code");
            println!("\nEnvironment Variables:");
            println!("  LPP_AOT=1        Enable Cranelift AOT compilation to native object file");
            println!("  LPP_LINKER=direct Use lpp-link on installed Linux x86-64 builds (experimental)");
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
        } else if arg == "--dump-c" {
            dump_c = true;
        } else if arg == "--check" {
            check_only = true;
        } else if arg == "--emit-object" || arg == "--aot" {
            emit_object = true;
        } else if !arg.starts_with('-') {
            filename = Some(arg.as_str());
        }
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
            eprintln!("Lexer error: {}", e);
            return;
        }
    };
    let lex_time = lex_start.elapsed();

    let parse_start = Instant::now();
    let mut parser = parser::Parser::new(tokens);
    let mut ast = match parser.parse() {
        Ok(ast) => ast,
        Err(e) => {
            eprintln!("Parser error: {}", e);
            return;
        }
    };
    let parse_time = parse_start.elapsed();

    let file_path = std::path::Path::new(&filename);
    let base_dir = file_path.parent().unwrap_or(std::path::Path::new("."));
    let mut imported_files = std::collections::HashSet::new();
    if let Err(e) = resolve_local_imports(&mut ast.declarations, &mut imported_files, base_dir) {
        eprintln!("Import error: {}", e);
        return;
    }

    let sem_start = Instant::now();
    let mut resolver = semantic::Resolver::new();
    if let Err(e) = resolver.resolve_program(&mut ast) {
        eprintln!("Semantic error: {}", e);
        return;
    }
    let sem_time = sem_start.elapsed();

    let ty_start = Instant::now();
    let mut type_table = {
        let mut type_checker = typecheck::TypeChecker::new(&mut resolver.table);
        if let Err(e) = type_checker.check_program(&ast) {
            eprintln!("Type check error: {}", e);
            return;
        }
        type_checker.type_table
    };
    let ty_time = ty_start.elapsed();
    
    if check_only {
        let total_time = total_start.elapsed();
        if env::var("BENCHMARK").is_ok() {
            println!("TIMING_JSON: {{\"io\": {}, \"lex\": {}, \"parse\": {}, \"semantic\": {}, \"typecheck\": {}, \"total\": {}}}", 
               io_time.as_secs_f64(), lex_time.as_secs_f64(), parse_time.as_secs_f64(), sem_time.as_secs_f64(), ty_time.as_secs_f64(), total_time.as_secs_f64());
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
            mir::pass_arc::run_arc_insertion_pass(&mut mir_program, &storage);
            
            if dump_mir {
                println!("--- Generated MIR ---");
                println!("{}", mir_program);
            }
            mir_time = mir_start.elapsed();

            let mut aot_time = std::time::Duration::ZERO;
            // AOT compilation via Cranelift
            // Enabled by setting the LPP_AOT environment variable.
            if env::var("LPP_AOT").is_ok() || emit_object {
                let aot_start = Instant::now();
                match cranelift_backend::compiler::AotCompiler::compile(&mir_program, &type_table) {
                    Ok(obj_bytes) => {
                        let obj_path = filename.replace(".lpp", ".o");
                        if let Err(e) = fs::write(&obj_path, &obj_bytes) {
                            eprintln!("Failed to write {}: {}", obj_path, e);
                        } else if env::var("BENCHMARK").is_err() && !dump_ast && !dump_symbols && !dump_types && !dump_escape && !dump_mir {
                            println!("[L++] AOT object file written to {}", obj_path);
                        }
                    }
                    Err(e) => eprintln!("[L++] AOT error: {}", e),
                }
                aot_time = aot_start.elapsed();
            }

            let codegen_start = Instant::now();
            let mut cg = codegen::Codegen::new(&resolver.table, &type_table, &storage);
            let c_code = cg.generate(&ast);
            let codegen_time = codegen_start.elapsed();
            let c_path = filename.replace(".lpp", ".c");
            if let Err(e) = fs::write(&c_path, &c_code) {
                eprintln!("Failed to write {}: {}", c_path, e);
            }
            
            if dump_c {
                println!("--- Generated C Code ---");
                println!("{}", c_code);
            }
            
            let total_time = total_start.elapsed();

            if env::var("BENCHMARK").is_ok() {
                println!("TIMING_JSON: {{\"io\": {}, \"lex\": {}, \"parse\": {}, \"semantic\": {}, \"typecheck\": {}, \"escape\": {}, \"mir\": {}, \"aot\": {}, \"c_codegen\": {}, \"total\": {}}}", 
                   io_time.as_secs_f64(), lex_time.as_secs_f64(), parse_time.as_secs_f64(), sem_time.as_secs_f64(), ty_time.as_secs_f64(), esc_time.as_secs_f64(), mir_time.as_secs_f64(), aot_time.as_secs_f64(), codegen_time.as_secs_f64(), total_time.as_secs_f64());
            } else if !dump_ast && !dump_symbols && !dump_types && !dump_escape && !dump_mir && !dump_c {
                println!("L++ v0.1.2\n");
                if explicit_emit {
                    println!("Artifacts emitted next to the source file.");
                } else {
                    println!("Source compilation completed; emitted C source next to the input.");
                    println!("Tip: use `lpp emit <file.lpp>` for explicit artifact emission or `lpp build` for a package executable.");
                }
                println!("Time: {:.1} ms", total_time.as_secs_f64() * 1000.0);
            }
        },
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
        if let ast::TopLevel::Import(module) = decl {
            if module != "json" && !imported_files.contains(module) {
                imports_to_process.push(module.clone());
            }
        }
    }
    
    for module in imports_to_process {
        imported_files.insert(module.clone());
        let mut filepath = base_dir.join(format!("{}.lpp", module));
        if !filepath.exists() {
            // Check in .lpp_packages/module/module.lpp
            let pkg_path = std::path::Path::new(".lpp_packages")
                .join(&module)
                .join(format!("{}.lpp", module));
            if pkg_path.exists() {
                filepath = pkg_path;
            } else {
                // Check in .lpp_packages/module/src/module.lpp
                let pkg_src_path = std::path::Path::new(".lpp_packages")
                    .join(&module)
                    .join("src")
                    .join(format!("{}.lpp", module));
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
