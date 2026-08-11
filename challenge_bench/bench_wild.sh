#!/bin/bash
# Wild vs lpp-link speed benchmark (L++ hello_world ELF objects)
cd /mnt/c/Users/khati/lpp/challenge_bench || exit 1
W=wilddist/wild-linker-0.10.0-x86_64-unknown-linux-gnu/wild

echo "=== Wild 0.10.0 (9 runs, -no-pie) ==="
for i in 1 2 3 4 5 6 7 8 9; do
    s=$(date +%s%N)
    $W -no-pie stub_start.o ../test/hello_world.obj runtime_linux.o -o hw_wild_bench 2>/dev/null
    e=$(date +%s%N)
    echo "WILD run $i: $(( (e - s) / 1000000 )) ms"
done

echo "=== outputs ==="
ls -la hw_wild_bench 2>/dev/null
./hw_wild_bench; echo "wild binary exit: $?"

echo "=== Wild on WSL-native ext4 (9 runs) ==="
mkdir -p ~/wildbench
cp stub_start.o ../test/hello_world.obj runtime_linux.o "$W" ~/wildbench/
cd ~/wildbench || exit 1
for i in 1 2 3 4 5 6 7 8 9; do
    s=$(date +%s%N)
    ./wild -no-pie stub_start.o hello_world.obj runtime_linux.o -o out_elf 2>/dev/null
    e=$(date +%s%N)
    echo "WILD_EXT4 run $i: $(( (e - s) / 1000000 )) ms"
done
./out_elf; echo "ext4 binary exit: $?"
