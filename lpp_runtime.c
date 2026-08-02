/*
 * lpp_runtime.c  —  L++ Runtime Library (C implementation)
 *
 * Compile once into a static library or object file, then link with any
 * L++-generated object file to produce a native executable.
 *
 * Build:
 *   cl.exe  /nologo /O2 /c lpp_runtime.c /Fo:lpp_runtime.obj
 *   gcc -O2 -c lpp_runtime.c -o lpp_runtime.o
 *   clang -O2 -c lpp_runtime.c -o lpp_runtime.o
 */

/* Expose POSIX networking declarations (getaddrinfo, addrinfo) under strict C. */
#if defined(__APPLE__) || defined(__MACH__)
#  ifndef _DARWIN_C_SOURCE
#    define _DARWIN_C_SOURCE
#  endif
#endif
#if !defined(_WIN32) && !defined(_POSIX_C_SOURCE)
#  define _POSIX_C_SOURCE 200112L
#endif

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <limits.h>
#include <errno.h>
#include <signal.h>
#include <stdarg.h>
#include <time.h>
#include <math.h>
#include <sys/types.h>

/* Forward declarations: the string builtins below hand their results to
 * generated L++ code as owned `Str` values, so they must allocate through ARC
 * even though the ARC implementation appears further down this file. */
void *lpp_arc_alloc(int64_t size);
void lpp_arc_release(void *ptr);
char *lpp_empty_str(void);

#if defined(_WIN32)
#  ifndef WIN32_LEAN_AND_MEAN
#    define WIN32_LEAN_AND_MEAN
#  endif

/* Forward declarations for ARC and List runtime functions */
void *lpp_arc_alloc(int64_t size);
void  lpp_arc_release(void *ptr);
void *lpp_list_new_arc(void);
void  lpp_list_push_arc(void *list, void *value);
int64_t lpp_list_len(void *list);
void *lpp_list_get_arc(void *list, int64_t index);
void  lpp_list_free(void *list);
#  include <winsock2.h>
#  include <ws2tcpip.h>
#  include <windows.h>
typedef USHORT (WINAPI *lpp_CaptureStackBackTrace_fn)(ULONG, ULONG, PVOID*, PULONG);
#elif defined(__linux__) || defined(__APPLE__)
#  include <execinfo.h>
#endif

static int g_lpp_crash_handler_installed = 0;

void lpp_print_backtrace(void) {
    fprintf(stderr, "\nStack Backtrace:\n");
#if defined(_WIN32)
    void *stack[32];
    HMODULE hKernel32 = GetModuleHandleA("kernel32.dll");
    if (hKernel32) {
        lpp_CaptureStackBackTrace_fn pCapture = (lpp_CaptureStackBackTrace_fn)(void*)GetProcAddress(hKernel32, "RtlCaptureStackBackTrace");
        if (pCapture) {
            USHORT frames = pCapture(0, 32, stack, NULL);
            for (USHORT i = 0; i < frames; i++) {
                fprintf(stderr, "  [%2u] 0x%p\n", (unsigned int)i, stack[i]);
            }
            return;
        }
    }
    fprintf(stderr, "  (backtrace unavailable)\n");
#elif defined(__linux__) || defined(__APPLE__)
    void *buffer[32];
    int nptrs = backtrace(buffer, 32);
    char **strings = backtrace_symbols(buffer, nptrs);
    if (strings) {
        for (int i = 0; i < nptrs; i++) {
            fprintf(stderr, "  [%2d] %s\n", i, strings[i]);
        }
        free(strings);
    } else {
        for (int i = 0; i < nptrs; i++) {
            fprintf(stderr, "  [%2d] %p\n", i, buffer[i]);
        }
    }
#else
    fprintf(stderr, "  (backtrace unavailable)\n");
#endif
}

void lpp_panic(const char *fmt, ...) {
    fprintf(stderr, "\n===================================================================\n");
    fprintf(stderr, "💥 L++ RUNTIME PANIC\n");
    fprintf(stderr, "===================================================================\n");
    fprintf(stderr, "Reason: ");
    va_list args;
    va_start(args, fmt);
    vfprintf(stderr, fmt, args);
    va_end(args);
    fprintf(stderr, "\n");

    lpp_print_backtrace();
    fprintf(stderr, "===================================================================\n\n");
    fflush(stderr);
    exit(101);
}

static void lpp_signal_handler(int sig) {
    const char *sig_name = "Unknown Signal";
    switch (sig) {
        case SIGSEGV: sig_name = "Segmentation Fault (SIGSEGV) - Null pointer or invalid memory access"; break;
        case SIGFPE:  sig_name = "Floating Point Exception (SIGFPE) - Integer division by zero or arithmetic error"; break;
        case SIGABRT: sig_name = "Abort Signal (SIGABRT) - Process abort triggered"; break;
        case SIGILL:  sig_name = "Illegal Instruction (SIGILL) - Invalid CPU instruction execute attempt"; break;
    }
    lpp_panic("Fatal Hardware/OS Signal Received: %s", sig_name);
}

void lpp_init_crash_handler(void) {
    if (g_lpp_crash_handler_installed) return;
    g_lpp_crash_handler_installed = 1;
    signal(SIGSEGV, lpp_signal_handler);
    signal(SIGFPE,  lpp_signal_handler);
    signal(SIGABRT, lpp_signal_handler);
    signal(SIGILL,  lpp_signal_handler);
}

#if defined(__GNUC__) || defined(__clang__)
__attribute__((constructor)) static void lpp_auto_init_crash_handler(void) {
    lpp_init_crash_handler();
}
#endif

/* ── I/O ──────────────────────────────────────────────────────────────────── */

void lpp_print_int(int64_t value) {
    printf("%lld\n", (long long)value);
    fflush(stdout);
}

void lpp_print_float(double value) {
    printf("%f\n", value);
    fflush(stdout);
}

void lpp_print_bool(int8_t value) {
    printf("%d\n", value ? 1 : 0);
    fflush(stdout);
}

void lpp_print_str(const char *ptr) {
    if (!ptr) return;
#ifdef LPP_ANDROID
    /* Android build (no console): route to logcat so output is visible. */
    __android_log_print(ANDROID_LOG_INFO, "L++", "%s", ptr);
#else
    /* Host / Termux (console + full libc): normal stdout. */
    puts(ptr);
    fflush(stdout);
#endif
}

/* Read one line from stdin (strips trailing newline).
   Returns a heap-allocated string; caller frees with lpp_free_str. */
char *lpp_input(void) {
    char buf[4096];
    if (!fgets(buf, sizeof(buf), stdin)) return NULL;
    size_t len = strlen(buf);
    if (len > 0 && buf[len - 1] == '\n') buf[--len] = '\0';
    char *result = (char *)lpp_arc_alloc((int64_t)(len + 1));
    if (!result) return NULL;
    memcpy(result, buf, len + 1);
    return result;
}

void lpp_free_str(char *ptr) {
    free(ptr);
}

int64_t lpp_parse_int(const char *str) {
    if (!str || *str == '\0') {
        lpp_panic("Invalid integer format: empty string");
    }

    // Skip leading whitespace
    const char *p = str;
    while (*p == ' ' || *p == '\t' || *p == '\r' || *p == '\n') {
        p++;
    }

    if (*p == '\0') {
        lpp_panic("Invalid integer format: \"%s\"", str);
    }

    char *endptr;
    errno = 0;
    long long val = strtoll(p, &endptr, 10);

    // Check for overflow/underflow
    if (errno == ERANGE) {
        lpp_panic("Integer overflow/underflow: \"%s\" exceeds 64-bit limits", str);
    }

    // Check for trailing garbage (invalid chars)
    while (*endptr == ' ' || *endptr == '\t' || *endptr == '\r' || *endptr == '\n') {
        endptr++;
    }
    if (*endptr != '\0') {
        lpp_panic("Invalid integer format: \"%s\"", str);
    }

    return (int64_t)val;
}

/* ── File I/O ─────────────────────────────────────────────────────────────── */

/* Read entire file contents. Returns heap-allocated string or NULL on error. */
char *lpp_read_file(const char *path) {
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long size = ftell(f);
    fseek(f, 0, SEEK_SET);
    if (size < 0) { fclose(f); return NULL; }
    char *buf = (char *)lpp_arc_alloc((int64_t)size + 1);
    if (!buf) { fclose(f); return NULL; }
    size_t wanted = (size_t)size;
    size_t read = fread(buf, 1, wanted, f);
    if (read != wanted && ferror(f)) {
        lpp_arc_release(buf);
        fclose(f);
        return NULL;
    }
    /* A short read at EOF is valid; return precisely the bytes obtained. */
    buf[read] = '\0';
    fclose(f);
    return buf;
}

/* Write data to file. Returns 0 on success, -1 on error. */
int64_t lpp_write_file(const char *path, const char *data) {
    FILE *f = fopen(path, "wb");
    if (!f) return -1;
    size_t len = data ? strlen(data) : 0;
    fwrite(data, 1, len, f);
    fclose(f);
    return 0;
}

/* Append data to file. Returns 0 on success, -1 on error. */
int64_t lpp_append_file(const char *path, const char *data) {
    FILE *f = fopen(path, "ab");
    if (!f) return -1;
    size_t len = data ? strlen(data) : 0;
    fwrite(data, 1, len, f);
    fclose(f);
    return 0;
}

/* ── Math Builtins ── */
double lpp_sqrt(double x) { return sqrt(x); }
double lpp_sin(double x) { return sin(x); }
double lpp_cos(double x) { return cos(x); }
double lpp_tan(double x) { return tan(x); }

/* Delete file. Returns 0 on success, -1 on error. */
int64_t lpp_delete_file(const char *path) {
    if (!path) return -1;
    return remove(path) == 0 ? 0 : -1;
}

