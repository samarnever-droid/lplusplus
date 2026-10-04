use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    lower_package,
};
use lpp_mir::{
    BasicBlockId, Constant, DefiniteInitializationLimits, DefiniteInitializationResource,
    InstructionKind, MirBuildErrorKind, MirBuildOptions, MirCapacity, MirLocalKind,
    MirVerificationErrorKind, Operand, Rvalue, Terminator, UnsupportedConstruct, build_mir,
    mir_snapshot, verify_mir, verify_mir_with_limits,
};
use lpp_types::{ShadowInferenceOptions, infer_hir_package};

#[derive(Debug, Default)]
struct MemoryFileSystem {
    files: BTreeMap<PathBuf, String>,
}

impl MemoryFileSystem {
    fn with_source(source: &str) -> Self {
        Self {
            files: BTreeMap::from([(PathBuf::from("/mir/main.lpp"), source.to_owned())]),
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

fn typed(
    source: &str,
) -> (
    lpp_hir::HirPackage,
    lpp_types::ShadowTypeOutput,
    lpp_common::SourceMap,
) {
    let filesystem = MemoryFileSystem::with_source(source);
    let graph = GraphBuilder::new(&filesystem)
        .build(GraphRequest::new(
            "/mir/main.lpp",
            PackageSpec::new("mir-test", "/mir"),
        ))
        .unwrap();
    let package = lower_package(&graph, ResolutionMode::Namespaced).unwrap();
    let types = infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap();
    (package, types, graph.sources.clone())
}

fn representative() -> (
    lpp_hir::HirPackage,
    lpp_types::ShadowTypeOutput,
    lpp_common::SourceMap,
) {
    typed(concat!(
        "def choose(value: Int, flag: Bool) -> Int:\n",
        "    mut total := value + 1\n",
        "    pair := (total, 3)\n",
        "    values := [total, 4]\n",
        "    if flag:\n",
        "        total = total + 2\n",
        "    else:\n",
        "        total = total + 3\n",
        "    while total < 10:\n",
        "        total = total + 1\n",
        "        if total == 8:\n",
        "            break\n",
        "    return total\n",
        "def main() -> Int:\n",
        "    result := choose(1, true)\n",
        "    return result\n",
    ))
}

#[test]
fn representative_typed_hir_builds_exact_valid_mir() {
    let (package, mut types, sources) = representative();
    let first = build_mir(&package, &sources, &mut types, MirBuildOptions::default()).unwrap();
    let second = build_mir(&package, &sources, &mut types, MirBuildOptions::default()).unwrap();
    assert_eq!(first, second);
    assert!(verify_mir(&first, &types.interner).is_empty());
    let snapshot = mir_snapshot(&first);
    if std::env::var_os("LPP_UPDATE_SNAPSHOTS").is_some() {
        fs::write(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/phase4a-mir.snap"),
            &snapshot,
        )
        .unwrap();
    }
    assert_eq!(
        snapshot.replace("\r\n", "\n"),
        include_str!("snapshots/phase4a-mir.snap").replace("\r\n", "\n")
    );
}

#[test]
fn verifier_rejects_bad_cfg_edges_and_instruction_types() {
    let (package, mut types, sources) = representative();
    let mut program =
        build_mir(&package, &sources, &mut types, MirBuildOptions::default()).unwrap();
    let first_function = program.functions().next().unwrap().0;
    let first_block = program.blocks().next().unwrap().0;
    program.function_mut(first_function).unwrap().entry = BasicBlockId::from_raw(999_998);
    program.block_mut(first_block).unwrap().terminator =
        Terminator::Goto(BasicBlockId::from_raw(999_999));
    let errors = verify_mir(&program, &types.interner);
    assert!(errors.iter().any(|error| error.code() == "E4101"));
    assert!(errors.iter().any(|error| error.code() == "E4102"));

    let mut program =
        build_mir(&package, &sources, &mut types, MirBuildOptions::default()).unwrap();
    let instruction = program.instructions().next().unwrap().0;
    let instruction = program.instruction_mut(instruction).unwrap();
    let InstructionKind::Assign { value, .. } = &mut instruction.kind else {
        unreachable!()
    };
    *value = Rvalue::Use(Operand::Constant(Constant::Bool(false)));
    let errors = verify_mir(&program, &types.interner);
    assert!(errors.iter().any(|error| error.code() == "E4103"));
}

#[test]
fn definite_initialization_intersects_diamond_predecessors() {
    let (package, mut types, sources) = typed(concat!(
        "def choose(flag: Bool) -> Int:\n",
        "    mut value := 0\n",
        "    if flag:\n",
        "        value = value + 1\n",
        "    return value\n",
    ));
    let mut program =
        build_mir(&package, &sources, &mut types, MirBuildOptions::default()).unwrap();
    let value = program
        .locals()
        .find_map(|(id, local)| {
            (local.kind == MirLocalKind::User && local.mutable).then_some((id, local.ty))
        })
        .unwrap();
    let temporary = program
        .locals()
        .find_map(|(id, local)| {
            (local.kind == MirLocalKind::Temporary && local.ty == value.1).then_some(id)
        })
        .unwrap();
    let initializer = program
        .instructions()
        .find_map(|(id, instruction)| {
            matches!(
                instruction.kind,
                InstructionKind::Assign { target, .. } if target == value.0
            )
            .then_some(id)
        })
        .unwrap();
    let instruction = program.instruction_mut(initializer).unwrap();
    let InstructionKind::Assign { target, .. } = &mut instruction.kind else {
        unreachable!()
    };
    *target = temporary;

    let errors = verify_mir(&program, &types.interner);
    assert!(errors.iter().any(|error| {
        error.code() == "E4104"
            && error.kind == MirVerificationErrorKind::UninitializedRead(value.0)
    }));
}

#[test]
fn definite_initialization_converges_across_loop_back_edges() {
    let (package, mut types, sources) = typed(concat!(
        "def count(flag: Bool) -> Int:\n",
        "    mut value := 0\n",
        "    while flag:\n",
        "        value = value + 1\n",
        "    return value\n",
    ));
    let mut program =
        build_mir(&package, &sources, &mut types, MirBuildOptions::default()).unwrap();
    let value = program
        .locals()
        .find_map(|(id, local)| {
            (local.kind == MirLocalKind::User && local.mutable).then_some((id, local.ty))
        })
        .unwrap();
    let temporary = program
        .locals()
        .find_map(|(id, local)| {
            (local.kind == MirLocalKind::Temporary && local.ty == value.1).then_some(id)
        })
        .unwrap();
    let initializer = program
        .instructions()
        .find_map(|(id, instruction)| {
            matches!(
                instruction.kind,
                InstructionKind::Assign { target, .. } if target == value.0
            )
            .then_some(id)
        })
        .unwrap();
    let instruction = program.instruction_mut(initializer).unwrap();
    let InstructionKind::Assign { target, .. } = &mut instruction.kind else {
        unreachable!()
    };
    *target = temporary;

    let errors = verify_mir(&program, &types.interner);
    assert!(errors.iter().any(|error| {
        error.code() == "E4104"
            && error.kind == MirVerificationErrorKind::UninitializedRead(value.0)
    }));
}

#[test]
fn unreachable_reads_do_not_fail_definite_initialization() {
    let (package, mut types, sources) = typed(concat!(
        "def choose(flag: Bool) -> Int:\n",
        "    if flag:\n",
        "        return 1 + 2\n",
        "    else:\n",
        "        return 3\n",
    ));
    let mut program =
        build_mir(&package, &sources, &mut types, MirBuildOptions::default()).unwrap();
    let temporary = program
        .locals()
        .find_map(|(id, local)| (local.kind == MirLocalKind::Temporary).then_some(id))
        .unwrap();
    let unreachable = program
        .blocks()
        .find_map(|(id, block)| matches!(block.terminator, Terminator::Unreachable).then_some(id))
        .unwrap();
    program.block_mut(unreachable).unwrap().terminator =
        Terminator::Return(Some(Operand::Copy(temporary)));
    assert!(verify_mir(&program, &types.interner).is_empty());
}

#[test]
fn definite_initialization_has_explicit_state_and_iteration_limits() {
    let (package, mut types, sources) = representative();
    let program = build_mir(&package, &sources, &mut types, MirBuildOptions::default()).unwrap();
    let state_errors = verify_mir_with_limits(
        &program,
        &types.interner,
        DefiniteInitializationLimits {
            max_state_words: 0,
            max_iterations: usize::MAX,
        },
    );
    assert!(state_errors.iter().any(|error| {
        error.code() == "E4105"
            && matches!(
                error.kind,
                MirVerificationErrorKind::DefiniteInitializationLimit {
                    resource: DefiniteInitializationResource::StateWords,
                    ..
                }
            )
    }));

    let iteration_errors = verify_mir_with_limits(
        &program,
        &types.interner,
        DefiniteInitializationLimits {
            max_state_words: usize::MAX,
            max_iterations: 0,
        },
    );
    assert!(iteration_errors.iter().any(|error| {
        error.code() == "E4105"
            && matches!(
                error.kind,
                MirVerificationErrorKind::DefiniteInitializationLimit {
                    resource: DefiniteInitializationResource::Iterations,
                    ..
                }
            )
    }));
}

#[test]
fn forward_direct_calls_and_contextual_empty_lists_verify() {
    let (package, mut types, sources) = typed(concat!(
        "def main() -> Int:\n",
        "    return consume([])\n",
        "def consume(values: List[Int]) -> Int:\n",
        "    return 1\n",
    ));
    let program = build_mir(&package, &sources, &mut types, MirBuildOptions::default()).unwrap();
    assert!(verify_mir(&program, &types.interner).is_empty());
}

#[test]
fn builder_limits_and_unsupported_constructs_are_structured() {
    let (package, mut types, sources) = representative();
    let error = build_mir(
        &package,
        &sources,
        &mut types,
        MirBuildOptions {
            max_blocks: 1,
            ..MirBuildOptions::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.kind, MirBuildErrorKind::Capacity(MirCapacity::Blocks),);
    assert_eq!(error.code(), "E4001");

    let (package, mut types, sources) = typed("def main() -> Int:\n    return missing_fn()\n");
    let error = build_mir(&package, &sources, &mut types, MirBuildOptions::default()).unwrap_err();
    assert_eq!(
        error.kind,
        MirBuildErrorKind::Unsupported(UnsupportedConstruct::UnresolvedName),
    );
    assert_eq!(error.code(), "E4002");
}

#[test]
fn mir_storage_growth_is_linear_for_straight_line_hir() {
    let small_source = generated_bindings(100);
    let large_source = generated_bindings(1_000);
    let (small_package, mut small_types, small_sources) = typed(&small_source);
    let (large_package, mut large_types, large_sources) = typed(&large_source);
    let small = build_mir(
        &small_package,
        &small_sources,
        &mut small_types,
        MirBuildOptions::default(),
    )
    .unwrap();
    let large = build_mir(
        &large_package,
        &large_sources,
        &mut large_types,
        MirBuildOptions::default(),
    )
    .unwrap();

    assert!(large.local_count() >= small.local_count() * 9);
    assert!(large.local_count() <= small.local_count() * 11);
    assert!(large.instruction_count() >= small.instruction_count() * 9);
    assert!(large.instruction_count() <= small.instruction_count() * 11);
    assert!(large.list_entry_count() <= small.list_entry_count() * 11);
    let small_snapshot = mir_snapshot(&small).len();
    let large_snapshot = mir_snapshot(&large).len();
    assert!(large_snapshot <= small_snapshot * 12);
    eprintln!(
        "phase4a-scaling bindings=100/1000 locals={}/{} instructions={}/{} list_entries={}/{} snapshot_bytes={}/{}",
        small.local_count(),
        large.local_count(),
        small.instruction_count(),
        large.instruction_count(),
        small.list_entry_count(),
        large.list_entry_count(),
        small_snapshot,
        large_snapshot,
    );
}

fn generated_bindings(count: usize) -> String {
    let mut source = String::from("def main():\n");
    for index in 0..count {
        source.push_str(&format!("    value_{index:04} := {index} + 1\n"));
    }
    source
}

#[test]
fn mir_ids_and_ranges_are_compact() {
    assert_eq!(std::mem::size_of::<lpp_mir::MirFunctionId>(), 4);
    assert_eq!(std::mem::size_of::<Option<lpp_mir::MirFunctionId>>(), 4);
    assert_eq!(std::mem::size_of::<lpp_mir::BasicBlockId>(), 4);
    assert_eq!(std::mem::size_of::<lpp_mir::MirLocalId>(), 4);
    assert_eq!(std::mem::size_of::<lpp_mir::MirPlaceId>(), 4);
    assert_eq!(std::mem::size_of::<Option<lpp_mir::MirPlaceId>>(), 4);
    assert_eq!(std::mem::size_of::<lpp_mir::InstructionId>(), 4);
    assert_eq!(std::mem::size_of::<lpp_mir::ListRange<Operand>>(), 8);
    assert_eq!(
        std::mem::size_of::<lpp_mir::ListRange<lpp_mir::PlaceProjection>>(),
        8
    );
}
