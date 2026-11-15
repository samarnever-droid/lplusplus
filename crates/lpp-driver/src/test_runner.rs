//! Native & MIR test discovery runner for `lpp test <file_or_dir>`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use lpp_common::{FileId, SourceMap};
use lpp_frontend::{ItemKind, parse_source};
use lpp_hir::Symbol;
use lpp_mir::{InterpreterLimits, execute_mir_with_stats};

use crate::compile::{
    BackendChoice, CompileOptions, build_executable_with_options, default_runtime_lib_dir,
};
use crate::frontend_eval::check_in_memory;

#[derive(Debug, Clone)]
struct DiscoveredTest {
    fn_name: String,
    returns_bool: bool,
}

fn collect_test_files(path: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if path.is_file() {
        if path.extension().and_then(|e| e.to_str()) == Some("lpp") {
            out.push(path.to_path_buf());
        }
        return Ok(());
    }
    if path.is_dir() {
        let mut entries: Vec<PathBuf> = fs::read_dir(path)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .collect();
        entries.sort();
        for entry in entries {
            let name = entry.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(name, ".git" | "target" | ".lpp_packages") {
                continue;
            }
            collect_test_files(&entry, out)?;
        }
    }
    Ok(())
}

fn discover_tests(source: &str) -> Result<Vec<DiscoveredTest>, String> {
    let mut sources = SourceMap::new();
    let fid = sources
        .add_file("test.lpp", source.to_owned())
        .unwrap_or(FileId::from_raw(1));
    let parsed = parse_source(fid, source).map_err(|diags| {
        diags
            .into_iter()
            .map(|d| d.message)
            .collect::<Vec<_>>()
            .join("; ")
    })?;

    let mut tests = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    for item in &parsed.items {
        if item.kind != ItemKind::Function {
            continue;
        }
        let Some(name) = item.name.as_deref() else {
            continue;
        };
        if name == "main" {
            continue;
        }
        let line_idx = sources
            .get(fid)
            .and_then(|sf| sf.line_column(item.span.start).ok())
            .map(|(l, _)| l.saturating_sub(1))
            .unwrap_or(0);
        let has_test_attr = item.attributes.iter().any(|a| a.name == "test")
            || (line_idx > 0 && lines.get(line_idx - 1).is_some_and(|l| l.trim() == "@test"))
            || lines.get(line_idx).is_some_and(|l| l.trim() == "@test");
        let is_test_name = name.starts_with("test_");
        if !(has_test_attr || is_test_name) {
            continue;
        }
        let def_line = (line_idx..=(line_idx + 2).min(lines.len().saturating_sub(1)))
            .find_map(|i| {
                lines
                    .get(i)
                    .copied()
                    .filter(|l| l.trim_start().starts_with("def "))
            })
            .unwrap_or("");
        let returns_bool = def_line.contains("-> Bool");
        if def_line.contains("()") {
            tests.push(DiscoveredTest {
                fn_name: name.to_owned(),
                returns_bool,
            });
        }
    }
    Ok(tests)
}

fn synthesize_harness(source: &str, test: &DiscoveredTest) -> String {
    let mut stripped = String::with_capacity(source.len() + 128);
    let mut skip_main = false;
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed == "@test" {
            continue;
        }
        if trimmed.starts_with("def main(") {
            skip_main = true;
            continue;
        }
        if skip_main {
            if !trimmed.is_empty() && !line.starts_with([' ', '\t']) {
                skip_main = false;
            } else {
                continue;
            }
        }
        stripped.push_str(line);
        stripped.push('\n');
    }
    stripped.push_str("\ndef main():\n");
    if test.returns_bool {
        stripped.push_str(&format!(
            "    if !{}():\n        print_str(\"__LPP_TEST_FAIL__\")\n        exit(1)\n",
            test.fn_name
        ));
    } else {
        stripped.push_str(&format!("    {}()\n", test.fn_name));
    }
    stripped
}