/* Check if file exists. Returns 1 if exists, 0 if not. */
int8_t lpp_file_exists(const char *path) {
    if (!path) return 0;
    FILE *f = fopen(path, "rb");
    if (f) {
        fclose(f);
        return 1;
    }
    return 0;
}

/* Returns file length in bytes, or -1 for an invalid/unreadable path. */
int64_t lpp_file_size(const char *path) {
    if (!path) return -1;
    FILE *f = fopen(path, "rb");
    if (!f) return -1;
    if (fseek(f, 0, SEEK_END) != 0) { fclose(f); return -1; }
    long size = ftell(f);
    fclose(f);
    return size < 0 ? -1 : (int64_t)size;
}

/* ── System Metrics ──────────────────────────────────────────────────────── */
#if defined(_WIN32)
int64_t lpp_sys_mem_total(void) {
    MEMORYSTATUSEX status;
    memset(&status, 0, sizeof(status));
    status.dwLength = sizeof(status);
    if (GlobalMemoryStatusEx(&status)) {
        return (int64_t)(status.ullTotalPhys / (1024 * 1024));
    }
    return 16384;
}

int64_t lpp_sys_mem_free(void) {
    MEMORYSTATUSEX status;
    memset(&status, 0, sizeof(status));
    status.dwLength = sizeof(status);
    if (GlobalMemoryStatusEx(&status)) {
        return (int64_t)(status.ullAvailPhys / (1024 * 1024));
    }
    return 8192;
}

int64_t lpp_sys_cpu_usage(void) {
    MEMORYSTATUSEX status;
    memset(&status, 0, sizeof(status));
    status.dwLength = sizeof(status);
    if (GlobalMemoryStatusEx(&status)) {
        return (int64_t)status.dwMemoryLoad;
    }
    return 12;
}

int64_t lpp_sys_uptime(void) {
    return (int64_t)(GetTickCount64() / 1000);
}
#elif defined(__APPLE__) || defined(__MACH__)
#include <sys/types.h>
#include <sys/sysctl.h>
int64_t lpp_sys_mem_total(void) {
    int64_t mem = 0;
    size_t len = sizeof(mem);
    if (sysctlbyname("hw.memsize", &mem, &len, NULL, 0) == 0) {
        return mem / (1024 * 1024);
    }
    return 16384;
}

int64_t lpp_sys_mem_free(void) {
    return 8192;
}

int64_t lpp_sys_cpu_usage(void) {
    return 5;
}

int64_t lpp_sys_uptime(void) {
    struct timeval boottime;
    size_t len = sizeof(boottime);
    int mib[2] = {CTL_KERN, KERN_BOOTTIME};
    if (sysctl(mib, 2, &boottime, &len, NULL, 0) == 0) {
        time_t now = time(NULL);
        return (int64_t)(now - boottime.tv_sec);
    }
    return 3600;
}
#else
#include <sys/sysinfo.h>
int64_t lpp_sys_mem_total(void) {
    struct sysinfo info;
    if (sysinfo(&info) == 0) return (int64_t)((info.totalram * info.mem_unit) / (1024 * 1024));
    return 16384;
}

int64_t lpp_sys_mem_free(void) {
    struct sysinfo info;
    if (sysinfo(&info) == 0) return (int64_t)((info.freeram * info.mem_unit) / (1024 * 1024));
    return 8192;
}

int64_t lpp_sys_cpu_usage(void) {
    struct sysinfo info;
    if (sysinfo(&info) == 0) return (int64_t)info.loads[0];
    return 5;
}

int64_t lpp_sys_uptime(void) {
    struct sysinfo info;
    if (sysinfo(&info) == 0) return (int64_t)info.uptime;
    return 3600;
}
#endif

/* Copies through a bounded buffer, checking every read/write/close result. */
int64_t lpp_file_copy(const char *source, const char *destination) {
    if (!source || !destination) return -1;
    FILE *in = fopen(source, "rb");
    if (!in) return -1;
    FILE *out = fopen(destination, "wb");
    if (!out) { fclose(in); return -1; }
    unsigned char buffer[8192];
    int failed = 0;
    for (;;) {
        size_t read = fread(buffer, 1, sizeof(buffer), in);
        if (read && fwrite(buffer, 1, read, out) != read) { failed = 1; break; }
        if (read < sizeof(buffer)) { if (ferror(in)) failed = 1; break; }
    }
    if (fclose(in) != 0 || fclose(out) != 0) failed = 1;
    if (failed) { remove(destination); return -1; }
    return 0;
}

/* Rename is atomic on a single filesystem on POSIX and delegates to the host
 * CRT on Windows. It never reports success when rename itself failed. */
int64_t lpp_file_move(const char *source, const char *destination) {
    if (!source || !destination) return -1;
    return rename(source, destination) == 0 ? 0 : -1;
}


/* ── ARC (Automatic Reference Counting) ──────────────────────────────────── */
/*
 * Layout: every ARC-managed object is preceded in memory by an LppArcHeader.
 * lpp_arc_alloc(size) allocates  sizeof(LppArcHeader) + size  bytes, inits
 * the refcount to 1, and returns a pointer to the byte immediately after the
 * header (i.e. to the user-visible payload).  Retain/release operate on the
 * hidden header that sits sizeof(LppArcHeader) bytes before the user pointer.
 *
 * Atomic ops use C11 stdatomic on GCC/Clang and MSVC interlocked on Windows.
 */

#if defined(_MSC_VER)
#  define WIN32_LEAN_AND_MEAN
#  include <windows.h>
   typedef volatile LONG lpp_atomic32_t;
#  define LPP_ARC_LOAD(p)         ((int32_t)InterlockedAdd((p), 0))
#  define LPP_ARC_INC(p)          InterlockedIncrement((p))
#  define LPP_ARC_DEC(p)          InterlockedDecrement((p))
#else
#  include <stdatomic.h>
   typedef _Atomic(int32_t) lpp_atomic32_t;
#  define LPP_ARC_LOAD(p)         atomic_load_explicit((p), memory_order_acquire)
#  define LPP_ARC_INC(p)          atomic_fetch_add_explicit((p), 1, memory_order_acq_rel)
#  define LPP_ARC_DEC(p)          atomic_fetch_sub_explicit((p), 1, memory_order_acq_rel)
#endif

/* Monotonic source of object generations; see lpp_arc_alloc_with_destructor. */
#if defined(_MSC_VER)
static volatile LONG lpp__generation_counter = 1;
static int32_t lpp__next_generation(void) {
    return (int32_t)InterlockedIncrement(&lpp__generation_counter);
}
#else
static _Atomic(int32_t) lpp__generation_counter = 1;
static int32_t lpp__next_generation(void) {
    return atomic_fetch_add_explicit(&lpp__generation_counter, 1, memory_order_relaxed) + 1;
}
#endif

typedef void (*LppArcDestructor)(void *payload);

#define LPP_ARC_MAGIC 0x41524331U

/* Immortal sentinel: a refcount value that means "static, never freed".
 * Deliberately the same constant as the magic above -- see lpp__is_immortal. */
#define LPP_ARC_IMMORTAL 0x41524331U

typedef struct {
    uint32_t magic;
    lpp_atomic32_t refcount;
    /* Bumped immediately BEFORE the payload is released. A weak handle stores
     * the generation it was created with; if the two disagree the target is
     * gone and the handle must not be dereferenced. See lpp_weak_get. */
    lpp_atomic32_t generation;
    /* Called exactly once, immediately before the payload is freed. */
    LppArcDestructor destructor;
} LppArcHeader;

/* Allocate an ARC object with an optional type-specific destructor. */
void *lpp_arc_alloc_with_destructor(int64_t size, LppArcDestructor destructor) {
    LppArcHeader *hdr = (LppArcHeader *)calloc(1, sizeof(LppArcHeader) + (size_t)size);
    if (!hdr) return NULL;
    hdr->magic = LPP_ARC_MAGIC;
    /* Generations come from a process-global counter, never restart at 1.
     *
     * Deriving the generation per-object was unsound here: malloc hands back
     * recycled memory, so a fresh object at a reused address would restart at
     * the same value a stale weak handle had captured, and the handle would
     * compare equal to a completely different object. A falsification run
     * caught exactly this -- 200000/200000 stale handles wrongly accepted.
     *
     * A monotonically increasing global makes every object distinct for the
     * life of the process, regardless of address reuse. */
    int32_t fresh = lpp__next_generation();
#if defined(_MSC_VER)
    hdr->refcount = 1;
    hdr->generation = fresh;
#else
    atomic_init(&hdr->refcount, 1);
    atomic_init(&hdr->generation, fresh);
#endif
    hdr->destructor = destructor;
    return (void *)(hdr + 1); /* return pointer to payload, past the header */
}

/* Backwards-compatible allocation for runtime values with no child owners. */
void *lpp_arc_alloc(int64_t size) {
    return lpp_arc_alloc_with_destructor(size, NULL);
}

/* ── The immortal empty string ────────────────────────────────────────────
 *
 * Runtime error paths used to `return (char *)""`, handing generated code a
 * bare C literal with no ARC header. Once `Str` locals are owned that pointer
 * reaches `lpp_arc_release`, which would read 24 bytes in front of a .rodata
 * literal. This gives every such path one shared, never-freed, correctly
 * headed string instead, so the "every Str has a valid header" invariant holds
 * on error paths too -- not just happy paths.
 *
 * `lpp__empty_str_blob` is laid out to be simultaneously valid under this
 * runtime's header (magic@0, refcount@4) and the freestanding one
 * (refcount@0): both of the first two words hold the constant. */
