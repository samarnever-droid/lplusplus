/*
 * Freestanding Windows x86-64 direct-link runtime — Phase 4 complete.
 * Builtins: print, ARC, closures, lists, threads + 15 string/exec/dir + networking.
 * Dependencies: Kernel32 imports only (zero libc).  Merged by lpp-link PE.
 */
#include <stdint.h>
#include <stddef.h>
#include <intrin.h>
#ifndef LPP_FREESTANDING
#include <string.h>
#endif

#if defined(_WIN32)
#if defined(_MSC_VER)
#pragma comment(lib, "user32.lib")
#pragma comment(lib, "gdi32.lib")
#pragma comment(lib, "ws2_32.lib")
#endif
#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#endif

typedef void (*LppArcDestructor)(void *payload);

#ifndef STD_OUTPUT_HANDLE
#define STD_OUTPUT_HANDLE ((DWORD)-11)
#endif
#ifndef CREATE_NO_WINDOW
#define CREATE_NO_WINDOW 0x08000000
#endif

typedef STARTUPINFOA REAL_STARTUPINFOA;

/* Native common ARC header: magic@0, refcount@4, generation@8, flags@12.
 * Destructor-bearing objects store their destructor in an optional 8-byte
 * prefix immediately before this header; plain strings use only 16 bytes. */
#define LPP_ARC_IMMORTAL 0x41524331U
#define LPP_WIN_FLAG_ARENA       0x80000000u
#define LPP_WIN_FLAG_COLLECTIBLE 0x40000000u
#define LPP_WIN_FLAG_BUFFERED    0x20000000u
#define LPP_WIN_FLAG_HAS_DTOR    0x10000000u

typedef struct {
    uint32_t magic;
    volatile long refcount;
    unsigned int generation;
    uint32_t flags;
} LppArcHeader;
#if defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L
_Static_assert(sizeof(LppArcHeader) == 16, "native common ARC header must be 16 bytes");
_Static_assert(sizeof(LppArcDestructor) == 8, "native destructor prefix must be 8 bytes");
#else
typedef char lpp__assert_arc_header_size[(sizeof(LppArcHeader) == 16) ? 1 : -1];
typedef char lpp__assert_arc_dtor_size[(sizeof(LppArcDestructor) == 8) ? 1 : -1];
#endif

static LppArcDestructor lpp__header_destructor(const LppArcHeader *h) {
    if (!(h->flags & LPP_WIN_FLAG_HAS_DTOR)) return 0;
    return *(LppArcDestructor const *)((const char *)h - sizeof(LppArcDestructor));
}
static void *lpp__allocation_base(LppArcHeader *h) {
    if (h->flags & LPP_WIN_FLAG_HAS_DTOR) return (char *)h - sizeof(LppArcDestructor);
    return h;
}
static void lpp__teardown_info(LppArcHeader *h, void **base_out,
                               LppArcDestructor *destructor_out) {
    uint32_t flags = h->flags;
    if (flags & LPP_WIN_FLAG_HAS_DTOR) {
        void *base = (char *)h - sizeof(LppArcDestructor);
        *base_out = base;
        *destructor_out = *(LppArcDestructor const *)base;
    } else {
        *base_out = h;
        *destructor_out = 0;
    }
}
static int lpp__is_immortal(const LppArcHeader *h) {
    return h->magic == LPP_ARC_IMMORTAL && (unsigned)h->refcount == LPP_ARC_IMMORTAL;
}
/* Cycle-collector hooks (defined with the collector below), the
 * monotonic generation source, and the shared teardown counters. */
static volatile long lpp__generation_counter = 1;
uint64_t lpp__runtime_drop_count = 0;
uint64_t lpp__cycle_collected_count = 0;
static void lpp__note_possible_root(void *payload);
static void lpp__cycle_deregister(void *payload);
static int lpp__cycle_is_collectible(const LppArcHeader *h) { return (h->flags & LPP_WIN_FLAG_COLLECTIBLE) != 0; }
typedef struct { int64_t *data; int64_t len; int64_t cap; uint64_t data_bytes; int arc_elements; } LppList;

