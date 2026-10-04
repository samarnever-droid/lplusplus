#!/bin/bash
# Wild timing on WSL-native ext4 with gcc-built inputs
cd ~/wildbench || exit 1
for i in 1 2 3 4 5 6 7 8 9; do
    s=$(date +%s%N)
    ./wild -no-pie stub_gcc.o hello_world.obj rt_gcc_nopic.o -o out2 2>/dev/null
    e=$(date +%s%N)
    echo "WILD_EXT4 run $i: $(( (e - s) / 1000000 )) ms"
done

echo "=== GNU ld (binutils, same inputs) ==="
for i in 1 2 3 4 5 6 7 8 9; do
    s=$(date +%s%N)
    ld -nostdlib -Ttext 0x400000 --oformat binary -o /dev/null stub_gcc.o hello_world.obj rt_gcc_nopic.o 2>/dev/null || \
    ld -no-pie -nostdlib stub_gcc.o hello_world.obj rt_gcc_nopic.o -o out_ld 2>/dev/null
    e=$(date +%s%N)
    echo "LD_EXT4 run $i: $(( (e - s) / 1000000 )) ms"
done
