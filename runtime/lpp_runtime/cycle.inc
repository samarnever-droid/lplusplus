/* ── Cycle collection for collectible (Shared) types ──────────────────────
 *
 * Plain reference counting cannot reclaim reference cycles. The 4D
 * ownership plan knows statically which types can sit on a cycle (the
 * `Shared` set); generated code marks objects of those types with
 * `lpp_arc_mark_collectible` and supplies a trace function that reports
 * owned child pointers through `lpp_trace_visit`. Everything else keeps
 * prompt plain-ARC teardown and never touches this machinery.
 *
 * The algorithm is trial deletion (Bacon & Rajan; CPython's gc):
 *
 *   1. A release that leaves a collectible object alive (refcount > 0)
 *      records it as a *possible root* — it may be the entry point of a
 *      cycle that just became unreachable. At 512 buffered roots (or an
 *      explicit `lpp_arc_collect_cycles` call, which the generated
 *      `main` wrapper makes before returning) a collection round runs.
 *   2. Each reachable collectible node gets a scratch count = its
 *      refcount minus one per incoming edge from another node in the
 *      scanned subgraph. A scratch count > 0 proves a reference from
 *      outside the subgraph, so the node (and everything it reaches)
 *      is live.
 *   3. The rest are unreachable cycles. They are invalidated (magic
 *      cleared, generation bumped, trace deregistered) BEFORE any
 *      destructor runs, so destructor releases back into the garbage
 *      set are no-ops: no double free, no destructor runs twice, and
 *      acyclic children hanging off a dead cycle are released normally
 *      by the destructors.
 *
 * Non-collectible objects have no trace; if one shows up as a child it
 * is treated as live (never freed on suspicion). Allocation failure
 * inside a round aborts the round having freed nothing — the failure
 * mode is a leak, never a crash.
 *
 * Threading: the registry/root-buffer are guarded by a spinlock (same
 * pattern as the arena lock); destructors run outside the lock so the
 * releases they perform can re-enter `lpp__note_possible_root`. As in
 * the Rust runtime, collection assumes the collectible graph is not
 * being mutated by another thread mid-round; acyclic ARC keeps its
 * full atomic guarantees either way. */

#define LPP_CYCLE_COLLECTIBLE 1u
#define LPP_CYCLE_BUFFERED 2u
#define LPP_CYCLE_ROOT_THRESHOLD 512

typedef void (*LppTraceFn)(void *payload, void *ctx);

/* Diagnostic counters (plain loads/stores; racy by design under
 * threads, exact in single-threaded programs). */
uint64_t lpp__runtime_drop_count = 0;
uint64_t lpp__cycle_collected_count = 0;

static inline uint32_t lpp__cycle_flags_load(const LppArcHeader *hdr) {
    return lpp__header_flags(hdr);
}

static inline void lpp__cycle_flags_or(LppArcHeader *hdr, uint32_t bits) {
#if defined(_MSC_VER)
    InterlockedOr((volatile LONG *)&hdr->flags, (LONG)bits);
#else
    atomic_fetch_or_explicit(&hdr->flags, (int32_t)bits, memory_order_relaxed);
#endif
}

static inline void lpp__cycle_flags_and(LppArcHeader *hdr, uint32_t bits) {
#if defined(_MSC_VER)
    InterlockedAnd((volatile LONG *)&hdr->flags, (LONG)bits);
#else
    atomic_fetch_and_explicit(&hdr->flags, (int32_t)bits, memory_order_relaxed);
#endif
}

#if defined(_MSC_VER)
static volatile LONG lpp__cycle_lock;
static void lpp__cycle_lock_acquire(void) {
    while (InterlockedCompareExchange(&lpp__cycle_lock, 1, 0) != 0) {}
}
static void lpp__cycle_lock_release(void) {
    InterlockedExchange(&lpp__cycle_lock, 0);
}
#else
static atomic_flag lpp__cycle_lock = ATOMIC_FLAG_INIT;
static void lpp__cycle_lock_acquire(void) {
    while (atomic_flag_test_and_set_explicit(&lpp__cycle_lock, memory_order_acquire)) {}
}
static void lpp__cycle_lock_release(void) {
    atomic_flag_clear_explicit(&lpp__cycle_lock, memory_order_release);
}
#endif