static uint64_t lpp_page_round(uint64_t s) { return (s+4095)&~4095ULL; }
static int lpp_strlen(const char *s) { int n=0; while(s&&s[n])n++; return n; }
static void lpp_memcpy(char *d, const char *s, int n) { if(d&&s&&n>0) __movsb((unsigned char*)d,(const unsigned char*)s,(size_t)n); }
static int lpp_strcmp(const char *a, const char *b) { while(*a&&*a==*b){a++;b++;} return *a-*b; }
static void lpp_strcpy(char *d, const char *s) { while((*d++=*s++)); }
static void lpp_memset(void *d, int v, size_t n) { if(d && n > 0) __stosb((unsigned char*)d, (unsigned char)v, n); }

static char* lpp_strdup(const char *s) { if(!s)return 0; int n=lpp_strlen(s); char *d=(char*)VirtualAlloc(0,lpp_page_round(n+1),MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE); if(d){lpp_memcpy(d,s,n);d[n]=0;} return d; }
static char* lpp_strstr(const char *h, const char *n) { int nl=lpp_strlen(n); if(!nl)return(char*)h; while(*h){int i=0;while(i<nl&&h[i]==n[i])i++;if(i==nl)return(char*)h;h++;} return 0; }
static int lpp_isspace(char c) { return c==' '||c=='\t'||c=='\n'||c=='\r'; }

/* MSVC intrinsic stubs */
#ifndef LPP_FREESTANDING
// Use CRT versions when linking with standard runtime
#else
#if !defined(__clang__)
#pragma function(memcpy)
#pragma function(memset)
#pragma function(strlen)
#pragma function(fmod)
#endif
void *memcpy(void *d, const void *s, size_t n) { if(d&&s&&n>0) __movsb((unsigned char*)d,(const unsigned char*)s,n); return d; }
void *memset(void *d, int c, size_t n) { if(d&&n>0) __stosb((unsigned char*)d,(unsigned char)c,n); return d; }
size_t strlen(const char *s) { size_t n=0; while(s&&s[n]) n++; return n; }
void __chkstk(void) {}
void ___chkstk_ms(void) {}
void __chkstk_ms(void) {}
#endif

static void lpp_write(const char *b, DWORD n) { DWORD w=0; WriteFile(GetStdHandle(STD_OUTPUT_HANDLE),b,n,&w,0); }
void lpp_print_int(int64_t v) { char b[32],*c=b+32; uint64_t m=v<0?(uint64_t)(-(v+1))+1:(uint64_t)v; *--c='\n'; do{*--c=(char)('0'+m%10);m/=10;}while(m); if(v<0)*--c='-'; lpp_write(c,(DWORD)((b+32)-c)); }

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
    lpp_write(cursor, (DWORD)((buffer + sizeof(buffer)) - cursor));
}
void lpp_print_bool(int8_t value) { lpp_print_int(value ? 1 : 0); }
void lpp_print_str(const char *t) { if(!t)return; int n=lpp_strlen(t); lpp_write(t,(DWORD)n); lpp_write("\n",1); }
void lpp_write_str(const char *t) { if(!t)return; int n=lpp_strlen(t); if(n>0) lpp_write(t,(DWORD)n); }

#pragma function(fmod)
double fmod(double x, double y) {
    if (y == 0.0) return 0.0;
    int64_t i = (int64_t)(x / y);
    return x - (double)i * y;
}

/* Small ARC objects dominate string-heavy loops. Reserving and committing a
 * whole 4 KiB VirtualAlloc mapping for every temporary made 100k-iteration
 * workloads perform hundreds of thousands of kernel calls. Use the same
 * bounded size-class strategy as the Linux freestanding runtime. All allocator
 * metadata is protected because the language also exposes `spawn`. */
#define LPP_WIN_CHUNK_BYTES  (1024 * 1024)
#define LPP_WIN_SIZE_CLASSES 8
#define LPP_WIN_ALLOC_TAG_MASK 0x0FFFFFFFu
#define LPP_WIN_ALLOC_LARGE 0x08000000u
#define LPP_WIN_ALLOC_LARGE_PAGES_MASK 0x07FFFFFFu
static void *lpp_win_free_lists[LPP_WIN_SIZE_CLASSES];
static char *lpp_win_bump_cursor;
static uint64_t lpp_win_bump_left;
static volatile long lpp_win_allocator_lock;

