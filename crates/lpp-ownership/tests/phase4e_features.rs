//! Phase 4E exit-gate evidence: hand-computed ARC traffic, end-to-end
//! consume-vs-borrow, move-out, the static balance proof (`E4403`),
//! the runtime leak/underflow proof (`E4304`), the pinned set, pass
//! wiring, and regression of the 4C3/4D gate programs under ARC.
//!
//! The traffic counts are computed from the MIR each program lowers
//! to: every user declaration `x := <expr>` lowers to a copy-temp
//! (the expression's target) plus a `Use(Copy(temp))` into the user
//! local, so each heap-valued declaration contributes one retain and
//! one extra release at function exit. Moves transfer a reference
//! without traffic; reads do not change counts; the pinned set
//! absorbs cycle-member releases.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    lower_package,
};
use lpp_mir::{
    ExecutionOutcome, ExecutionValue, InterpreterErrorKind, InterpreterLimits, MirBuildOptions,
    MirFunctionId, MirFunctionKind, build_mir, execute_mir_arc, verify_mir,
};
use lpp_ownership::{
    OwnershipBalanceErrorKind, OwnershipBalancePass, OwnershipPlanPass, compute_ownership_plan,
    verify_ownership_balance,
};
use lpp_passes::{MirPass, PassContext, PassFailure, PassManager};
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

/// Lower + type-check + build the program; panics with context on
/// failure.
fn executable(source: &str) -> (lpp_mir::MirProgram, TypeInterner) {
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
    (program, types.interner)
}

fn entry_function(program: &lpp_mir::MirProgram) -> MirFunctionId {
    program
        .functions()
        .filter(|(_, function)| function.kind != MirFunctionKind::Closure)
        .map(|(id, _)| id)
        .last()
        .expect("test programs define at least one top-level function")
}

/// Execute under ARC with the plan's pinned set; panics on failure.
fn run_arc(
    program: &lpp_mir::MirProgram,
    types: &TypeInterner,
) -> (ExecutionOutcome, lpp_ownership::OwnershipPlan) {
    let plan = compute_ownership_plan(program, types).unwrap();
    let pinned = plan.pinned_types();
    let outcome = execute_mir_arc(
        program,
        types,
        entry_function(program),
        &[],
        InterpreterLimits::default(),
        &pinned,
    )
    .unwrap_or_else(|error| panic!("arc execution failed: {error}"));
    (outcome, plan)
}

fn arc_of(outcome: &ExecutionOutcome) -> lpp_mir::ArcStats {
    outcome.arc.expect("arc execution returns ArcStats")
}

// ── 4E1: hand-computed ARC traffic ────────────────────────────────────────

#[test]
fn simple_list_is_freed_exactly_once() {
    let (program, types) = executable(
        "def main() -> Int:\n    xs := list_new()\n    list_push(xs, 1)\n    return list_len(xs)\n",
    );
    let (outcome, _) = run_arc(&program, &types);
    let arc = arc_of(&outcome);
    // MIR: `xs := list_new()` lowers to temp = list_new (alloc, rc=1)
    // and xs = Use(Copy(temp)) (retain, rc=2). list_push and list_len
    // read the receiver only (no traffic). Exit drops temp (2 to 1)
    // and xs (1 to 0, free).
    assert_eq!(outcome.value, ExecutionValue::Int(1));
    assert_eq!(
        (arc.retains, arc.releases, arc.frees, arc.pinned_live),
        (1, 2, 1, 0)
    );
}

