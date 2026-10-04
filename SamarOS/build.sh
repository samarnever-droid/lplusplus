#!/usr/bin/env bash
# SamarOS — build the bootable disk image.
#
#   ./build.sh            build build/samaros.img
#   ./build.sh clean      remove build artefacts
#
# Pipeline:
#   assets/fonts/*.ttf  --genfont.py-->  build/font.c        (glyph atlas)
#   kernel/src/*.lpp    --lppc.py---->   build/samaros_lpp.c (L++ front end)
#   *.c + *.S           --gcc/ld----->   build/kernel.bin    (flat 32-bit)
#   boot1 + stage2 + kernel ---------->  build/samaros.img   (bootable)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD="$ROOT/build"
PY="${PYTHON:-python3}"

if [ "${1:-}" = "clean" ]; then
    rm -rf "$BUILD"
    echo "cleaned"
    exit 0
fi

mkdir -p "$BUILD"

CFLAGS=(
    -m32 -std=gnu99 -Os -march=i686
    -ffreestanding -fno-pic -fno-pie -fno-stack-protector -fno-builtin
    -fno-asynchronous-unwind-tables -fno-strict-aliasing -fomit-frame-pointer
    -mno-sse -mno-mmx -mno-80387 -mno-red-zone
    -Wall -Wextra -Wno-unused-parameter -Wno-unused-function
    -I"$ROOT/kernel/arch" -I"$ROOT/kernel/runtime" -I"$BUILD"
)

# ---------------------------------------------------------------- fonts
if [ ! -f "$BUILD/font.c" ] || [ "$ROOT/tools/genfont.py" -nt "$BUILD/font.c" ]; then
    echo "==> rasterising font atlas"
    "$PY" "$ROOT/tools/genfont.py" \
        "$ROOT/assets/fonts/InterVariable.ttf" \
        "$ROOT/assets/fonts/DejaVuSansMono.ttf" \
        "$BUILD/font.c" "$BUILD/font.h"
fi

# ------------------------------------------------------------------ L++
echo "==> compiling L++ kernel sources"
"$PY" "$ROOT/tools/lppc.py" "$BUILD/samaros_lpp.c" \
    "$ROOT/kernel/src/sys.lpp" \
    "$ROOT/kernel/src/ui.lpp" \
    "$ROOT/kernel/src/apps.lpp" \
    "$ROOT/kernel/src/samaros.lpp"

# ------------------------------------------------------------------- C
echo "==> compiling kernel"
OBJS=()
for src in "$ROOT/kernel/arch/kmain.c" "$ROOT/kernel/arch/gfx.c" \
           "$ROOT/kernel/arch/input.c" "$ROOT/kernel/runtime/kruntime.c" \
           "$BUILD/font.c" "$BUILD/samaros_lpp.c"; do
    obj="$BUILD/$(basename "${src%.c}").o"
    gcc "${CFLAGS[@]}" -c "$src" -o "$obj"
    OBJS+=("$obj")
done
gcc -m32 -c "$ROOT/kernel/arch/entry.S" -o "$BUILD/entry.o"

ld -m elf_i386 -T "$ROOT/kernel/kernel.ld" -o "$BUILD/kernel.elf" \
   "$BUILD/entry.o" "${OBJS[@]}"
objcopy -O binary "$BUILD/kernel.elf" "$BUILD/kernel.bin"

KSIZE=$(stat -c%s "$BUILD/kernel.bin")
KSECT=$(( (KSIZE + 511) / 512 ))
if [ "$KSIZE" -gt 458752 ]; then
    echo "error: kernel is ${KSIZE} bytes; the 0x20000..0x9F000 load window holds 448 KiB" >&2
    exit 1
fi

# ------------------------------------------------------------- stage 2
echo "==> assembling boot loader"
as --32 --defsym KERNEL_SECTORS="$KSECT" --defsym KERNEL_LBA=1 \
   "$ROOT/boot/stage2.S" -o "$BUILD/stage2.o"
ld -m elf_i386 -Ttext 0x7E00 --oformat binary -o "$BUILD/stage2.bin" "$BUILD/stage2.o"
S2SIZE=$(stat -c%s "$BUILD/stage2.bin")
S2SECT=$(( (S2SIZE + 511) / 512 ))

# stage 2 lives at LBA 1, so the kernel starts right after it
as --32 --defsym KERNEL_SECTORS="$KSECT" --defsym KERNEL_LBA=$((1 + S2SECT)) \
   "$ROOT/boot/stage2.S" -o "$BUILD/stage2.o"
ld -m elf_i386 -Ttext 0x7E00 --oformat binary -o "$BUILD/stage2.bin" "$BUILD/stage2.o"

# --------------------------------------------------------- boot sector
as --32 --defsym STAGE2_SECTORS="$S2SECT" "$ROOT/boot/boot1.S" -o "$BUILD/boot1.o"
ld -m elf_i386 -Ttext 0x7C00 --oformat binary -o "$BUILD/boot1.bin" "$BUILD/boot1.o"
if [ "$(stat -c%s "$BUILD/boot1.bin")" -ne 512 ]; then
    echo "error: boot sector must be exactly 512 bytes" >&2
    exit 1
fi

# --------------------------------------------------------------- image
echo "==> writing disk image"
IMG="$BUILD/samaros.img"
rm -f "$IMG"
# 4 MiB, because BIOSes (SeaBIOS included) refuse to read disks that are too
# small to have a sane CHS geometry.
dd if=/dev/zero of="$IMG" bs=1M count=4 status=none
dd if="$BUILD/boot1.bin"  of="$IMG" conv=notrunc status=none
dd if="$BUILD/stage2.bin" of="$IMG" bs=512 seek=1 conv=notrunc status=none
dd if="$BUILD/kernel.bin" of="$IMG" bs=512 seek=$((1 + S2SECT)) conv=notrunc status=none

cp "$IMG" "$ROOT/web/samaros.img"

printf '\n  boot sector : %5d bytes\n' "$(stat -c%s "$BUILD/boot1.bin")"
printf '  stage 2     : %5d bytes (%d sectors)\n' "$S2SIZE" "$S2SECT"
printf '  kernel      : %5d bytes (%d sectors)\n' "$KSIZE" "$KSECT"
printf '  disk image  : %5d bytes -> %s\n\n' "$(stat -c%s "$IMG")" "$IMG"
