#!/bin/bash
# Wild vs lpp-link head-to-head on identical L++ ELF inputs (WSL Arch)
set -e
cd /mnt/c/Users/khati/lpp/challenge_bench

echo "=== build inputs with gcc -fno-pic (production recipe) ==="
gcc -O2 -DLPP_FREESTANDING -ffreestanding -fno-stack-protector -fno-pic -mno-red-zone \
    -c ../runtime/linux_x86_64_min.c -o rt_gcc_nopic.o
as stub_start.s -o stub_gcc.o
echo "built: $(ls -la rt_gcc_nopic.o stub_gcc.o | awk '{print $NF, $5"B"}')"

echo
echo "=== Wild 0.10.0 (9 runs) ==="
W=wilddist/wild-linker-0.10.0-x86_64-unknown-linux-gnu/wild
for i in 1 2 3 4 5 6 7 8 9; do
    s=$(date +%s%N)
    $W -no-pie stub_gcc.o ../test/hello_world.obj rt_gcc_nopic.o -o cmp_wild 2>/dev/null
    e=$(date +%s%N)
    echo "WILD run $i: $(( (e - s) / 1000000 )) ms"
done
./cmp_wild; echo "wild binary exit: $?"

echo
echo "=== lpp-link release (9 runs, via WSL interop; synthesizes _start itself) ==="
L=/mnt/c/Users/khati/lpp/target/release/lpp-link.exe
for i in 1 2 3 4 5 6 7 8 9; do
    s=$(date +%s%N)
    $L ../test/hello_world.obj rt_gcc_nopic.o -o cmp_lpplink 2>/dev/null
    e=$(date +%s%N)
    echo "LPPLINK run $i: $(( (e - s) / 1000000 )) ms"
done
./cmp_lpplink; echo "lpp-link binary exit: $?"
ls -la cmp_wild cmp_lpplink
