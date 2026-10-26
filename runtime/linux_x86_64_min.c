/*
 * Freestanding Phase 2 ELF runtime.
 *
 * This runtime supports syscall-backed integer/string output, ARC memory,
 * dynamic lists, file I/O, and socket networking without libc.
 * Build with:
 * cc -O2 -ffreestanding -fno-stack-protector -fno-pic -mno-red-zone -c \
 *    runtime/linux_x86_64_min.c -o lpp_runtime_min.o
 */

#undef _FORTIFY_SOURCE
#define _FORTIFY_SOURCE 0

#include <stdint.h>
#include <time.h>

void lpp_exit(int64_t code);
int64_t lpp_str_eq(const char *a, const char *b);

uint64_t strlen(const char *s) { uint64_t n = 0; while (s && s[n]) n++; return n; }

/* Compiler-generated calls (zig/clang lower struct init & zeroing to these)
   even in freestanding mode, so the runtime must provide them itself. */
void *memset(void *dest, int value, unsigned long n) {
    unsigned char *d = (unsigned char *)dest;
    for (unsigned long i = 0; i < n; i++) d[i] = (unsigned char)value;
    return dest;
}

void *memcpy(void *dest, const void *src, unsigned long n) {
    unsigned char *d = (unsigned char *)dest;
    const unsigned char *s = (const unsigned char *)src;
    for (unsigned long i = 0; i < n; i++) d[i] = s[i];
    return dest;
}

void *memmove(void *dest, const void *src, unsigned long n) {
    unsigned char *d = (unsigned char *)dest;
    const unsigned char *s = (const unsigned char *)src;
    if (d < s) {
        for (unsigned long i = 0; i < n; i++) d[i] = s[i];
    } else {
        for (unsigned long i = n; i > 0; i--) d[i - 1] = s[i - 1];
    }
    return dest;
}

/* Freestanding clock_gettime via syscall 228 (x86_64). glibc headers
   (pulled in by lpp_gui.c's stdlib.h) supply struct timespec/clockid_t. */
int clock_gettime(clockid_t clock_id, struct timespec *tp) {
    long result;
    __asm__ volatile (
        "syscall"
        : "=a"(result)
        : "a"(228), "D"((long)clock_id), "S"(tp)
        : "rcx", "r11", "memory"
    );
    return (int)result;
}

/* Minimal freestanding libc shims for what lpp_gui.c references on Linux.
   Diagnostic prints go out via syscall 1; dlopen/dlsym fail so the X11
   backend reports "unavailable" and degrades gracefully. */
#include <stdio.h>
static long lpp_sys_write(long fd, const void *buffer, long count);
FILE *stdin = (FILE *)0, *stdout = (FILE *)0, *stderr = (FILE *)0;

int printf(const char *fmt, ...) {
    (void)lpp_sys_write(1, fmt, (long)strlen(fmt));
    return 0;
}

int fprintf(FILE *stream, const char *fmt, ...) {
    long fd = (stream == stderr) ? 2 : 1;
    (void)lpp_sys_write(fd, fmt, (long)strlen(fmt));
    return 0;
}

int fputs(const char *s, FILE *stream) {
    long fd = (stream == stderr) ? 2 : 1;
    (void)lpp_sys_write(fd, s, (long)strlen(s));
    return 0;
}

int puts(const char *s) {
    (void)lpp_sys_write(1, s, (long)strlen(s));
    (void)lpp_sys_write(1, "\n", 1);
    return 0;
}

int fflush(FILE *stream) { (void)stream; return 0; }

typedef struct {
    void *map_addr;
    uint64_t map_size;
    const char *dynstr;
    void *dynsym;
    uint64_t num_syms;
    uint64_t sym_ent_size;
} LppSoHandle;

typedef struct {
    uint32_t st_name;
    unsigned char st_info;
    unsigned char st_other;
    uint16_t st_shndx;
    uint64_t st_value;
    uint64_t st_size;
} LppElf64_Sym;

