/* ── Floating-point math primitives ──────────────────────────────────────── */
double fmod(double x, double y) {
    if (y == 0.0) return 0.0;
    int64_t i = (int64_t)(x / y);
    return x - (double)i * y;
}

/* ── Freestanding ARC foundation ─────────────────────────────────────────── */
/* Every direct-link ARC allocation owns a whole mmap region. */

typedef void (*LppArcDestructor)(void *payload);
#define LPP_ARC_IMMORTAL 0x41524331U
#define LPP_SIZE_CLASSES 8
#define LPP_MAP_FLAG_COLLECTIBLE 0x80000000u
#define LPP_MAP_FLAG_BUFFERED 0x40000000u
#define LPP_MAP_FLAG_MASK 0xC0000000u
#define LPP_MAP_LARGE_DTOR_FLAG 0x00000800u

typedef struct {
    uint32_t magic;
    int refcount;
    int generation;
    uint32_t map_size;
} LppArcHeader;
_Static_assert(sizeof(LppArcHeader) == 16, "native common ARC header must be 16 bytes");

static inline uint32_t lpp__map_tag(const LppArcHeader *header) {
    return header->map_size & ~LPP_MAP_FLAG_MASK;
}

static inline int lpp__tag_has_destructor(uint32_t tag) {
    if (tag >= LPP_SIZE_CLASSES + 1 && tag <= 2 * LPP_SIZE_CLASSES) return 1;
    return tag > 2 * LPP_SIZE_CLASSES && (tag & LPP_MAP_LARGE_DTOR_FLAG) != 0;
}

static inline int lpp__header_has_destructor(const LppArcHeader *header) {
    return lpp__tag_has_destructor(lpp__map_tag(header));
}

static inline LppArcDestructor lpp__header_destructor(const LppArcHeader *header) {
    if (!lpp__header_has_destructor(header)) return 0;
    return *(LppArcDestructor const *)((const char *)header - sizeof(LppArcDestructor));
}

static inline void *lpp__allocation_base(LppArcHeader *header) {
    return lpp__header_has_destructor(header)
        ? (void *)((char *)header - sizeof(LppArcDestructor))
        : (void *)header;
}

static inline void lpp__teardown_info(LppArcHeader *header, void **base_out,
                                      LppArcDestructor *destructor_out) {
    uint32_t tag = lpp__map_tag(header);
    if (lpp__tag_has_destructor(tag)) {
        void *base = (char *)header - sizeof(LppArcDestructor);
        *base_out = base;
        *destructor_out = *(LppArcDestructor const *)base;
    } else {
        *base_out = header;
        *destructor_out = 0;
    }
}

/* Immortal literals use the same 16-byte common header in host and
 * freestanding builds: magic@0, refcount@4, generation@8, flags@12. */
static inline int lpp__is_immortal(const LppArcHeader *header) {
    return (uint32_t)header->magic == LPP_ARC_IMMORTAL &&
           (uint32_t)header->refcount == LPP_ARC_IMMORTAL;
}

/* Cycle-collector hooks (defined with the collector below) and the
 * shared teardown counters every runtime exposes. */
