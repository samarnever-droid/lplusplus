#!/usr/bin/env bash
set -euo pipefail

# Publish all monorepo packages to llppregistry in topological order.
# Requires git push credentials for https://github.com/samarnever-droid/llppregistry.git
# (or KEEL_REGISTRY pointing to your writable registry clone/remote).

KEEL_BIN="${KEEL_BIN:-keel}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [ -n "${GITHUBTOKENALL:-}" ] && [ -z "${KEEL_REGISTRY:-}" ]; then
  export KEEL_REGISTRY="https://x-access-token:${GITHUBTOKENALL}@github.com/samarnever-droid/llppregistry.git"
fi

LEAF_PACKAGES=(
  lpp-semver
  lpp-toml
  lpp-math
  lpp-graph
  lpp-zip
  lpp-zundo
  lpp-bindgen
  simple-markdown
  simple-pdf
  compresslpp
  lpp-analyzer
  lpp-engine
  lpp-git
  lppdb
  lppsqlite
  lppstore
  lreact
  lpp-collections
  c2lpp
)

for pkg in "${LEAF_PACKAGES[@]}"; do
  dir="$REPO_ROOT/packages/$pkg"
  if [ -f "$dir/Keel.toml" ]; then
    echo "==> Publishing $pkg..."
    (cd "$dir" && "$KEEL_BIN" publish) || echo "    (skipped or already published: $pkg)"
  fi
done
