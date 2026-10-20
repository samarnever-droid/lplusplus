//! Regression suite for ARC cycle breaking (see
//! `documentation/CYCLE_BREAKING.md`).
//!
//! The original failure: the ownership plan classified recursive types
//! but nothing reclaimed values whose references form a cycle —
//! struct cycles were not even classified (the plan ignored struct
//! fields), the interpreter pinned cyclic types and leaked by
//! design, and native codegen dropped the plan and emitted plain
//! refcounting. These tests pin the corrected behaviour at the
//! interpreter/oracle level:
//!
//! * the plan classifies every type that can sit on a value cycle as
//!   collectible (`Shared`, exposed as `pinned_types`),
//! * acyclic values of recursive types free promptly at refcount
//!   zero (`collected == 0`),
//! * unreachable cycles are reclaimed by trial deletion
//!   (`collected == cycle size`), never before they become
//!   unreachable, and nothing leaks (`execute_mir_arc` fails with
//!   E4304 on any survivor).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    lower_package,
};
use lpp_mir::{
    ArcStats, ExecutionOutcome, ExecutionValue, InterpreterErrorKind, InterpreterLimits, LeakClass,
    LeakReport, MirBuildOptions, MirFunctionId, MirFunctionKind, MirProgram, build_mir,
    execute_mir_arc, execute_mir_arc_diagnosed,
};
use lpp_ownership::compute_ownership_plan;
use lpp_types::{ShadowInferenceOptions, TypeInterner, infer_hir_package};

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
        if self.files.contains_key(path) || self.files.keys().any(|f| f.starts_with(path)) {
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
    let fs = MemoryFileSystem::new(source);
    let graph = GraphBuilder::new(&fs)
        .build(GraphRequest::new(
            "/ownership/main.lpp",
            PackageSpec::new("ownership", "/ownership"),
        ))
        .unwrap();
    let package = lower_package(&graph, ResolutionMode::Namespaced).unwrap();
    let mut types = infer_hir_package(&package, ShadowInferenceOptions::default())
        .unwrap_or_else(|e| panic!("types: {e:?}"));
    let program = build_mir(
        &package,
        &graph.sources,
        &mut types,
        MirBuildOptions::default(),
    )
    .unwrap_or_else(|e| panic!("build: {e:?}"));
    (program, types.interner)
}

fn entry(program: &MirProgram) -> MirFunctionId {
    program
        .functions()
        .filter(|(_, f)| f.kind != MirFunctionKind::Closure)
        .map(|(id, _)| id)
        .last()
        .unwrap()
}

/// Compile + run under the ARC oracle; panics unless the run balances.
fn run(source: &str) -> (ExecutionOutcome, ArcStats, usize) {
    run_with_limits(source, InterpreterLimits::default())
}

fn run_with_limits(source: &str, limits: InterpreterLimits) -> (ExecutionOutcome, ArcStats, usize) {
    let (program, types) = executable(source);
    let plan = compute_ownership_plan(&program, &types).expect("ownership plan");
    let collectible = plan.pinned_types();
    let outcome = execute_mir_arc(&program, &types, entry(&program), &[], limits, &collectible)
        .unwrap_or_else(|e| panic!("ARC run failed: code={} kind={:?}", e.code(), e.kind));
    let arc = outcome.arc.clone().expect("ARC stats");
    assert_eq!(arc.pinned_live, 0, "collectible survivors leaked");
    (outcome, arc, collectible.len())
}

/// Run with an explicit collectible set (not the plan's), returning
/// the diagnosed result so leak classification can be asserted.
fn run_diagnosed(
    source: &str,
    pinned: &[lpp_types::TypeId],
) -> (
    Result<ExecutionOutcome, lpp_mir::InterpreterError>,
    Option<LeakReport>,
) {
    let (program, types) = executable(source);
    execute_mir_arc_diagnosed(
        &program,
        &types,
        entry(&program),
        &[],
        InterpreterLimits::default(),
        pinned,
    )
}

fn int(outcome: &ExecutionOutcome) -> i64 {
    match outcome.value {
        ExecutionValue::Int(v) => v,
        ref other => panic!("expected Int result, got {other:?}"),
    }
}