/* Open-addressing pointer → int64 map: the trace registry (value is a
 * trace function pointer) and each round's node index (value is a
 * node-slot index + 1). Key 0 = empty, 1 = tombstone. */
typedef struct {
    uintptr_t *keys;
    int64_t *vals;
    size_t cap;
    size_t len;
    size_t tombs;
} LppPtrMap;

static int lpp__map_grow(LppPtrMap *map, size_t new_cap) {
    uintptr_t *keys = (uintptr_t *)calloc(new_cap, sizeof(uintptr_t));
    int64_t *vals = (int64_t *)malloc(new_cap * sizeof(int64_t));
    if (!keys || !vals) {
        free(keys);
        free(vals);
        return 0;
    }
    for (size_t i = 0; i < map->cap; i++) {
        uintptr_t key = map->keys[i];
        if (key > 1) {
            size_t at = (size_t)((key >> 4) * UINT64_C(0x9E3779B97F4A7C15)) & (new_cap - 1);
            while (keys[at]) at = (at + 1) & (new_cap - 1);
            keys[at] = key;
            vals[at] = map->vals[i];
        }
    }
    free(map->keys);
    free(map->vals);
    map->keys = keys;
    map->vals = vals;
    map->cap = new_cap;
    map->tombs = 0;
    return 1;
}

/* Slot address for `key`, or NULL (absent / allocation failure). */
static int64_t *lpp__map_slot(LppPtrMap *map, uintptr_t key, int create) {
    if (!create && !map->cap) return NULL;
    if (create && (map->len + map->tombs + 1) * 10 >= map->cap * 7) {
        size_t new_cap = map->cap ? map->cap * 2 : 64;
        if (!lpp__map_grow(map, new_cap)) return NULL;
    }
    size_t mask = map->cap - 1;
    size_t at = (size_t)((key >> 4) * UINT64_C(0x9E3779B97F4A7C15)) & mask;
    size_t first_tomb = (size_t)-1;
    for (;;) {
        uintptr_t found = map->keys[at];
        if (found == key) return &map->vals[at];
        if (found == 1 && first_tomb == (size_t)-1) first_tomb = at;
        if (found == 0) {
            if (!create) return NULL;
            size_t use = first_tomb != (size_t)-1 ? first_tomb : at;
            if (first_tomb != (size_t)-1) map->tombs--;
            map->keys[use] = key;
            map->vals[use] = 0;
            map->len++;
            return &map->vals[use];
        }
        at = (at + 1) & mask;
    }
}

static void lpp__map_remove(LppPtrMap *map, uintptr_t key) {
    int64_t *slot = lpp__map_slot(map, key, 0);
    if (!slot) return;
    map->keys[(size_t)(slot - map->vals)] = 1;
    *slot = 0;
    map->len--;
    map->tombs++;
}

static void lpp__map_free(LppPtrMap *map) {
    free(map->keys);
    free(map->vals);
    map->keys = NULL;
    map->vals = NULL;
    map->cap = map->len = map->tombs = 0;
}

static LppPtrMap lpp__trace_registry;

typedef struct {
    void *ptr;
    int32_t generation;
} LppCycleCandidate;

static LppCycleCandidate *lpp__root_buffer;
static size_t lpp__root_len;
static size_t lpp__root_cap;
static int lpp__collecting;

/* The trace callback sink: trace functions report children through
 * `lpp_trace_visit(ctx, child)` where ctx is one of these. */
typedef struct {
    void **items;
    size_t len;
    size_t cap;
    int oom;
} LppTraceSink;

void lpp_trace_visit(void *ctx, void *child) {
    LppTraceSink *sink = (LppTraceSink *)ctx;
    if (!sink || !child || sink->oom) return;
    if (sink->len == sink->cap) {
        size_t new_cap = sink->cap ? sink->cap * 2 : 16;
        void **grown = (void **)realloc(sink->items, new_cap * sizeof(void *));
        if (!grown) {
            sink->oom = 1;
            return;
        }
        sink->items = grown;
        sink->cap = new_cap;
    }
    sink->items[sink->len++] = child;
}

/* Caller holds the cycle lock. */
static void lpp__cycle_deregister_locked(void *payload) {
    lpp__map_remove(&lpp__trace_registry, (uintptr_t)payload);
}

