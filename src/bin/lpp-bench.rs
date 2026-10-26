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
#[allow(unused_imports)]
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

#[path = "lpp_bench/bench.rs"]
mod bench;
#[path = "lpp_bench/linker.rs"]
mod linker;
#[path = "lpp_bench/manifest.rs"]
mod manifest;
#[path = "lpp_bench/platform.rs"]
mod platform;

use bench::{run_full_bench, run_self_tests};
use linker::*;
use manifest::load_king20;
use platform::*;

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
            vec![
                "/O2".into(),
                "/GS-".into(),
                "/DLPP_FREESTANDING".into(),
                "/c".into(),
            ]
        } else if cfg!(target_os = "macos") {
            vec![
                "-O2".into(),
                "-DLPP_FREESTANDING".into(),
                "-fPIC".into(),
                "-c".into(),
            ]
        } else {
            vec![
                "-O2".into(),
                "-DLPP_FREESTANDING".into(),
                "-ffreestanding".into(),
                "-fno-stack-protector".into(),
                "-fno-pic".into(),
                "-mno-red-zone".into(),
                "-c".into(),
            ]
        }
    }

    #[allow(dead_code)]
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
