//! `keel test` — run the project's L++ test files.
//!
//! Convention: each `tests/*.lpp` file is a standalone program. It **passes**
//! when it compiles AND runs to exit code 0 — a failing test asserts by
//! exiting non-zero (e.g. `exit(1)`) or by failing to compile.

use std::path::{Path, PathBuf};
use std::time::Instant;

use tabled::builder::Builder;

/// Test files for one directory: `tests/*.lpp`, sorted by file name.
fn discover_one(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let tests_dir = dir.join("tests");
    if !tests_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(&tests_dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "lpp"))
        .collect();
    files.sort();
    Ok(files)
}

/// The workspace's test files: `tests/*.lpp` of every member (monorepo),
/// in member order then file order; a standalone project behaves as before.
fn discover(ws: &lpp_pm::Workspace, dir: &Path) -> Result<Vec<PathBuf>, String> {
    if ws.members.is_empty() {
        return discover_one(dir);
    }
    let mut files = Vec::new();
    for m in &ws.members {
        files.extend(discover_one(&m.dir)?);
    }
    Ok(files)
}

/// Run every `tests/*.lpp` in the project with `lpp <file> --run`.
///
/// Prints a table (test / result / time). Returns `Ok(())` when every test
/// passes (or there are none); otherwise `Err` naming the failures. Failing
/// tests also print their lpp output to stderr for debugging.
///
/// Deps (path + registry) are staged first, and `lpp` runs with cwd =
/// workspace root, so `.lpp_packages` and the shared `target/` resolve in
/// a monorepo too.
pub fn test_run(dir: &Path, lpp_bin: &str, reg: Option<&lpp_pm::Registry>) -> Result<(), String> {
    let ws = lpp_pm::Workspace::discover(dir).map_err(|e| e.to_string())?;
    crate::commands::build::stage_all_deps(&ws, reg)?;
    let files = discover(&ws, dir)?;
    if files.is_empty() {
        println!("no tests found (add .lpp files under tests/)");
        return Ok(());
    }

    let mut b = Builder::default();
    b.push_record(["test".to_string(), "result".to_string(), "time (ms)".to_string()]);
    let mut passed = 0usize;
    let mut failed: Vec<String> = Vec::new();

    for file in &files {
        // Name the test relative to the workspace root (disambiguates
        // same-named test files in different members).
        let name = match file.strip_prefix(&ws.root) {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(_) => file
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| file.display().to_string()),
        };
        let started = Instant::now();
        let out = std::process::Command::new(lpp_bin)
            .arg(file)
            .arg("--run")
            .current_dir(&ws.root)
            .output()
            .map_err(|e| format!("failed to run lpp ({lpp_bin}): {e}"))?;
        let ms = started.elapsed().as_millis() as u64;

        if out.status.success() {
            passed += 1;
            b.push_record([name, "PASS".to_string(), ms.to_string()]);
        } else {
            failed.push(name.clone());
            b.push_record([
                name.clone(),
                format!("FAIL ({})", out.status.code().unwrap_or(-1)),
                ms.to_string(),
            ]);
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            if !stdout.trim().is_empty() || !stderr.trim().is_empty() {
                eprintln!("--- {name} output ---");
                if !stdout.trim().is_empty() {
                    eprintln!("{stdout}");
                }
                if !stderr.trim().is_empty() {
                    eprintln!("{stderr}");
                }
                eprintln!("---------------------");
            }
        }
    }

    println!("{}", b.build());
    if failed.is_empty() {
        println!("test OK ({passed} passed, {passed} tests)");
        Ok(())
    } else {
        Err(format!(
            "{} passed, {} failed: {}",
            passed,
            failed.len(),
            failed.join(", ")
        ))
    }
}
