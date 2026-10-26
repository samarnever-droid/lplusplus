#!/usr/bin/env python3
"""Measure profile-guided MIR inlining on a hot direct-call loop.

Run from any directory with:
  python3 benchmarks/pgo/run.py
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


def build(lpp: Path, source: Path, output: Path, *flags: str) -> None:
    subprocess.run(
        [str(lpp), str(source), "-O3", *flags, "--linker", "cc", "-o", str(output)],
        cwd=ROOT,
        check=True,
        stdout=subprocess.DEVNULL,
    )


def sample(executable: Path, count: int) -> tuple[list[float], str]:
    timings: list[float] = []
    output = ""
    for _ in range(count):
        start = time.perf_counter()
        run = subprocess.run([str(executable)], capture_output=True, text=True, check=True)
        timings.append(time.perf_counter() - start)
        output = run.stdout.strip()
    return timings, output


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lpp", type=Path, default=ROOT / "target/debug/lpp")
    parser.add_argument("--samples", type=int, default=7)
    args = parser.parse_args()
    if args.samples < 3:
        parser.error("use at least 3 timing samples")
    lpp = args.lpp if args.lpp.is_absolute() else ROOT / args.lpp
    subprocess.run(["cargo", "build", "--locked", "-p", "lpp-runtime"], cwd=ROOT, check=True)
    if not lpp.is_file():
        subprocess.run(["cargo", "build", "--locked", "--bin", "lpp"], cwd=ROOT, check=True)
    lpp = lpp.resolve()
    source = ROOT / "benchmarks/pgo/pgo_loop.lpp"

    with tempfile.TemporaryDirectory(prefix="lpp-pgo-bench-") as temporary:
        work = Path(temporary)
        profile = work / "hot.pgo"
        instrumented = work / "instrumented"
        baseline = work / "baseline"
        optimized = work / "pgo-use"

        build(lpp, source, instrumented, "--pgo-generate")
        profile_env = os.environ.copy()
        profile_env["LPP_PGO_PROFILE"] = str(profile)
        generation_start = time.perf_counter()
        subprocess.run([str(instrumented)], cwd=ROOT, env=profile_env, check=True,
                       stdout=subprocess.DEVNULL)
        profile_generation_ms = (time.perf_counter() - generation_start) * 1000
        if not profile.is_file():
            raise SystemExit("instrumented run did not write its PGO profile")
        profile_records = sum(1 for line in profile.read_text().splitlines()[1:] if line.strip())

        build(lpp, source, baseline)
        build(lpp, source, optimized, "--pgo-use", str(profile))
        baseline_samples, baseline_output = sample(baseline, args.samples)
        pgo_samples, pgo_output = sample(optimized, args.samples)

    if baseline_output != pgo_output:
        raise SystemExit(f"semantic mismatch: baseline={baseline_output}, pgo={pgo_output}")
    print(f"host={os.uname().machine if hasattr(os, 'uname') else 'unknown'} samples={args.samples}")
    print(f"output={baseline_output}")
    print(f"profile_records={profile_records}")
    print(f"instrumented_profile_generation_ms={profile_generation_ms:.3f}")
    for label, samples in [("without PGO", baseline_samples), ("with PGO", pgo_samples)]:
        print(
            f"{label}: median_ms={statistics.median(samples) * 1000:.3f} "
            f"min_ms={min(samples) * 1000:.3f}"
        )
    print(f"median_speedup={statistics.median(baseline_samples) / statistics.median(pgo_samples):.2f}x")


if __name__ == "__main__":
    main()
