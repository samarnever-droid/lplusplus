#!/usr/bin/env python3
"""Validate the version-controlled L++ wiki before publishing it."""
from __future__ import annotations
import re
import sys
from pathlib import Path

root = Path(__file__).resolve().parents[1]
wiki = root / "wiki"
errors: list[str] = []
pages = {p.stem for p in wiki.glob("*.md")}
if "Home" not in pages:
    errors.append("wiki/Home.md is required by GitHub Wiki")

link = re.compile(r"\[\[([^\]|]+)(?:\|[^\]]+)?\]\]")
for page in sorted(wiki.glob("*.md")):
    text = page.read_text(encoding="utf-8")
    for target in link.findall(text):
        # GitHub Wiki names pages by basename; anchors may follow '#'.
        target = target.strip().split("#", 1)[0]
        if target and target not in pages:
            errors.append(f"{page.relative_to(root)} links to missing wiki page [[{target}]]")

for required in ("README.md", "Home.md", "Networking.md", "Roadmap.md"):
    if not (wiki / required).is_file():
        errors.append(f"missing required wiki page: wiki/{required}")

if errors:
    print("Wiki validation failed:", file=sys.stderr)
    print("\n".join(f"- {error}" for error in errors), file=sys.stderr)
    raise SystemExit(1)
print(f"Wiki validation passed: {len(pages)} pages")
