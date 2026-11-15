//! Concurrency ownership planning and synchronization balance tests
//! (CONCURRENCY.md §4 Steps 1 & 2).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lpp_common::OptimizationLevel;
use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    lower_package,
};
use lpp_mir::{
    MirBuildOptions, MirFunctionId, MirFunctionKind, MirLocalKind, MirProgram, build_mir,
    verify_mir,
};
use lpp_ownership::{
    ContainmentNode, OwnershipBalanceErrorKind, OwnershipBalancePass, OwnershipPlan,
    OwnershipPlanErrorKind, OwnershipPlanPass, TypeStrategy, ValuePlacement,
    compute_ownership_plan, ownership_plan_snapshot, verify_ownership_balance,
    verify_ownership_plan,
};
use lpp_passes::{PassManager, run_optimization};
use lpp_types::{ShadowInferenceOptions, TypeInterner, TypeKind, infer_hir_package};

#[derive(Debug)]
struct MemoryFileSystem {
    files: BTreeMap<PathBuf, String>,
}

impl MemoryFileSystem {
    fn new(source: &str) -> Self {
        Self {
            files: BTreeMap::from([(PathBuf::from("/ownership/main.lpp"), source.to_owned())]),
        }
    }
}

impl FileSystem for MemoryFileSystem {
    fn is_file(&self, path: &Path) -> Result<bool, FileSystemError> {
        Ok(self.files.contains_key(path))
    }

    fn canonicalize(&self, path: &Path) -> Result<PathBuf, FileSystemError> {
        if self.files.contains_key(path) || self.files.keys().any(|file| file.starts_with(path)) {
            Ok(path.to_owned())
        } else {
            Err(FileSystemError::new("canonicalize", path, "path not found"))
        }
    }

    fn read_to_string(&self, path: &Path) -> Result<String, FileSystemError> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| FileSystemError::new("read", path, "path not found"))
    }
}

fn executable(source: &str) -> (MirProgram, TypeInterner) {
    let filesystem = MemoryFileSystem::new(source);
    let graph = GraphBuilder::new(&filesystem)
        .build(GraphRequest::new(
            "/ownership/main.lpp",
            PackageSpec::new("ownership", "/ownership"),
        ))
        .unwrap();
    let package = lower_package(&graph, ResolutionMode::Namespaced).unwrap();
    let mut types = infer_hir_package(&package, ShadowInferenceOptions::default())
        .unwrap_or_else(|error| panic!("type stage: {error:?}"));
    let program = build_mir(
        &package,
        &graph.sources,
        &mut types,
        MirBuildOptions::default(),
    )
    .unwrap_or_else(|error| panic!("build: {error:?}"));
    assert!(verify_mir(&program, &types.interner).is_empty());
    (program, types.interner)
}

fn function_by_index(program: &MirProgram, index: usize) -> MirFunctionId {
    program
        .functions()
        .nth(index)
        .map(|(id, _)| id)
        .expect("function index in bounds")
}

fn main_function(program: &MirProgram) -> MirFunctionId {
    program
        .functions()
        .filter(|(_, function)| function.kind != MirFunctionKind::Closure)
        .map(|(id, _)| id)
        .last()
        .expect("program defines main")
}

// ── Step 1: Concurrent vs Cycle-Shared separation in OwnershipPlan ─────────