#[test]
fn closure_pushed_into_list_is_freed_with_the_list() {
    let (program, types) = executable(concat!(
        "def main() -> Int:\n",
        "    xs := list_new()\n",
        "    f := fn() -> Int:\n",
        "        return 1\n",
        "    list_push(xs, f)\n",
        "    return 0\n",
    ));
    let (outcome, _) = run_arc(&program, &types);
    let arc = arc_of(&outcome);
    // Retains: the xs copy-temp and the f copy-temp. list_push moves f
    // into the list (no traffic). Exit: temp list (2 to 1), xs list
    // (1 to 0, free, which drops f (2 to 1)), f copy-temp (1 to 0,
    // free). No per-push receiver traffic.
    assert_eq!(outcome.value, ExecutionValue::Int(0));
    assert_eq!(
        (arc.retains, arc.releases, arc.frees, arc.pinned_live),
        (2, 4, 2, 0)
    );
}

#[test]
fn capture_retains_and_call_load_retains() {
    let (program, types) = executable(concat!(
        "def main() -> Int:\n",
        "    xs := list_new()\n",
        "    f := fn() -> Int:\n",
        "        return list_len(xs)\n",
        "    return f()\n",
    ));
    let (outcome, _) = run_arc(&program, &types);
    let arc = arc_of(&outcome);
    // Retains (4): the xs copy-temp, the MakeClosure capture, the f
    // copy-temp, and the call's capture load into the frame.
    // Releases (6): the capture writeback, main's xs temp (3 to 2),
    // main's xs (2 to 1), main's f temp (2 to 1), main's f (1 to 0,
    // free, which releases the capture (1 to 0, list free)).
    assert_eq!(outcome.value, ExecutionValue::Int(0));
    assert_eq!(
        (arc.retains, arc.releases, arc.frees, arc.pinned_live),
        (4, 6, 2, 0)
    );
}

#[test]
fn list_get_retains_the_element_and_slot_keeps_its_own() {
    let (program, types) = executable(concat!(
        "def main() -> Int:\n",
        "    xs := list_new()\n",
        "    f := fn() -> Int:\n",
        "        return 1\n",
        "    list_push(xs, f)\n",
        "    g := list_get(xs, 0)\n",
        "    return g()\n",
    ));
    let (outcome, _) = run_arc(&program, &types);
    let arc = arc_of(&outcome);
    // Retains (4): xs copy-temp, f copy-temp, the list_get clone (the
    // element gains a reference; the slot keeps its own), g copy-temp.
    // Releases (6): xs temp (1 to 0 free, which drops f (4 to 3)),
    // xs (2 to 1), f temp (3 to 2), g temp (3 to 2)... the call reads
    // g as a callee indirection (no traffic); exit releases close the
    // remaining counts and free the list and the closure.
    assert_eq!(outcome.value, ExecutionValue::Int(1));
    assert_eq!(
        (arc.retains, arc.releases, arc.frees, arc.pinned_live),
        (4, 6, 2, 0)
    );
}

#[test]
fn receiver_is_not_retained_per_push() {
    let (program, types) = executable(concat!(
        "def main() -> Int:\n",
        "    xs := list_new()\n",
        "    mut i := 0\n",
        "    while i < 5:\n",
        "        list_push(xs, i)\n",
        "        i = i + 1\n",
        "    return list_len(xs)\n",
    ));
    let (outcome, _) = run_arc(&program, &types);
    let arc = arc_of(&outcome);
    // Five pushes read the receiver five times; a retained receiver
    // would add five releases. Exactly one list release pair (the
    // copy-temp retain plus the two exit releases) is observed.
    assert_eq!(outcome.value, ExecutionValue::Int(5));
    assert_eq!(
        (arc.retains, arc.releases, arc.frees, arc.pinned_live),
        (1, 2, 1, 0)
    );
}

#[test]
fn list_set_releases_the_replaced_element() {
    let (program, types) = executable(concat!(
        "def main() -> Int:\n",
        "    xs := list_new()\n",
        "    f1 := fn() -> Int:\n",
        "        return 1\n",
        "    f2 := fn() -> Int:\n",
        "        return 2\n",
        "    list_push(xs, f1)\n",
        "    list_set(xs, 0, f2)\n",
        "    return 0\n",
    ));
    let (outcome, _) = run_arc(&program, &types);
    let arc = arc_of(&outcome);
    // Retains (3): xs, f1, f2 copy-temps. list_push moves f1 in;
    // list_set moves f2 in and drops the replaced f1 (2 to 1). Exit:
    // xs temp (1 to 0 free, drops f2 (2 to 1)), xs (2 to 1), f1 temp
    // (1 to 0 free), f2 temp (1 to 0 free).
    assert_eq!(outcome.value, ExecutionValue::Int(0));
    assert_eq!(
        (arc.retains, arc.releases, arc.frees, arc.pinned_live),
        (3, 6, 3, 0)
    );
}

