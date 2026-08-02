TEMP=$(mktemp -d)
LPP="./target/debug/lpp"
LINKER="./target/debug/lpp-link"
cat > "$TEMP/direct.lpp" <<'INNER'
def main():
    x := 1
INNER
LPP_AOT=1 "$LPP" "$TEMP/direct.lpp" >/dev/null
"$LINKER" macho-arm64 "$TEMP/direct.o" -o "$TEMP/direct"
