#!/usr/bin/env python3
"""Measure phase-by-phase L++ compiler scaling at 10k, 50k, and 100k LOC."""
from __future__ import annotations

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
HERE = Path(__file__).resolve().parent
TARGETS = (10_000, 50_000, 100_000)
PHASES = ("io", "lex", "parse", "semantic", "typecheck", "escape", "mir", "aot", "total")


def command_version(command: list[str]) -> str:
    try:
        return subprocess.check_output(command, text=True, stderr=subprocess.STDOUT).splitlines()[0]
    except (OSError, subprocess.CalledProcessError):
        return "unavailable"


def sysinfo() -> dict[str, object]:
    cpu = "unknown"
    memory_mib = None
    try:
        for line in Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith("model name"):
                cpu = line.split(":", 1)[1].strip()
                break
        for line in Path("/proc/meminfo").read_text().splitlines():
            if line.startswith("MemTotal:"):
                memory_mib = round(int(re.findall(r"\d+", line)[0]) / 1024, 1)
                break
    except OSError:
        pass
    return {
        "platform": platform.platform(), "cpu": cpu, "logical_cpus": os.cpu_count(),
        "memory_mib": memory_mib, "rustc": command_version(["rustc", "--version"]),
        "cc": command_version([os.environ.get("CC", "cc"), "--version"]),
    }


def main() -> None:
    cc = os.environ.get("CC", "cc")
    if not shutil.which("cargo") or not shutil.which(cc):
        raise SystemExit("requires cargo and a host C compiler")
    subprocess.run([sys.executable, str(HERE / "generate.py")], check=True)
    subprocess.run(["cargo", "build", "--release"], cwd=ROOT, check=True)
    compiler = ROOT / "target" / "release" / ("lpp.exe" if os.name == "nt" else "lpp")
    generated = HERE / "generated"
    rows = []

    with tempfile.TemporaryDirectory(prefix="lpp-scale-") as temp_dir:
        temp = Path(temp_dir)
        for lines in TARGETS:
            source = generated / f"scale_{lines}.lpp"
            copied = temp / source.name
            copied.write_text(source.read_text())
            env = os.environ.copy()
            env.update({"LPP_AOT": "1", "LPP_RELEASE": "1", "BENCHMARK": "1"})
            compile_result = subprocess.run([str(compiler), str(copied)], env=env, text=True, capture_output=True)
            timing = next((json.loads(line.split("TIMING_JSON: ", 1)[1])
                           for line in compile_result.stdout.splitlines()
                           if line.startswith("TIMING_JSON:")), None)
            obj = copied.with_suffix(".o")
            if compile_result.returncode != 0 or timing is None or not obj.exists():
                raise RuntimeError(f"{lines} LOC compile failed:\n{compile_result.stderr}")
            executable = temp / f"scale_{lines}"
            started = time.perf_counter()
            subprocess.run([cc, "-O2", str(obj), str(ROOT / "lpp_runtime.c"), "-o", str(executable), "-pthread"], check=True, capture_output=True)
            link_ms = (time.perf_counter() - started) * 1000
            run = subprocess.run([str(executable)], text=True, capture_output=True)
            expected = str(lines - 4)
            if run.returncode != 0 or run.stdout.strip() != expected:
                raise RuntimeError(f"{lines} LOC output mismatch: {run.stdout!r}, exit={run.returncode}")
            row = {"loc": lines, "link_ms": link_ms, "object_bytes": obj.stat().st_size,
                   "executable_bytes": executable.stat().st_size}
            row.update({phase: timing[phase] * 1000 for phase in PHASES})
            rows.append(row)

    result = {"generated_utc": datetime.now(timezone.utc).isoformat(), "system": sysinfo(), "rows": rows}
    (HERE / "latest.json").write_text(json.dumps(result, indent=2) + "\n")
    markdown = [
        "# L++ Compiler Scalability", "",
        f"Generated: `{result['generated_utc']}`", "",
        "## System", "",
        f"- Platform: `{result['system']['platform']}`",
        f"- CPU: `{result['system']['cpu']}`",
        f"- Logical CPUs: `{result['system']['logical_cpus']}`",
        f"- Memory: `{result['system']['memory_mib']} MiB`", "",
        "## Phase scaling", "",
        "Single-run development measurements. Link time is reported separately because it is dominated by the host linker.", "",
        "| LOC | I/O ms | Lex ms | Parse ms | Semantic ms | Typecheck ms | Escape ms | MIR ms | AOT ms | Compiler total ms | Link ms |", 
        "|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|",
    ]
    for row in rows:
        markdown.append(
            "| {loc} | {io:.3f} | {lex:.3f} | {parse:.3f} | {semantic:.3f} | {typecheck:.3f} | {escape:.3f} | {mir:.3f} | {aot:.3f} | {total:.3f} | {link_ms:.3f} |".format(**row)
        )
    (HERE / "latest.md").write_text("\n".join(markdown) + "\n")
    print((HERE / "latest.md").read_text())


if __name__ == "__main__":
    main()