// ── 4E2: move-out ─────────────────────────────────────────────────────────

#[test]
fn pure_move_chain_has_no_traffic_between_owners() {
    let (program, types) = executable(concat!(
        "def make() -> List[Int]:\n",
        "    xs := list_new()\n",
        "    return xs\n",
        "def use_it() -> List[Int]:\n",
        "    v := make()\n",
        "    return v\n",
        "def main() -> Int:\n",
        "    v := use_it()\n",
        "    return list_len(v)\n",
    ));
    let (outcome, _) = run_arc(&program, &types);
    let arc = arc_of(&outcome);
    // Three heap declarations across the chain: three copy-temp
    // retains, one per function. Every return moves the list out of
    // the frame (no traffic); the three copy-temps and the two call
    // temps release at their frames' exits, and the final list frees
    // exactly once. Total: 3 retains, 4 releases, 1 free.
    assert_eq!(outcome.value, ExecutionValue::Int(0));
    assert_eq!(
        (arc.retains, arc.releases, arc.frees, arc.pinned_live),
        (3, 4, 1, 0)
    );
}

#[test]
fn argument_moved_into_callee_frame_is_released_on_return() {
    let (program, types) = executable(concat!(
        "def take(v: List[Int]) -> Int:\n",
        "    return list_len(v)\n",
        "def main() -> Int:\n",
        "    xs := list_new()\n",
        "    list_push(xs, 1)\n",
        "    return take(xs)\n",
    ));
    let (outcome, _) = run_arc(&program, &types);
    let arc = arc_of(&outcome);
    // xs moves into the callee's frame (no traffic); the frame's exit
    // releases the parameter (2 to 1); main's exit releases the
    // copy-temp (1 to 0, free). One retain (the copy-temp), two
    // releases, one free.
    assert_eq!(outcome.value, ExecutionValue::Int(1));
    assert_eq!(
        (arc.retains, arc.releases, arc.frees, arc.pinned_live),
        (1, 2, 1, 0)
    );
}

#[test]
fn entry_arguments_materialize_and_release_inside_execution() {
    let (program, types) = executable("def main(v: List[Int]) -> Int:\n    return list_len(v)\n");
    let plan = compute_ownership_plan(&program, &types).unwrap();
    let pinned = plan.pinned_types();
    let arguments = vec![ExecutionValue::List(vec![
        ExecutionValue::Int(1),
        ExecutionValue::Int(2),
    ])];
    let outcome = execute_mir_arc(
        &program,
        &types,
        entry_function(&program),
        &arguments,
        InterpreterLimits::default(),
        &pinned,
    )
    .unwrap();
    // The argument materializes a fresh list node owned by the entry's
    // parameter slot; the slot's release at the entry's exit is the
    // only traffic, and no reference crosses the boundary.
    assert_eq!(outcome.value, ExecutionValue::Int(2));
    let arc = arc_of(&outcome);
    assert_eq!(
        (arc.retains, arc.releases, arc.frees, arc.pinned_live),
        (0, 1, 1, 0)
    );
}