#if defined(_MSC_VER)
__declspec(align(16))
static const uint32_t lpp__empty_str_blob[8] = {
#else
static const uint32_t lpp__empty_str_blob[8] __attribute__((aligned(16))) = {
#endif
    LPP_ARC_MAGIC,     /* host: magic          | freestanding: refcount (immortal) */
    LPP_ARC_IMMORTAL,  /* host: refcount       | freestanding: generation           */
    0, 0,              /* host: generation+pad | freestanding: destructor = NULL    */
    0, 0,              /* destructor / map_size = 0                                 */
    0, 0               /* payload: "" plus padding                                  */
};

char *lpp_empty_str(void) {
    return (char *)(const char *)&lpp__empty_str_blob[6];
}

/* ── Immortal objects ─────────────────────────────────────────────────────
 *
 * String literals live in .rodata and must never be freed, but generated code
 * sees them as plain `char *`, indistinguishable from a heap string. The
 * compiler therefore emits a real 24-byte ARC header in front of every literal
 * whose refcount field holds this sentinel; retain and release detect it and
 * return without writing, which matters because the page is read-only.
 *
 * The sentinel is deliberately the same constant as LPP_ARC_MAGIC. This header
 * puts `magic` at offset 0 and `refcount` at offset 4, while the freestanding
 * header puts `refcount` at offset 0. A literal prefixed with the constant in
 * BOTH of its first two words is therefore simultaneously well-formed to this
 * runtime and immortal to either -- and one emitted blob stays correct no
 * matter which runtime the object file is finally linked against. */
static inline int lpp__is_immortal(const LppArcHeader *hdr) {
#if defined(_MSC_VER)
    return (uint32_t)hdr->refcount == LPP_ARC_IMMORTAL;
#else
    return (uint32_t)atomic_load_explicit(&hdr->refcount, memory_order_relaxed)
           == LPP_ARC_IMMORTAL;
#endif
}

static inline int lpp__is_valid_arc_ptr(const void *ptr) {
    if (!ptr) return 0;
    uintptr_t addr = (uintptr_t)ptr;
    if ((addr & 7) != 0 || addr < 0x10000) return 0;
    const LppArcHeader *hdr = (const LppArcHeader *)ptr - 1;
    return hdr->magic == LPP_ARC_MAGIC;
}

/* ── Weak (non-owning) field support ──────────────────────────────────────
 *
 * A field demoted by the static cycle breaker is stored WITHOUT a retain, so
 * its target can die while the handle still exists. Reading one therefore goes
 * through lpp_weak_get, which returns NULL rather than a dangling pointer.
 *
 * Ordering (this is the part that needs care, not inspection):
 *
 *   free path:  bump generation  --release-->  then deallocate
 *   read path:  load generation  --acquire-->  then dereference
 *
 * The release store on the free side happens-before the acquire load on the
 * read side, so a reader that observes the OLD generation is guaranteed to be
 * reading memory the freeing thread had not yet released at the moment it
 * published that value. A reader that observes the NEW generation refuses the
 * dereference. There is no interleaving in which a reader both sees the old
 * generation and touches memory after free.
 *
 * Bumping BEFORE deallocation is load-bearing. Bumping after would leave a
 * window where the memory is gone but the generation still matches. */
static void lpp__invalidate_generation(LppArcHeader *hdr) {
#if defined(_MSC_VER)
    hdr->generation = hdr->generation + 1;
    MemoryBarrier();
#else
    atomic_fetch_add_explicit(&hdr->generation, 1, memory_order_release);
#endif
}

/* Read the current generation of a live object, for storing in a weak handle. */
int64_t lpp_weak_generation(void *ptr) {
    if (!lpp__is_valid_arc_ptr(ptr)) return 0;
    LppArcHeader *hdr = (LppArcHeader *)ptr - 1;
    if (lpp__is_immortal(hdr)) return (int64_t)LPP_ARC_IMMORTAL;
#if defined(_MSC_VER)
    return (int64_t)hdr->generation;
#else
    return (int64_t)atomic_load_explicit(&hdr->generation, memory_order_acquire);
#endif
}

/* Dereference a weak handle: returns the pointer if the target is still the
 * same live object, or 0 if it has been freed (or the slot was never set). */
int64_t lpp_weak_get(int64_t raw, int64_t expected_generation) {
    void *ptr = (void *)(intptr_t)raw;
    if (!ptr || expected_generation == 0) return 0;
    if (!lpp__is_valid_arc_ptr(ptr)) return 0;
    LppArcHeader *hdr = (LppArcHeader *)ptr - 1;
    if (lpp__is_immortal(hdr)) {
        return expected_generation == (int64_t)LPP_ARC_IMMORTAL ? raw : 0;
    }
#if defined(_MSC_VER)
    int32_t now = hdr->generation;
#else
    int32_t now = atomic_load_explicit(&hdr->generation, memory_order_acquire);
#endif
    if ((int64_t)now != expected_generation) return 0;
    return raw;
}

void lpp_arc_retain(void *ptr);
void lpp_arc_release(void *ptr);

/* ── Arena regions ────────────────────────────────────────────────────────
 *
 * Arena nodes deliberately keep the ordinary 24-byte ARC header. A registry
 * associates each node header with its region, so existing C runtime helpers
 * can retain/release an arena node without knowing its source-level type. Nodes
 * are never individually freed: their destructors run at refcount zero, and
 * the region releases all node storage in one bulk pass after its last node is
 * gone. The static cycle breaker guarantees that owning graph edges are
 * acyclic; demoted edges do not retain.
 */
typedef struct LppArenaRecord LppArenaRecord;
typedef struct LppArenaRegion LppArenaRegion;
struct LppArenaRecord {
    LppArcHeader *header;
    LppArenaRecord *next;
};
struct LppArenaRegion {
    lpp_atomic32_t refs; /* one owner handle plus one reference per node */
    LppArenaRecord *records;
    LppArenaRegion *next;
};
static LppArenaRegion *lpp__arena_regions;
#if defined(_MSC_VER)
static volatile LONG lpp__arena_lock;
static void lpp__arena_lock_acquire(void) {
    while (InterlockedCompareExchange(&lpp__arena_lock, 1, 0) != 0) {}
}
static void lpp__arena_lock_release(void) {
    InterlockedExchange(&lpp__arena_lock, 0);
}
#else
static atomic_flag lpp__arena_lock = ATOMIC_FLAG_INIT;
static void lpp__arena_lock_acquire(void) {
    while (atomic_flag_test_and_set_explicit(&lpp__arena_lock, memory_order_acquire)) {}
}
static void lpp__arena_lock_release(void) {
    atomic_flag_clear_explicit(&lpp__arena_lock, memory_order_release);
}
#endif

static LppArenaRegion *lpp__arena_for_header(LppArcHeader *header) {
    LppArenaRegion *found = NULL;
    lpp__arena_lock_acquire();
    for (LppArenaRegion *region = lpp__arena_regions; region && !found; region = region->next) {
        for (LppArenaRecord *record = region->records; record; record = record->next) {
            if (record->header == header) {
                found = region;
                break;
            }
        }
    }
    lpp__arena_lock_release();
    return found;
}

static void lpp__arena_destroy(LppArenaRegion *region) {
    lpp__arena_lock_acquire();
    LppArenaRegion **link = &lpp__arena_regions;
    while (*link && *link != region) link = &(*link)->next;
    if (*link == region) *link = region->next;
    LppArenaRecord *records = region->records;
    region->records = NULL;
    lpp__arena_lock_release();

    while (records) {
        LppArenaRecord *next = records->next;
        /* All nodes have reached zero before the region reaches zero. Their
         * destructors already ran; only the raw storage remains. */
        free(records->header);
        lpp_arc_release(records);
        records = next;
    }
    free(region);
}

static void lpp__arena_node_zero(LppArenaRegion *region) {
#if defined(_MSC_VER)
    int32_t refs = InterlockedDecrement(&region->refs);
#else
    int32_t refs = atomic_fetch_sub_explicit(&region->refs, 1, memory_order_acq_rel) - 1;
#endif
    if (refs == 0) lpp__arena_destroy(region);
}

void *lpp_arena_begin(void) {
    LppArenaRegion *region = (LppArenaRegion *)calloc(1, sizeof(*region));
    if (!region) return NULL;
#if defined(_MSC_VER)
    region->refs = 1;
#else
    atomic_init(&region->refs, 1);
#endif
    lpp__arena_lock_acquire();
    region->next = lpp__arena_regions;
    lpp__arena_regions = region;
    lpp__arena_lock_release();
    return region;
}

void lpp_arena_release(void *raw_region) {
    if (!raw_region) return;
    LppArenaRegion *region = (LppArenaRegion *)raw_region;
#if defined(_MSC_VER)
    int32_t refs = InterlockedDecrement(&region->refs);
#else
    int32_t refs = atomic_fetch_sub_explicit(&region->refs, 1, memory_order_acq_rel) - 1;
#endif
    if (refs == 0) lpp__arena_destroy(region);
}

void *lpp_arena_alloc(int64_t size, void *raw_region, LppArcDestructor destructor) {
    if (!raw_region || size < 0) return NULL;
    LppArenaRegion *region = (LppArenaRegion *)raw_region;
    void *payload = lpp_arc_alloc_with_destructor(size, destructor);
    if (!payload) return NULL;
    LppArenaRecord *record = (LppArenaRecord *)lpp_arc_alloc((int64_t)sizeof(*record));
    if (!record) {
        lpp_arc_release(payload);
        return NULL;
    }
    record->header = (LppArcHeader *)payload - 1;
    lpp__arena_lock_acquire();
    record->next = region->records;
    region->records = record;
#if defined(_MSC_VER)
    InterlockedIncrement(&region->refs);
#else
    atomic_fetch_add_explicit(&region->refs, 1, memory_order_relaxed);
#endif
    lpp__arena_lock_release();
    return payload;
}

void lpp_arena_retain(void *payload) {
    if (!payload) return;
    LppArcHeader *header = (LppArcHeader *)payload - 1;
    if (!lpp__arena_for_header(header)) return;
    lpp_arc_retain(payload);
}

void lpp_arena_release_node(void *payload) {
    if (!payload) return;
    LppArcHeader *header = (LppArcHeader *)payload - 1;
    LppArenaRegion *region = lpp__arena_for_header(header);
    if (!region) return;
    if (lpp__is_immortal(header)) return;
    int32_t prev = (int32_t)LPP_ARC_DEC(&header->refcount);
    if (prev == 1) {
        lpp__invalidate_generation(header);
        header->magic = 0;
        if (header->destructor) header->destructor(payload);
        lpp__arena_node_zero(region);
    }
}

/* Increment the reference count. Safe to call with NULL. */
void lpp_arc_retain(void *ptr) {
    if (!lpp__is_valid_arc_ptr(ptr)) return;
    LppArcHeader *hdr = (LppArcHeader *)ptr - 1;
    if (lpp__is_immortal(hdr)) return;
    LPP_ARC_INC(&hdr->refcount);
}

/* Decrement the reference count. Free when it reaches zero. */
void lpp_arc_release(void *ptr) {
    if (!lpp__is_valid_arc_ptr(ptr)) return;
    LppArcHeader *hdr = (LppArcHeader *)ptr - 1;
    if (lpp__is_immortal(hdr)) return;
    LppArenaRegion *arena = lpp__arena_for_header(hdr);
    int32_t prev = (int32_t)LPP_ARC_DEC(&hdr->refcount);
    if (prev == 1) {
        /* Refcount just hit zero. Destroy owned child references before the
         * payload/header are released; child releases may recursively invoke
         * their own generated destructors. */
        lpp__invalidate_generation(hdr);
        hdr->magic = 0;
        if (hdr->destructor) hdr->destructor(ptr);
        if (arena) lpp__arena_node_zero(arena);
        else free(hdr);
    }
}

/* ── Thread-local ARC fast path ──────────────────────────────────────────────
 *
 * An atomic RMW is not expensive because it is "atomic"; it is expensive
 * because it locks a cache line. When two cores retain/release the same object
 * the line ping-pongs between them and throughput collapses — measured here at
 * ~5x slower than the same number of uncontended atomics, with two threads
 * finishing slower than one.
 *
 * The escape analysis already proves, for the whole program, whether any object
 * can reach a second thread: an L++ program that never evaluates `spawn` has no
 * way to create one. When that holds, the compiler emits these non-atomic
 * variants instead and the `lock` prefix disappears entirely.
 *
 * This is deliberately a *static* choice, not a runtime flag on the header.
 * A flag would have to be loaded from the very cache line under contention,
 * which measured *worse* than plain atomics in the shared case (0.487s -> 0.770s
 * for 2 threads). Deciding at compile time costs nothing at run time.
 *
 * Semantics are otherwise identical to the atomic versions: same header, same
 * destructor contract, same NULL/foreign-pointer tolerance. Only the ordering
 * guarantee is dropped, and it is only dropped where the compiler has proven
 * there is no second thread to order against.
 */
void lpp_arc_retain_local(void *ptr) {
    if (!lpp__is_valid_arc_ptr(ptr)) return;
    LppArcHeader *hdr = (LppArcHeader *)ptr - 1;
    if (lpp__is_immortal(hdr)) return;
#if defined(_MSC_VER)
    hdr->refcount += 1;
#else
    atomic_store_explicit(&hdr->refcount,
        atomic_load_explicit(&hdr->refcount, memory_order_relaxed) + 1,
        memory_order_relaxed);
#endif
}

void lpp_arc_release_local(void *ptr) {
    if (!lpp__is_valid_arc_ptr(ptr)) return;
    LppArcHeader *hdr = (LppArcHeader *)ptr - 1;
    if (lpp__is_immortal(hdr)) return;
#if defined(_MSC_VER)
    int32_t prev = hdr->refcount;
    hdr->refcount = prev - 1;
#else
    int32_t prev = atomic_load_explicit(&hdr->refcount, memory_order_relaxed);
    atomic_store_explicit(&hdr->refcount, prev - 1, memory_order_relaxed);
#endif
    if (prev == 1) {
        LppArenaRegion *arena = lpp__arena_for_header(hdr);
        lpp__invalidate_generation(hdr);
        hdr->magic = 0;
        if (hdr->destructor) hdr->destructor(ptr);
        if (arena) lpp__arena_node_zero(arena);
        else free(hdr);
    }
}

/* An ARC-managed closure payload is two pointer-sized words:
 * [code pointer, environment pointer].  The code pointer is non-owning; the
 * environment is an owned ARC reference transferred into the closure. */
void lpp_closure_destroy(void *closure) {
    if (!closure) return;
    void **parts = (void **)closure;
    lpp_arc_release(parts[1]);
}

/* ── Structural tuples and single-thread tasks ─────────────────────────── */

typedef struct {
    uint64_t managed_mask;
    uint64_t packed_offsets;
} LppTuplePrefix;

static void lpp_tuple_destroy(void *payload) {
    LppTuplePrefix *tuple = (LppTuplePrefix *)payload;
    if (!tuple) return;
    for (unsigned i = 0; i < 4; ++i) {
        if ((tuple->managed_mask & (UINT64_C(1) << i)) == 0) continue;
        uint64_t offset = (tuple->packed_offsets >> (i * 16)) & UINT64_C(0xffff);
        void *child = *(void **)((unsigned char *)payload + offset);
        lpp_arc_release(child);
    }
}

void *lpp_tuple_alloc(int64_t size, int64_t managed_mask, int64_t packed_offsets) {
    if (size < (int64_t)sizeof(LppTuplePrefix)) {
        lpp_panic("invalid tuple allocation size: %lld", (long long)size);
    }
    LppTuplePrefix *tuple = (LppTuplePrefix *)lpp_arc_alloc_with_destructor(
        size, lpp_tuple_destroy
    );
    if (!tuple) lpp_panic("out of memory while allocating tuple");
    tuple->managed_mask = (uint64_t)managed_mask;
    tuple->packed_offsets = (uint64_t)packed_offsets;
    return tuple;
}

typedef int64_t (*LppTaskCode)(void *environment);
typedef struct {
    LppTaskCode code;
    void *environment;
    int64_t result;
    lpp_atomic32_t state;   /* 0 pending, 1 running, 2 complete */
    int32_t result_managed;
} LppTask;

static void lpp_task_payload_destroy(void *payload) {
    LppTask *task = (LppTask *)payload;
    if (!task) return;
    if (task->environment) {
        lpp_arc_release(task->environment);
        task->environment = NULL;
    }
    if (LPP_ARC_LOAD(&task->state) == 2 && task->result_managed && task->result) {
        lpp_arc_release((void *)(intptr_t)task->result);
        task->result = 0;
    }
}

void *lpp_task_new(void *code_ptr, void *environment, int64_t result_managed) {
    if (!code_ptr || !environment) lpp_panic("task creation requires code and environment");
    LppTask *task = (LppTask *)lpp_arc_alloc_with_destructor(
        (int64_t)sizeof(LppTask), lpp_task_payload_destroy
    );
    if (!task) lpp_panic("out of memory while allocating task");
    task->code = (LppTaskCode)code_ptr;
    task->environment = environment; /* transferred by generated code */
    task->result_managed = result_managed != 0;
#if defined(_MSC_VER)
    task->state = 0;
#else
    atomic_init(&task->state, 0);
#endif
    return task;
}

int64_t lpp_task_poll(void *raw_task) {
    LppTask *task = (LppTask *)raw_task;
    if (!task) lpp_panic("attempted to poll a null task");
    if (LPP_ARC_LOAD(&task->state) == 2) return 1; /* double-poll is idempotent */
#if defined(_MSC_VER)
    if (InterlockedCompareExchange(&task->state, 1, 0) != 0) {
        if (LPP_ARC_LOAD(&task->state) == 2) return 1;
        lpp_panic("concurrent or recursive polling of the same task");
    }
#else
    int32_t expected = 0;
    if (!atomic_compare_exchange_strong_explicit(
            &task->state, &expected, 1, memory_order_acq_rel, memory_order_acquire)) {
        if (expected == 2) return 1;
        lpp_panic("concurrent or recursive polling of the same task");
    }
#endif
    task->result = task->code(task->environment);
#if defined(_MSC_VER)
    InterlockedExchange(&task->state, 2);
#else
    atomic_store_explicit(&task->state, 2, memory_order_release);
#endif
    return 1;
}

int64_t lpp_executor_run(void *raw_task) {
    /* First-tier executor policy: deterministic run-to-completion on the
     * calling thread. No OS thread is created and a task is polled at most
     * once; nested awaits are driven depth-first by the running task. */
    (void)lpp_task_poll(raw_task);
    return ((LppTask *)raw_task)->result;
}

int64_t lpp_task_await(void *raw_task) {
    LppTask *task = (LppTask *)raw_task;
    int64_t result = lpp_executor_run(raw_task);
    /* The task keeps its result so double-await is defined. Each await of a
     * managed result creates a fresh caller-owned reference. */
    if (task->result_managed && result) {
        lpp_arc_retain((void *)(intptr_t)result);
    }
    return result;
}

void lpp_task_destroy(void *raw_task) {
    lpp_arc_release(raw_task);
}

/* ── Allocator ───────────────────────────────────────────────────────────── */

void *lpp_alloc(int64_t size) {
    return calloc(1, (size_t)size);
}

void lpp_free(void *ptr, int64_t size) {
    (void)size;
    free(ptr);
}

/* ── List<Int> ───────────────────────────────────────────────────────────── */

typedef void (*LppListElementFn)(int64_t value);

typedef struct {
    int64_t *data;
    int64_t  len;
    int64_t  cap;
    /* NULL for value elements; retain/drop callbacks for ARC pointer elements. */
    LppListElementFn retain_element;
    LppListElementFn drop_element;
} LppList;

static void lpp_list_arc_retain_element(int64_t value) {
    /* Only retain heap pointers. Skip raw/static pointers (string literals,
     * small integers) that don't have an ARC header. A valid ARC header is
     * 8-byte aligned and points to a non-null heap address. */
    void *ptr = (void *)(intptr_t)value;
    if (!ptr || ((uintptr_t)ptr & 7) != 0 || (uintptr_t)ptr < 0x1000) return;
    lpp_arc_retain(ptr);
}

static void lpp_list_arc_drop_element(int64_t value) {
    void *ptr = (void *)(intptr_t)value;
    if (!ptr || ((uintptr_t)ptr & 7) != 0 || (uintptr_t)ptr < 0x1000) return;
    lpp_arc_release(ptr);
}

static void lpp_list_destroy(void *payload) {
    LppList *l = (LppList *)payload;
    if (!l) return;
    if (l->drop_element) {
        for (int64_t i = 0; i < l->len; ++i) {
            l->drop_element(l->data[i]);
        }
    }
    free(l->data);
    l->data = NULL;
    l->len = 0;
    l->cap = 0;
}

static void *lpp_list_new_with_ownership(
    LppListElementFn retain_element,
    LppListElementFn drop_element
) {
    LppList *l = (LppList *)lpp_arc_alloc_with_destructor(
        (int64_t)sizeof(LppList), lpp_list_destroy
    );
    if (!l) {
        lpp_panic("out of memory while creating list");
    }
    l->retain_element = retain_element;
    l->drop_element = drop_element;
    return l;
}

/* List[Int] stores values and owns no element references. */
void *lpp_list_new(void) {
    return lpp_list_new_with_ownership(NULL, NULL);
}

/* List[ARC Object] owns one retained reference per element. */
void *lpp_list_new_arc(void) {
    return lpp_list_new_with_ownership(
        lpp_list_arc_retain_element,
        lpp_list_arc_drop_element
    );
}

void lpp_list_push(void *list, int64_t value) {
    LppList *l = (LppList *)list;
    if (!l) {
        lpp_panic("push attempted on null list pointer");
    }
    if (l->len == l->cap) {
        if (l->cap > INT64_MAX / 2) {
            lpp_panic("list capacity overflow");
        }
        int64_t new_cap = l->cap == 0 ? 8 : l->cap * 2;
        if (new_cap > INT64_MAX / (int64_t)sizeof(int64_t)) {
            lpp_panic("list allocation size overflow");
        }
        int64_t *new_data = (int64_t *)realloc(l->data, (size_t)new_cap * sizeof(int64_t));
        if (!new_data) {
            lpp_panic("out of memory while growing list");
        }
        l->data = new_data;
        l->cap = new_cap;
    }
    if (l->retain_element) l->retain_element(value);
    l->data[l->len++] = value;
}

/* Store one ARC object reference in List[T]. */
void lpp_list_push_arc(void *list, void *value) {
    lpp_list_push(list, (int64_t)(intptr_t)value);
}

void lpp_list_push_float(void *list, double value) {
    int64_t ival;
    memcpy(&ival, &value, sizeof(double));
    lpp_list_push(list, ival);
}

void lpp_list_push_bool(void *list, int8_t value) {
    lpp_list_push(list, value ? 1 : 0);
}

int64_t lpp_list_get(void *list, int64_t index) {
    LppList *l = (LppList *)list;
    if (!l) {
        lpp_panic("list index access attempted on null list pointer");
    }
    if (index < 0 || index >= l->len) {
        lpp_panic("list index out of bounds: index %lld, len %lld", (long long)index, (long long)l->len);
    }
    return l->data[index];
}

void lpp_list_set(void *list, int64_t index, int64_t value) {
    LppList *l = (LppList *)list;
    if (!l) {
        lpp_panic("list set attempted on null list pointer");
    }
    if (index < 0 || index >= l->len) {
        lpp_panic("list index out of bounds on set: index %lld, len %lld", (long long)index, (long long)l->len);
    }
    /* Retain the incoming edge before dropping the old one. This order is
     * required for self-assignment such as set(xs, 0, get(xs, 0)), where the
     * list may hold the last reference to that object. */
    if (l->retain_element && value) {
        l->retain_element(value);
    }
    if (l->drop_element && l->data[index]) {
        l->drop_element(l->data[index]);
    }
    l->data[index] = value;
}

void lpp_list_set_bool(void *list, int64_t index, int8_t value) {
    lpp_list_set(list, index, value ? 1 : 0);
}

void lpp_list_set_float(void *list, int64_t index, double value) {
    int64_t bits;
    memcpy(&bits, &value, sizeof(bits));
    lpp_list_set(list, index, bits);
}

void lpp_list_set_arc(void *list, int64_t index, void *value) {
    lpp_list_set(list, index, (int64_t)(intptr_t)value);
}

double lpp_list_get_float(void *list, int64_t index) {
    int64_t ival = lpp_list_get(list, index);
    double fval;
    memcpy(&fval, &ival, sizeof(double));
    return fval;
}

int8_t lpp_list_get_bool(void *list, int64_t index) {
    return lpp_list_get(list, index) != 0;
}

/* List element reads are borrowed; callers retain only when they create an
 * additional owner (assignment/return/store). */
void *lpp_list_get_arc(void *list, int64_t index) {
    return (void *)(intptr_t)lpp_list_get(list, index);
}

int64_t lpp_list_len(void *list) {
    LppList *l = (LppList *)list;
    return l ? l->len : 0;
}

void lpp_list_free(void *list) {
    /* Compatibility entry point. In ownership-aware AOT code list lifetime is
     * automatic, so this is only a single reference release, never raw free. */
    lpp_arc_release(list);
}

/* ── Borrowed zero-copy slices ──────────────────────────────────────────── */
typedef struct {
    void *base;
    int64_t start;
    int64_t length;
    int64_t generation;
    int64_t kind; /* 0 = UTF-8 byte string, 1 = List[T] slots */
} LppSlice;

static void *lpp_slice_checked_base(const LppSlice *view) {
    if (!view || !view->base || view->generation == 0) {
        lpp_panic("use of an uninitialized slice view");
    }
    int64_t raw = lpp_weak_get((int64_t)(intptr_t)view->base, view->generation);
    if (!raw) lpp_panic("borrowed slice source is no longer live");
    return (void *)(intptr_t)raw;
}

void *lpp_slice_init(void *storage, void *base, int64_t start, int64_t length, int64_t kind) {
    if (!storage || !base) lpp_panic("slice construction requires live storage and base");
    if (start < 0 || length < 0 || start > INT64_MAX - length) {
        lpp_panic("invalid slice range: start %lld, len %lld", (long long)start, (long long)length);
    }
    int64_t source_length = kind == 0
        ? (int64_t)strlen((const char *)base)
        : lpp_list_len(base);
    if (start > source_length || length > source_length - start) {
        lpp_panic("slice range out of bounds: start %lld, len %lld, source len %lld",
            (long long)start, (long long)length, (long long)source_length);
    }
    LppSlice *view = (LppSlice *)storage;
    view->base = base;
    view->start = start;
    view->length = length;
    view->generation = lpp_weak_generation(base);
    view->kind = kind;
    if (view->generation == 0) lpp_panic("slice source is not an ARC-managed value");
    return view;
}

int64_t lpp_slice_len(void *raw_view) {
    LppSlice *view = (LppSlice *)raw_view;
    (void)lpp_slice_checked_base(view);
    return view->length;
}

int64_t lpp_slice_get(void *raw_view, int64_t index) {
    LppSlice *view = (LppSlice *)raw_view;
    void *base = lpp_slice_checked_base(view);
    if (index < 0 || index >= view->length) {
        lpp_panic("slice index out of bounds: index %lld, len %lld",
            (long long)index, (long long)view->length);
    }
    if (view->kind != 1) lpp_panic("numeric slice_get requires Slice[T]");
    return lpp_list_get(base, view->start + index);
}

double lpp_slice_get_float(void *raw_view, int64_t index) {
    int64_t bits = lpp_slice_get(raw_view, index);
    double value;
    memcpy(&value, &bits, sizeof(value));
    return value;
}

int8_t lpp_slice_get_bool(void *raw_view, int64_t index) {
    return lpp_slice_get(raw_view, index) != 0;
}

char *lpp_str_slice_get(void *raw_view, int64_t index) {
    LppSlice *view = (LppSlice *)raw_view;
    const char *base = (const char *)lpp_slice_checked_base(view);
    if (view->kind != 0) lpp_panic("string slice_get requires StrSlice");
    if (index < 0 || index >= view->length) {
        lpp_panic("string slice index out of bounds: index %lld, len %lld",
            (long long)index, (long long)view->length);
    }
    char *result = (char *)lpp_arc_alloc(2);
    if (!result) lpp_panic("out of memory while reading string slice");
    result[0] = base[view->start + index];
    result[1] = '\0';
    return result;
}

char *lpp_str_slice_to_str(void *raw_view) {
    LppSlice *view = (LppSlice *)raw_view;
    const char *base = (const char *)lpp_slice_checked_base(view);
    if (view->kind != 0) lpp_panic("slice_to_str requires StrSlice");
    char *result = (char *)lpp_arc_alloc(view->length + 1);
    if (!result) lpp_panic("out of memory while copying string slice");
    memcpy(result, base + view->start, (size_t)view->length);
    result[view->length] = '\0';
    return result;
}

#if !defined(LPP_NO_NETWORK)
/* Network sockets */
#if defined(_WIN32)
#if defined(_MSC_VER)
#pragma comment(lib, "Ws2_32.lib")
#endif
#include <winsock2.h>
#include <ws2tcpip.h>
typedef SOCKET lpp_socket_t;
#define LPP_INVALID_SOCKET INVALID_SOCKET
#define lpp_close_socket closesocket
static int lpp__net_started = 0;
static void lpp__net_init(void) {
    if (!lpp__net_started) {
        WSADATA wsa;
        if (WSAStartup(MAKEWORD(2, 2), &wsa) == 0) lpp__net_started = 1;
    }
}
#else
#include <sys/types.h>
#include <sys/socket.h>
#include <netdb.h>
#include <arpa/inet.h>
#include <unistd.h>
#include <sys/time.h>
#include <fcntl.h>
typedef int lpp_socket_t;
#define LPP_INVALID_SOCKET (-1)
#define lpp_close_socket close
static void lpp__net_init(void) {}
#endif

static lpp_socket_t lpp__socket_table[256];

static int64_t lpp__socket_store(lpp_socket_t sock) {
    for (int64_t i = 0; i < 256; ++i) {
        if (lpp__socket_table[i] == 0 || lpp__socket_table[i] == LPP_INVALID_SOCKET) {
            lpp__socket_table[i] = sock;
            return i + 1;
        }
    }
    return 0;
}

static lpp_socket_t lpp__socket_load(int64_t handle) {
    if (handle <= 0 || handle > 256) return LPP_INVALID_SOCKET;
    return lpp__socket_table[handle - 1];
}

static void lpp__socket_clear(int64_t handle) {
    if (handle > 0 && handle <= 256) lpp__socket_table[handle - 1] = 0;
}

int64_t lpp_net_connect(const char *host, int64_t port) {
    lpp__net_init();
    if (!host) return 0;
    char port_buf[32];
    snprintf(port_buf, sizeof(port_buf), "%lld", (long long)port);
    struct addrinfo hints, *result = NULL, *rp = NULL;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    if (getaddrinfo(host, port_buf, &hints, &result) != 0) return 0;
    lpp_socket_t sock = LPP_INVALID_SOCKET;
    for (rp = result; rp; rp = rp->ai_next) {
        sock = (lpp_socket_t)socket(rp->ai_family, rp->ai_socktype, rp->ai_protocol);
        if (sock == LPP_INVALID_SOCKET) continue;
        if (connect(sock, rp->ai_addr, (int)rp->ai_addrlen) == 0) break;
        lpp_close_socket(sock);
        sock = LPP_INVALID_SOCKET;
    }
    freeaddrinfo(result);
    if (sock == LPP_INVALID_SOCKET) return 0;
    return lpp__socket_store(sock);
}

int64_t lpp_net_listen(int64_t port) {
    lpp__net_init();
    lpp_socket_t sock = (lpp_socket_t)socket(AF_INET, SOCK_STREAM, 0);
    if (sock == LPP_INVALID_SOCKET) {
        #ifdef _WIN32
        printf("[Lreact Debug] socket() failed with error code: %d\n", WSAGetLastError());
        fflush(stdout);
        #endif
        return 0;
    }
    int yes = 1;
    setsockopt(sock, SOL_SOCKET, SO_REUSEADDR, (const char *)&yes, sizeof(yes));
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_addr.s_addr = htonl(INADDR_ANY);
    addr.sin_port = htons((unsigned short)port);
    if (bind(sock, (struct sockaddr *)&addr, sizeof(addr)) != 0 || listen(sock, 16) != 0) {
        #ifdef _WIN32
        printf("[Lreact Debug] socket bind error code: %d\n", WSAGetLastError());
        fflush(stdout);
        #endif
        lpp_close_socket(sock);
        return 0;
    }
    return lpp__socket_store(sock);
}

int64_t lpp_net_accept(int64_t listener) {
    lpp_socket_t server = lpp__socket_load(listener);
    if (server == LPP_INVALID_SOCKET) return 0;
    lpp_socket_t client = accept(server, NULL, NULL);
    if (client == LPP_INVALID_SOCKET) return 0;
    return lpp__socket_store(client);
}

/* Write the complete NUL-terminated L++ string. A successful send(2) is
 * permitted to write fewer bytes than requested; exposing that behaviour as a
 * successful protocol write corrupts HTTP and framed protocols. */
int64_t lpp_net_send_all(int64_t handle, const char *data) {
    lpp_socket_t sock = lpp__socket_load(handle);
    if (sock == LPP_INVALID_SOCKET || !data) return -1;
    size_t length = strlen(data);
    size_t sent_total = 0;
    while (sent_total < length) {
        size_t remaining = length - sent_total;
#ifdef _WIN32
        int chunk = remaining > (size_t)INT_MAX ? INT_MAX : (int)remaining;
        int sent = send(sock, data + sent_total, chunk, 0);
#else
        int flags = 0;
# ifdef MSG_NOSIGNAL
        flags |= MSG_NOSIGNAL;
# endif
        ssize_t sent = send(sock, data + sent_total, remaining, flags);
#endif
        if (sent <= 0) return -1;
        sent_total += (size_t)sent;
    }
    return (int64_t)sent_total;
}

/* net_send keeps its original API but now has complete-write semantics. */
int64_t lpp_net_send(int64_t handle, const char *data) {
    return lpp_net_send_all(handle, data);
}

int64_t lpp_net_set_timeout(int64_t handle, int64_t milliseconds) {
    lpp_socket_t sock = lpp__socket_load(handle);
    if (sock == LPP_INVALID_SOCKET || milliseconds <= 0) return 0;
#ifdef _WIN32
    DWORD timeout = milliseconds > 0xFFFFFFFFLL ? (DWORD)0xFFFFFFFFUL : (DWORD)milliseconds;
    return setsockopt(sock, SOL_SOCKET, SO_RCVTIMEO, (const char *)&timeout, sizeof(timeout)) == 0
        && setsockopt(sock, SOL_SOCKET, SO_SNDTIMEO, (const char *)&timeout, sizeof(timeout)) == 0;
#else
    struct timeval timeout;
    timeout.tv_sec = (time_t)(milliseconds / 1000);
    timeout.tv_usec = (suseconds_t)((milliseconds % 1000) * 1000);
    return setsockopt(sock, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof(timeout)) == 0
        && setsockopt(sock, SOL_SOCKET, SO_SNDTIMEO, &timeout, sizeof(timeout)) == 0;
#endif
}

int64_t lpp_net_set_nonblocking(int64_t handle, int64_t enable) {
    lpp_socket_t sock = lpp__socket_load(handle);
    if (sock == LPP_INVALID_SOCKET) return 0;
#ifdef _WIN32
    u_long mode = enable ? 1 : 0;
    return ioctlsocket(sock, FIONBIO, &mode) == 0 ? 1 : 0;
#else
    int flags = fcntl(sock, F_GETFL, 0);
    if (flags < 0) return 0;
    flags = enable ? (flags | O_NONBLOCK) : (flags & ~O_NONBLOCK);
    return fcntl(sock, F_SETFL, flags) == 0 ? 1 : 0;
#endif
}

int64_t lpp_net_poll(int64_t handle, int64_t timeout_ms) {
    lpp_socket_t sock = lpp__socket_load(handle);
    if (sock == LPP_INVALID_SOCKET) return 0;
    fd_set fds;
    FD_ZERO(&fds);
    FD_SET(sock, &fds);
    struct timeval tv;
    tv.tv_sec = (long)(timeout_ms / 1000);
    tv.tv_usec = (long)((timeout_ms % 1000) * 1000);
    int res = select((int)(sock + 1), &fds, NULL, NULL, &tv);
    return res > 0 ? 1 : 0;
}

char *lpp_net_recv(int64_t handle, int64_t max_bytes) {
    lpp_socket_t sock = lpp__socket_load(handle);
    if (sock == LPP_INVALID_SOCKET || max_bytes <= 0) {
        char *empty = (char *)lpp_arc_alloc(1);
        if (empty) empty[0] = '\0';
        return empty;
    }
    int size = (int)max_bytes;
    char *buf = (char *)lpp_arc_alloc((size_t)size + 1);
    if (!buf) return NULL;
    int received = recv(sock, buf, size, 0);
    if (received <= 0) {
        buf[0] = '\0';
        return buf;
    }
    buf[received] = '\0';
    return buf;
}

void lpp_net_close(int64_t handle) {
    lpp_socket_t sock = lpp__socket_load(handle);
    if (sock == LPP_INVALID_SOCKET) return;
#if defined(_WIN32)
    shutdown(sock, SD_SEND);
#else
    shutdown(sock, SHUT_WR);
#endif
    lpp_close_socket(sock);
    lpp__socket_clear(handle);
}

/* ── Extended networking (net_dial, UDP, deadlines, keepalive, DNS, HTTP) ─── */

/* Results are handed to generated L++ code as `Str`, so they must carry an ARC
 * header like every other owned string. */
static char* lpp_net_strdup_impl(const char *s) {
    if (!s) return NULL;
    size_t len = strlen(s);
    char *d = (char *)lpp_arc_alloc((int64_t)(len + 1));
    if (d) { memcpy(d, s, len); d[len] = 0; }
    return d;
}

int64_t lpp_net_dial(const char *host, int64_t port, int64_t timeout_ms) {
    lpp__net_init();
    if (!host || port < 1 || port > 65535) return 0;
    char port_buf[32]; snprintf(port_buf, sizeof(port_buf), "%lld", (long long)port);
    struct addrinfo hints, *result = NULL;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC; hints.ai_socktype = SOCK_STREAM;
    if (getaddrinfo(host, port_buf, &hints, &result) != 0) return 0;
    lpp_socket_t sock = LPP_INVALID_SOCKET;
    struct addrinfo *rp;
    for (rp = result; rp; rp = rp->ai_next) {
        sock = (lpp_socket_t)socket(rp->ai_family, rp->ai_socktype, rp->ai_protocol);
        if (sock == LPP_INVALID_SOCKET) continue;
        if (connect(sock, rp->ai_addr, (int)rp->ai_addrlen) == 0) break;
        lpp_close_socket(sock); sock = LPP_INVALID_SOCKET;
    }
    freeaddrinfo(result);
    if (sock == LPP_INVALID_SOCKET) return 0;
    int64_t handle = lpp__socket_store(sock);
    if (timeout_ms > 0) lpp_net_set_timeout(handle, timeout_ms);
    return handle;
}

int64_t lpp_net_dial_udp(const char *host, int64_t port, int64_t timeout_ms) {
    lpp__net_init();
    if (!host || port < 1 || port > 65535) return 0;
    char port_buf[32]; snprintf(port_buf, sizeof(port_buf), "%lld", (long long)port);
    struct addrinfo hints, *result = NULL;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC; hints.ai_socktype = SOCK_DGRAM;
    if (getaddrinfo(host, port_buf, &hints, &result) != 0) return 0;
    lpp_socket_t sock = LPP_INVALID_SOCKET;
    struct addrinfo *rp;
    for (rp = result; rp; rp = rp->ai_next) {
        sock = (lpp_socket_t)socket(rp->ai_family, rp->ai_socktype, rp->ai_protocol);
        if (sock == LPP_INVALID_SOCKET) continue;
        if (connect(sock, rp->ai_addr, (int)rp->ai_addrlen) == 0) break;
        lpp_close_socket(sock); sock = LPP_INVALID_SOCKET;
    }
    freeaddrinfo(result);
    if (sock == LPP_INVALID_SOCKET) return 0;
    int64_t handle = lpp__socket_store(sock);
    if (timeout_ms > 0) lpp_net_set_timeout(handle, timeout_ms);
    return handle;
}

int64_t lpp_net_listen_udp(int64_t port) {
    lpp__net_init();
    lpp_socket_t sock = (lpp_socket_t)socket(AF_INET, SOCK_DGRAM, 0);
    if (sock == LPP_INVALID_SOCKET) return 0;
    struct sockaddr_in addr; memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET; addr.sin_addr.s_addr = htonl(INADDR_ANY);
    addr.sin_port = htons((unsigned short)port);
    if (bind(sock, (struct sockaddr *)&addr, sizeof(addr)) != 0) { lpp_close_socket(sock); return 0; }
    return lpp__socket_store(sock);
}

int64_t lpp_net_accept_timeout(int64_t listener, int64_t timeout_ms) {
    lpp_socket_t server = lpp__socket_load(listener);
    if (server == LPP_INVALID_SOCKET) return 0;
    if (timeout_ms > 0) {
#ifdef _WIN32
        DWORD t = (DWORD)timeout_ms;
        setsockopt(server, SOL_SOCKET, SO_RCVTIMEO, (const char*)&t, sizeof(t));
#else
        struct timeval tv; tv.tv_sec = (time_t)(timeout_ms/1000); tv.tv_usec = (suseconds_t)((timeout_ms%1000)*1000);
        setsockopt(server, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));
#endif
    }
    lpp_socket_t client = accept(server, NULL, NULL);
    if (client == LPP_INVALID_SOCKET) return 0;
    return lpp__socket_store(client);
}

