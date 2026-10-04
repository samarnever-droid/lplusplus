use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    lower_package,
};
use lpp_mir::{
    Constant, ExecutionValue, InstructionKind, InterpreterLimits, MirBuildErrorKind,
    MirBuildOptions, MirCapacity, MirFunctionId, MirVerificationErrorKind, Operand,
    PlaceProjection, Rvalue, Terminator, build_mir, execute_mir_with_stats, mir_snapshot,
    verify_mir,
};
use lpp_types::{ShadowInferenceOptions, ShadowTypeOutput, TypeInterner, infer_hir_package};

#[derive(Debug)]
struct MemoryFileSystem {
    files: BTreeMap<PathBuf, String>,
}

impl MemoryFileSystem {
    fn new(source: &str) -> Self {
        Self {
            files: BTreeMap::from([(PathBuf::from("/enum-flow/main.lpp"), source.to_owned())]),
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
            "/enum-flow/main.lpp",
            PackageSpec::new("enum-flow", "/enum-flow"),
        ))
        .unwrap();
    let package = lower_package(&graph, ResolutionMode::Namespaced).unwrap();
    let types = infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap();
    (package, types, graph.sources.clone())
}

fn built(source: &str) -> (lpp_mir::MirProgram, TypeInterner) {
    let (package, mut types, sources) = typed(source);
    let program = build_mir(&package, &sources, &mut types, MirBuildOptions::default()).unwrap();
    let errors = verify_mir(&program, &types.interner);
    assert!(errors.is_empty(), "{errors:#?}");
    (program, types.interner)
}

fn last_function(program: &lpp_mir::MirProgram) -> MirFunctionId {
    MirFunctionId::from_raw(u32::try_from(program.function_count() - 1).unwrap())
}

