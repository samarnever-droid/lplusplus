use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    lower_package,
};
use lpp_mir::{
    CORE_MIR_INVARIANTS, Constant, InstructionKind, MirBuildOptions, MirInvariant, MirProgram,
    Operand, Terminator, build_mir,
};
use lpp_passes::{MirPass, PassContext, PassFailure, PassManager, PassManagerErrorKind};
use lpp_types::{ShadowInferenceOptions, TypeInterner, infer_hir_package};

#[derive(Debug)]
struct MemoryFileSystem {
    files: BTreeMap<PathBuf, String>,
}

impl MemoryFileSystem {
    fn new() -> Self {
        Self {
            files: BTreeMap::from([(
                PathBuf::from("/passes/main.lpp"),
                "def main() -> Int:\n    value := 1 + 2\n    return value\n".to_owned(),
            )]),
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
            .ok_or_else(|| FileSystemError::new("read", path, "file not found"))
    }
}

fn valid_program() -> (MirProgram, TypeInterner) {
    let filesystem = MemoryFileSystem::new();
    let graph = GraphBuilder::new(&filesystem)
        .build(GraphRequest::new(
            "/passes/main.lpp",
            PackageSpec::new("passes", "/passes"),
        ))
        .unwrap();
    let package = lower_package(&graph, ResolutionMode::Namespaced).unwrap();
    let mut types = infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap();
    let program = build_mir(
        &package,
        &graph.sources,
        &mut types,
        MirBuildOptions::default(),
    )
    .unwrap();
    (program, types.interner)
}

struct NoOp;

impl MirPass for NoOp {
    fn name(&self) -> &'static str {
        "no-op"
    }

    fn preserved(&self) -> &'static [MirInvariant] {
        CORE_MIR_INVARIANTS
    }

    fn run(
        &mut self,
        _program: &mut MirProgram,
        _context: &PassContext<'_>,
    ) -> Result<(), PassFailure> {
        Ok(())
    }
}

struct RequiresOwnership;

impl MirPass for RequiresOwnership {
    fn name(&self) -> &'static str {
        "requires-ownership"
    }

    fn required(&self) -> &'static [MirInvariant] {
        &[MirInvariant::Ownership]
    }

    fn run(
        &mut self,
        _program: &mut MirProgram,
        _context: &PassContext<'_>,
    ) -> Result<(), PassFailure> {
        panic!("a pass with an unmet precondition must not run")
    }
}

struct CorruptReturn;

impl MirPass for CorruptReturn {
    fn name(&self) -> &'static str {
        "corrupt-return"
    }

    fn run(
        &mut self,
        program: &mut MirProgram,
        _context: &PassContext<'_>,
    ) -> Result<(), PassFailure> {
        let block = program.blocks().next().unwrap().0;
        program.block_mut(block).unwrap().terminator =
            Terminator::Return(Some(Operand::Constant(Constant::Bool(false))));
        Ok(())
    }
}

struct CorruptInitialization;

impl MirPass for CorruptInitialization {
    fn name(&self) -> &'static str {
        "corrupt-initialization"
    }

    fn run(
        &mut self,
        program: &mut MirProgram,
        _context: &PassContext<'_>,
    ) -> Result<(), PassFailure> {
        let instructions = program.instructions().map(|(id, _)| id).collect::<Vec<_>>();
        let InstructionKind::Assign {
            target: replacement,
            ..
        } = program.instruction(instructions[1]).unwrap().kind
        else {
            unreachable!()
        };
        let InstructionKind::Assign { target, .. } =
            &mut program.instruction_mut(instructions[0]).unwrap().kind
        else {
            unreachable!()
        };
        *target = replacement;
        Ok(())
    }
}

struct Observer(Rc<Cell<bool>>);

impl MirPass for Observer {
    fn name(&self) -> &'static str {
        "observer"
    }

    fn run(
        &mut self,
        _program: &mut MirProgram,
        _context: &PassContext<'_>,
    ) -> Result<(), PassFailure> {
        self.0.set(true);
        Ok(())
    }
}

#[test]
fn manager_runs_valid_passes_and_records_established_invariants() {
    let (mut program, types) = valid_program();
    let mut manager = PassManager::new();
    manager.push(NoOp);
    let outcome = manager.run(&mut program, &types).unwrap();
    assert_eq!(outcome.executed, ["no-op"]);
    assert!(
        CORE_MIR_INVARIANTS
            .iter()
            .all(|invariant| outcome.established.contains(invariant)),
    );
}

#[test]
fn manager_rejects_unmet_preconditions_before_running_a_pass() {
    let (mut program, types) = valid_program();
    let mut manager = PassManager::new();
    manager.push(RequiresOwnership);
    let error = manager.run(&mut program, &types).unwrap_err();
    assert_eq!(error.pass, Some("requires-ownership"));
    assert_eq!(
        error.kind,
        PassManagerErrorKind::UnmetPrecondition(MirInvariant::Ownership),
    );
    assert_eq!(error.code(), "E4202");
}

#[test]
fn manager_reestablishes_definite_initialization_after_every_pass() {
    let (mut program, types) = valid_program();
    let observed = Rc::new(Cell::new(false));
    let mut manager = PassManager::new();
    manager.push(CorruptInitialization);
    manager.push(Observer(observed.clone()));
    let error = manager.run(&mut program, &types).unwrap_err();
    assert_eq!(error.pass, Some("corrupt-initialization"));
    let PassManagerErrorKind::InvalidPassOutput(diagnostics) = error.kind else {
        panic!("definite-initialization corruption must invalidate pass output");
    };
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code() == "E4104")
    );
    assert!(!observed.get());
}

#[test]
fn manager_revalidates_before_the_next_pass_observes_invalid_mir() {
    let (mut program, types) = valid_program();
    let observed = Rc::new(Cell::new(false));
    let mut manager = PassManager::new();
    manager.push(CorruptReturn);
    manager.push(Observer(observed.clone()));
    let error = manager.run(&mut program, &types).unwrap_err();
    assert_eq!(error.pass, Some("corrupt-return"));
    assert!(matches!(
        error.kind,
        PassManagerErrorKind::InvalidPassOutput(_),
    ));
    assert_eq!(error.code(), "E4204");
    assert!(!observed.get());
}
