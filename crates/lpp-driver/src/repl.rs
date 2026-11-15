//! Interactive & piped L++ REPL (`lpp repl`) backed by the full typed MIR pipeline.

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::Path;

use lpp_common::{FileId, SourceMap};
use lpp_frontend::parse_source;
use lpp_hir::Symbol;
use lpp_mir::{InterpreterLimits, execute_mir_with_stats};
use lpp_types::{PrimitiveType, TypeKind};

use crate::frontend_eval::check_in_memory;

#[derive(Default)]
struct ReplState {
    decls: Vec<String>,
    stmts: Vec<String>,
    printed_count: usize,
}

impl ReplState {
    fn render(
        &self,
        extra_decl: Option<&str>,
        extra_stmt: Option<&str>,
        eval_expr: Option<&str>,
    ) -> String {
        let mut src = String::new();
        for d in &self.decls {
            src.push_str(d);
            src.push('\n');
        }
        if let Some(d) = extra_decl {
            src.push_str(d);
            src.push('\n');
        }
        src.push_str("def main():\n");
        let has_body = !self.stmts.is_empty() || extra_stmt.is_some() || eval_expr.is_some();
        if !has_body {
            src.push_str("    return\n");
            return src;
        }
        for s in &self.stmts {
            push_indented(&mut src, s);
        }
        if let Some(s) = extra_stmt {
            push_indented(&mut src, s);
        }
        if let Some(expr) = eval_expr {
            push_indented(&mut src, expr);
        }
        src
    }
}

fn push_indented(dst: &mut String, block: &str) {
    for line in block.lines() {
        if line.trim().is_empty() {
            dst.push('\n');
        } else {
            dst.push_str("    ");
            dst.push_str(line);
            dst.push('\n');
        }
    }
}

fn is_top_level_decl(trimmed: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "def ",
        "async def ",
        "unsafe def ",
        "struct ",
        "enum ",
        "trait ",
        "impl ",
        "type ",
        "import ",
        "from ",
        "extern ",
    ];
    PREFIXES.iter().any(|p| trimmed.starts_with(p))
}

fn format_print_call(kind: TypeKind, expr: &str) -> Option<String> {
    match kind {
        TypeKind::Primitive(
            PrimitiveType::Int
            | PrimitiveType::I8
            | PrimitiveType::U8
            | PrimitiveType::I16
            | PrimitiveType::U16
            | PrimitiveType::I32
            | PrimitiveType::U32
            | PrimitiveType::U64
            | PrimitiveType::ISize
            | PrimitiveType::USize,
        ) => Some(format!("print({expr})")),
        TypeKind::Primitive(PrimitiveType::Float) => Some(format!("print_float({expr})")),
        TypeKind::Primitive(PrimitiveType::Bool) => Some(format!("print_bool({expr})")),
        TypeKind::Primitive(PrimitiveType::String) => Some(format!("print_str({expr})")),
        _ => None,
    }
}

fn is_plain_expression(expr: &str) -> bool {
    let src = format!("def __probe():\n    __v := {expr}\n");
    let mut sources = SourceMap::new();
    let fid = sources
        .add_file("probe.lpp", src.clone())
        .unwrap_or(FileId::from_raw(1));
    parse_source(fid, &src).is_ok()
}

fn find_main_id(unit: &crate::frontend_eval::CheckedUnit) -> Option<lpp_mir::MirFunctionId> {
    unit.mir
        .functions()
        .find(|(_, func)| {
            func.name.is_some_and(|s| {
                unit.package
                    .names
                    .symbols
                    .resolve(Symbol::from_raw(s.raw()))
                    == Some("main")
            })
        })
        .map(|(id, _)| id)
}

fn print_new_output(state: &mut ReplState, output: &[String], record_stmt: bool) {
    if output.len() > state.printed_count {
        for line in &output[state.printed_count..] {
            println!("{line}");
        }
        if record_stmt {
            state.printed_count = output.len();
        }
    }
}

