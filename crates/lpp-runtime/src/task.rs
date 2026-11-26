//! The production multi-threaded work-stealing async task runtime.
//!
//! Each `LppTask` preserves the 32-byte C ABI layout (`code`, `environment`,
//! `result`, atomic `state`, `result_managed`) while backing execution onto
//! a sharded work-stealing thread pool:
//! - 64 striped shards for wait/completion handle lookup and teardown,
//! - Cache-aligned per-worker LIFO run deques paired with a global injector
//!   and round-robin FIFO sibling work stealing,
//! - Cooperative task stealing inside `.await`/`poll` so arbitrary-depth
//!   fork-join trees cannot starve the worker pool,
//! - Worker ARC reference release before completion signaling so immediate
//!   caller destroys observe deterministic drop counts.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicUsize, Ordering, fence};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread;

use libc::c_void;

use crate::arc::{lpp_arc_alloc_with_destructor, lpp_arc_release, lpp_arc_retain};
use crate::panic::runtime_panic;

/// The task code entry point: `int64_t (*)(void *environment)`.
type TaskCode = unsafe extern "C" fn(*mut c_void) -> i64;

const HANDLE_SHARDS: usize = 64;
const INJECTOR_POLL_INTERVAL: usize = 63;

/// The task payload. `#[repr(C)]` + field order match the C `LppTask`
/// exactly (code@0, environment@8, result@16, state@24, result_managed@28;
/// size 32). `state`: 0 pending, 1 running, 2 complete.
#[repr(C)]
pub struct LppTask {
    code: usize,
    environment: *mut c_void,
    result: i64,
    state: AtomicI32,
    result_managed: i32,
}

type TaskQueueItem = (usize, Arc<TaskHandle>);

struct TaskHandle {
    worker_owns: AtomicBool,
    finished: AtomicBool,
    done_mu: Mutex<()>,
    done_cv: Condvar,
}

impl TaskHandle {
    fn new() -> Self {
        Self {
            worker_owns: AtomicBool::new(true),
            finished: AtomicBool::new(false),
            done_mu: Mutex::new(()),
            done_cv: Condvar::new(),
        }
    }

    #[inline]
    fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    fn mark_finished(&self) {
        let _guard = self.done_mu.lock().unwrap_or_else(|e| e.into_inner());
        self.finished.store(true, Ordering::Release);
        self.done_cv.notify_all();
    }

    fn wait_cooperatively(&self, pool: &'static TaskPool) {
        while !self.is_finished() {
            if try_steal_and_run_one(pool) {
                continue;
            }
            let guard = self.done_mu.lock().unwrap_or_else(|e| e.into_inner());
            if !self.is_finished() {
                let _guard = self
                    .done_cv
                    .wait_while(guard, |_| !self.is_finished())
                    .unwrap_or_else(|e| e.into_inner());
            }
        }
    }
}

#[repr(align(64))]
struct WorkerDeque {
    queue: Mutex<VecDeque<TaskQueueItem>>,
}

impl WorkerDeque {
    fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
        }
    }
}

type HandleShard = Mutex<HashMap<usize, Arc<TaskHandle>>>;

struct TaskPool {
    workers: Vec<WorkerDeque>,
    injector: Mutex<VecDeque<TaskQueueItem>>,
    shards: Box<[HandleShard]>,
    sleeping: AtomicUsize,
    park_mu: Mutex<()>,
    park_cv: Condvar,
}

impl TaskPool {
    #[inline]
    fn shard_for(&self, addr: usize) -> &HandleShard {
        let idx = ((addr >> 4) ^ (addr >> 13)) & (HANDLE_SHARDS - 1);
        &self.shards[idx]
    }
}

static TASK_POOL: OnceLock<TaskPool> = OnceLock::new();
static TASKS_COMPLETED: AtomicU64 = AtomicU64::new(0);
static TASKS_STOLEN: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static ACTIVE_TASKS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    static WORKER_ID: Cell<Option<usize>> = const { Cell::new(None) };
}

struct ActiveTaskGuard {
    addr: usize,
}

impl ActiveTaskGuard {
    fn enter(addr: usize) -> Self {
        ACTIVE_TASKS.with(|cell| {
            let mut active = cell.borrow_mut();
            if active.contains(&addr) {
                runtime_panic("concurrent or recursive polling of the same task");
            }
            active.push(addr);
        });
        Self { addr }
    }
}