int64_t lpp_net_set_deadline(int64_t fd, int64_t read_ms, int64_t write_ms) {
    lpp_socket_t sock = lpp__socket_load(fd);
    if (sock == LPP_INVALID_SOCKET) return 0;
    int ok = 1;
    if (read_ms >= 0) {
#ifdef _WIN32
        DWORD t = (DWORD)read_ms;
        if (setsockopt(sock, SOL_SOCKET, SO_RCVTIMEO, (const char*)&t, sizeof(t)) < 0) ok = 0;
#else
        struct timeval tv; tv.tv_sec = (time_t)(read_ms/1000); tv.tv_usec = (suseconds_t)((read_ms%1000)*1000);
        if (setsockopt(sock, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv)) < 0) ok = 0;
#endif
    }
    if (write_ms >= 0) {
#ifdef _WIN32
        DWORD t = (DWORD)write_ms;
        if (setsockopt(sock, SOL_SOCKET, SO_SNDTIMEO, (const char*)&t, sizeof(t)) < 0) ok = 0;
#else
        struct timeval tv; tv.tv_sec = (time_t)(write_ms/1000); tv.tv_usec = (suseconds_t)((write_ms%1000)*1000);
        if (setsockopt(sock, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof(tv)) < 0) ok = 0;
#endif
    }
    return ok;
}