fn eval_repl_input(state: &mut ReplState, input: &str, virtual_path: &Path) {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return;
    }

    if is_top_level_decl(trimmed) {
        let candidate = state.render(Some(input), None, None);
        match check_in_memory(&candidate, virtual_path) {
            Ok(_) => {
                state.decls.push(input.to_owned());
            }
            Err(diags) => {
                for d in diags {
                    eprintln!("error [{}]: {}", d.code, d.message);
                }
            }
        }
        return;
    }

    // First, try interpreting as a standalone expression if valid expression syntax
    // (and not an assignment/binding/control statement).
    let looks_like_expr = !trimmed.starts_with("return ")
        && !trimmed.starts_with("if ")
        && !trimmed.starts_with("for ")
        && !trimmed.starts_with("while ")
        && !trimmed.starts_with("match ")
        && !trimmed.starts_with("mut ")
        && !trimmed.contains(":=")
        && is_plain_expression(trimmed);

    if looks_like_expr {
        let probe_src = state.render(None, Some(&format!("__repl_val := {trimmed}")), None);
        if let Ok(checked) = check_in_memory(&probe_src, virtual_path)
            && let Some(main_id) = find_main_id(&checked)
        {
            let val_kind = checked
                .package
                .locals
                .enumerate()
                .filter(|(_, l)| {
                    checked.package.names.symbols.resolve(l.name) == Some("__repl_val")
                })
                .last()
                .and_then(|(lid, _)| checked.inference.assignments.local(lid))
                .map(|ty| checked.inference.interner.kind(ty));

            if let Some(kind) = val_kind
                && let Some(print_stmt) = format_print_call(kind, trimmed)
            {
                let run_src = state.render(None, None, Some(&print_stmt));
                if let Ok(run_checked) = check_in_memory(&run_src, virtual_path)
                    && let Some(run_main) = find_main_id(&run_checked)
                {
                    match execute_mir_with_stats(
                        &run_checked.mir,
                        &run_checked.inference.interner,
                        run_main,
                        &[],
                        InterpreterLimits::default(),
                    ) {
                        Ok(outcome) => {
                            print_new_output(state, &outcome.output, false);
                            return;
                        }
                        Err(err) => {
                            eprintln!("runtime error: {err}");
                            return;
                        }
                    }
                }
            }

            // Void or non-scalar expression (e.g. `print_str("hi")`): execute once and keep no dead expr if void.
            match execute_mir_with_stats(
                &checked.mir,
                &checked.inference.interner,
                main_id,
                &[],
                InterpreterLimits::default(),
            ) {
                Ok(outcome) => {
                    print_new_output(state, &outcome.output, false);
                    return;
                }
                Err(err) => {
                    eprintln!("runtime error: {err}");
                    return;
                }
            }
        }
    }

    // Statement path (`x := ...`, `for ...`, `if ...`, calls, mutations)
    let stmt_src = state.render(None, Some(input), None);
    match check_in_memory(&stmt_src, virtual_path) {
        Ok(checked) => {
            if let Some(main_id) = find_main_id(&checked) {
                match execute_mir_with_stats(
                    &checked.mir,
                    &checked.inference.interner,
                    main_id,
                    &[],
                    InterpreterLimits::default(),
                ) {
                    Ok(outcome) => {
                        state.stmts.push(input.to_owned());
                        print_new_output(state, &outcome.output, true);
                    }
                    Err(err) => eprintln!("runtime error: {err}"),
                }
            }
        }
        Err(diags) => {
            for d in diags {
                eprintln!("error [{}]: {}", d.code, d.message);
            }
        }
    }
}

/// CLI entry point for `lpp repl`.
pub fn run_repl(args: &[String], cwd: &Path) -> i32 {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("Usage: lpp repl");
        println!("Interactive L++ REPL commands: :help, :show, :clear, :quit");
        return 0;
    }

    let interactive = io::stdin().is_terminal();
    let virtual_path = cwd.join("__repl__.lpp");
    if interactive {
        println!(
            "L++ v{} REPL (type :help for commands, :quit to exit)",
            env!("CARGO_PKG_VERSION")
        );
    }

    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let mut state = ReplState::default();
    let mut block_buf = String::new();
    let mut in_block = false;

    loop {
        if interactive {
            print!("{}", if in_block { "... " } else { "lpp> " });
            let _ = io::stdout().flush();
        }
        let Some(Ok(line)) = lines.next() else {
            if !block_buf.trim().is_empty() {
                eval_repl_input(&mut state, &block_buf, &virtual_path);
            }
            break;
        };

        let trimmed = line.trim();
        if !in_block {
            match trimmed {
                ":quit" | ":exit" | ":q" => break,
                ":help" | ":h" => {
                    println!("Commands: :help, :show, :clear, :quit");
                    continue;
                }
                ":clear" | ":reset" => {
                    state = ReplState::default();
                    continue;
                }
                ":show" => {
                    println!("{}", state.render(None, None, None));
                    continue;
                }
                "" => continue,
                _ => {}
            }
            if trimmed.ends_with(':') {
                in_block = true;
                block_buf.clear();
                block_buf.push_str(&line);
                block_buf.push('\n');
                continue;
            }
            eval_repl_input(&mut state, &line, &virtual_path);
        } else if trimmed.is_empty() {
            in_block = false;
            eval_repl_input(&mut state, &block_buf, &virtual_path);
            block_buf.clear();
        } else if !line.starts_with([' ', '\t'])
            && !trimmed.starts_with("elif")
            && !trimmed.starts_with("else")
        {
            in_block = false;
            eval_repl_input(&mut state, &block_buf, &virtual_path);
            block_buf.clear();
            if trimmed == ":quit" || trimmed == ":exit" {
                break;
            }
            eval_repl_input(&mut state, &line, &virtual_path);
        } else {
            block_buf.push_str(&line);
            block_buf.push('\n');
        }
    }
    0
}