/// CLI entry point for standalone `lpp test <file_or_dir>`.
pub fn run_tests_cli(args: &[String], cwd: &Path) -> i32 {
    let mut filter: Option<String> = None;
    let mut mir_only = false;
    let mut backend = BackendChoice::Cranelift;
    let mut paths: Vec<PathBuf> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--filter" | "-k" => {
                let Some(val) = args.get(i + 1) else {
                    eprintln!("test: missing value for --filter");
                    return 2;
                };
                filter = Some(val.clone());
                i += 2;
            }
            "--mir" | "--interpret" => {
                mir_only = true;
                i += 1;
            }
            "--llvm" => {
                backend = BackendChoice::Llvm;
                i += 1;
            }
            "--help" | "-h" => {
                println!("Usage: lpp test [--filter <pat>] [--mir] [--llvm] <file_or_dir>...");
                return 0;
            }
            other if other.starts_with('-') => {
                eprintln!("test: unknown option `{other}`");
                return 2;
            }
            p => {
                let path = Path::new(p);
                paths.push(if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    cwd.join(path)
                });
                i += 1;
            }
        }
    }

    let mut files = Vec::new();
    for p in &paths {
        if !p.exists() {
            eprintln!("test: path not found: {}", p.display());
            return 2;
        }
        if let Err(err) = collect_test_files(p, &mut files) {
            eprintln!("test: error reading {}: {err}", p.display());
            return 1;
        }
    }

    if files.is_empty() {
        eprintln!("test: no `.lpp` test files found");
        return 1;
    }

    let tmp_root = std::env::temp_dir().join(format!("lpp-test-runner-{}", std::process::id()));
    let _ = fs::create_dir_all(&tmp_root);

    let mut passed = 0usize;
    let mut failed = 0usize;

    for file in &files {
        let Ok(src) = fs::read_to_string(file) else {
            eprintln!("test: failed reading {}", file.display());
            failed += 1;
            continue;
        };
        let discovered = match discover_tests(&src) {
            Ok(list) => list,
            Err(msg) => {
                eprintln!("[FAIL] {} (parse error: {msg})", file.display());
                failed += 1;
                continue;
            }
        };
        for test in discovered {
            if let Some(pat) = &filter
                && !test.fn_name.contains(pat.as_str())
            {
                continue;
            }
            let started = Instant::now();
            let harness = synthesize_harness(&src, &test);
            let test_label = format!("{}::{}", file.display(), test.fn_name);
            let ok = if mir_only {
                let virtual_path = file.with_file_name(format!("__test_{}.lpp", test.fn_name));
                match check_in_memory(&harness, &virtual_path) {
                    Ok(unit) => {
                        let main_id = unit.mir.functions().find_map(|(id, f)| {
                            (f.name.and_then(|s| {
                                unit.package
                                    .names
                                    .symbols
                                    .resolve(Symbol::from_raw(s.raw()))
                            }) == Some("main"))
                            .then_some(id)
                        });
                        match main_id {
                            Some(id) => execute_mir_with_stats(
                                &unit.mir,
                                &unit.inference.interner,
                                id,
                                &[],
                                InterpreterLimits::default(),
                            )
                            .is_ok_and(|out| {
                                !out.output.iter().any(|l| l.contains("__LPP_TEST_FAIL__"))
                            }),
                            None => false,
                        }
                    }
                    Err(diags) => {
                        for d in diags {
                            eprintln!("  [{}] {}", d.code, d.message);
                        }
                        false
                    }
                }
            } else {
                let case_src =
                    tmp_root.join(format!("case_{}_{}.lpp", passed + failed, test.fn_name));
                let case_exe = tmp_root.join(format!(
                    "case_{}_{}{}",
                    passed + failed,
                    test.fn_name,
                    std::env::consts::EXE_SUFFIX
                ));
                if fs::write(&case_src, &harness).is_err() {
                    false
                } else {
                    let runtime_dir = default_runtime_lib_dir();
                    match build_executable_with_options(
                        &case_src,
                        "lpp_test",
                        backend,
                        CompileOptions::for_backend(backend),
                        &case_exe,
                        &runtime_dir,
                    ) {
                        Ok(()) => match crate::compile::exec_output_retry(&case_exe) {
                            Ok(out) if out.status.success() => true,
                            Ok(out) => {
                                let stdout = String::from_utf8_lossy(&out.stdout);
                                let stderr = String::from_utf8_lossy(&out.stderr);
                                if !stdout.trim().is_empty() {
                                    eprintln!("  stdout: {}", stdout.trim());
                                }
                                if !stderr.trim().is_empty() {
                                    eprintln!("  stderr: {}", stderr.trim());
                                }
                                false
                            }
                            Err(err) => {
                                eprintln!("  exec error: {err}");
                                false
                            }
                        },
                        Err(err) => {
                            eprintln!("  compile error: {err}");
                            false
                        }
                    }
                }
            };
            let elapsed_ms = started.elapsed().as_millis();
            if ok {
                println!("[PASS] {test_label} ({elapsed_ms}ms)");
                passed += 1;
            } else {
                println!("[FAIL] {test_label} ({elapsed_ms}ms)");
                failed += 1;
            }
        }
    }

    let _ = fs::remove_dir_all(&tmp_root);
    println!(
        "test result: {}. {passed} passed; {failed} failed",
        if failed == 0 { "ok" } else { "FAILED" }
    );
    if failed == 0 { 0 } else { 1 }
}
