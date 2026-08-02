/*
 * Freestanding Phase 2 ELF runtime.
 *
 * This runtime supports syscall-backed integer/string output, ARC memory,
 * dynamic lists, file I/O, and socket networking without libc.
 * Build with:
 * cc -O2 -ffreestanding -fno-stack-protector -fno-pic -mno-red-zone -c \
 *    runtime/linux_x86_64_min.c -o lpp_runtime_min.o
 */

#include <stdint.h>

void lpp_exit(int64_t code);

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

/* ── Floating-point math primitives ──────────────────────────────────────── */
double fmod(double x, double y) {
    if (y == 0.0) return 0.0;
    int64_t i = (int64_t)(x / y);
    return x - (double)i * y;
}

/* ── Freestanding ARC foundation ─────────────────────────────────────────── */
/* Every direct-link ARC allocation owns a whole mmap region. */

typedef void (*LppArcDestructor)(void *payload);
typedef struct {
    int refcount;
    /* Bumped immediately BEFORE the payload is released; a weak handle compares
     * against the value it captured. See lpp_weak_get. */
    int generation;
    LppArcDestructor destructor;
    uint64_t map_size;
} LppArcHeader;

/* ── Immortal objects ─────────────────────────────────────────────────────
 *
 * A string literal lives in .rodata and must never be freed -- but generated
 * code cannot tell it apart from a heap string, because both are just a
 * `char *`. Previously the compiler worked around this by refusing to treat
 * `Str` as owned at all, which traded a crash for an unbounded leak.
 *
 * Instead every string literal now carries a real 24-byte ARC header, emitted
 * into .rodata immediately before its bytes, whose refcount field holds this
 * sentinel. Retain and release test for it and return without touching memory.
 *
 * Two properties make this work:
 *
 *   * The check is a *read* of a mapped, read-only page. It never writes, so
 *     it cannot fault on .rodata. A plain "large refcount" would still be
 *     decremented, and the write would fault.
 *   * The sentinel value is the same 32-bit constant as LPP_ARC_MAGIC in the
 *     host runtime. That is deliberate: the host header places `magic` at
 *     offset 0 and `refcount` at offset 4, this header places `refcount` at
 *     offset 0, so a literal whose first *two* words are both the constant is
 *     simultaneously "valid magic" to the host runtime and "immortal" to both.
 *     One blob emitted by the compiler is correct under either runtime, which
 *     matters because the same object file links against either.
 *
 * A genuine object never reaches this count: it would need ~1.1 billion live
 * references, which cannot exist in an address space that must also hold them.
 */
#define LPP_ARC_IMMORTAL 0x41524331U

static inline int lpp__is_immortal(const LppArcHeader *header) {
    return (uint32_t)header->refcount == LPP_ARC_IMMORTAL;
}

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

/* ── Object allocator ─────────────────────────────────────────────────────
 *
 * Every ARC object used to own a whole mmap region: one syscall and one 4 KB
 * page for a 16-byte struct. Measured on a 3M-iteration allocation loop the
 * freestanding runtime took 18.3 s against the host runtime's 0.037 s -- a
 * 493x penalty, paid by the *default* linker.
 *
 * Objects are now carved from 1 MiB mmap'd chunks by a bump pointer, and freed
 * blocks go on a per-size-class free list for reuse, so the syscall count
 * tracks peak live memory rather than total allocations.
 *
 * The header's `map_size` field is reused to record the size class, leaving the
 * layout and every existing reader unchanged. A value of 0, or anything above
 * the class count, means "owns its own mmap region" -- the path large objects
 * still take.
 */

/* Monotonic source of object generations; see the host runtime for why this
 * must not be derived per-object. */
static int lpp__generation_counter = 1;

#define LPP_CHUNK_BYTES   (1024 * 1024)
#define LPP_SIZE_CLASSES  8

/* Class i holds blocks of 32 << i bytes: 32 .. 4096. */
static void *lpp_free_lists[LPP_SIZE_CLASSES];
static char *lpp_bump_cursor;
static uint64_t lpp_bump_left;

static uint64_t lpp_class_bytes(int cls) { return (uint64_t)32 << cls; }

static int lpp_class_for(uint64_t need) {
    int cls = 0;
    while (cls < LPP_SIZE_CLASSES && lpp_class_bytes(cls) < need) cls++;
    return cls;
}

void *lpp_arc_alloc_with_destructor(int64_t payload_size, LppArcDestructor destructor) {
    if (payload_size < 0) return 0;
    uint64_t need = (uint64_t)payload_size + sizeof(LppArcHeader);
    int cls = lpp_class_for(need);

    LppArcHeader *header;
    if (cls >= LPP_SIZE_CLASSES) {
        uint64_t total = lpp_page_round(need);
        header = (LppArcHeader *)lpp_sys_mmap(total);
        if (!header) return 0;
        header->map_size = total;
    } else {
        uint64_t bytes = lpp_class_bytes(cls);
        if (lpp_free_lists[cls]) {
            header = (LppArcHeader *)lpp_free_lists[cls];
            /* The first word of a free block is the next-free link. */
            lpp_free_lists[cls] = *(void **)header;
        } else {
            if (lpp_bump_left < bytes) {
                char *chunk = (char *)lpp_sys_mmap(LPP_CHUNK_BYTES);
                if (!chunk) return 0;
                lpp_bump_cursor = chunk;
                lpp_bump_left = LPP_CHUNK_BYTES;
            }
            header = (LppArcHeader *)lpp_bump_cursor;
            lpp_bump_cursor += bytes;
            lpp_bump_left -= bytes;
        }
        /* Recycled blocks must look freshly mapped: callers rely on
         * zero-initialised payload fields. The generation is assigned from the
         * process-global counter below, so a reused address never reuses a
         * generation. */
        char *raw = (char *)header;
        for (uint64_t i = 0; i < bytes; i++) raw[i] = 0;
        header->map_size = (uint64_t)(cls + 1);
    }
    header->refcount = 1;
    /* Generations come from a monotonic global, never restarting, so a stale
     * weak handle can never match a new occupant of a reused address. */
    header->generation = __atomic_add_fetch(&lpp__generation_counter, 1, __ATOMIC_RELAXED);
    header->destructor = destructor;
    return (void *)(header + 1);
}

void *lpp_arc_alloc(int64_t payload_size) {
    return lpp_arc_alloc_with_destructor(payload_size, 0);
}

/* Weak (non-owning) field support; see lpp_runtime.c for the ordering
 * argument. Free bumps the generation with release ordering before releasing
 * the block; reads load it with acquire ordering before dereferencing. */
int64_t lpp_weak_generation(void *payload) {
    if (!payload) return 0;
    LppArcHeader *header = (LppArcHeader *)payload - 1;
    /* An immortal target never dies, so any non-zero generation is stable. */
    if (lpp__is_immortal(header)) return (int64_t)LPP_ARC_IMMORTAL;
    return (int64_t)__atomic_load_n(&header->generation, __ATOMIC_ACQUIRE);
}

int64_t lpp_weak_get(int64_t raw, int64_t expected_generation) {
    void *payload = (void *)(intptr_t)raw;
    if (!payload || expected_generation == 0) return 0;
    LppArcHeader *header = (LppArcHeader *)payload - 1;
    if (lpp__is_immortal(header)) {
        return expected_generation == (int64_t)LPP_ARC_IMMORTAL ? raw : 0;
    }
    int now = __atomic_load_n(&header->generation, __ATOMIC_ACQUIRE);
    if ((int64_t)now != expected_generation) return 0;
    return raw;
}

typedef struct LppArenaRecord LppArenaRecord;
typedef struct LppArenaRegion LppArenaRegion;
static void lpp_arc_free(LppArcHeader *header);
void lpp_arc_retain(void *payload);
void lpp_arc_release(void *payload);
struct LppArenaRecord {
    LppArcHeader *header;
    LppArenaRecord *next;
};
struct LppArenaRegion {
    int refs; /* one owner handle plus one reference per node */
    LppArenaRecord *records;
    LppArenaRegion *next;
};
static LppArenaRegion *lpp_arena_regions;

static LppArenaRegion *lpp_arena_for_header(LppArcHeader *header) {
    for (LppArenaRegion *region = lpp_arena_regions; region; region = region->next) {
        for (LppArenaRecord *record = region->records; record; record = record->next) {
            if (record->header == header) return region;
        }
    }
    return 0;
}