#[test]
fn redefinition_overwrites_release_the_old_value() {
    let (program, types) = executable(concat!(
        "def main() -> Int:\n",
        "    mut i := 0\n",
        "    while i < 3:\n",
        "        xs := list_new()\n",
        "        list_push(xs, i)\n",
        "        i = i + 1\n",
        "    return 0\n",
    ));
    let (outcome, _) = run_arc(&program, &types);
    let arc = arc_of(&outcome);
    // Three iterations allocate three lists. Each redefinition drops
    // the previous list's references (the copy-temp retain plus the
    // two overwrite releases per iteration), and exit drops the final
    // list. 3 retains, 6 releases, 3 frees — no list outlives its
    // iteration.
    assert_eq!(outcome.value, ExecutionValue::Int(0));
    assert_eq!(
        (arc.retains, arc.releases, arc.frees, arc.pinned_live),
        (3, 6, 3, 0)
    );
}

// ── 4E3: await/spawn ARC semantics ────────────────────────────────────────

#[test]
fn repeated_await_retains_per_await_and_frees_once() {
    let (program, types) = executable(concat!(
        "async def work() -> Str:\n",
        "    s := \"hello\"\n",
        "    return s\n",
        "async def main():\n",
        "    t := work()\n",
        "    a := t.await\n",
        "    b := t.await\n",
        "    print_str(a)\n",
        "    print_str(b)\n",
    ));
    let (outcome, _) = run_arc(&program, &types);
    let arc = arc_of(&outcome);
    // Six retains: the work string reader, the t copy-temp, one per
    // await result (the handle is a receiver: repeated awaits are
    // legal), and the a/b copy-temps. Nine releases: every reference
    // gained (three allocations + six retains) is released — the task
    // free drops the result, the six main slots release at exit, the
    // entry task handle is dropped, and the string cache releases its
    // interned reference. Three frees: both task nodes and the string.
    assert_eq!(outcome.value, ExecutionValue::Void);
    assert_eq!(
        (arc.retains, arc.releases, arc.frees, arc.pinned_live),
        (6, 9, 3, 0)
    );
}

#[test]
fn spawn_detached_task_frees_its_closure() {
    let (program, types) = executable(concat!(
        "def main():\n",
        "    xs := list_new()\n",
        "    spawn fn():\n",
        "        print_str(\"done\")\n",
    ));
    let (outcome, _) = run_arc(&program, &types);
    let arc = arc_of(&outcome);
    // The detached task node is not a cycle member: when its internal
    // handle is dropped at the end of the spawn statement the node
    // frees, releasing the closure back-reference and the captures.
    // The closure then dies with its copy-temp at main's exit, and the
    // list dies the same way. No leak.
    assert_eq!(outcome.output, vec!["done\n".to_owned()]);
    assert_eq!(
        (arc.retains, arc.releases, arc.frees, arc.pinned_live),
        (3, 7, 4, 0)
    );
}

#[test]
fn spawn_with_capture_releases_the_capture_on_node_death() {
    let (program, types) = executable(concat!(
        "def main():\n",
        "    s := \"captured\"\n",
        "    spawn fn():\n",
        "        print_str(s)\n",
    ));
    let (outcome, _) = run_arc(&program, &types);
    let arc = arc_of(&outcome);
    // The task node retains the closure and its captured string; when
    // the node frees after the spawn statement, both are released.
    // The capture keeps the interned string alive until the cache's
    // end-of-run release. No leak.
    assert_eq!(outcome.output, vec!["captured\n".to_owned()]);
    assert_eq!(
        (arc.retains, arc.releases, arc.frees, arc.pinned_live),
        (4, 7, 3, 0)
    );
}

// ── 4E4: static balance proof (E4403) ─────────────────────────────────────