typedef struct {
    unsigned char e_ident[16];
    uint16_t e_type;
    uint16_t e_machine;
    uint32_t e_version;
    uint64_t e_entry;
    uint64_t e_phoff;
    uint64_t e_shoff;
    uint32_t e_flags;
    uint16_t e_ehsize;
    uint16_t e_phentsize;
    uint16_t e_phnum;
    uint16_t e_shentsize;
    uint16_t e_shnum;
    uint16_t e_shstrndx;
} LppElf64_Ehdr;

typedef struct {
    uint32_t sh_name;
    uint32_t sh_type;
    uint64_t sh_flags;
    uint64_t sh_addr;
    uint64_t sh_offset;
    uint64_t sh_size;
    uint32_t sh_link;
    uint32_t sh_info;
    uint64_t sh_addralign;
    uint64_t sh_entsize;
} LppElf64_Shdr;

static uint64_t lpp_page_round(uint64_t size);
static void *lpp_sys_mmap(uint64_t size);
static void lpp_sys_munmap(void *address, uint64_t size);
void *lpp_arc_alloc(int64_t payload_size);
void lpp_arc_release(void *payload);
void lpp_print_str(const char *text);
void lpp_print_int(int64_t value);

static long lpp_sys_open(const char *path, int flags, int mode);
static long lpp_sys_read(long fd, void *buf, long count);
static long lpp_sys_close(long fd);
static long lpp_sys_lseek(long fd, long offset, int whence);

void *dlopen(const char *path, int mode) {
    (void)mode;
    if (!path) return (void *)0;
    long fd = lpp_sys_open(path, 0, 0);
    if (fd < 0) {
        __asm__ volatile (
            "syscall"
            : "=a"(fd)
            : "a"(257), "D"(-100L), "S"(path), "d"(0L), "r"(0L)
            : "rcx", "r11", "memory"
        );
    }
    if (fd < 0) return (void *)0;
    long file_size = lpp_sys_lseek(fd, 0, 2);
    if (file_size <= 0) { lpp_sys_close(fd); return (void *)0; }
    (void)lpp_sys_lseek(fd, 0, 0);

    uint64_t map_size = lpp_page_round((uint64_t)file_size);
    void *map = lpp_sys_mmap(map_size);
    if (!map) { lpp_sys_close(fd); return (void *)0; }

    long read_bytes = lpp_sys_read(fd, map, file_size);
    lpp_sys_close(fd);
    if (read_bytes < (long)sizeof(LppElf64_Ehdr)) {
        lpp_sys_munmap(map, map_size);
        return (void *)0;
    }

    LppElf64_Ehdr *eh = (LppElf64_Ehdr *)map;
    if (eh->e_ident[0] != 0x7f || eh->e_ident[1] != 'E' || eh->e_ident[2] != 'L' || eh->e_ident[3] != 'F') {
        lpp_sys_munmap(map, map_size);
        return (void *)0;
    }

    /* Bounds-check section headers before walking them */
    if (eh->e_shoff == 0 || eh->e_shnum == 0 ||
        eh->e_shoff + (uint64_t)eh->e_shnum * sizeof(LppElf64_Shdr) > (uint64_t)file_size) {
        lpp_sys_munmap(map, map_size);
        return (void *)0;
    }

    LppElf64_Shdr *sections = (LppElf64_Shdr *)((char *)map + eh->e_shoff);
    const char *dynstr = (const char *)0;
    void *dynsym = (void *)0;
    uint64_t num_syms = 0;
    uint64_t sym_ent_size = sizeof(LppElf64_Sym);

    for (uint16_t i = 0; i < eh->e_shnum; i++) {
        if (sections[i].sh_type == 11 || sections[i].sh_type == 2) {
            /* Verify section data is within the mapped file */
            if (sections[i].sh_offset + sections[i].sh_size > (uint64_t)file_size) continue;
            dynsym = (char *)map + sections[i].sh_offset;
            uint32_t str_link = sections[i].sh_link;
            if (str_link < eh->e_shnum &&
                sections[str_link].sh_offset + sections[str_link].sh_size <= (uint64_t)file_size) {
                dynstr = (const char *)map + sections[str_link].sh_offset;
            }
            sym_ent_size = sections[i].sh_entsize ? sections[i].sh_entsize : sizeof(LppElf64_Sym);
            num_syms = sections[i].sh_size / sym_ent_size;
            if (sections[i].sh_type == 11) break;
        }
    }

    if (!dynstr || !dynsym) {
        lpp_sys_munmap(map, map_size);
        return (void *)0;
    }

    /* Use direct mmap for the handle — avoids arc allocator init ordering */
    LppSoHandle *handle = (LppSoHandle *)lpp_sys_mmap(lpp_page_round(sizeof(LppSoHandle)));
    if (!handle) {
        lpp_sys_munmap(map, map_size);
        return (void *)0;
    }

    handle->map_addr = map;
    handle->map_size = map_size;
    handle->dynstr = dynstr;
    handle->dynsym = dynsym;
    handle->num_syms = num_syms;
    handle->sym_ent_size = sym_ent_size;
    return (void *)handle;
}

