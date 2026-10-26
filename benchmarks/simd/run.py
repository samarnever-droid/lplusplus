#!/usr/bin/env python3
"""Compare L++'s runtime-dispatched checksum intrinsic with a scalar loop.

Run from any directory:
  python3 benchmarks/simd/run.py
  python3 benchmarks/simd/run.py --lpp target/debug/lpp --runtime target/release/liblpp_runtime.so

The benchmark compiles both programs with Cranelift -O3, links against the
optimized Rust runtime, checks identical output, then measures process wall time.
"""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import statistics
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]


def build_if_missing(executable: Path, package: str | None = None) -> None:
    if executable.is_file():
        return
    command = ["cargo", "build", "--locked"]
    if package:
        command += ["--release", "-p", package]
    else:
        command += ["--bin", "lpp"]
    subprocess.run(command, cwd=ROOT, check=True)


def avx2_available() -> bool | None:
    cpuinfo = Path("/proc/cpuinfo")
    if not cpuinfo.exists():
        return None
    return "avx2" in cpuinfo.read_text(errors="ignore").lower()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lpp", type=Path, default=ROOT / "target/debug/lpp")
    parser.add_argument(
        "--runtime", type=Path, default=ROOT / "target/release/liblpp_runtime.so"
    )
    parser.add_argument("--samples", type=int, default=9)
    args = parser.parse_args()
    if args.samples < 3:
        parser.error("use at least 3 timing samples")

    lpp = args.lpp if args.lpp.is_absolute() else ROOT / args.lpp
    runtime = args.runtime if args.runtime.is_absolute() else ROOT / args.runtime
    build_if_missing(lpp)
    build_if_missing(runtime, "lpp-runtime")
    if not lpp.is_file() or not runtime.is_file():
        raise SystemExit("compiler or Rust runtime was not produced")

    compile_env = os.environ.copy()
    compile_env["LPP_RUNTIME_LIB"] = str(runtime.resolve())
    sources = {
        "SIMD runtime dispatch": ROOT / "benchmarks/simd/checksum_builtin.lpp",
        "scalar loop": ROOT / "benchmarks/simd/checksum_scalar.lpp",
    }
    with tempfile.TemporaryDirectory(prefix="lpp-simd-bench-") as temporary:
        work = Path(temporary)
        executables: dict[str, Path] = {}
        for label, source in sources.items():
            executable = work / ("simd" if label.startswith("SIMD") else "scalar")
            subprocess.run(
                [str(lpp.resolve()), str(source), "-O3", "--linker", "cc", "-o", str(executable)],
                cwd=ROOT,
                env=compile_env,
                check=True,
                stdout=subprocess.DEVNULL,
            )
            executables[label] = executable

        measurements: dict[str, list[float]] = {}
        outputs: dict[str, str] = {}
        for label, executable in executables.items():
            samples: list[float] = []
            for _ in range(args.samples):
                start = time.perf_counter()
                run = subprocess.run(
                    [str(executable)], capture_output=True, text=True, check=True
                )
                samples.append(time.perf_counter() - start)
                outputs[label] = run.stdout.strip()
            measurements[label] = samples

    if outputs["SIMD runtime dispatch"] != outputs["scalar loop"]:
        raise SystemExit(f"semantic mismatch: {outputs}")

    avx2 = avx2_available()
    print(f"host={os.uname().machine if hasattr(os, 'uname') else 'unknown'} avx2={avx2}")
    print(f"runtime={runtime.resolve()}")
    print(f"samples={args.samples} output={outputs['scalar loop']}")
    for label, samples in measurements.items():
        print(
            f"{label}: median_ms={statistics.median(samples) * 1000:.3f} "
            f"min_ms={min(samples) * 1000:.3f}"
        )
    scalar = statistics.median(measurements["scalar loop"])
    vector = statistics.median(measurements["SIMD runtime dispatch"])
    print(f"median_speedup={scalar / vector:.2f}x")


if __name__ == "__main__":
    main()
