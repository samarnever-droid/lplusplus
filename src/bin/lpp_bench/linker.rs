use super::*;

// ═══════════════════════════════════════════════════════════════════════════
//  Linker modes
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LinkerMode {
    Direct,
    Host,
    CTranspiler,
}

impl LinkerMode {
    pub(super) fn from_str(s: &str) -> Vec<LinkerMode> {
        s.split(',')
            .filter_map(|tok| match tok.trim() {
                "direct" => Some(LinkerMode::Direct),
                "mold" | "host" => Some(LinkerMode::Host),
                "c" => Some(LinkerMode::CTranspiler),
                _ => None,
            })
            .collect()
    }

    pub(super) fn short_label(&self) -> &'static str {
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

pub(super) fn build_runtime_obj(tmp: &Path, freestanding_src: &Path) -> Result<PathBuf, String> {
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

pub(super) fn run_linker_direct(
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

pub(super) fn run_linker_host(
    obj: &Path,
    runtime_src: &Path,
    output: &Path,
) -> Result<LinkerResult, String> {
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

pub(super) fn run_linker_c_transpile(
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
