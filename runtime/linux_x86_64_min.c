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

/* ── Freestanding ARC foundation ─────────────────────────────────────────── */
/* Every direct-link ARC allocation owns a whole mmap region. This is not yet
 * a high-performance allocator, but it gives direct ELF programs correct ARC
 * headers, destructors, and lifetime behavior without libc. */

typedef void (*LppArcDestructor)(void *payload);
typedef struct {
    int refcount;
    LppArcDestructor destructor;
    uint64_t map_size;
} LppArcHeader;

static uint64_t lpp_page_round(uint64_t size) {
    const uint64_t page = 4096;
    return (size + page - 1) & ~(page - 1);
}

static void *lpp_sys_mmap(uint64_t size) {
    long result;
    register long r10 __asm__("r10") = 0x22; /* MAP_PRIVATE | MAP_ANONYMOUS */
    register long r8 __asm__("r8") = -1;
    register long r9 __asm__("r9") = 0;
    __asm__ volatile (
        "syscall"
        : "=a"(result)
        : "a"(9), "D"((long)0), "S"((long)size), "d"((long)3),
          "r"(r10), "r"(r8), "r"(r9)
        : "rcx", "r11", "memory"
    );
    return result < 0 ? (void *)0 : (void *)result;
}

static void lpp_sys_munmap(void *address, uint64_t size) {
    long ignored;
    __asm__ volatile (
        "syscall"
        : "=a"(ignored)
        : "a"(11), "D"(address), "S"((long)size)
        : "rcx", "r11", "memory"
    );
}

void *lpp_arc_alloc_with_destructor(int64_t payload_size, LppArcDestructor destructor) {
    if (payload_size < 0) return 0;
    uint64_t total = lpp_page_round((uint64_t)payload_size + sizeof(LppArcHeader));
    LppArcHeader *header = (LppArcHeader *)lpp_sys_mmap(total);
    if (!header) return 0;
    header->refcount = 1;
    header->destructor = destructor;
    header->map_size = total;
    return (void *)(header + 1);
}

void *lpp_arc_alloc(int64_t payload_size) {
    return lpp_arc_alloc_with_destructor(payload_size, 0);
}

void lpp_arc_retain(void *payload) {
    if (!payload) return;
    LppArcHeader *header = (LppArcHeader *)payload - 1;
    (void)__atomic_add_fetch(&header->refcount, 1, __ATOMIC_ACQ_REL);
}

void lpp_arc_release(void *payload) {
    if (!payload) return;
    LppArcHeader *header = (LppArcHeader *)payload - 1;
    if (__atomic_sub_fetch(&header->refcount, 1, __ATOMIC_ACQ_REL) == 0) {
        if (header->destructor) header->destructor(payload);
        lpp_sys_munmap(header, header->map_size);
    }
}

void *lpp_alloc(int64_t size) {
    return lpp_arc_alloc(size);
}

void lpp_free(void *payload, int64_t size) {
    (void)size;
    lpp_arc_release(payload);
}

/* ARC closure payload: [code pointer, owned environment pointer]. */
void lpp_closure_destroy(void *closure) {
    if (!closure) return;
    void **parts = (void **)closure;
    lpp_arc_release(parts[1]);
}