#[test]
fn step1_acyclic_spawned_struct_is_concurrent_not_cycle_shared() {
    let source = r#"
struct Counter:
    value: Int

def main():
    c := Counter(0)
    spawn fn():
        print(c.value)
"#;
    let (program, types) = executable(source);
    let plan: OwnershipPlan = compute_ownership_plan(&program, &types).unwrap();
    assert!(verify_ownership_plan(&program, &types, &plan).is_empty());

    // Acyclic Counter must NOT become cycle-Shared or enter pinned_types().
    assert!(plan.pinned_types().is_empty());
    assert_eq!(plan.stats.cycle_count, 0);
    assert_eq!(plan.stats.shared_node_count, 0);
    assert_eq!(plan.stats.shared_cell_count, 0);

    // Concurrent tracking records Counter and the spawned closure
    // without touching TypeStrategy::Shared.
    assert!(plan.stats.concurrent_node_count > 0);
    assert!(plan.stats.concurrent_cell_count > 0);
    assert!(!plan.concurrent_types().is_empty());

    let main_id = main_function(&program);
    let counter_cells: Vec<_> = plan
        .cells
        .iter()
        .filter(|cell| {
            cell.function == main_id
                && program.local(cell.local).is_some_and(|l| {
                    l.kind != MirLocalKind::Temporary
                        && matches!(types.kind(l.ty), TypeKind::Nominal { .. })
                })
        })
        .collect();
    assert_eq!(counter_cells.len(), 1);
    assert_eq!(counter_cells[0].placement, ValuePlacement::Owned);
    assert!(counter_cells[0].concurrent);
    assert!(plan.is_cell_concurrent(counter_cells[0].function, counter_cells[0].local));
}

#[test]
fn step1_cycle_shared_local_only_is_not_concurrent() {
    let source = r#"
struct Node:
    value: Int
    next: List[Node]

def main():
    a := Node(1, [])
    list_push(a.next, a)
    print(a.value)
"#;
    let (program, types) = executable(source);
    let plan = compute_ownership_plan(&program, &types).unwrap();
    assert!(verify_ownership_plan(&program, &types, &plan).is_empty());

    // Cyclic Node is cycle-Shared (`pinned_types()` non-empty), but never reaches spawn.
    assert!(!plan.pinned_types().is_empty());
    assert!(plan.stats.shared_node_count > 0);
    assert_eq!(plan.stats.concurrent_node_count, 0);
    assert_eq!(plan.stats.concurrent_cell_count, 0);
    assert!(plan.concurrent_types().is_empty());
    assert!(plan.concurrent_cells().is_empty());
}

#[test]
fn step1_unspawned_closure_of_same_type_is_not_marked_concurrent() {
    let source = r#"
struct Box:
    x: Int

def main():
    local_box := Box(1)
    shared_box := Box(2)
    f_local := fn():
        print(local_box.x)
    f_spawn := fn():
        print(shared_box.x)
    f_local()
    spawn(f_spawn)
"#;
    let (program, types) = executable(source);
    let plan = compute_ownership_plan(&program, &types).unwrap();
    assert!(verify_ownership_plan(&program, &types, &plan).is_empty());

    let callable_nodes: Vec<_> = plan
        .nodes
        .iter()
        .filter(|node| matches!(node.node, ContainmentNode::Callable(_)))
        .collect();
    assert_eq!(callable_nodes.len(), 2);
    // Exactly one closure (`f_spawn`) is concurrent; `f_local` stays non-concurrent.
    assert_eq!(
        callable_nodes.iter().filter(|n| n.concurrent).count(),
        1,
        "only the spawned callable should be marked concurrent"
    );
    assert!(
        callable_nodes
            .iter()
            .all(|n| n.strategy == TypeStrategy::Owned)
    );
}

#[test]
fn step1_nested_type_node_concurrency_closes_downward_and_snapshot_observes_it() {
    let source = r#"
struct Inner:
    v: Int

struct Outer:
    inner: Inner

def main():
    o := Outer(Inner(5))
    spawn fn():
        print(o.inner.v)
"#;
    let (program, types) = executable(source);
    let plan = compute_ownership_plan(&program, &types).unwrap();
    assert!(verify_ownership_plan(&program, &types, &plan).is_empty());

    // Both Outer and Inner aggregate nodes must be concurrent via downward closure.
    let aggregate_nodes: Vec<_> = plan
        .nodes
        .iter()
        .filter(|node| matches!(node.node, ContainmentNode::Aggregate(_)))
        .collect();
    assert_eq!(aggregate_nodes.len(), 2);
    assert!(aggregate_nodes.iter().all(|n| n.concurrent));

    let snap = ownership_plan_snapshot(&plan);
    assert!(snap.contains("concurrent-cells="));
    assert!(snap.contains("concurrent-nodes="));
    assert!(snap.contains(" concurrent"));
}

