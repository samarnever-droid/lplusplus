#!/usr/bin/env python3
"""Verify that the reviewed L++ v1.2 compatibility fixtures did not drift."""

from __future__ import annotations

import argparse
import hashlib
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_HASHES = ROOT / "compatibility" / "v1.2.0" / "files.sha256"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--hashes", type=Path, default=DEFAULT_HASHES)
    args = parser.parse_args()

    try:
        lines = args.hashes.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        print(f"cannot read compatibility hashes: {error}", file=sys.stderr)
        return 2

    failures: list[str] = []
    checked = 0
    seen: set[str] = set()
    for number, line in enumerate(lines, 1):
        if not line or line.startswith("#"):
            continue
        try:
            expected, relative = line.split("  ", 1)
        except ValueError:
            failures.append(f"line {number}: expected '<sha256>  <path>'")
            continue
        if len(expected) != 64 or any(char not in "0123456789abcdef" for char in expected):
            failures.append(f"line {number}: invalid SHA-256 digest")
            continue
        if relative in seen:
            failures.append(f"line {number}: duplicate path {relative}")
            continue
        seen.add(relative)

        path = (ROOT / relative).resolve()
        try:
            path.relative_to(ROOT.resolve())
        except ValueError:
            failures.append(f"line {number}: path escapes repository: {relative}")
            continue
        if not path.is_file():
            failures.append(f"missing fixture: {relative}")
            continue
        actual = hashlib.sha256(path.read_bytes()).hexdigest()
        if actual != expected:
            failures.append(
                f"fixture changed: {relative}\n  expected {expected}\n  actual   {actual}"
            )
        checked += 1

    if failures:
        print("L++ v1.2 compatibility freeze FAILED:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        print(
            "Update compatibility hashes only after classifying and reviewing the behavior change.",
            file=sys.stderr,
        )
        return 1

    print(f"PASS: {checked} L++ v1.2 compatibility fixtures are unchanged")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