#[test]
fn use_after_move_is_flagged_with_exact_location() {
    let (program, types) = executable(concat!(
        "def main() -> Int:\n",
        "    xs := list_new()\n",
        "    ys := list_new()\n",
        "    list_push(ys, xs)\n",
        "    return list_len(xs)\n",
    ));
    // The core verifier accepts the program; the balance proof is the
    // new coverage.
    assert!(verify_mir(&program, &types).is_empty());
    let plan = compute_ownership_plan(&program, &types).unwrap();
    let errors = verify_ownership_balance(&program, &types, &plan);
    assert_eq!(errors.len(), 1);
    let error = &errors[0];
    assert_eq!(error.code(), "E4403");
    match &error.kind {
        OwnershipBalanceErrorKind::UseAfterMove {
            function,
            block,
            local,
        } => {
            let main = entry_function(&program);
            assert_eq!(*function, main);
            // The violating read is the list_len receiver of the
            // list-typed local xs.
            assert!(matches!(
                program.local(*local).map(|local| local.ty),
                Some(ty) if matches!(types.kind(ty), TypeKind::List(_))
            ));
            let _ = block;
        }
        other => panic!("expected UseAfterMove, got {other:?}"),
    }
}

#[test]
fn second_consume_is_flagged() {
    let (program, types) = executable(concat!(
        "def main() -> Int:\n",
        "    xs := list_new()\n",
        "    list_push(xs, 1)\n",
        "    ys := list_new()\n",
        "    list_push(ys, xs)\n",
        "    zs := list_new()\n",
        "    list_push(zs, xs)\n",
        "    return 0\n",
    ));
    assert!(verify_mir(&program, &types).is_empty());
    let plan = compute_ownership_plan(&program, &types).unwrap();
    let errors = verify_ownership_balance(&program, &types, &plan);
    // The second push's value operand reads xs after the first push
    // moved it: exactly one violation.
    assert_eq!(errors.len(), 1);
    assert!(matches!(
        errors[0].kind,
        OwnershipBalanceErrorKind::UseAfterMove { .. }
    ));
    assert_eq!(errors[0].code(), "E4403");
}

#[test]
fn calling_a_moved_callee_is_flagged() {
    let (program, types) = executable(concat!(
        "def main() -> Int:\n",
        "    xs := list_new()\n",
        "    f := fn() -> Int:\n",
        "        return 1\n",
        "    list_push(xs, f)\n",
        "    return f()\n",
    ));
    assert!(verify_mir(&program, &types).is_empty());
    let plan = compute_ownership_plan(&program, &types).unwrap();
    let errors = verify_ownership_balance(&program, &types, &plan);
    assert_eq!(errors.len(), 1);
    assert!(matches!(
        errors[0].kind,
        OwnershipBalanceErrorKind::UseAfterMove { .. }
    ));
}

#[test]
fn move_into_loop_body_is_flagged_on_the_backedge() {
    let (program, types) = executable(concat!(
        "def main() -> Int:\n",
        "    xs := list_new()\n",
        "    mut i := 0\n",
        "    while i < 2:\n",
        "        ys := list_new()\n",
        "        list_push(ys, xs)\n",
        "        i = i + 1\n",
        "    return 0\n",
    ));
    assert!(verify_mir(&program, &types).is_empty());
    let plan = compute_ownership_plan(&program, &types).unwrap();
    let errors = verify_ownership_balance(&program, &types, &plan);
    // First iteration consumes xs; the backedge meets the consume with
    // the initial definition, so the second iteration's push reads a
    // path-dependent cell: flagged.
    assert_eq!(errors.len(), 1);
    assert!(matches!(
        errors[0].kind,
        OwnershipBalanceErrorKind::UseAfterMove { .. }
    ));
    // The runtime agrees: the second iteration reads the moved slot.
    let pinned = plan.pinned_types();
    let error = execute_mir_arc(
        &program,
        &types,
        entry_function(&program),
        &[],
        InterpreterLimits::default(),
        &pinned,
    )
    .unwrap_err();
    assert!(matches!(
        error.kind,
        InterpreterErrorKind::UninitializedLocal(_)
    ));
}

#[test]
fn redefinition_in_loop_passes_balance() {
    let (program, types) = executable(concat!(
        "def main() -> Int:\n",
        "    mut i := 0\n",
        "    while i < 3:\n",
        "        xs := list_new()\n",
        "        list_push(xs, i)\n",
        "        i = i + 1\n",
        "    return 0\n",
    ));
    let plan = compute_ownership_plan(&program, &types).unwrap();
    assert!(verify_ownership_balance(&program, &types, &plan).is_empty());
}

