use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    lower_package,
};
use lpp_mir::{
    Constant, ExecutionValue, InstructionKind, InterpreterErrorKind, InterpreterLimit,
    InterpreterLimits, MirBuildErrorKind, MirBuildOptions, MirCapacity, MirFunctionId, MirLocalId,
    MirLocalKind, MirPlaceId, MirVerificationErrorKind, Operand, PlaceProjection, Rvalue,
    build_mir, execute_mir, mir_snapshot, verify_mir,
};
use lpp_types::{ShadowInferenceOptions, ShadowTypeError, TypeError, infer_hir_package};

#[derive(Debug)]
struct MemoryFileSystem {
    files: BTreeMap<PathBuf, String>,
}

impl MemoryFileSystem {
    fn new(source: &str) -> Self {
        Self {
            files: BTreeMap::from([(PathBuf::from("/places/main.lpp"), source.to_owned())]),
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

fn lowered(source: &str) -> (lpp_hir::HirPackage, lpp_common::SourceMap) {
    let fs = MemoryFileSystem::new(source);
    let graph = GraphBuilder::new(&fs)
        .build(GraphRequest::new(
            "/places/main.lpp",
            PackageSpec::new("places", "/places"),
        ))
        .unwrap();
    (
        lower_package(&graph, ResolutionMode::Namespaced).unwrap(),
        graph.sources.clone(),
    )
}

fn typed(source: &str) -> (lpp_hir::HirPackage, lpp_types::ShadowTypeOutput, lpp_common::SourceMap) {
    let (package, sources) = lowered(source);
    let types = infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap();
    (package, types, sources)
}

fn type_error(source: &str) -> ShadowTypeError {
    let (package, _) = lowered(source);
    infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap_err()
}

fn executable(source: &str) -> (lpp_mir::MirProgram, lpp_types::ShadowTypeOutput) {
    let (package, mut types, sources) = typed(source);
    let program = build_mir(&package, &sources, &mut types, MirBuildOptions::default()).unwrap();
    assert_eq!(verify_mir(&program, &types.interner), []);
    (program, types)
}

fn last_function(program: &lpp_mir::MirProgram) -> MirFunctionId {
    MirFunctionId::from_raw(u32::try_from(program.function_count() - 1).unwrap())
}

fn run_main(source: &str) -> ExecutionValue {
    let (program, types) = executable(source);
    execute_mir(
        &program,
        &types.interner,
        last_function(&program),
        &[],
        InterpreterLimits::default(),
    )
    .unwrap()
}

#[test]
fn list_and_struct_stores_preserve_aliases_and_augmented_place_identity() {
    let source = concat!(
        "struct Box:\n",
        "    value: Int\n",
        "def main() -> Int:\n",
        "    mut values := [1, 2, 3]\n",
        "    alias := values\n",
        "    mut index := 0\n",
        "    values[index] += 4\n",
        "    values[1] = alias[0] + 5\n",
        "    mut box := Box(10)\n",
        "    mut box_alias := box\n",
        "    box_alias.value += 2\n",
        "    return alias[0] + alias[1] + box.value\n",
    );
    assert_eq!(run_main(source), ExecutionValue::Int(27));

    let (program, _) = executable(source);
    assert!(program.places().any(|(_, place)| {
        program
            .place_projections(place)
            .iter()
            .any(|projection| matches!(projection, PlaceProjection::Field(_)))
    }));
    assert!(
        program
            .instructions()
            .any(|(_, instruction)| { matches!(instruction.kind, InstructionKind::Store { .. }) })
    );
}

#[test]
fn nested_place_paths_mutate_the_shared_leaf() {
    let source = concat!(
        "struct Box:\n",
        "    values: List[Int]\n",
        "def main() -> Int:\n",
        "    mut box := Box([1, 2])\n",
        "    alias := box.values\n",
        "    box.values[1] = 41\n",
        "    return alias[0] + alias[1]\n",
    );
    assert_eq!(run_main(source), ExecutionValue::Int(42));
    let (program, _) = executable(source);
    assert!(program.places().any(|(_, place)| matches!(
        program.place_projections(place),
        [PlaceProjection::Field(_)]
    )));
    assert!(program.places().any(|(_, place)| {
        matches!(
            program.place_projections(place),
            [PlaceProjection::ListIndex(_)]
        ) && program.local(place.root).is_some_and(|root| root.mutable)
    }));
}

#[test]
fn nested_store_base_is_materialized_before_the_rhs_mutates_its_parent() {
    let source = concat!(
        "struct Box:\n",
        "    value: Int\n",
        "def replace(boxes: List[Box]) -> Int:\n",
        "    boxes[0] = Box(100)\n",
        "    return 7\n",
        "def main() -> Int:\n",
        "    mut boxes := [Box(1)]\n",
        "    old := boxes[0]\n",
        "    boxes[0].value = replace(boxes)\n",
        "    return old.value * 1000 + boxes[0].value\n",
    );
    assert_eq!(run_main(source), ExecutionValue::Int(7100));
}

#[test]
fn tuple_dot_bracket_and_destructuring_share_static_places() {
    let source = concat!(
        "def main() -> Int:\n",
        "    pair := (20, 22)\n",
        "    (left, right) := pair\n",
        "    return pair.0 + pair[1] + left + right\n",
    );
    assert_eq!(run_main(source), ExecutionValue::Int(84));
    let (program, _) = executable(source);
    let tuple_projections = program
        .places()
        .flat_map(|(_, place)| program.place_projections(place))
        .filter(|projection| matches!(projection, PlaceProjection::TupleField(_)))
        .count();
    assert_eq!(tuple_projections, 4);
}

#[test]
fn list_for_cfg_routes_nested_break_and_continue_through_step_and_exit() {
    let source = concat!(
        "def main() -> Int:\n",
        "    mut total := 0\n",
        "    for x in [1, 2, 3, 4]:\n",
        "        if x == 2:\n",
        "            continue\n",
        "        for y in [10, 20, 30]:\n",
        "            if y == 20:\n",
        "                continue\n",
        "            if x == 4:\n",
        "                break\n",
        "            total += x + y\n",
        "    return total\n",
    );
    assert_eq!(run_main(source), ExecutionValue::Int(88));
    let (program, _) = executable(source);
    assert!(program.instructions().any(|(_, instruction)| {
        matches!(
            instruction.kind,
            InstructionKind::Assign {
                value: lpp_mir::Rvalue::ListLen(_),
                ..
            }
        )
    }));
}

#[test]
fn list_iterable_expression_is_evaluated_once() {
    let source = concat!(
        "def items(counter: List[Int]) -> List[Int]:\n",
        "    counter[0] += 1\n",
        "    return [1, 2]\n",
        "def main() -> Int:\n",
        "    mut counter := [0]\n",
        "    mut total := 0\n",
        "    for value in items(counter):\n",
        "        total += value\n",
        "    return counter[0] * 100 + total\n",
    );
    assert_eq!(run_main(source), ExecutionValue::Int(103));
}

#[test]
fn dynamic_augmented_index_is_evaluated_once_and_parameter_projection_is_mutable() {
    let source = concat!(
        "def choose(counter: List[Int]) -> Int:\n",
        "    counter[0] += 1\n",
        "    return 0\n",
        "def main() -> Int:\n",
        "    mut counter := [0]\n",
        "    mut values := [1]\n",
        "    values[choose(counter)] += 5\n",
        "    return counter[0] * 100 + values[0]\n",
    );
    assert_eq!(run_main(source), ExecutionValue::Int(106));
}

#[test]
fn list_reads_and_writes_are_checked_at_runtime() {
    let source = concat!(
        "def main() -> Int:\n",
        "    mut values := [1]\n",
        "    values[1] = 2\n",
        "    return values[0]\n",
    );
    let (program, types) = executable(source);
    let error = execute_mir(
        &program,
        &types.interner,
        MirFunctionId::from_raw(0),
        &[],
        InterpreterLimits::default(),
    )
    .unwrap_err();
    assert_eq!(
        error.kind,
        InterpreterErrorKind::IndexOutOfBounds { index: 1, len: 1 }
    );
}

#[test]
fn type_stage_rejects_immutable_roots_and_unsound_tuple_indices() {
    let immutable = type_error(concat!(
        "def main() -> Int:\n",
        "    values := [1]\n",
        "    values[0] = 2\n",
        "    return values[0]\n",
    ));
    assert_eq!(immutable.error, TypeError::ImmutableAssignmentRoot);

    let dynamic = type_error(concat!(
        "def main() -> Int:\n",
        "    pair := (1, 2)\n",
        "    index := 0\n",
        "    return pair[index]\n",
    ));
    assert_eq!(dynamic.error, TypeError::TupleIndexMustBeStatic);

    let bounds = type_error(concat!(
        "def main() -> Int:\n",
        "    pair := (1, 2)\n",
        "    return pair[2]\n",
    ));
    assert_eq!(
        bounds.error,
        TypeError::TupleIndexOutOfBounds { index: 2, arity: 2 }
    );

    let tuple_store = type_error(concat!(
        "def main() -> Int:\n",
        "    mut pair := (1, 2)\n",
        "    pair[0] = 3\n",
        "    return pair[0]\n",
    ));
    assert_eq!(tuple_store.error, TypeError::TupleElementAssignment);
}

#[test]
fn place_capacity_and_malformed_references_fail_structurally() {
    let source = concat!(
        "def main() -> Int:\n",
        "    mut values := [1]\n",
        "    values[0] = 2\n",
        "    return values[0]\n",
    );
    let (package, mut types, sources) = typed(source);
    let error = build_mir(&package, &sources, &mut types,
        MirBuildOptions {
            max_places: 0,
            ..MirBuildOptions::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.kind, MirBuildErrorKind::Capacity(MirCapacity::Places));

    let (package, mut types, sources) = typed(source);
    let error = build_mir(&package, &sources, &mut types,
        MirBuildOptions {
            max_projections: 0,
            ..MirBuildOptions::default()
        },
    )
    .unwrap_err();
    assert_eq!(
        error.kind,
        MirBuildErrorKind::Capacity(MirCapacity::Projections)
    );

    let (mut program, types) = executable(source);
    let store = program
        .instructions()
        .find_map(|(id, instruction)| {
            matches!(instruction.kind, InstructionKind::Store { .. }).then_some(id)
        })
        .unwrap();
    let instruction = program.instruction_mut(store).unwrap();
    let InstructionKind::Store { place, .. } = &mut instruction.kind else {
        unreachable!()
    };
    *place = MirPlaceId::from_raw(999_999);
    assert!(verify_mir(&program, &types.interner).iter().any(|error| {
        error.kind
            == MirVerificationErrorKind::MissingReference(lpp_mir::MirEntity::Place(
                MirPlaceId::from_raw(999_999),
            ))
    }));
}

#[test]
fn verifier_rejects_immutable_tuple_and_uninitialized_store_roots() {
    let source = concat!(
        "def main() -> Int:\n",
        "    mut values := [1]\n",
        "    values[0] = 2\n",
        "    return values[0]\n",
    );
    let (program, types) = executable(source);
    let store_place = program
        .instructions()
        .find_map(|(_, instruction)| match instruction.kind {
            InstructionKind::Store { place, .. } => Some(place),
            InstructionKind::Assign { .. } => None,
        })
        .unwrap();
    let root = program.place(store_place).unwrap().root;

    let mut invalid_index = program.clone();
    invalid_index.place_projections_mut(store_place).unwrap()[0] =
        PlaceProjection::ListIndex(Operand::Constant(Constant::Bool(false)));
    assert!(
        verify_mir(&invalid_index, &types.interner)
            .iter()
            .any(|error| matches!(error.kind, MirVerificationErrorKind::TypeMismatch { .. }))
    );

    let mut immutable = program.clone();
    immutable.local_mut(root).unwrap().mutable = false;
    assert!(
        verify_mir(&immutable, &types.interner)
            .iter()
            .any(|error| { error.kind == MirVerificationErrorKind::ImmutableStore(root) })
    );

    let mut uninitialized = program.clone();
    let list_temporary = uninitialized
        .locals()
        .find_map(|(id, local)| (local.kind == MirLocalKind::Temporary).then_some(id))
        .unwrap();
    let initializer = uninitialized
        .instructions()
        .find_map(|(id, instruction)| {
            matches!(
                instruction.kind,
                InstructionKind::Assign { target, .. } if target == root
            )
            .then_some(id)
        })
        .unwrap();
    let InstructionKind::Assign { target, .. } =
        &mut uninitialized.instruction_mut(initializer).unwrap().kind
    else {
        unreachable!()
    };
    *target = list_temporary;
    assert!(
        verify_mir(&uninitialized, &types.interner)
            .iter()
            .any(|error| { error.kind == MirVerificationErrorKind::UninitializedRead(root) })
    );

    let (mut tuple_program, tuple_types) = executable(concat!(
        "def main() -> Int:\n",
        "    mut pair := (1, 2)\n",
        "    return pair[0]\n",
    ));
    let (load_instruction, tuple_place) = tuple_program
        .instructions()
        .find_map(|(id, instruction)| match instruction.kind {
            InstructionKind::Assign {
                value: Rvalue::Load(place),
                ..
            } => Some((id, place)),
            _ => None,
        })
        .unwrap();
    tuple_program
        .instruction_mut(load_instruction)
        .unwrap()
        .kind = InstructionKind::Store {
        place: tuple_place,
        value: Operand::Constant(Constant::Integer(3)),
    };
    assert!(
        verify_mir(&tuple_program, &tuple_types.interner)
            .iter()
            .any(|error| error.kind == MirVerificationErrorKind::TupleElementStore)
    );
}

#[test]
fn heap_nodes_have_an_independent_bound() {
    let (program, types) = executable(concat!("def main() -> List[Int]:\n", "    return [1]\n",));
    let error = execute_mir(
        &program,
        &types.interner,
        MirFunctionId::from_raw(0),
        &[],
        InterpreterLimits {
            max_heap_nodes: 0,
            ..InterpreterLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(
        error.kind,
        InterpreterErrorKind::LimitExceeded(InterpreterLimit::HeapNodes)
    );
}

fn growth_source(count: usize) -> String {
    let mut source = String::from("def main() -> Int:\n    mut values := [0]\n");
    for value in 0..count {
        source.push_str(&format!("    values[0] = {value}\n"));
    }
    source.push_str("    return values[0]\n");
    source
}

#[test]
fn place_and_projection_growth_is_linear_from_twenty_to_two_hundred() {
    let (small, _) = executable(&growth_source(20));
    let (large, _) = executable(&growth_source(200));
    assert_eq!(small.aggregate_count(), large.aggregate_count());
    assert!(large.place_count() <= small.place_count() * 10);
    assert!(large.projection_count() <= small.projection_count() * 10);
    assert!(large.instruction_count() <= small.instruction_count() * 10);
    assert!(mir_snapshot(&large).len() <= mir_snapshot(&small).len() * 10);

    let bogus = MirLocalId::from_raw(999_999);
    assert!(large.local(bogus).is_none());
}

#[test]
fn phase4c2b_mir_snapshot_is_stable() {
    let source = concat!(
        "struct Box:\n",
        "    values: List[Int]\n",
        "def main() -> Int:\n",
        "    pair := (1, 2)\n",
        "    (left, right) := pair\n",
        "    mut box := Box([left, right])\n",
        "    box.values[0] += pair.0\n",
        "    mut total := 0\n",
        "    for value in box.values:\n",
        "        total += value\n",
        "    return total\n",
    );
    let (program, _) = executable(source);
    let snapshot = mir_snapshot(&program);
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/phase4c2b-places-mir.snap");
    if std::env::var_os("LPP_UPDATE_SNAPSHOTS").is_some() {
        fs::write(&fixture, &snapshot).unwrap();
    }
    assert_eq!(
        snapshot,
        include_str!("snapshots/phase4c2b-places-mir.snap")
    );
}