#[test]
fn step1_tampered_concurrent_flags_fail_plan_verification() {
    let source = r#"
struct Box:
    x: Int

def main():
    b := Box(1)
    spawn fn():
        print(b.x)
"#;
    let (program, types) = executable(source);
    let baseline = compute_ownership_plan(&program, &types).unwrap();

    // Tamper a concurrent cell's flag -> InvalidCellConcurrency + DeterminismMismatch.
    let mut bad_cell_plan = baseline.clone();
    let conc_cell = bad_cell_plan
        .cells
        .iter_mut()
        .find(|c| c.concurrent)
        .expect("plan has a concurrent cell");
    conc_cell.concurrent = false;
    let cell_errors = verify_ownership_plan(&program, &types, &bad_cell_plan);
    assert!(cell_errors.iter().any(|e| matches!(
        e.kind,
        OwnershipPlanErrorKind::InvalidCellConcurrency { .. }
    )));
    assert!(
        cell_errors
            .iter()
            .any(|e| matches!(e.kind, OwnershipPlanErrorKind::DeterminismMismatch))
    );

    // Tamper a concurrent node's flag -> InvalidNodeConcurrency + DeterminismMismatch.
    let mut bad_node_plan = baseline.clone();
    let conc_node = bad_node_plan
        .nodes
        .iter_mut()
        .find(|n| n.concurrent)
        .expect("plan has a concurrent node");
    conc_node.concurrent = false;
    let node_errors = verify_ownership_plan(&program, &types, &bad_node_plan);
    assert!(node_errors.iter().any(|e| matches!(
        e.kind,
        OwnershipPlanErrorKind::InvalidNodeConcurrency { .. }
    )));
    assert!(
        node_errors
            .iter()
            .any(|e| matches!(e.kind, OwnershipPlanErrorKind::DeterminismMismatch))
    );

    let _ = function_by_index(&program, 0);
}

// ── Step 2: Unsynchronized mutable write rejection (E4403) ─────────────────

#[test]
fn step2_positive_unique_move_into_spawn_allows_mutation() {
    let source = r#"
struct Box:
    x: Int

def main():
    mut b := Box(1)
    b.x = 2
    spawn fn():
        b.x = 3
        print(b.x)
"#;
    let (program, types) = executable(source);
    let plan = compute_ownership_plan(&program, &types).unwrap();
    let errors = verify_ownership_balance(&program, &types, &plan);
    assert!(
        errors.is_empty(),
        "unique transfer to spawn should pass balance: {errors:?}"
    );
}

#[test]
fn step2_positive_mutate_before_spawn_with_read_only_across_threads_is_safe() {
    // Mutates `b.x` strictly before `spawn`; after `spawn`, main only reads `b.x`.
    let source = r#"
struct Box:
    x: Int

def main():
    mut b := Box(1)
    b.x = 10
    spawn fn():
        print(b.x)
    print(b.x)
"#;
    let (program, types) = executable(source);
    let plan = compute_ownership_plan(&program, &types).unwrap();
    let errors = verify_ownership_balance(&program, &types, &plan);
    assert!(
        errors.is_empty(),
        "mutate-before-spawn + read-only sharing should pass: {errors:?}"
    );
}

#[test]
fn step2_positive_mutex_lock_unlock_synchronizes_shared_mutation() {
    let source = r#"
struct Box:
    x: Int

def main():
    m := mutex_new()
    mut b := Box(0)
    spawn fn():
        mutex_lock(m)
        b.x = 10
        mutex_unlock(m)
    mutex_lock(m)
    b.x = 20
    mutex_unlock(m)
"#;
    let (program, types) = executable(source);
    let plan = compute_ownership_plan(&program, &types).unwrap();
    let errors = verify_ownership_balance(&program, &types, &plan);
    assert!(
        errors.is_empty(),
        "mutex-guarded shared mutation should pass: {errors:?}"
    );
}