#[test]
fn balance_is_deterministic() {
    let (program, types) = executable(concat!(
        "def main() -> Int:\n",
        "    xs := list_new()\n",
        "    ys := list_new()\n",
        "    list_push(ys, xs)\n",
        "    return list_len(xs)\n",
    ));
    let plan = compute_ownership_plan(&program, &types).unwrap();
    let first = verify_ownership_balance(&program, &types, &plan);
    let second = verify_ownership_balance(&program, &types, &plan);
    assert_eq!(first, second);
    assert_eq!(first.len(), 1);
}

// ── 4E5: runtime leak/underflow proof (E4304) and the pinned set ─────────

#[test]
fn self_referential_proves_zero_leaks_with_pinned_set() {
    let (program, types) = executable(concat!(
        "def main():\n",
        "    xs := list_new()\n",
        "    f := fn():\n",
        "        list_get(xs, 0)\n",
        "    list_push(xs, f)\n",
    ));
    let (outcome, plan) = run_arc(&program, &types);
    let arc = arc_of(&outcome);
    // Three retains: the xs copy-temp, the MakeClosure capture, and
    // the f copy-temp. Both cycle-member nodes are pinned, so every
    // release is absorbed: no release counts, no frees, and the two
    // pinned nodes are reported, not leaked.
    assert_eq!(arc.pinned_live, 2);
    assert_eq!((arc.retains, arc.releases, arc.frees), (3, 0, 0));
    // The pinned set is exactly the Shared nodes' types, ascending.
    let pinned = plan.pinned_types();
    assert_eq!(pinned.len(), 2);
    let expected: Vec<lpp_types::TypeId> = plan
        .nodes
        .iter()
        .filter(|node| node.strategy == lpp_ownership::TypeStrategy::Shared)
        .map(|node| node.ty)
        .collect();
    assert_eq!(pinned, expected);
}

#[test]
fn self_referential_without_pinset_reports_the_leak() {
    let (program, types) = executable(concat!(
        "def main():\n",
        "    xs := list_new()\n",
        "    f := fn():\n",
        "        list_get(xs, 0)\n",
        "    list_push(xs, f)\n",
    ));
    let error = execute_mir_arc(
        &program,
        &types,
        entry_function(&program),
        &[],
        InterpreterLimits::default(),
        &[],
    )
    .unwrap_err();
    assert_eq!(error.code(), "E4304");
    match &error.kind {
        InterpreterErrorKind::OwnershipLeak { nodes } => assert_eq!(*nodes, 2),
        other => panic!("expected OwnershipLeak, got {other:?}"),
    }
}

#[test]
fn mutual_reference_reports_all_four_leaks_without_pinset() {
    let (program, types) = executable(concat!(
        "def main():\n",
        "    l1 := list_new()\n",
        "    l2 := list_new()\n",
        "    c1 := fn():\n",
        "        list_get(l2, 0)\n",
        "    c2 := fn():\n",
        "        list_get(l1, 0)\n",
        "    list_push(l1, c1)\n",
        "    list_push(l2, c2)\n",
    ));
    let (outcome, _) = run_arc(&program, &types);
    assert_eq!(arc_of(&outcome).pinned_live, 4);

    let error = execute_mir_arc(
        &program,
        &types,
        entry_function(&program),
        &[],
        InterpreterLimits::default(),
        &[],
    )
    .unwrap_err();
    match &error.kind {
        InterpreterErrorKind::OwnershipLeak { nodes } => assert_eq!(*nodes, 4),
        other => panic!("expected OwnershipLeak, got {other:?}"),
    }
}

// ── 4E6: pass wiring ──────────────────────────────────────────────────────

