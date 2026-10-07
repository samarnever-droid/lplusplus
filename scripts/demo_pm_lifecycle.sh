#!/usr/bin/env bash
# L++ PM lifecycle demo — the full Keel + lpp system against a git registry.
#
# Usage:  bash scripts/demo_pm_lifecycle.sh
#
# Requires: prebuilt `lpp` + `keel` (cargo build), git, tar, cc, python3.
# Works against the LOCAL bare registry by default; point KEEL_REGISTRY at
# any git URL (GitHub/Gitea) with write access to demo a real host.
#
# The demo exercises, in order:
#   registry setup → publish → new → add → fetch → run (registry import!)
#   → tree → outdated → publish bump → outdated → update → run (new code)
#   → why → verify → TAMPER the registry → verify (must FAIL) → restore
#
# Exit code 0 = every step behaved as expected.
set -euo pipefail

ROOT=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
LPP=${LPP_BIN:-$ROOT/target/debug/lpp}
KEEL=${KEEL_BIN:-$ROOT/target/debug/keel}
REG=${KEEL_REGISTRY:-}   # empty = local bare registry in the temp dir

for tool in git tar cc python3; do
  command -v "$tool" >/dev/null 2>&1 || { echo "missing tool: $tool" >&2; exit 2; }
done
[ -x "$LPP" ] || { echo "missing lpp binary: $LPP (run: cargo build)" >&2; exit 2; }
[ -x "$KEEL" ] || { echo "missing keel binary: $KEEL (run: cargo build)" >&2; exit 2; }

T=$(mktemp -d "${TMPDIR:-/tmp}/lpp-lifecycle.XXXXXX")
export XDG_CACHE_HOME="$T/cache"
trap 'rm -rf "$T"' EXIT
cd "$T"

step() { printf '\n\033[1;36m== %s ==\033[0m\n' "$*"; }
say()  { printf '   %s\n' "$*"; }
expect_grep() { # $1=expected  $2=label
  if ! printf '%s' "$STDOUT" | grep -qF "$1"; then
    echo "FAILED: $2 — expected output to contain: $1" >&2
    printf '%s\n' "$STDOUT" >&2
    exit 1
  fi
}

# --------------------------------------------------------------------------
step "1. git registry"
if [ -z "$REG" ]; then
  mkdir work && cd work && git init -q -b main . && git config user.name demo \
    && git config user.email demo@lpp.local && git commit -q --allow-empty -m init \
    && git init -q --bare "$T/remote.git" --initial-branch=main \
    && git push -q "$T/remote.git" HEAD:main && cd "$T"
  REG="$T/remote.git"
  say "local bare registry: $REG"
else
  say "using external registry: $REG"
fi
export PATH="$ROOT/target/debug:$PATH"   # keel finds `lpp` via PATH
K() { "$KEEL" --registry "$REG" "$@"; }

# --------------------------------------------------------------------------
step "2. publish mathx 1.0.0 (a real git commit to the registry)"
mkdir mathx && cd mathx
printf '[package]\nname = "mathx"\nversion = "1.0.0"\n' > Keel.toml
mkdir src
printf 'def double(x: Int) -> Int:\n    return x * 2\n' > src/lib.lpp
K publish
cd "$T"

# --------------------------------------------------------------------------
step "3. keel new app + keel add mathx + keel fetch"
K new app
cd app
K add mathx
K fetch
cd "$T"

# --------------------------------------------------------------------------
step "4. keel run — the app imports the REGISTRY package (staging!)"
printf 'import mathx\n\ndef main():\n    print(mathx.double(21))\n' > app/src/main.lpp
cd app
STDOUT=$(K run)
cd "$T"
expect_grep "42" "first run must print 42"
say "run printed: $(printf '%s' "$STDOUT" | grep -v '^$' | tail -1)"

# --------------------------------------------------------------------------
step "5. keel tree (lock-driven, offline)"
cd app
STDOUT=$(K tree)
cd "$T"
expect_grep "mathx v1.0.0 (registry)" "tree must show the locked registry dep"
printf '%s\n' "$STDOUT"

# --------------------------------------------------------------------------
step "6. keel outdated (nothing new yet)"
cd app
STDOUT=$(K outdated)
cd "$T"
expect_grep "up to date" "everything is up to date"
printf '%s\n' "$STDOUT"

# --------------------------------------------------------------------------
step "7. publish mathx 1.1.0 (double now adds 100)"
mkdir "$T/mathx11" && cd "$T/mathx11"
printf '[package]\nname = "mathx"\nversion = "1.1.0"\n' > Keel.toml
mkdir src
printf 'def double(x: Int) -> Int:\n    return x * 2 + 100\n' > src/lib.lpp
K publish
cd "$T"

