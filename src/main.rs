#[path = "frontend/ast.rs"]
mod ast;
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
#[path = "backend/cranelift/mod.rs"]
pub mod cranelift_backend;
#[path = "mir/mod.rs"]
pub mod mir;

use std::fs;
use std::env;
use std::time::Instant;

fn main() {
    let args: Vec<String> = env::args().collect();
    let filename = if args.len() > 1 { &args[1] } else { "escape_demo.lpp" };
    
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
    let ast = match parser.parse() {
        Ok(ast) => ast,
        Err(e) => {
            eprintln!("Parser error: {}", e);
            return;
        }
    };
    let parse_time = parse_start.elapsed();

    let sem_start = Instant::now();
    let mut resolver = semantic::Resolver::new();
    if let Err(e) = resolver.resolve_program(&ast) {
        eprintln!("Semantic error: {}", e);
        return;
    }
    let sem_time = sem_start.elapsed();

    let ty_start = Instant::now();
    let type_table = {
        let mut type_checker = typecheck::TypeChecker::new(&mut resolver.table);
        if let Err(e) = type_checker.check_program(&ast) {
            eprintln!("Type check error: {}", e);
            return;
        }
        type_checker.type_table
    };
    let ty_time = ty_start.elapsed();

    #[allow(unused_assignments)]
    let mut mir_time = std::time::Duration::ZERO;
    let esc_start = Instant::now();
    match escape::EscapeAnalyzer::analyze(&ast, &resolver.table, &type_table) {
        Ok(storage) => {
            if env::var("BENCHMARK").is_err() {
                println!("\nStorage Classification Map:");
                for (id, class) in &storage {
                    let binding = &resolver.table.bindings[id.0];
                    println!("  Binding '{}' -> {:?}", binding.name, class);
                }
            }
            
            let mir_start = Instant::now();
            let mut mir_ctx = mir::lower::MirLowerCtx::new(&resolver.table, &type_table);
            let mut mir_program = mir_ctx.lower_program(&ast);
            mir::pass_arc::run_arc_insertion_pass(&mut mir_program, &storage);
            
            if env::var("BENCHMARK").is_err() {
                println!("\n--- Generated MIR ---");
                println!("{}", mir_program);
            }
            mir_time = mir_start.elapsed();

            // AOT compilation via Cranelift
            // Enabled by setting the LPP_AOT environment variable.
            if env::var("LPP_AOT").is_ok() {
                let aot_start = Instant::now();
                match cranelift_backend::compiler::AotCompiler::compile(&mir_program) {
                    Ok(obj_bytes) => {
                        let obj_path = filename.replace(".lpp", ".o");
                        if let Err(e) = fs::write(&obj_path, &obj_bytes) {
                            eprintln!("Failed to write {}: {}", obj_path, e);
                        } else if env::var("BENCHMARK").is_err() {
                            println!("[L++] AOT object file written to {}", obj_path);
                        }
                    }
                    Err(e) => eprintln!("[L++] AOT error: {}", e),
                }
                if env::var("BENCHMARK").is_err() {
                    println!("AOT: {:?}", aot_start.elapsed());
                }
            }

            let mut cg = codegen::Codegen::new(&resolver.table, &type_table, &storage);
            let c_code = cg.generate(&ast);
            if let Err(e) = fs::write("output.c", c_code) {
                eprintln!("Failed to write output.c: {}", e);
            }
        },
        Err(e) => {
            eprintln!("Escape Analysis error: {}", e);
            return;
        }
    }
    let esc_time = esc_start.elapsed();
    
    let total_time = total_start.elapsed();

    if env::var("BENCHMARK").is_ok() {
        println!("TIMING_JSON: {{\"io\": {}, \"lex\": {}, \"parse\": {}, \"semantic\": {}, \"typecheck\": {}, \"escape\": {}, \"total\": {}}}", 
           io_time.as_secs_f64(), lex_time.as_secs_f64(), parse_time.as_secs_f64(), sem_time.as_secs_f64(), ty_time.as_secs_f64(), esc_time.as_secs_f64(), total_time.as_secs_f64());
    } else {
        println!("--- L++ Compilation Successful ---");
        println!("Lex: {:?}", lex_time);
        println!("Parse: {:?}", parse_time);
        println!("Semantic: {:?}", sem_time);
        println!("Typecheck: {:?}", ty_time);
        println!("Escape: {:?}", esc_time);
        println!("MIR: {:?}", mir_time);
        println!("Total: {:?}", total_time);
    }
}
