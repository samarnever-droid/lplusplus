/* ── Cycle collector: trial deletion over collectible objects ─────────────
 *
 * The C mirror of the Rust runtime's `collect` module. Generated code
 * marks allocations whose type the ownership plan classified Shared
 * via lpp_arc_mark_collectible (with a trace function enumerating the
 * payload's owned ARC children). A release that leaves a collectible
 * object alive buffers it as a possible cycle root; collection
 * subtracts one scratch count per edge inside the candidate subgraph
 * and frees exactly the nodes no external reference can explain.
 */

typedef void (*LppTraceFn)(void *payload, void *ctx);

#define LPP_CYCLE_COLLECTIBLE 1u
#define LPP_CYCLE_BUFFERED 2u
#define LPP_CYCLE_THRESHOLD 512

uint64_t lpp__cycle_collected_count = 0;
uint64_t lpp__runtime_drop_count = 0;

/* A minimal open-addressing uintptr_t -> int64_t map (0 = empty key,
 * UINTPTR_MAX = tombstone). Used for the trace registry and, per
 * round, the scratch index. All operations bail out (returning 0) on
 * allocation failure; collectors treat that as "collect nothing". */
typedef struct {
    uintptr_t *keys;
    int64_t *vals;
    size_t cap;
    size_t len;
    size_t used; /* len + tombstones */
} LppPtrMap;

static size_t lpp__ptr_hash(uintptr_t p) {
    p >>= 4;
    p *= (uintptr_t)0x9E3779B97F4A7C15u;
    return (size_t)(p >> 24);
}

static int lpp__map_grow(LppPtrMap *map) {
    size_t new_cap = map->cap ? map->cap * 2 : 64;
    uintptr_t *new_keys = (uintptr_t *)calloc(new_cap, sizeof(uintptr_t));
    int64_t *new_vals = (int64_t *)calloc(new_cap, sizeof(int64_t));
    if (!new_keys || !new_vals) {
        free(new_keys);
        free(new_vals);
        return 0;
    }
    for (size_t i = 0; i < map->cap; i++) {
        uintptr_t key = map->keys[i];
        if (!key || key == UINTPTR_MAX) continue;
        size_t at = lpp__ptr_hash(key) & (new_cap - 1);
        while (new_keys[at]) at = (at + 1) & (new_cap - 1);
        new_keys[at] = key;
        new_vals[at] = map->vals[i];
    }
    free(map->keys);
    free(map->vals);
    map->keys = new_keys;
    map->vals = new_vals;
    map->cap = new_cap;
    map->used = map->len;
    return 1;
}

static int64_t *lpp__map_slot(LppPtrMap *map, uintptr_t key, int create) {
    if (!map->cap || (create && (map->used + 1) * 10 >= map->cap * 7)) {
        if (!create && !map->cap) return (int64_t *)0;
        if (!lpp__map_grow(map)) return (int64_t *)0;
    }
    size_t mask = map->cap - 1;
    size_t at = lpp__ptr_hash(key) & mask;
    size_t tomb = SIZE_MAX;
    for (;;) {
        uintptr_t cur = map->keys[at];
        if (cur == key) return &map->vals[at];
        if (cur == UINTPTR_MAX && tomb == SIZE_MAX) tomb = at;
        if (!cur) {
            if (!create) return (int64_t *)0;
            size_t slot = (tomb != SIZE_MAX) ? tomb : at;
            if (tomb == SIZE_MAX) map->used++;
            map->keys[slot] = key;
            map->vals[slot] = 0;
            map->len++;
            return &map->vals[slot];
        }
        at = (at + 1) & mask;
    }
}

static void lpp__map_remove(LppPtrMap *map, uintptr_t key) {
    if (!map->cap) return;
    size_t mask = map->cap - 1;
    size_t at = lpp__ptr_hash(key) & mask;
    while (map->keys[at]) {
        if (map->keys[at] == key) {
            map->keys[at] = UINTPTR_MAX;
            map->len--;
            return;
        }
        at = (at + 1) & mask;
    }
}

static void lpp__map_free(LppPtrMap *map) {
    free(map->keys);
    free(map->vals);
    map->keys = (uintptr_t *)0;
    map->vals = (int64_t *)0;
    map->cap = map->len = map->used = 0;
}