#[test]
fn step2_positive_rwlock_wrlock_synchronizes_shared_mutation() {
    let source = r#"
struct Box:
    x: Int

def main():
    rw := rwlock_new()
    mut b := Box(0)
    spawn fn():
        rwlock_wrlock(rw)
        b.x = 1
        rwlock_wrunlock(rw)
    rwlock_wrlock(rw)
    b.x = 2
    rwlock_wrunlock(rw)
"#;
    let (program, types) = executable(source);
    let plan = compute_ownership_plan(&program, &types).unwrap();
    let errors = verify_ownership_balance(&program, &types, &plan);
    assert!(
        errors.is_empty(),
        "rwlock_wrlock-guarded shared mutation should pass: {errors:?}"
    );
}

#[test]
fn step2_negative_unsynchronized_field_write_in_spawn_and_main_rejected() {
    let source = r#"
struct Box:
    x: Int

def main():
    mut b := Box(0)
    spawn fn():
        b.x = 1
    b.x = 2
"#;
    let (program, types) = executable(source);
    let plan = compute_ownership_plan(&program, &types).unwrap();
    let errors = verify_ownership_balance(&program, &types, &plan);
    let unsync_count = errors
        .iter()
        .filter(|e| {
            matches!(
                e.kind,
                OwnershipBalanceErrorKind::UnsynchronizedConcurrentWrite { .. }
            )
        })
        .count();
    assert!(
        unsync_count >= 2,
        "expected unsynchronized write errors in both closure and main, got: {errors:?}"
    );
    assert!(errors.iter().all(|e| e.code() == "E4403"));
}

#[test]
fn step2_negative_rwlock_rdlock_does_not_authorize_mutable_write() {
    let source = r#"
struct Box:
    x: Int

def main():
    rw := rwlock_new()
    mut b := Box(0)
    spawn fn():
        rwlock_rdlock(rw)
        b.x = 1
        rwlock_rdunlock(rw)
    print(b.x)
"#;
    let (program, types) = executable(source);
    let plan = compute_ownership_plan(&program, &types).unwrap();
    let errors = verify_ownership_balance(&program, &types, &plan);
    assert!(
        errors.iter().any(|e| matches!(
            e.kind,
            OwnershipBalanceErrorKind::UnsynchronizedConcurrentWrite { .. }
        )),
        "rwlock_rdlock must not permit concurrent mutable write: {errors:?}"
    );
}

#[test]
fn step2_negative_live_alias_prevents_unique_transfer() {
    let source = r#"
struct Box:
    x: Int

def main():
    mut b := Box(1)
    b2 := b
    spawn fn():
        print(b2.x)
    b.x = 99
"#;
    let (program, types) = executable(source);
    let plan = compute_ownership_plan(&program, &types).unwrap();
    let errors = verify_ownership_balance(&program, &types, &plan);
    assert!(
        errors.iter().any(|e| matches!(
            e.kind,
            OwnershipBalanceErrorKind::UnsynchronizedConcurrentWrite { .. }
        )),
        "live alias `b2` captured in spawn must prevent unique write on `b`: {errors:?}"
    );
}

#[test]
fn step2_positive_partial_multi_capture_unique_vs_negative_shared() {
    // `b1` is uniquely moved into spawn (allowed to mutate), while `b2` stays
    // live on main and is mutated inside spawn without a lock (rejected).
    let ok_source = r#"
struct Box:
    x: Int

def main():
    mut b1 := Box(1)
    b2 := Box(2)
    spawn fn():
        b1.x = 10
        print(b2.x)
    print(b2.x)
"#;
    let (ok_prog, ok_types) = executable(ok_source);
    let ok_plan = compute_ownership_plan(&ok_prog, &ok_types).unwrap();
    let ok_errors = verify_ownership_balance(&ok_prog, &ok_types, &ok_plan);
    assert!(
        ok_errors.is_empty(),
        "b1 should transfer uniquely even though sibling capture b2 stays live: {ok_errors:?}"
    );

    let bad_source = r#"
struct Box:
    x: Int

def main():
    mut b1 := Box(1)
    mut b2 := Box(2)
    spawn fn():
        b1.x = 10
        b2.x = 20
    print(b2.x)
"#;
    let (bad_prog, bad_types) = executable(bad_source);
    let bad_plan = compute_ownership_plan(&bad_prog, &bad_types).unwrap();
    let bad_errors = verify_ownership_balance(&bad_prog, &bad_types, &bad_plan);
    assert!(bad_errors.iter().any(|e| matches!(
        e.kind,
        OwnershipBalanceErrorKind::UnsynchronizedConcurrentWrite { .. }
    )));
}