static void lpp__cycle_deregister(void *payload) {
    lpp__cycle_lock_acquire();
    lpp__cycle_deregister_locked(payload);
    lpp__cycle_lock_release();
}

void lpp_arc_collect_cycles(void);

static void lpp__note_possible_root(void *payload) {
    LppArcHeader *header = (LppArcHeader *)payload - 1;
    uint32_t flags = lpp__cycle_flags_load(header);
    if (!(flags & LPP_CYCLE_COLLECTIBLE) || (flags & LPP_CYCLE_BUFFERED)) return;
    int trigger = 0;
    lpp__cycle_lock_acquire();
    /* Re-check under the lock: another thread may have buffered it. */
    flags = lpp__cycle_flags_load(header);
    if ((flags & LPP_CYCLE_COLLECTIBLE) && !(flags & LPP_CYCLE_BUFFERED)) {
        if (lpp__root_len == lpp__root_cap) {
            size_t new_cap = lpp__root_cap ? lpp__root_cap * 2 : 64;
            LppCycleCandidate *grown = (LppCycleCandidate *)realloc(
                lpp__root_buffer, new_cap * sizeof(LppCycleCandidate));
            if (!grown) {
                lpp__cycle_lock_release();
                return; /* leak the hint, not the program */
            }
            lpp__root_buffer = grown;
            lpp__root_cap = new_cap;
        }
        lpp__root_buffer[lpp__root_len].ptr = payload;
        lpp__root_buffer[lpp__root_len].generation = LPP_ARC_LOAD(&header->generation);
        lpp__root_len++;
        lpp__cycle_flags_or(header, LPP_CYCLE_BUFFERED);
        trigger = lpp__root_len >= LPP_CYCLE_ROOT_THRESHOLD && !lpp__collecting;
    }
    lpp__cycle_lock_release();
    if (trigger) lpp_arc_collect_cycles();
}

void lpp_arc_mark_collectible(void *payload, LppTraceFn trace) {
    if (!trace || !lpp__is_valid_arc_ptr(payload)) return;
    LppArcHeader *header = (LppArcHeader *)payload - 1;
    if (lpp__is_immortal(header)) return;
    lpp__cycle_lock_acquire();
    int64_t *slot = lpp__map_slot(&lpp__trace_registry, (uintptr_t)payload, 1);
    if (slot) *slot = (int64_t)(intptr_t)(void *)trace;
    lpp__cycle_flags_or(header, LPP_CYCLE_COLLECTIBLE);
    lpp__cycle_lock_release();
}

static int lpp__candidate_valid(void *ptr, int32_t generation) {
    if (!lpp__is_valid_arc_ptr(ptr)) return 0;
    const LppArcHeader *header = (const LppArcHeader *)ptr - 1;
    if (lpp__is_immortal(header)) return 0;
    return LPP_ARC_LOAD(&header->generation) == generation
        && (lpp__cycle_flags_load(header) & LPP_CYCLE_COLLECTIBLE);
}

/* Caller holds the cycle lock. Returns 0 when no trace is registered
 * or the trace ran out of memory. */
static int lpp__trace_into(void *ptr, LppTraceSink *sink) {
    int64_t *slot = lpp__map_slot(&lpp__trace_registry, (uintptr_t)ptr, 0);
    if (!slot || !*slot) return 0;
    LppTraceFn trace = (LppTraceFn)(void *)(intptr_t)*slot;
    sink->len = 0;
    sink->oom = 0;
    trace(ptr, sink);
    return !sink->oom;
}

#define LPP__PUSH_NODE(ptr_)                                              \
    do {                                                                  \
        if (count == cap) {                                               \
            size_t nc_ = cap ? cap * 2 : 64;                              \
            void **nn_ = (void **)realloc(nodes, nc_ * sizeof(void *));   \
            int64_t *ni_ = (int64_t *)realloc(internal, nc_ * sizeof(int64_t)); \
            uint8_t *nl_ = (uint8_t *)realloc(live, nc_ * sizeof(uint8_t)); \
            if (nn_) nodes = nn_;                                         \
            if (ni_) internal = ni_;                                      \
            if (nl_) live = nl_;                                          \
            if (!nn_ || !ni_ || !nl_) goto done;                          \
            cap = nc_;                                                    \
        }                                                                 \
        nodes[count] = (ptr_);                                            \
        live[count] = 0;                                                  \
        count++;                                                          \
    } while (0)

