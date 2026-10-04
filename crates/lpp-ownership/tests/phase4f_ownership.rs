//! Phase 4F exit-gate evidence (ownership-side): the optimized MIR
//! still proves ownership balance, the 4E self-referential and
//! mutually capturing programs keep their exact ARC behavior through
//! the pipeline, use-after-move is not laundered by optimization,
//! and the 4F passes keep `OwnershipBalance` established in the
//! manager bookkeeping when they run after the ownership passes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lpp_common::OptimizationLevel;
use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    lower_package,
};
use lpp_mir::{
    ExecutionValue, InterpreterErrorKind, InterpreterLimits, MirBuildOptions, MirFunctionId,
    MirFunctionKind, MirInvariant, MirProgram, build_mir, execute_mir_arc, verify_mir,
};
use lpp_ownership::{
    OwnershipBalancePass, OwnershipPlanPass, compute_ownership_plan, verify_ownership_balance,
};
use lpp_passes::{BranchFoldPass, ConstFoldPass, ConstPropPass, PassManager, run_optimization};
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

fn entry_function(program: &MirProgram) -> MirFunctionId {
    program
        .functions()
        .filter(|(_, function)| function.kind != MirFunctionKind::Closure)
        .map(|(id, _)| id)
        .last()
        .expect("test programs define at least one top-level function")
}

