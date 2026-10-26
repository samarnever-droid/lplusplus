/* ── 5C2: closures, tuples, tasks ─────────────────────────────────────────── */

/* The closure capsule payload is [code @0, env @8]; the capsule's
 * destructor releases the env reference (NULL for zero-capture
 * closures is a no-op). */
void lpp_closure_destroy(void *payload) {
    int64_t env;
    memcpy(&env, (char *)payload + 8, sizeof(env));
    lpp_arc_release((void *)(intptr_t)env);
}

void lpp_closure_trace(void *payload, void *ctx) {
    int64_t env;
    memcpy(&env, (char *)payload + 8, sizeof(env));
    lpp_trace_visit(ctx, (void *)(intptr_t)env);
}

/* The task env tuple: 16-byte prefix (managed mask + packed 16-bit
 * absolute offsets), value slots from offset 16. No slot-count limit
 * beyond the 64 mask bits. */
static void lpp_shim_tuple_destroy(void *raw) {
    int64_t mask, offsets;
    memcpy(&mask, raw, sizeof(mask));
    memcpy(&offsets, (char *)raw + 8, sizeof(offsets));
    for (int i = 0; i < 64; i++) {
        if (!((uint64_t)mask >> i & 1)) continue;
        int64_t offset = (offsets >> (16 * i)) & 0xFFFF;
        if (!offset) continue;
        int64_t slot;
        memcpy(&slot, (char *)raw + offset, sizeof(slot));
        lpp_arc_release((void *)(intptr_t)slot);
    }
}

void lpp_tuple_trace(void *raw, void *ctx) {
    int64_t mask, offsets;
    memcpy(&mask, raw, sizeof(mask));
    memcpy(&offsets, (char *)raw + 8, sizeof(offsets));
    for (int i = 0; i < 64; i++) {
        if (!((uint64_t)mask >> i & 1)) continue;
        int64_t offset = (offsets >> (16 * i)) & 0xFFFF;
        if (!offset) continue;
        int64_t slot;
        memcpy(&slot, (char *)raw + offset, sizeof(slot));
        lpp_trace_visit(ctx, (void *)(intptr_t)slot);
    }
}

void lpp_panic_div_zero(void) { fprintf(stderr, "lpp shim: integer division by zero\n"); exit(101); }

void *lpp_tuple_alloc(int64_t size, int64_t mask, int64_t packed_offsets) {
    if (size < 16) size = 16;
    if (size > 16 + 64 * 8) { fprintf(stderr, "lpp shim: tuple too large\n"); exit(101); }
    void *payload = lpp_arc_alloc_with_destructor(size, lpp_shim_tuple_destroy);
    if (!payload) return (void *)0;
    memcpy(payload, &mask, sizeof(mask));
    memcpy((char *)payload + 8, &packed_offsets, sizeof(packed_offsets));
    return payload;
}

/* The task node: { code, environment, result, state, result_managed }.
 * The destructor releases the environment and, once resolved and
 * managed, the result. */
typedef struct {
    int64_t code;
    int64_t environment;
    int64_t result;
    int32_t state;
    int32_t result_managed;
} LppShimTask;

static void lpp_shim_task_destroy(void *raw) {
    LppShimTask *task = (LppShimTask *)raw;
    lpp_arc_release((void *)(intptr_t)task->environment);
    if (task->state == 2 && task->result_managed) {
        lpp_arc_release((void *)(intptr_t)task->result);
    }
}

void lpp_task_trace(void *raw, void *ctx) {
    LppShimTask *task = (LppShimTask *)raw;
    lpp_trace_visit(ctx, (void *)(intptr_t)task->environment);
    if (task->state == 2 && task->result_managed) {
        lpp_trace_visit(ctx, (void *)(intptr_t)task->result);
    }
}

void *lpp_task_new(int64_t code, int64_t environment, int64_t managed) {
    if (!code || !environment) {
        fprintf(stderr, "lpp shim: task with null code or environment\n");
        exit(101);
    }
    void *payload = lpp_arc_alloc_with_destructor(sizeof(LppShimTask), lpp_shim_task_destroy);
    if (!payload) return (void *)0;
    LppShimTask *task = (LppShimTask *)payload;
    task->code = code;
    task->environment = environment;
    task->result = 0;
    task->state = 0;
    task->result_managed = (int32_t)managed;
    return payload;
}

int64_t lpp_task_poll(int64_t task_raw) {
    LppShimTask *task = (LppShimTask *)(intptr_t)task_raw;
    if (!task || task->state != 0) {
        fprintf(stderr, "lpp shim: poll of a non-fresh task\n");
        exit(101);
    }
    task->state = 1;
    int64_t result = ((int64_t (*)(void *))task->code)((void *)(intptr_t)task->environment);
    task->result = result;
    task->state = 2;
    return result;
}

int64_t lpp_task_await(int64_t task_raw) {
    LppShimTask *task = (LppShimTask *)(intptr_t)task_raw;
    if (!task || !task->environment) {
        fprintf(stderr, "lpp shim: await of a null task\n");
        exit(101);
    }
    if (task->state == 0) lpp_task_poll(task_raw);
    /* The task keeps its share; the awaiter gains one (the oracle's
     * repeated-await semantics). */
    if (task->result_managed) lpp_arc_retain((void *)(intptr_t)task->result);
    return task->result;
}

void lpp_task_destroy(int64_t task_raw) {
    lpp_arc_release((void *)(intptr_t)task_raw);
}

