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
    InterlockedIncrement(&hdr->refcount);
#else
    atomic_fetch_add_explicit(&hdr->refcount, 1, memory_order_relaxed);
#endif
}

void lpp_arc_release_local(void *ptr) {
    if (!lpp__is_valid_arc_ptr(ptr)) return;
    LppArcHeader *hdr = (LppArcHeader *)ptr - 1;
    if (lpp__is_immortal(hdr)) return;
    uint32_t flags = lpp__cycle_flags_load(hdr);
    int trigger = 0;
    if ((flags & LPP_CYCLE_COLLECTIBLE) && LPP_ARC_LOAD(&hdr->refcount) > 1) {
        trigger = lpp__note_possible_root_buffer(ptr);
    }
    int32_t prev = (int32_t)LPP_ARC_DEC(&hdr->refcount);
    if (prev == 1) {
        LppArenaRegion *arena = lpp__arena_for_header(hdr);
        lpp__inc_drop_count();
        if (flags & LPP_CYCLE_COLLECTIBLE) {
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
    } else if (prev > 1 && trigger) {
        lpp_arc_collect_cycles();
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

void lpp_closure_trace(void *closure, void *ctx) {
    if (!closure) return;
    void **parts = (void **)closure;
    lpp_trace_visit(ctx, parts[1]);
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

void lpp_tuple_trace(void *payload, void *ctx) {
    LppTuplePrefix *tuple = (LppTuplePrefix *)payload;
    if (!tuple) return;
    for (unsigned i = 0; i < 4; ++i) {
        if ((tuple->managed_mask & (UINT64_C(1) << i)) == 0) continue;
        uint64_t offset = (tuple->packed_offsets >> (i * 16)) & UINT64_C(0xffff);
        void *child = *(void **)((unsigned char *)payload + offset);
        lpp_trace_visit(ctx, child);
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

#if defined(_WIN32)
#  ifndef WIN32_LEAN_AND_MEAN
#    define WIN32_LEAN_AND_MEAN
#  endif
#  include <windows.h>
typedef CRITICAL_SECTION lpp_pool_mutex_t;
typedef CONDITION_VARIABLE lpp_pool_cond_t;
#  define LPP_POOL_LOCK(m)          EnterCriticalSection(m)
#  define LPP_POOL_UNLOCK(m)        LeaveCriticalSection(m)
#  define LPP_POOL_MUTEX_INIT(m)    InitializeCriticalSection(m)
#  define LPP_POOL_COND_INIT(c)     InitializeConditionVariable(c)
#  define LPP_POOL_COND_WAIT(c, m)  SleepConditionVariableCS((c), (m), INFINITE)
#  define LPP_POOL_COND_SIGNAL(c)   WakeConditionVariable(c)
#  define LPP_POOL_COND_BCAST(c)    WakeAllConditionVariable(c)
#  if defined(_MSC_VER)
#    define LPP_TLS __declspec(thread)
#  else
#    define LPP_TLS __thread
#  endif
#else
#  include <pthread.h>
#  include <unistd.h>
typedef pthread_mutex_t lpp_pool_mutex_t;
typedef pthread_cond_t lpp_pool_cond_t;
#  define LPP_POOL_LOCK(m)          pthread_mutex_lock(m)
#  define LPP_POOL_UNLOCK(m)        pthread_mutex_unlock(m)
#  define LPP_POOL_MUTEX_INIT(m)    pthread_mutex_init((m), NULL)
#  define LPP_POOL_COND_INIT(c)     pthread_cond_init((c), NULL)
#  define LPP_POOL_COND_WAIT(c, m)  pthread_cond_wait((c), (m))
#  define LPP_POOL_COND_SIGNAL(c)   pthread_cond_signal(c)
#  define LPP_POOL_COND_BCAST(c)    pthread_cond_broadcast(c)
#  define LPP_TLS __thread
#endif

typedef int64_t (*LppTaskCode)(void *environment);
typedef struct {
    LppTaskCode code;
    void *environment;
    int64_t result;
    lpp_atomic32_t state;   /* 0 pending, 1 running, 2 complete */
    int32_t result_managed;
} LppTask;

typedef struct LppTaskHandle {
    void *task;
    int worker_owns;
    int finished;
    int in_map;
    int refs;
    struct LppTaskHandle *prev_q;
    struct LppTaskHandle *next_q;
    struct LppTaskHandle *next_map;
} LppTaskHandle;

typedef struct {
    lpp_pool_mutex_t mu;
    LppTaskHandle *head;
    LppTaskHandle *tail;
    int64_t _pad[4];
} LppTaskDeque;

#define LPP_TASK_MAX_WORKERS 64
#define LPP_TASK_SHARDS 64
#define LPP_TASK_BUCKETS 256

static struct {
    int initialized;
    int num_workers;
    int sleeping;
    lpp_pool_mutex_t park_mu;
    lpp_pool_cond_t park_cv;
    lpp_pool_mutex_t stat_mu;
    int64_t tasks_completed;
    int64_t tasks_stolen;
    LppTaskDeque injector;
    LppTaskDeque workers[LPP_TASK_MAX_WORKERS];
    lpp_pool_mutex_t shard_mu[LPP_TASK_SHARDS];
    lpp_pool_cond_t shard_done_cv[LPP_TASK_SHARDS];
    LppTaskHandle *buckets[LPP_TASK_BUCKETS];
} g_lpp_task_pool = { 0 };

#if defined(_WIN32)
static INIT_ONCE g_lpp_pool_once = INIT_ONCE_STATIC_INIT;
#else
static pthread_once_t g_lpp_pool_once = PTHREAD_ONCE_INIT;
#endif

#define LPP_ACTIVE_TASK_CAP 256
static LPP_TLS void *g_lpp_active_tasks[LPP_ACTIVE_TASK_CAP];
static LPP_TLS int g_lpp_active_depth = 0;
static LPP_TLS int g_lpp_worker_id = -1;

static size_t lpp_task_bucket(const void *ptr) {
    uintptr_t v = (uintptr_t)ptr;
    return ((v >> 4) ^ (v >> 13)) & (LPP_TASK_BUCKETS - 1);
}

static int lpp_configured_workers(void) {
    const char *env = getenv("LPP_ASYNC_THREADS");
    if (env && env[0] != '\0') {
        long parsed = strtol(env, NULL, 10);
        if (parsed >= 2 && parsed <= LPP_TASK_MAX_WORKERS) return (int)parsed;
        if (parsed > LPP_TASK_MAX_WORKERS) return LPP_TASK_MAX_WORKERS;
        if (parsed > 0) return 2;
    }
#if defined(_WIN32)
    SYSTEM_INFO si;
    GetSystemInfo(&si);
    int w = (int)si.dwNumberOfProcessors;
#else
    long cpus = sysconf(_SC_NPROCESSORS_ONLN);
    int w = cpus > 0 ? (int)cpus : 4;
#endif
    if (w < 2) w = 2;
    if (w > 16) w = 16;
    return w;
}

static void lpp_stat_inc_completed(void) {
    LPP_POOL_LOCK(&g_lpp_task_pool.stat_mu);
    g_lpp_task_pool.tasks_completed++;
    LPP_POOL_UNLOCK(&g_lpp_task_pool.stat_mu);
}

static void lpp_stat_inc_stolen(void) {
    LPP_POOL_LOCK(&g_lpp_task_pool.stat_mu);
    g_lpp_task_pool.tasks_stolen++;
    LPP_POOL_UNLOCK(&g_lpp_task_pool.stat_mu);
}

static void lpp_deque_push_back(LppTaskDeque *dq, LppTaskHandle *h) {
    LPP_POOL_LOCK(&dq->mu);
    h->prev_q = dq->tail;
    h->next_q = NULL;
    if (dq->tail) {
        dq->tail->next_q = h;
    } else {
        dq->head = h;
    }
    dq->tail = h;
    LPP_POOL_UNLOCK(&dq->mu);
}

static LppTaskHandle *lpp_deque_pop_back(LppTaskDeque *dq) {
    LPP_POOL_LOCK(&dq->mu);
    LppTaskHandle *h = dq->tail;
    if (h) {
        dq->tail = h->prev_q;
        if (dq->tail) {
            dq->tail->next_q = NULL;
        } else {
            dq->head = NULL;
        }
        h->prev_q = NULL;
        h->next_q = NULL;
    }
    LPP_POOL_UNLOCK(&dq->mu);
    return h;
}

static LppTaskHandle *lpp_deque_pop_front(LppTaskDeque *dq) {
    LPP_POOL_LOCK(&dq->mu);
    LppTaskHandle *h = dq->head;
    if (h) {
        dq->head = h->next_q;
        if (dq->head) {
            dq->head->prev_q = NULL;
        } else {
            dq->tail = NULL;
        }
        h->prev_q = NULL;
        h->next_q = NULL;
    }
    LPP_POOL_UNLOCK(&dq->mu);
    return h;
}

static int lpp_deque_is_empty(LppTaskDeque *dq) {
    LPP_POOL_LOCK(&dq->mu);
    int empty = (dq->head == NULL);
    LPP_POOL_UNLOCK(&dq->mu);
    return empty;
}

static int lpp_has_any_pending_work(void) {
    if (!lpp_deque_is_empty(&g_lpp_task_pool.injector)) return 1;
    for (int i = 0; i < g_lpp_task_pool.num_workers; i++) {
        if (!lpp_deque_is_empty(&g_lpp_task_pool.workers[i])) return 1;
    }
    return 0;
}

static LppTaskHandle *lpp_pop_next_task(int wid, unsigned int ticks) {
    LppTaskHandle *h = NULL;
    if (wid >= 0 && wid < g_lpp_task_pool.num_workers) {
        if ((ticks & 63u) == 0u) {
            h = lpp_deque_pop_front(&g_lpp_task_pool.injector);
            if (h) return h;
        }
        h = lpp_deque_pop_back(&g_lpp_task_pool.workers[wid]);
        if (h) return h;
        h = lpp_deque_pop_front(&g_lpp_task_pool.injector);
        if (h) return h;
        int n = g_lpp_task_pool.num_workers;
        for (int off = 1; off < n; off++) {
            int victim = (wid + off) % n;
            h = lpp_deque_pop_front(&g_lpp_task_pool.workers[victim]);
            if (h) {
                lpp_stat_inc_stolen();
                return h;
            }
        }
        return NULL;
    }
    h = lpp_deque_pop_front(&g_lpp_task_pool.injector);
    if (h) return h;
    for (int v = 0; v < g_lpp_task_pool.num_workers; v++) {
        h = lpp_deque_pop_front(&g_lpp_task_pool.workers[v]);
        if (h) {
            lpp_stat_inc_stolen();
            return h;
        }
    }
    return NULL;
}

static void lpp_wake_one_sleeper(void) {
    LPP_POOL_LOCK(&g_lpp_task_pool.park_mu);
    if (g_lpp_task_pool.sleeping > 0) {
        LPP_POOL_COND_SIGNAL(&g_lpp_task_pool.park_cv);
    }
    LPP_POOL_UNLOCK(&g_lpp_task_pool.park_mu);
}

static int lpp_active_contains(const void *raw_task) {
    for (int i = 0; i < g_lpp_active_depth; i++) {
        if (g_lpp_active_tasks[i] == raw_task) return 1;
    }
    return 0;
}

static void lpp_active_push(void *raw_task) {
    if (lpp_active_contains(raw_task)) {
        lpp_panic("concurrent or recursive polling of the same task");
    }
    if (g_lpp_active_depth >= LPP_ACTIVE_TASK_CAP) {
        lpp_panic("task active nesting depth exceeded");
    }
    g_lpp_active_tasks[g_lpp_active_depth++] = raw_task;
}

static void lpp_active_pop(void) {
    if (g_lpp_active_depth > 0) {
        g_lpp_active_depth--;
    }
}

static void lpp_task_drive(void *raw_task) {
    LppTask *task = (LppTask *)raw_task;
    if (LPP_ARC_LOAD(&task->state) == 2) return;
#if defined(_MSC_VER)
    if (InterlockedCompareExchange(&task->state, 1, 0) != 0) {
        if (LPP_ARC_LOAD(&task->state) == 2) return;
        lpp_panic("concurrent or recursive polling of the same task");
    }
#else
    int32_t expected = 0;
    if (!atomic_compare_exchange_strong_explicit(
            &task->state, &expected, 1, memory_order_acq_rel, memory_order_acquire)) {
        if (expected == 2) return;
        lpp_panic("concurrent or recursive polling of the same task");
    }
#endif
    lpp_active_push(raw_task);
    task->result = task->code(task->environment);
    lpp_active_pop();
#if defined(_MSC_VER)
    InterlockedExchange(&task->state, 2);
#else
    atomic_store_explicit(&task->state, 2, memory_order_release);
#endif
    lpp_stat_inc_completed();
}

static void lpp_handle_release_locked(LppTaskHandle *h) {
    h->refs--;
    if (h->refs == 0) {
        free(h);
    }
}

static void lpp_task_execute_claimed(void *raw_task, LppTaskHandle *h) {
    size_t s = lpp_task_bucket(raw_task) & (LPP_TASK_SHARDS - 1);
    lpp_task_drive(raw_task);
    /* Release pool ARC reference before notifying waiters so an immediate
     * caller-side destroy drops refcount synchronously. */
    lpp_arc_release(raw_task);
    LPP_POOL_LOCK(&g_lpp_task_pool.shard_mu[s]);
    h->finished = 1;
    LPP_POOL_COND_BCAST(&g_lpp_task_pool.shard_done_cv[s]);
    lpp_handle_release_locked(h);
    LPP_POOL_UNLOCK(&g_lpp_task_pool.shard_mu[s]);
}

static int lpp_try_steal_and_run_one(void) {
    LppTaskHandle *h = lpp_pop_next_task(g_lpp_worker_id, 1u);
    if (!h) return 0;
    size_t s = lpp_task_bucket(h->task) & (LPP_TASK_SHARDS - 1);
    LPP_POOL_LOCK(&g_lpp_task_pool.shard_mu[s]);
    if (h->worker_owns) {
        h->worker_owns = 0;
        void *raw_task = h->task;
        LPP_POOL_UNLOCK(&g_lpp_task_pool.shard_mu[s]);
        lpp_task_execute_claimed(raw_task, h);
        return 1;
    }
    lpp_handle_release_locked(h);
    LPP_POOL_UNLOCK(&g_lpp_task_pool.shard_mu[s]);
    return 0;
}

static void lpp_task_worker_run(int wid) {
    g_lpp_worker_id = wid;
    unsigned int ticks = 1u;
    for (;;) {
        LppTaskHandle *h = lpp_pop_next_task(wid, ticks);
        if (h) {
            ticks++;
            size_t s = lpp_task_bucket(h->task) & (LPP_TASK_SHARDS - 1);
            LPP_POOL_LOCK(&g_lpp_task_pool.shard_mu[s]);
            if (h->worker_owns) {
                h->worker_owns = 0;
                void *raw_task = h->task;
                LPP_POOL_UNLOCK(&g_lpp_task_pool.shard_mu[s]);
                lpp_task_execute_claimed(raw_task, h);
            } else {
                lpp_handle_release_locked(h);
                LPP_POOL_UNLOCK(&g_lpp_task_pool.shard_mu[s]);
            }
            continue;
        }
        LPP_POOL_LOCK(&g_lpp_task_pool.park_mu);
        if (lpp_has_any_pending_work()) {
            LPP_POOL_UNLOCK(&g_lpp_task_pool.park_mu);
            continue;
        }
        g_lpp_task_pool.sleeping++;
        LPP_POOL_COND_WAIT(&g_lpp_task_pool.park_cv, &g_lpp_task_pool.park_mu);
        g_lpp_task_pool.sleeping--;
        LPP_POOL_UNLOCK(&g_lpp_task_pool.park_mu);
    }
}

static void lpp_task_pool_init_common(void) {
    LPP_POOL_MUTEX_INIT(&g_lpp_task_pool.park_mu);
    LPP_POOL_COND_INIT(&g_lpp_task_pool.park_cv);
    LPP_POOL_MUTEX_INIT(&g_lpp_task_pool.stat_mu);
    LPP_POOL_MUTEX_INIT(&g_lpp_task_pool.injector.mu);
    for (int i = 0; i < LPP_TASK_SHARDS; i++) {
        LPP_POOL_MUTEX_INIT(&g_lpp_task_pool.shard_mu[i]);
        LPP_POOL_COND_INIT(&g_lpp_task_pool.shard_done_cv[i]);
    }
    int workers = lpp_configured_workers();
    g_lpp_task_pool.num_workers = workers;
    for (int i = 0; i < workers; i++) {
        LPP_POOL_MUTEX_INIT(&g_lpp_task_pool.workers[i].mu);
    }
    g_lpp_task_pool.initialized = 1;
}

#if defined(_WIN32)
static DWORD WINAPI lpp_task_worker_main(LPVOID arg) {
    lpp_task_worker_run((int)(intptr_t)arg);
    return 0;
}

static BOOL CALLBACK lpp_task_pool_init_cb(PINIT_ONCE once, PVOID param, PVOID *ctx) {
    (void)once; (void)param; (void)ctx;
    lpp_task_pool_init_common();
    for (int i = 0; i < g_lpp_task_pool.num_workers; i++) {
        HANDLE th = CreateThread(NULL, 0, lpp_task_worker_main, (LPVOID)(intptr_t)i, 0, NULL);
        if (th) CloseHandle(th);
    }
    return TRUE;
}
#else
static void *lpp_task_worker_main(void *arg) {
    lpp_task_worker_run((int)(intptr_t)arg);
    return NULL;
}

static void lpp_task_pool_init_posix(void) {
    lpp_task_pool_init_common();
    for (int i = 0; i < g_lpp_task_pool.num_workers; i++) {
        pthread_t th;
        if (pthread_create(&th, NULL, lpp_task_worker_main, (void *)(intptr_t)i) == 0) {
            pthread_detach(th);
        }
    }
}
#endif

static void lpp_task_pool_ensure(void) {
#if defined(_WIN32)
    InitOnceExecuteOnce(&g_lpp_pool_once, lpp_task_pool_init_cb, NULL, NULL);
#else
    pthread_once(&g_lpp_pool_once, lpp_task_pool_init_posix);
#endif
}

static LppTaskHandle *lpp_task_find_locked(size_t bucket, const void *raw_task) {
    LppTaskHandle *cur = g_lpp_task_pool.buckets[bucket];
    while (cur) {
        if (cur->task == raw_task) return cur;
        cur = cur->next_map;
    }
    return NULL;
}

static void lpp_task_payload_destroy(void *payload) {
    LppTask *task = (LppTask *)payload;
    if (!task) return;
    if (g_lpp_task_pool.initialized) {
        size_t b = lpp_task_bucket(payload);
        size_t s = b & (LPP_TASK_SHARDS - 1);
        LPP_POOL_LOCK(&g_lpp_task_pool.shard_mu[s]);
        LppTaskHandle **prev = &g_lpp_task_pool.buckets[b];
        LppTaskHandle *cur = *prev;
        while (cur) {
            if (cur->task == payload) {
                *prev = cur->next_map;
                cur->in_map = 0;
                lpp_handle_release_locked(cur);
                break;
            }
            prev = &cur->next_map;
            cur = cur->next_map;
        }
        LPP_POOL_UNLOCK(&g_lpp_task_pool.shard_mu[s]);
    }
    if (task->environment) {
        lpp_arc_release(task->environment);
        task->environment = NULL;
    }
    if (LPP_ARC_LOAD(&task->state) == 2 && task->result_managed && task->result) {
        lpp_arc_release((void *)(intptr_t)task->result);
        task->result = 0;
    }
}

void lpp_task_trace(void *payload, void *ctx) {
    LppTask *task = (LppTask *)payload;
    if (!task) return;
    lpp_trace_visit(ctx, task->environment);
    if (LPP_ARC_LOAD(&task->state) == 2 && task->result_managed && task->result) {
        lpp_trace_visit(ctx, (void *)(intptr_t)task->result);
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
    lpp_task_pool_ensure();
    LppTaskHandle *h = (LppTaskHandle *)calloc(1, sizeof(LppTaskHandle));
    if (!h) lpp_panic("out of memory while allocating task handle");
    h->task = task;
    h->worker_owns = 1;
    h->finished = 0;
    h->in_map = 1;
    h->refs = 2; /* 1 map entry + 1 queue slot */
    /* Retain one ARC reference for the pool execution slot. */
    lpp_arc_retain(task);
    size_t b = lpp_task_bucket(task);
    size_t s = b & (LPP_TASK_SHARDS - 1);
    LPP_POOL_LOCK(&g_lpp_task_pool.shard_mu[s]);
    h->next_map = g_lpp_task_pool.buckets[b];
    g_lpp_task_pool.buckets[b] = h;
    LPP_POOL_UNLOCK(&g_lpp_task_pool.shard_mu[s]);

    int wid = g_lpp_worker_id;
    if (wid >= 0 && wid < g_lpp_task_pool.num_workers) {
        lpp_deque_push_back(&g_lpp_task_pool.workers[wid], h);
    } else {
        lpp_deque_push_back(&g_lpp_task_pool.injector, h);
    }
    lpp_wake_one_sleeper();
    return task;
}

int64_t lpp_task_poll(void *raw_task) {
    if (!raw_task) lpp_panic("attempted to poll a null task");
    if (lpp_active_contains(raw_task)) {
        lpp_panic("concurrent or recursive polling of the same task");
    }
    lpp_task_pool_ensure();
    size_t b = lpp_task_bucket(raw_task);
    size_t s = b & (LPP_TASK_SHARDS - 1);
    LPP_POOL_LOCK(&g_lpp_task_pool.shard_mu[s]);
    LppTaskHandle *h = lpp_task_find_locked(b, raw_task);
    if (h) {
        h->refs++;
        if (h->worker_owns) {
            h->worker_owns = 0;
            LPP_POOL_UNLOCK(&g_lpp_task_pool.shard_mu[s]);
            lpp_task_execute_claimed(raw_task, h);
            return 1;
        }
        while (!h->finished) {
            LPP_POOL_UNLOCK(&g_lpp_task_pool.shard_mu[s]);
            if (lpp_try_steal_and_run_one()) {
                LPP_POOL_LOCK(&g_lpp_task_pool.shard_mu[s]);
                continue;
            }
            LPP_POOL_LOCK(&g_lpp_task_pool.shard_mu[s]);
            if (!h->finished) {
                LPP_POOL_COND_WAIT(&g_lpp_task_pool.shard_done_cv[s], &g_lpp_task_pool.shard_mu[s]);
            }
        }
        lpp_handle_release_locked(h);
        LPP_POOL_UNLOCK(&g_lpp_task_pool.shard_mu[s]);
        return 1;
    }
    LPP_POOL_UNLOCK(&g_lpp_task_pool.shard_mu[s]);
    lpp_task_drive(raw_task);
    return 1;
}

int64_t lpp_executor_run(void *raw_task) {
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

int64_t lpp_async_worker_count(void) {
    lpp_task_pool_ensure();
    return (int64_t)g_lpp_task_pool.num_workers;
}

int64_t lpp_async_tasks_completed(void) {
    lpp_task_pool_ensure();
    LPP_POOL_LOCK(&g_lpp_task_pool.stat_mu);
    int64_t c = g_lpp_task_pool.tasks_completed;
    LPP_POOL_UNLOCK(&g_lpp_task_pool.stat_mu);
    return c;
}

int64_t lpp_async_tasks_stolen(void) {
    lpp_task_pool_ensure();
    LPP_POOL_LOCK(&g_lpp_task_pool.stat_mu);
    int64_t c = g_lpp_task_pool.tasks_stolen;
    LPP_POOL_UNLOCK(&g_lpp_task_pool.stat_mu);
    return c;
}

/* ── Allocator ───────────────────────────────────────────────────────────── */

void *lpp_alloc(int64_t size) {
    return calloc(1, (size_t)size);
}

void lpp_free(void *ptr, int64_t size) {
    (void)size;
    free(ptr);
}

