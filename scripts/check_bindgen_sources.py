#!/usr/bin/env python3
"""Compile the complete lpp-bindgen source graph with the current lpp driver.

The Rust workspace tests do not type-check package-local L++ sources. The
bindgen package's entry module imports most of its sources; small runtime
modules are emitted as text and are intentionally not part of that graph. This
script compiles the entry graph plus every otherwise-unreachable source module,
so a package-only type/ownership regression fails the ordinary fast gate.
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCE_DIR = ROOT / "packages" / "lpp-bindgen" / "src"
IMPORT = re.compile(r"^\s*import\s+([A-Za-z_][A-Za-z0-9_]*)\s*(?:#.*)?$", re.MULTILINE)


def driver() -> str:
    configured = os.environ.get("LPP")
    if configured:
        candidate = Path(configured)
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return str(candidate.resolve())
        resolved = shutil.which(configured)
        if resolved:
            return resolved
        raise SystemExit(f"lpp-bindgen compile gate: LPP is not executable or on PATH: {configured}")

    for candidate in (ROOT / "target/debug/lpp", ROOT / "target/release/lpp"):
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return str(candidate)
    resolved = shutil.which("lpp")
    if resolved:
        return resolved
    raise SystemExit("lpp-bindgen compile gate: build `lpp` or set LPP to its executable")


def import_graph() -> tuple[dict[str, Path], dict[str, set[str]]]:
    modules = {path.stem: path for path in SOURCE_DIR.glob("*.lpp")}
    if "main" not in modules:
        raise SystemExit(f"lpp-bindgen compile gate: missing entry source {SOURCE_DIR / 'main.lpp'}")
    graph: dict[str, set[str]] = {}
    for name, path in modules.items():
        graph[name] = {
            imported
            for imported in IMPORT.findall(path.read_text(encoding="utf-8"))
            if imported in modules
        }
    return modules, graph


def roots_for(modules: dict[str, Path], graph: dict[str, set[str]]) -> list[str]:
    reachable: set[str] = set()
    pending = ["main"]
    while pending:
        module = pending.pop()
        if module in reachable:
            continue
        reachable.add(module)
        pending.extend(sorted(graph[module] - reachable, reverse=True))
    return ["main", *sorted(set(modules) - reachable)]


def main() -> int:
    lpp = driver()
    modules, graph = import_graph()
    roots = roots_for(modules, graph)
    for root in roots:
        path = modules[root]
        result = subprocess.run(
            [lpp, str(path), "--check"],
            cwd=ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if result.returncode != 0:
            print(f"FAIL lpp-bindgen compile graph rooted at {path.relative_to(ROOT)}", file=sys.stderr)
            if result.stdout:
                print(result.stdout, end="", file=sys.stderr)
            if result.stderr:
                print(result.stderr, end="", file=sys.stderr)
            return result.returncode
    print(
        "PASS lpp-bindgen source compilation: "
        f"{len(modules)} modules across {len(roots)} entry graphs"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