static uint64_t lpp_win_class_bytes(int cls) { return (uint64_t)32 << cls; }
static int lpp_win_class_for(uint64_t need) {
    int cls = 0;
    while (cls < LPP_WIN_SIZE_CLASSES && lpp_win_class_bytes(cls) < need) cls++;
    return cls;
}
static void lpp_win_allocator_acquire(void) {
    while (_InterlockedCompareExchange(&lpp_win_allocator_lock, 1, 0) != 0) {
        _mm_pause();
    }
}
static void lpp_win_allocator_release(void) {
    _InterlockedExchange(&lpp_win_allocator_lock, 0);
}

void *lpp_arc_alloc_with_destructor(int64_t sz, LppArcDestructor dtor) {
    if (sz < 0) return 0;
    uint64_t extra = dtor ? sizeof(LppArcDestructor) : 0;
    if ((uint64_t)sz > UINT64_MAX - sizeof(LppArcHeader) - extra) return 0;
    uint64_t need = (uint64_t)sz + sizeof(LppArcHeader) + extra;
    int cls = lpp_win_class_for(need);
    void *block = 0;
    uint32_t allocation_tag;

    if (cls < LPP_WIN_SIZE_CLASSES) {
        uint64_t bytes = lpp_win_class_bytes(cls);
        lpp_win_allocator_acquire();
        if (lpp_win_free_lists[cls]) {
            block = lpp_win_free_lists[cls];
            lpp_win_free_lists[cls] = *(void **)block;
        } else {
            if (lpp_win_bump_left < bytes) {
                char *chunk = (char *)VirtualAlloc(0, LPP_WIN_CHUNK_BYTES,
                    MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
                if (!chunk) {
                    lpp_win_allocator_release();
                    return 0;
                }
                lpp_win_bump_cursor = chunk;
                lpp_win_bump_left = LPP_WIN_CHUNK_BYTES;
            }
            block = lpp_win_bump_cursor;
            lpp_win_bump_cursor += bytes;
            lpp_win_bump_left -= bytes;
        }
        lpp_win_allocator_release();
        lpp_memset(block, 0, (size_t)bytes);
        allocation_tag = (uint32_t)(cls + 1);
    } else {
        if (need > UINT64_MAX - 4095) return 0;
        uint64_t total = lpp_page_round(need);
        uint64_t pages = total >> 12;
        if (pages == 0 || pages > LPP_WIN_ALLOC_LARGE_PAGES_MASK) return 0;
        block = VirtualAlloc(0, (SIZE_T)total, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
        if (!block) return 0;
        allocation_tag = LPP_WIN_ALLOC_LARGE | (uint32_t)pages;
    }

    LppArcHeader *h = (LppArcHeader *)((char *)block + extra);
    h->magic = LPP_ARC_IMMORTAL;
    h->refcount = 1;
    h->generation = (unsigned int)_InterlockedIncrement(&lpp__generation_counter);
    h->flags = allocation_tag | (dtor ? LPP_WIN_FLAG_HAS_DTOR : 0);
    if (dtor) *(LppArcDestructor *)block = dtor;
    return h + 1;
}
void *lpp_arc_alloc(int64_t sz) { return lpp_arc_alloc_with_destructor(sz,0); }

static int lpp_win_is_arena_header(const LppArcHeader *h) {
    return (h->flags & LPP_WIN_FLAG_ARENA) != 0;
}
static void lpp_arc_free_base(LppArcHeader *h, void *block) {
    if (!h || !block) return;
    uint32_t tag = h->flags & LPP_WIN_ALLOC_TAG_MASK;
    if (tag >= 1 && tag <= LPP_WIN_SIZE_CLASSES) {
        int cls = (int)(tag - 1);
        lpp_win_allocator_acquire();
        *(void **)block = lpp_win_free_lists[cls];
        lpp_win_free_lists[cls] = block;
        lpp_win_allocator_release();
    } else if (tag & LPP_WIN_ALLOC_LARGE) {
        VirtualFree(block, 0, MEM_RELEASE);
    }
}
static void lpp_arc_free(LppArcHeader *h) {
    if (h) lpp_arc_free_base(h, lpp__allocation_base(h));
}

typedef struct WinArenaRecord WinArenaRecord;
typedef struct WinArenaRegion WinArenaRegion;
struct WinArenaRecord { LppArcHeader *header; WinArenaRecord *next; };
struct WinArenaRegion { long refs; WinArenaRecord *records; WinArenaRegion *next; };
static WinArenaRegion *lpp_arena_regions;
void lpp_arc_retain(void *p);
void lpp_arc_release(void *p);

static WinArenaRegion *lpp_arena_for_header(LppArcHeader *header) {
    WinArenaRegion *r;
    for (r=lpp_arena_regions;r;r=r->next) {
        WinArenaRecord *n;
        for (n=r->records;n;n=n->next) if(n->header==header)return r;
    }
    return 0;
}
static void lpp_arena_destroy(WinArenaRegion *r) {
    WinArenaRegion **link=&lpp_arena_regions;
    while(*link&&*link!=r)link=&(*link)->next;
    if(*link==r)*link=r->next;
    WinArenaRecord *n=r->records;
    while(n){WinArenaRecord *next=n->next;lpp_arc_free(n->header);lpp_arc_release(n);n=next;}
    lpp_arc_release(r);
}
static void lpp_arena_node_zero(WinArenaRegion *r) { if(_InterlockedDecrement(&r->refs)==0)lpp_arena_destroy(r); }
void *lpp_arena_begin(void) { WinArenaRegion*r=(WinArenaRegion*)lpp_arc_alloc(sizeof(*r));if(!r)return 0;r->refs=1;r->records=0;r->next=lpp_arena_regions;lpp_arena_regions=r;return r; }
void lpp_arena_release(void *raw) { if(!raw)return;WinArenaRegion*r=(WinArenaRegion*)raw;if(_InterlockedDecrement(&r->refs)==0)lpp_arena_destroy(r); }
void *lpp_arena_alloc(int64_t size, void *raw, LppArcDestructor dtor) {
    if(!raw||size<0)return 0;WinArenaRegion*r=(WinArenaRegion*)raw;
    void*p=lpp_arc_alloc_with_destructor(size,dtor);if(!p)return 0;
    WinArenaRecord*n=(WinArenaRecord*)lpp_arc_alloc(sizeof(*n));
    if(!n){lpp_arc_release(p);return 0;}
    n->header=(LppArcHeader*)p-1;n->header->flags|=LPP_WIN_FLAG_ARENA;n->next=r->records;r->records=n;_InterlockedIncrement(&r->refs);return p;
}
void lpp_arena_retain(void *p) { if(!p)return;LppArcHeader*h=(LppArcHeader*)p-1;if(lpp_win_is_arena_header(h))lpp_arc_retain(p); }
void lpp_arena_release_node(void *p) { if(!p)return;LppArcHeader*h=(LppArcHeader*)p-1;if(!lpp_win_is_arena_header(h)||lpp__is_immortal(h))return;WinArenaRegion*r=lpp_arena_for_header(h);if(!r)return;long left=_InterlockedDecrement(&h->refcount);if(left==0){lpp__runtime_drop_count++;if(lpp__cycle_is_collectible(h))lpp__cycle_deregister(p);h->generation++;LppArcDestructor destructor=lpp__header_destructor(h);if(destructor)destructor(p);lpp_arena_node_zero(r);}else if(left>0)lpp__note_possible_root(p); }

/* Shared immortal empty string: runtime error paths return this instead of a
 * bare C literal, so every Str handed to generated code has a valid header. */
__declspec(align(16)) static const unsigned int lpp__empty_str_blob[8] = {
    LPP_ARC_IMMORTAL, LPP_ARC_IMMORTAL, 0, 0, 0, 0, 0, 0
};
char *lpp_empty_str(void) { return (char *)(const char *)&lpp__empty_str_blob[4]; }
#include "windows_x86_64_min/cycle.inc"
#include "windows_x86_64_min/arc_and_strings.inc"
#include "windows_x86_64_min/strings_exec_io.inc"
#include "windows_x86_64_min/io_files.inc"
#include "windows_x86_64_min/metrics_cptr.inc"
#include "windows_x86_64_min/gui_webview.inc"
#include "windows_x86_64_min/builtin_stubs.inc"
#include "windows_x86_64_min/json_stubs.inc"
#include "windows_x86_64_min/simd_concurrency.inc"