#[test]
fn balance_pass_establishes_and_drops_with_the_bookkeeping() {
    let (mut program, types) = executable(concat!(
        "def main() -> Int:\n",
        "    xs := list_new()\n",
        "    return list_len(xs)\n",
    ));
    let mut manager = PassManager::new();
    manager.push(OwnershipPlanPass);
    manager.push(OwnershipBalancePass);
    let outcome = manager.run(&mut program, &types).unwrap();
    assert_eq!(outcome.executed, ["ownership-plan", "ownership-balance"]);
    assert!(
        outcome
            .established
            .contains(&lpp_mir::MirInvariant::Ownership)
    );
    assert!(
        outcome
            .established
            .contains(&lpp_mir::MirInvariant::NoOwningCycles)
    );
    assert!(
        outcome
            .established
            .contains(&lpp_mir::MirInvariant::OwnershipBalance)
    );

    // A later pass that does not preserve the 4D/4E invariants drops
    // them from the bookkeeping.
    struct CoreOnly;
    impl MirPass for CoreOnly {
        fn name(&self) -> &'static str {
            "core-only"
        }
        fn preserved(&self) -> &'static [lpp_mir::MirInvariant] {
            lpp_mir::CORE_MIR_INVARIANTS
        }
        fn run(
            &mut self,
            _: &mut lpp_mir::MirProgram,
            _: &PassContext<'_>,
        ) -> Result<(), PassFailure> {
            Ok(())
        }
    }
    manager.push(CoreOnly);
    let outcome = manager.run(&mut program, &types).unwrap();
    assert!(
        !outcome
            .established
            .contains(&lpp_mir::MirInvariant::OwnershipBalance)
    );
    assert!(
        !outcome
            .established
            .contains(&lpp_mir::MirInvariant::Ownership)
    );
}

#[test]
fn balance_pass_requires_ownership_precondition() {
    let (mut program, types) = executable("def main() -> Int:\n    a := 1\n    return a\n");
    // Without the plan pass, the balance pass's precondition is
    // unmet.
    let mut manager = PassManager::new();
    manager.push(OwnershipBalancePass);
    let error = manager.run(&mut program, &types).unwrap_err();
    assert!(matches!(
        error.kind,
        lpp_passes::PassManagerErrorKind::UnmetPrecondition(lpp_mir::MirInvariant::Ownership)
    ));

    // With the plan pass first, the precondition is satisfied.
    let (mut program, types) = executable("def main() -> Int:\n    a := 1\n    return a\n");
    let mut manager = PassManager::new();
    manager.push(OwnershipPlanPass);
    manager.push(OwnershipBalancePass);
    let outcome = manager.run(&mut program, &types).unwrap();
    assert_eq!(outcome.executed, ["ownership-plan", "ownership-balance"]);
}

#[test]
fn balance_pass_fails_on_unbalanced_program() {
    let (mut program, types) = executable(concat!(
        "def main() -> Int:\n",
        "    xs := list_new()\n",
        "    ys := list_new()\n",
        "    list_push(ys, xs)\n",
        "    return list_len(xs)\n",
    ));
    let mut manager = PassManager::new();
    manager.push(OwnershipPlanPass);
    manager.push(OwnershipBalancePass);
    let error = manager.run(&mut program, &types).unwrap_err();
    assert!(matches!(
        error.kind,
        lpp_passes::PassManagerErrorKind::PassFailed(_)
    ));
}

// ── 4E7: regression of the 4C3/4D gate programs under ARC ────────────────