static void lpp_arena_destroy(LppArenaRegion *region) {
    LppArenaRegion **link = &lpp_arena_regions;
    while (*link && *link != region) link = &(*link)->next;
    if (*link == region) *link = region->next;
    LppArenaRecord *records = region->records;
    while (records) {
        LppArenaRecord *next = records->next;
        /* Node destructors already ran when their refcounts reached zero. */
        lpp_arc_free(records->header);
        lpp_arc_release(records);
        records = next;
    }
    lpp_arc_release(region);
}

static void lpp_arena_node_zero(LppArenaRegion *region) {
    if (--region->refs == 0) lpp_arena_destroy(region);
}

void *lpp_arena_begin(void) {
    LppArenaRegion *region = (LppArenaRegion *)lpp_arc_alloc(sizeof(*region));
    if (!region) return 0;
    region->refs = 1;
    region->records = 0;
    region->next = lpp_arena_regions;
    lpp_arena_regions = region;
    return region;
}

void lpp_arena_release(void *raw_region) {
    if (!raw_region) return;
    LppArenaRegion *region = (LppArenaRegion *)raw_region;
    if (--region->refs == 0) lpp_arena_destroy(region);
}

void *lpp_arena_alloc(int64_t size, void *raw_region, LppArcDestructor destructor) {
    if (!raw_region || size < 0) return 0;
    LppArenaRegion *region = (LppArenaRegion *)raw_region;
    void *payload = lpp_arc_alloc_with_destructor(size, destructor);
    if (!payload) return 0;
    LppArenaRecord *record = (LppArenaRecord *)lpp_arc_alloc(sizeof(*record));
    if (!record) {
        lpp_arc_release(payload);
        return 0;
    }
    record->header = (LppArcHeader *)payload - 1;
    record->next = region->records;
    region->records = record;
    region->refs += 1;
    return payload;
}

void lpp_arena_retain(void *payload) {
    if (!payload) return;
    LppArcHeader *header = (LppArcHeader *)payload - 1;
    if (lpp_arena_for_header(header)) lpp_arc_retain(payload);
}

void lpp_arena_release_node(void *payload) {
    if (!payload) return;
    LppArcHeader *header = (LppArcHeader *)payload - 1;
    LppArenaRegion *region = lpp_arena_for_header(header);
    if (!region || lpp__is_immortal(header)) return;
    if (--header->refcount == 0) {
        (void)__atomic_add_fetch(&header->generation, 1, __ATOMIC_RELEASE);
        if (header->destructor) header->destructor(payload);
        lpp_arena_node_zero(region);
    }
}

void lpp_arc_retain(void *payload) {
    if (!payload) return;
    LppArcHeader *header = (LppArcHeader *)payload - 1;
    /* Immortal (string literal in .rodata): never counted, never freed, and
     * crucially never written to -- the page is read-only. */
    if (lpp__is_immortal(header)) return;
    (void)__atomic_add_fetch(&header->refcount, 1, __ATOMIC_ACQ_REL);
}

/* Return a dead object to its size-class free list, or unmap it if it owns a
 * whole region. `map_size` distinguishes the two: a value in 1..=classes is a
 * size class plus one, anything else is a byte count for munmap. */
static void lpp_arc_free(LppArcHeader *header) {
    uint64_t tag = header->map_size;
    if (tag >= 1 && tag <= LPP_SIZE_CLASSES) {
        int cls = (int)(tag - 1);
        *(void **)header = lpp_free_lists[cls];
        lpp_free_lists[cls] = (void *)header;
        return;
    }
    lpp_sys_munmap(header, tag);
}

void lpp_arc_release(void *payload) {
    if (!payload) return;
    LppArcHeader *header = (LppArcHeader *)payload - 1;
    if (lpp__is_immortal(header)) return;
    LppArenaRegion *arena = lpp_arena_for_header(header);
    if (__atomic_sub_fetch(&header->refcount, 1, __ATOMIC_ACQ_REL) == 0) {
        (void)__atomic_add_fetch(&header->generation, 1, __ATOMIC_RELEASE);
        if (header->destructor) header->destructor(payload);
        if (arena) lpp_arena_node_zero(arena);
        else lpp_arc_free(header);
    }
}

/* Non-atomic ARC, emitted when the compiler proves the program is
 * single-threaded. See the long comment in lpp_runtime.c.
 *
 * This build is doubly safe to use them: the freestanding runtime exposes no
 * thread primitive at all (no pthreads, no clone), so a program linked against
 * it cannot create a second thread even in principle. Every `lock xadd` it
 * executes today is pure overhead with nothing to synchronise against. */
void lpp_arc_retain_local(void *payload) {
    if (!payload) return;
    LppArcHeader *header = (LppArcHeader *)payload - 1;
    if (lpp__is_immortal(header)) return;
    __atomic_add_fetch(&header->refcount, 1, __ATOMIC_RELAXED);
}

