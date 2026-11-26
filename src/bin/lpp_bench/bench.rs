use super::*;

// ═══════════════════════════════════════════════════════════════════════════
//  Main benchmark runner
// ═══════════════════════════════════════════════════════════════════════════

pub(super) fn run_full_bench(
    linkers: &[LinkerMode],
    show_disk: bool,
    show_mem: bool,
) -> BenchReport {
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
    if show_mem && let Some(rss) = read_peak_rss_kb() {
        eprintln!("  Peak RSS:         {} MB", rss as f64 / 1024.0);
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

pub(super) fn run_self_tests() -> bool {
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

        if !linked && let Some(ref cc) = cc {
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