uint64_t lpp__runtime_drop_count = 0;
uint64_t lpp__cycle_collected_count = 0;
static void lpp__note_possible_root(void *payload);
static void lpp__cycle_deregister(void *payload);
static inline int lpp__cycle_is_collectible(const LppArcHeader *header) {
    return (header->map_size & 0x80000000u) != 0;
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
        : "a"(9), "D"((long)0), "S"((long)size), "d"((long)7),
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
 * The common ARC header is 16 bytes. A non-null destructor is stored in an
 * optional 8-byte prefix before it. The map_size word encodes the allocator
 * size class plus that prefix bit (or a page-rounded mapping size for large
 * objects), while the top two bits remain cycle-collector flags.
 */

/* Monotonic source of object generations; see the host runtime for why this
 * must not be derived per-object. */
static int lpp__generation_counter = 1;

#define LPP_CHUNK_BYTES   (1024 * 1024)

/* Class i holds blocks of 32 << i bytes: 32 .. 4096. */
static void *lpp_free_lists[LPP_SIZE_CLASSES] = {0, 0, 0, 0, 0, 0, 0, 0};
static char *lpp_bump_cursor = (char *)0;
static uint64_t lpp_bump_left = 0;

static uint64_t lpp_class_bytes(int cls) { return (uint64_t)32 << cls; }

static int lpp_class_for(uint64_t need) {
    int cls = 0;
    while (cls < LPP_SIZE_CLASSES && lpp_class_bytes(cls) < need) cls++;
    return cls;
}

void *lpp_arc_alloc_with_destructor(int64_t payload_size, LppArcDestructor destructor) {
    if (payload_size < 0) return 0;
    uint64_t destructor_prefix = destructor ? sizeof(LppArcDestructor) : 0;
    if ((uint64_t)payload_size > UINT64_MAX - sizeof(LppArcHeader) - destructor_prefix) return 0;
    uint64_t need = (uint64_t)payload_size + sizeof(LppArcHeader) + destructor_prefix;
    int cls = lpp_class_for(need);
    void *block;
    uint32_t size_tag;

    if (cls >= LPP_SIZE_CLASSES) {
        uint64_t total = lpp_page_round(need);
        /* The top two bits are cycle flags; the low page-offset bit marks
         * the optional destructor prefix. Keep large allocations below 1 GiB. */
        if (total > 0x3ffff000ULL) return 0;
        block = lpp_sys_mmap(total);
        if (!block) return 0;
        size_tag = (uint32_t)total | (destructor ? LPP_MAP_LARGE_DTOR_FLAG : 0);
    } else {
        uint64_t bytes = lpp_class_bytes(cls);
        if (lpp_free_lists[cls]) {
            block = lpp_free_lists[cls];
            /* The first word of a free block is the next-free link. */
            lpp_free_lists[cls] = *(void **)block;
        } else {
            if (lpp_bump_left < bytes) {
                char *chunk = (char *)lpp_sys_mmap(LPP_CHUNK_BYTES);
                if (!chunk) return 0;
                lpp_bump_cursor = chunk;
                lpp_bump_left = LPP_CHUNK_BYTES;
            }
            block = lpp_bump_cursor;
            lpp_bump_cursor += bytes;
            lpp_bump_left -= bytes;
        }
        /* Recycled blocks must look freshly mapped: callers rely on
         * zero-initialised payload fields. */
        char *raw = (char *)block;
        for (uint64_t i = 0; i < bytes; i++) raw[i] = 0;
        size_tag = (uint32_t)(cls + 1) + (destructor ? LPP_SIZE_CLASSES : 0);
    }

    LppArcHeader *header = (LppArcHeader *)((char *)block + destructor_prefix);
    header->map_size = size_tag;
    header->magic = LPP_ARC_IMMORTAL;
    header->refcount = 1;
    /* Generations come from a monotonic global, never restarting, so a stale
     * weak handle can never match a new occupant of a reused address. */
    header->generation = __atomic_add_fetch(&lpp__generation_counter, 1, __ATOMIC_RELAXED);
    if (destructor) *(LppArcDestructor *)block = destructor;
    return (void *)(header + 1);
}

void *lpp_arc_alloc(int64_t payload_size) {
    return lpp_arc_alloc_with_destructor(payload_size, 0);
}

static const uint32_t lpp__empty_str_blob[8] __attribute__((aligned(16))) = {
    LPP_ARC_IMMORTAL, LPP_ARC_IMMORTAL, 0, 0, 0, 0, 0, 0
};

char *lpp_empty_str(void) {
    return (char *)&lpp__empty_str_blob[4];
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
        lpp__runtime_drop_count++;
        if (lpp__cycle_is_collectible(header)) lpp__cycle_deregister(payload);
        LppArcDestructor destructor = lpp__header_destructor(header);
        if (destructor) destructor(payload);
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

/* Return a dead object to its size-class free list, or unmap its full block.
 * Class tags 1..8 mean no destructor; 9..16 mean the same class with the
 * optional 8-byte destructor prefix. Large mmap sizes are page-rounded, with
 * bit 11 encoding the prefix and the top two bits reserved for cycle flags. */
static void lpp_arc_free_base(LppArcHeader *header, void *block) {
    uint64_t tag = lpp__map_tag(header);
    if (tag >= 1 && tag <= 2 * LPP_SIZE_CLASSES) {
        int cls = (int)((tag - 1) % LPP_SIZE_CLASSES);
        *(void **)block = lpp_free_lists[cls];
        lpp_free_lists[cls] = block;
        return;
    }
    uint64_t bytes = tag & ~0xFFFULL;
    lpp_sys_munmap(block, bytes);
}

static void lpp_arc_free(LppArcHeader *header) {
    if (header) lpp_arc_free_base(header, lpp__allocation_base(header));
}