#[test]
fn dense_match_dispatch_binds_every_payload_and_evaluates_subject_once() {
    assert!(std::mem::size_of::<Terminator>() <= 32);
    assert!(std::mem::size_of::<PlaceProjection>() <= 32);
    let source = concat!(
        "enum Event:\n",
        "    Pair(left: Int, right: Int)\n",
        "    Empty\n",
        "    Other\n",
        "def make() -> Event:\n",
        "    return Event.Pair(20, 22)\n",
        "def main() -> Int:\n",
        "    mut answer := 0\n",
        "    match make():\n",
        "        Pair(left, right):\n",
        "            answer = left + right\n",
        "        Pair(unreachable_left, unreachable_right):\n",
        "            answer = 100\n",
        "        _:\n",
        "            answer = 7\n",
        "    return answer\n",
    );
    let (program, types) = built(source);
    let outcome = execute_mir_with_stats(
        &program,
        &types,
        last_function(&program),
        &[],
        InterpreterLimits::default(),
    )
    .unwrap();
    assert_eq!(outcome.value, ExecutionValue::Int(42));
    assert_eq!(outcome.stats.calls, 2, "the match subject call runs once");

    let switches = program
        .blocks()
        .filter_map(|(_, block)| match block.terminator {
            Terminator::SwitchEnum { targets, .. } => Some(targets),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(switches.len(), 1);
    let targets = program.switch_targets(switches[0]);
    assert_eq!(targets.len(), 3);
    assert_eq!(targets[1], targets[2], "wildcard arms share one body block");
    assert_ne!(targets[0], targets[1]);
    assert_eq!(program.switch_target_count(), 3);
    assert_eq!(
        program
            .instructions()
            .filter(|(_, instruction)| matches!(
                instruction.kind,
                InstructionKind::Store {
                    value: Operand::Constant(Constant::Integer(100)),
                    ..
                }
            ))
            .count(),
        1,
        "an unreachable source arm is retained once but never targeted",
    );
    assert_eq!(
        program
            .places()
            .flat_map(|(_, place)| program.place_projections(place))
            .filter(|projection| matches!(projection, PlaceProjection::Downcast(_)))
            .count(),
        4,
        "all bindings in both emitted source arms use checked downcasts",
    );
    let ordinals = program
        .variants()
        .map(|(_, variant)| variant.ordinal)
        .collect::<Vec<_>>();
    assert_eq!(ordinals, vec![0, 1, 2]);
    assert!(mir_snapshot(&program).starts_with("mir-v6\n"));
}

#[test]
fn non_exhaustive_statement_match_routes_uncovered_variants_to_no_effect_join() {
    let source = concat!(
        "enum Choice:\n",
        "    A\n",
        "    B\n",
        "def main() -> Int:\n",
        "    mut answer := 42\n",
        "    value := Choice.B\n",
        "    match value:\n",
        "        A:\n",
        "            answer = 0\n",
        "    return answer\n",
    );
    let (program, types) = built(source);
    let outcome = execute_mir_with_stats(
        &program,
        &types,
        last_function(&program),
        &[],
        InterpreterLimits::default(),
    )
    .unwrap();
    assert_eq!(outcome.value, ExecutionValue::Int(42));
    let targets = program
        .blocks()
        .find_map(|(_, block)| match block.terminator {
            Terminator::SwitchEnum { targets, .. } => Some(program.switch_targets(targets)),
            _ => None,
        })
        .unwrap();
    assert_ne!(targets[0], targets[1]);
}

#[test]
fn match_arms_preserve_nested_break_and_continue_control_flow() {
    let source = concat!(
        "enum Flow:\n",
        "    Take(value: Int)\n",
        "    Skip\n",
        "    Stop\n",
        "def classify(value: Int) -> Flow:\n",
        "    if value == 2:\n",
        "        return Flow.Skip\n",
        "    if value == 4:\n",
        "        return Flow.Stop\n",
        "    return Flow.Take(value)\n",
        "def main() -> Int:\n",
        "    mut total := 0\n",
        "    for value in [1, 2, 3, 4, 5]:\n",
        "        match classify(value):\n",
        "            Take(item):\n",
        "                total += item\n",
        "            Skip:\n",
        "                continue\n",
        "            Stop:\n",
        "                break\n",
        "    return total\n",
    );
    let (program, types) = built(source);
    let outcome = execute_mir_with_stats(
        &program,
        &types,
        last_function(&program),
        &[],
        InterpreterLimits::default(),
    )
    .unwrap();
    assert_eq!(outcome.value, ExecutionValue::Int(4));
    assert_eq!(
        outcome.stats.calls, 5,
        "main plus four exact-once classifiers"
    );
}

#[test]
fn try_success_extracts_payload_and_evaluates_operand_once() {
    let source = concat!(
        "enum Result:\n",
        "    Ok(value: Int)\n",
        "    Err(error: Int)\n",
        "def produce() -> Result:\n",
        "    return Result.Ok(42)\n",
        "def propagate() -> Result:\n",
        "    payload := produce()?\n",
        "    return Result.Ok(payload)\n",
        "def main() -> Int:\n",
        "    result := propagate()\n",
        "    match result:\n",
        "        Ok(value):\n",
        "            return value\n",
        "        Err(error):\n",
        "            return error\n",
        "    return 0\n",
    );
    let (program, types) = built(source);
    let outcome = execute_mir_with_stats(
        &program,
        &types,
        last_function(&program),
        &[],
        InterpreterLimits::default(),
    )
    .unwrap();
    assert_eq!(outcome.value, ExecutionValue::Int(42));
    assert_eq!(
        outcome.stats.calls, 3,
        "main, propagate, and one produce call"
    );
}

#[test]
fn try_propagates_direct_handles_and_rebuilds_only_cross_instance_residuals() {
    let source = concat!(
        "enum Result[T, E]:\n",
        "    Ok(value: T)\n",
        "    Err(error: E)\n",
        "def direct(value: Result[Int, Int]) -> Result[Int, Int]:\n",
        "    payload := value?\n",
        "    return Result.Ok(payload + 1)\n",
        "def convert(value: Result[Int, Int]) -> Result[Bool, Int]:\n",
        "    payload := value?\n",
        "    return Result.Ok(payload == 0)\n",
        "def main() -> Int:\n",
        "    direct_error := direct(Result.Err(20))\n",
        "    converted_error := convert(Result.Err(22))\n",
        "    mut total := 0\n",
        "    match direct_error:\n",
        "        Ok(value):\n",
        "            total = value\n",
        "        Err(error):\n",
        "            total = total + error\n",
        "    match converted_error:\n",
        "        Ok(value):\n",
        "            total = 100\n",
        "        Err(error):\n",
        "            total = total + error\n",
        "    return total\n",
    );
    let (program, types) = built(source);
    let outcome = execute_mir_with_stats(
        &program,
        &types,
        last_function(&program),
        &[],
        InterpreterLimits::default(),
    )
    .unwrap();
    assert_eq!(outcome.value, ExecutionValue::Int(42));
    assert_eq!(
        outcome.stats.heap_nodes, 3,
        "two input residuals plus only the required cross-instance reconstruction",
    );
    assert_eq!(
        program
            .instructions()
            .filter(|(_, instruction)| matches!(
                instruction.kind,
                InstructionKind::Assign {
                    value: Rvalue::ConstructVariant { .. },
                    ..
                }
            ))
            .count(),
        5,
        "four source constructors plus one cross-instance residual reconstruction",
    );
}

#[test]
fn cross_instance_try_maps_every_residual_variant_in_descriptor_order() {
    let source = concat!(
        "enum Outcome[T]:\n",
        "    Ok(value: T)\n",
        "    Error(code: Int)\n",
        "    Cancelled\n",
        "def convert(value: Outcome[Int]) -> Outcome[Bool]:\n",
        "    payload := value?\n",
        "    return Outcome.Ok(payload == 0)\n",
        "def main() -> Int:\n",
        "    result := convert(Outcome.Cancelled)\n",
        "    match result:\n",
        "        Ok(value):\n",
        "            return 0\n",
        "        Error(code):\n",
        "            return code\n",
        "        Cancelled:\n",
        "            return 42\n",
        "    return 1\n",
    );
    let (program, types) = built(source);
    let outcome = execute_mir_with_stats(
        &program,
        &types,
        last_function(&program),
        &[],
        InterpreterLimits::default(),
    )
    .unwrap();
    assert_eq!(outcome.value, ExecutionValue::Int(42));
    assert_eq!(outcome.stats.heap_nodes, 2);
    let target_lengths = program
        .blocks()
        .filter_map(|(_, block)| match block.terminator {
            Terminator::SwitchEnum { targets, .. } => Some(targets.len()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(target_lengths, vec![3, 3]);
}

#[test]
fn enum_mir_rejects_bad_ordinals_downcasts_and_downcast_stores() {
    let source = concat!(
        "enum Pair:\n",
        "    Both(left: Int, right: Int)\n",
        "    Empty\n",
        "def main() -> Int:\n",
        "    value := Pair.Both(20, 22)\n",
        "    match value:\n",
        "        Both(left, right):\n",
        "            return left + right\n",
        "        Empty:\n",
        "            return 0\n",
        "    return 1\n",
    );
    let (program, types) = built(source);

    let mut bad_ordinal = program.clone();
    bad_ordinal
        .variant_mut(lpp_mir::MirVariantId::from_raw(0))
        .unwrap()
        .ordinal = 1;
    assert!(
        verify_mir(&bad_ordinal, &types)
            .iter()
            .any(|error| { error.kind == MirVerificationErrorKind::InvalidAggregateDescriptor })
    );

    let mut bad_downcast = program.clone();
    let place = bad_downcast
        .places()
        .find_map(|(place, value)| value.projections.len().eq(&2).then_some(place))
        .unwrap();
    let projections = bad_downcast.place_projections_mut(place).unwrap();
    projections[0] = PlaceProjection::Downcast(lpp_mir::MirVariantId::from_raw(1));
    assert!(verify_mir(&bad_downcast, &types).iter().any(|error| {
        matches!(
            error.kind,
            MirVerificationErrorKind::InvalidAggregateType(_)
        )
    }));

    let mut missing_downcast = program.clone();
    missing_downcast.place_projections_mut(place).unwrap()[0] =
        PlaceProjection::Downcast(lpp_mir::MirVariantId::from_raw(999_999));
    assert!(verify_mir(&missing_downcast, &types).iter().any(|error| {
        error.kind
            == MirVerificationErrorKind::MissingReference(lpp_mir::MirEntity::Variant(
                lpp_mir::MirVariantId::from_raw(999_999),
            ))
    }));

    let mut bad_store = program.clone();
    let instruction = bad_store
        .instructions()
        .find_map(|(instruction, value)| match value.kind {
            InstructionKind::Assign {
                value: Rvalue::Load(candidate),
                ..
            } if candidate == place => Some(instruction),
            _ => None,
        })
        .unwrap();
    bad_store.instruction_mut(instruction).unwrap().kind = InstructionKind::Store {
        place,
        value: Operand::Constant(Constant::Integer(0)),
    };
    assert!(
        verify_mir(&bad_store, &types)
            .iter()
            .any(|error| error.kind == MirVerificationErrorKind::DowncastStore)
    );
}

#[test]
fn enum_switch_rejects_wrong_table_arity_and_cross_function_targets() {
    let source = concat!(
        "enum Two:\n",
        "    A\n",
        "    B\n",
        "enum Three:\n",
        "    A\n",
        "    B\n",
        "    C\n",
        "def choose_two() -> Int:\n",
        "    value := Two.A\n",
        "    match value:\n",
        "        _:\n",
        "            return 2\n",
        "    return 0\n",
        "def main() -> Int:\n",
        "    value := Three.C\n",
        "    match value:\n",
        "        _:\n",
        "            return 3\n",
        "    return 0\n",
    );
    let (program, types) = built(source);
    let switches = program
        .blocks()
        .filter_map(|(block, value)| match value.terminator {
            Terminator::SwitchEnum {
                subject,
                aggregate,
                targets,
            } => Some((block, subject, aggregate, targets)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(switches.len(), 2);
    let (short_block, subject, aggregate, short_targets) = switches[0];
    let (_, _, _, long_targets) = switches[1];
    assert_eq!(program.switch_targets(short_targets).len(), 2);
    assert_eq!(program.switch_targets(long_targets).len(), 3);

    let mut malformed = program.clone();
    malformed.block_mut(short_block).unwrap().terminator = Terminator::SwitchEnum {
        subject,
        aggregate,
        targets: long_targets,
    };
    let errors = verify_mir(&malformed, &types);
    assert!(
        errors
            .iter()
            .any(|error| { error.kind == MirVerificationErrorKind::InvalidAggregateDescriptor })
    );
    assert!(errors.iter().any(|error| {
        matches!(
            error.kind,
            MirVerificationErrorKind::CrossFunctionReference(_)
        )
    }));
}

#[test]
fn switch_target_capacity_is_independent_and_structured() {
    let source = concat!(
        "enum Choice:\n",
        "    A\n",
        "    B\n",
        "    C\n",
        "def main() -> Int:\n",
        "    value := Choice.A\n",
        "    match value:\n",
        "        _:\n",
        "            return 1\n",
        "    return 0\n",
    );
    let (package, mut types, sources) = typed(source);
    let error = build_mir(&package, &sources, &mut types,
        MirBuildOptions {
            max_switch_targets: 2,
            ..MirBuildOptions::default()
        },
    )
    .unwrap_err();
    assert_eq!(
        error.kind,
        MirBuildErrorKind::Capacity(MirCapacity::SwitchTargets)
    );
}

#[test]
fn phase4c2c_mir_snapshot_is_stable() {
    let source = concat!(
        "enum Result[T, E]:\n",
        "    Ok(value: T)\n",
        "    Err(error: E)\n",
        "def propagate(value: Result[Int, Int]) -> Result[Int, Int]:\n",
        "    payload := value?\n",
        "    return Result.Ok(payload)\n",
        "def main() -> Int:\n",
        "    result := propagate(Result.Err(42))\n",
        "    match result:\n",
        "        Ok(value):\n",
        "            return value\n",
        "        Err(error):\n",
        "            return error\n",
        "    return 0\n",
    );
    let (program, _) = built(source);
    let actual = mir_snapshot(&program);
    let path = Path::new("tests/snapshots/phase4c2c-enum-flow-mir.snap");
    if std::env::var_os("LPP_UPDATE_SNAPSHOTS").is_some() {
        fs::write(path, &actual).unwrap();
    }
    assert_eq!(
        actual,
        include_str!("snapshots/phase4c2c-enum-flow-mir.snap"),
        "Phase 4C2C MIR snapshot changed",
    );
}

fn payload_source(fields: usize) -> String {
    let declarations = (0..fields)
        .map(|index| format!("field{index}: Int"))
        .collect::<Vec<_>>()
        .join(", ");
    let values = (0..fields)
        .map(|index| index.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let bindings = (0..fields)
        .map(|index| format!("value{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "enum Wide:\n    Data({declarations})\ndef main() -> Int:\n    value := Wide.Data({values})\n    match value:\n        Data({bindings}):\n            return value0\n    return 0\n"
    )
}

#[test]
fn payload_binding_has_linear_independent_20_vs_200_growth() {
    let (small, small_types) = built(&payload_source(20));
    let (large, large_types) = built(&payload_source(200));
    assert_eq!(small.field_count(), 20);
    assert_eq!(large.field_count(), 200);
    assert_eq!(small.projection_count(), 40);
    assert_eq!(large.projection_count(), 400);
    assert_eq!(large.instruction_count() - small.instruction_count(), 180);
    assert_eq!(small.switch_target_count(), 1);
    assert_eq!(large.switch_target_count(), 1);
    assert!(verify_mir(&small, &small_types).is_empty());
    assert!(verify_mir(&large, &large_types).is_empty());
}

fn generic_instances_source(instances: usize) -> String {
    let mut source = String::from("enum Cell[T]:\n    Value(value: T)\n    Empty\n");
    for index in 0..instances {
        let mut type_groups = Vec::with_capacity(4);
        let mut value_groups = Vec::with_capacity(4);
        for group in 0..4 {
            let mut types = Vec::with_capacity(4);
            let mut values = Vec::with_capacity(4);
            for offset in 0..4 {
                let bit = group * 4 + offset;
                if index & (1 << bit) == 0 {
                    types.push("Int");
                    values.push("0");
                } else {
                    types.push("Bool");
                    values.push("true");
                }
            }
            type_groups.push(format!("({})", types.join(", ")));
            value_groups.push(format!("({})", values.join(", ")));
        }
        source.push_str(&format!(
            "def make{index}() -> Cell[({})]:\n    return Cell.Value(({}))\n",
            type_groups.join(", "),
            value_groups.join(", "),
        ));
    }
    source.push_str("def main() -> Int:\n    return 42\n");
    source
}

#[test]
fn generic_enum_instances_have_linear_independent_20_vs_200_growth() {
    let (small, small_types) = built(&generic_instances_source(20));
    let (large, large_types) = built(&generic_instances_source(200));
    assert_eq!(small.aggregate_count(), 20);
    assert_eq!(large.aggregate_count(), 200);
    assert_eq!(small.field_count(), 20);
    assert_eq!(large.field_count(), 200);
    assert_eq!(small.variant_count(), 40);
    assert_eq!(large.variant_count(), 400);
    assert_eq!(large.function_count() - small.function_count(), 180);
    assert_eq!(
        large.instruction_count() - small.instruction_count(),
        180 * 6
    );
    assert!(verify_mir(&small, &small_types).is_empty());
    assert!(verify_mir(&large, &large_types).is_empty());
}

fn dense_source(variants: usize) -> String {
    let mut source = String::from("enum Dense:\n");
    for index in 0..variants {
        source.push_str(&format!("    V{index}\n"));
    }
    source.push_str("def main() -> Int:\n    value := Dense.V0\n    match value:\n        _:\n            return 1\n    return 0\n");
    source
}

#[test]
fn dense_dispatch_has_linear_20_vs_200_growth_without_arm_duplication() {
    let (small, small_types) = built(&dense_source(20));
    let (large, large_types) = built(&dense_source(200));
    assert_eq!(small.switch_target_count(), 20);
    assert_eq!(large.switch_target_count(), 200);
    assert_eq!(large.block_count(), small.block_count());
    assert_eq!(large.instruction_count(), small.instruction_count());
    assert_eq!(
        execute_mir_with_stats(
            &large,
            &large_types,
            last_function(&large),
            &[],
            InterpreterLimits::default(),
        )
        .unwrap()
        .value,
        ExecutionValue::Int(1),
    );
    assert!(verify_mir(&small, &small_types).is_empty());

    let small_targets = small
        .blocks()
        .find_map(|(_, block)| match block.terminator {
            Terminator::SwitchEnum { targets, .. } => Some(small.switch_targets(targets)),
            _ => None,
        })
        .unwrap();
    let large_targets = large
        .blocks()
        .find_map(|(_, block)| match block.terminator {
            Terminator::SwitchEnum { targets, .. } => Some(large.switch_targets(targets)),
            _ => None,
        })
        .unwrap();
    assert_eq!(small_targets.iter().collect::<BTreeSet<_>>().len(), 1);
    assert_eq!(large_targets.iter().collect::<BTreeSet<_>>().len(), 1);
}
