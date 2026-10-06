use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    lower_package,
};
use lpp_mir::{
    ExecutionValue, Instruction, InstructionKind, InterpreterErrorKind, InterpreterLimit,
    InterpreterLimits, ListRange, MirAggregateId, MirAggregateKind, MirBuildErrorKind,
    MirBuildOptions, MirCapacity, MirFieldId, MirFunctionId, MirVariantId,
    MirVerificationErrorKind, Operand, PlaceProjection, Rvalue, build_mir, execute_mir,
    mir_snapshot, verify_mir,
};
use lpp_types::{
    InstanceKey, InstanceLimits, InstancePlanner, InstanceRequestKind, PrimitiveType,
    ShadowInferenceOptions, ShadowTypeOutput, TypeId, TypeInterner, TypeKind, infer_hir_package,
};

fn assignment_rvalue(instruction: &Instruction) -> Option<Rvalue> {
    match instruction.kind {
        InstructionKind::Assign { value, .. } => Some(value),
        InstructionKind::Store { .. } => None,
    }
}

fn assignment_rvalue_mut(instruction: &mut Instruction) -> &mut Rvalue {
    let InstructionKind::Assign { value, .. } = &mut instruction.kind else {
        panic!("expected assignment instruction")
    };
    value
}

#[derive(Debug)]
struct MemoryFileSystem {
    files: BTreeMap<PathBuf, String>,
}