const NODE: &str = "struct Node:\n    id: Int\n    next: Node\n\n";

#[test]
fn self_loop_is_collected() {
    let (out, arc, shared) = run(&format!(
        "{NODE}def main() -> Int:\n    mut a := Node(7, 0)\n    a.next = a\n    return a.next.id\n"
    ));
    assert_eq!(int(&out), 7);
    assert!(shared >= 1, "Node must be classified collectible");
    assert_eq!(arc.collected, 1);
}

#[test]
fn two_node_cycle_is_collected() {
    let (out, arc, _) = run(&format!(
        "{NODE}def main() -> Int:\n    mut a := Node(100, 0)\n    mut b := Node(200, 0)\n    a.next = b\n    b.next = a\n    return a.next.next.id\n"
    ));
    assert_eq!(int(&out), 100);
    assert_eq!(arc.collected, 2);
}

#[test]
fn three_node_cycle_is_collected() {
    let (out, arc, _) = run(&format!(
        "{NODE}def main() -> Int:\n    mut a := Node(1, 0)\n    mut b := Node(2, 0)\n    mut c := Node(3, 0)\n    a.next = b\n    b.next = c\n    c.next = a\n    return a.next.next.next.id\n"
    ));
    assert_eq!(int(&out), 1);
    assert_eq!(arc.collected, 3);
}

#[test]
fn mutual_struct_cycle_is_collected() {
    let (out, arc, _) = run(
        "struct Child:\n    tag: Int\n    parent: Parent\n\nstruct Parent:\n    tag: Int\n    kid: Child\n\ndef main() -> Int:\n    mut c := Child(2, 0)\n    mut p := Parent(1, 0)\n    p.kid = c\n    c.parent = p\n    return c.parent.kid.tag\n",
    );
    assert_eq!(int(&out), 2);
    assert_eq!(arc.collected, 2);
}

#[test]
fn acyclic_values_of_recursive_type_free_promptly() {
    let (out, arc, _) = run(&format!(
        "{NODE}def main() -> Int:\n    tail := Node(2, 0)\n    head := Node(1, tail)\n    return head.next.id\n"
    ));
    assert_eq!(int(&out), 2);
    assert_eq!(
        arc.collected, 0,
        "acyclic values must not need the collector"
    );
    assert_eq!(arc.frees, 2);
}

#[test]
fn multi_edge_cycle_is_collected() {
    // Two fields of `a` point at `b`: the collector must subtract one
    // scratch count per edge, matching the two retains.
    let (out, arc, _) = run(
        "struct Pair:\n    id: Int\n    left: Pair\n    right: Pair\n\ndef main() -> Int:\n    mut a := Pair(1, 0, 0)\n    mut b := Pair(2, 0, 0)\n    a.left = b\n    a.right = b\n    b.left = a\n    return a.left.left.right.id\n",
    );
    assert_eq!(int(&out), 2);
    assert_eq!(arc.collected, 2);
}

#[test]
fn independent_cycles_are_each_collected() {
    let (out, arc, _) = run(&format!(
        "{NODE}def main() -> Int:\n    mut a := Node(1, 0)\n    a.next = a\n    mut b := Node(2, 0)\n    b.next = b\n    return a.id + b.id\n"
    ));
    assert_eq!(int(&out), 3);
    assert_eq!(arc.collected, 2);
}

#[test]
fn diamond_cycle_is_collected() {
    // a -> b, a -> c (via left/right), b -> d, c -> d, d -> a.
    let (out, arc, _) = run(
        "struct D:\n    id: Int\n    left: D\n    right: D\n\ndef main() -> Int:\n    mut a := D(1, 0, 0)\n    mut b := D(2, 0, 0)\n    mut c := D(3, 0, 0)\n    mut d := D(4, 0, 0)\n    a.left = b\n    a.right = c\n    b.left = d\n    c.left = d\n    d.left = a\n    return a.right.left.left.id\n",
    );
    assert_eq!(int(&out), 1);
    assert_eq!(arc.collected, 4);
}

