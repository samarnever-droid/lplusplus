//! Concurrency safety preservation tests for MIR optimization passes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lpp_common::OptimizationLevel;
use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    lower_package,
};
use lpp_mir::{
    InstructionKind, MirBuildOptions, MirInvariant, MirProgram, Rvalue, build_mir, mir_snapshot,
    verify_mir,
};
use lpp_passes::{
    BranchFoldPass, ConstFoldPass, ConstPropPass, InlinePass, MirPass, PassContext, PassFailure,
    PassManager, optimization_passes, run_optimization,
};
use lpp_types::{ShadowInferenceOptions, TypeInterner, infer_hir_package};

#[derive(Debug)]
struct MemoryFileSystem {
    files: BTreeMap<PathBuf, String>,
}

impl MemoryFileSystem {
    fn new(source: &str) -> Self {
        Self {
            files: BTreeMap::from([(PathBuf::from("/passes/main.lpp"), source.to_owned())]),
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
            "/passes/main.lpp",
            PackageSpec::new("passes", "/passes"),
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

fn builtin_sequence(program: &MirProgram) -> Vec<String> {
    let mut out = Vec::new();
    for (_, func) in program.functions() {
        for &bid in program.function_blocks(func) {
            let block = program.block(bid).unwrap();
            for &iid in program.block_instructions(block) {
                let inst = program.instruction(iid).unwrap();
                if let InstructionKind::Assign {
                    value: Rvalue::Builtin { builtin, .. },
                    ..
                } = inst.kind
                {
                    let name = builtin.descriptor().name;
                    out.push(name.strip_prefix("lpp_").unwrap_or(name).to_owned());
                }
            }
        }
    }
    out
}

struct SeedOwnershipInvariantsPass;

impl MirPass for SeedOwnershipInvariantsPass {
    fn name(&self) -> &'static str {
        "seed-ownership-invariants"
    }

    fn preserved(&self) -> &'static [MirInvariant] {
        &[
            MirInvariant::References,
            MirInvariant::ControlFlow,
            MirInvariant::Types,
            MirInvariant::DefiniteInitialization,
        ]
    }

    fn established(&self) -> &'static [MirInvariant] {
        &[
            MirInvariant::Ownership,
            MirInvariant::NoOwningCycles,
            MirInvariant::OwnershipBalance,
        ]
    }

    fn run(
        &mut self,
        _program: &mut MirProgram,
        _context: &PassContext<'_>,
    ) -> Result<(), PassFailure> {
        Ok(())
    }
}

#[test]
fn optimization_passes_preserve_ownership_balance_when_already_established() {
    let source = r#"
struct Box:
    x: Int

def helper(n: Int) -> Int:
    return n + 1

def main():
    m := mutex_new()
    mut b := Box(0)
    spawn fn():
        mutex_lock(m)
        b.x = helper(41)
        mutex_unlock(m)
    mutex_lock(m)
    b.x = 1 + 2
    mutex_unlock(m)
"#;
    let (mut program, types) = executable(source);
    let mut manager = PassManager::new();
    manager.push(SeedOwnershipInvariantsPass);
    manager.push(ConstFoldPass);
    manager.push(ConstPropPass);
    manager.push(BranchFoldPass);
    manager.push(InlinePass::whole_program());
    let outcome = manager.run(&mut program, &types).unwrap();
    assert_eq!(outcome.executed.len(), 5);
    assert!(outcome.established.contains(&MirInvariant::Ownership));
    assert!(outcome.established.contains(&MirInvariant::NoOwningCycles));
    assert!(
        outcome
            .established
            .contains(&MirInvariant::OwnershipBalance)
    );
    assert!(verify_mir(&program, &types).is_empty());
}

#[test]
fn optimization_preserves_lock_and_spawn_boundaries_and_determinism() {
    let source = r#"
struct Box:
    x: Int

def main():
    m := mutex_new()
    mut b := Box(0)
    if true:
        spawn fn():
            mutex_lock(m)
            b.x = 10 + 5
            mutex_unlock(m)
        mutex_lock(m)
        b.x = 2 + 3
        mutex_unlock(m)
"#;
    let (mut prog_a, types_a) = executable(source);
    let (mut prog_b, types_b) = executable(source);

    let before_builtins = builtin_sequence(&prog_a);
    let lock_count_before = before_builtins
        .iter()
        .filter(|n| *n == "mutex_lock")
        .count();
    let unlock_count_before = before_builtins
        .iter()
        .filter(|n| *n == "mutex_unlock")
        .count();
    assert_eq!(lock_count_before, 2);
    assert_eq!(unlock_count_before, 2);

    run_optimization(&mut prog_a, &types_a, OptimizationLevel::O2).unwrap();
    run_optimization(&mut prog_b, &types_b, OptimizationLevel::O2).unwrap();
    assert_eq!(mir_snapshot(&prog_a), mir_snapshot(&prog_b));

    let after_builtins = builtin_sequence(&prog_a);
    assert_eq!(
        after_builtins.iter().filter(|n| *n == "mutex_lock").count(),
        lock_count_before
    );
    assert_eq!(
        after_builtins
            .iter()
            .filter(|n| *n == "mutex_unlock")
            .count(),
        unlock_count_before
    );

    // Spawn instructions remain intact after O2 optimization.
    let mut spawn_count = 0usize;
    for (_, func) in prog_a.functions() {
        for &bid in prog_a.function_blocks(func) {
            for &iid in prog_a.block_instructions(prog_a.block(bid).unwrap()) {
                if matches!(
                    prog_a.instruction(iid).unwrap().kind,
                    InstructionKind::Assign {
                        value: Rvalue::Spawn(_),
                        ..
                    }
                ) {
                    spawn_count += 1;
                }
            }
        }
    }
    assert_eq!(spawn_count, 1);

    // Also confirm `optimization_passes()` and `run_optimization(..., O3)` produce clean verified MIR.
    let (mut prog_o3, types_o3) = executable(source);
    optimization_passes().run(&mut prog_o3, &types_o3).unwrap();
    run_optimization(&mut prog_o3, &types_o3, OptimizationLevel::O3).unwrap();
    assert!(verify_mir(&prog_o3, &types_o3).is_empty());
}