void lpp_arc_release_local(void *payload) {
    if (!payload) return;
    LppArcHeader *header = (LppArcHeader *)payload - 1;
    if (lpp__is_immortal(header)) return;
    if (__atomic_sub_fetch(&header->refcount, 1, __ATOMIC_RELAXED) == 0) {
        LppArenaRegion *arena = lpp_arena_for_header(header);
        (void)__atomic_add_fetch(&header->generation, 1, __ATOMIC_RELEASE);
        if (header->destructor) header->destructor(payload);
        if (arena) lpp_arena_node_zero(arena);
        else lpp_arc_free(header);
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

typedef struct { uint64_t managed_mask; uint64_t packed_offsets; } LppTuplePrefix;
static void lpp_tuple_destroy(void *payload) {
    LppTuplePrefix *tuple = (LppTuplePrefix *)payload;
    for (unsigned i = 0; tuple && i < 4; ++i) {
        if ((tuple->managed_mask & ((uint64_t)1 << i)) == 0) continue;
        uint64_t offset = (tuple->packed_offsets >> (i * 16)) & 0xffffu;
        lpp_arc_release(*(void **)((char *)payload + offset));
    }
}
void *lpp_tuple_alloc(int64_t size, int64_t mask, int64_t offsets) {
    if (size < 16) lpp_exit(101);
    LppTuplePrefix *tuple = (LppTuplePrefix *)lpp_arc_alloc_with_destructor(size, lpp_tuple_destroy);
    if (!tuple) lpp_exit(101);
    tuple->managed_mask = (uint64_t)mask;
    tuple->packed_offsets = (uint64_t)offsets;
    return tuple;
}

typedef int64_t (*LppTaskCode)(void *);
typedef struct {
    LppTaskCode code; void *environment; int64_t result; int64_t state; int64_t result_managed;
} LppTask;
static void lpp_task_payload_destroy(void *payload) {
    LppTask *task = (LppTask *)payload;
    if (!task) return;
    if (task->environment) { lpp_arc_release(task->environment); task->environment = 0; }
    if (task->state == 2 && task->result_managed && task->result) {
        lpp_arc_release((void *)(intptr_t)task->result); task->result = 0;
    }
}
void *lpp_task_new(void *code, void *environment, int64_t managed) {
    if (!code || !environment) lpp_exit(101);
    LppTask *task = (LppTask *)lpp_arc_alloc_with_destructor(sizeof(LppTask), lpp_task_payload_destroy);
    if (!task) lpp_exit(101);
    task->code = (LppTaskCode)code; task->environment = environment; task->result_managed = managed != 0;
    return task;
}
int64_t lpp_task_poll(void *raw) {
    LppTask *task = (LppTask *)raw;
    if (!task || task->state == 1) lpp_exit(101);
    if (task->state == 2) return 1;
    task->state = 1; task->result = task->code(task->environment); task->state = 2; return 1;
}
int64_t lpp_executor_run(void *raw) { (void)lpp_task_poll(raw); return ((LppTask *)raw)->result; }
int64_t lpp_task_await(void *raw) {
    LppTask *task = (LppTask *)raw; int64_t result = lpp_executor_run(raw);
    if (task->result_managed && result) lpp_arc_retain((void *)(intptr_t)result);
    return result;
}
void lpp_task_destroy(void *raw) { lpp_arc_release(raw); }

/* ── Freestanding List runtime ──────────────────────────────────────────── */
typedef struct {
    int64_t *data;
    int64_t len;
    int64_t cap;
    uint64_t data_map_size;
    int arc_elements;
} LppList;

static void lpp_list_destroy(void *payload) {
    LppList *list = (LppList *)payload;
    if (!list) return;
    if (list->arc_elements) {
        for (int64_t i = 0; i < list->len; ++i) {
            lpp_arc_release((void *)(intptr_t)list->data[i]);
        }
    }
    if (list->data) lpp_sys_munmap(list->data, list->data_map_size);
}

static void *lpp_list_new_with_mode(int arc_elements) {
    LppList *list = (LppList *)lpp_arc_alloc_with_destructor(
        (int64_t)sizeof(LppList), lpp_list_destroy
    );
    if (!list) return 0;
    list->arc_elements = arc_elements;
    return list;
}

void *lpp_list_new(void) {
    return lpp_list_new_with_mode(0);
}

void *lpp_list_new_arc(void) {
    return lpp_list_new_with_mode(1);
}

void lpp_list_push(void *raw, int64_t value) {
    LppList *list = (LppList *)raw;
    if (!list) return;
    if (list->len == list->cap) {
        int64_t next_cap = list->cap == 0 ? 8 : list->cap * 2;
        if (next_cap < list->cap || next_cap > (int64_t)(0x7fffffffffffffffLL / 8)) return;
        uint64_t next_size = lpp_page_round((uint64_t)next_cap * sizeof(int64_t));
        int64_t *next_data = (int64_t *)lpp_sys_mmap(next_size);
        if (!next_data) return;
        for (int64_t i = 0; i < list->len; ++i) next_data[i] = list->data[i];
        if (list->data) lpp_sys_munmap(list->data, list->data_map_size);
        list->data = next_data;
        list->cap = next_cap;
        list->data_map_size = next_size;
    }
    if (list->arc_elements) lpp_arc_retain((void *)(intptr_t)value);
    list->data[list->len++] = value;
}

void lpp_list_push_arc(void *list, void *value) {
    lpp_list_push(list, (int64_t)(intptr_t)value);
}

void lpp_list_push_float(void *list, double value) {
    int64_t ival;
    for (int i = 0; i < 8; i++) ((char*)&ival)[i] = ((char*)&value)[i];
    lpp_list_push(list, ival);
}

void lpp_list_push_bool(void *list, int8_t value) {
    lpp_list_push(list, value ? 1 : 0);
}

int64_t lpp_list_get(void *raw, int64_t index) {
    LppList *list = (LppList *)raw;
    if (!list || index < 0 || index >= list->len) { __asm__ volatile("syscall"::"a"(60),"D"(101):"rcx","r11"); for(;;); }
    return list->data[index];
}

void lpp_list_set(void *raw, int64_t index, int64_t value) {
    LppList *list = (LppList *)raw;
    if (!list || index < 0 || index >= list->len) lpp_exit(101);
    if (list->arc_elements) {
        lpp_arc_retain((void *)(intptr_t)value);
        lpp_arc_release((void *)(intptr_t)list->data[index]);
    }
    list->data[index] = value;
}

void lpp_list_set_bool(void *list, int64_t index, int8_t value) {
    lpp_list_set(list, index, value ? 1 : 0);
}

void lpp_list_set_float(void *list, int64_t index, double value) {
    int64_t bits;
    for (int i = 0; i < 8; ++i) ((char *)&bits)[i] = ((char *)&value)[i];
    lpp_list_set(list, index, bits);
}

void lpp_list_set_arc(void *list, int64_t index, void *value) {
    lpp_list_set(list, index, (int64_t)(intptr_t)value);
}

double lpp_list_get_float(void *list, int64_t index) {
    int64_t ival = lpp_list_get(list, index);
    double fval;
    for (int i = 0; i < 8; i++) ((char*)&fval)[i] = ((char*)&ival)[i];
    return fval;
}

int8_t lpp_list_get_bool(void *list, int64_t index) {
    return lpp_list_get(list, index) != 0;
}

void *lpp_list_get_arc(void *list, int64_t index) {
    return (void *)(intptr_t)lpp_list_get(list, index);
}

int64_t lpp_list_len(void *raw) {
    LppList *list = (LppList *)raw;
    return list ? list->len : 0;
}

void lpp_list_free(void *list) {
    lpp_arc_release(list);
}

typedef struct { void *base; int64_t start, length, generation, kind; } LppSlice;
static void *lpp_slice_checked_base(LppSlice *view) {
    if (!view || !view->base || !view->generation) lpp_exit(101);
    int64_t raw = lpp_weak_get((int64_t)(intptr_t)view->base, view->generation);
    if (!raw) lpp_exit(101);
    return (void *)(intptr_t)raw;
}
void *lpp_slice_init(void *storage, void *base, int64_t start, int64_t length, int64_t kind) {
    if (!storage || !base || start < 0 || length < 0 || start > 0x7fffffffffffffffLL - length) lpp_exit(101);
    int64_t source_length = 0;
    if (kind == 0) { const char *p = (const char *)base; while (p[source_length]) source_length++; }
    else source_length = lpp_list_len(base);
    if (start > source_length || length > source_length - start) lpp_exit(101);
    LppSlice *view = (LppSlice *)storage;
    view->base = base; view->start = start; view->length = length;
    view->generation = lpp_weak_generation(base); view->kind = kind;
    if (!view->generation) lpp_exit(101);
    return view;
}
int64_t lpp_slice_len(void *raw) { LppSlice *view=(LppSlice *)raw; (void)lpp_slice_checked_base(view); return view->length; }
int64_t lpp_slice_get(void *raw, int64_t index) {
    LppSlice *view=(LppSlice *)raw; void *base=lpp_slice_checked_base(view);
    if (view->kind != 1 || index < 0 || index >= view->length) lpp_exit(101);
    return lpp_list_get(base, view->start + index);
}
double lpp_slice_get_float(void *raw, int64_t index) {
    int64_t bits = lpp_slice_get(raw, index);
    double value;
    for (int i = 0; i < 8; i++) {
        ((char *)&value)[i] = ((char *)&bits)[i];
    }
    return value;
}
int8_t lpp_slice_get_bool(void *raw, int64_t index) {
    return lpp_slice_get(raw, index) != 0;
}
char *lpp_str_slice_get(void *raw, int64_t index) {
    LppSlice *view = (LppSlice *)raw;
    const char *base = (const char *)lpp_slice_checked_base(view);
    if (view->kind != 0 || index < 0 || index >= view->length) lpp_exit(101);
    char *result = (char *)lpp_arc_alloc(2);
    if (!result) lpp_exit(101);
    result[0] = base[view->start + index];
    result[1] = 0;
    return result;
}
char *lpp_str_slice_to_str(void *raw) {
    LppSlice *view = (LppSlice *)raw;
    const char *base = (const char *)lpp_slice_checked_base(view);
    if (view->kind != 0) lpp_exit(101);
    char *result = (char *)lpp_arc_alloc(view->length + 1);
    if (!result) lpp_exit(101);
    for (int64_t i = 0; i < view->length; i++) {
        result[i] = base[view->start + i];
    }
    result[view->length] = 0;
    return result;
}

/* ── Freestanding File I/O ─────────────────────────────────────────── */

static long lpp_sys_open(const char *path, int flags, int mode) {
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(2), "D"(path), "S"((long)flags), "d"((long)mode)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static long lpp_sys_read(long fd, void *buf, long count) {
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(0), "D"(fd), "S"(buf), "d"(count)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static long lpp_sys_close(long fd) {
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(3), "D"(fd)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static long lpp_sys_lseek(long fd, long offset, int whence) {
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(8), "D"(fd), "S"(offset), "d"((long)whence)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static long lpp_sys_unlink(const char *path) {
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(87), "D"(path)
        : "rcx", "r11", "memory"
    );
    return ret;
}

char* lpp_read_file(const char *filename) {
    if (!filename) return (char*)"";
    long fd = lpp_sys_open(filename, 0, 0); /* O_RDONLY */
    if (fd < 0) return (char*)"";
    long len = lpp_sys_lseek(fd, 0, 2); /* SEEK_END */
    if (len < 0) {
        lpp_sys_close(fd);
        return (char*)"";
    }
    (void)lpp_sys_lseek(fd, 0, 0); /* SEEK_SET */
    char *buf = (char*)lpp_arc_alloc(len + 1);
    if (!buf) {
        lpp_sys_close(fd);
        return (char*)"";
    }
    long bytes_read = lpp_sys_read(fd, buf, len);
    lpp_sys_close(fd);
    if (bytes_read < 0) bytes_read = 0;
    buf[bytes_read] = '\0';
    return buf;
}

int64_t lpp_write_file(const char *filename, const char *content) {
    if (!filename || !content) return 0;
    long fd = lpp_sys_open(filename, 0101, 0644); /* O_WRONLY | O_CREAT | O_TRUNC */
    if (fd < 0) return 0;
    long clen = 0;
    while (content[clen]) clen++;
    long written = lpp_sys_write(fd, content, clen);
    lpp_sys_close(fd);
    return written >= 0 ? 1 : 0;
}

int64_t lpp_append_file(const char *filename, const char *content) {
    if (!filename || !content) return 0;
    long fd = lpp_sys_open(filename, 02001, 0644); /* O_WRONLY | O_CREAT | O_APPEND */
    if (fd < 0) return 0;
    long clen = 0;
    while (content[clen]) clen++;
    long written = lpp_sys_write(fd, content, clen);
    lpp_sys_close(fd);
    return written >= 0 ? 1 : 0;
}

int64_t lpp_delete_file(const char *filename) {
    if (!filename) return 0;
    return lpp_sys_unlink(filename) == 0 ? 1 : 0;
}

int64_t lpp_file_exists(const char *filename) {
    if (!filename) return 0;
    long fd = lpp_sys_open(filename, 0, 0);
    if (fd >= 0) {
        lpp_sys_close(fd);
        return 1;
    }
    return 0;
}

int64_t lpp_file_size(const char *filename) {
    if (!filename) return 0;
    long fd = lpp_sys_open(filename, 0, 0);
    if (fd < 0) return 0;
    long sz = lpp_sys_lseek(fd, 0, 2);
    lpp_sys_close(fd);
    return sz >= 0 ? (int64_t)sz : 0;
}

/* ── Freestanding Socket Networking ────────────────────────── */

struct lpp_sockaddr_in {
    uint16_t sin_family;
    uint16_t sin_port;
    uint32_t sin_addr;
    char sin_zero[8];
};

static uint16_t lpp_htons(uint16_t val) {
    return (uint16_t)((val << 8) | (val >> 8));
}

static long lpp_sys_socket(int domain, int type, int protocol) {
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(41), "D"((long)domain), "S"((long)type), "d"((long)protocol)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static long lpp_sys_connect(long fd, const void *addr, int addrlen) {
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(42), "D"(fd), "S"(addr), "d"((long)addrlen)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static long lpp_sys_accept(long fd, void *addr, void *addrlen) {
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(43), "D"(fd), "S"(addr), "d"(addrlen)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static long lpp_sys_sendto(long fd, const void *buf, long len, int flags) {
    long ret;
    register long r10 __asm__("r10") = (long)flags;
    register long r8 __asm__("r8") = 0;
    register long r9 __asm__("r9") = 0;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(44), "D"(fd), "S"(buf), "d"(len), "r"(r10), "r"(r8), "r"(r9)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static long lpp_sys_recvfrom(long fd, void *buf, long len, int flags) {
    long ret;
    register long r10 __asm__("r10") = (long)flags;
    register long r8 __asm__("r8") = 0;
    register long r9 __asm__("r9") = 0;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(45), "D"(fd), "S"(buf), "d"(len), "r"(r10), "r"(r8), "r"(r9)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static long lpp_sys_bind(long fd, const void *addr, int addrlen) {
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(49), "D"(fd), "S"(addr), "d"((long)addrlen)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static long lpp_sys_listen(long fd, int backlog) {
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(50), "D"(fd), "S"((long)backlog)
        : "rcx", "r11", "memory"
    );
    return ret;
}

int64_t lpp_net_listen(int64_t port) {
    long sock = lpp_sys_socket(2, 1, 0); /* AF_INET, SOCK_STREAM */
    if (sock < 0) return -1;
    struct lpp_sockaddr_in addr = {0};
    addr.sin_family = 2;
    addr.sin_port = lpp_htons((uint16_t)port);
    addr.sin_addr = 0; /* INADDR_ANY */
    if (lpp_sys_bind(sock, &addr, sizeof(addr)) < 0) {
        lpp_sys_close(sock);
        return -1;
    }
    if (lpp_sys_listen(sock, 128) < 0) {
        lpp_sys_close(sock);
        return -1;
    }
    return (int64_t)sock;
}

int64_t lpp_net_accept(int64_t listener) {
    if (listener < 0) return -1;
    long client = lpp_sys_accept((long)listener, 0, 0);
    return (int64_t)client;
}

int64_t lpp_net_connect(const char *host, int64_t port) {
    (void)host;
    long sock = lpp_sys_socket(2, 1, 0);
    if (sock < 0) return -1;
    struct lpp_sockaddr_in addr = {0};
    addr.sin_family = 2;
    addr.sin_port = lpp_htons((uint16_t)port);
    addr.sin_addr = 0x0100007f; /* 127.0.0.1 in network byte order */
    if (lpp_sys_connect(sock, &addr, sizeof(addr)) < 0) {
        lpp_sys_close(sock);
        return -1;
    }
    return (int64_t)sock;
}

int64_t lpp_net_send(int64_t fd, const char *data) {
    if (fd < 0 || !data) return -1;
    long len = 0;
    while (data[len]) len++;
    long sent = lpp_sys_sendto((long)fd, data, len, 0x4000); /* MSG_NOSIGNAL */
    return (int64_t)sent;
}

int64_t lpp_net_send_all(int64_t fd, const char *data) {
    if (fd < 0 || !data) return -1;
    long total = 0;
    long len = 0;
    while (data[len]) len++;
    while (total < len) {
        long sent = lpp_sys_sendto((long)fd, data + total, len - total, 0x4000);
        if (sent <= 0) break;
        total += sent;
    }
    return (int64_t)total;
}

char* lpp_net_recv(int64_t fd, int64_t max_bytes) {
    if (fd < 0 || max_bytes <= 0) return (char*)"";
    char *buf = (char*)lpp_arc_alloc(max_bytes + 1);
    if (!buf) return (char*)"";
    long recvd = lpp_sys_recvfrom((long)fd, buf, max_bytes, 0);
    if (recvd < 0) recvd = 0;
    buf[recvd] = '\0';
    return buf;
}

void lpp_net_close(int64_t fd) {
    if (fd >= 0) {
        lpp_sys_close((long)fd);
    }
}

int64_t lpp_net_set_timeout(int64_t fd, int64_t ms) {
    (void)fd; (void)ms;
    return 1;
}

/* ── Freestanding Map Runtime ───────────────────────────────────────────── */
typedef struct LppMapEntry {
    int64_t key;
    int64_t val;
    int is_str_key;
    int occupied;
} LppMapEntry;

typedef struct LppMap {
    LppMapEntry *entries;
    int64_t cap;
    int64_t len;
    uint64_t entries_map_size;
    int arc_values; /* 1 = values are ARC-managed pointers */
} LppMap;

static uint64_t lpp_hash_str(const char *s) {
    if (!s) return 0;
    uint64_t hash = 14695981039346656037ULL;
    while (*s) {
        hash ^= (unsigned char)(*s++);
        hash *= 1099511628211ULL;
    }
    return hash;
}

static uint64_t lpp_hash_int(int64_t key) {
    uint64_t k = (uint64_t)key;
    k = (~k) + (k << 21);
    k = k ^ (k >> 24);
    k = (k + (k << 3)) + (k << 8);
    k = k ^ (k >> 14);
    k = (k + (k << 2)) + (k << 4);
    k = k ^ (k >> 28);
    k = k + (k << 31);
    return k;
}

static int lpp_map_key_equal(int64_t k1, int64_t k2) {
    if (k1 == k2) return 1;
    if (k1 >= 0x400000 && k2 >= 0x400000) {
        const char *s1 = (const char *)(uintptr_t)k1;
        const char *s2 = (const char *)(uintptr_t)k2;
        int i = 0;
        while (s1[i] && s1[i] == s2[i]) i++;
        if (s1[i] == s2[i]) return 1;
    }
    return 0;
}

void lpp_map_destroy(void *payload) {
    LppMap *m = (LppMap *)payload;
    if (!m) return;
    if (m->arc_values && m->entries) {
        for (int64_t i = 0; i < m->cap; i++) {
            if (m->entries[i].occupied == 1) {
                lpp_arc_release((void *)(uintptr_t)m->entries[i].val);
            }
        }
    }
    if (m->entries) lpp_sys_munmap(m->entries, m->entries_map_size);
    m->entries = 0;
    m->cap = 0;
    m->len = 0;
}

static void *lpp_map_new_with_mode(int arc_values) {
    LppMap *m = (LppMap *)lpp_arc_alloc_with_destructor((int64_t)sizeof(LppMap), lpp_map_destroy);
    if (!m) return 0;
    m->cap = 16;
    m->len = 0;
    m->arc_values = arc_values;
    m->entries_map_size = lpp_page_round((uint64_t)m->cap * sizeof(LppMapEntry));
    m->entries = (LppMapEntry *)lpp_sys_mmap(m->entries_map_size);
    if (!m->entries) lpp_exit(101);
    return m;
}

void *lpp_map_new(void) {
    return lpp_map_new_with_mode(0);
}

void *lpp_map_new_arc(void) {
    return lpp_map_new_with_mode(1);
}

static void lpp_map_rehash(LppMap *m, int64_t new_cap) {
    int64_t old_cap = m->cap;
    LppMapEntry *old_entries = m->entries;
    uint64_t old_size = m->entries_map_size;

    m->cap = new_cap;
    m->entries_map_size = lpp_page_round((uint64_t)m->cap * sizeof(LppMapEntry));
    m->entries = (LppMapEntry *)lpp_sys_mmap(m->entries_map_size);
    if (!m->entries) lpp_exit(101);
    m->len = 0;

    for (int64_t i = 0; i < old_cap; i++) {
        if (old_entries[i].occupied == 1) {
            int64_t key = old_entries[i].key;
            int64_t val = old_entries[i].val;
            int is_str = old_entries[i].is_str_key;
            uint64_t h = is_str ? lpp_hash_str((const char *)(uintptr_t)key) : lpp_hash_int(key);
            int64_t idx = (int64_t)(h % (uint64_t)m->cap);
            while (m->entries[idx].occupied == 1) {
                idx = (idx + 1) % m->cap;
            }
            m->entries[idx].key = key;
            m->entries[idx].val = val;
            m->entries[idx].is_str_key = is_str;
            m->entries[idx].occupied = 1;
            m->len++;
        }
    }
    if (old_entries) lpp_sys_munmap(old_entries, old_size);
}

static void lpp_map_put_internal(LppMap *m, int64_t key, int64_t val, int is_str) {
    if (!m) return;
    int64_t occupied = 0;
    for (int64_t i = 0; i < m->cap; i++) {
        if (m->entries[i].occupied != 0) occupied++;
    }
    if (occupied * 10 >= m->cap * 7) {
        int64_t new_cap = (m->len * 100 < m->cap * 35) ? m->cap : m->cap * 2;
        if (new_cap < 16) new_cap = 16;
        lpp_map_rehash(m, new_cap);
    }

    uint64_t h = is_str ? lpp_hash_str((const char *)(uintptr_t)key) : lpp_hash_int(key);
    int64_t idx = (int64_t)(h % (uint64_t)m->cap);
    int64_t first_tombstone = -1;

    while (m->entries[idx].occupied != 0) {
        if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == is_str) {
            int match = is_str
                ? lpp_map_key_equal(m->entries[idx].key, key)
                : (m->entries[idx].key == key);
            if (match) {
                if (m->arc_values) {
                    lpp_arc_retain((void *)(uintptr_t)val);
                    lpp_arc_release((void *)(uintptr_t)m->entries[idx].val);
                }
                m->entries[idx].val = val;
                return;
            }
        }
        if (m->entries[idx].occupied == 2 && first_tombstone == -1) {
            first_tombstone = idx;
        }
        idx = (idx + 1) % m->cap;
    }

    if (first_tombstone != -1) {
        idx = first_tombstone;
    }

    if (m->arc_values) lpp_arc_retain((void *)(uintptr_t)val);
    m->entries[idx].key = key;
    m->entries[idx].val = val;
    m->entries[idx].is_str_key = is_str;
    m->entries[idx].occupied = 1;
    m->len++;
}

void lpp_map_put(void *map, int64_t key, int64_t val) {
    lpp_map_put_internal((LppMap *)map, key, val, 0);
}

void lpp_map_put_str(void *map, const char *key, int64_t val) {
    lpp_map_put_internal((LppMap *)map, (int64_t)(uintptr_t)key, val, 1);
}

int64_t lpp_map_get(void *map, int64_t key) {
    LppMap *m = (LppMap *)map;
    if (!m || m->len == 0) return 0;

    uint64_t h = lpp_hash_int(key);
    int64_t idx = (int64_t)(h % (uint64_t)m->cap);
    int64_t start_idx = idx;

    while (m->entries[idx].occupied != 0) {
        if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 0 && m->entries[idx].key == key) {
            return m->entries[idx].val;
        }
        idx = (idx + 1) % m->cap;
        if (idx == start_idx) break;
    }
    return 0;
}

int64_t lpp_map_get_str(void *map, const char *key) {
    LppMap *m = (LppMap *)map;
    if (!m || !key || m->len == 0) return 0;

    uint64_t h = lpp_hash_str(key);
    int64_t idx = (int64_t)(h % (uint64_t)m->cap);
    int64_t start_idx = idx;

    while (m->entries[idx].occupied != 0) {
        if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 1) {
            if (lpp_map_key_equal(m->entries[idx].key, (int64_t)(uintptr_t)key)) {
                return m->entries[idx].val;
            }
        }
        idx = (idx + 1) % m->cap;
        if (idx == start_idx) break;
    }
    return 0;
}

int64_t lpp_map_has(void *map, int64_t key) {
    LppMap *m = (LppMap *)map;
    if (!m || m->len == 0) return 0;

    uint64_t h = lpp_hash_int(key);
    int64_t idx = (int64_t)(h % (uint64_t)m->cap);
    int64_t start_idx = idx;

    while (m->entries[idx].occupied != 0) {
        if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 0 && m->entries[idx].key == key) {
            return 1;
        }
        idx = (idx + 1) % m->cap;
        if (idx == start_idx) break;
    }
    return 0;
}

int64_t lpp_map_has_str(void *map, const char *key) {
    LppMap *m = (LppMap *)map;
    if (!m || !key || m->len == 0) return 0;

    uint64_t h = lpp_hash_str(key);
    int64_t idx = (int64_t)(h % (uint64_t)m->cap);
    int64_t start_idx = idx;

    while (m->entries[idx].occupied != 0) {
        if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 1) {
            if (lpp_map_key_equal(m->entries[idx].key, (int64_t)(uintptr_t)key)) {
                return 1;
            }
        }
        idx = (idx + 1) % m->cap;
        if (idx == start_idx) break;
    }
    return 0;
}

int64_t lpp_map_len(void *map) {
    LppMap *m = (LppMap *)map;
    return m ? m->len : 0;
}

void lpp_map_remove(void *map, int64_t key) {
    LppMap *m = (LppMap *)map;
    if (!m || m->len == 0) return;

    uint64_t h = lpp_hash_int(key);
    int64_t idx = (int64_t)(h % (uint64_t)m->cap);
    int64_t start_idx = idx;

    while (m->entries[idx].occupied != 0) {
        if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 0 && m->entries[idx].key == key) {
            if (m->arc_values) lpp_arc_release((void *)(uintptr_t)m->entries[idx].val);
            m->entries[idx].occupied = 2;
            m->len--;
            return;
        }
        idx = (idx + 1) % m->cap;
        if (idx == start_idx) break;
    }
}

void lpp_map_remove_str(void *map, const char *key) {
    LppMap *m = (LppMap *)map;
    if (!m || !key || m->len == 0) return;

    uint64_t h = lpp_hash_str(key);
    int64_t idx = (int64_t)(h % (uint64_t)m->cap);
    int64_t start_idx = idx;

    while (m->entries[idx].occupied != 0) {
        if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 1) {
            if (lpp_map_key_equal(m->entries[idx].key, (int64_t)(uintptr_t)key)) {
                if (m->arc_values) lpp_arc_release((void *)(uintptr_t)m->entries[idx].val);
                m->entries[idx].occupied = 2;
                m->len--;
                return;
            }
        }
        idx = (idx + 1) % m->cap;
        if (idx == start_idx) break;
    }
}

void lpp_map_put_float(void *map, int64_t key, double val) {
    int64_t ival;
    for (int i = 0; i < 8; i++) ((char*)&ival)[i] = ((char*)&val)[i];
    lpp_map_put(map, key, ival);
}

double lpp_map_get_float(void *map, int64_t key) {
    int64_t ival = lpp_map_get(map, key);
    double fval;
    for (int i = 0; i < 8; i++) ((char*)&fval)[i] = ((char*)&ival)[i];
    return fval;
}

void lpp_map_put_str_float(void *map, const char *key, double val) {
    int64_t ival;
    for (int i = 0; i < 8; i++) ((char*)&ival)[i] = ((char*)&val)[i];
    lpp_map_put_str(map, key, ival);
}

double lpp_map_get_str_float(void *map, const char *key) {
    int64_t ival = lpp_map_get_str(map, key);
    double fval;
    for (int i = 0; i < 8; i++) ((char*)&fval)[i] = ((char*)&ival)[i];
    return fval;
}

/* ── String builtins (freestanding, using lpp_alloc/lpp_sys_mmap) ── */

static int64_t lpp_strlen(const char *s) {
    if (!s) return 0;
    int64_t n = 0;
    while (s[n]) n++;
    return n;
}

int64_t lpp_str_len(const char *s) {
    return lpp_strlen(s);
}

char *lpp_str_concat(const char *a, const char *b) {
    int64_t alen = lpp_strlen(a);
    int64_t blen = lpp_strlen(b);
    char *out = (char *)lpp_alloc(alen + blen + 1);
    for (int64_t i = 0; i < alen; i++) out[i] = a[i];
    for (int64_t i = 0; i < blen; i++) out[alen + i] = b[i];
    out[alen + blen] = 0;
    return out;
}

char *lpp_str_substr(const char *s, int64_t start, int64_t length) {
    if (!s) return (char *)lpp_alloc(1);
    int64_t slen = lpp_strlen(s);
    if (start < 0) start = 0;
    if (start >= slen || length <= 0) {
        char *out = (char *)lpp_alloc(1);
        out[0] = 0;
        return out;
    }
    if (start + length > slen) length = slen - start;
    char *out = (char *)lpp_alloc(length + 1);
    for (int64_t i = 0; i < length; i++) out[i] = s[start + i];
    out[length] = 0;
    return out;
}

char *lpp_str_repeat(const char *s, int64_t n) {
    if (!s || n <= 0) { char *e = (char *)lpp_alloc(1); e[0] = 0; return e; }
    int64_t slen = lpp_strlen(s);
    if (slen == 0) { char *e = (char *)lpp_alloc(1); e[0] = 0; return e; }
    if (n > 0 && slen > 0x7FFFFFFFFFFFFFFFLL / n) {
        lpp_exit(101);
    }
    int64_t total = slen * n;
    char *out = (char *)lpp_alloc(total + 1);
    for (int64_t i = 0; i < n; i++)
        for (int64_t j = 0; j < slen; j++)
            out[i * slen + j] = s[j];
    out[total] = 0;
    return out;
}

char *lpp_char_at(const char *s, int64_t idx) {
    if (!s) return (char *)lpp_alloc(1);
    int64_t slen = lpp_strlen(s);
    if (idx < 0 || idx >= slen) return (char *)lpp_alloc(1);
    char *out = (char *)lpp_alloc(2);
    out[0] = s[idx];
    out[1] = 0;
    return out;
}

int64_t lpp_ord(const char *s) {
    if (!s || !s[0]) return 0;
    return (int64_t)(unsigned char)s[0];
}

char *lpp_chr(int64_t code) {
    char *out = (char *)lpp_alloc(2);
    out[0] = (char)(code & 0xFF);
    out[1] = 0;
    return out;
}

int64_t lpp_str_find(const char *haystack, const char *needle) {
    if (!haystack || !needle) return -1;
    int64_t hlen = lpp_strlen(haystack);
    int64_t nlen = lpp_strlen(needle);
    if (nlen == 0) return 0;
    if (nlen > hlen) return -1;
    for (int64_t i = 0; i <= hlen - nlen; i++) {
        int64_t j = 0;
        while (j < nlen && haystack[i + j] == needle[j]) j++;
        if (j == nlen) return i;
    }
    return -1;
}

int64_t lpp_str_contains(const char *haystack, const char *needle) {
    return lpp_str_find(haystack, needle) >= 0 ? 1 : 0;
}

int64_t lpp_str_starts_with(const char *s, const char *prefix) {
    if (!s || !prefix) return 0;
    int64_t plen = lpp_strlen(prefix);
    for (int64_t i = 0; i < plen; i++) {
        if (s[i] != prefix[i] || s[i] == 0) return 0;
    }
    return 1;
}

int64_t lpp_str_ends_with(const char *s, const char *suffix) {
    if (!s || !suffix) return 0;
    int64_t slen = lpp_strlen(s);
    int64_t xlen = lpp_strlen(suffix);
    if (xlen > slen) return 0;
    for (int64_t i = 0; i < xlen; i++) {
        if (s[slen - xlen + i] != suffix[i]) return 0;
    }
    return 1;
}

char *lpp_str_upper(const char *s) {
    if (!s) return (char *)lpp_alloc(1);
    int64_t len = lpp_strlen(s);
    char *out = (char *)lpp_alloc(len + 1);
    for (int64_t i = 0; i < len; i++)
        out[i] = (s[i] >= 'a' && s[i] <= 'z') ? s[i] - 32 : s[i];
    out[len] = 0;
    return out;
}

char *lpp_str_lower(const char *s) {
    if (!s) return (char *)lpp_alloc(1);
    int64_t len = lpp_strlen(s);
    char *out = (char *)lpp_alloc(len + 1);
    for (int64_t i = 0; i < len; i++)
        out[i] = (s[i] >= 'A' && s[i] <= 'Z') ? s[i] + 32 : s[i];
    out[len] = 0;
    return out;
}

char *lpp_str_trim(const char *s) {
    if (!s) return (char *)lpp_alloc(1);
    int64_t len = lpp_strlen(s);
    int64_t start = 0, end = len;
    while (start < len && (s[start] == ' ' || s[start] == '\t' || s[start] == '\n' || s[start] == '\r')) start++;
    while (end > start && (s[end-1] == ' ' || s[end-1] == '\t' || s[end-1] == '\n' || s[end-1] == '\r')) end--;
    int64_t rlen = end - start;
    char *out = (char *)lpp_alloc(rlen + 1);
    for (int64_t i = 0; i < rlen; i++) out[i] = s[start + i];
    out[rlen] = 0;
    return out;
}

char *lpp_str_replace(const char *s, const char *old, const char *new_) {
    if (!s || !old || !new_) return (char *)lpp_alloc(1);
    int64_t slen = lpp_strlen(s);
    int64_t olen = lpp_strlen(old);
    int64_t nlen = lpp_strlen(new_);
    if (olen == 0) { /* copy */ char *out = (char *)lpp_alloc(slen + 1); for (int64_t i = 0; i <= slen; i++) out[i] = s[i]; return out; }
    /* count occurrences */
    int64_t count = 0;
    for (int64_t i = 0; i <= slen - olen; i++) {
        int64_t j = 0;
        while (j < olen && s[i+j] == old[j]) j++;
        if (j == olen) { count++; i += olen - 1; }
    }
    int64_t delta = nlen - olen;
    if (count > 0 && delta > 0 && slen > 0x7FFFFFFFFFFFFFFFLL - count * delta) {
        lpp_exit(101);
    }
    int64_t rlen = slen + count * delta;
    if (rlen < 0) rlen = 0;
    char *out = (char *)lpp_alloc(rlen + 1);
    int64_t w = 0;
    for (int64_t i = 0; i < slen; ) {
        int64_t j = 0;
        if (i <= slen - olen) { while (j < olen && s[i+j] == old[j]) j++; }
        if (j == olen) {
            for (int64_t k = 0; k < nlen; k++) out[w++] = new_[k];
            i += olen;
        } else {
            out[w++] = s[i++];
        }
    }
    out[rlen] = 0;
    return out;
}

char *lpp_int_to_str(int64_t val) {
    char buf[24];
    int neg = val < 0;
    if (neg) val = -val;
    int i = 23;
    buf[i] = 0;
    do { buf[--i] = '0' + (val % 10); val /= 10; } while (val);
    if (neg) buf[--i] = '-';
    int64_t len = 23 - i;
    char *out = (char *)lpp_alloc(len + 1);
    for (int64_t j = 0; j <= len; j++) out[j] = buf[i + j];
    return out;
}

/* float_to_str: the host runtime uses snprintf("%g"), which this freestanding
 * build has no libc for. Formatting is done by hand with the same six-decimal
 * fixed form lpp_print_float already uses, then trailing zeros are trimmed so
 * 2.5 prints as "2.5" rather than "2.500000".
 *
 * Without this symbol the internal lpp-link fails with
 * "unresolved GOT symbol 'lpp_float_to_str'" for any program that calls
 * float_to_str — the function existed only in runtime/lpp_str.c (host path). */
char *lpp_float_to_str(double val) {
    char buffer[64];
    int64_t w = 0;
    int negative = (val < 0.0);
    if (negative) val = -val;
    int64_t ipart = (int64_t)val;
    double fpart = val - (double)ipart;
    int64_t frac = (int64_t)(fpart * 1000000.0 + 0.5);
    if (frac >= 1000000) { frac -= 1000000; ipart += 1; }

    char tmp[32];
    int64_t t = 0;
    uint64_t magnitude = (uint64_t)ipart;
    do { tmp[t++] = (char)('0' + (magnitude % 10)); magnitude /= 10; } while (magnitude != 0);
    if (negative) buffer[w++] = '-';
    while (t > 0) buffer[w++] = tmp[--t];

    char fbuf[8];
    for (int i = 5; i >= 0; i--) { fbuf[i] = (char)('0' + (frac % 10)); frac /= 10; }
    int64_t flen = 6;
    while (flen > 0 && fbuf[flen - 1] == '0') flen--;
    if (flen > 0) {
        buffer[w++] = '.';
        for (int64_t i = 0; i < flen; i++) buffer[w++] = fbuf[i];
    }
    buffer[w] = 0;

    char *out = (char *)lpp_alloc(w + 1);
    for (int64_t i = 0; i <= w; i++) out[i] = buffer[i];
    return out;
}

int64_t lpp_str_to_int(const char *s) {
    if (!s) return 0;
    int64_t val = 0, neg = 0;
    int64_t i = 0;
    while (s[i] == ' ' || s[i] == '\t') i++;
    if (s[i] == '-') { neg = 1; i++; }
    else if (s[i] == '+') i++;
    while (s[i] >= '0' && s[i] <= '9') { val = val * 10 + (s[i] - '0'); i++; }
    return neg ? -val : val;
}
int64_t lpp_parse_int(const char *s) { return lpp_str_to_int(s); }

typedef struct LppEnvOverride {
    const char *name;
    const char *value;
    struct LppEnvOverride *next;
} LppEnvOverride;

static LppEnvOverride *lpp_env_overrides = 0;

int64_t lpp_env_set(const char *name, const char *value) {
    if (!name || !*name) return -1;
    LppEnvOverride *curr = lpp_env_overrides;
    while (curr) {
        if (lpp_str_eq(curr->name, name)) {
            if (curr->value) {
                lpp_free((void *)curr->value, 0);
            }
            if (value) {
                int len = 0;
                while (value[len]) len++;
                char *new_val = (char *)lpp_alloc(len + 1);
                for (int i = 0; i <= len; i++) new_val[i] = value[i];
                curr->value = new_val;
            } else {
                curr->value = 0;
            }
            return 0;
        }
        curr = curr->next;
    }
    LppEnvOverride *node = (LppEnvOverride *)lpp_alloc(sizeof(LppEnvOverride));
    int nlen = 0;
    while (name[nlen]) nlen++;
    char *new_name = (char *)lpp_alloc(nlen + 1);
    for (int i = 0; i <= nlen; i++) new_name[i] = name[i];
    node->name = new_name;
    if (value) {
        int vlen = 0;
        while (value[vlen]) vlen++;
        char *new_val = (char *)lpp_alloc(vlen + 1);
        for (int i = 0; i <= vlen; i++) new_val[i] = value[i];
        node->value = new_val;
    } else {
        node->value = 0;
    }
    node->next = lpp_env_overrides;
    lpp_env_overrides = node;
    return 0;
}

char *lpp_env_get(const char *name) {
    if (!name || !*name) {
        char *empty = (char *)lpp_alloc(1);
        empty[0] = 0;
        return empty;
    }
    LppEnvOverride *curr = lpp_env_overrides;
    while (curr) {
        if (lpp_str_eq(curr->name, name)) {
            if (!curr->value) {
                char *empty = (char *)lpp_alloc(1);
                empty[0] = 0;
                return empty;
            }
            int len = 0;
            while (curr->value[len]) len++;
            char *out = (char *)lpp_alloc(len + 1);
            for (int i = 0; i <= len; i++) out[i] = curr->value[i];
            return out;
        }
        curr = curr->next;
    }
    long fd = lpp_sys_open("/proc/self/environ", 0, 0);
    if (fd < 0) {
        char *empty = (char *)lpp_alloc(1);
        empty[0] = 0;
        return empty;
    }
    long cap = 32768;
    char *buf = (char *)lpp_arc_alloc(cap);
    if (!buf) {
        lpp_sys_close(fd);
        char *empty = (char *)lpp_alloc(1);
        empty[0] = 0;
        return empty;
    }
    long total = 0;
    while (total < cap) {
        long rd = lpp_sys_read(fd, buf + total, cap - total);
        if (rd <= 0) break;
        total += rd;
    }
    lpp_sys_close(fd);
    long name_len = 0;
    while (name[name_len]) name_len++;
    long i = 0;
    while (i < total) {
        int match = 1;
        for (long j = 0; j < name_len; j++) {
            if (i + j >= total || buf[i + j] != name[j]) {
                match = 0;
                break;
            }
        }
        if (match && i + name_len < total && buf[i + name_len] == '=') {
            long val_start = i + name_len + 1;
            long val_len = 0;
            while (val_start + val_len < total && buf[val_start + val_len] != '\0') {
                val_len++;
            }
            char *out = (char *)lpp_alloc(val_len + 1);
            for (long k = 0; k < val_len; k++) {
                out[k] = buf[val_start + k];
            }
            out[val_len] = 0;
            lpp_arc_release(buf);
            return out;
        }
        while (i < total && buf[i] != '\0') {
            i++;
        }
        i++;
    }
    lpp_arc_release(buf);
    char *empty = (char *)lpp_alloc(1);
    empty[0] = 0;
    return empty;
}

static long lpp_sys_fork(void) {
    long result;
    __asm__ volatile (
        "syscall"
        : "=a"(result)
        : "a"(57)
        : "rcx", "r11", "memory"
    );
    return result;
}

static long lpp_sys_execve(const char *pathname, char *const argv[], char *const envp[]) {
    long result;
    __asm__ volatile (
        "syscall"
        : "=a"(result)
        : "a"(59), "D"(pathname), "S"(argv), "d"(envp)
        : "rcx", "r11", "memory"
    );
    return result;
}

static long lpp_sys_wait4(long pid, int *wstatus, int options, void *rusage) {
    long result;
    __asm__ volatile (
        "syscall"
        : "=a"(result)
        : "a"(61), "D"(pid), "S"(wstatus), "d"((long)options), "r10"(rusage)
        : "rcx", "r11", "memory"
    );
    return result;
}

static long lpp_sys_pipe(int pipefd[2]) {
    long result;
    __asm__ volatile (
        "syscall"
        : "=a"(result)
        : "a"(22), "D"(pipefd)
        : "rcx", "r11", "memory"
    );
    return result;
}

static long lpp_sys_dup2(int oldfd, int newfd) {
    long result;
    __asm__ volatile (
        "syscall"
        : "=a"(result)
        : "a"(33), "D"(oldfd), "S"(newfd)
        : "rcx", "r11", "memory"
    );
    return result;
}

int64_t lpp_command_exec(const char *cmdline) {
    if (!cmdline) return -1;
    long pid = lpp_sys_fork();
    if (pid < 0) return -1;
    if (pid == 0) {
        const char *argv[] = { "/bin/sh", "-c", cmdline, 0 };
        lpp_sys_execve("/bin/sh", (char *const *)argv, 0);
        lpp_exit(127);
    }
    int status = 0;
    long ret = lpp_sys_wait4(pid, &status, 0, 0);
    if (ret < 0) return -1;
    if ((status & 0x7f) == 0) {
        return (int64_t)((status >> 8) & 0xff);
    }
    return -1;
}

char *lpp_command_output(const char *cmdline) {
    if (!cmdline) {
        char *empty = (char *)lpp_alloc(1);
        empty[0] = 0;
        return empty;
    }
    int pipefd[2];
    if (lpp_sys_pipe(pipefd) < 0) {
        char *empty = (char *)lpp_alloc(1);
        empty[0] = 0;
        return empty;
    }
    long pid = lpp_sys_fork();
    if (pid < 0) {
        lpp_sys_close(pipefd[0]);
        lpp_sys_close(pipefd[1]);
        char *empty = (char *)lpp_alloc(1);
        empty[0] = 0;
        return empty;
    }
    if (pid == 0) {
        lpp_sys_dup2(pipefd[1], 1);
        lpp_sys_dup2(pipefd[1], 2);
        lpp_sys_close(pipefd[0]);
        lpp_sys_close(pipefd[1]);
        const char *argv[] = { "/bin/sh", "-c", cmdline, 0 };
        lpp_sys_execve("/bin/sh", (char *const *)argv, 0);
        lpp_exit(127);
    }
    lpp_sys_close(pipefd[1]);
    long cap = 1024;
    long length = 0;
    char *buf = (char *)lpp_alloc(cap);
    while (1) {
        if (length + 256 >= cap) {
            long new_cap = cap * 2;
            char *new_buf = (char *)lpp_alloc(new_cap);
            for (long i = 0; i < length; i++) new_buf[i] = buf[i];
            lpp_arc_release(buf);
            buf = new_buf;
            cap = new_cap;
        }
        long rd = lpp_sys_read(pipefd[0], buf + length, 256);
        if (rd <= 0) break;
        length += rd;
    }
    buf[length] = 0;
    lpp_sys_close(pipefd[0]);
    int status = 0;
    lpp_sys_wait4(pid, &status, 0, 0);
    return buf;
}

const char *lpp_input(void) {
    char buf[4096];
    long rd = lpp_sys_read(0, buf, sizeof(buf) - 1);
    if (rd < 0) rd = 0;
    while (rd > 0 && (buf[rd - 1] == '\n' || buf[rd - 1] == '\r')) {
        rd--;
    }
    char *out = (char *)lpp_alloc(rd + 1);
    for (long i = 0; i < rd; i++) {
        out[i] = buf[i];
    }
    out[rd] = 0;
    return out;
}

/* ── Math builtins ── */
int64_t lpp_abs(int64_t x) { return x < 0 ? -x : x; }
int64_t lpp_min(int64_t a, int64_t b) { return a < b ? a : b; }
int64_t lpp_max(int64_t a, int64_t b) { return a > b ? a : b; }
int64_t lpp_int_pow(int64_t base, int64_t exp) { int64_t r=1; while(exp>0){if(exp&1)r*=base;base*=base;exp>>=1;} return r; }
double lpp_int_to_float(int64_t x) { return (double)x; }
int64_t lpp_float_to_int(double x) { return (int64_t)x; }
double lpp_sqrt(double x) { if(x<=0)return 0; double g=x; for(int i=0;i<50;i++)g=0.5*(g+x/g); return g; }
double lpp_floor(double x) { int64_t i=(int64_t)x; return (double)(x<(double)i?i-1:i); }
double lpp_ceil(double x) { int64_t i=(int64_t)x; return (double)(x>(double)i?i+1:i); }
double lpp_pow(double b,double e) { int64_t ie=(int64_t)e; if((double)ie==e&&ie>=0){double r=1;while(ie>0){if(ie&1)r*=b;b*=b;ie>>=1;}return r;} return 0; }

/* ── Random (stubs — no writable .bss in static freestanding) ── */
void lpp_random_seed(int64_t seed) { (void)seed; }
int64_t lpp_random(void) { return 42; }
int64_t lpp_random_range(int64_t lo, int64_t hi) { return lo < hi ? lo : 0; }

/* ── Time (syscalls) ── */
int64_t lpp_time_ms(void) { uint64_t buf[2]; __asm__ volatile("syscall":"=a"(buf[0]):"a"(228),"D"(1),"S"(buf):"rcx","r11","memory"); return (int64_t)(buf[0]*1000+buf[1]/1000000); }
void lpp_sleep_ms(int64_t ms) { uint64_t buf[2]; buf[0]=(uint64_t)(ms/1000); buf[1]=(uint64_t)((ms%1000)*1000000); __asm__ volatile("syscall"::"a"(35),"D"(buf),"S"(0):"rcx","r11","memory"); }
void lpp_exit(int64_t code) { __asm__ volatile("syscall"::"a"(60),"D"(code):"rcx","r11"); for(;;); }

/* ── String equality ── */
int64_t lpp_str_eq(const char *a, const char *b) { if(a==b)return 1; if(!a||!b)return 0; while(*a&&*a==*b){a++;b++;} return *a==*b?1:0; }

/* ── Buffers ── */
int64_t lpp_buf_alloc(int64_t size) { if(size<=0)size=64; int64_t t=size+8; void*m=lpp_sys_mmap(lpp_page_round((uint64_t)t)); if(!m)return 0; *(int64_t*)m=size; return(int64_t)(uintptr_t)((char*)m+8); }
void lpp_buf_free(void*p) { if(!p)return; char*b=(char*)p-8; int64_t s=*(int64_t*)b; lpp_sys_munmap(b,lpp_page_round((uint64_t)(s+8))); }
int64_t lpp_buf_len(void*p) { if(!p)return 0; return*(int64_t*)((char*)p-8); }
int64_t lpp_buf_get8(void*p,int64_t o) { if(!p)return 0; return(int64_t)(unsigned char)((char*)p)[o]; }
void lpp_buf_set8(void*p,int64_t o,int64_t v) { if(!p)return; ((char*)p)[o]=(char)(v&0xFF); }
void lpp_buf_set16le(void*p,int64_t o,int64_t v) { if(!p)return; char*d=(char*)p+o; d[0]=(char)(v&0xFF); d[1]=(char)((v>>8)&0xFF); }
int64_t lpp_buf_get16le(void*p,int64_t o) { if(!p)return 0; unsigned char*d=(unsigned char*)((char*)p+o); return(int64_t)d[0]|((int64_t)d[1]<<8); }
void lpp_buf_set32le(void*p,int64_t o,int64_t v) { if(!p)return; char*d=(char*)p+o; d[0]=(char)(v&0xFF); d[1]=(char)((v>>8)&0xFF); d[2]=(char)((v>>16)&0xFF); d[3]=(char)((v>>24)&0xFF); }
int64_t lpp_buf_get32le(void*p,int64_t o) { if(!p)return 0; unsigned char*d=(unsigned char*)((char*)p+o); return(int64_t)d[0]|((int64_t)d[1]<<8)|((int64_t)d[2]<<16)|((int64_t)d[3]<<24); }
void lpp_buf_copy(void*dst,int64_t do2,void*src,int64_t so,int64_t len) { if(!dst||!src||len<=0)return; char*d=(char*)dst+do2; char*s=(char*)src+so; for(int64_t i=0;i<len;i++)d[i]=s[i]; }
char*lpp_buf_read_str(void*p,int64_t o,int64_t len) { if(!p||len<=0){char*e=(char*)lpp_alloc(1);e[0]=0;return e;} char*out=(char*)lpp_alloc(len+1); char*s=(char*)p+o; for(int64_t i=0;i<len;i++)out[i]=s[i]; out[len]=0; return out; }

/* Mirror runtime/lpp_buf.c semantics: 0 on success, -1 on error. */
void *lpp_buf_read(const char*path) { if(!path)return 0; long fd=lpp_sys_open(path,0,0); if(fd<0)return 0; long size=lpp_sys_lseek(fd,0,2); (void)lpp_sys_lseek(fd,0,0); if(size<0){lpp_sys_close(fd);return 0;} void*buf=(void*)(uintptr_t)lpp_buf_alloc(size); if(!buf){lpp_sys_close(fd);return 0;} long off=0; while(off<size){ long r=lpp_sys_read(fd,(char*)buf+off,size-off); if(r<=0)break; off+=r; } lpp_sys_close(fd); if(off!=size){lpp_buf_free(buf);return 0;} return buf; }
int64_t lpp_buf_write(const char*path,void*p) { if(!path||!p)return -1; int64_t size=*(int64_t*)((char*)p-8); long fd=lpp_sys_open(path,0101,0644); if(fd<0)return -1; int64_t off=0; while(off<size){ long w=lpp_sys_write(fd,(char*)p+off,size-off); if(w<=0){lpp_sys_close(fd);return -1;} off+=w; } lpp_sys_close(fd); return 0; }
int64_t lpp_buf_write_str(void*p,int64_t o,const char*str) { if(!p||!str)return -1; int64_t n=0; while(str[n])n++; int64_t cap=lpp_buf_len(p); if(o<0||o+n>cap)return -1; char*d=(char*)p+o; for(int64_t i=0;i<n;i++)d[i]=str[i]; return n; }
int64_t lpp_buf_crc32(void*p,int64_t off,int64_t len) { if(!p||len<0)return 0; int64_t cap=lpp_buf_len(p); if(off<0||off+len>cap)return 0; const unsigned char*d=(const unsigned char*)((char*)p+off); uint32_t crc=0xFFFFFFFFu; for(int64_t i=0;i<len;i++){ crc^=d[i]; for(int b=0;b<8;b++){ uint32_t m=~(crc&1u)+1u; crc=(crc>>1)^(0xEDB88320u&m); } } return (int64_t)(uint32_t)(~crc); }

/* ── Additional net stubs ── */
int64_t lpp_net_accept_timeout(int64_t l,int64_t t){(void)t;return lpp_net_accept(l);}
int64_t lpp_net_dial(const char*h,int64_t p,int64_t t){(void)t;return lpp_net_connect(h,p);}
int64_t lpp_net_dial_udp(const char*h,int64_t p,int64_t t){(void)h;(void)p;(void)t;return 0;}
int64_t lpp_net_listen_udp(int64_t p){(void)p;return 0;}
int64_t lpp_net_set_deadline(int64_t f,int64_t r,int64_t w){return lpp_net_set_timeout(f,r>w?r:w);}
int64_t lpp_net_set_keepalive(int64_t f,int64_t e,int64_t i,int64_t v,int64_t c){(void)f;(void)e;(void)i;(void)v;(void)c;return 1;}

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