# --------------------------------------------------------------------------
step "8. keel outdated (an update is available now)"
cd app
STDOUT=$(K outdated)
cd "$T"
expect_grep "update available" "outdated must flag 1.1.0"
expect_grep "1.1.0" "latest column shows 1.1.0"
printf '%s\n' "$STDOUT"

# --------------------------------------------------------------------------
step "9. keel update (diff table + lock refresh)"
cd app
STDOUT=$(K update)
cd "$T"
expect_grep "1.0.0" "diff shows the old version"
expect_grep "1.1.0" "diff shows the new version"
printf '%s\n' "$STDOUT"

# --------------------------------------------------------------------------
step "10. keel run again — the NEW version is staged and linked"
cd app
STDOUT=$(K run)
cd "$T"
expect_grep "142" "second run must print 142 (21*2+100)"
say "run printed: $(printf '%s' "$STDOUT" | grep -v '^$' | tail -1)"

# --------------------------------------------------------------------------
step "11. keel why mathx (dependency chains)"
cd app
STDOUT=$(K why mathx)
cd "$T"
expect_grep "mathx ← app (direct)" "why must show the direct chain"
printf '%s\n' "$STDOUT"

# --------------------------------------------------------------------------
step "12. keel verify (supply-chain audit — honest registry)"
cd app
STDOUT=$(K verify)
cd "$T"
expect_grep "verified OK" "verify must pass on the honest registry"
printf '%s\n' "$STDOUT"

# --------------------------------------------------------------------------
step "13. TAMPER the registry: replace mathx bytes under its locked checksum"
python3 - "$T" "$REG" <<'EOF'
import subprocess, sys, os, json, glob, hashlib, re
T, REG = sys.argv[1], sys.argv[2]
clone = os.path.join(T, "attacker")
subprocess.run(["git", "clone", "-q", REG, clone], check=True)
subprocess.run(["git", "config", "user.name", "attacker"], cwd=clone, check=True)
subprocess.run(["git", "config", "user.email", "a@e.f"], cwd=clone, check=True)
idx = glob.glob(os.path.join(clone, "index", "**", "mathx*"), recursive=True)
assert idx, "mathx index not found"
idx = idx[0]
doc = json.load(open(idx))
# Tamper the version the app is LOCKED to (from app/Keel.lock).
lock = open(os.path.join(T, "app", "Keel.lock")).read()
m = re.search(r'name = "mathx"\nversion = "([^"\n]+)"', lock)
locked = m.group(1)
v = next(v for v in doc["versions"] if v["version"] == locked)
evil = b"def double(x: Int) -> Int:\n    return 0  # attacker\n"
v["checksum"] = hashlib.sha256(evil).hexdigest()
json.dump(doc, open(idx, "w"))
blob = os.path.join(clone, "blob", v["checksum"])
open(blob, "wb").write(evil)
subprocess.run(["git", "add", "index", "blob"], cwd=clone, check=True)
subprocess.run(["git", "commit", "-q", "-m", "tamper"], cwd=clone, check=True)
subprocess.run(["git", "push", "-q", "origin", "HEAD"], cwd=clone, check=True)
print(f"   attacker pushed evil bytes under the locked version {locked}")
EOF

# --------------------------------------------------------------------------
step "14. keel verify — must now FAIL (exit 1)"
cd app
FAILED=0
if STDOUT=$(K verify 2>&1); then
  FAILED=1
else
  :
fi
cd "$T"
if [ "$FAILED" -ne 0 ]; then
  echo "FAILED: verify must fail on the tampered registry" >&2
  printf '%s\n' "$STDOUT" >&2
  exit 1
fi
expect_grep "mismatch" "verify must report the checksum mismatch"
printf '%s\n' "$STDOUT"
say "tamper detected — exactly as designed"

# --------------------------------------------------------------------------
step "15. restore the registry + keel build (incremental layer works too)"
python3 - "$REG" <<'EOF'
import subprocess, sys
REG = sys.argv[1]
# Rewind the registry's main branch past the tamper commit (pure ref op).
subprocess.run(
    ["git", "-C", REG, "update-ref", "refs/heads/main", "HEAD~1"], check=True)
print("   registry rewound to the pre-tamper commit")
EOF
cd app
STDOUT=$(K verify)
expect_grep "verified OK" "verify passes again after restore"
STDOUT=$(K build 2>&1) || { echo "build failed" >&2; printf '%s\n' "$STDOUT" >&2; exit 1; }
say "build: $(printf '%s' "$STDOUT" | tail -1)"
cd "$T"

# --------------------------------------------------------------------------
printf '\n\033[1;32mL++ PM lifecycle demo: ALL STEPS PASSED\033[0m\n'
say "registry: $REG"
say "every command ran against a real git registry with the real lpp compiler."
