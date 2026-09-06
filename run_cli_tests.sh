cat << 'INNER_EOF' > tests/test_cli_commands.sh
#!/usr/bin/env sh
# Verify core package manager CLI commands and command routing.
set -eu

ROOT=\$(CDPATH= cd -- "\$(dirname -- "\$0")/.." && pwd)
LPP="\$ROOT/target/release/lpp"
TEMP=\$(mktemp -d "\${TMPDIR:-/tmp}/lpp-cli-commands.XXXXXX")
cleanup() { rm -rf "\$TEMP"; }
trap cleanup HUP INT TERM

if [ ! -x "\$LPP" ]; then
    (cd "\$ROOT" && cargo build --release --bin lpp)
fi

export LPP_EMULATOR=1

# Test 1: Create a package using lpp new
cd "\$TEMP"
"\$LPP" new test_pkg >/dev/null
if [ ! -d "test_pkg/src" ] || [ ! -f "test_pkg/lpp.toml" ] || [ ! -f "test_pkg/src/main.lpp" ]; then
    echo "FAIL: lpp new did not create expected files"
    return 1
fi

# Test 2: Check a package directory using lpp check
"\$LPP" check test_pkg >/dev/null

# Test 3: Run a package directory using lpp run
OUTPUT=\$("\$LPP" run test_pkg)
if ! echo "\$OUTPUT" | grep "Hello, world!" >/dev/null; then
    echo "FAIL: lpp run output was: \$OUTPUT"
    return 1
fi

# Test 4: Verify implicit run when passing a package directory directly
OUTPUT=\$("\$LPP" test_pkg)
if ! echo "\$OUTPUT" | grep "Hello, world!" >/dev/null; then
    echo "FAIL: lpp pkg_dir output was: \$OUTPUT"
    return 1
fi

# Test 5: Verify correct failure when trying to check/run a non-existent package
if "\$LPP" check non_existent_pkg 2>/dev/null; then
    echo "FAIL: expected check on non-existent package to fail"
    return 1
fi

echo "PASS cli command split"
INNER_EOF
chmod +x tests/test_cli_commands.sh
LPP_EMULATOR=1 sh tests/test_cli_commands.sh
