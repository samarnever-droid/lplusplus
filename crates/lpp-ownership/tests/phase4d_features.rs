//! Phase 4D exit-gate evidence: per-cell Frame/Owned/Shared placement,
//! the containment graph with cycle breaking, frame arenas, the plan
//! proof, the pass-manager invariants, snapshot determinism, and
//! linear scaling.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    lower_package,
};
use lpp_mir::{
    CORE_MIR_INVARIANTS, MirBuildOptions, MirFunctionId, MirInvariant, MirLocalId, MirProgram,
    build_mir, mir_snapshot,
};
use lpp_ownership::{
    ArenaPlan, ContainmentNode, OwnershipPlan, OwnershipPlanErrorKind, OwnershipPlanPass,
    TypeStrategy, ValuePlacement, compute_ownership_plan, ownership_plan_snapshot,
    verify_ownership_plan,
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
    (program, types.interner)
}

fn functions(program: &MirProgram) -> Vec<MirFunctionId> {
    program.functions().map(|(id, _)| id).collect()
}

fn function_at(program: &MirProgram, index: usize) -> MirFunctionId {
    functions(program)[index]
}

/// The plan's cell for `(function, local)`, or a panic with context.
fn cell<'plan>(
    program: &'plan MirProgram,
    plan: &'plan OwnershipPlan,
    function: MirFunctionId,
    local: MirLocalId,
) -> &'plan lpp_ownership::CellPlan {
    plan.cells
        .iter()
        .find(|candidate| candidate.function == function && candidate.local == local)
        .unwrap_or_else(|| {
            panic!(
                "plan has no cell for {function:?} / {local:?} ({} functions)",
                program.function_count()
            )
        })
}

/// The locals used by a function's return terminators.
fn returned_locals(program: &MirProgram, function: MirFunctionId) -> Vec<MirLocalId> {
    let function = program.function(function).expect("function exists");
    let mut returned = Vec::new();
    for &block_id in program.function_blocks(function) {
        let block = program.block(block_id).expect("block exists");
        if let lpp_mir::Terminator::Return(Some(lpp_mir::Operand::Copy(local))) = block.terminator {
            returned.push(local);
        }
    }
    returned
}

/// The locals of a function, in MIR declaration order.
fn locals_of(program: &MirProgram, function: MirFunctionId) -> Vec<MirLocalId> {
    let function = program.function(function).expect("function exists");
    program.function_locals(function).to_vec()
}

#[test]
fn non_escaping_locals_are_frame_and_returned_are_owned() {
    let (program, types) =
        executable("def main() -> Int:\n    a := 1\n    b := a + 2\n    return b\n");
    let plan = compute_ownership_plan(&program, &types).unwrap();
    assert!(verify_ownership_plan(&program, &types, &plan).is_empty());

    let main = function_at(&program, 0);
    let locals = locals_of(&program, main);
    // Exactly one local is used by the return terminator; that cell is
    // heap, and every other cell is frame.
    let returned = returned_locals(&program, main);
    assert_eq!(returned.len(), 1);
    for local in &locals {
        let expected = if returned.contains(local) {
            ValuePlacement::Owned
        } else {
            ValuePlacement::Frame
        };
        assert_eq!(
            cell(&program, &plan, main, *local).placement,
            expected,
            "local {local:?} must be {expected:?}"
        );
    }
    assert!(
        plan.stats.frame_cell_count >= 1,
        "at least one non-escaping local must be frame"
    );
    assert_eq!(plan.stats.owned_cell_count, returned.len());
    assert_eq!(plan.stats.shared_cell_count, 0);
    // The arena holds exactly the frame cells, in local order.
    let expected_arena: Vec<MirLocalId> = locals
        .iter()
        .filter(|local| cell(&program, &plan, main, **local).placement == ValuePlacement::Frame)
        .copied()
        .collect();
    assert_eq!(plan.stats.arena_count, 1);
    assert_eq!(plan.arenas[0].cells, expected_arena);
}

