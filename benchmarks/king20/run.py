#!/usr/bin/env python3
"""Run the L++ King 20 AOT benchmark/correctness standard.

The suite intentionally combines runtime workloads with ownership regressions.
Every case must produce its expected stdout and process exit status before its
single-run timing is recorded.
"""
from __future__ import annotations

import argparse
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SUITE_ROOT = Path(__file__).resolve().parent


def command_version(command: list[str]) -> str:
    try:
        return subprocess.check_output(command, text=True, stderr=subprocess.STDOUT).splitlines()[0]
    except (OSError, subprocess.CalledProcessError):
        return "unavailable"


def system_info() -> dict[str, object]:
    cpu_model = "unknown"
    try:
        for line in Path("/proc/cpuinfo").read_text().splitlines():
            if line.lower().startswith("model name"):
                cpu_model = line.split(":", 1)[1].strip()
                break
    except OSError:
        pass
    memory_kib = None
    try:
        for line in Path("/proc/meminfo").read_text().splitlines():
            if line.startswith("MemTotal:"):
                memory_kib = int(re.findall(r"\d+", line)[0])
                break
    except OSError:
        pass
    return {
        "platform": platform.platform(),
        "python": sys.version.split()[0],
        "cpu_model": cpu_model,
        "logical_cpus": os.cpu_count(),
        "memory_mib": round(memory_kib / 1024, 1) if memory_kib else None,
        "rustc": command_version(["rustc", "--version"]),
        "cc": command_version([os.environ.get("CC", "cc"), "--version"]),
    }


def benchmark(manifest_path: Path) -> dict[str, object]:
    manifest = json.loads(manifest_path.read_text())
    cc = os.environ.get("CC", "cc")
    if shutil.which("cargo") is None or shutil.which(cc) is None:
        raise RuntimeError("King 20 requires cargo and a host C compiler")

    subprocess.run(["cargo", "build", "--release"], cwd=ROOT, check=True)
    compiler = ROOT / "target" / "release" / ("lpp.exe" if os.name == "nt" else "lpp")
    if not compiler.exists():
        raise RuntimeError(f"compiler missing at {compiler}")

    rows = []
    with tempfile.TemporaryDirectory(prefix="lpp-king20-") as temp_dir:
        temp = Path(temp_dir)
        for case in manifest["benchmarks"]:
            source = ROOT / case["source"]
            copied = temp / f"{case['id']:02d}_{source.name}"
            copied.write_text(source.read_text())
            env = os.environ.copy()
            env.update({"LPP_AOT": "1", "BENCHMARK": "1", "LPP_RELEASE": "1"})
            compile_run = subprocess.run(
                [str(compiler), str(copied)], env=env, text=True, capture_output=True
            )
            timing = next(
                (json.loads(line.split("TIMING_JSON: ", 1)[1])
                 for line in compile_run.stdout.splitlines()
                 if line.startswith("TIMING_JSON:")),
                None,
            )
            obj = copied.with_suffix(".o")
            if compile_run.returncode != 0 or timing is None or not obj.exists():
                raise RuntimeError(
                    f"#{case['id']} {case['name']} failed to emit AOT object:\n{compile_run.stderr}"
                )
            executable = temp / f"{case['id']:02d}_{case['name']}"
            link_start = time.perf_counter()
            subprocess.run(
                [cc, "-O2", str(obj), str(ROOT / "lpp_runtime.c"), "-o", str(executable), "-pthread"],
                check=True, capture_output=True, text=True,
            )
            link_seconds = time.perf_counter() - link_start
            run_start = time.perf_counter()
            execution = subprocess.run([str(executable)], text=True, capture_output=True)
            runtime_seconds = time.perf_counter() - run_start
            actual = execution.stdout.strip()
            expected = case["expected"]
            passed = execution.returncode == 0 and actual == expected
            rows.append({
                "id": case["id"], "name": case["name"], "source": case["source"],
                "expected": expected, "actual": actual, "exit_code": execution.returncode,
                "passed": passed, "compiler_ms": timing["total"] * 1000,
                "aot_ms": timing["aot"] * 1000, "link_ms": link_seconds * 1000,
                "runtime_ms": runtime_seconds * 1000, "object_bytes": obj.stat().st_size,
                "executable_bytes": executable.stat().st_size,
            })
            if not passed:
                raise RuntimeError(
                    f"#{case['id']} {case['name']} mismatch: expected {expected!r}, "
                    f"got {actual!r}, exit {execution.returncode}"
                )
    return {
        "suite": manifest["suite"], "version": manifest["version"],
        "generated_utc": datetime.now(timezone.utc).isoformat(),
        "system": system_info(), "rows": rows,
    }


def render_markdown(result: dict[str, object]) -> str:
    system = result["system"]
    lines = [
        "# L++ King 20 Benchmark Results", "",
        f"Generated: `{result['generated_utc']}`", "",
        "## System information", "",
        f"- Platform: `{system['platform']}`",
        f"- CPU: `{system['cpu_model']}`",
        f"- Logical CPUs: `{system['logical_cpus']}`",
        f"- Memory: `{system['memory_mib']} MiB`",
        f"- Rust: `{system['rustc']}`",
        f"- C compiler: `{system['cc']}`", "",
        "## Results", "",
        "Single-run development measurements. A result is recorded only after stdout and process exit status match the manifest.", "",
        "| # | Benchmark | Compiler ms | AOT ms | Link ms | Runtime ms | Object B | EXE B | Status |",
        "|---:|---|---:|---:|---:|---:|---:|---:|---|",
    ]
    for row in result["rows"]:
        lines.append(
            f"| {row['id']} | `{row['name']}` | {row['compiler_ms']:.3f} | "
            f"{row['aot_ms']:.3f} | {row['link_ms']:.3f} | {row['runtime_ms']:.3f} | "
            f"{row['object_bytes']} | {row['executable_bytes']} | PASS |"
        )
    lines += [
        "", "## Method", "",
        "Each source file is compiled with `LPP_AOT=1` and `BENCHMARK=1`, linked with the host C compiler and `lpp_runtime.c`, then executed once. The external link step is reported separately because Cranelift currently emits an object file; a host linker is still required for a standalone executable.",
    ]
    return "\n".join(lines) + "\n"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Run an L++ King 20 benchmark suite")
    parser.add_argument(
        "--suite", choices=("stable", "experimental"), default="experimental",
        help="stable runs frozen v1; experimental runs the evolving ownership suite",
    )
    options = parser.parse_args()
    if options.suite == "stable":
        manifest_path = SUITE_ROOT / "stable" / "v1" / "manifest.json"
        result_dir = SUITE_ROOT / "stable" / "v1"
    else:
        manifest_path = SUITE_ROOT / "experimental" / "manifest.json"
        result_dir = SUITE_ROOT / "experimental"

    result = benchmark(manifest_path)
    result_json = result_dir / "latest.json"
    result_md = result_dir / "latest.md"
    result_json.write_text(json.dumps(result, indent=2) + "\n")
    result_md.write_text(render_markdown(result))
    print(render_markdown(result))
