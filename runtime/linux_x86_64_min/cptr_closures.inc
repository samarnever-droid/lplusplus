/* ── Native CPtr & Memory Builtins ── */
int64_t lpp_c_malloc(int64_t size) {
    if (size <= 0) return 0;
    uint64_t total = lpp_page_round((uint64_t)size + sizeof(uint64_t));
    uint64_t *ptr = (uint64_t *)lpp_sys_mmap(total);
    if (!ptr) return 0;
    ptr[0] = total;
    return (int64_t)(uintptr_t)(ptr + 1);
}

void lpp_c_free(int64_t ptr) {
    if (ptr != 0) {
        uint64_t *hdr = (uint64_t *)(uintptr_t)ptr - 1;
        lpp_sys_munmap(hdr, hdr[0]);
    }
}

int64_t lpp_c_load_u8(int64_t ptr, int64_t offset) {
    if (ptr == 0) return 0;
    const uint8_t *p = (const uint8_t *)(uintptr_t)(ptr + offset);
    return (int64_t)(*p);
}

void lpp_c_store_u8(int64_t ptr, int64_t offset, int64_t val) {
    if (ptr == 0) return;
    uint8_t *p = (uint8_t *)(uintptr_t)(ptr + offset);
    *p = (uint8_t)val;
}

int64_t lpp_c_load_i32(int64_t ptr, int64_t offset) {
    if (ptr == 0) return 0;
    const int32_t *p = (const int32_t *)(uintptr_t)(ptr + offset);
    return (int64_t)(*p);
}

void lpp_c_store_i32(int64_t ptr, int64_t offset, int64_t val) {
    if (ptr == 0) return;
    int32_t *p = (int32_t *)(uintptr_t)(ptr + offset);
    *p = (int32_t)val;
}

int64_t lpp_c_load_i64(int64_t ptr, int64_t offset) {
    if (ptr == 0) return 0;
    const int64_t *p = (const int64_t *)(uintptr_t)(ptr + offset);
    return *p;
}

void lpp_c_store_i64(int64_t ptr, int64_t offset, int64_t val) {
    if (ptr == 0) return;
    int64_t *p = (int64_t *)(uintptr_t)(ptr + offset);
    *p = val;
}

/* ARC closure payload: [code pointer, owned environment pointer]. */
void lpp_closure_destroy(void *closure) {
    if (!closure) return;
    void **parts = (void **)closure;
    lpp_arc_release(parts[1]);
}

void lpp_closure_trace(void *closure, void *ctx) {
    if (!closure) return;
    void **parts = (void **)closure;
    lpp_trace_visit(ctx, parts[1]);
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
void lpp_tuple_trace(void *payload, void *ctx) {
    LppTuplePrefix *tuple = (LppTuplePrefix *)payload;
    for (unsigned i = 0; tuple && i < 4; ++i) {
        if ((tuple->managed_mask & ((uint64_t)1 << i)) == 0) continue;
        uint64_t offset = (tuple->packed_offsets >> (i * 16)) & 0xffffu;
        lpp_trace_visit(ctx, *(void **)((char *)payload + offset));
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
void lpp_task_trace(void *payload, void *ctx) {
    LppTask *task = (LppTask *)payload;
    if (!task) return;
    lpp_trace_visit(ctx, task->environment);
    if (task->state == 2 && task->result_managed && task->result) {
        lpp_trace_visit(ctx, (void *)(intptr_t)task->result);
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