#[test]
fn call_arguments_and_returns_are_heap() {
    let (program, types) = executable(
        "def callee(x: Int) -> Int:\n    return x\ndef main() -> Int:\n    a := 1\n    b := callee(a)\n    return b\n",
    );
    let plan = compute_ownership_plan(&program, &types).unwrap();
    assert!(verify_ownership_plan(&program, &types, &plan).is_empty());

    // Every local of both functions escapes (call argument, return,
    // returned parameter) or is a temporary; no cell is frame.
    for function in functions(&program) {
        for local in locals_of(&program, function) {
            let mir_local = program.local(local).unwrap();
            if mir_local.kind == lpp_mir::MirLocalKind::Temporary {
                continue;
            }
            assert_eq!(
                cell(&program, &plan, function, local).placement,
                ValuePlacement::Owned,
                "local {local:?} of {function:?} must be owned"
            );
        }
    }
    // Any remaining arena holds exactly the frame (temporary) cells.
    let frame_cells: Vec<MirLocalId> = plan
        .cells
        .iter()
        .filter(|c| c.placement == ValuePlacement::Frame)
        .map(|c| c.local)
        .collect();
    let arena_cells: Vec<MirLocalId> = plan
        .arenas
        .iter()
        .flat_map(|arena| arena.cells.iter().copied())
        .collect();
    assert_eq!(frame_cells, arena_cells);
}

#[test]
fn spawn_operands_escape() {
    let (program, types) =
        executable("def main():\n    f := fn():\n        print_str(\"x\")\n    spawn(f)\n");
    let plan = compute_ownership_plan(&program, &types).unwrap();
    assert!(verify_ownership_plan(&program, &types, &plan).is_empty());

    let main = function_at(&program, 0);
    // The closure-typed local (the spawn operand) is heap; temporaries
    // are excluded.
    let closure_cells: Vec<_> = plan
        .cells
        .iter()
        .filter(|c| c.function == main)
        .filter(|c| program.local(c.local).unwrap().kind != lpp_mir::MirLocalKind::Temporary)
        .filter(|c| {
            matches!(
                types.kind(program.local(c.local).unwrap().ty),
                TypeKind::Function { .. }
            )
        })
        .collect();
    assert_eq!(closure_cells.len(), 1);
    assert_eq!(closure_cells[0].placement, ValuePlacement::Owned);
}

#[test]
fn capture_operands_escape_and_capture_cells_are_heap() {
    let (program, types) = executable(
        "def main():\n    x := 10\n    f := fn() -> Int:\n        return x\n    print(f())\n",
    );
    let plan = compute_ownership_plan(&program, &types).unwrap();
    assert!(verify_ownership_plan(&program, &types, &plan).is_empty());

    let main = function_at(&program, 0);
    let closure = function_at(&program, 1);
    // The captured local (from the closure's capture list) is heap.
    let captured = program
        .function_captures(program.function(closure).unwrap())
        .to_vec();
    assert_eq!(captured.len(), 1);
    assert_eq!(
        cell(&program, &plan, main, captured[0]).placement,
        ValuePlacement::Owned
    );
    // The closure value itself is not returned, spawned, captured, or
    // passed: it stays in the frame.
    let f_locals: Vec<MirLocalId> = locals_of(&program, main)
        .into_iter()
        .filter(|local| {
            let local = program.local(*local).unwrap();
            local.kind != lpp_mir::MirLocalKind::Temporary
                && matches!(types.kind(local.ty), TypeKind::Function { .. })
        })
        .collect();
    assert_eq!(f_locals.len(), 1);
    assert_eq!(
        cell(&program, &plan, main, f_locals[0]).placement,
        ValuePlacement::Frame
    );
    // The closure's capture cell is heap with an acyclic Int type;
    // temporaries are excluded.
    let capture_cell: Vec<_> = plan
        .cells
        .iter()
        .filter(|c| c.function == closure)
        .filter(|c| program.local(c.local).unwrap().kind != lpp_mir::MirLocalKind::Temporary)
        .collect();
    assert_eq!(capture_cell.len(), 1);
    assert_eq!(capture_cell[0].placement, ValuePlacement::Owned);
}