#[test]
fn acyclic_tail_hanging_off_cycle_is_freed_exactly_once() {
    // a <-> b cycle; b also owns a plain chain through a second
    // field. The tail has no reference from outside the unreachable
    // subgraph either, so trial deletion frees all four nodes; the
    // invariant that matters is exactly-once teardown (four nodes,
    // four frees, no survivor).
    let (out, arc, _) = run(
        "struct T:\n    id: Int\n    next: T\n    tail: T\n\ndef main() -> Int:\n    mut a := T(1, 0, 0)\n    mut b := T(2, 0, 0)\n    a.next = b\n    b.next = a\n    b.tail = T(3, 0, 0)\n    b.tail.tail = T(4, 0, 0)\n    return b.tail.tail.id\n",
    );
    assert_eq!(int(&out), 4);
    assert_eq!(arc.collected, 4);
    assert_eq!(arc.frees, 4);
}

#[test]
fn cycle_through_list_is_collected() {
    // a -> a.kids -> b -> b.kids -> a. (`list_push` moves its element
    // argument, so the back-edge is installed through the fresh
    // owner returned by `list_get`.)
    let (out, arc, _) = run(
        "struct Bag:\n    id: Int\n    kids: List[Bag]\n\ndef main() -> Int:\n    mut a := Bag(5, list_new())\n    mut b := Bag(6, list_new())\n    list_push(a.kids, b)\n    got := list_get(a.kids, 0)\n    list_push(got.kids, a)\n    return got.id\n",
    );
    assert_eq!(int(&out), 6);
    assert_eq!(arc.collected, 4, "two nodes + their lists");
}

#[test]
fn cycle_through_closure_capture_is_collected() {
    let (_out, arc, _) = run(
        "def main():\n    xs := list_new()\n    f := fn():\n        list_get(xs, 0)\n    list_push(xs, f)\n",
    );
    assert!(
        arc.collected >= 2,
        "list + closure environment, got {}",
        arc.collected
    );
}

#[test]
fn live_cycle_survives_collections_and_is_freed_when_dropped() {
    // Build a kept 2-cycle, then churn 600 temporary self-loops to
    // force mid-run collection rounds (threshold is 512 roots). The
    // kept cycle is reachable throughout and must still read back
    // intact afterwards; it is reclaimed only when main returns.
    let (out, arc, _) = run(&format!(
        "{NODE}def main() -> Int:\n    mut a := Node(10, 0)\n    mut b := Node(20, 0)\n    a.next = b\n    b.next = a\n    mut i := 0\n    while i < 600:\n        mut t := Node(1, 0)\n        t.next = t\n        i = i + 1\n    return a.next.id + b.next.id\n"
    ));
    assert_eq!(int(&out), 30);
    assert_eq!(arc.collected, 602);
}

#[test]
fn recursive_enum_acyclic_values_free_promptly() {
    let (out, arc, _) = run(
        "enum EList:\n    Nil\n    Cons(head: Int, tail: EList)\n\ndef main() -> Int:\n    xs := EList.Cons(1, EList.Cons(2, EList.Nil))\n    return 3\n",
    );
    assert_eq!(int(&out), 3);
    assert_eq!(arc.collected, 0);
}

#[test]
fn deep_acyclic_chain_drops_without_leak() {
    let (out, arc, _) = run_with_limits(
        &format!(
            "{NODE}def main() -> Int:\n    mut head := Node(0, 0)\n    mut i := 1\n    while i < 2000:\n        head = Node(i, head)\n        i = i + 1\n    return head.id\n"
        ),
        InterpreterLimits {
            // Value snapshots expand the chain per node (O(n^2)
            // elements), far above the default aggregate cap.
            max_aggregate_elements: 100_000_000,
            ..InterpreterLimits::default()
        },
    );
    assert_eq!(int(&out), 1999);
    assert_eq!(arc.collected, 0);
    assert_eq!(arc.frees, 2000);
}

#[test]
fn plain_struct_program_has_no_collectible_types() {
    let (_out, arc, shared) = run(
        "struct Point:\n    x: Int\n    y: Int\n\ndef main() -> Int:\n    p := Point(3, 4)\n    return p.x + p.y\n",
    );
    assert_eq!(shared, 0, "acyclic types stay on plain ARC");
    assert_eq!(arc.collected, 0);
}