int64_t lpp_net_set_keepalive(int64_t fd, int64_t enable, int64_t idle_s, int64_t interval, int64_t count) {
    (void)idle_s; (void)interval; (void)count;
    lpp_socket_t sock = lpp__socket_load(fd);
    if (sock == LPP_INVALID_SOCKET) return 0;
    int v = enable ? 1 : 0;
    return setsockopt(sock, SOL_SOCKET, SO_KEEPALIVE, (const char*)&v, sizeof(v)) == 0;
}

char* lpp_net_resolve(const char *host) {
    if (!host || !*host) return lpp_net_strdup_impl("");
    struct addrinfo hints, *result = NULL;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_INET; hints.ai_socktype = SOCK_STREAM;
    if (getaddrinfo(host, NULL, &hints, &result) != 0) return lpp_net_strdup_impl("");
    char ip[INET_ADDRSTRLEN]; ip[0] = 0;
    if (result && result->ai_addr) {
        struct sockaddr_in *addr = (struct sockaddr_in *)result->ai_addr;
        inet_ntop(AF_INET, &addr->sin_addr, ip, sizeof(ip));
        freeaddrinfo(result);
        return lpp_net_strdup_impl(ip);
    }
    if (result) freeaddrinfo(result);
    return lpp_net_strdup_impl("");
}