static LppPtrMap lpp__trace_registry;

typedef struct {
    void *ptr;
    int32_t generation;
} LppCycleCandidate;

static LppCycleCandidate *lpp__root_buffer = (LppCycleCandidate *)0;
static size_t lpp__root_len = 0;
static size_t lpp__root_cap = 0;
static int lpp__collecting = 0;

void lpp_arc_collect_cycles(void);

static int lpp__cycle_valid(void *payload) {
    if (!payload) return 0;
    LppShimHeader *header = (LppShimHeader *)payload - 1;
    return header->magic == LPP_SHIM_IMMORTAL && !lpp_shim_is_immortal(header);
}

void lpp_arc_mark_collectible(void *payload, LppTraceFn trace) {
    if (!trace || !lpp__cycle_valid(payload)) return;
    int64_t *slot = lpp__map_slot(&lpp__trace_registry, (uintptr_t)payload, 1);
    if (!slot) return;
    *slot = (int64_t)(uintptr_t)trace;
    ((LppShimHeader *)payload - 1)->cycle_flags |= LPP_CYCLE_COLLECTIBLE;
}

static void lpp__cycle_deregister(void *payload) {
    lpp__map_remove(&lpp__trace_registry, (uintptr_t)payload);
}

static void lpp__note_possible_root(void *payload) {
    LppShimHeader *header = (LppShimHeader *)payload - 1;
    if (!(header->cycle_flags & LPP_CYCLE_COLLECTIBLE)) return;
    if (header->cycle_flags & LPP_CYCLE_BUFFERED) return;
    if (lpp__root_len == lpp__root_cap) {
        size_t new_cap = lpp__root_cap ? lpp__root_cap * 2 : 128;
        LppCycleCandidate *grown = (LppCycleCandidate *)realloc(
            lpp__root_buffer, new_cap * sizeof(LppCycleCandidate));
        if (!grown) return; /* unbuffered: a later release retries */
        lpp__root_buffer = grown;
        lpp__root_cap = new_cap;
    }
    lpp__root_buffer[lpp__root_len].ptr = payload;
    lpp__root_buffer[lpp__root_len].generation = header->generation;
    lpp__root_len++;
    header->cycle_flags |= LPP_CYCLE_BUFFERED;
    if (lpp__root_len >= LPP_CYCLE_THRESHOLD && !lpp__collecting) {
        lpp_arc_collect_cycles();
    }
}

/* Trace sink: trace functions report each owned child here. `ctx` is a
 * pointer to this sink, forwarded untouched by generated code. */
typedef struct {
    void **items;
    size_t len;
    size_t cap;
    int oom;
} LppTraceSink;

