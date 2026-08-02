#!/bin/bash
LPP="./target/debug/lpp"
echo -e "\n[1/5] Testing lpp-engine..."
$LPP packages/lpp-engine/src/main.lpp --linker direct
if [ $? -eq 0 ]; then
    ./packages/lpp-engine/src/main
else
    echo "lpp-engine compilation failed!"
fi