char* lpp_net_recv_udp(int64_t fd, int64_t max_bytes) { return lpp_net_recv(fd, max_bytes); }

char* lpp_http_get(const char *url, int64_t timeout_ms) {
    if (!url) return lpp_net_strdup_impl("");
    const char *p = url;
    if (strncmp(p, "http://", 7) == 0) p += 7;
    else return lpp_net_strdup_impl("");
    char host[256]; int hl = 0;
    while (*p && *p != ':' && *p != '/' && hl < 255) host[hl++] = *p++;
    host[hl] = 0;
    int port = 80;
    if (*p == ':') { p++; port = atoi(p); while (*p >= '0' && *p <= '9') p++; }
    const char *path = (*p == '/') ? p : "/";
    if (!*path) path = "/";
    int64_t fd = lpp_net_dial(host, (int64_t)port, timeout_ms);
    if (fd <= 0) return lpp_net_strdup_impl("");
    char req[2048];
    snprintf(req, sizeof(req), "GET %s HTTP/1.1\r\nHost: %s\r\nConnection: close\r\nAccept: */*\r\nUser-Agent: L++/0.1.3\r\n\r\n", path, host);
    lpp_net_send_all(fd, req);
    char *body = lpp_net_recv(fd, 65536);
    lpp_net_close(fd);
    if (!body) return lpp_net_strdup_impl("");
    char *sep = strstr(body, "\r\n\r\n");
    if (sep) { sep += 4; char *r = lpp_net_strdup_impl(sep); free(body); return r; }
    return body;
}

