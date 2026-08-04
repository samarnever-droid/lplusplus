#!/usr/bin/env sh
# End-to-end regression coverage for the Rust package manager.
# The test uses only local path dependencies, so it is deterministic and does
# not require registry/network access.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
LPP=${LPP:-"$ROOT/target/debug/lpp"}
if [ ! -x "$LPP" ]; then
    (cd "$ROOT" && cargo build --bin lpp >/dev/null)
fi

TMP=$(mktemp -d "${TMPDIR:-/tmp}/lpp-pm.XXXXXX")
cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT HUP INT TERM

cd "$TMP"
"$LPP" init demo >/dev/null
[ -f lpp.toml ]
[ -f src/main.lpp ]

[ "$($LPP version)" = "demo 0.1.0" ]
"$LPP" version bump patch >/dev/null
[ "$($LPP version)" = "demo 0.1.1" ]
grep -Fq 'version = "0.1.1"' lpp.toml

mkdir -p dep/src
cat > dep/lpp.toml <<'EOF'
[package]
name = "dep"
version = "1.0.0"
entry = "src/main.lpp"

[dependencies]
EOF
cat > dep/src/main.lpp <<'EOF'
def main():
    print(1)
EOF

"$LPP" add dep --path "$TMP/dep" >/dev/null
# The dependency belongs in [dependencies], not in a later section, and the
# install action must produce a v2 lockfile.
grep -Fq 'dep = { path = ' lpp.toml
grep -Fq 'lock_version = 2' lpp.lock
[ -f .lpp_packages/dep/src/main.lpp ]
"$LPP" list >/dev/null
"$LPP" tree >/dev/null
"$LPP" metadata >/dev/null
"$LPP" install --offline >/dev/null
"$LPP" test >/dev/null
"$LPP" remove dep >/dev/null
! grep -Fq 'dep = {' lpp.toml
[ ! -e .lpp_packages/dep ]

# Build, run, and clean use the same status-aware PM path. Host linking keeps
# this test independent of an installed direct-link runtime.
LPP_HOME="$ROOT" LPP_LINKER=host "$LPP" build >/dev/null
[ -x LppData/build/release/demo ]
[ "$(LPP_HOME="$ROOT" LPP_LINKER=host "$LPP" run 2>/dev/null | tail -1)" = "Hello from L++ project!" ]
"$LPP" clean >/dev/null
[ ! -e LppData ]

# Workspace members inherit the root version and are visible to both the list
# and graph actions.
mkdir -p workspace/a/src workspace/b/src
cat > workspace/lpp.toml <<'EOF'
[workspace]
version = "4.0.0"
members = ["a", "b"]
EOF
for member in a b; do
    cat > "workspace/$member/lpp.toml" <<EOF
[package]
name = "$member"
version = { workspace = true }
entry = "src/main.lpp"

[dependencies]
EOF
    cat > "workspace/$member/src/main.lpp" <<EOF
def main():
    print(0)
EOF
done
(cd workspace && "$LPP" workspace members >/dev/null && "$LPP" workspace graph >/dev/null)
[ "$(cd workspace/a && "$LPP" version)" = "a 4.0.0" ]

# JSON manifests use the same versioning implementation.
mkdir json-project
cd json-project
mkdir src
cat > lpp.json <<'EOF'
{
  "name": "json-project",
  "version": "2.3.4",
  "main": "src/main.lpp",
  "dependencies": {}
}
EOF
cat > src/main.lpp <<'EOF'
def main():
    print(2)
EOF
[ "$($LPP version)" = "json-project 2.3.4" ]
"$LPP" version set 3.0.0 >/dev/null
[ "$($LPP version)" = "json-project 3.0.0" ]

# Compiler failures are observable to shell actions instead of being reported
# as successful commands.
set +e
"$LPP" missing.lpp >/dev/null 2>&1
status=$?
set -e
[ "$status" -ne 0 ]

echo "PASS package manager workflow"
