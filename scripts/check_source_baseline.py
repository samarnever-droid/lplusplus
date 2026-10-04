#!/usr/bin/env python3
"""Fail when L++ source validation differs from the reviewed baseline.

This replaces the old `lpp --checkall || true` CI step. Known failures are
explicitly classified in tests/source_baseline.json, so a new failure, changed
diagnostic, or silently fixed-but-unreviewed entry makes CI fail.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_MANIFEST = ROOT / "tests" / "source_baseline.json"
FAILURE_RE = re.compile(r"^\s{2}(.+?\.lpp):\d+:\d+:\s+(.+)$")
VALID_STATUSES = {"known-debt", "custom-dialect", "compile-fail"}


def normalize_path(raw: str) -> str:
    path = raw.strip().replace("\\", "/")
    while path.startswith("./"):
        path = path[2:]
    return path


def load_manifest(path: Path) -> dict:
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise SystemExit(f"cannot read source baseline {path}: {error}") from error

    if (
        manifest.get("schema") != 1
        or not isinstance(manifest.get("scopes"), list)
        or not isinstance(manifest.get("project_validators", []), list)
    ):
        raise SystemExit(f"unsupported source baseline schema in {path}")
    return manifest


def parse_failures(output: str) -> dict[str, str]:
    failures: dict[str, str] = {}
    for line in output.splitlines():
        match = FAILURE_RE.match(line)
        if match:
            path = normalize_path(match.group(1))
            if path in failures:
                raise SystemExit(f"compiler reported duplicate source failure for {path}")
            failures[path] = match.group(2)
    return failures


def validate_scope(compiler: Path, scope: dict) -> list[str]:
    name = scope.get("name")
    scope_path = scope.get("path")
    entries = scope.get("expected_failures")
    if not isinstance(name, str) or not isinstance(entries, list):
        return ["manifest scope is missing a name or expected_failures list"]

    command = [str(compiler), "--checkall"]
    if scope_path is not None:
        command.append(str(scope_path))

    completed = subprocess.run(
        command,
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        check=False,
    )
    output = completed.stdout
    print(f"\n=== source baseline: {name} ===")
    print(output, end="" if output.endswith("\n") else "\n")

    errors: list[str] = []
    expected: dict[str, dict] = {}
    for entry in entries:
        if not isinstance(entry, dict):
            errors.append(f"{name}: non-object baseline entry")
            continue
        path = normalize_path(str(entry.get("path", "")))
        status = entry.get("status")
        fingerprint = entry.get("contains")
        if not path or status not in VALID_STATUSES or not isinstance(fingerprint, str):
            errors.append(f"{name}: invalid baseline entry {entry!r}")
            continue
        if path in expected:
            errors.append(f"{name}: duplicate baseline path {path}")
            continue
        expected[path] = entry

    actual = parse_failures(output)
    expected_paths = set(expected)
    actual_paths = set(actual)

    for path in sorted(actual_paths - expected_paths):
        errors.append(f"{name}: NEW FAILURE {path}: {actual[path]}")
    for path in sorted(expected_paths - actual_paths):
        errors.append(
            f"{name}: baseline entry no longer fails: {path}; remove or reclassify it in "
            f"{DEFAULT_MANIFEST.relative_to(ROOT)}"
        )
    for path in sorted(expected_paths & actual_paths):
        fingerprint = expected[path]["contains"]
        if fingerprint not in actual[path]:
            errors.append(
                f"{name}: diagnostic drift for {path}\n"
                f"  expected substring: {fingerprint}\n"
                f"  actual: {actual[path]}"
            )

    if entries and completed.returncode == 0:
        errors.append(f"{name}: compiler returned success despite expected failures")
    if not entries and completed.returncode != 0:
        errors.append(f"{name}: compiler returned {completed.returncode} with no expected failures")

    statuses = Counter(entry["status"] for entry in expected.values())
    status_text = ", ".join(f"{key}={statuses[key]}" for key in sorted(statuses))
    if errors:
        print(f"FAIL {name}: source baseline drift", file=sys.stderr)
    else:
        print(f"PASS {name}: {len(actual)} explicitly classified failures ({status_text})")
    return errors


def validate_projects(manifest: dict) -> list[str]:
    errors: list[str] = []
    with tempfile.TemporaryDirectory(prefix="lpp-project-validation-") as temporary:
        for index, validator in enumerate(manifest.get("project_validators", [])):
            if not isinstance(validator, dict):
                errors.append("non-object project validator")
                continue
            name = validator.get("name")
            raw_command = validator.get("command")
            if (
                not isinstance(name, str)
                or not isinstance(raw_command, list)
                or not raw_command
                or not all(isinstance(argument, str) for argument in raw_command)
                or not any("{output}" in argument for argument in raw_command)
            ):
                errors.append(f"invalid project validator: {validator!r}")
                continue
            output = Path(temporary) / f"validator-{index}.out"
            command = [argument.replace("{output}", str(output)) for argument in raw_command]
            completed = subprocess.run(
                command,
                cwd=ROOT,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                check=False,
            )
            print(f"\n=== project validator: {name} ===")
            print(completed.stdout, end="" if completed.stdout.endswith("\n") else "\n")
            if completed.returncode != 0:
                errors.append(f"{name}: validator exited {completed.returncode}")
            elif not output.is_file() or output.stat().st_size == 0:
                errors.append(f"{name}: validator produced no output")
            else:
                print(f"PASS {name}: generated {output.stat().st_size} bytes")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "compiler",
        nargs="?",
        default=str(ROOT / "target" / "release" / "lpp"),
        help="path to the lpp compiler",
    )
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    args = parser.parse_args()

    compiler = Path(args.compiler)
    if not compiler.is_absolute():
        compiler = (Path.cwd() / compiler).resolve()
    if not compiler.is_file():
        print(f"compiler not found: {compiler}", file=sys.stderr)
        return 2

    manifest = load_manifest(args.manifest)
    all_errors = validate_projects(manifest)
    for scope in manifest["scopes"]:
        all_errors.extend(validate_scope(compiler, scope))

    if all_errors:
        print("\nSource baseline errors:", file=sys.stderr)
        for error in all_errors:
            print(f"- {error}", file=sys.stderr)
        return 1

    print("\nPASS: L++ source validation matches the reviewed baseline")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