#[test]
fn await_operands_escape() {
    let (program, types) = executable(
        "async def work() -> Int:\n    return 7\ndef main() -> Int:\n    t := work()\n    r := t.await\n    return r\n",
    );
    let plan = compute_ownership_plan(&program, &types).unwrap();
    assert!(verify_ownership_plan(&program, &types, &plan).is_empty());

    let main = function_at(&program, 1);
    // The task-typed local is the await operand: heap; temporaries are
    // excluded.
    let task_cells: Vec<_> = plan
        .cells
        .iter()
        .filter(|c| c.function == main)
        .filter(|c| program.local(c.local).unwrap().kind != lpp_mir::MirLocalKind::Temporary)
        .filter(|c| {
            matches!(
                types.kind(program.local(c.local).unwrap().ty),
                TypeKind::Task(_)
            )
        })
        .collect();
    assert_eq!(task_cells.len(), 1);
    assert_eq!(task_cells[0].placement, ValuePlacement::Owned);
    // The returned result local is heap too.
    let returned = returned_locals(&program, main);
    for local in &returned {
        assert_eq!(
            cell(&program, &plan, main, *local).placement,
            ValuePlacement::Owned
        );
    }
    // The task type node exists and is not a cycle member.
    assert!(plan.nodes.iter().any(|node| {
        matches!(node.node, ContainmentNode::Task(_)) && node.strategy == TypeStrategy::Owned
    }));
}

#[test]
fn self_referential_list_of_capturing_closure_becomes_shared() {
    let (program, types) = executable(
        "def main():\n    xs := list_new()\n    f := fn():\n        list_get(xs, 0)\n    list_push(xs, f)\n",
    );
    let plan = compute_ownership_plan(&program, &types).unwrap();
    assert!(verify_ownership_plan(&program, &types, &plan).is_empty());

    let main = function_at(&program, 0);
    // xs and f both escape (call arguments) and both own cycle-member
    // types.
    for local in locals_of(&program, main) {
        let mir_local = program.local(local).unwrap();
        if mir_local.kind == lpp_mir::MirLocalKind::Temporary {
            continue;
        }
        assert_eq!(
            cell(&program, &plan, main, local).placement,
            ValuePlacement::Shared,
            "local {local:?} must be shared"
        );
    }
    // Exactly one cycle: the callable and its list node.
    assert_eq!(plan.stats.cycle_count, 1);
    assert_eq!(plan.stats.shared_node_count, 2);
    let shared_nodes: Vec<_> = plan
        .nodes
        .iter()
        .filter(|node| node.strategy == TypeStrategy::Shared)
        .map(|node| node.node)
        .collect();
    assert_eq!(shared_nodes.len(), 2);
    assert!(
        shared_nodes
            .iter()
            .any(|node| matches!(node, ContainmentNode::Callable(_)))
    );
    assert!(
        shared_nodes
            .iter()
            .any(|node| matches!(node, ContainmentNode::List(_)))
    );
    // The closure's capture cell is shared too (the body temporary is
    // frame, so it is not part of the closure's capture cell).
    let closure = function_at(&program, 1);
    let capture: Vec<_> = plan
        .cells
        .iter()
        .filter(|c| c.function == closure)
        .filter(|c| program.local(c.local).unwrap().kind == lpp_mir::MirLocalKind::Capture)
        .collect();
    assert_eq!(capture.len(), 1);
    assert_eq!(capture[0].placement, ValuePlacement::Shared);
    // Frame temporaries give each function a frame arena.
    assert_eq!(plan.stats.arena_count, 2);
}

#[test]
fn mutually_capturing_closures_become_shared() {
    let (program, types) = executable(concat!(
        "def main():\n",
        "    l1 := list_new()\n",
        "    l2 := list_new()\n",
        "    c1 := fn(a: Int):\n",
        "        list_get(l2, 0)\n",
        "    c2 := fn() -> Int:\n",
        "        list_get(l1, 0)\n",
        "        return 1\n",
        "    list_push(l1, c1)\n",
        "    list_push(l2, c2)\n",
    ));
    let plan = compute_ownership_plan(&program, &types).unwrap();
    assert!(verify_ownership_plan(&program, &types, &plan).is_empty());

    // One cycle through both callables and both list nodes.
    assert_eq!(plan.stats.cycle_count, 1);
    assert_eq!(plan.stats.shared_node_count, 4);
    let main = function_at(&program, 0);
    for local in locals_of(&program, main) {
        let mir_local = program.local(local).unwrap();
        if mir_local.kind == lpp_mir::MirLocalKind::Temporary {
            continue;
        }
        assert_eq!(
            cell(&program, &plan, main, local).placement,
            ValuePlacement::Shared,
            "local {local:?} must be shared"
        );
    }
}