#[test]
fn diagnosed_oracle_classifies_an_uncollectible_cycle() {
    // Deliberately pass an empty collectible set (a wrong plan): the
    // oracle must not just count the leak, it must classify it as a
    // closed component whose refcounts are internally explained —
    // the signature of "this type should have been Shared".
    let source = format!(
        "{NODE}def main() -> Int:\n    mut a := Node(1, 0)\n    mut b := Node(2, 0)\n    a.next = b\n    b.next = a\n    return a.id\n"
    );
    let (result, report) = run_diagnosed(&source, &[]);
    let error = result.expect_err("uncollectible cycle must leak");
    assert_eq!(error.code(), "E4304");
    match error.kind {
        InterpreterErrorKind::OwnershipLeak {
            nodes,
            phantom,
            closed,
        } => {
            assert_eq!((nodes, phantom, closed), (2, 0, 2));
        }
        other => panic!("expected OwnershipLeak, got {other:?}"),
    }
    let report = report.expect("leak report");
    assert_eq!(report.live(), 2);
    assert_eq!(report.count(LeakClass::ClosedGarbage), 2);
    for node in &report.nodes {
        assert_eq!(node.kind, "nominal");
        assert!(node.ty.is_some(), "allocation type recorded");
        assert!(!node.collectible, "empty pinned set: nothing collectible");
        assert_eq!(
            node.refcount, node.internal_refs,
            "closed garbage: counts fully explained by the graph"
        );
    }
}

#[test]
fn diagnosed_oracle_reports_nothing_for_a_healthy_run() {
    let source = format!(
        "{NODE}def main() -> Int:\n    mut a := Node(1, 0)\n    mut b := Node(2, 0)\n    a.next = b\n    b.next = a\n    return a.next.id\n"
    );
    let (program, types) = executable(&source);
    let plan = compute_ownership_plan(&program, &types).expect("ownership plan");
    let (result, report) = execute_mir_arc_diagnosed(
        &program,
        &types,
        entry(&program),
        &[],
        InterpreterLimits::default(),
        &plan.pinned_types(),
    );
    let outcome = result.expect("collectible cycle balances");
    assert_eq!(int(&outcome), 2);
    assert!(report.is_none(), "no survivors, no report");
}

#[test]
fn diagnosed_oracle_includes_the_acyclic_tail_of_a_leaked_cycle() {
    // With collection disabled (empty pinned set), the whole
    // unreachable subgraph — cycle members and the acyclic tail they
    // alone reference — is closed garbage: every refcount is
    // explained by edges inside the leaked component.
    let (result, report) = run_diagnosed(
        "struct T:\n    id: Int\n    next: T\n    tail: T\n\ndef main() -> Int:\n    mut a := T(1, 0, 0)\n    mut b := T(2, 0, 0)\n    a.next = b\n    b.next = a\n    b.tail = T(3, 0, 0)\n    return b.tail.id\n",
        &[],
    );
    assert!(result.is_err());
    let report = report.expect("leak report");
    assert_eq!(report.live(), 3);
    assert_eq!(report.count(LeakClass::ClosedGarbage), 3);
    assert_eq!(report.count(LeakClass::PhantomRoot), 0);
}

#[test]
fn cycle_through_structural_tuple_is_collected() {
    // N -> H -> (N, Int) -> N: the containment cycle runs through a
    // structural tuple type, so the tuple type itself must classify
    // Shared and the collector must see through it (the interpreter
    // flattens inline tuples in `heap_children_of`; natively the
    // generated `lpp_trace_t{n}` walks the record's slots).
    let source = "struct H:\n    t: (N, Int)\nstruct N:\n    id: Int\n    h: H\n\ndef main() -> Int:\n    mut a := N(1, 0)\n    mut b := N(2, 0)\n    a.h = H((b, 10))\n    mut bb := a.h.t.0\n    bb.h = H((a, 20))\n    return bb.h.t.0.id + bb.id\n";
    let (outcome, stats, collectible) = run(source);
    assert_eq!(int(&outcome), 3);
    assert_eq!(stats.pinned_live, 0);
    assert!(collectible >= 3, "N, H, and the tuple type are Shared");
    assert!(
        stats.collected >= 2,
        "cycle through tuple must be collected, got {stats:?}"
    );
}
