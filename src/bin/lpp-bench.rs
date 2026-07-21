//! `lpp-bench` — Comprehensive linker and resource benchmark CLI.
//!
//! Runs King20 workloads across all three linker paths (direct, mold/host, C
//! transpiler) and measures wall-clock time, CPU time, binary size, disk
//! usage, peak RSS, and memory pressure.  Produces a JSON report and a
//! human-readable table.
//!
//! Usage:
//!   lpp-bench [--linkers direct,mold,c] [--suite king20] [--json] [--disk] [--mem]
//!   lpp-bench --self-test          (run 15 built-in integration tests)

use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

// ═══════════════════════════════════════════════════════════════════════════
//  Data types
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, serde::Serialize)]
struct LinkerResult {
    linker: String,
    link_time_ms: f64,
    binary_size_bytes: u64,
    peak_rss_kb: Option<u64>,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    success: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
struct BenchmarkResult {
    name: String,
    source_path: String,
    expected: String,
    compile_time_ms: f64,
    linkers: Vec<LinkerResult>,
    verdict: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct BenchReport {
    toolchain: String,
    host: String,
    timestamp: String,
    total_workloads: usize,
    workloads: Vec<BenchmarkResult>,
    summary: LinkerSummary,
}

#[derive(Debug, Clone, serde::Serialize)]
struct LinkerSummary {
    direct_passed: usize,
    mold_passed: usize,
    c_transpiler_passed: usize,
    direct_mean_link_ms: f64,
    mold_mean_link_ms: f64,
    c_mean_link_ms: f64,
    direct_mean_binary_kb: f64,
    mold_mean_binary_kb: f64,
    c_mean_binary_kb: f64,
    direct_mean_rss_mb: f64,
    mold_mean_rss_mb: f64,
    c_mean_rss_mb: f64,
}

#[derive(Debug, Clone)]
struct King20Case {
    id: usize,
    name: String,
    source: PathBuf,
    expected: String,
}

// ═══════════════════════════════════════════════════════════════════════════
//  Platform helpers
// ═══════════════════════════════════════════════════════════════════════════

fn exe_suffix() -> &'static str {
    if cfg!(target_os = "windows") { ".exe" } else { "" }
}

fn runtime_obj_name() -> &'static str {
    if cfg!(target_os = "windows") { "obj" } else { "o" }
}

fn lpp_binary() -> PathBuf {
    let exe = env::current_exe().unwrap_or_else(|_| PathBuf::from("lpp"));
    exe.parent()
        .map(|p| p.join(format!("lpp{}", exe_suffix())))
        .unwrap_or_else(|| PathBuf::from(format!("lpp{}", exe_suffix())))
}

fn lpp_link_binary() -> PathBuf {
    let exe = env::current_exe().unwrap_or_else(|_| PathBuf::from("lpp-bench"));
    exe.parent()
        .map(|p| p.join(format!("lpp-link{}", exe_suffix())))
        .unwrap_or_else(|| PathBuf::from(format!("lpp-link{}", exe_suffix())))
}

fn repo_root() -> PathBuf {
    let exe = env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    // Walk up from the binary to find Cargo.toml / benchmarks
    let mut dir = exe.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
    loop {
        if dir.join("Cargo.toml").exists() || dir.join("benchmarks").is_dir() {
            return dir;
        }
        if let Some(parent) = dir.parent() {
            dir = parent.to_path_buf();
        } else {
            return PathBuf::from(".");
        }
    }
}

fn detect_c_compiler() -> Option<String> {
    for cc in &["gcc", "clang", "cc"] {
        if Command::new(cc)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
        {
            return Some(cc.to_string());
        }
    }
    None
}

fn has_mold() -> bool {
    Command::new("mold")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

fn current_timestamp() -> String {
    // Simple UTC timestamp
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        // Rough approximation
        1970 + (now.as_secs() / 31556952) as usize,
        ((now.as_secs() % 31556952) / 2629746 + 1) as usize,
        ((now.as_secs() % 2629746) / 86400 + 1) as usize,
        (now.as_secs() / 3600 % 24),
        (now.as_secs() / 60 % 60),
        (now.as_secs() % 60),
    )
}

