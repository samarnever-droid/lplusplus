//! Phase 4F exit-gate evidence (MIR-side): fold semantics matched to
//! the oracle bit-for-bit, unoptimized-versus-optimized equivalence
//! through the legacy execution entry, shape stability (no
//! instruction is added, removed, or moved), compile-fail gates
//! (exact manager codes for bad pass output and bad input), level
//! budget semantics, linear growth, and tight-limit runs.
//!
//! Ownership-side evidence (balance proof, ARC equivalence,
//! pinned sets, use-after-move, pass wiring) lives in
//! `crates/lpp-ownership/tests/phase4f_ownership.rs`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lpp_common::OptimizationLevel;
use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    lower_package,
};
use lpp_mir::{
    BinaryOperator, Constant, ExecutionOutcome, ExecutionValue, InstructionId, InstructionKind,
    InterpreterErrorKind, InterpreterLimits, MirBuildOptions, MirFunctionId, MirFunctionKind,
    MirInvariant, MirProgram, Operand, Rvalue, Terminator, UnaryOperator, build_mir,
    execute_mir_with_stats, mir_snapshot, verify_mir,
};
use lpp_passes::{
    MirPass, PassContext, PassFailure, PassManager, PassManagerErrorKind, fold_binary, fold_unary,
    optimization_passes, run_optimization,
};
use lpp_types::{ShadowInferenceOptions, TypeInterner, infer_hir_package};

// ── scaffolding ────────────────────────────────────────────────────────────

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

/// Legacy execution with default limits; panics with context on
/// failure.
fn run_legacy(
    program: &MirProgram,
    types: &TypeInterner,
    entry: MirFunctionId,
) -> ExecutionOutcome {
    execute_mir_with_stats(program, types, entry, &[], InterpreterLimits::default())
        .unwrap_or_else(|error| panic!("legacy execution failed: {error}"))
}

fn legacy_with_limits(
    program: &MirProgram,
    types: &TypeInterner,
    entry: MirFunctionId,
    limits: InterpreterLimits,
) -> Result<ExecutionOutcome, lpp_mir::InterpreterError> {
    execute_mir_with_stats(program, types, entry, &[], limits)
}

/// Per-block instruction id sequences: the shape-stability witness.
fn shape(program: &MirProgram) -> Vec<(lpp_mir::BasicBlockId, Vec<InstructionId>)> {
    program
        .blocks()
        .map(|(id, block)| (id, program.block_instructions(block).to_vec()))
        .collect()
}

fn assert_shape_stable(before: &MirProgram, after: &MirProgram) {
    assert_eq!(before.function_count(), after.function_count());
    assert_eq!(before.block_count(), after.block_count());
    assert_eq!(before.local_count(), after.local_count());
    assert_eq!(before.instruction_count(), after.instruction_count());
    assert_eq!(
        shape(before),
        shape(after),
        "the 4F passes rewrite in place: no block or instruction is added, removed, or moved"
    );
}

/// Collect the constants of one kind that appear as the sole value of
/// an `Assign` in the program (used to feed the pure fold functions
/// without hand-constructing origin-tagged constants).
fn assigned_constants(program: &MirProgram, which: fn(&Constant) -> bool) -> Vec<Constant> {
    program
        .instructions()
        .filter_map(|(_, instruction)| {
            if let InstructionKind::Assign {
                value: Rvalue::Use(Operand::Constant(constant)),
                ..
            } = &instruction.kind
            {
                (which)(constant).then_some(*constant)
            } else {
                None
            }
        })
        .collect()
}

/// Run the level and check the full equivalence gate: value and
/// output identical before and after, steps never increasing, shape
/// stable, MIR still validated. Returns the optimized program and its
/// types.
fn assert_equivalence(
    source: &str,
    level: OptimizationLevel,
    expected: ExecutionValue,
) -> (MirProgram, TypeInterner, ExecutionOutcome) {
    let (unoptimized, types) = executable(source);
    let entry = entry_function(&unoptimized);
    let before = run_legacy(&unoptimized, &types, entry);
    assert_eq!(before.value, expected);

    let mut optimized = unoptimized.clone();
    run_optimization(&mut optimized, &types, level)
        .unwrap_or_else(|error| panic!("optimization failed: {error:?}"));
    assert!(verify_mir(&optimized, &types).is_empty());
    assert_shape_stable(&unoptimized, &optimized);

    let after = run_legacy(&optimized, &types, entry);
    assert_eq!(after.value, before.value, "value changed by optimization");
    assert_eq!(
        after.output, before.output,
        "output changed by optimization"
    );
    assert!(
        after.stats.steps <= before.stats.steps,
        "steps increased: {} -> {}",
        before.stats.steps,
        after.stats.steps
    );
    (optimized, types, after)
}

// ── fold semantics: bit-for-bit against the oracle ─────────────────────────

#[path = "phase4f_gate/equivalence.rs"]
mod equivalence;
#[path = "phase4f_gate/folding.rs"]
mod folding;
#[path = "phase4f_gate/limits.rs"]
mod limits;
#[path = "phase4f_gate/lto.rs"]
mod lto;