#[test]
fn step2_negative_loop_spawn_is_not_unique() {
    let source = r#"
struct Box:
    x: Int

def main():
    mut b := Box(0)
    mut i := 0
    while i < 2:
        spawn fn():
            b.x = i
        i = i + 1
"#;
    let (program, types) = executable(source);
    let plan = compute_ownership_plan(&program, &types).unwrap();
    let errors = verify_ownership_balance(&program, &types, &plan);
    assert!(
        errors.iter().any(|e| matches!(
            e.kind,
            OwnershipBalanceErrorKind::UnsynchronizedConcurrentWrite { .. }
        )),
        "spawning a closure referencing `b` inside a loop cannot be unique: {errors:?}"
    );
}

#[test]
fn step2_helper_call_respects_caller_lock_and_rejects_unlocked() {
    let locked_source = r#"
struct Box:
    x: Int

def mutate_box(b: Box):
    mut local_b := b
    local_b.x = 42

def main():
    m := mutex_new()
    b := Box(0)
    spawn fn():
        mutex_lock(m)
        mutate_box(b)
        mutex_unlock(m)
    mutex_lock(m)
    mutate_box(b)
    mutex_unlock(m)
"#;
    let (locked_prog, locked_types) = executable(locked_source);
    let locked_plan = compute_ownership_plan(&locked_prog, &locked_types).unwrap();
    let locked_errors = verify_ownership_balance(&locked_prog, &locked_types, &locked_plan);
    assert!(
        locked_errors.is_empty(),
        "locked caller calling helper should pass: {locked_errors:?}"
    );

    let unlocked_source = r#"
struct Box:
    x: Int

def mutate_box(b: Box):
    mut local_b := b
    local_b.x = 42

def main():
    b := Box(0)
    spawn fn():
        print(b.x)
    mutate_box(b)
"#;
    let (unlocked_prog, unlocked_types) = executable(unlocked_source);
    let unlocked_plan = compute_ownership_plan(&unlocked_prog, &unlocked_types).unwrap();
    let unlocked_errors = verify_ownership_balance(&unlocked_prog, &unlocked_types, &unlocked_plan);
    assert!(
        unlocked_errors.iter().any(|e| matches!(
            e.kind,
            OwnershipBalanceErrorKind::UnsynchronizedConcurrentWrite { .. }
        )),
        "unlocked caller calling mutating helper on shared Box must reject: {unlocked_errors:?}"
    );
}

#[test]
fn step2_negative_list_slot_and_push_races_and_nested_field_race_rejected() {
    let list_race = r#"
def main():
    items := [1, 2, 3]
    spawn fn():
        list_push(items, 4)
    list_set(items, 0, 9)
"#;
    let (prog, types) = executable(list_race);
    let plan = compute_ownership_plan(&prog, &types).unwrap();
    let errors = verify_ownership_balance(&prog, &types, &plan);
    let unsync: Vec<_> = errors
        .iter()
        .filter(|e| {
            matches!(
                e.kind,
                OwnershipBalanceErrorKind::UnsynchronizedConcurrentWrite { .. }
            )
        })
        .collect();
    assert!(
        unsync.len() >= 2,
        "both list_push in spawn and list_set in main must be flagged: {errors:?}\nMIR:\n{}",
        lpp_mir::mir_snapshot(&prog)
    );

    let nested_race = r#"
struct Inner:
    v: Int

struct Outer:
    inner: Inner

def main():
    mut o := Outer(Inner(1))
    spawn fn():
        o.inner.v = 2
    print(o.inner.v)
"#;
    let (n_prog, n_types) = executable(nested_race);
    let n_plan = compute_ownership_plan(&n_prog, &n_types).unwrap();
    let n_errors = verify_ownership_balance(&n_prog, &n_types, &n_plan);
    assert!(
        n_errors.iter().any(|e| matches!(
            e.kind,
            OwnershipBalanceErrorKind::UnsynchronizedConcurrentWrite { .. }
        )),
        "nested mutable field write through shared Outer must be rejected: {n_errors:?}"
    );
}

