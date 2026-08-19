#!/bin/bash
LPP="./target/debug/lpp"

for i in {1..5}; do
    pkg=""
    case $i in
        1) pkg="lpp-engine" ;;
        2) pkg="lpp-math" ;;
        3) pkg="lpp-git" ;;
        4) pkg="lpp-zip" ;;
        5) pkg="lpp-bindgen" ;;
    esac

    echo -e "\n[$i/5] Testing $pkg..."
    if [ -f "packages/$pkg/src/main.lpp" ]; then
        $LPP packages/$pkg/src/main.lpp --linker direct
        if [ $? -eq 0 ]; then
            if ! ./packages/$pkg/src/main; then
                echo "Failed natively. Trying Emulator (WASM)..."
                $LPP packages/$pkg/src/main.lpp --target wasm32-wasi
                wasmtime packages/$pkg/src/main.wasm || echo "Emulator failed too."
            fi
        else
            echo "$pkg compilation failed!"
        fi
    else
        echo "No main.lpp found for $pkg."
    fi
done
