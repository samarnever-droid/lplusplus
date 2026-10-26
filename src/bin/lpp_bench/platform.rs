use super::*;

// ═══════════════════════════════════════════════════════════════════════════
//  Binary discovery
// ═══════════════════════════════════════════════════════════════════════════

pub(super) fn plat() -> Platform {
    Platform::detect()
}

pub(super) fn lpp_binary() -> PathBuf {
    let p = plat();
    let exe = env::current_exe().unwrap_or_else(|_| PathBuf::from("lpp"));
    exe.parent()
        .map(|d| d.join(format!("lpp{}", p.exe_suffix)))
        .unwrap_or_else(|| PathBuf::from(format!("lpp{}", p.exe_suffix)))
}

pub(super) fn lpp_link_binary() -> PathBuf {
    let p = plat();
    let exe = env::current_exe().unwrap_or_else(|_| PathBuf::from("lpp-bench"));
    exe.parent()
        .map(|d| d.join(format!("lpp-link{}", p.exe_suffix)))
        .unwrap_or_else(|| PathBuf::from(format!("lpp-link{}", p.exe_suffix)))
}

pub(super) fn repo_root() -> PathBuf {
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

pub(super) fn detect_c_compiler() -> Option<String> {
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
        None
    }
    #[cfg(not(target_os = "windows"))]
    {
        for cc in &["clang", "gcc", "cc"] {
            if try_run(cc, &["--version"]) {
                return Some(cc.to_string());
            }
        }
        None
    }
}

/// Locate vcvars64.bat at standard MSVC install paths (Windows only).
#[cfg(target_os = "windows")]
pub(super) fn find_vcvars64() -> Option<PathBuf> {
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
pub(super) fn load_vcvars_env(vcvars: &Path) -> Result<(), String> {
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
#[allow(dead_code)]
pub(super) fn find_vcvars64() -> Option<PathBuf> {
    None
}
#[cfg(not(target_os = "windows"))]
#[allow(dead_code)]
pub(super) fn load_vcvars_env(_vcvars: &Path) -> Result<(), String> {
    Ok(())
}

pub(super) fn try_run(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

pub(super) fn has_mold() -> bool {
    plat().can_mold && try_run("mold", &["--version"])
}

// ═══════════════════════════════════════════════════════════════════════════
//  Timestamp
// ═══════════════════════════════════════════════════════════════════════════

pub(super) fn current_timestamp() -> String {
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
pub(super) fn read_peak_rss_kb() -> Option<u64> {
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
pub(super) fn read_peak_rss_kb() -> Option<u64> {
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
pub(super) fn read_peak_rss_kb() -> Option<u64> {
    // Windows: use GetProcessMemoryInfo via a small PS snippet
    let pid = std::process::id();
    let ps = format!("(Get-Process -Id {pid}).WorkingSet64 / 1KB");
    if let Ok(out) = Command::new("powershell").args(["-Command", &ps]).output() {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        return s.parse::<f64>().ok().map(|v| v as u64);
    }
    None
}

pub(super) fn file_size_bytes(path: &Path) -> u64 {
    fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

pub(super) fn disk_usage_kb(dir: &Path) -> u64 {
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