impl Drop for ActiveTaskGuard {
    fn drop(&mut self) {
        ACTIVE_TASKS.with(|cell| {
            let mut active = cell.borrow_mut();
            if let Some(pos) = active.iter().rposition(|&x| x == self.addr) {
                active.remove(pos);
            }
        });
    }
}

fn configured_worker_count() -> usize {
    if let Ok(raw) = std::env::var("LPP_ASYNC_THREADS")
        && let Ok(parsed) = raw.trim().parse::<usize>()
    {
        return parsed.clamp(2, 64);
    }
    thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(2, 16)
}

fn task_pool() -> &'static TaskPool {
    TASK_POOL.get_or_init(|| {
        let count = configured_worker_count();
        let workers = (0..count).map(|_| WorkerDeque::new()).collect();
        let shards: Vec<_> = (0..HANDLE_SHARDS)
            .map(|_| Mutex::new(HashMap::new()))
            .collect();
        let pool = TaskPool {
            workers,
            injector: Mutex::new(VecDeque::new()),
            shards: shards.into_boxed_slice(),
            sleeping: AtomicUsize::new(0),
            park_mu: Mutex::new(()),
            park_cv: Condvar::new(),
        };
        for idx in 0..count {
            let _ = thread::Builder::new()
                .name(format!("lpp-async-worker-{idx}"))
                .spawn(move || worker_loop(idx));
        }
        pool
    })
}

fn wake_one_sleeper(pool: &TaskPool) {
    fence(Ordering::SeqCst);
    if pool.sleeping.load(Ordering::SeqCst) > 0 {
        let _guard = pool.park_mu.lock().unwrap_or_else(|e| e.into_inner());
        pool.park_cv.notify_one();
    }
}

fn enqueue_task(pool: &TaskPool, item: TaskQueueItem) {
    if let Some(wid) = WORKER_ID.get()
        && wid < pool.workers.len()
    {
        let mut q = pool.workers[wid]
            .queue
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        q.push_back(item);
        drop(q);
        wake_one_sleeper(pool);
        return;
    }
    let mut q = pool.injector.lock().unwrap_or_else(|e| e.into_inner());
    q.push_back(item);
    drop(q);
    wake_one_sleeper(pool);
}

fn steal_from_worker(pool: &TaskPool, victim: usize) -> Option<TaskQueueItem> {
    let Ok(mut q) = pool.workers[victim].queue.try_lock() else {
        return None;
    };
    let item = q.pop_front();
    if item.is_some() {
        TASKS_STOLEN.fetch_add(1, Ordering::Relaxed);
    }
    item
}

fn pop_next_task(pool: &TaskPool, worker_id: Option<usize>, ticks: usize) -> Option<TaskQueueItem> {
    if let Some(wid) = worker_id {
        if (ticks & INJECTOR_POLL_INTERVAL) == 0
            && let Some(item) = pool
                .injector
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pop_front()
        {
            return Some(item);
        }
        if let Some(item) = pool.workers[wid]
            .queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_back()
        {
            return Some(item);
        }
        if let Some(item) = pool
            .injector
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front()
        {
            return Some(item);
        }
        let n = pool.workers.len();
        for offset in 1..n {
            let victim = (wid + offset) % n;
            if let Some(item) = steal_from_worker(pool, victim) {
                return Some(item);
            }
        }
        return None;
    }

    if let Some(item) = pool
        .injector
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .pop_front()
    {
        return Some(item);
    }
    for victim in 0..pool.workers.len() {
        if let Some(item) = steal_from_worker(pool, victim) {
            return Some(item);
        }
    }
    None
}

fn has_any_pending_work(pool: &TaskPool) -> bool {
    if !pool
        .injector
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_empty()
    {
        return true;
    }
    pool.workers
        .iter()
        .any(|w| !w.queue.lock().unwrap_or_else(|e| e.into_inner()).is_empty())
}

fn try_steal_and_run_one(pool: &TaskPool) -> bool {
    let wid = WORKER_ID.get();
    let Some((raw_addr, handle)) = pop_next_task(pool, wid, 1) else {
        return false;
    };
    if handle.worker_owns.swap(false, Ordering::AcqRel) {
        unsafe { execute_claimed_task(raw_addr, &handle) };
        return true;
    }
    false
}

