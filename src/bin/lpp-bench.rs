//! `lpp-bench` — Cross-platform linker and resource benchmark CLI.
//!
//! Runs King20 workloads across all three linker paths (direct, mold/host, C
//! transpiler) and measures wall-clock time, binary size, disk usage, peak
//! RSS, and memory pressure. Produces JSON and human-readable reports.
//!
//! Works on Linux, Windows (MSVC / MinGW), and macOS (clang).
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
    host_passed: usize,
    c_transpiler_passed: usize,
    direct_mean_link_ms: f64,
    host_mean_link_ms: f64,
    c_mean_link_ms: f64,
    direct_mean_binary_kb: f64,
    host_mean_binary_kb: f64,
    c_mean_binary_kb: f64,
    direct_mean_rss_mb: f64,
    host_mean_rss_mb: f64,
    c_mean_rss_mb: f64,
}

struct King20Case {
    id: usize,
    name: String,
    source: PathBuf,
    expected: String,
}

// ═══════════════════════════════════════════════════════════════════════════
//  Platform abstraction layer
// ═══════════════════════════════════════════════════════════════════════════

struct Platform {
    /// "linux" | "windows" | "macos"
    #[allow(dead_code)]
    os: &'static str,
    /// Executable suffix: ".exe" or ""
    exe_suffix: &'static str,
    /// Object file suffix: "o" or "obj"
    obj_suffix: &'static str,
    /// Whether `-fuse-ld=mold` makes sense here
    can_mold: bool,
}

impl Platform {
    fn detect() -> Self {
        Self {
            os: std::env::consts::OS,
            exe_suffix: if cfg!(target_os = "windows") {
                ".exe"
            } else {
                ""
            },
            obj_suffix: if cfg!(target_os = "windows") {
                "obj"
            } else {
                "o"
            },
            can_mold: cfg!(all(unix, not(target_os = "macos"))),
        }
    }

    fn freestanding_runtime(&self, root: &Path) -> PathBuf {
        if cfg!(target_os = "windows") {
            root.join("runtime").join("windows_x86_64_min.c")
        } else if cfg!(target_os = "macos") {
            // macOS freestanding runtime uses libSystem, not raw syscalls.
            // Until the Mach-O direct runtime exists, use the full runtime.
            root.join("lpp_runtime.c")
        } else {
            root.join("runtime").join("linux_x86_64_min.c")
        }
    }

    fn runtime_flags(&self, cc: &str) -> Vec<String> {
        if cc.contains("cl.exe") || cc == "cl" {
            vec!["/O2".into(), "/GS-".into(), "/c".into()]
        } else if cfg!(target_os = "macos") {
            vec!["-O2".into(), "-fPIC".into(), "-c".into()]
        } else {
            vec![
                "-O2".into(),
                "-ffreestanding".into(),
                "-fno-stack-protector".into(),
                "-fno-pic".into(),
                "-mno-red-zone".into(),
                "-c".into(),
            ]
        }
    }