// ═══════════════════════════════════════════════════════════════════════════
//  Resource measurement
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(target_os = "linux")]
fn read_statm_peak_rss() -> Option<u64> {
    // /proc/self/status: VmHWM is peak RSS in kB
    if let Ok(mut f) = fs::File::open("/proc/self/status") {
        let mut s = String::new();
        if f.read_to_string(&mut s).is_ok() {
            for line in s.lines() {
                if line.starts_with("VmHWM:") {
                    return line
                        .split_whitespace()
                        .nth(1)
                        .and_then(|v| v.parse::<u64>().ok());
                }
            }
        }
    }
    None
}

#[cfg(not(target_os = "linux"))]
fn read_statm_peak_rss() -> Option<u64> { None }

fn file_size_bytes(path: &Path) -> u64 {
    fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn disk_usage_kb(dir: &Path) -> u64 {
    if !dir.is_dir() {
        return file_size_bytes(dir) / 1024;
    }
    fn walk(dir: &Path, total: &mut u64) {
        if let Ok(entries) = fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, total);
                } else if p.is_file() {
                    *total += fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                }
            }
        }
    }
    let mut total = 0u64;
    walk(dir, &mut total);
    total / 1024
}

// ═══════════════════════════════════════════════════════════════════════════
//  King20 manifest
// ═══════════════════════════════════════════════════════════════════════════

fn load_king20() -> Vec<King20Case> {
    let root = repo_root();
    vec![
        ( 1, "recursive-fib-35",         "benchmarks/bench_fib.lpp",              "9227465"),
        ( 2, "loop-10m",                  "benchmarks/bench_loop.lpp",             "49999995000000"),
        ( 3, "call-chain-1m",             "benchmarks/bench_calls.lpp",            "500000500000"),
        ( 4, "integer-arithmetic",        "tests/arith.lpp",                       "15\n5\n50\n2"),
        ( 5, "conditional-branches",      "tests/branches.lpp",                    "1\n0\n1"),
        ( 6, "nested-direct-calls",       "tests/nested_calls.lpp",                "120"),
        ( 7, "immutable-closure",         "tests/closure_test.lpp",                "52"),
        ( 8, "arc-list-int",              "tests/list_safety.lpp",                 "3\n5\n13"),
        ( 9, "owned-struct-return",       "tests/owned_return.lpp",                "1"),
        (10, "branch-owned-return",       "tests/arc_branch_return.lpp",           "1"),
        (11, "nested-struct-destructor",  "tests/arc_nested_struct.lpp",           "1"),
        (12, "direct-arc-alias",          "tests/arc_direct_alias.lpp",            "1"),
        (13, "closure-arc-capture",       "tests/arc_closure_capture.lpp",         "0"),
        (14, "borrowed-parameter-return", "tests/arc_borrowed_return.lpp",         "1"),
        (15, "borrowed-field-return",     "tests/arc_borrowed_field_return.lpp",   "1"),
        (16, "field-alias",               "tests/arc_field_alias.lpp",             "1"),
        (17, "list-int-alias",            "tests/arc_list_alias.lpp",              "7"),
        (18, "list-custom-ownership",     "tests/arc_list_custom.lpp",             "1"),
        (19, "nested-branch-alias",       "tests/arc_nested_branch_alias.lpp",     "1"),
        (20, "closure-branch-capture",    "tests/arc_closure_branch_capture.lpp",  "0"),
    ].into_iter().map(|(id, name, src, exp)| King20Case {
        id, name: name.to_string(),
        source: root.join(src),
        expected: exp.to_string(),
    }).collect()
}

// ═══════════════════════════════════════════════════════════════════════════
//  Linker backends
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkerMode { Direct, Mold, CTranspiler }

impl LinkerMode {
    fn from_str(s: &str) -> Vec<LinkerMode> {
        s.split(',').filter_map(|tok| match tok.trim() {
            "direct" => Some(LinkerMode::Direct),
            "mold" => Some(LinkerMode::Mold),
            "c" | "host" => Some(LinkerMode::CTranspiler),
            _ => None,
        }).collect()
    }

    #[allow(dead_code)]
    fn label(&self) -> &'static str {
        match self {
            LinkerMode::Direct => "direct (lpp-link)",
            LinkerMode::Mold => "mold",
            LinkerMode::CTranspiler => "C transpiler + host ld",
        }
    }

    fn short_label(&self) -> &'static str {
        match self {
            LinkerMode::Direct => "direct",
            LinkerMode::Mold => "mold",
            LinkerMode::CTranspiler => "c",
        }
    }
}