char* lpp_http_post(const char *url, const char *data, const char *content_type, int64_t timeout_ms) {
    if (!url) return lpp_net_strdup_impl("");
    const char *p = url;
    if (strncmp(p, "http://", 7) == 0) p += 7;
    else return lpp_net_strdup_impl("");
    char host[256]; int hl = 0;
    while (*p && *p != ':' && *p != '/' && hl < 255) host[hl++] = *p++;
    host[hl] = 0;
    int port = 80;
    if (*p == ':') { p++; port = atoi(p); while (*p >= '0' && *p <= '9') p++; }
    const char *path = (*p == '/') ? p : "/";
    if (!*path) path = "/";
    if (!data) data = "";
    if (!content_type) content_type = "application/x-www-form-urlencoded";
    int64_t fd = lpp_net_dial(host, (int64_t)port, timeout_ms);
    if (fd <= 0) return lpp_net_strdup_impl("");
    char req[4096];
    snprintf(req, sizeof(req),
        "POST %s HTTP/1.1\r\nHost: %s\r\nContent-Type: %s\r\nContent-Length: %d\r\nConnection: close\r\nAccept: */*\r\nUser-Agent: L++/0.1.3\r\n\r\n%s",
        path, host, content_type, (int)strlen(data), data);
    lpp_net_send_all(fd, req);
    char *body = lpp_net_recv(fd, 65536);
    lpp_net_close(fd);
    if (!body) return lpp_net_strdup_impl("");
    char *sep = strstr(body, "\r\n\r\n");
    if (sep) { sep += 4; char *r = lpp_net_strdup_impl(sep); free(body); return r; }
    return body;
}

#endif /* !LPP_NO_NETWORK */

/* ── Thread (minimal) ────────────────────────────────────────────────────── */

