#!/usr/bin/env python3
"""Run the runtime-minimum King 20 subset through lpp-link (no host final link)."""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
MANIFEST = HERE / "stable" / "v1" / "manifest.json"
# The freestanding runtime covers scalar output, ARC, and closures. List
# objects still need their own relocatable runtime-data implementation.
SUPPORTED_IDS = set(range(1, 21)) - {8, 17, 18}


def main() -> None:
    if not shutil.which("cargo") or not shutil.which("cc"):
        raise SystemExit("requires cargo and cc to package the freestanding runtime object")
    subprocess.run(["cargo", "build", "--release", "--bin", "lpp", "--bin", "lpp-link"], cwd=ROOT, check=True)
    lpp = ROOT / "target/release" / ("lpp.exe" if os.name == "nt" else "lpp")
    linker = ROOT / "target/release" / ("lpp-link.exe" if os.name == "nt" else "lpp-link")
    manifest = json.loads(MANIFEST.read_text())
    cases = [case for case in manifest["benchmarks"] if case["id"] in SUPPORTED_IDS]
    rows = []

    with tempfile.TemporaryDirectory(prefix="lpp-king20-direct-") as td:
        temp = Path(td)
        runtime = temp / "lpp_runtime_min.o"
        subprocess.run([
            "cc", "-O2", "-ffreestanding", "-fno-stack-protector", "-fno-pic", "-mno-red-zone",
            "-c", str(ROOT / "runtime" / "linux_x86_64_min.c"), "-o", str(runtime),
        ], check=True)
        for case in cases:
            source = temp / Path(case["source"]).name
            source.write_text((ROOT / case["source"]).read_text())
            env = os.environ.copy(); env.update({"LPP_AOT": "1", "LPP_RELEASE": "1"})
            started = time.perf_counter()
            subprocess.run([str(lpp), str(source)], env=env, check=True, capture_output=True)
            compile_ms = (time.perf_counter() - started) * 1000
            executable = temp / f"{case['id']:02d}_{case['name']}"
            started = time.perf_counter()
            subprocess.run([str(linker), str(source.with_suffix('.o')), str(runtime), "-o", str(executable)], check=True)
            link_ms = (time.perf_counter() - started) * 1000
            started = time.perf_counter()
            run = subprocess.run([str(executable)], text=True, capture_output=True)
            runtime_ms = (time.perf_counter() - started) * 1000
            passed = run.returncode == 0 and run.stdout.strip() == case["expected"]
            rows.append({"id": case["id"], "name": case["name"], "passed": passed,
                         "compile_ms": compile_ms, "direct_link_ms": link_ms, "runtime_ms": runtime_ms})
            if not passed:
                raise RuntimeError(f"#{case['id']} {case['name']} failed: {run.stdout!r}, exit={run.returncode}")

    result = {"generated_utc": datetime.now(timezone.utc).isoformat(), "rows": rows}
    (HERE / "direct_elf_latest.json").write_text(json.dumps(result, indent=2) + "\n")
    lines = ["# King 20 direct ELF subset", "", f"Generated: `{result['generated_utc']}`", "",
             "This report links without a host final linker. The freestanding runtime object is packaged before the run.", "",
             "| # | Workload | Compile ms | Direct link ms | Runtime ms | Status |",
             "|---:|---|---:|---:|---:|---|"]
    for row in rows:
        lines.append(f"| {row['id']} | `{row['name']}` | {row['compile_ms']:.3f} | {row['direct_link_ms']:.3f} | {row['runtime_ms']:.3f} | PASS |")
    (HERE / "direct_elf_latest.md").write_text("\n".join(lines) + "\n")
    print((HERE / "direct_elf_latest.md").read_text())


if __name__ == "__main__":
    main()