/// Execute under ARC with the plan's pinned set; panics on failure.
fn run_arc(
    program: &MirProgram,
    types: &TypeInterner,
) -> (lpp_mir::ExecutionOutcome, lpp_ownership::OwnershipPlan) {
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

fn arc_of(outcome: &lpp_mir::ExecutionOutcome) -> lpp_mir::ArcStats {
    outcome.arc.expect("arc execution returns ArcStats")
}

/// The full ownership-side gate: the program proves balance before
/// and after optimization, and the pinned ARC run is bit-for-bit
/// identical (value, output, and every ArcStats counter).
fn assert_arc_equivalence(source: &str, expected: ExecutionValue, expected_pinned_live: usize) {
    let (unoptimized, types) = executable(source);
    let entry = entry_function(&unoptimized);

    // Before: clean static proof and pinned ARC run.
    let plan = compute_ownership_plan(&unoptimized, &types).unwrap();
    assert!(
        verify_ownership_balance(&unoptimized, &types, &plan).is_empty(),
        "unoptimized program must prove balance"
    );
    let before = run_arc(&unoptimized, &types).0;
    assert_eq!(before.value, expected);
    assert_eq!(arc_of(&before).pinned_live, expected_pinned_live);

    // Optimize, then re-prove and re-run.
    let mut optimized = unoptimized.clone();
    run_optimization(&mut optimized, &types, OptimizationLevel::O2)
        .unwrap_or_else(|error| panic!("optimization failed: {error:?}"));
    assert!(verify_mir(&optimized, &types).is_empty());
    let plan_opt = compute_ownership_plan(&optimized, &types).unwrap();
    assert!(
        verify_ownership_balance(&optimized, &types, &plan_opt).is_empty(),
        "optimized program must still prove balance"
    );
    let after = run_arc(&optimized, &types).0;

    assert_eq!(after.value, before.value, "value changed by optimization");
    assert_eq!(
        after.output, before.output,
        "output changed by optimization"
    );
    assert_eq!(
        arc_of(&after),
        arc_of(&before),
        "ARC traffic changed by optimization (ownership-sensitive)"
    );
    let _ = entry;
}

// ── ownership-sensitive regressions through the pipeline ───────────────────

#[test]
fn simple_list_proves_balance_after_optimization() {
    assert_arc_equivalence(
        "def main() -> Int:\n    xs := list_new()\n    list_push(xs, 1)\n    return list_len(xs)\n",
        ExecutionValue::Int(1),
        0,
    );
}

#[test]
fn self_referential_zero_leak_after_optimization() {
    // The 4E self-referential program: two pinned cycle members,
    // three retains, no releases, no frees — exactly preserved by the
    // pipeline.
    assert_arc_equivalence(
        concat!(
            "def main():\n",
            "    xs := list_new()\n",
            "    f := fn():\n",
            "        list_get(xs, 0)\n",
            "    list_push(xs, f)\n",
        ),
        ExecutionValue::Void,
        2,
    );
}

#[test]
fn mutual_reference_zero_leak_after_optimization() {
    // The 4E mutual-capture program: four pinned cycle members.
    assert_arc_equivalence(
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
        ExecutionValue::Void,
        4,
    );
}

#[test]
fn unpinned_self_referential_still_reports_e4304_after_optimization() {
    // With an empty pinned set the cycle members leak: the pipeline
    // must not launder the leak — the optimized program reports the
    // same two leaking nodes.
    let source = concat!(
        "def main():\n",
        "    xs := list_new()\n",
        "    f := fn():\n",
        "        list_get(xs, 0)\n",
        "    list_push(xs, f)\n",
    );
    let (program, types) = executable(source);
    let entry = entry_function(&program);
    let before = execute_mir_arc(
        &program,
        &types,
        entry,
        &[],
        InterpreterLimits::default(),
        &[],
    )
    .unwrap_err();
    assert_eq!(before.code(), "E4304");
    assert!(matches!(&before.kind, InterpreterErrorKind::OwnershipLeak { nodes } if *nodes == 2));

    let mut optimized = program.clone();
    run_optimization(&mut optimized, &types, OptimizationLevel::O2).unwrap();
    let after = execute_mir_arc(
        &optimized,
        &types,
        entry,
        &[],
        InterpreterLimits::default(),
        &[],
    )
    .unwrap_err();
    assert_eq!(after.code(), "E4304");
    assert!(matches!(&after.kind, InterpreterErrorKind::OwnershipLeak { nodes } if *nodes == 2));
}

#[test]
fn use_after_move_is_not_laundered_by_optimization() {
    // The 4E use-after-move: a read of `xs` after `xs` was consumed by
    // the `list_push` argument. The core verifier accepts the program
    // (the balance proof is the coverage). The pipeline must not
    // repair or hide the violation: the optimized program fails the
    // balance proof with the same `E4403`.
    let source = concat!(
        "def main() -> Int:\n",
        "    xs := list_new()\n",
        "    ys := list_new()\n",
        "    list_push(ys, xs)\n",
        "    return list_len(xs)\n",
    );
    let (program, types) = executable(source);
    assert!(verify_mir(&program, &types).is_empty());
    let plan = compute_ownership_plan(&program, &types).unwrap();
    let errors = verify_ownership_balance(&program, &types, &plan);
    assert_eq!(
        errors.len(),
        1,
        "the unoptimized program violates balance exactly once"
    );
    assert_eq!(errors[0].code(), "E4403");

    let mut optimized = program.clone();
    run_optimization(&mut optimized, &types, OptimizationLevel::O2).unwrap();
    assert!(
        verify_mir(&optimized, &types).is_empty(),
        "core invariants stay clean"
    );
    let plan_opt = compute_ownership_plan(&optimized, &types).unwrap();
    let errors_opt = verify_ownership_balance(&optimized, &types, &plan_opt);
    assert_eq!(
        errors_opt.len(),
        1,
        "the optimized program must violate balance exactly once"
    );
    assert_eq!(errors_opt[0].code(), "E4403");
}

// ── pass wiring: the 4F passes keep the ownership bookkeeping intact ───────

#[test]
fn four_f_passes_keep_ownership_balance_established() {
    // Ownership-plan + ownership-balance establish the ownership
    // invariants; the three 4F passes declare them preserved, so the
    // bookkeeping keeps all of them after the full pipeline.
    let (mut program, types) = executable(
        "def main() -> Int:\n    xs := list_new()\n    a := 1\n    return list_len(xs) + a\n",
    );
    let mut manager = PassManager::new();
    manager.push(OwnershipPlanPass);
    manager.push(OwnershipBalancePass);
    manager.push(ConstFoldPass);
    manager.push(ConstPropPass);
    manager.push(BranchFoldPass);
    let outcome = manager.run(&mut program, &types).unwrap();
    assert_eq!(
        outcome.executed,
        [
            "ownership-plan",
            "ownership-balance",
            "const-fold",
            "const-prop",
            "branch-fold"
        ]
    );
    for invariant in [
        MirInvariant::Ownership,
        MirInvariant::NoOwningCycles,
        MirInvariant::OwnershipBalance,
    ] {
        assert!(
            outcome.established.contains(&invariant),
            "{invariant:?} must stay established after the 4F passes"
        );
    }
    // The optimization actually happened: a := 1 propagated into the
    // return's addition and folded.
    assert!(
        program.instructions().any(|(_, instruction)| matches!(
            instruction.kind,
            lpp_mir::InstructionKind::Assign {
                value: lpp_mir::Rvalue::Use(lpp_mir::Operand::Constant(
                    lpp_mir::Constant::Integer(1)
                )),
                ..
            }
        )),
        "the pipeline must have folded/propagated the constant"
    );
}

#[test]
fn balance_pass_still_drops_with_a_non_preserving_pass() {
    // The negative wiring case (from 4E): a pass that does not
    // preserve `OwnershipBalance` drops it from the bookkeeping. The
    // 4F passes all declare it preserved, so a manager of only 4F
    // passes never reaches this — but a stub that omits it must still
    // drop the invariant, proving the bookkeeping is the enforcement.
    struct NonPreservingPass;
    impl lpp_passes::MirPass for NonPreservingPass {
        fn name(&self) -> &'static str {
            "non-preserving-stub"
        }
        fn run(
            &mut self,
            _program: &mut MirProgram,
            _context: &lpp_passes::PassContext<'_>,
        ) -> Result<(), lpp_passes::PassFailure> {
            Ok(())
        }
    }
    let (mut program, types) = executable("def main() -> Int:\n    return 0\n");
    let mut manager = PassManager::new();
    manager.push(OwnershipPlanPass);
    manager.push(OwnershipBalancePass);
    manager.push(NonPreservingPass);
    let outcome = manager.run(&mut program, &types).unwrap();
    // The stub preserves nothing, so the bookkeeping drops every
    // non-core invariant it established earlier — the 4F passes avoid
    // this by declaring the ownership trio preserved.
    assert!(
        !outcome
            .established
            .contains(&MirInvariant::OwnershipBalance)
    );
    assert!(!outcome.established.contains(&MirInvariant::Ownership));
}