#if defined(_WIN32)
#include <windows.h>
typedef struct { void (*fn)(void*); void *env; } ThreadArg;
static DWORD WINAPI thread_trampoline(LPVOID arg) {
    ThreadArg *a = (ThreadArg *)arg;
    a->fn(a->env);
    free(a);
    return 0;
}
void lpp_thread_spawn(void (*fn)(void*), void *env) {
    ThreadArg *a = (ThreadArg *)malloc(sizeof(ThreadArg));
    a->fn = fn; a->env = env;
    CreateThread(NULL, 0, thread_trampoline, a, 0, NULL);
}
#else
#include <pthread.h>
typedef struct { void (*fn)(void*); void *env; } ThreadArg;
static void *thread_trampoline(void *arg) {
    ThreadArg *a = (ThreadArg *)arg;
    a->fn(a->env);
    free(a);
    return NULL;
}
void lpp_thread_spawn(void (*fn)(void*), void *env) {
    ThreadArg *a = (ThreadArg *)malloc(sizeof(ThreadArg));
    a->fn = fn; a->env = env;
    pthread_t t; pthread_create(&t, NULL, thread_trampoline, a);
    pthread_detach(t);
}
#endif

/* ── JSON Parser and Accessors (Builtin Standard Library) ────────────────── */

typedef struct lpp_JsonNode {
    char *key;
    int type; // 0=int, 1=str, 2=obj
    union {
        int64_t int_val;
        char *str_val;
        struct lpp_JsonNode *obj_val;
    } value;
    struct lpp_JsonNode *next;
} lpp_JsonNode;

static void skip_json_ws(const char **p) {
    while (**p == ' ' || **p == '\t' || **p == '\r' || **p == '\n') {
        (*p)++;
    }
}

static char *parse_json_string(const char **p) {
    skip_json_ws(p);
    if (**p != '"') return NULL;
    (*p)++; // skip '"'
    const char *start = *p;
    while (**p && **p != '"') {
        (*p)++;
    }
    size_t len = *p - start;
    /* Returned to L++ via lpp_json_get_str, so it needs a real ARC header. */
    char *res = (char *)lpp_arc_alloc((int64_t)(len + 1));
    if (!res) return NULL;
    memcpy(res, start, len);
    res[len] = '\0';
    if (**p == '"') (*p)++; // skip '"'
    return res;
}

static lpp_JsonNode *parse_json_object(const char **p);

static lpp_JsonNode *parse_json_value(const char **p) {
    skip_json_ws(p);
    if (**p == '{') {
        return parse_json_object(p);
    } else if (**p == '"') {
        char *s = parse_json_string(p);
        lpp_JsonNode *n = calloc(1, sizeof(lpp_JsonNode));
        n->type = 1;
        n->value.str_val = s;
        return n;
    } else if ((**p >= '0' && **p <= '9') || **p == '-') {
        char *end;
        long long val = strtoll(*p, &end, 10);
        *p = end;
        lpp_JsonNode *n = calloc(1, sizeof(lpp_JsonNode));
        n->type = 0;
        n->value.int_val = (int64_t)val;
        return n;
    }
    return NULL;
}

static lpp_JsonNode *parse_json_object(const char **p) {
    skip_json_ws(p);
    if (**p != '{') return NULL;
    (*p)++; // skip '{'
    
    lpp_JsonNode *head = NULL;
    lpp_JsonNode *tail = NULL;
    
    while (**p && **p != '}') {
        skip_json_ws(p);
        if (**p == '}') break;
        char *key = parse_json_string(p);
        skip_json_ws(p);
        if (**p != ':') {
            free(key);
            break;
        }
        (*p)++; // skip ':'
        lpp_JsonNode *val = parse_json_value(p);
        if (val) {
            val->key = key;
            if (!head) {
                head = val;
                tail = val;
            } else {
                tail->next = val;
                tail = val;
            }
        } else {
            free(key);
        }
        skip_json_ws(p);
        if (**p == ',') {
            (*p)++; // skip ','
        } else if (**p != '}') {
            break;
        }
    }
    if (**p == '}') (*p)++; // skip '}'
    
    lpp_JsonNode *n = calloc(1, sizeof(lpp_JsonNode));
    n->type = 2;
    n->value.obj_val = head;
    return n;
}

void *lpp_json_parse(const char *str) {
    if (!str) return NULL;
    const char *p = str;
    return parse_json_value(&p);
}

int64_t lpp_json_get_int(void *json, const char *key) {
    lpp_JsonNode *node = (lpp_JsonNode *)json;
    if (!node) return 0;
    if (node->type == 2) {
        lpp_JsonNode *curr = node->value.obj_val;
        while (curr) {
            if (curr->key && strcmp(curr->key, key) == 0) {
                if (curr->type == 0) return curr->value.int_val;
                return 0;
            }
            curr = curr->next;
        }
    }
    return 0;
}

const char *lpp_json_get_str(void *json, const char *key) {
    lpp_JsonNode *node = (lpp_JsonNode *)json;
    if (!node) return lpp_empty_str();
    if (node->type == 2) {
        lpp_JsonNode *curr = node->value.obj_val;
        while (curr) {
            if (curr->key && strcmp(curr->key, key) == 0) {
                if (curr->type == 1) return curr->value.str_val ? curr->value.str_val : lpp_empty_str();
                return lpp_empty_str();
            }
            curr = curr->next;
        }
    }
    return lpp_empty_str();
}

void *lpp_json_get_obj(void *json, const char *key) {
    lpp_JsonNode *node = (lpp_JsonNode *)json;
    if (!node) return NULL;
    if (node->type == 2) {
        lpp_JsonNode *curr = node->value.obj_val;
        while (curr) {
            if (curr->key && strcmp(curr->key, key) == 0) {
                if (curr->type == 2) return curr;
                return NULL;
            }
            curr = curr->next;
        }
    }
    return NULL;
}

static void lpp_json_free_node(lpp_JsonNode *node) {
    if (!node) return;
    if (node->key) free(node->key);
    if (node->type == 1) {
        if (node->value.str_val) lpp_arc_release(node->value.str_val);
    } else if (node->type == 2) {
        lpp_JsonNode *curr = node->value.obj_val;
        while (curr) {
            lpp_JsonNode *next = curr->next;
            lpp_json_free_node(curr);
            curr = next;
        }
    }
    free(node);
}

void lpp_json_free(void *json) {
    lpp_json_free_node((lpp_JsonNode *)json);
}

#include "runtime/lpp_str.c"
#define LPP_EXEC_EXCLUDE_BUILTINS
#include "runtime/lpp_exec.c"
#undef LPP_EXEC_EXCLUDE_BUILTINS
#include "runtime/lpp_dir.c"
#include "runtime/lpp_buf.c"
#include "runtime/lpp_map.c"
#include "runtime/lpp_gui.c"


/* Scalar reference for the explicit LLVM vector intrinsic. Cranelift calls
 * this implementation; LLVM lowers the same builtin to a <4 x i64> loop. */
#if defined(__GNUC__)
typedef int64_t lpp_i64x4 __attribute__((vector_size(32)));
__attribute__((target("avx2")))
static int64_t lpp_vec_i64_checksum_avx2(int64_t n) {
    int64_t total = 0;
    int64_t i = 0;
    const lpp_i64x4 three = {3, 3, 3, 3};
    while (i + 4 <= n) {
        lpp_i64x4 v = {i, i + 1, i + 2, i + 3};
        lpp_i64x4 x = (v * three) ^ (v >> 1);
        total += x[0] + x[1] + x[2] + x[3];
        i += 4;
    }
    for (; i < n; ++i) total += (i * 3) ^ (i >> 1);
    return total;
}
#endif
int64_t lpp_vec_i64_checksum(int64_t n) {
    if (n < 0) return 0;
#if defined(__GNUC__)
    return lpp_vec_i64_checksum_avx2(n);
#else
    int64_t total = 0;
    for (int64_t i = 0; i < n; ++i) total += (i * 3) ^ (i >> 1);
    return total;
#endif
}

int64_t lpp_command_exec(int64_t cmd_ptr) {
    if (!cmd_ptr) return -1;
    const char *cmd = (const char *)(intptr_t)cmd_ptr;
    int res = system(cmd);
    return (int64_t)res;
}

int64_t lpp_command_output(int64_t cmd_ptr) {
    if (!cmd_ptr) {
        char *empty = (char *)lpp_arc_alloc(1);
        empty[0] = '\0';
        return (int64_t)(intptr_t)empty;
    }
    const char *cmd = (const char *)(intptr_t)cmd_ptr;
    
#if defined(_WIN32)
    FILE *fp = _popen(cmd, "r");
#else
    FILE *fp = popen(cmd, "r");
#endif

    if (!fp) {
        char *empty = (char *)lpp_arc_alloc(1);
        empty[0] = '\0';
        return (int64_t)(intptr_t)empty;
    }
    
    size_t cap = 1024;
    size_t len = 0;
    char *buf = (char *)lpp_arc_alloc((int64_t)cap);
    if (!buf) {
#if defined(_WIN32)
        _pclose(fp);
#else
        pclose(fp);
#endif
        return 0;
    }
    
    while (1) {
        if (len + 256 > cap) {
            cap *= 2;
            char *new_buf = (char *)lpp_arc_alloc((int64_t)cap);
            if (!new_buf) break;
            memcpy(new_buf, buf, len);
            lpp_arc_release(buf);
            buf = new_buf;
        }
        size_t n = fread(buf + len, 1, 256, fp);
        if (n == 0) break;
        len += n;
    }
    buf[len] = '\0';
    
#if defined(_WIN32)
    _pclose(fp);
#else
    pclose(fp);
#endif
    return (int64_t)(intptr_t)buf;
}