impl MemoryFileSystem {
    fn new(source: &str) -> Self {
        Self {
            files: BTreeMap::from([(PathBuf::from("/nominal/main.lpp"), source.to_owned())]),
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

fn typed(source: &str) -> (lpp_hir::HirPackage, ShadowTypeOutput, lpp_common::SourceMap) {
    let filesystem = MemoryFileSystem::new(source);
    let graph = GraphBuilder::new(&filesystem)
        .build(GraphRequest::new(
            "/nominal/main.lpp",
            PackageSpec::new("nominal", "/nominal"),
        ))
        .unwrap();
    let package = lower_package(&graph, ResolutionMode::Namespaced).unwrap();
    let types = infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap();
    (package, types, graph.sources.clone())
}

fn built(source: &str) -> (lpp_mir::MirProgram, TypeInterner) {
    let (package, mut types, sources) = typed(source);
    let program = build_mir(&package, &sources, &mut types, MirBuildOptions::default()).unwrap();
    assert!(verify_mir(&program, &types.interner).is_empty());
    (program, types.interner)
}

fn execute_main(program: &lpp_mir::MirProgram, types: &TypeInterner) -> ExecutionValue {
    execute_mir(
        program,
        types,
        MirFunctionId::from_raw(0),
        &[],
        InterpreterLimits::default(),
    )
    .unwrap()
}

fn contains_non_concrete(root: TypeId, types: &TypeInterner) -> bool {
    let mut pending = vec![root];
    let mut visited = BTreeSet::new();
    while let Some(ty) = pending.pop() {
        if !visited.insert(ty) {
            continue;
        }
        match types.kind(ty) {
            TypeKind::Tuple(elements) => pending.extend(types.list(elements).iter().copied()),
            TypeKind::List(element) | TypeKind::Slice(element) | TypeKind::Task(element) => {
                pending.push(element);
            }
            TypeKind::Map { key, value } => {
                pending.push(key);
                pending.push(value);
            }
            TypeKind::Function { parameters, result } => {
                pending.extend(types.list(parameters).iter().copied());
                pending.push(result);
            }
            TypeKind::Nominal { arguments, .. } => {
                pending.extend(types.list(arguments).iter().copied());
            }
            TypeKind::Error
            | TypeKind::GenericParameter(_)
            | TypeKind::BoundVariable(_)
            | TypeKind::InferenceVariable(_)
            | TypeKind::UnresolvedName { .. } => return true,
            TypeKind::Never | TypeKind::Primitive(_) => {}
        }
    }
    false
}

#[test]
fn generic_structs_materialize_once_and_nested_reads_execute() {
    let source = concat!(
        "struct Pair[T]:\n",
        "    first: T\n",
        "    second: T\n",
        "struct Box[T]:\n",
        "    value: T\n",
        "def main() -> Int:\n",
        "    pair := Pair[Int](20, 22)\n",
        "    boxed := Box[Pair[Int]](pair)\n",
        "    return boxed.value.first + boxed.value.second\n",
    );
    let (package, mut first_types, sources) = typed(source);
    let first = build_mir(
        &package,
        &sources,
        &mut first_types,
        MirBuildOptions::default(),
    )
    .unwrap();
    let repeated = build_mir(
        &package,
        &sources,
        &mut first_types,
        MirBuildOptions::default(),
    )
    .unwrap();
    let (package, mut second_types, sources) = typed(source);
    let second = build_mir(
        &package,
        &sources,
        &mut second_types,
        MirBuildOptions::default(),
    )
    .unwrap();

    assert_eq!(first, repeated);
    assert_eq!(first, second);
    assert_eq!(mir_snapshot(&first), mir_snapshot(&second));
    assert!(verify_mir(&first, &first_types.interner).is_empty());
    assert_eq!(
        execute_main(&first, &first_types.interner),
        ExecutionValue::Int(42)
    );
    assert_eq!(first.aggregate_count(), 2);
    assert_eq!(first.field_count(), 3);
    assert_eq!(first.variant_count(), 0);
    assert_eq!(
        first
            .aggregates()
            .map(|(_, aggregate)| aggregate.ty)
            .collect::<BTreeSet<_>>()
            .len(),
        first.aggregate_count(),
    );
    for (_, aggregate) in first.aggregates() {
        assert!(!contains_non_concrete(aggregate.ty, &first_types.interner));
        assert!(aggregate.instance.is_some());
    }
    for (_, field) in first.fields() {
        assert!(!contains_non_concrete(field.ty, &first_types.interner));
    }
    assert_eq!(
        first
            .instructions()
            .filter(|(_, instruction)| {
                matches!(
                    assignment_rvalue(instruction),
                    Some(Rvalue::ConstructStruct { .. })
                )
            })
            .count(),
        2,
    );
    let pair_operands = first
        .instructions()
        .find_map(|(_, instruction)| match assignment_rvalue(instruction) {
            Some(Rvalue::ConstructStruct { fields, .. }) if fields.len() == 2 => Some(fields),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        first.operands(pair_operands),
        [
            Operand::Constant(lpp_mir::Constant::Integer(20)),
            Operand::Constant(lpp_mir::Constant::Integer(22)),
        ],
        "constructor operands retain deterministic source order and are stored once",
    );
    assert_eq!(
        first
            .places()
            .flat_map(|(_, place)| first.place_projections(place))
            .filter(|projection| matches!(projection, PlaceProjection::Field(_)))
            .count(),
        4,
    );
}

#[test]
fn generic_function_aggregate_types_are_concrete_and_optional_records_stay_non_executable() {
    let generic_source = concat!(
        "struct Box[T]:\n",
        "    value: T\n",
        "def unwrap[T](value: T) -> T:\n",
        "    box := Box[T](value)\n",
        "    return box.value\n",
        "def main() -> Int:\n",
        "    return unwrap[Int](42)\n",
    );
    let (program, types) = built(generic_source);
    assert_eq!(execute_main(&program, &types), ExecutionValue::Int(42));
    assert_eq!(program.aggregate_count(), 1);
    let aggregate = program.aggregates().next().unwrap().1;
    assert!(aggregate.instance.is_some());
    assert!(!contains_non_concrete(aggregate.ty, &types));
    assert!(!contains_non_concrete(
        program
            .field(program.aggregate_fields(aggregate)[0])
            .unwrap()
            .ty,
        &types,
    ));

    let direct_source = concat!(
        "struct Box[T]:\n",
        "    value: T\n",
        "def main() -> Int:\n",
        "    box := Box[Int](42)\n",
        "    return box.value\n",
    );
    let (package, mut optional_types, sources) = typed(direct_source);
    let int = optional_types.interner.primitive(PrimitiveType::Int);
    let arguments = optional_types.interner.intern_list(&[int]).unwrap();
    let mut optional = InstancePlanner::new(InstanceLimits::default());
    optional
        .request_root(
            InstanceKey::new(lpp_hir::HirItemId::from_raw(0), arguments),
            None,
            InstanceRequestKind::OptionalSpecialization,
        )
        .unwrap();
    let work = optional.pop_next().unwrap().unwrap();
    optional.complete(work.id).unwrap();
    optional_types.instances = optional;
    let program = build_mir(
        &package,
        &sources,
        &mut optional_types,
        MirBuildOptions::default(),
    )
    .unwrap();
    assert!(verify_mir(&program, &optional_types.interner).is_empty());
    assert_eq!(
        execute_main(&program, &optional_types.interner),
        ExecutionValue::Int(42)
    );
    assert_eq!(program.aggregate_count(), 1);
    assert_eq!(program.aggregates().next().unwrap().1.instance, None);
}

#[test]
fn enum_variants_preserve_identity_payload_width_and_types() {
    let source = concat!(
        "enum Choice[T]:\n",
        "    None\n",
        "    Some(value: T)\n",
        "    Pair(left: T, right: T)\n",
        "def main() -> Choice[Int]:\n",
        "    return Choice.Pair(20, 22)\n",
    );
    let (program, types) = built(source);
    assert_eq!(program.aggregate_count(), 1);
    assert_eq!(program.variant_count(), 3);
    assert_eq!(program.field_count(), 3);
    let aggregate = program.aggregates().next().unwrap();
    assert_eq!(aggregate.1.kind, MirAggregateKind::Enum);
    let variants = program.aggregate_variants(aggregate.1);
    assert_eq!(
        variants
            .iter()
            .map(|variant| program
                .variant_fields(program.variant(*variant).unwrap())
                .len())
            .collect::<Vec<_>>(),
        vec![0, 1, 2],
    );
    let value = execute_main(&program, &types);
    assert_eq!(
        value,
        ExecutionValue::Nominal {
            aggregate: aggregate.0,
            variant: Some(variants[2]),
            fields: vec![ExecutionValue::Int(20), ExecutionValue::Int(22)],
        },
    );

    for (constructor, expected_index, expected_fields) in [
        ("Choice.None()", 0, Vec::new()),
        ("Choice.Some(42)", 1, vec![ExecutionValue::Int(42)]),
    ] {
        let source = format!(
            "enum Choice:\n    None\n    Some(value: Int)\ndef main() -> Choice:\n    return {constructor}\n"
        );
        let (program, types) = built(&source);
        let aggregate = program.aggregates().next().unwrap();
        let variants = program.aggregate_variants(aggregate.1);
        assert_eq!(
            execute_main(&program, &types),
            ExecutionValue::Nominal {
                aggregate: aggregate.0,
                variant: Some(variants[expected_index]),
                fields: expected_fields,
            },
        );
    }
}

#[test]
fn aggregate_verifier_rejects_malformed_rvalues_and_descriptors() {
    let source = concat!(
        "struct Mixed[T]:\n",
        "    value: T\n",
        "    flag: Bool\n",
        "enum Choice[T]:\n",
        "    Some(value: T)\n",
        "def main() -> Int:\n",
        "    mixed := Mixed[Int](42, true)\n",
        "    choice := Choice.Some(7)\n",
        "    return mixed.value\n",
    );
    let (package, mut types, sources) = typed(source);
    let valid = build_mir(&package, &sources, &mut types, MirBuildOptions::default()).unwrap();
    assert!(verify_mir(&valid, &types.interner).is_empty());
    let struct_id = valid
        .aggregates()
        .find_map(|(id, aggregate)| (aggregate.kind == MirAggregateKind::Struct).then_some(id))
        .unwrap();
    let enum_id = valid
        .aggregates()
        .find_map(|(id, aggregate)| (aggregate.kind == MirAggregateKind::Enum).then_some(id))
        .unwrap();
    let fields = valid
        .aggregate_fields(valid.aggregate(struct_id).unwrap())
        .to_vec();
    let construct_struct = valid
        .instructions()
        .find_map(|(id, instruction)| {
            matches!(
                assignment_rvalue(instruction),
                Some(Rvalue::ConstructStruct { .. })
            )
            .then_some(id)
        })
        .unwrap();
    let construct_variant = valid
        .instructions()
        .find_map(|(id, instruction)| {
            matches!(
                assignment_rvalue(instruction),
                Some(Rvalue::ConstructVariant { .. })
            )
            .then_some(id)
        })
        .unwrap();
    let projection = valid
        .instructions()
        .find_map(|(id, instruction)| {
            matches!(assignment_rvalue(instruction), Some(Rvalue::Load(_))).then_some(id)
        })
        .unwrap();

    let mut malformed = valid.clone();
    *assignment_rvalue_mut(malformed.instruction_mut(construct_struct).unwrap()) =
        Rvalue::ConstructStruct {
            aggregate: MirAggregateId::from_raw(999_999),
            fields: ListRange::empty(),
        };
    assert!(verify_mir(&malformed, &types.interner).iter().any(|error| {
        error.kind
            == MirVerificationErrorKind::MissingReference(lpp_mir::MirEntity::Aggregate(
                MirAggregateId::from_raw(999_999),
            ))
            && error.code() == "E4101"
    }));

    let mut malformed = valid.clone();
    let instruction = malformed.instruction_mut(construct_struct).unwrap();
    let Rvalue::ConstructStruct { aggregate, .. } = *assignment_rvalue_mut(instruction) else {
        unreachable!()
    };
    *assignment_rvalue_mut(instruction) = Rvalue::ConstructStruct {
        aggregate,
        fields: ListRange::empty(),
    };
    assert!(verify_mir(&malformed, &types.interner).iter().any(|error| {
        matches!(error.kind, MirVerificationErrorKind::ArityMismatch { .. })
            && error.code() == "E4103"
    }));

    let mut malformed = valid.clone();
    let instruction = malformed.instruction(projection).unwrap();
    let Some(Rvalue::Load(place)) = assignment_rvalue(instruction) else {
        unreachable!()
    };
    malformed.place_mut(place).unwrap().ty = valid.field(fields[1]).unwrap().ty;
    assert!(verify_mir(&malformed, &types.interner).iter().any(|error| {
        matches!(error.kind, MirVerificationErrorKind::TypeMismatch { .. })
            && error.code() == "E4103"
    }));

    let mut malformed = valid.clone();
    let instruction = malformed.instruction_mut(construct_variant).unwrap();
    let Rvalue::ConstructVariant {
        variant, fields, ..
    } = *assignment_rvalue_mut(instruction)
    else {
        unreachable!()
    };
    *assignment_rvalue_mut(instruction) = Rvalue::ConstructVariant {
        aggregate: struct_id,
        variant,
        fields,
    };
    assert!(verify_mir(&malformed, &types.interner).iter().any(|error| {
        matches!(
            error.kind,
            MirVerificationErrorKind::InvalidAggregateType(_)
        ) && error.code() == "E4103"
    }));

    let mut malformed = valid.clone();
    malformed.aggregate_mut(struct_id).unwrap().arguments = types.interner.empty_list();
    assert!(verify_mir(&malformed, &types.interner).iter().any(|error| {
        error.kind == MirVerificationErrorKind::InvalidAggregateDescriptor
            && error.code() == "E4103"
    }));

    let mut malformed = valid.clone();
    malformed
        .variant_mut(
            *malformed
                .aggregate_variants(malformed.aggregate(enum_id).unwrap())
                .first()
                .unwrap(),
        )
        .unwrap()
        .aggregate = struct_id;
    assert!(verify_mir(&malformed, &types.interner).iter().any(|error| {
        error.kind == MirVerificationErrorKind::InvalidAggregateDescriptor
            && error.code() == "E4103"
    }));
}

#[test]
fn aggregate_builder_and_interpreter_limits_are_structured() {
    let cases = [
        (
            MirBuildOptions {
                max_aggregates: 0,
                ..MirBuildOptions::default()
            },
            MirCapacity::Aggregates,
        ),
        (
            MirBuildOptions {
                max_aggregate_fields: 1,
                ..MirBuildOptions::default()
            },
            MirCapacity::AggregateFields,
        ),
        (
            MirBuildOptions {
                max_operands: 1,
                ..MirBuildOptions::default()
            },
            MirCapacity::Operands,
        ),
    ];
    let struct_source = concat!(
        "struct Pair:\n",
        "    left: Int\n",
        "    right: Int\n",
        "def main() -> Int:\n",
        "    pair := Pair(20, 22)\n",
        "    return pair.left + pair.right\n",
    );
    for (options, capacity) in cases {
        let (package, mut types, sources) = typed(struct_source);
        let error = build_mir(&package, &sources, &mut types, options).unwrap_err();
        assert_eq!(error.kind, MirBuildErrorKind::Capacity(capacity));
        assert_eq!(error.code(), "E4001");
    }

    let enum_source = concat!(
        "enum Choice:\n",
        "    None\n",
        "def main() -> Choice:\n",
        "    return Choice.None()\n",
    );
    let (package, mut types, sources) = typed(enum_source);
    let error = build_mir(
        &package,
        &sources,
        &mut types,
        MirBuildOptions {
            max_aggregate_variants: 0,
            ..MirBuildOptions::default()
        },
    )
    .unwrap_err();
    assert_eq!(
        error.kind,
        MirBuildErrorKind::Capacity(MirCapacity::AggregateVariants),
    );
    assert_eq!(error.code(), "E4001");

    let (program, types) = built(struct_source);
    let error = execute_mir(
        &program,
        &types,
        MirFunctionId::from_raw(0),
        &[],
        InterpreterLimits {
            max_aggregate_elements: 1,
            ..InterpreterLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(
        error.kind,
        InterpreterErrorKind::LimitExceeded(InterpreterLimit::AggregateElements),
    );
    assert_eq!(error.code(), "E4302");
}

#[test]
fn repeated_aggregate_use_deduplicates_schema_and_scales_linearly() {
    let small_source = repeated_pairs(20);
    let large_source = repeated_pairs(200);
    let (small, small_types) = built(&small_source);
    let (large, large_types) = built(&large_source);

    assert_eq!(small.aggregate_count(), 1);
    assert_eq!(large.aggregate_count(), 1);
    assert_eq!(small.field_count(), 2);
    assert_eq!(large.field_count(), 2);
    assert_eq!(small.variant_count(), 0);
    assert_eq!(large.variant_count(), 0);
    assert_eq!(execute_main(&small, &small_types), ExecutionValue::Int(77));
    assert_eq!(execute_main(&large, &large_types), ExecutionValue::Int(797));
    assert!(large.instruction_count() <= small.instruction_count() * 10);
    assert!(large.local_count() <= small.local_count() * 10);
}

fn repeated_pairs(count: usize) -> String {
    let mut source =
        String::from("struct Pair:\n    left: Int\n    right: Int\ndef main() -> Int:\n");
    for index in 0..count {
        source.push_str(&format!(
            "    pair_{index:04} := Pair({}, {})\n",
            index * 2,
            index * 2 + 1,
        ));
    }
    source.push_str(&format!(
        "    return pair_{:04}.left + pair_{:04}.right\n",
        count - 1,
        count - 1,
    ));
    source
}

#[test]
fn concrete_descriptor_snapshot_is_stable() {
    let source = concat!(
        "struct Pair[T]:\n",
        "    left: T\n",
        "    right: T\n",
        "enum Choice[T]:\n",
        "    None\n",
        "    Some(value: T)\n",
        "def main() -> Int:\n",
        "    pair := Pair[Int](20, 22)\n",
        "    choice := Choice.Some(42)\n",
        "    return pair.left + pair.right\n",
    );
    let (program, _) = built(source);
    let snapshot = mir_snapshot(&program);
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/phase4c2a-nominal-mir.snap");
    if std::env::var_os("LPP_UPDATE_SNAPSHOTS").is_some() {
        fs::write(&fixture, &snapshot).unwrap();
    }
    assert_eq!(
        snapshot,
        include_str!("snapshots/phase4c2a-nominal-mir.snap"),
    );
}

#[test]
fn compact_nominal_ids_remain_32_bit() {
    assert_eq!(std::mem::size_of::<MirAggregateId>(), 4);
    assert_eq!(std::mem::size_of::<MirFieldId>(), 4);
    assert_eq!(std::mem::size_of::<MirVariantId>(), 4);
}