/* One trial-deletion round over the candidates drained from the root
 * buffer. Called with the cycle lock held and `lpp__collecting` set;
 * releases the lock around destructor execution and re-acquires it
 * before returning. Returns the number of objects freed. */
static uint64_t lpp__collect_round(LppCycleCandidate *candidates, size_t ncands) {
    uint64_t freed = 0;
    void **nodes = NULL;
    int64_t *internal = NULL; /* scratch counts */
    uint8_t *live = NULL;
    size_t count = 0, cap = 0;
    LppPtrMap index = {0};
    LppTraceSink sink = {0};
    size_t *stack = NULL;
    size_t stack_len = 0, stack_cap = 0;
    void **garbage = NULL;
    size_t garbage_count = 0;

    /* Seed: every still-valid candidate becomes a node. */
    for (size_t r = 0; r < ncands; r++) {
        void *ptr = candidates[r].ptr;
        if (!lpp__candidate_valid(ptr, candidates[r].generation)) continue;
        LppArcHeader *header = (LppArcHeader *)ptr - 1;
        lpp__cycle_flags_and(header, ~LPP_CYCLE_BUFFERED);
        if (!lpp__map_slot(&lpp__trace_registry, (uintptr_t)ptr, 0)) continue;
        if (lpp__map_slot(&index, (uintptr_t)ptr, 0)) continue;
        int64_t *slot = lpp__map_slot(&index, (uintptr_t)ptr, 1);
        if (!slot) goto done;
        *slot = (int64_t)count + 1;
        LPP__PUSH_NODE(ptr);
        internal[count - 1] = (int64_t)LPP_ARC_LOAD(&header->refcount);
    }

    /* Mark gray: expand the node set over collectible edges, then
     * subtract one scratch count per incoming internal edge (multi-
     * edges subtract multiply, matching the retains that created
     * them). A node whose trace is unavailable is treated as
     * externally referenced — freed on suspicion is not an option. */
    for (size_t i = 0; i < count; i++) {
        if (!lpp__trace_into(nodes[i], &sink)) {
            if (internal[i] < 1) internal[i] = 1;
            continue;
        }
        for (size_t j = 0; j < sink.len; j++) {
            void *child = sink.items[j];
            if (!lpp__is_valid_arc_ptr(child)) continue;
            LppArcHeader *cheader = (LppArcHeader *)child - 1;
            if (lpp__is_immortal(cheader)) continue;
            if (!(lpp__cycle_flags_load(cheader) & LPP_CYCLE_COLLECTIBLE)) continue;
            if (!lpp__map_slot(&lpp__trace_registry, (uintptr_t)child, 0)) continue;
            int64_t *existing = lpp__map_slot(&index, (uintptr_t)child, 0);
            if (existing) {
                internal[(size_t)*existing - 1]--;
            } else {
                int64_t *slot = lpp__map_slot(&index, (uintptr_t)child, 1);
                if (!slot) goto done;
                *slot = (int64_t)count + 1;
                LPP__PUSH_NODE(child);
                internal[count - 1] = (int64_t)LPP_ARC_LOAD(&cheader->refcount) - 1;
            }
        }
    }

    /* Scan: nodes keeping a positive scratch count have a reference
     * from outside the subgraph; liveness flows along their edges. */
    for (size_t i = 0; i < count; i++) {
        if (internal[i] > 0 && !live[i]) {
            live[i] = 1;
            if (stack_len == stack_cap) {
                size_t nc = stack_cap ? stack_cap * 2 : 64;
                size_t *grown = (size_t *)realloc(stack, nc * sizeof(size_t));
                if (!grown) goto done;
                stack = grown;
                stack_cap = nc;
            }
            stack[stack_len++] = i;
        }
    }
    while (stack_len) {
        size_t slot_i = stack[--stack_len];
        /* A failed trace here would under-mark liveness: abort the
         * round (nothing freed) rather than risk a live node. */
        if (!lpp__trace_into(nodes[slot_i], &sink)) goto done;
        for (size_t j = 0; j < sink.len; j++) {
            int64_t *existing = lpp__map_slot(&index, (uintptr_t)sink.items[j], 0);
            if (!existing) continue;
            size_t child_i = (size_t)*existing - 1;
            if (live[child_i]) continue;
            live[child_i] = 1;
            if (stack_len == stack_cap) {
                size_t nc = stack_cap ? stack_cap * 2 : 64;
                size_t *grown = (size_t *)realloc(stack, nc * sizeof(size_t));
                if (!grown) goto done;
                stack = grown;
                stack_cap = nc;
            }
            stack[stack_len++] = child_i;
        }
    }

    /* Collect: invalidate the whole garbage set first so destructor
     * releases back into it are no-ops, then run destructors with the
     * lock released (their releases re-enter the root buffer). */
    garbage = (void **)malloc((count ? count : 1) * sizeof(void *));
    if (!garbage) goto done;
    for (size_t i = 0; i < count; i++) {
        if (live[i]) continue;
        void *ptr = nodes[i];
        LppArcHeader *header = (LppArcHeader *)ptr - 1;
        lpp__invalidate_generation(header);
        header->magic = 0;
        lpp__cycle_deregister_locked(ptr);
        garbage[garbage_count++] = ptr;
    }
    lpp__cycle_lock_release();
    /* Destructors first, storage second: a member destructor that
     * releases another member must land on that member's intact
     * (invalidated) header, never on recycled memory. */
    for (size_t i = 0; i < garbage_count; i++) {
        void *ptr = garbage[i];
        LppArcHeader *header = (LppArcHeader *)ptr - 1;
        LppArcDestructor destructor = lpp__header_destructor(header);
        if (destructor) destructor(ptr);
        lpp__runtime_drop_count++;
        lpp__cycle_collected_count++;
        freed++;
    }
    for (size_t i = 0; i < garbage_count; i++) {
        void *ptr = garbage[i];
        LppArcHeader *header = (LppArcHeader *)ptr - 1;
        LppArenaRegion *arena = lpp__arena_for_header(header);
        if (arena) lpp__arena_node_zero(arena);
        else free(lpp__allocation_base(header));
    }
    lpp__cycle_lock_acquire();

done:
    free(nodes);
    free(internal);
    free(live);
    free(stack);
    free(garbage);
    free(sink.items);
    lpp__map_free(&index);
    return freed;
}