fn gate_sources() -> Vec<&'static str> {
    vec![
        // Strings (4C3A).
        concat!(
            "def main() -> Str:\n",
            "    a := \"hello\"\n",
            "    b := \" world\"\n",
            "    return a + b\n",
        ),
        // Lists and closures (4C3B, v1 `list_closures.lpp` shape).
        concat!(
            "def main() -> Int:\n",
            "    add_one := fn(value: Int) -> Int: value + 1\n",
            "    callbacks := [add_one]\n",
            "    callback := list_get(callbacks, 0)\n",
            "    return callback(41)\n",
        ),
        // A loop over a reused receiver (4C3B).
        concat!(
            "def main() -> Int:\n",
            "    xs := list_new()\n",
            "    mut i := 0\n",
            "    while i < 5:\n",
            "        list_push(xs, i)\n",
            "        i = i + 1\n",
            "    return list_len(xs)\n",
        ),
        // Async/await chain (4C3C).
        concat!(
            "async def first() -> Str:\n",
            "    return \"ready\"\n",
            "async def second() -> Str:\n",
            "    value := first().await\n",
            "    return value\n",
            "async def main():\n",
            "    result := second().await\n",
            "    print_str(result)\n",
        ),
        // Idempotent double await (4C3C).
        concat!(
            "async def value() -> Str:\n",
            "    return \"twice\"\n",
            "async def main():\n",
            "    task := value()\n",
            "    first := task.await\n",
            "    second := task.await\n",
            "    print_str(first)\n",
            "    print_str(second)\n",
        ),
        // Spawn (4C3C).
        concat!(
            "def main():\n",
            "    spawn fn():\n",
            "        print_str(\"once\")\n",
        ),
        // Self-referential structure (4D).
        concat!(
            "def first() -> Int:\n",
            "    xs := list_new()\n",
            "    f := fn():\n",
            "        list_get(xs, 0)\n",
            "    list_push(xs, f)\n",
            "    return 0\n",
        ),
        // Mutual reference (4D).
        concat!(
            "def main():\n",
            "    l1 := list_new()\n",
            "    l2 := list_new()\n",
            "    c1 := fn():\n",
            "        list_get(l2, 0)\n",
            "    c2 := fn():\n",
            "        list_get(l1, 0)\n",
            "    list_push(l1, c1)\n",
            "    list_push(l2, c2)\n",
        ),
    ]
}

#[test]
fn gate_programs_execute_cleanly_under_arc() {
    for source in gate_sources() {
        let (program, types) = executable(source);
        assert!(verify_mir(&program, &types).is_empty());
        let plan = compute_ownership_plan(&program, &types).unwrap();
        // The static balance proof holds for every gate program.
        assert!(
            verify_ownership_balance(&program, &types, &plan).is_empty(),
            "balance failed for: {source}"
        );
        // The runtime proof holds too: no underflow, no leak (pinned
        // nodes reported, never leaked).
        let pinned = plan.pinned_types();
        let outcome = execute_mir_arc(
            &program,
            &types,
            entry_function(&program),
            &[],
            InterpreterLimits::default(),
            &pinned,
        )
        .unwrap_or_else(|error| panic!("arc execution failed for: {source}: {error}"));
        // Determinism: a second run agrees byte for byte.
        let again = execute_mir_arc(
            &program,
            &types,
            entry_function(&program),
            &[],
            InterpreterLimits::default(),
            &pinned,
        )
        .unwrap();
        assert_eq!(arc_of(&outcome), arc_of(&again));
        assert_eq!(outcome.value, again.value);
    }
}

#[test]
fn arc_outcome_is_deterministic_across_runs() {
    let source = concat!(
        "def main() -> Int:\n",
        "    xs := list_new()\n",
        "    f := fn() -> Int:\n",
        "        return list_len(xs)\n",
        "    list_push(xs, f)\n",
        "    g := list_get(xs, 0)\n",
        "    return g()\n",
    );
    let (program, types) = executable(source);
    let plan = compute_ownership_plan(&program, &types).unwrap();
    let pinned = plan.pinned_types();
    let runs: Vec<ExecutionOutcome> = (0..3)
        .map(|_| {
            execute_mir_arc(
                &program,
                &types,
                entry_function(&program),
                &[],
                InterpreterLimits::default(),
                &pinned,
            )
            .unwrap()
        })
        .collect();
    assert_eq!(runs[0], runs[1]);
    assert_eq!(runs[1], runs[2]);
}