fn worker_loop(wid: usize) {
    WORKER_ID.set(Some(wid));
    let pool = task_pool();
    let mut ticks = 1usize;
    loop {
        if let Some((raw_addr, handle)) = pop_next_task(pool, Some(wid), ticks) {
            ticks = ticks.wrapping_add(1);
            if handle.worker_owns.swap(false, Ordering::AcqRel) {
                unsafe { execute_claimed_task(raw_addr, &handle) };
            }
            continue;
        }

        let guard = pool.park_mu.lock().unwrap_or_else(|e| e.into_inner());
        pool.sleeping.fetch_add(1, Ordering::SeqCst);
        fence(Ordering::SeqCst);
        if has_any_pending_work(pool) {
            pool.sleeping.fetch_sub(1, Ordering::SeqCst);
            drop(guard);
            continue;
        }
        let _woken = pool.park_cv.wait(guard).unwrap_or_else(|e| e.into_inner());
        pool.sleeping.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Drive `raw_task` to completion once (`state: 0 -> 1 -> 2`).
unsafe fn drive_task(raw_task: *mut c_void) {
    let task = unsafe { &mut *(raw_task as *mut LppTask) };
    if task.state.load(Ordering::Acquire) == 2 {
        return;
    }
    match task
        .state
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
    {
        Ok(_) => {}
        Err(2) => return,
        Err(_) => runtime_panic("concurrent or recursive polling of the same task"),
    }
    let _active_guard = ActiveTaskGuard::enter(raw_task as usize);
    let code: TaskCode = unsafe { std::mem::transmute(task.code) };
    let env = task.environment;
    let result = unsafe { code(env) };
    task.result = result;
    task.state.store(2, Ordering::Release);
    TASKS_COMPLETED.fetch_add(1, Ordering::Relaxed);
}

/// Run a task whose worker ownership token was just claimed, release the
/// pool's ARC reference, and wake any thread awaiting `handle`.
unsafe fn execute_claimed_task(raw_addr: usize, handle: &TaskHandle) {
    let raw_task = raw_addr as *mut c_void;
    unsafe { drive_task(raw_task) };
    // Drop the pool's ARC reference *before* marking finished so an immediate
    // caller-side `lpp_task_destroy` sees refcount reach zero synchronously.
    unsafe { lpp_arc_release(raw_task) };
    handle.mark_finished();
}

/// The ARC destructor: release the owned environment, and — once complete —
/// the owned managed result. Mirrors C `lpp_task_payload_destroy`.
unsafe extern "C" fn task_payload_destroy(payload: *mut c_void) {
    if payload.is_null() {
        return;
    }
    if let Some(pool) = TASK_POOL.get() {
        let mut shard = pool
            .shard_for(payload as usize)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        shard.remove(&(payload as usize));
    }
    let task = unsafe { &mut *(payload as *mut LppTask) };
    if !task.environment.is_null() {
        unsafe { lpp_arc_release(task.environment) };
        task.environment = std::ptr::null_mut();
    }
    if task.state.load(Ordering::Acquire) == 2 && task.result_managed != 0 && task.result != 0 {
        unsafe { lpp_arc_release(task.result as usize as *mut c_void) };
        task.result = 0;
    }
}

/// Trace for a task: the owned environment, plus the owned managed
/// result once complete (mirrors `task_payload_destroy`).
///
/// # Safety
///
/// Caller must uphold the L++ task runtime ABI; any non-null task, closure code, environment, or trace context pointer must be valid for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_task_trace(payload: *mut c_void, ctx: *mut c_void) {
    if payload.is_null() {
        return;
    }
    let task = unsafe { &*(payload as *mut LppTask) };
    unsafe { crate::collect::lpp_trace_visit(ctx, task.environment) };
    if task.state.load(Ordering::Acquire) == 2 && task.result_managed != 0 && task.result != 0 {
        unsafe {
            crate::collect::lpp_trace_visit(ctx, task.result as usize as *mut c_void);
        }
    }
}

/// Create a task and submit it to the multi-threaded work-stealing pool.
/// `code_ptr` and `environment` are both required; the environment reference
/// is transferred (owned by the task).
///
/// # Safety
///
/// Caller must uphold the L++ task runtime ABI; any non-null task, closure code, environment, or trace context pointer must be valid for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_task_new(
    code_ptr: *mut c_void,
    environment: *mut c_void,
    result_managed: i64,
) -> *mut c_void {
    if code_ptr.is_null() || environment.is_null() {
        runtime_panic("task creation requires code and environment");
    }
    let size = std::mem::size_of::<LppTask>() as i64;
    let raw = unsafe { lpp_arc_alloc_with_destructor(size, Some(task_payload_destroy)) };
    if raw.is_null() {
        runtime_panic("out of memory while allocating task");
    }
    let task = unsafe { &mut *(raw as *mut LppTask) };
    task.code = code_ptr as usize;
    task.environment = environment;
    task.result = 0;
    task.state.store(0, Ordering::Relaxed);
    task.result_managed = i32::from(result_managed != 0);

    let pool = task_pool();
    let handle = Arc::new(TaskHandle::new());
    {
        let mut shard = pool
            .shard_for(raw as usize)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        shard.insert(raw as usize, Arc::clone(&handle));
    }
    // Retain one ARC reference for the queued/worker execution slot.
    unsafe { lpp_arc_retain(raw) };
    enqueue_task(pool, (raw as usize, handle));
    raw
}

