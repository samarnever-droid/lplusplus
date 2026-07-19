/*
 * Freestanding Phase 2 ELF runtime.
 *
 * This runtime intentionally supports only syscall-backed integer/string output.
 * It has no libc dependency and can be merged directly by lpp-link.
 * Build with:
 * cc -O2 -ffreestanding -fno-stack-protector -fno-pic -mno-red-zone -c \
 *    runtime/linux_x86_64_min.c -o lpp_runtime_min.o
 */

#include <stdint.h>

static long lpp_sys_write(long fd, const void *buffer, long count) {
    long result;
    __asm__ volatile (
        "syscall"
        : "=a"(result)
        : "a"(1), "D"(fd), "S"(buffer), "d"(count)
        : "rcx", "r11", "memory"
    );
    return result;
}

void lpp_print_int(int64_t value) {
    char buffer[32];
    char *cursor = buffer + sizeof(buffer);
    uint64_t magnitude;
    int negative = value < 0;
    if (negative) {
        /* Avoid signed overflow for INT64_MIN. */
        magnitude = (uint64_t)(-(value + 1)) + 1;
    } else {
        magnitude = (uint64_t)value;
    }
    *--cursor = '\n';
    do {
        *--cursor = (char)('0' + (magnitude % 10));
        magnitude /= 10;
    } while (magnitude != 0);
    if (negative) *--cursor = '-';
    (void)lpp_sys_write(1, cursor, (long)((buffer + sizeof(buffer)) - cursor));
}

void lpp_print_str(const char *text) {
    const char *end = text;
    char newline = '\n';
    if (!text) return;
    while (*end) end++;
    (void)lpp_sys_write(1, text, (long)(end - text));
    (void)lpp_sys_write(1, &newline, 1);
}