void *dlsym(void *raw_handle, const char *name) {
    if (!raw_handle || !name) return (void *)0;
    LppSoHandle *handle = (LppSoHandle *)raw_handle;
    for (uint64_t i = 0; i < handle->num_syms; i++) {
        LppElf64_Sym *sym = (LppElf64_Sym *)((char *)handle->dynsym + i * handle->sym_ent_size);
        if (sym->st_name && lpp_str_eq(handle->dynstr + sym->st_name, name) == 1) {
            return (char *)handle->map_addr + sym->st_value;
        }
    }
    return (void *)0;
}

int dlclose(void *raw_handle) {
    if (!raw_handle) return 0;
    LppSoHandle *handle = (LppSoHandle *)raw_handle;
    if (handle->map_addr) {
        lpp_sys_munmap(handle->map_addr, handle->map_size);
    }
    lpp_sys_munmap(handle, lpp_page_round(sizeof(LppSoHandle)));
    return 0;
}

char *dlerror(void) { return (char *)0; }

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

void lpp_print_float(double v) {
    char buffer[64];
    char *cursor = buffer + sizeof(buffer);
    *--cursor = '\n';
    int negative = (v < 0.0);
    if (negative) v = -v;
    int64_t ipart = (int64_t)v;
    double fpart = v - (double)ipart;
    int64_t frac = (int64_t)(fpart * 1000000.0 + 0.5);
    for (int i = 0; i < 6; i++) {
        *--cursor = (char)('0' + (frac % 10));
        frac /= 10;
    }
    *--cursor = '.';
    uint64_t magnitude = (uint64_t)ipart;
    do {
        *--cursor = (char)('0' + (magnitude % 10));
        magnitude /= 10;
    } while (magnitude != 0);
    if (negative) *--cursor = '-';
    (void)lpp_sys_write(1, cursor, (long)((buffer + sizeof(buffer)) - cursor));
}

void lpp_print_bool(int8_t value) {
    lpp_print_int(value ? 1 : 0);
}

void lpp_print_str(const char *text) {
    const char *end = text;
    char newline = '\n';
    if (!text) return;
    while (*end) end++;
    (void)lpp_sys_write(1, text, (long)(end - text));
    (void)lpp_sys_write(1, &newline, 1);
}

#include "linux_x86_64_min/math_arc.inc"
#include "linux_x86_64_min/cycle.inc"
#include "linux_x86_64_min/cptr_closures.inc"
#include "linux_x86_64_min/lists.inc"
#include "linux_x86_64_min/file_io.inc"
#include "linux_x86_64_min/network.inc"
#include "linux_x86_64_min/maps.inc"
#include "linux_x86_64_min/strings.inc"
#include "linux_x86_64_min/extras.inc"

#include "lpp_gui.c"