#[test]
fn acyclic_container_of_shared_type_stays_owned() {
    let (program, types) = executable(concat!(
        "def main():\n",
        "    inner := list_new()\n",
        "    f := fn():\n",
        "        list_get(inner, 0)\n",
        "    list_push(inner, f)\n",
        "    outer := list_new()\n",
        "    list_push(outer, inner)\n",
    ));
    let plan = compute_ownership_plan(&program, &types).unwrap();
    assert!(verify_ownership_plan(&program, &types, &plan).is_empty());

    let main = function_at(&program, 0);
    let mut saw_owned_list = false;
    let mut saw_shared_list = 0;
    for local in locals_of(&program, main) {
        let mir_local = program.local(local).unwrap();
        if mir_local.kind == lpp_mir::MirLocalKind::Temporary {
            continue;
        }
        let placement = cell(&program, &plan, main, local).placement;
        // The outer list's element is a list type; the inner one's
        // element is a closure type.
        let element_is_list = matches!(
            types.kind(mir_local.ty),
            TypeKind::List(element) if matches!(types.kind(element), TypeKind::List(_))
        );
        if element_is_list {
            saw_owned_list = true;
            assert_eq!(
                placement,
                ValuePlacement::Owned,
                "the outer list must stay owned"
            );
        } else if matches!(types.kind(mir_local.ty), TypeKind::List(_)) {
            saw_shared_list += 1;
            assert_eq!(
                placement,
                ValuePlacement::Shared,
                "the inner list is a cycle member"
            );
        } else {
            // The closure local is a cycle member too.
            assert_eq!(placement, ValuePlacement::Shared);
        }
    }
    assert!(saw_owned_list, "the outer list local must exist");
    assert_eq!(saw_shared_list, 1);
    assert_eq!(plan.stats.cycle_count, 1);
    assert_eq!(plan.stats.shared_node_count, 2);
}

#[test]
fn aggregate_nodes_carry_their_field_containment() {
    let (program, types) = executable(
        "struct Box:\n    value: Int\ndef main() -> Int:\n    b := Box(1)\n    return b.value\n",
    );
    let plan = compute_ownership_plan(&program, &types).unwrap();
    assert!(verify_ownership_plan(&program, &types, &plan).is_empty());

    let box_node = plan
        .nodes
        .iter()
        .find(|node| matches!(node.node, ContainmentNode::Aggregate(_)))
        .expect("the struct instance has a containment node");
    assert_eq!(box_node.strategy, TypeStrategy::Owned);
    assert!(
        box_node.contains.is_empty(),
        "an Int field carries no containment edge"
    );
    // The field load does not escape the local: it stays frame, in the
    // arena.
    let main = function_at(&program, 0);
    let local_cells: Vec<_> = plan.cells.iter().filter(|c| c.function == main).collect();
    let frame_cells: Vec<_> = local_cells
        .iter()
        .filter(|c| c.placement == ValuePlacement::Frame)
        .collect();
    assert!(!frame_cells.is_empty(), "b must be a frame cell");
    assert_eq!(plan.stats.arena_count, 1);
    assert_eq!(
        plan.arenas[0].cells,
        frame_cells.iter().map(|c| c.local).collect::<Vec<_>>()
    );
}