    fn host_link_flags(&self, cc: &str) -> Vec<String> {
        if cc.contains("cl.exe") || cc == "cl" {
            vec!["/nologo".into(), "/O2".into(), "/Fe:".into()]
        } else if cfg!(target_os = "macos") {
            vec!["-O2".into()]
        } else {
            vec!["-O2".into(), "-pthread".into()]
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Binary discovery
// ═══════════════════════════════════════════════════════════════════════════

fn plat() -> Platform {
    Platform::detect()
}

fn lpp_binary() -> PathBuf {
    let p = plat();
    let exe = env::current_exe().unwrap_or_else(|_| PathBuf::from("lpp"));
    exe.parent()
        .map(|d| d.join(format!("lpp{}", p.exe_suffix)))
        .unwrap_or_else(|| PathBuf::from(format!("lpp{}", p.exe_suffix)))
}

fn lpp_link_binary() -> PathBuf {
    let p = plat();
    let exe = env::current_exe().unwrap_or_else(|_| PathBuf::from("lpp-bench"));
    exe.parent()
        .map(|d| d.join(format!("lpp-link{}", p.exe_suffix)))
        .unwrap_or_else(|| PathBuf::from(format!("lpp-link{}", p.exe_suffix)))
}

fn repo_root() -> PathBuf {
    let exe = env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    let mut dir = exe
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
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

// ═══════════════════════════════════════════════════════════════════════════
//  Compiler detection (cross-platform)
// ═══════════════════════════════════════════════════════════════════════════

fn detect_c_compiler() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        // Try cl.exe on PATH first (already in VS command prompt)
        for cc in &["cl.exe", "cl"] {
            if try_run(cc, &["/?", ">/nul"]) || try_run(cc, &["--version"]) {
                return Some(cc.to_string());
            }
        }
        // Auto-detect MSVC via vcvars64.bat, same as pm.rs
        if let Some(vcvars) = find_vcvars64() {
            if load_vcvars_env(&vcvars).is_ok() {
                if try_run("cl.exe", &["/?", ">/nul"]) || try_run("cl", &["/?", ">/nul"]) {
                    return Some("cl.exe".to_string());
                }
                for cc in &["gcc", "clang"] {
                    if try_run(cc, &["--version"]) {
                        return Some(cc.to_string());
                    }
                }
            }
        }
        for cc in &["gcc", "clang"] {
            if try_run(cc, &["--version"]) {
                return Some(cc.to_string());
            }
        }
        return None;
    }
    #[cfg(not(target_os = "windows"))]
    {
        for cc in &["clang", "gcc", "cc"] {
            if try_run(cc, &["--version"]) {
                return Some(cc.to_string());
            }
        }
    }
    None
}

/// Locate vcvars64.bat at standard MSVC install paths (Windows only).
#[cfg(target_os = "windows")]
fn find_vcvars64() -> Option<PathBuf> {
    let fallbacks = [
        r"C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat",
        r"C:\Program Files\Microsoft Visual Studio\2022\Professional\VC\Auxiliary\Build\vcvars64.bat",
        r"C:\Program Files\Microsoft Visual Studio\2022\Enterprise\VC\Auxiliary\Build\vcvars64.bat",
        r"C:\Program Files\Microsoft Visual Studio\2019\Community\VC\Auxiliary\Build\vcvars64.bat",
        r"C:\Program Files\Microsoft Visual Studio\2019\Professional\VC\Auxiliary\Build\vcvars64.bat",
        r"C:\Program Files\Microsoft Visual Studio\2019\Enterprise\VC\Auxiliary\Build\vcvars64.bat",
    ];
    for f in &fallbacks {
        let p = Path::new(f);
        if p.exists() {
            return Some(p.to_path_buf());
        }
    }
    None
}

/// Load environment from a vcvars64.bat script (Windows only).
#[cfg(target_os = "windows")]
fn load_vcvars_env(vcvars: &Path) -> Result<(), String> {
    let temp_dir = std::env::temp_dir();
    let bat_path = temp_dir.join("lpp_bench_vcvars.bat");
    let bat_content = format!("@echo off\ncall \"{}\" > nul\nset\n", vcvars.display());
    fs::write(&bat_path, bat_content).map_err(|e| format!("write batch: {e}"))?;
    let output = Command::new("cmd.exe")
        .args(["/c", &bat_path.to_string_lossy()])
        .output()
        .map_err(|e| format!("run cmd: {e}"))?;
    let _ = fs::remove_file(&bat_path);
    if !output.status.success() {
        return Err("vcvars failed".to_string());
    }
    let env_dump = String::from_utf8_lossy(&output.stdout);
    for line in env_dump.lines() {
        if let Some(eq_idx) = line.find('=') {
            let name = &line[..eq_idx];
            let val = &line[eq_idx + 1..];
            unsafe {
                std::env::set_var(name, val);
            }
        }
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn find_vcvars64() -> Option<PathBuf> {
    None
}
#[cfg(not(target_os = "windows"))]
fn load_vcvars_env(_vcvars: &Path) -> Result<(), String> {
    Ok(())
}

fn try_run(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

fn has_mold() -> bool {
    plat().can_mold && try_run("mold", &["--version"])
}

// ═══════════════════════════════════════════════════════════════════════════
//  Timestamp
// ═══════════════════════════════════════════════════════════════════════════

fn current_timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let days = secs / 86400;
    let time = secs % 86400;
    // Days since unix epoch → approximate YYYY-MM-DD
    let mut y = 1970i64;
    let mut d = days as i64;
    loop {
        let year_days = if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
            366
        } else {
            365
        };
        if d < year_days {
            break;
        }
        d -= year_days;
        y += 1;
    }
    let months_days = if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut m = 0;
    for (i, md) in months_days.iter().enumerate() {
        if d < *md {
            m = i + 1;
            break;
        }
        d -= md;
    }
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d + 1,
        time / 3600,
        (time % 3600) / 60,
        time % 60
    )
}

// ═══════════════════════════════════════════════════════════════════════════
//  Resource measurement  (cross-platform best-effort)
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(target_os = "linux")]
fn read_peak_rss_kb() -> Option<u64> {
    if let Ok(mut f) = fs::File::open("/proc/self/status") {
        let mut s = String::new();
        if f.read_to_string(&mut s).is_ok() {
            for line in s.lines() {
                if line.starts_with("VmHWM:") {
                    return line.split_whitespace().nth(1).and_then(|v| v.parse().ok());
                }
            }
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn read_peak_rss_kb() -> Option<u64> {
    // task_info via libc or sysctl — fall back to getrusage
    // On macOS we can't easily get peak RSS from /proc (doesn't exist).
    // Fallback: use ps subprocess.
    let pid = std::process::id();
    if let Ok(out) = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        return s.parse::<u64>().ok();
    }
    None
}

#[cfg(target_os = "windows")]
fn read_peak_rss_kb() -> Option<u64> {
    // Windows: use GetProcessMemoryInfo via a small PS snippet
    let pid = std::process::id();
    let ps = format!("(Get-Process -Id {pid}).WorkingSet64 / 1KB");
    if let Ok(out) = Command::new("powershell").args(["-Command", &ps]).output() {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        return s.parse::<f64>().ok().map(|v| v as u64);
    }
    None
}

fn file_size_bytes(path: &Path) -> u64 {
    fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn disk_usage_kb(dir: &Path) -> u64 {
    if !dir.is_dir() {
        return file_size_bytes(dir) / 1024;
    }
    let mut total = 0u64;
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                total += disk_usage_kb(&p);
            } else {
                total += fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    total / 1024
}

// ═══════════════════════════════════════════════════════════════════════════
//  King20 manifest
// ═══════════════════════════════════════════════════════════════════════════

fn load_king20() -> Vec<King20Case> {
    let root = repo_root();
    vec![
        (1, "recursive-fib-35", "benchmarks/bench_fib.lpp", "9227465"),
        (2, "loop-10m", "benchmarks/bench_loop.lpp", "49999995000000"),
        (
            3,
            "call-chain-1m",
            "benchmarks/bench_calls.lpp",
            "500000500000",
        ),
        (4, "integer-arithmetic", "tests/arith.lpp", "15\n5\n50\n2"),
        (5, "conditional-branches", "tests/branches.lpp", "1\n0\n1"),
        (6, "nested-direct-calls", "tests/nested_calls.lpp", "120"),
        (7, "immutable-closure", "tests/closure_test.lpp", "52"),
        (8, "arc-list-int", "tests/list_safety.lpp", "3\n5\n13"),
        (9, "owned-struct-return", "tests/owned_return.lpp", "1"),
        (
            10,
            "branch-owned-return",
            "tests/arc_branch_return.lpp",
            "1",
        ),
        (
            11,
            "nested-struct-destructor",
            "tests/arc_nested_struct.lpp",
            "1",
        ),
        (12, "direct-arc-alias", "tests/arc_direct_alias.lpp", "1"),
        (
            13,
            "closure-arc-capture",
            "tests/arc_closure_capture.lpp",
            "0",
        ),
        (
            14,
            "borrowed-parameter-return",
            "tests/arc_borrowed_return.lpp",
            "1",
        ),
        (
            15,
            "borrowed-field-return",
            "tests/arc_borrowed_field_return.lpp",
            "1",
        ),
        (16, "field-alias", "tests/arc_field_alias.lpp", "1"),
        (17, "list-int-alias", "tests/arc_list_alias.lpp", "7"),
        (
            18,
            "list-custom-ownership",
            "tests/arc_list_custom.lpp",
            "1",
        ),
        (
            19,
            "nested-branch-alias",
            "tests/arc_nested_branch_alias.lpp",
            "1",
        ),
        (
            20,
            "closure-branch-capture",
            "tests/arc_closure_branch_capture.lpp",
            "0",
        ),
    ]
    .into_iter()
    .map(|(id, name, src, exp)| King20Case {
        id,
        name: name.to_string(),
        source: root.join(src),
        expected: exp.to_string(),
    })
    .collect()
}

// ═══════════════════════════════════════════════════════════════════════════
//  Linker modes
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkerMode {
    Direct,
    Host,
    CTranspiler,
}

impl LinkerMode {
    fn from_str(s: &str) -> Vec<LinkerMode> {
        s.split(',')
            .filter_map(|tok| match tok.trim() {
                "direct" => Some(LinkerMode::Direct),
                "mold" | "host" => Some(LinkerMode::Host),
                "c" => Some(LinkerMode::CTranspiler),
                _ => None,
            })
            .collect()
    }

    fn short_label(&self) -> &'static str {
        match self {
            LinkerMode::Direct => "direct",
            LinkerMode::Host => "host",
            LinkerMode::CTranspiler => "c",
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Linker runners
// ═══════════════════════════════════════════════════════════════════════════

fn build_runtime_obj(tmp: &Path, freestanding_src: &Path) -> Result<PathBuf, String> {
    let p = plat();
    let obj = tmp.join(format!("lpp_runtime_min.{}", p.obj_suffix));
    if obj.exists() {
        return Ok(obj);
    }

    let cc = detect_c_compiler().ok_or("no C compiler found")?;
    let mut cmd = Command::new(&cc);
    for flag in &p.runtime_flags(&cc) {
        if cc.contains("cl.exe") || cc == "cl" {
            cmd.arg(flag);
        } else {
            cmd.arg(flag);
        }
    }
    let flags = p.runtime_flags(&cc);
    if cc.contains("cl.exe") || cc == "cl" {
        cmd.args(&flags).arg(format!("/Fo:{}", obj.display()));
    } else {
        cmd.args(&flags).arg("-o").arg(&obj);
    }
    cmd.arg(freestanding_src);
    let status = cmd.status().map_err(|e| format!("compile runtime: {e}"))?;
    if status.success() {
        Ok(obj)
    } else {
        Err("runtime compile failed".into())
    }
}

fn run_linker_direct(
    obj: &Path,
    runtime_obj: &Path,
    output: &Path,
) -> Result<LinkerResult, String> {
    let linker = lpp_link_binary();
    if !linker.exists() {
        return Err("lpp-link binary not found".into());
    }
    let start = Instant::now();
    let out = Command::new(&linker)
        .arg(obj)
        .arg(runtime_obj)
        .arg("-o")
        .arg(output)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn: {e}"))?
        .wait_with_output()
        .map_err(|e| format!("wait: {e}"))?;
    Ok(LinkerResult {
        linker: "direct (lpp-link)".into(),
        link_time_ms: start.elapsed().as_secs_f64() * 1000.0,
        binary_size_bytes: file_size_bytes(output),
        peak_rss_kb: read_peak_rss_kb(),
        exit_code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        success: out.status.success(),
    })
}

fn run_linker_host(obj: &Path, runtime_src: &Path, output: &Path) -> Result<LinkerResult, String> {
    let _p = plat();
    let cc = detect_c_compiler().ok_or("no C compiler found")?;
    let start = Instant::now();
    let mut cmd = Command::new(&cc);

    if has_mold() {
        cmd.arg("-fuse-ld=mold");
        cmd.arg("-Wl,--icf=all");
        cmd.arg("-Wl,-O1");
    }

    if cc.contains("cl.exe") || cc == "cl" {
        cmd.arg("/nologo")
            .arg("/O2")
            .arg(obj)
            .arg(runtime_src)
            .arg(format!("/Fe:{}", output.display()));
    } else {
        cmd.arg("-O2")
            .arg(obj)
            .arg(runtime_src)
            .arg("-o")
            .arg(output);
        if !cfg!(target_os = "macos") {
            cmd.arg("-pthread");
        }
    }

    let out = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn {cc}: {e}"))?
        .wait_with_output()
        .map_err(|e| format!("wait: {e}"))?;
    let label = if has_mold() {
        "mold via ".to_string() + &cc
    } else {
        cc
    };

    Ok(LinkerResult {
        linker: label,
        link_time_ms: start.elapsed().as_secs_f64() * 1000.0,
        binary_size_bytes: file_size_bytes(output),
        peak_rss_kb: read_peak_rss_kb(),
        exit_code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        success: out.status.success(),
    })
}

fn run_linker_c_transpile(
    source: &Path,
    runtime_src: &Path,
    output: &Path,
    lpp: &Path,
) -> Result<LinkerResult, String> {
    let _p = plat();
    let cc = detect_c_compiler().ok_or("no C compiler found")?;
    let transpile_start = Instant::now();

    if !Command::new(lpp)
        .arg(source)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        return Err("transpile failed".into());
    }

    let c_path = source.with_extension("c");
    if !c_path.exists() {
        return Err("C file not generated".into());
    }

    let start = Instant::now();
    let mut cmd = Command::new(&cc);
    if cc.contains("cl.exe") || cc == "cl" {
        cmd.arg("/nologo")
            .arg("/O2")
            .arg(&c_path)
            .arg(runtime_src)
            .arg(format!("/Fe:{}", output.display()));
    } else {
        cmd.arg("-O2")
            .arg(&c_path)
            .arg(runtime_src)
            .arg("-o")
            .arg(output);
        if !cfg!(target_os = "macos") {
            cmd.arg("-pthread");
        }
    }
    let out = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("C compile: {e}"))?
        .wait_with_output()
        .map_err(|e| format!("wait: {e}"))?;
    let _ = fs::remove_file(&c_path);
    Ok(LinkerResult {
        linker: "C transpiler".into(),
        link_time_ms: (transpile_start.elapsed().as_secs_f64() + start.elapsed().as_secs_f64())
            * 1000.0,
        binary_size_bytes: file_size_bytes(output),
        peak_rss_kb: read_peak_rss_kb(),
        exit_code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        success: out.status.success(),
    })
}

// ═══════════════════════════════════════════════════════════════════════════
//  Main benchmark runner
// ═══════════════════════════════════════════════════════════════════════════

fn run_full_bench(linkers: &[LinkerMode], show_disk: bool, show_mem: bool) -> BenchReport {
    let p = plat();
    let lpp = lpp_binary();
    let root = repo_root();
    let runtime_src = root.join("lpp_runtime.c");
    let freestanding_src = p.freestanding_runtime(&root);
    let workloads = load_king20();
    let tmp = std::env::temp_dir().join("lpp-bench");
    let _ = fs::create_dir_all(&tmp);

    // Pre-build freestanding runtime object for direct linker
    let rt_obj = if linkers.contains(&LinkerMode::Direct) {
        build_runtime_obj(&tmp, &freestanding_src).ok()
    } else {
        None
    };

    let mut results = Vec::new();

    for case in &workloads {
        eprintln!("\n[{:>2}/{}] {}", case.id, workloads.len(), case.name);
        let tmp_src = tmp.join(format!("{}.lpp", case.name));
        let tmp_obj = tmp.join(format!("{}.o", case.name));
        let _ = fs::copy(&case.source, &tmp_src);

        let compile_start = Instant::now();
        let ok = Command::new(&lpp)
            .env("LPP_AOT", "1")
            .env("LPP_AOT_ONLY", "1")
            .arg(&tmp_src)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        let compile_ms = compile_start.elapsed().as_secs_f64() * 1000.0;

        if !ok {
            eprintln!("  SKIP: compile failed");
            continue;
        }

        let obj_path = if tmp_obj.exists() {
            tmp_obj.clone()
        } else {
            tmp_src.with_extension(p.obj_suffix)
        };
        let mut linker_results = Vec::new();

        for mode in linkers {
            let exe = tmp.join(format!(
                "{}_{}{}",
                case.name,
                mode.short_label(),
                p.exe_suffix
            ));
            let r = match mode {
                LinkerMode::Direct => {
                    if let Some(ref rt) = rt_obj {
                        run_linker_direct(&obj_path, rt, &exe)
                    } else {
                        Err("no runtime object".into())
                    }
                }
                LinkerMode::Host => run_linker_host(&obj_path, &runtime_src, &exe),
                LinkerMode::CTranspiler => {
                    run_linker_c_transpile(&tmp_src, &runtime_src, &exe, &lpp)
                }
            }
            .unwrap_or_else(|e| LinkerResult {
                linker: mode.short_label().into(),
                link_time_ms: 0.0,
                binary_size_bytes: 0,
                peak_rss_kb: None,
                exit_code: Some(1),
                stdout: String::new(),
                stderr: e,
                success: false,
            });

            let (run_stdout, run_exit) = if r.success && exe.exists() {
                if let Ok(out) = Command::new(&exe).output() {
                    (
                        String::from_utf8_lossy(&out.stdout).trim().to_string(),
                        out.status.code(),
                    )
                } else {
                    (String::new(), Some(1))
                }
            } else {
                (String::new(), None)
            };

            let verdict = if run_stdout == case.expected {
                "PASS"
            } else {
                "FAIL"
            };
            eprintln!(
                "  {} | {:>8} | {}ms | {}",
                mode.short_label(),
                verdict,
                r.link_time_ms as u64,
                if r.success {
                    format!("{}KB", r.binary_size_bytes / 1024)
                } else {
                    "FAIL".into()
                }
            );

            let mut result = r;
            result.stdout = run_stdout;
            result.exit_code = run_exit;
            result.success = verdict == "PASS";
            linker_results.push(result);
            let _ = fs::remove_file(&exe);
        }

        let overall = if linker_results.iter().all(|lr| lr.success) {
            "PASS"
        } else if linker_results.iter().any(|lr| lr.success) {
            "PARTIAL"
        } else {
            "FAIL"
        };
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
    let mut dir_p = 0usize;
    let mut dir_ms = 0.0;
    let mut dir_kb = 0.0;
    let mut dir_rss = 0.0;
    let mut dir_n = 0u64;
    let mut host_p = 0usize;
    let mut host_ms = 0.0;
    let mut host_kb = 0.0;
    let mut host_rss = 0.0;
    let mut host_n = 0u64;
    let mut c_p = 0usize;
    let mut c_ms = 0.0;
    let mut c_kb = 0.0;
    let mut c_rss = 0.0;
    let mut c_n = 0u64;

    for w in &results {
        for lr in &w.linkers {
            match lr.linker.as_str() {
                "direct (lpp-link)" => {
                    if lr.success {
                        dir_p += 1;
                    }
                    dir_ms += lr.link_time_ms;
                    dir_kb += lr.binary_size_bytes as f64;
                    if let Some(r) = lr.peak_rss_kb {
                        dir_rss += r as f64;
                    }
                    dir_n += 1;
                }
                "C transpiler" => {
                    if lr.success {
                        c_p += 1;
                    }
                    c_ms += lr.link_time_ms;
                    c_kb += lr.binary_size_bytes as f64;
                    if let Some(r) = lr.peak_rss_kb {
                        c_rss += r as f64;
                    }
                    c_n += 1;
                }
                _ => {
                    if lr.success {
                        host_p += 1;
                    }
                    host_ms += lr.link_time_ms;
                    host_kb += lr.binary_size_bytes as f64;
                    if let Some(r) = lr.peak_rss_kb {
                        host_rss += r as f64;
                    }
                    host_n += 1;
                }
            }
        }
    }
    if dir_n > 0 {
        dir_ms /= dir_n as f64;
        dir_kb /= dir_n as f64;
        dir_rss /= dir_n as f64;
    }
    if host_n > 0 {
        host_ms /= host_n as f64;
        host_kb /= host_n as f64;
        host_rss /= host_n as f64;
    }
    if c_n > 0 {
        c_ms /= c_n as f64;
        c_kb /= c_n as f64;
        c_rss /= c_n as f64;
    }

    let summary = LinkerSummary {
        direct_passed: dir_p,
        host_passed: host_p,
        c_transpiler_passed: c_p,
        direct_mean_link_ms: dir_ms,
        host_mean_link_ms: host_ms,
        c_mean_link_ms: c_ms,
        direct_mean_binary_kb: dir_kb / 1024.0,
        host_mean_binary_kb: host_kb / 1024.0,
        c_mean_binary_kb: c_kb / 1024.0,
        direct_mean_rss_mb: dir_rss / 1024.0,
        host_mean_rss_mb: host_rss / 1024.0,
        c_mean_rss_mb: c_rss / 1024.0,
    };

    if show_disk {
        eprintln!("\nDisk usage:");
        eprintln!("  lpp binary:      {} KB", file_size_bytes(&lpp) / 1024);
        eprintln!(
            "  lpp-link binary:  {} KB",
            file_size_bytes(&lpp_link_binary()) / 1024
        );
        eprintln!("  tmp artifacts:    {} KB", disk_usage_kb(&tmp));
    }
    if show_mem {
        if let Some(rss) = read_peak_rss_kb() {
            eprintln!("  Peak RSS:         {} MB", rss as f64 / 1024.0);
        }
    }

    BenchReport {
        toolchain: format!(
            "L++ v{} (rustc {})",
            env!("CARGO_PKG_VERSION"),
            Command::new("rustc")
                .arg("--version")
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_else(|_| "unknown".into())
        ),
        host: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        timestamp: current_timestamp(),
        total_workloads: results.len(),
        workloads: results,
        summary,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  15 integration tests
// ═══════════════════════════════════════════════════════════════════════════

fn run_self_tests() -> bool {
    println!("═══ L++ Bench Self-Test Suite (15 tests) ═══\n");
    let p = plat();
    let lpp = lpp_binary();
    let linker = lpp_link_binary();
    let root = repo_root();
    let tmp = std::env::temp_dir().join("lpp-bench-self-test");
    let _ = fs::create_dir_all(&tmp);

    // Build freestanding runtime for direct linking
    let freestanding = p.freestanding_runtime(&root);
    let runtime_obj = build_runtime_obj(&tmp, &freestanding).ok();
    let cc = detect_c_compiler();

    let cases: Vec<(&str, &str, &str)> = vec![
        ("T01_arith", "tests/arith.lpp", "15\n5\n50\n2"),
        ("T02_branches", "tests/branches.lpp", "1\n0\n1"),
        ("T03_fib", "benchmarks/bench_fib.lpp", "9227465"),
        ("T04_loop", "benchmarks/bench_loop.lpp", "49999995000000"),
        ("T05_calls", "benchmarks/bench_calls.lpp", "500000500000"),
        ("T06_nested", "tests/nested_calls.lpp", "120"),
        ("T07_closure", "tests/closure_test.lpp", "52"),
        ("T08_list", "tests/list_safety.lpp", "3\n5\n13"),
        ("T09_owned_return", "tests/owned_return.lpp", "1"),
        ("T10_arc_branch", "tests/arc_branch_return.lpp", "1"),
        ("T11_arc_nested", "tests/arc_nested_struct.lpp", "1"),
        ("T12_arc_alias", "tests/arc_direct_alias.lpp", "1"),
        ("T13_arc_closure", "tests/arc_closure_capture.lpp", "0"),
        ("T14_arc_field", "tests/arc_field_alias.lpp", "1"),
        ("T15_arc_list_custom", "tests/arc_list_custom.lpp", "1"),
    ];

    let mut passed = 0;
    let mut failed = 0;

    for (name, src_rel, expected) in &cases {
        let src = root.join(src_rel);
        let tmp_src = tmp.join(format!("{}.lpp", name));
        let _ = fs::copy(&src, &tmp_src);

        if !Command::new(&lpp)
            .env("LPP_AOT", "1")
            .env("LPP_AOT_ONLY", "1")
            .arg(&tmp_src)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            eprintln!("  FAIL {name}: compile");
            failed += 1;
            continue;
        }

        let obj = tmp_src.with_extension(p.obj_suffix);
        let exe = tmp.join(format!("{}{}", name, p.exe_suffix));

        // Try direct link first, fall back to host linker
        let linked = if let Some(ref rt) = runtime_obj {
            linker.exists()
                && Command::new(&linker)
                    .arg(&obj)
                    .arg(rt)
                    .arg("-o")
                    .arg(&exe)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false)
        } else {
            false
        };

        if !linked {
            if let Some(ref cc) = cc {
                let runtime = root.join("lpp_runtime.c");
                let mut cmd = Command::new(cc);
                if cc.contains("cl.exe") || cc == "cl" {
                    cmd.args([
                        "/nologo",
                        "/O2",
                        &obj.to_string_lossy(),
                        &runtime.to_string_lossy(),
                        &format!("/Fe:{}", exe.display()),
                    ]);
                } else {
                    cmd.args([
                        "-O2",
                        &obj.to_string_lossy(),
                        &runtime.to_string_lossy(),
                        "-o",
                        &exe.to_string_lossy(),
                    ]);
                    if !cfg!(target_os = "macos") {
                        cmd.arg("-pthread");
                    }
                }
                cmd.stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .ok();
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
                eprintln!("  FAIL {name}: exec error");
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
//  CLI
// ═══════════════════════════════════════════════════════════════════════════

fn usage() {
    eprintln!("lpp-bench — L++ cross-platform linker benchmark");
    eprintln!();
    eprintln!("Usage:  lpp-bench [options]");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --linkers <mode,...>  Comma-separated: direct,host,c (default: all)");
    eprintln!("  --suite king20        Benchmark suite (default: king20)");
    eprintln!("  --json                Output JSON report to stdout");
    eprintln!("  --pretty              Pretty-print JSON");
    eprintln!("  --disk                Show disk usage");
    eprintln!("  --mem                 Show memory (RSS)");
    eprintln!("  --self-test           Run 15 integration tests");
    eprintln!("  --help                Show this help");
    eprintln!();
    eprintln!("Linker modes:");
    eprintln!("  direct   lpp-link native emitter (fastest)");
    eprintln!("  host     system cc + ld (or mold if auto-detected)");
    eprintln!("  c        C transpiler backend via lpp emit");
    eprintln!();
    eprintln!("Platforms: Linux (x86-64), Windows (x86-64 MSVC/MinGW), macOS (x86-64/arm64)");
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        usage();
        return;
    }
    if args.iter().any(|a| a == "--self-test") {
        std::process::exit(if run_self_tests() { 0 } else { 1 });
    }

    let linkers = args
        .iter()
        .position(|a| a == "--linkers")
        .and_then(|i| args.get(i + 1))
        .map(|s| {
            let m = LinkerMode::from_str(s);
            if m.is_empty() {
                LinkerMode::from_str("direct,host,c")
            } else {
                m
            }
        })
        .unwrap_or_else(|| LinkerMode::from_str("direct,host,c"));

    let show_json = args.iter().any(|a| a == "--json");
    let pretty = args.iter().any(|a| a == "--pretty");
    let show_disk = args.iter().any(|a| a == "--disk");
    let show_mem = args.iter().any(|a| a == "--mem");

    eprintln!("═══ L++ Linker Benchmark ═══");
    eprintln!(
        "Platform: {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    eprintln!(
        "Linkers:  {}",
        linkers
            .iter()
            .map(|m| m.short_label())
            .collect::<Vec<_>>()
            .join(", ")
    );
    eprintln!("Suite:    King20 (20 workloads)");
    eprintln!();

    let report = run_full_bench(&linkers, show_disk, show_mem);

    if show_json {
        let json = if pretty {
            serde_json::to_string_pretty(&report)
        } else {
            serde_json::to_string(&report)
        };
        if let Ok(j) = json {
            println!("{j}");
        }
    }

    eprintln!("\n═══ Summary ═══");
    eprintln!("           | Passed | Link(ms) | Binary(KB) | RSS(MB)");
    eprintln!("-----------+--------+----------+------------+--------");
    eprintln!(
        " direct    | {:>6} | {:>8.2} | {:>10.1} | {:>7.2}",
        report.summary.direct_passed,
        report.summary.direct_mean_link_ms,
        report.summary.direct_mean_binary_kb,
        report.summary.direct_mean_rss_mb
    );
    eprintln!(
        " host      | {:>6} | {:>8.2} | {:>10.1} | {:>7.2}",
        report.summary.host_passed,
        report.summary.host_mean_link_ms,
        report.summary.host_mean_binary_kb,
        report.summary.host_mean_rss_mb
    );
    eprintln!(
        " C transp. | {:>6} | {:>8.2} | {:>10.1} | {:>7.2}",
        report.summary.c_transpiler_passed,
        report.summary.c_mean_link_ms,
        report.summary.c_mean_binary_kb,
        report.summary.c_mean_rss_mb
    );

    let total_ok = report
        .workloads
        .iter()
        .filter(|w| w.verdict == "PASS")
        .count();
    std::process::exit(if total_ok == report.total_workloads {
        0
    } else {
        1
    });
}
