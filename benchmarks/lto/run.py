#!/usr/bin/env python3
"""Measure the bounded whole-program MIR inliner on a direct-call loop.

Run from any directory with:
  python3 benchmarks/lto/run.py
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

    source = ROOT / "benchmarks/lto/inline_loop.lpp"
    with tempfile.TemporaryDirectory(prefix="lpp-lto-bench-") as temporary:
        work = Path(temporary)
        programs: dict[str, Path] = {}
        for label, flags in [("without LTO", []), ("with MIR LTO", ["--lto"])]:
            output = work / ("baseline" if not flags else "lto")
            subprocess.run(
                [str(lpp.resolve()), str(source), "-O3", *flags, "--linker", "cc", "-o", str(output)],
                cwd=ROOT,
                check=True,
                stdout=subprocess.DEVNULL,
            )
            programs[label] = output

        samples_by_label: dict[str, list[float]] = {}
        outputs: dict[str, str] = {}
        for label, output in programs.items():
            samples: list[float] = []
            for _ in range(args.samples):
                start = time.perf_counter()
                run = subprocess.run([str(output)], capture_output=True, text=True, check=True)
                samples.append(time.perf_counter() - start)
                outputs[label] = run.stdout.strip()
            samples_by_label[label] = samples

    if outputs["without LTO"] != outputs["with MIR LTO"]:
        raise SystemExit(f"semantic mismatch: {outputs}")
    print(f"host={os.uname().machine if hasattr(os, 'uname') else 'unknown'} samples={args.samples}")
    print(f"output={outputs['without LTO']}")
    for label, samples in samples_by_label.items():
        print(
            f"{label}: median_ms={statistics.median(samples) * 1000:.3f} "
            f"min_ms={min(samples) * 1000:.3f}"
        )
    baseline = statistics.median(samples_by_label["without LTO"])
    lto = statistics.median(samples_by_label["with MIR LTO"])
    print(f"median_speedup={baseline / lto:.2f}x")


if __name__ == "__main__":
    main()