#[test]
fn tampered_plans_fail_the_proof_and_valid_plans_pass() {
    let (program, types) = executable(
        "def main():\n    xs := list_new()\n    f := fn():\n        list_get(xs, 0)\n    list_push(xs, f)\n",
    );
    let plan = compute_ownership_plan(&program, &types).unwrap();
    assert!(verify_ownership_plan(&program, &types, &plan).is_empty());

    // Flip a cell placement.
    let mut flipped = plan.clone();
    flipped.cells[0].placement = ValuePlacement::Owned;
    let errors = verify_ownership_plan(&program, &types, &flipped);
    assert!(
        errors.iter().any(|error| {
            matches!(
                error.kind,
                OwnershipPlanErrorKind::InvalidCellPlacement { .. }
            )
        }),
        "a flipped placement must fail the proof: {errors:?}"
    );
    assert!(errors.iter().all(|error| error.code() == "E4402"));

    // Flip every node strategy.
    let mut strategy = plan.clone();
    for node in &mut strategy.nodes {
        node.strategy = if node.strategy == TypeStrategy::Owned {
            TypeStrategy::Shared
        } else {
            TypeStrategy::Owned
        };
    }
    let errors = verify_ownership_plan(&program, &types, &strategy);
    assert!(errors.iter().any(|error| {
        matches!(
            error.kind,
            OwnershipPlanErrorKind::InvalidNodeStrategy { .. }
        )
    }));

    // Drop an arena (from a program that has one). `hop`'s only cell
    // (the returned parameter) is heap, so it has no arena.
    let (frame_program, frame_types) = executable(
        "def hop(x: Int) -> Int:\n    return x\ndef main() -> Int:\n    a := 1\n    b := a + 2\n    return b\n",
    );
    let frame_plan = compute_ownership_plan(&frame_program, &frame_types).unwrap();
    assert!(verify_ownership_plan(&frame_program, &frame_types, &frame_plan).is_empty());
    let mut no_arena = frame_plan.clone();
    no_arena.arenas.clear();
    let errors = verify_ownership_plan(&frame_program, &frame_types, &no_arena);
    assert!(
        errors
            .iter()
            .any(|error| { matches!(error.kind, OwnershipPlanErrorKind::MissingArena { .. }) })
    );

    // Add a bogus arena for the function without one (hop, index 0).
    let mut extra = frame_plan.clone();
    extra.arenas.push(ArenaPlan {
        function: function_at(&frame_program, 0),
        cells: vec![MirLocalId::from_raw(99)],
    });
    let errors = verify_ownership_plan(&frame_program, &frame_types, &extra);
    assert!(
        errors
            .iter()
            .any(|error| { matches!(error.kind, OwnershipPlanErrorKind::ExtraArena { .. }) })
    );

    // Reorder the cells: the determinism proof must catch it.
    let mut reordered = frame_plan.clone();
    let last = reordered.cells.pop().unwrap();
    reordered.cells.insert(0, last);
    let errors = verify_ownership_plan(&frame_program, &frame_types, &reordered);
    assert!(
        errors
            .iter()
            .any(|error| { matches!(error.kind, OwnershipPlanErrorKind::DeterminismMismatch) })
    );
}

#[test]
fn pass_manager_establishes_and_drops_the_ownership_invariants() {
    let (mut program, types) = executable("def main() -> Int:\n    a := 1\n    return a\n");
    let mut manager = PassManager::new();
    manager.push(OwnershipPlanPass);
    let outcome = manager.run(&mut program, &types).unwrap();
    assert!(outcome.executed.contains(&"ownership-plan"));
    assert!(outcome.established.contains(&MirInvariant::Ownership));
    assert!(outcome.established.contains(&MirInvariant::NoOwningCycles));

    // A later pass that does not preserve ownership drops the
    // invariants from the bookkeeping.
    struct CoreOnly;
    impl MirPass for CoreOnly {
        fn name(&self) -> &'static str {
            "core-only"
        }
        fn preserved(&self) -> &'static [MirInvariant] {
            CORE_MIR_INVARIANTS
        }
        fn run(&mut self, _: &mut MirProgram, _: &PassContext<'_>) -> Result<(), PassFailure> {
            Ok(())
        }
    }
    manager.push(CoreOnly);
    let outcome = manager.run(&mut program, &types).unwrap();
    assert!(!outcome.established.contains(&MirInvariant::Ownership));
    assert!(!outcome.established.contains(&MirInvariant::NoOwningCycles));

    // The previously rejected ownership precondition is now
    // satisfiable: a pass requiring Ownership runs after the plan.
    let (mut program, types) = executable("def main() -> Int:\n    a := 1\n    return a\n");
    let observed = Rc::new(Cell::new(false));
    struct RequiresOwnership(Rc<Cell<bool>>);
    impl MirPass for RequiresOwnership {
        fn name(&self) -> &'static str {
            "requires-ownership"
        }
        fn required(&self) -> &'static [MirInvariant] {
            &[MirInvariant::Ownership]
        }
        fn run(&mut self, _: &mut MirProgram, _: &PassContext<'_>) -> Result<(), PassFailure> {
            self.0.set(true);
            Ok(())
        }
    }
    let mut manager = PassManager::new();
    manager.push(OwnershipPlanPass);
    manager.push(RequiresOwnership(observed.clone()));
    let outcome = manager.run(&mut program, &types).unwrap();
    assert!(observed.get());
    assert_eq!(outcome.executed, ["ownership-plan", "requires-ownership"]);
}