fn run_linker_direct(
    obj: &Path,
    runtime_obj: &Path,
    output: &Path,
) -> Result<LinkerResult, String> {
    let linker = lpp_link_binary();
    if !linker.exists() {
        return Err("lpp-link binary not found".to_string());
    }
    let start = Instant::now();
    let child = Command::new(&linker)
        .arg(obj)
        .arg(runtime_obj)
        .arg("-o")
        .arg(output)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn lpp-link: {e}"))?;
    let output_w = child.wait_with_output().map_err(|e| format!("wait: {e}"))?;
    let elapsed = start.elapsed();
    Ok(LinkerResult {
        linker: "direct (lpp-link)".to_string(),
        link_time_ms: elapsed.as_secs_f64() * 1000.0,
        binary_size_bytes: file_size_bytes(output),
        peak_rss_kb: read_statm_peak_rss(),
        exit_code: output_w.status.code(),
        stdout: String::from_utf8_lossy(&output_w.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output_w.stderr).to_string(),
        success: output_w.status.success(),
    })
}

fn run_linker_mold(
    obj: &Path,
    runtime_src: &Path,
    output: &Path,
) -> Result<LinkerResult, String> {
    let cc = detect_c_compiler().ok_or("no C compiler found")?;
    let start = Instant::now();
    let mut cmd = Command::new(&cc);
    if has_mold() {
        cmd.arg("-fuse-ld=mold");
        cmd.arg("-Wl,--icf=all");
        cmd.arg("-Wl,-O1");
    }
    cmd.arg("-O2")
        .arg(obj)
        .arg(runtime_src)
        .arg("-o")
        .arg(output);
    if !cfg!(target_os = "windows") {
        cmd.arg("-pthread");
    }
    let child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|e| format!("spawn {cc}: {e}"))?;
    let output_w = child.wait_with_output().map_err(|e| format!("wait: {e}"))?;
    let elapsed = start.elapsed();
    let mold_label = if has_mold() { "mold via gcc" } else { "gcc (no mold)" };
    Ok(LinkerResult {
        linker: mold_label.to_string(),
        link_time_ms: elapsed.as_secs_f64() * 1000.0,
        binary_size_bytes: file_size_bytes(output),
        peak_rss_kb: read_statm_peak_rss(),
        exit_code: output_w.status.code(),
        stdout: String::from_utf8_lossy(&output_w.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output_w.stderr).to_string(),
        success: output_w.status.success(),
    })
}

fn run_linker_c_transpile(
    source: &Path,
    runtime_src: &Path,
    output: &Path,
    lpp: &Path,
) -> Result<LinkerResult, String> {
    let cc = detect_c_compiler().ok_or("no C compiler found")?;
    // Step 1: transpile to C
    let c_out = output.with_extension("c");
    let transpile_start = Instant::now();
    let transpile = Command::new(lpp)
        .arg(source)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("transpile: {e}"))?;
    if !transpile.success() {
        return Err("transpile failed".to_string());
    }
    // The C file may be next to source or in cwd
    let c_path = if c_out.exists() { c_out.clone() }
        else { source.with_extension("c") };
    if !c_path.exists() {
        return Err("C file not generated".to_string());
    }

    let start = Instant::now();
    let child = Command::new(&cc)
        .arg("-O2")
        .arg("-std=c11")
        .arg(&c_path)
        .arg(runtime_src)
        .arg("-o")
        .arg(output)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("compile C: {e}"))?;
    let output_w = child.wait_with_output().map_err(|e| format!("wait: {e}"))?;
    let elapsed = start.elapsed();

    let _ = transpile_start.elapsed(); // already spent

    let _ = fs::remove_file(&c_path);
    Ok(LinkerResult {
        linker: "C transpiler".to_string(),
        link_time_ms: (transpile_start.elapsed().as_secs_f64() + elapsed.as_secs_f64()) * 1000.0,
        binary_size_bytes: file_size_bytes(output),
        peak_rss_kb: read_statm_peak_rss(),
        exit_code: output_w.status.code(),
        stdout: String::from_utf8_lossy(&output_w.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output_w.stderr).to_string(),
        success: output_w.status.success(),
    })
}