void lpp_trace_visit(void *ctx, void *child) {
    if (!ctx || !child) return;
    LppTraceSink *sink = (LppTraceSink *)ctx;
    if (sink->len == sink->cap) {
        size_t new_cap = sink->cap ? sink->cap * 2 : 32;
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

/* Returns 1 and fills `sink` with children if the node has a trace. */
static int lpp__trace_into(void *payload, LppTraceSink *sink) {
    int64_t *slot = lpp__map_slot(&lpp__trace_registry, (uintptr_t)payload, 0);
    if (!slot) return 0;
    sink->len = 0;
    sink->oom = 0;
    ((LppTraceFn)(uintptr_t)*slot)(payload, sink);
    return !sink->oom;
}

static size_t lpp__collect_round(void) {
    /* Drain the candidate buffer. */
    size_t cand_len = lpp__root_len;
    LppCycleCandidate *cands = lpp__root_buffer;
    lpp__root_buffer = (LppCycleCandidate *)0;
    lpp__root_len = lpp__root_cap = 0;
    if (!cand_len) {
        free(cands);
        return 0;
    }

    size_t freed = 0;
    LppPtrMap index = {0};            /* payload -> slot + 1 */
    void **nodes = (void **)0;        /* slot -> payload */
    int64_t *internal = (int64_t *)0; /* slot -> scratch count */
    uint8_t *live = (uint8_t *)0;     /* slot -> reached-from-root flag */
    size_t count = 0, cap = 0;
    size_t *stack = (size_t *)0;
    size_t stack_len = 0, stack_cap = 0;
    LppTraceSink sink = {0};

#define LPP__PUSH_NODE(payload_, count_, slot_out_)                        \
    do {                                                                   \
        if (count == cap) {                                                \
            size_t nc_ = cap ? cap * 2 : 64;                               \
            void **nn_ = (void **)realloc(nodes, nc_ * sizeof(void *));    \
            int64_t *ni_ = (int64_t *)realloc(internal, nc_ * sizeof(int64_t)); \
            uint8_t *nl_ = (uint8_t *)realloc(live, nc_ * sizeof(uint8_t)); \
            if (nn_) nodes = nn_;                                          \
            if (ni_) internal = ni_;                                       \
            if (nl_) live = nl_;                                           \
            if (!nn_ || !ni_ || !nl_) goto done;                           \
            cap = nc_;                                                     \
        }                                                                  \
        int64_t *ip_ = lpp__map_slot(&index, (uintptr_t)(payload_), 1);    \
        if (!ip_) goto done;                                               \
        *ip_ = (int64_t)count + 1;                                         \
        nodes[count] = (payload_);                                         \
        internal[count] = (count_);                                        \
        live[count] = 0;                                                   \
        *(slot_out_) = count;                                              \
        count++;                                                           \
    } while (0)

    /* Seed the gray set with still-valid candidates. */
    for (size_t i = 0; i < cand_len; i++) {
        void *payload = cands[i].ptr;
        if (!lpp__cycle_valid(payload)) continue;
        LppShimHeader *header = (LppShimHeader *)payload - 1;
        header->cycle_flags &= ~LPP_CYCLE_BUFFERED;
        if (!(header->cycle_flags & LPP_CYCLE_COLLECTIBLE)) continue;
        if (header->generation != cands[i].generation) continue;
        if (header->refcount <= 0) continue;
        if (!lpp__map_slot(&lpp__trace_registry, (uintptr_t)payload, 0)) continue;
        if (lpp__map_slot(&index, (uintptr_t)payload, 0)) continue;
        size_t slot;
        LPP__PUSH_NODE(payload, header->refcount, &slot);
        if (stack_len == stack_cap) {
            size_t nc = stack_cap ? stack_cap * 2 : 64;
            size_t *ns = (size_t *)realloc(stack, nc * sizeof(size_t));
            if (!ns) goto done;
            stack = ns;
            stack_cap = nc;
        }
        stack[stack_len++] = slot;
    }

    /* Mark-gray: subtract one scratch count per internal edge. */
    while (stack_len) {
        size_t slot = stack[--stack_len];
        if (!lpp__trace_into(nodes[slot], &sink)) {
            /* Gone or out of memory: keep the subgraph alive. */
            internal[slot] = internal[slot] < 1 ? 1 : internal[slot];
            continue;
        }
        for (size_t j = 0; j < sink.len; j++) {
            void *child = sink.items[j];
            if (!lpp__cycle_valid(child)) continue;
            LppShimHeader *cheader = (LppShimHeader *)child - 1;
            if (!(cheader->cycle_flags & LPP_CYCLE_COLLECTIBLE)) continue;
            if (cheader->refcount <= 0) continue;
            int64_t *existing = lpp__map_slot(&index, (uintptr_t)child, 0);
            if (existing) {
                internal[(size_t)*existing - 1]--;
                continue;
            }
            if (!lpp__map_slot(&lpp__trace_registry, (uintptr_t)child, 0)) {
                /* Untraceable collectible: pin it live this round. */
                size_t pinned;
                LPP__PUSH_NODE(child, 1, &pinned);
                live[pinned] = 1;
                continue;
            }
            size_t child_slot;
            LPP__PUSH_NODE(child, (int64_t)cheader->refcount - 1, &child_slot);
            if (stack_len == stack_cap) {
                size_t nc = stack_cap ? stack_cap * 2 : 64;
                size_t *ns = (size_t *)realloc(stack, nc * sizeof(size_t));
                if (!ns) goto done;
                stack = ns;
                stack_cap = nc;
            }
            stack[stack_len++] = child_slot;
        }
    }

    /* Scan: nodes with external references are live roots. */
    stack_len = 0;
    for (size_t i = 0; i < count; i++) {
        if (internal[i] > 0 && !live[i]) {
            live[i] = 1;
            if (stack_len == stack_cap) {
                size_t nc = stack_cap ? stack_cap * 2 : 64;
                size_t *ns = (size_t *)realloc(stack, nc * sizeof(size_t));
                if (!ns) goto done;
                stack = ns;
                stack_cap = nc;
            }
            stack[stack_len++] = i;
        }
    }
    while (stack_len) {
        size_t slot = stack[--stack_len];
        /* A failed trace here would under-mark liveness: abort the
         * round (nothing freed) rather than risk a live node. */
        if (!lpp__trace_into(nodes[slot], &sink)) goto done;
        for (size_t j = 0; j < sink.len; j++) {
            int64_t *existing = lpp__map_slot(&index, (uintptr_t)sink.items[j], 0);
            if (!existing) continue;
            size_t child_slot = (size_t)*existing - 1;
            if (!live[child_slot]) {
                live[child_slot] = 1;
                if (stack_len == stack_cap) {
                    size_t nc = stack_cap ? stack_cap * 2 : 64;
                    size_t *ns = (size_t *)realloc(stack, nc * sizeof(size_t));
                    if (!ns) goto done;
                    stack = ns;
                    stack_cap = nc;
                }
                stack[stack_len++] = child_slot;
            }
        }
    }

    /* Collect: invalidate first (member destructors releasing members
     * become no-ops via the magic check in release), then destroy and
     * free in slot order. */
    for (size_t i = 0; i < count; i++) {
        if (live[i]) continue;
        LppShimHeader *header = (LppShimHeader *)nodes[i] - 1;
        header->generation++;
        header->magic = 0;
        header->cycle_flags &= LPP_SHIM_HAS_DTOR;
        lpp__cycle_deregister(nodes[i]);
    }
    /* Destructors first, storage second: a member destructor that
     * releases another member must land on that member's intact
     * (invalidated) header, never on recycled memory. */
    for (size_t i = 0; i < count; i++) {
        if (live[i]) continue;
        LppShimHeader *header = (LppShimHeader *)nodes[i] - 1;
        lpp__runtime_drop_count++;
        lpp__cycle_collected_count++;
        LppShimDestructor destructor = lpp_shim_destructor(header);
        if (destructor) destructor(nodes[i]);
        freed++;
    }
    for (size_t i = 0; i < count; i++) {
        if (live[i]) continue;
        free(lpp_shim_allocation_base((LppShimHeader *)nodes[i] - 1));
    }

done:
    lpp__map_free(&index);
    free(nodes);
    free(internal);
    free(live);
    free(stack);
    free(sink.items);
    free(cands);
    return freed;
#undef LPP__PUSH_NODE
}

void lpp_arc_collect_cycles(void) {
    if (lpp__collecting) return;
    lpp__collecting = 1;
    for (;;) {
        size_t freed = lpp__collect_round();
        if (!freed || !lpp__root_len) break;
    }
    lpp__collecting = 0;
}

void lpp_arc_retain(void *payload) {
    if (!payload) return;
    LppShimHeader *header = (LppShimHeader *)payload - 1;
    if (header->magic != LPP_SHIM_IMMORTAL) return;
    if (lpp_shim_is_immortal(header)) return;
    header->refcount++;
}

void lpp_arc_release(void *payload) {
    if (!payload) return;
    LppShimHeader *header = (LppShimHeader *)payload - 1;
    if (header->magic != LPP_SHIM_IMMORTAL) return;
    if (lpp_shim_is_immortal(header)) return;
    if (--header->refcount == 0) {
        lpp__runtime_drop_count++;
        header->generation++;
        header->magic = 0;
        if (header->cycle_flags & LPP_CYCLE_COLLECTIBLE) {
            lpp__cycle_deregister(payload);
        }
        LppShimDestructor destructor = lpp_shim_destructor(header);
        if (destructor) destructor(payload);
        free(lpp_shim_allocation_base(header));
    } else if (header->refcount > 0) {
        lpp__note_possible_root(payload);
    }
}

void lpp_print_str(const char *text) {
    fputs(text, stdout);
    fputc('\n', stdout);
}