#[test]
fn snapshots_are_deterministic_and_stably_ordered() {
    let (program, types) = executable(concat!(
        "def first() -> Int:\n",
        "    xs := list_new()\n",
        "    f := fn():\n",
        "        list_get(xs, 0)\n",
        "    list_push(xs, f)\n",
        "    return 0\n",
        "def second() -> Int:\n",
        "    a := 1\n",
        "    b := a + 2\n",
        "    return b\n",
    ));
    let plan_a = compute_ownership_plan(&program, &types).unwrap();
    let plan_b = compute_ownership_plan(&program, &types).unwrap();
    assert!(verify_ownership_plan(&program, &types, &plan_a).is_empty());
    assert_eq!(plan_a, plan_b);
    assert_eq!(
        ownership_plan_snapshot(&plan_a),
        ownership_plan_snapshot(&plan_b)
    );

    // Cells are grouped by function id in ascending order.
    let function_ids: Vec<MirFunctionId> = plan_a.cells.iter().map(|c| c.function).collect();
    let ascending = function_ids.windows(2).all(|pair| pair[0] <= pair[1]);
    assert!(ascending, "cells must be grouped by function id");
}

fn scale_source(cyclic: bool, count: usize) -> String {
    let mut source = String::new();
    for index in 0..count {
        source.push_str(&format!("def work_{index}() -> Int:\n"));
        source.push_str("    xs := list_new()\n");
        if cyclic {
            source.push_str("    f := fn():\n        list_get(xs, 0)\n");
        } else {
            source.push_str("    f := fn():\n        print_str(\"x\")\n");
        }
        source.push_str("    list_push(xs, f)\n");
        source.push_str("    return 0\n");
    }
    source
}

#[test]
fn plans_scale_linearly_from_20_to_200() {
    for cyclic in [false, true] {
        let (small_program, small_types) = executable(&scale_source(cyclic, 20));
        let (large_program, large_types) = executable(&scale_source(cyclic, 200));
        let small = compute_ownership_plan(&small_program, &small_types).unwrap();
        let large = compute_ownership_plan(&large_program, &large_types).unwrap();
        assert!(verify_ownership_plan(&small_program, &small_types, &small).is_empty());
        assert!(verify_ownership_plan(&large_program, &large_types, &large).is_empty());

        let delta = |small: usize, large: usize| -> usize {
            assert!(large >= small, "the 200-function plan must not shrink");
            (large - small) / 180
        };
        let per_cell = delta(small.stats.cell_count, large.stats.cell_count);
        let per_arena = delta(small.stats.arena_count, large.stats.arena_count);

        if cyclic {
            // All closures share one function type and all lists share
            // one list type: per function, xs, f, and the capture cell
            // are shared (3 cells); one giant cycle; the shared nodes
            // are the N callables plus the single shared list node.
            assert_eq!(per_cell, 7, "cells per function: {per_cell}");
            assert_eq!(per_arena, 2, "each work_i keeps its own two arenas");
            assert_eq!(small.stats.cycle_count, 1);
            assert_eq!(large.stats.cycle_count, 1);
            assert_eq!(small.stats.shared_node_count, 21);
            assert_eq!(large.stats.shared_node_count, 201);
            assert_eq!(small.stats.shared_cell_count, 60);
            assert_eq!(large.stats.shared_cell_count, 600);
        } else {
            // xs and f are owned call arguments; no cycle. The N
            // callables are distinct nodes but all lists share one
            // type, so node counts are N callables + 1 list node.
            assert_eq!(per_cell, 6, "cells per function: {per_cell}");
            assert_eq!(per_arena, 2, "each work_i keeps its own two arenas");
            assert_eq!(small.stats.node_count, 21);
            assert_eq!(large.stats.node_count, 201);
            assert_eq!(small.stats.cycle_count, 0);
            assert_eq!(large.stats.cycle_count, 0);
            assert_eq!(large.stats.shared_node_count, 0);
        }
    }
}

