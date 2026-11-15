/* ── ARC (Automatic Reference Counting) ──────────────────────────────────── */
/*
 * Native ARC payloads share a 16-byte header immediately before the payload:
 * magic@0, refcount@4, generation@8, flags@12. When a destructor is present,
 * its 8-byte function pointer lives immediately before the common header, so
 * destructor-bearing allocations still have a 24-byte total prefix. Plain
 * objects (including strings) use only the common 16-byte header.
 *
 * Atomic ops use C11 stdatomic on GCC/Clang and MSVC interlocked on Windows.
 */

#if defined(_MSC_VER)
#  define WIN32_LEAN_AND_MEAN
#  include <windows.h>
   typedef volatile LONG lpp_atomic32_t;
#  define LPP_ARC_LOAD(p)         ((int32_t)InterlockedAdd((p), 0))
#  define LPP_ARC_INC(p)          InterlockedIncrement((p))
#  define LPP_ARC_DEC(p)          InterlockedExchangeAdd((p), -1)
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
    /* Cycle-collector and layout flags; HAS_DTOR records the optional prefix. */
    lpp_atomic32_t flags;
} LppArcHeader;

#include <stddef.h>
#if defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L
_Static_assert(sizeof(LppArcHeader) == 16, "native common ARC header must be 16 bytes");
_Static_assert(offsetof(LppArcHeader, refcount) == 4, "refcount at 4");
_Static_assert(offsetof(LppArcHeader, generation) == 8, "generation at 8");
_Static_assert(offsetof(LppArcHeader, flags) == 12, "flags at 12");
_Static_assert(sizeof(LppArcDestructor) == 8, "native destructor prefix must be 8 bytes");
#endif

#define LPP_ARC_FLAG_HAS_DTOR 4u

static inline uint32_t lpp__header_flags(const LppArcHeader *hdr) {
    return (uint32_t)LPP_ARC_LOAD((lpp_atomic32_t *)&hdr->flags);
}

static inline LppArcDestructor lpp__header_destructor(const LppArcHeader *hdr) {
    if (!(lpp__header_flags(hdr) & LPP_ARC_FLAG_HAS_DTOR)) return NULL;
    return *(LppArcDestructor const *)((const char *)hdr - sizeof(LppArcDestructor));
}

static inline void *lpp__allocation_base(LppArcHeader *hdr) {
    if (lpp__header_flags(hdr) & LPP_ARC_FLAG_HAS_DTOR)
        return (char *)hdr - sizeof(LppArcDestructor);
    return hdr;
}

static inline void lpp__arc_teardown_info(LppArcHeader *hdr, void **allocation_out,
                                           LppArcDestructor *destructor_out) {
    uint32_t flags = lpp__header_flags(hdr);
    if (flags & LPP_ARC_FLAG_HAS_DTOR) {
        void *base = (char *)hdr - sizeof(LppArcDestructor);
        *allocation_out = base;
        *destructor_out = *(LppArcDestructor const *)base;
    } else {
        *allocation_out = hdr;
        *destructor_out = NULL;
    }
}

/* Allocate an ARC object with an optional type-specific destructor. */
void *lpp_arc_alloc_with_destructor(int64_t size, LppArcDestructor destructor) {
    if (size < 0) return NULL;
    size_t destructor_prefix = destructor ? sizeof(LppArcDestructor) : 0;
    size_t prefix = sizeof(LppArcHeader) + destructor_prefix;
    if ((uint64_t)size > SIZE_MAX - prefix) return NULL;
    void *allocation = calloc(1, prefix + (size_t)size);
    if (!allocation) return NULL;
    LppArcHeader *hdr = (LppArcHeader *)((char *)allocation + destructor_prefix);
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
    hdr->flags = destructor ? (int32_t)LPP_ARC_FLAG_HAS_DTOR : 0;
#else
    atomic_init(&hdr->refcount, 1);
    atomic_init(&hdr->generation, fresh);
    atomic_init(&hdr->flags, destructor ? (int32_t)LPP_ARC_FLAG_HAS_DTOR : 0);
#endif
    if (destructor) *(LppArcDestructor *)allocation = destructor;
    return (void *)(hdr + 1); /* payload follows the common 16-byte header */
}

/* Backwards-compatible allocation for runtime values with no child owners. */
void *lpp_arc_alloc(int64_t size) {
    return lpp_arc_alloc_with_destructor(size, NULL);
}

/* ── The immortal empty string ────────────────────────────────────────────
 *
 * Runtime error paths need a real ARC payload rather than a bare C literal,
 * because generated code may retain or release the returned `Str`. The blob
 * uses the shared 16-byte native header and returns its payload at word 4.
 * It is simultaneously valid for the hosted and freestanding x86-64 runtimes.
 */
#if defined(_MSC_VER)
__declspec(align(16))
static const uint32_t lpp__empty_str_blob[8] = {
#else
static const uint32_t lpp__empty_str_blob[8] __attribute__((aligned(16))) = {
#endif
    LPP_ARC_MAGIC, LPP_ARC_IMMORTAL, 0, 0, 0, 0, 0, 0
};

char *lpp_empty_str(void) {
    return (char *)(const char *)&lpp__empty_str_blob[4];
}

/* ── Immortal objects ─────────────────────────────────────────────────────
 *
 * String literals live in .rodata and must never be freed, but generated code
 * sees them as plain `char *`, indistinguishable from a heap string. The
 * compiler emits a real 16-byte ARC header in front of each literal, with
 * both magic@0 and refcount@4 set to this sentinel. Retain/release detect the
 * value and return without writing, which matters because the page is read-only.
 * The hosted and freestanding x86-64 runtimes use this same header layout. */
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
 * A non-owning (weak) field is stored WITHOUT a retain, so
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
 * Arena nodes use the same 16-byte common header as ordinary native objects;
 * destructor-bearing nodes additionally carry the optional 8-byte prefix. A
 * registry associates each node header with its region, so existing C runtime
 * helpers can retain/release an arena node without knowing its source-level type. Nodes
 * are never individually freed: their destructors run at refcount zero, and
 * the region releases all node storage in one bulk pass after its last node is
 * gone. (Arenas remain a legacy allocation strategy; reference cycles
 * in ordinary ARC allocations are reclaimed by the trial-deletion
 * collector below, and no edge is demoted. See
 * docs/CYCLE_BREAKING.md.)
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
        free(lpp__allocation_base(records->header));
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
    atomic_fetch_add_explicit(&region->refs, 1, memory_order_release);
#endif
    lpp__arena_lock_release();
    return payload;
}