#undef LPP__PUSH_NODE

void lpp_arc_collect_cycles(void) {
    for (;;) {
        lpp__cycle_lock_acquire();
        if (lpp__collecting) {
            lpp__cycle_lock_release();
            return;
        }
        if (lpp__root_len == 0) {
            lpp__cycle_lock_release();
            return;
        }
        lpp__collecting = 1;
        LppCycleCandidate *candidates = lpp__root_buffer;
        size_t ncands = lpp__root_len;
        lpp__root_buffer = NULL;
        lpp__root_len = 0;
        lpp__root_cap = 0;
        uint64_t freed = lpp__collect_round(candidates, ncands);
        free(candidates);
        int again = (freed > 0 && lpp__root_len > 0)
            || lpp__root_len >= LPP_CYCLE_ROOT_THRESHOLD;
        lpp__collecting = 0;
        lpp__cycle_lock_release();
        if (!again) break;
    }
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
        lpp__runtime_drop_count++;
        if (lpp__cycle_flags_load(header) & LPP_CYCLE_COLLECTIBLE) {
            lpp__cycle_deregister(payload);
        }
        LppArcDestructor destructor = lpp__header_destructor(header);
        lpp__invalidate_generation(header);
        header->magic = 0;
        if (destructor) destructor(payload);
        lpp__arena_node_zero(region);
    } else if (prev > 1) {
        lpp__note_possible_root(payload);
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
        lpp__runtime_drop_count++;
        if (lpp__cycle_flags_load(hdr) & LPP_CYCLE_COLLECTIBLE) {
            lpp__cycle_deregister(ptr);
        }
        void *allocation;
        LppArcDestructor destructor;
        lpp__arc_teardown_info(hdr, &allocation, &destructor);
        lpp__invalidate_generation(hdr);
        hdr->magic = 0;
        if (destructor) destructor(ptr);
        if (arena) lpp__arena_node_zero(arena);
        else free(allocation);
    } else if (prev > 1) {
        /* A collectible object that survives a release may head a
         * cycle that just became unreachable. */
        lpp__note_possible_root(ptr);
    }
}