#[test]
fn all_phase4c3_exit_gate_programs_plan_and_prove_clean() {
    let generated = {
        let mut source = String::from("def main() -> Int:\n    mut total := 0\n");
        for index in 0..200 {
            source.push_str(&format!("    f{index} := fn() -> Int: {index}\n"));
            source.push_str(&format!("    total = total + f{index}()\n"));
        }
        source.push_str("    return total\n");
        Box::leak(source.into_boxed_str())
    };
    let programs: &[&str] = &[
        // 4C3A: string escapes, concatenation, the deterministic subset.
        "def main() -> Str:\n    return \"tab\\tnewline\\nquote\\\"backslash\\\\\"\n",
        "def main() -> Str:\n    return str_concat(\"a\", \"b\")\n",
        // 4C3B: the stateful counter (v1 test_mutable_closure shape).
        concat!(
            "def main():\n",
            "    mut count := 0\n",
            "    counter := fn() -> Int:\n",
            "        count = count + 1\n",
            "        return count\n",
            "    print(counter())\n",
            "    print(counter())\n",
            "    print(counter())\n",
        ),
        // 4C3B: spawn runs its closure exactly once.
        "def main():\n    spawn fn():\n        print_str(\"once\")\n",
        // 4C3B: 200 distinct closures.
        generated,
        // 4C3C: async await chain and double await.
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
    ];
    for source in programs {
        let (program, types) = {
            let filesystem = MemoryFileSystem::new(source);
            let graph = GraphBuilder::new(&filesystem)
                .build(GraphRequest::new(
                    "/ownership/main.lpp",
                    PackageSpec::new("ownership", "/ownership"),
                ))
                .unwrap();
            let package = lower_package(&graph, ResolutionMode::Namespaced).unwrap();
            let mut types = infer_hir_package(&package, ShadowInferenceOptions::default())
                .unwrap_or_else(|error| panic!("type stage for {source:?}: {error:?}"));
            let program = build_mir(
                &package,
                &graph.sources,
                &mut types,
                MirBuildOptions::default(),
            )
            .unwrap_or_else(|error| panic!("build for {source:?}: {error:?}"));
            (program, types.interner)
        };
        let before = mir_snapshot(&program);
        let plan = compute_ownership_plan(&program, &types)
            .unwrap_or_else(|error| panic!("plan failed for {source:?}: {error}"));
        let errors = verify_ownership_plan(&program, &types, &plan);
        assert!(
            errors.is_empty(),
            "the proof must pass for {source:?}: {errors:?}"
        );
        assert_eq!(
            mir_snapshot(&program),
            before,
            "4D must not mutate MIR for {source:?}"
        );
        // None of the 4C3 programs contains an ownership cycle.
        assert_eq!(
            plan.stats.cycle_count, 0,
            "4C3 exit-gate programs have no cycles: {source:?}"
        );
        // Every function with frame cells has exactly one arena.
        let expected_arenas = functions(&program)
            .iter()
            .filter(|function| {
                plan.cells.iter().any(|cell| {
                    cell.function == **function && cell.placement == ValuePlacement::Frame
                })
            })
            .count();
        assert_eq!(plan.stats.arena_count, expected_arenas);
    }
}
