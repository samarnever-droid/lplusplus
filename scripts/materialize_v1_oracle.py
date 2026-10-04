#!/usr/bin/env python3
"""Materialize a verified v1.2.0 compatibility-oracle compiler.

By default this checks out the exact frozen commit, verifies Cargo.lock, builds
with the pinned Rust toolchain, and writes a SHA-256 sidecar. A prebuilt binary
is accepted only when both its URL and trusted SHA-256 digest are supplied.
"""

from __future__ import annotations

import argparse
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
import tomllib
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ORACLE = ROOT / "compatibility" / "v1.2.0" / "oracle.toml"


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def load_oracle() -> dict:
    data = tomllib.loads(ORACLE.read_text(encoding="utf-8"))
    required = ("commit", "repository", "cargo_lock_sha256", "rust_toolchain")
    if data.get("schema") != 1 or any(not isinstance(data.get(key), str) for key in required):
        raise ValueError(f"invalid oracle metadata: {ORACLE}")
    return data


def run(command: list[str], *, cwd: Path | None = None, environment: dict[str, str] | None = None) -> None:
    print("+", " ".join(command), flush=True)
    subprocess.run(command, cwd=cwd, env=environment, check=True)


def write_verified_binary(source: Path, destination: Path, expected: str | None = None) -> None:
    actual = sha256_file(source)
    if expected is not None and actual.lower() != expected.lower():
        raise ValueError(f"oracle binary SHA-256 mismatch: expected {expected.lower()}, got {actual}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(destination.name + ".tmp")
    shutil.copyfile(source, temporary)
    temporary.chmod(0o755)
    os.replace(temporary, destination)
    destination.with_name(destination.name + ".sha256").write_text(
        f"{actual}  {destination.name}\n", encoding="ascii"
    )
    print(f"PASS: oracle binary {destination}")
    print(f"SHA-256: {actual}")


def download_binary(url: str, expected: str, destination: Path) -> None:
    if len(expected) != 64 or any(character not in "0123456789abcdefABCDEF" for character in expected):
        raise ValueError("--binary-sha256 must be exactly 64 hexadecimal characters")
    with tempfile.TemporaryDirectory(prefix="lpp-v1-oracle-download-") as temporary:
        downloaded = Path(temporary) / "lpp-oracle"
        request = urllib.request.Request(url, headers={"User-Agent": "lplusplus-oracle-materializer/1"})
        with urllib.request.urlopen(request, timeout=60) as response, downloaded.open("wb") as output:
            shutil.copyfileobj(response, output)
        write_verified_binary(downloaded, destination, expected)


def build_binary(metadata: dict, destination: Path, cargo: str) -> None:
    commit = metadata["commit"]
    if len(commit) != 40 or any(character not in "0123456789abcdef" for character in commit):
        raise ValueError("oracle commit must be a full lowercase 40-character Git object ID")

    with tempfile.TemporaryDirectory(prefix="lpp-v1-oracle-build-") as temporary:
        checkout = Path(temporary) / "source"
        checkout.mkdir()
        run(["git", "init", "--quiet"], cwd=checkout)
        run(["git", "remote", "add", "origin", metadata["repository"]], cwd=checkout)
        run(["git", "fetch", "--quiet", "--depth=1", "origin", commit], cwd=checkout)
        run(["git", "checkout", "--quiet", "--detach", "FETCH_HEAD"], cwd=checkout)
        actual_commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=checkout, text=True).strip()
        if actual_commit != commit:
            raise ValueError(f"oracle checkout mismatch: expected {commit}, got {actual_commit}")
        actual_lock = sha256_file(checkout / "Cargo.lock")
        if actual_lock != metadata["cargo_lock_sha256"]:
            raise ValueError(
                "oracle Cargo.lock mismatch: "
                f"expected {metadata['cargo_lock_sha256']}, got {actual_lock}"
            )

        environment = os.environ.copy()
        environment["RUSTUP_TOOLCHAIN"] = metadata["rust_toolchain"]
        environment["CARGO_INCREMENTAL"] = "0"
        environment["SOURCE_DATE_EPOCH"] = subprocess.check_output(
            ["git", "show", "-s", "--format=%ct", "HEAD"], cwd=checkout, text=True
        ).strip()
        rustc_version = subprocess.check_output(
            ["rustc", "--version"], env=environment, text=True
        ).split()
        if len(rustc_version) < 2 or rustc_version[1] != metadata["rust_toolchain"]:
            raise ValueError(
                f"oracle build requires rustc {metadata['rust_toolchain']}, got {' '.join(rustc_version)}"
            )
        run([cargo, "build", "--release", "--locked", "--bin", "lpp"], cwd=checkout, environment=environment)
        executable = checkout / "target" / "release" / ("lpp.exe" if os.name == "nt" else "lpp")
        write_verified_binary(executable, destination)


def main() -> int:
    default_name = "lpp-v1.2.0-oracle.exe" if os.name == "nt" else "lpp-v1.2.0-oracle"
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "target" / "compatibility" / default_name)
    parser.add_argument("--cargo", default="cargo", help="cargo/rustup proxy used for source builds")
    parser.add_argument("--binary-url", help="URL of a prebuilt oracle binary")
    parser.add_argument("--binary-sha256", help="trusted SHA-256 for --binary-url")
    args = parser.parse_args()

    if bool(args.binary_url) != bool(args.binary_sha256):
        parser.error("--binary-url and --binary-sha256 must be supplied together")
    try:
        metadata = load_oracle()
        if args.binary_url:
            download_binary(args.binary_url, args.binary_sha256, args.output.resolve())
        else:
            build_binary(metadata, args.output.resolve(), args.cargo)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"materialize_v1_oracle.py: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