/// Poll the task to completion on the multi-threaded work-stealing executor
/// (stealing it inline if still queued, or cooperatively executing peer
/// work while waiting for the assigned worker thread to finish).
///
/// # Safety
///
/// Caller must uphold the L++ task runtime ABI; any non-null task, closure code, environment, or trace context pointer must be valid for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_task_poll(raw_task: *mut c_void) -> i64 {
    if raw_task.is_null() {
        runtime_panic("attempted to poll a null task");
    }
    let addr = raw_task as usize;
    ACTIVE_TASKS.with(|cell| {
        if cell.borrow().contains(&addr) {
            runtime_panic("concurrent or recursive polling of the same task");
        }
    });

    let pool = task_pool();
    let handle = pool
        .shard_for(addr)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&addr)
        .cloned();
    if let Some(handle) = handle {
        if handle.worker_owns.swap(false, Ordering::AcqRel) {
            unsafe { execute_claimed_task(addr, &handle) };
        } else {
            handle.wait_cooperatively(pool);
        }
    } else {
        unsafe { drive_task(raw_task) };
    }
    1
}

/// Await the task: ensure it completes on the multi-threaded executor, then
/// return its result. Each await of a managed result creates a fresh
/// caller-owned reference (the task keeps its own).
///
/// # Safety
///
/// Caller must uphold the L++ task runtime ABI; any non-null task, closure code, environment, or trace context pointer must be valid for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_task_await(raw_task: *mut c_void) -> i64 {
    unsafe { lpp_task_poll(raw_task) };
    let task = unsafe { &*(raw_task as *mut LppTask) };
    let result = task.result;
    if task.result_managed != 0 && result != 0 {
        unsafe { lpp_arc_retain(result as usize as *mut c_void) };
    }
    result
}

/// Return the number of OS worker threads backing the async task pool.
///
/// # Safety
///
/// Caller must uphold the L++ task runtime ABI; any non-null task, closure code, environment, or trace context pointer must be valid for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_async_worker_count() -> i64 {
    task_pool().workers.len() as i64
}

/// Return the cumulative number of async tasks completed across all threads.
///
/// # Safety
///
/// Caller must uphold the L++ task runtime ABI; any non-null task, closure code, environment, or trace context pointer must be valid for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_async_tasks_completed() -> i64 {
    TASKS_COMPLETED.load(Ordering::Relaxed) as i64
}

/// Return the cumulative number of tasks stolen across worker deques.
///
/// # Safety
///
/// Caller must uphold the L++ task runtime ABI; any non-null task, closure code, environment, or trace context pointer must be valid for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_async_tasks_stolen() -> i64 {
    TASKS_STOLEN.load(Ordering::Relaxed) as i64
}

