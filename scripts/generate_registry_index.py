#!/usr/bin/env python3
"""Generate the public L++ registry mirror index.

Canonical package registry writes are git commits containing sparse `index/*`
entries and content-addressed `blob/<sha256>` artifacts. This script generates
one website/HTTP-friendly aggregate JSON document from that canonical layout.

For the current repository, which still carries the older aggregate
`registry/index.json`, the script also validates and normalizes that file, then
copies it to `website/public/registry/index.json` so the website and Worker read
one identical static index.
"""

from __future__ import annotations

import json
import pathlib
import sys
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[1]
SPARSE_INDEX = ROOT / "registry" / "index"
AGGREGATE_INDEX = ROOT / "registry" / "index.json"
WEBSITE_INDEX = ROOT / "website" / "public" / "registry" / "index.json"
REGISTRY_URL = "https://registry.lplusplus.bond"
WEBSITE_INDEX_URL = "https://lplusplus.bond/registry/index.json"


def load_json(path: pathlib.Path) -> Any:
    try:
        return json.loads(path.read_text())
    except json.JSONDecodeError as exc:
        raise SystemExit(f"{path}: invalid JSON: {exc}") from exc


def sparse_to_manifest() -> dict[str, Any]:
    packages: dict[str, Any] = {}
    if not SPARSE_INDEX.exists():
        return {}

    for path in sorted(p for p in SPARSE_INDEX.rglob("*") if p.is_file()):
        entry = load_json(path)
        name = entry.get("name")
        if not isinstance(name, str) or not name:
            raise SystemExit(f"{path}: missing package name")
        versions = entry.get("versions", [])
        latest = versions[-1] if versions else {}
        packages[name] = {
            "name": name,
            "version": latest.get("version", "0.0.0"),
            "versions": {
                version.get("version", "0.0.0"): {
                    **version,
                    "download_url": f"{REGISTRY_URL}/blob/{version.get('checksum', version.get('sha256', ''))}",
                }
                for version in versions
            },
            "description": entry.get("description", ""),
            "authors": entry.get("authors", []),
            "license": entry.get("license", ""),
            "keywords": entry.get("keywords", []),
        }
    return {"packages": packages}


def normalize_manifest(manifest: dict[str, Any]) -> dict[str, Any]:
    packages = manifest.get("packages") or manifest.get("results") or {}
    if isinstance(packages, list):
        packages = {pkg["name"]: pkg for pkg in packages if isinstance(pkg, dict) and pkg.get("name")}
    if not isinstance(packages, dict):
        raise SystemExit("registry manifest must contain packages as an object or list")

    for name, pkg in sorted(packages.items()):
        if not isinstance(pkg, dict):
            raise SystemExit(f"package {name}: entry must be an object")
        pkg.setdefault("name", name)
        if pkg["name"] != name:
            raise SystemExit(f"package key {name!r} does not match package name {pkg['name']!r}")
        if "versions" in pkg and not isinstance(pkg["versions"], dict):
            raise SystemExit(f"package {name}: versions must be an object when present")

    return {
        "registry": {
            "name": "L++ Official Package Registry",
            "version": "3.0.0",
            "url": REGISTRY_URL,
            "static_index_url": WEBSITE_INDEX_URL,
            "source_of_truth": "git",
            "description": "Official L++ package registry — git-backed, static-mirrored, SHA-256 verified",
            "package_count": len(packages),
        },
        "packages": {name: packages[name] for name in sorted(packages)},
    }


def main() -> int:
    sparse_manifest = sparse_to_manifest()
    if sparse_manifest:
        manifest = sparse_manifest
    elif AGGREGATE_INDEX.exists():
        manifest = load_json(AGGREGATE_INDEX)
    else:
        raise SystemExit("no registry/index sparse tree or registry/index.json found")

    normalized = normalize_manifest(manifest)
    text = json.dumps(normalized, indent=2, sort_keys=True, ensure_ascii=False) + "\n"
    AGGREGATE_INDEX.parent.mkdir(parents=True, exist_ok=True)
    WEBSITE_INDEX.parent.mkdir(parents=True, exist_ok=True)
    AGGREGATE_INDEX.write_text(text)
    WEBSITE_INDEX.write_text(text)
    print(f"generated {AGGREGATE_INDEX.relative_to(ROOT)} and {WEBSITE_INDEX.relative_to(ROOT)} ({len(normalized['packages'])} packages)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