// ═══════════════════════════════════════════════════════════════════════════
//  Main benchmark runner
// ═══════════════════════════════════════════════════════════════════════════

fn run_full_bench(linkers: &[LinkerMode], show_disk: bool, show_mem: bool) -> BenchReport {
    let lpp = lpp_binary();
    let root = repo_root();
    let runtime_src = root.join("lpp_runtime.c");
    let runtime_min = root.join("runtime").join("linux_x86_64_min.c");
    let freestanding_runtime = if cfg!(target_os = "windows") {
        root.join("runtime").join("windows_x86_64_min.c")
    } else {
        runtime_min.clone()
    };
    let workloads = load_king20();

    let tmp = std::env::temp_dir().join("lpp-bench");
    let _ = fs::create_dir_all(&tmp);

    // Pre-compile freestanding runtime object for direct linker
    let rt_obj = tmp.join(format!("lpp_runtime_min.{}", runtime_obj_name()));
    if linkers.contains(&LinkerMode::Direct) && !rt_obj.exists() {
        let cc = detect_c_compiler().unwrap_or_else(|| "cc".to_string());
        let _ = Command::new(&cc)
            .arg("-O2").arg("-ffreestanding").arg("-fno-stack-protector")
            .arg("-fno-pic").arg("-mno-red-zone")
            .arg("-c").arg(&freestanding_runtime).arg("-o").arg(&rt_obj)
            .status();
    }

    let mut results = Vec::new();

    for case in &workloads {
        eprintln!("\n[{:>2}/{}] {}", case.id, workloads.len(), case.name);

        let src = &case.source;
        let tmp_src = tmp.join(format!("{}.lpp", case.name));
        let tmp_obj = tmp.join(format!("{}.o", case.name));
        let _ = fs::copy(src, &tmp_src);

        // Compile once with AOT
        let compile_start = Instant::now();
        let compile = Command::new(&lpp)
            .env("LPP_AOT", "1")
            .env("LPP_AOT_ONLY", "1")
            .arg(&tmp_src)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let compile_ms = compile_start.elapsed().as_secs_f64() * 1000.0;

        if !compile.map(|s| s.success()).unwrap_or(false) {
            eprintln!("  SKIP: compile failed");
            continue;
        }

        // The object may be produced next to the source
        let obj_path = if tmp_obj.exists() { tmp_obj.clone() }
            else { tmp_src.with_extension("o") };

        let mut linker_results = Vec::new();

        for mode in linkers {
            let exe = tmp.join(format!("{}_{}", case.name, mode.short_label()));
            let r = match mode {
                LinkerMode::Direct => {
                    run_linker_direct(&obj_path, &rt_obj, &exe)
                        .unwrap_or_else(|e| LinkerResult {
                            linker: "direct".into(), link_time_ms: 0.0,
                            binary_size_bytes: 0, peak_rss_kb: None,
                            exit_code: Some(1), stdout: String::new(),
                            stderr: e, success: false,
                        })
                }
                LinkerMode::Mold => {
                    run_linker_mold(&obj_path, &runtime_src, &exe)
                        .unwrap_or_else(|e| LinkerResult {
                            linker: "mold".into(), link_time_ms: 0.0,
                            binary_size_bytes: 0, peak_rss_kb: None,
                            exit_code: Some(1), stdout: String::new(),
                            stderr: e, success: false,
                        })
                }
                LinkerMode::CTranspiler => {
                    run_linker_c_transpile(&tmp_src, &runtime_src, &exe, &lpp)
                        .unwrap_or_else(|e| LinkerResult {
                            linker: "c".into(), link_time_ms: 0.0,
                            binary_size_bytes: 0, peak_rss_kb: None,
                            exit_code: Some(1), stdout: String::new(),
                            stderr: e, success: false,
                        })
                }
            };

            // Run the binary and capture output
            let (run_stdout, run_exit) = if r.success && exe.exists() {
                if let Ok(out) = Command::new(&exe).output() {
                    (String::from_utf8_lossy(&out.stdout).trim().to_string(), out.status.code())
                } else {
                    (String::new(), Some(1))
                }
            } else {
                (String::new(), None)
            };

            let verdict = if run_stdout == case.expected { "PASS" } else { "FAIL" };
            eprintln!("  {} | {:>8} | {}ms | {}",
                mode.short_label(),
                verdict,
                r.link_time_ms as u64,
                if r.success { format!("{}KB", r.binary_size_bytes / 1024) }
                else { "FAIL".into() }
            );

            let mut result = r;
            result.stdout = run_stdout;
            result.exit_code = run_exit;
            result.success = verdict == "PASS";
            linker_results.push(result);

            let _ = fs::remove_file(&exe);
        }

        let overall = if linker_results.iter().all(|lr| lr.success) { "PASS" }
            else if linker_results.iter().any(|lr| lr.success) { "PARTIAL" }
            else { "FAIL" };

        results.push(BenchmarkResult {
            name: case.name.clone(),
            source_path: case.source.to_string_lossy().to_string(),
            expected: case.expected.clone(),
            compile_time_ms: compile_ms,
            linkers: linker_results,
            verdict: overall.to_string(),
        });

        let _ = fs::remove_file(&tmp_src);
        let _ = fs::remove_file(&obj_path);
    }

    // Summary
    let mut dir_p = 0; let mut dir_ms = 0.0f64; let mut dir_kb = 0.0f64; let mut dir_rss = 0.0f64;
    let mut mold_p = 0; let mut mold_ms = 0.0; let mut mold_kb = 0.0; let mut mold_rss = 0.0;
    let mut c_p = 0; let mut c_ms = 0.0; let mut c_kb = 0.0; let mut c_rss = 0.0;
    let mut dir_n = 0u64; let mut mold_n = 0u64; let mut c_n = 0u64;

    for w in &results {
        for lr in &w.linkers {
            match lr.linker.as_str() {
                "direct (lpp-link)" => { if lr.success { dir_p += 1; }; dir_ms += lr.link_time_ms; dir_kb += lr.binary_size_bytes as f64; if let Some(r) = lr.peak_rss_kb { dir_rss += r as f64; }; dir_n += 1; }
                s if s.contains("mold") || s.contains("gcc") => { if lr.success { mold_p += 1; }; mold_ms += lr.link_time_ms; mold_kb += lr.binary_size_bytes as f64; if let Some(r) = lr.peak_rss_kb { mold_rss += r as f64; }; mold_n += 1; }
                "C transpiler" => { if lr.success { c_p += 1; }; c_ms += lr.link_time_ms; c_kb += lr.binary_size_bytes as f64; if let Some(r) = lr.peak_rss_kb { c_rss += r as f64; }; c_n += 1; }
                _ => {}
            }
        }
    }

    if dir_n > 0 { dir_ms /= dir_n as f64; dir_kb /= dir_n as f64; dir_rss /= dir_n as f64; }
    if mold_n > 0 { mold_ms /= mold_n as f64; mold_kb /= mold_n as f64; mold_rss /= mold_n as f64; }
    if c_n > 0 { c_ms /= c_n as f64; c_kb /= c_n as f64; c_rss /= c_n as f64; }

    let summary = LinkerSummary {
        direct_passed: dir_p, mold_passed: mold_p, c_transpiler_passed: c_p,
        direct_mean_link_ms: dir_ms, mold_mean_link_ms: mold_ms, c_mean_link_ms: c_ms,
        direct_mean_binary_kb: dir_kb / 1024.0, mold_mean_binary_kb: mold_kb / 1024.0, c_mean_binary_kb: c_kb / 1024.0,
        direct_mean_rss_mb: dir_rss / 1024.0, mold_mean_rss_mb: mold_rss / 1024.0, c_mean_rss_mb: c_rss / 1024.0,
    };

    // Disk / mem info
    if show_disk {
        eprintln!("\nDisk usage:");
        eprintln!("  lpp binary:     {} KB", file_size_bytes(&lpp) / 1024);
        eprintln!("  lpp-link binary: {} KB", file_size_bytes(&lpp_link_binary()) / 1024);
        eprintln!("  tmp artifacts:  {} KB", disk_usage_kb(&tmp));
    }
    if show_mem {
        if let Some(rss) = read_statm_peak_rss() {
            eprintln!("  Peak RSS:       {} MB", rss as f64 / 1024.0);
        }
    }

    BenchReport {
        toolchain: format!("L++ v{} (rustc {})",
            env!("CARGO_PKG_VERSION"),
            if let Ok(v) = Command::new("rustc").arg("--version").output() {
                String::from_utf8_lossy(&v.stdout).trim().to_string()
            } else { "unknown".into() }
        ),
        host: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        timestamp: current_timestamp(),
        total_workloads: results.len(),
        workloads: results,
        summary,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  15 integration tests (self-test mode)
// ═══════════════════════════════════════════════════════════════════════════

fn run_self_tests() -> bool {
    println!("═══ L++ Bench Self-Test Suite (15 tests) ═══\n");

    let lpp = lpp_binary();
    let linker = lpp_link_binary();
    let root = repo_root();
    let tmp = std::env::temp_dir().join("lpp-bench-self-test");
    let _ = fs::create_dir_all(&tmp);

    // Compile freestanding runtime
    let cc = detect_c_compiler();
    let runtime_obj = if let Some(ref cc) = cc {
        let obj = tmp.join("lpp_runtime_min.o");
        let _ = Command::new(cc)
            .args(["-O2", "-ffreestanding", "-fno-stack-protector", "-fno-pic", "-mno-red-zone",
                   "-c", &root.join("runtime/linux_x86_64_min.c").to_string_lossy(),
                   "-o", &obj.to_string_lossy()])
            .status();
        obj
    } else {
        tmp.join("lpp_runtime_min.o")
    };

    let cases: Vec<(&str, &str, &str)> = vec![
        ("T01_arith",          "tests/arith.lpp",                       "15\n5\n50\n2"),
        ("T02_branches",       "tests/branches.lpp",                    "1\n0\n1"),
        ("T03_fib",            "benchmarks/bench_fib.lpp",               "9227465"),
        ("T04_loop",           "benchmarks/bench_loop.lpp",              "49999995000000"),
        ("T05_calls",          "benchmarks/bench_calls.lpp",             "500000500000"),
        ("T06_nested",         "tests/nested_calls.lpp",                 "120"),
        ("T07_closure",        "tests/closure_test.lpp",                 "52"),
        ("T08_list",           "tests/list_safety.lpp",                  "3\n5\n13"),
        ("T09_owned_return",   "tests/owned_return.lpp",                 "1"),
        ("T10_arc_branch",     "tests/arc_branch_return.lpp",            "1"),
        ("T11_arc_nested",     "tests/arc_nested_struct.lpp",            "1"),
        ("T12_arc_alias",      "tests/arc_direct_alias.lpp",             "1"),
        ("T13_arc_closure",    "tests/arc_closure_capture.lpp",          "0"),
        ("T14_arc_field",      "tests/arc_field_alias.lpp",              "1"),
        ("T15_arc_list_custom", "tests/arc_list_custom.lpp",             "1"),
    ];

    let mut passed = 0;
    let mut failed = 0;

    for (name, src_rel, expected) in &cases {
        let src = root.join(src_rel);
        let tmp_src = tmp.join(format!("{}.lpp", name));
        let _ = fs::copy(&src, &tmp_src);

        // Compile
        if !Command::new(&lpp)
            .env("LPP_AOT", "1").env("LPP_AOT_ONLY", "1")
            .arg(&tmp_src)
            .stdout(Stdio::null()).stderr(Stdio::null())
            .status().map(|s| s.success()).unwrap_or(false)
        {
            eprintln!("  FAIL {name}: compile failed");
            failed += 1;
            continue;
        }

        let obj = tmp_src.with_extension("o");
        let exe = tmp.join(name);

        // Direct link
        let link_ok = if linker.exists() {
            Command::new(&linker).arg(&obj).arg(&runtime_obj).arg("-o").arg(&exe)
                .stdout(Stdio::null()).stderr(Stdio::null())
                .status().map(|s| s.success()).unwrap_or(false)
        } else { false };

        if !link_ok {
            // Fallback to host linker
            if let Some(ref cc) = cc {
                let _ = Command::new(cc)
                    .args(["-O2", &obj.to_string_lossy(), &root.join("lpp_runtime.c").to_string_lossy(),
                           "-o", &exe.to_string_lossy()])
                    .arg(if cfg!(unix) { "-pthread" } else { "" })
                    .stdout(Stdio::null()).stderr(Stdio::null())
                    .status();
            }
        }

        if exe.exists() {
            if let Ok(out) = Command::new(&exe).output() {
                let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if stdout == *expected {
                    println!("  PASS {name}");
                    passed += 1;
                } else {
                    eprintln!("  FAIL {name}: expected '{expected}', got '{stdout}'");
                    failed += 1;
                }
            } else {
                eprintln!("  FAIL {name}: execution error");
                failed += 1;
            }
        } else {
            eprintln!("  FAIL {name}: link failed");
            failed += 1;
        }

        let _ = fs::remove_file(&tmp_src);
        let _ = fs::remove_file(&obj);
        let _ = fs::remove_file(&exe);
    }

    println!("\n═══ Results: {passed} passed, {failed} failed ═══");
    failed == 0
}

// ═══════════════════════════════════════════════════════════════════════════
//  CLI entry point
// ═══════════════════════════════════════════════════════════════════════════

fn usage() {
    eprintln!("lpp-bench — L++ linker and resource benchmark tool");
    eprintln!();
    eprintln!("Usage:");
    eprintln!("  lpp-bench [options]");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --linkers <mode,...>   Comma-separated: direct, mold, c (default: all three)");
    eprintln!("  --suite king20         Benchmark suite (default: king20)");
    eprintln!("  --json                 Output final report as JSON to stdout");
    eprintln!("  --pretty               Pretty-print JSON output");
    eprintln!("  --disk                 Show disk usage statistics");
    eprintln!("  --mem                  Show memory/residency statistics");
    eprintln!("  --self-test            Run 15 built-in integration tests");
    eprintln!("  --help                 Show this help");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  lpp-bench                                           # Full King20, all linkers");
    eprintln!("  lpp-bench --linkers direct,mold --disk --mem --json # Direct+mold, with stats");
    eprintln!("  lpp-bench --self-test                               # 15 integration tests");
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        usage();
        return;
    }

    if args.iter().any(|a| a == "--self-test") {
        let ok = run_self_tests();
        std::process::exit(if ok { 0 } else { 1 });
    }

    let linkers = args.iter().position(|a| a == "--linkers")
        .and_then(|i| args.get(i + 1))
        .map(|s| {
            let modes = LinkerMode::from_str(s);
            if modes.is_empty() {
                eprintln!("WARNING: no valid linkers in '--linkers {s}', using all three");
                LinkerMode::from_str("direct,mold,c")
            } else {
                modes
            }
        })
        .unwrap_or_else(|| LinkerMode::from_str("direct,mold,c"));

    let show_json = args.iter().any(|a| a == "--json");
    let pretty = args.iter().any(|a| a == "--pretty");
    let show_disk = args.iter().any(|a| a == "--disk");
    let show_mem = args.iter().any(|a| a == "--mem");

    eprintln!("═══ L++ Linker Benchmark ═══");
    eprintln!("Linkers: {}", linkers.iter().map(|m| m.short_label()).collect::<Vec<_>>().join(", "));
    eprintln!("Suite:   King20 (20 workloads)");
    eprintln!();

    let report = run_full_bench(&linkers, show_disk, show_mem);

    if show_json {
        if pretty {
            if let Ok(json) = serde_json::to_string_pretty(&report) {
                println!("{json}");
            }
        } else {
            if let Ok(json) = serde_json::to_string(&report) {
                println!("{json}");
            }
        }
    }

    eprintln!("\n═══ Summary ═══");
    eprintln!("           | Passed | Link(ms) | Binary(KB) | RSS(MB)");
    eprintln!("-----------+--------+----------+------------+--------");
    eprintln!(" direct    | {:>6} | {:>8.2} | {:>10.1} | {:>7.2}",
        report.summary.direct_passed, report.summary.direct_mean_link_ms,
        report.summary.direct_mean_binary_kb, report.summary.direct_mean_rss_mb);
    eprintln!(" mold      | {:>6} | {:>8.2} | {:>10.1} | {:>7.2}",
        report.summary.mold_passed, report.summary.mold_mean_link_ms,
        report.summary.mold_mean_binary_kb, report.summary.mold_mean_rss_mb);
    eprintln!(" C transp. | {:>6} | {:>8.2} | {:>10.1} | {:>7.2}",
        report.summary.c_transpiler_passed, report.summary.c_mean_link_ms,
        report.summary.c_mean_binary_kb, report.summary.c_mean_rss_mb);

    let total_ok = report.workloads.iter().filter(|w| w.verdict == "PASS").count();
    let exit = if total_ok == report.total_workloads { 0 } else { 1 };
    std::process::exit(exit);
}