/// Destroy the task: a single ARC reference release (the destructor handles
/// the environment and managed result).
///
/// # Safety
///
/// Caller must uphold the L++ task runtime ABI; any non-null task, closure code, environment, or trace context pointer must be valid for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_task_destroy(raw_task: *mut c_void) {
    unsafe { lpp_arc_release(raw_task) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arc::lpp_arc_alloc_with_destructor;
    use crate::numeric::lpp_vec_i64_checksum;
    use crate::tuple::lpp_tuple_alloc;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn task_layout_matches_c() {
        assert_eq!(std::mem::size_of::<LppTask>(), 32, "LppTask size");
        assert_eq!(std::mem::align_of::<LppTask>(), 8, "LppTask align");
    }

    static DROPS: AtomicUsize = AtomicUsize::new(0);
    static SERIAL: Mutex<()> = Mutex::new(());

    unsafe extern "C" fn dtor(_p: *mut c_void) {
        DROPS.fetch_add(1, Ordering::Relaxed);
    }
    unsafe extern "C" fn code_int(_env: *mut c_void) -> i64 {
        42
    }
    unsafe extern "C" fn code_managed(_env: *mut c_void) -> i64 {
        let o = unsafe { lpp_arc_alloc_with_destructor(8, Some(dtor)) };
        o as i64
    }

    fn mix(h: &mut i64, v: i64) {
        *h = (*h ^ v).wrapping_mul(1099511628211);
    }

    /// The golden fingerprint of the managed scenario (tasks + tuple + vec
    /// checksum), produced by the v1 C reference. The C side is pinned in
    /// `tests/runtime_gate.rs::c_reference_matches_the_managed_golden`.
    const MANAGED_GOLDEN: i64 = -1109191003795678529;

    fn scenario() -> i64 {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let mut h: i64 = 1469598103934665603;
        DROPS.store(0, Ordering::Relaxed);
        unsafe {
            // Task A: plain int result.
            let env_a = lpp_arc_alloc_with_destructor(8, Some(dtor));
            let t_a = lpp_task_new(code_int as TaskCode as *mut c_void, env_a, 0);
            mix(&mut h, lpp_task_poll(t_a));
            mix(&mut h, lpp_task_await(t_a));
            mix(&mut h, lpp_task_poll(t_a));
            lpp_task_destroy(t_a);
            mix(&mut h, DROPS.load(Ordering::Relaxed) as i64);

            // Task B: managed result (await retains for the caller).
            let env_b = lpp_arc_alloc_with_destructor(8, Some(dtor));
            let t_b = lpp_task_new(code_managed as TaskCode as *mut c_void, env_b, 1);
            let r = lpp_task_await(t_b);
            mix(&mut h, i64::from(r != 0));
            lpp_arc_release(r as usize as *mut c_void);
            lpp_task_destroy(t_b);
            mix(&mut h, DROPS.load(Ordering::Relaxed) as i64);

            // Tuple with one managed child slot at byte offset 16.
            let child = lpp_arc_alloc_with_destructor(8, Some(dtor));
            let tup = lpp_tuple_alloc(32, 1, 16);
            *(tup.cast::<u8>().add(16) as *mut *mut c_void) = child;
            lpp_arc_release(tup);
            mix(&mut h, DROPS.load(Ordering::Relaxed) as i64);

            // Vec checksum.
            mix(&mut h, lpp_vec_i64_checksum(0));
            mix(&mut h, lpp_vec_i64_checksum(1));
            mix(&mut h, lpp_vec_i64_checksum(10));
            mix(&mut h, lpp_vec_i64_checksum(100));
            mix(&mut h, lpp_vec_i64_checksum(-5));
            mix(&mut h, lpp_vec_i64_checksum(1000));
        }
        h
    }

    #[test]
    fn managed_scenario_is_golden() {
        assert_eq!(
            scenario(),
            MANAGED_GOLDEN,
            "Rust task/tuple/vec runtime diverges from the C reference"
        );
    }

    #[repr(C)]
    struct RendezvousEnv {
        arrived: std::sync::atomic::AtomicUsize,
        released: std::sync::atomic::AtomicBool,
    }

    unsafe extern "C" fn rendezvous_code(raw_env: *mut c_void) -> i64 {
        let env = unsafe { &*(raw_env as *const *const RendezvousEnv) };
        let shared = unsafe { &**env };
        let nth = shared.arrived.fetch_add(1, Ordering::AcqRel) + 1;
        if nth >= 2 {
            shared.released.store(true, Ordering::Release);
            return nth as i64;
        }
        // Wait for the peer task on another worker thread; bounded timeout
        // ensures a regression to single-threaded execution fails quickly
        // instead of hanging CI.
        for _ in 0..500 {
            if shared.released.load(Ordering::Acquire) {
                return nth as i64;
            }
            thread::sleep(std::time::Duration::from_millis(2));
        }
        -1
    }

    unsafe extern "C" fn child_leaf_code(_raw_env: *mut c_void) -> i64 {
        7
    }

    unsafe extern "C" fn parent_nested_code(_raw_env: *mut c_void) -> i64 {
        let mut children = Vec::with_capacity(4);
        for _ in 0..4 {
            let child_env = unsafe { lpp_arc_alloc_with_destructor(8, None) };
            let child =
                unsafe { lpp_task_new(child_leaf_code as TaskCode as *mut c_void, child_env, 0) };
            children.push(child);
        }
        let mut sum = 0i64;
        for child in children {
            sum += unsafe { lpp_task_await(child) };
            unsafe { lpp_task_destroy(child) };
        }
        sum
    }

    #[test]
    fn parallel_tasks_run_concurrently_on_worker_pool() {
        let state = RendezvousEnv {
            arrived: std::sync::atomic::AtomicUsize::new(0),
            released: std::sync::atomic::AtomicBool::new(false),
        };
        let state_ptr: *const RendezvousEnv = &state;
        unsafe {
            let env1 = lpp_arc_alloc_with_destructor(8, None);
            *(env1 as *mut *const RendezvousEnv) = state_ptr;
            let env2 = lpp_arc_alloc_with_destructor(8, None);
            *(env2 as *mut *const RendezvousEnv) = state_ptr;

            let t1 = lpp_task_new(rendezvous_code as TaskCode as *mut c_void, env1, 0);
            let t2 = lpp_task_new(rendezvous_code as TaskCode as *mut c_void, env2, 0);

            let r1 = lpp_task_await(t1);
            let r2 = lpp_task_await(t2);
            lpp_task_destroy(t1);
            lpp_task_destroy(t2);

            assert!(
                r1 > 0 && r2 > 0,
                "both tasks must rendezvous concurrently across worker threads (r1={r1}, r2={r2})"
            );
            assert_eq!(r1 + r2, 3);
        }
    }

    #[test]
    fn nested_awaits_complete_without_starving_worker_pool() {
        unsafe {
            let mut parents = Vec::with_capacity(16);
            for _ in 0..16 {
                let env = lpp_arc_alloc_with_destructor(8, None);
                parents.push(lpp_task_new(
                    parent_nested_code as TaskCode as *mut c_void,
                    env,
                    0,
                ));
            }
            let mut total = 0i64;
            for p in parents {
                total += lpp_task_await(p);
                lpp_task_destroy(p);
            }
            assert_eq!(total, 16 * 28);
        }
    }

    #[test]
    fn dropped_unawaited_task_completes_and_releases_environment() {
        static DROPPED_ENV: AtomicUsize = AtomicUsize::new(0);
        unsafe extern "C" fn drop_env(_p: *mut c_void) {
            DROPPED_ENV.fetch_add(1, Ordering::AcqRel);
        }
        unsafe extern "C" fn noop_code(_env: *mut c_void) -> i64 {
            11
        }
        DROPPED_ENV.store(0, Ordering::Release);
        unsafe {
            let env = lpp_arc_alloc_with_destructor(8, Some(drop_env));
            let t = lpp_task_new(noop_code as TaskCode as *mut c_void, env, 0);
            lpp_task_destroy(t);
        }
        for _ in 0..500 {
            if DROPPED_ENV.load(Ordering::Acquire) == 1 {
                return;
            }
            thread::sleep(std::time::Duration::from_millis(2));
        }
        panic!("worker pool did not finish and release unawaited task environment in time");
    }

    unsafe extern "C" fn fib_task_code(raw_env: *mut c_void) -> i64 {
        let n = unsafe { *(raw_env as *const i64) };
        if n <= 1 {
            return n;
        }
        unsafe {
            let env_a = lpp_arc_alloc_with_destructor(8, None);
            *(env_a as *mut i64) = n - 1;
            let env_b = lpp_arc_alloc_with_destructor(8, None);
            *(env_b as *mut i64) = n - 2;
            let t_a = lpp_task_new(fib_task_code as TaskCode as *mut c_void, env_a, 0);
            let t_b = lpp_task_new(fib_task_code as TaskCode as *mut c_void, env_b, 0);
            let res = lpp_task_await(t_a) + lpp_task_await(t_b);
            lpp_task_destroy(t_a);
            lpp_task_destroy(t_b);
            res
        }
    }

    #[test]
    fn deep_fork_join_fibonacci_does_not_starve_or_deadlock() {
        let before = unsafe { lpp_async_tasks_completed() };
        let res = unsafe {
            let env = lpp_arc_alloc_with_destructor(8, None);
            *(env as *mut i64) = 15;
            let t = lpp_task_new(fib_task_code as TaskCode as *mut c_void, env, 0);
            let out = lpp_task_await(t);
            lpp_task_destroy(t);
            out
        };
        assert_eq!(res, 610);
        let after = unsafe { lpp_async_tasks_completed() };
        assert!(
            after - before >= 1973,
            "expected at least 1973 fib(15) tasks executed, got {}",
            after - before
        );
        assert!(unsafe { lpp_async_worker_count() } >= 2);
    }

    #[test]
    fn high_fanout_1000_managed_tasks_with_work_stealing_zero_leaks() {
        static ENV_DROPS: AtomicUsize = AtomicUsize::new(0);
        static RES_DROPS: AtomicUsize = AtomicUsize::new(0);
        unsafe extern "C" fn env_drop(_p: *mut c_void) {
            ENV_DROPS.fetch_add(1, Ordering::AcqRel);
        }
        unsafe extern "C" fn res_drop(_p: *mut c_void) {
            RES_DROPS.fetch_add(1, Ordering::AcqRel);
        }
        unsafe extern "C" fn make_managed_pair(raw_env: *mut c_void) -> i64 {
            let idx = unsafe { *(raw_env as *const i64) };
            let out = unsafe { lpp_arc_alloc_with_destructor(8, Some(res_drop)) };
            unsafe { *(out as *mut i64) = idx * 3 };
            out as usize as i64
        }

        ENV_DROPS.store(0, Ordering::Release);
        RES_DROPS.store(0, Ordering::Release);
        const TOTAL: usize = 1000;
        let mut awaited_handles = Vec::with_capacity(800);
        unsafe {
            for i in 0..TOTAL {
                let env = lpp_arc_alloc_with_destructor(8, Some(env_drop));
                *(env as *mut i64) = i as i64;
                let t = lpp_task_new(make_managed_pair as TaskCode as *mut c_void, env, 1);
                if i % 5 == 0 {
                    // Drop unawaited every 5th task immediately; worker must still execute + free.
                    lpp_task_destroy(t);
                } else {
                    awaited_handles.push((i as i64, t));
                }
            }
            for (i, t) in awaited_handles.into_iter().rev() {
                let res_ptr = lpp_task_await(t) as usize as *mut c_void;
                assert!(!res_ptr.is_null());
                assert_eq!(*(res_ptr as *const i64), i * 3);
                lpp_arc_release(res_ptr);
                lpp_task_destroy(t);
            }
        }
        for _ in 0..1000 {
            if ENV_DROPS.load(Ordering::Acquire) == TOTAL
                && RES_DROPS.load(Ordering::Acquire) == TOTAL
            {
                return;
            }
            thread::sleep(std::time::Duration::from_millis(2));
        }
        panic!(
            "leaked async objects: env_drops={} res_drops={} (expected {TOTAL})",
            ENV_DROPS.load(Ordering::Acquire),
            RES_DROPS.load(Ordering::Acquire)
        );
    }

    #[test]
    fn multi_producer_concurrent_task_submission_from_host_threads() {
        unsafe extern "C" fn square_task(raw_env: *mut c_void) -> i64 {
            let v = unsafe { *(raw_env as *const i64) };
            v * v
        }
        let mut threads = Vec::new();
        for t_id in 0..8i64 {
            threads.push(thread::spawn(move || {
                let mut sum = 0i64;
                let mut pending = Vec::with_capacity(64);
                unsafe {
                    for k in 0..64i64 {
                        let env = lpp_arc_alloc_with_destructor(8, None);
                        *(env as *mut i64) = t_id * 64 + k;
                        pending.push(lpp_task_new(square_task as TaskCode as *mut c_void, env, 0));
                    }
                    for task in pending {
                        sum += lpp_task_await(task);
                        lpp_task_destroy(task);
                    }
                }
                sum
            }));
        }
        let actual: i64 = threads.into_iter().map(|h| h.join().unwrap()).sum();
        let expected: i64 = (0..512i64).map(|x| x * x).sum();
        assert_eq!(actual, expected);
        assert!(unsafe { lpp_async_worker_count() } >= 2);
        let _stolen = unsafe { lpp_async_tasks_stolen() };
    }
}