#[test]
fn step2_optimization_does_not_launder_unsynchronized_concurrent_write() {
    let source = r#"
struct Box:
    x: Int

def main():
    mut b := Box(0)
    spawn fn():
        b.x = 1 + 2
    b.x = 3 + 4
"#;
    let (mut program, types) = executable(source);
    let _report = run_optimization(&mut program, &types, OptimizationLevel::O2).unwrap();

    let mut manager = PassManager::new();
    manager.push(OwnershipPlanPass);
    manager.push(OwnershipBalancePass);
    let err = manager
        .run(&mut program, &types)
        .expect_err("optimization must not launder unsynchronized concurrent writes");
    assert!(
        matches!(
            err.kind,
            lpp_passes::PassManagerErrorKind::PassFailed { .. }
        ),
        "expected PassFailed from OwnershipBalancePass, got {err:?}"
    );

    let plan = compute_ownership_plan(&program, &types).unwrap();
    let errors = verify_ownership_balance(&program, &types, &plan);
    assert!(errors.iter().any(|e| matches!(
        e.kind,
        OwnershipBalanceErrorKind::UnsynchronizedConcurrentWrite { .. }
    )));
}

#[test]
fn step2_async_aliased_unsynchronized_write_rejected() {
    let source = r#"
struct Box:
    x: Int

async def worker(b: Box) -> Int:
    b.x = 77
    return b.x

def main():
    mut b := Box(0)
    t := worker(b)
    b.x = 88
    res := t.await
    print(res)
"#;
    let (program, types) = executable(source);
    let plan = compute_ownership_plan(&program, &types).unwrap();
    let errors = verify_ownership_balance(&program, &types, &plan);
    assert!(
        errors.iter().any(|e| matches!(
            e.kind,
            OwnershipBalanceErrorKind::UnsynchronizedConcurrentWrite { .. }
        )),
        "expected UnsynchronizedConcurrentWrite for aliased async argument mutation, got {errors:?}"
    );
}

#[test]
fn step2_async_mutex_synchronized_write_and_unique_move_accepted() {
    let synced = r#"
struct Box:
    x: Int

async def worker(b: Box, m: Int) -> Int:
    mutex_lock(m)
    b.x = 77
    mutex_unlock(m)
    return 1

def main():
    mut b := Box(0)
    m := mutex_new()
    t1 := worker(b, m)
    t2 := worker(b, m)
    print(t1.await + t2.await)
    mutex_free(m)
"#;
    let (program, types) = executable(synced);
    let plan = compute_ownership_plan(&program, &types).unwrap();
    let errors = verify_ownership_balance(&program, &types, &plan);
    assert!(
        errors.is_empty(),
        "expected mutex-synchronized async tasks to pass, got {errors:?}"
    );

    let unique = r#"
struct Box:
    x: Int

async def own_worker(b: Box) -> Int:
    b.x = 99
    return b.x

def main():
    mut b := Box(1)
    t := own_worker(b)
    print(t.await)
"#;
    let (program_u, types_u) = executable(unique);
    let plan_u = compute_ownership_plan(&program_u, &types_u).unwrap();
    let errors_u = verify_ownership_balance(&program_u, &types_u, &plan_u);
    assert!(
        errors_u.is_empty(),
        "expected uniquely transferred async task arg to pass, got {errors_u:?}"
    );
}
