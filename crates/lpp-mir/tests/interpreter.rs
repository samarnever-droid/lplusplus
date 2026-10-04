use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    lower_package,
};
use lpp_mir::{
    Constant, ExecutionValue, InterpreterErrorKind, InterpreterLimit, InterpreterLimits,
    MirBuildOptions, MirFunctionId, Operand, Terminator, build_mir, execute_mir,
    execute_mir_with_stats,
};
use lpp_types::{ShadowInferenceOptions, TypeInterner, infer_hir_package};

#[derive(Debug)]
struct MemoryFileSystem {
    files: BTreeMap<PathBuf, String>,
}

impl MemoryFileSystem {
    fn new(source: &str) -> Self {
        Self {
            files: BTreeMap::from([(PathBuf::from("/execute/main.lpp"), source.to_owned())]),
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

fn executable(source: &str) -> (lpp_mir::MirProgram, TypeInterner) {
    let filesystem = MemoryFileSystem::new(source);
    let graph = GraphBuilder::new(&filesystem)
        .build(GraphRequest::new(
            "/execute/main.lpp",
            PackageSpec::new("execute", "/execute"),
        ))
        .unwrap();
    let package = lower_package(&graph, ResolutionMode::Namespaced).unwrap();
    let mut types = infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap();
    let program = build_mir(&package, &graph.sources, &mut types, MirBuildOptions::default()).unwrap();
    (program, types.interner)
}

fn last_function(program: &lpp_mir::MirProgram) -> MirFunctionId {
    MirFunctionId::from_raw(u32::try_from(program.function_count() - 1).unwrap())
}

fn execute_last(source: &str) -> ExecutionValue {
    let (program, types) = executable(source);
    execute_mir(
        &program,
        &types,
        last_function(&program),
        &[],
        InterpreterLimits::default(),
    )
    .unwrap()
}

#[test]
fn executes_calls_branches_loops_and_integer_operations_deterministically() {
    let source = concat!(
        "def choose(value: Int, flag: Bool) -> Int:\n",
        "    mut total := value + 1\n",
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
        "    return choose(1, true)\n",
    );
    assert_eq!(execute_last(source), ExecutionValue::Int(8));
    assert_eq!(execute_last(source), ExecutionValue::Int(8));
}

#[test]
fn executes_float_tuple_and_list_values() {
    assert_eq!(
        execute_last("def main() -> Float:\n    return 1.5 + 2.25\n"),
        ExecutionValue::FloatBits(3.75_f64.to_bits()),
    );
    assert_eq!(
        execute_last("def main() -> (Int, Bool):\n    return (3, true)\n"),
        ExecutionValue::Tuple(vec![ExecutionValue::Int(3), ExecutionValue::Bool(true)]),
    );
    assert_eq!(
        execute_last("def main() -> List[Int]:\n    return [1, 2, 3]\n"),
        ExecutionValue::List(vec![
            ExecutionValue::Int(1),
            ExecutionValue::Int(2),
            ExecutionValue::Int(3),
        ]),
    );
}

#[test]
fn equivalent_expression_trees_have_the_same_observable_result() {
    let left = execute_last("def main() -> Int:\n    return (1 + 2) + 3\n");
    let right = execute_last("def main() -> Int:\n    return 1 + (2 + 3)\n");
    assert_eq!(left, ExecutionValue::Int(6));
    assert_eq!(right, left);
}

#[test]
fn recursive_calls_and_infinite_loops_hit_explicit_limits() {
    let (program, types) = executable(concat!(
        "def recurse(value: Int) -> Int:\n",
        "    return recurse(value)\n",
    ));
    let error = execute_mir(
        &program,
        &types,
        MirFunctionId::from_raw(0),
        &[ExecutionValue::Int(1)],
        InterpreterLimits {
            max_call_depth: 4,
            ..InterpreterLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(
        error.kind,
        InterpreterErrorKind::LimitExceeded(InterpreterLimit::CallDepth),
    );
    assert_eq!(error.code(), "E4302");

    let (program, types) = executable(concat!(
        "def main():\n",
        "    while true:\n",
        "        continue\n",
    ));
    let error = execute_mir(
        &program,
        &types,
        MirFunctionId::from_raw(0),
        &[],
        InterpreterLimits {
            max_steps: 20,
            ..InterpreterLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(
        error.kind,
        InterpreterErrorKind::LimitExceeded(InterpreterLimit::Steps),
    );
    assert!(error.block.is_some());
}

#[test]
fn interpreter_work_counts_scale_linearly() {
    let small_source = generated_bindings(100);
    let large_source = generated_bindings(1_000);
    let (small_program, small_types) = executable(&small_source);
    let (large_program, large_types) = executable(&large_source);
    let small = execute_mir_with_stats(
        &small_program,
        &small_types,
        MirFunctionId::from_raw(0),
        &[],
        InterpreterLimits::default(),
    )
    .unwrap();
    let large = execute_mir_with_stats(
        &large_program,
        &large_types,
        MirFunctionId::from_raw(0),
        &[],
        InterpreterLimits::default(),
    )
    .unwrap();
    assert_eq!(small.value, ExecutionValue::Int(100));
    assert_eq!(large.value, ExecutionValue::Int(1_000));
    assert_eq!(small.stats.steps, 201);
    assert_eq!(large.stats.steps, 2_001);
    assert_eq!(small.stats.calls, 1);
    assert_eq!(large.stats.calls, 1);
    assert!(large.stats.steps <= small.stats.steps * 10);
}

fn generated_bindings(count: usize) -> String {
    let mut source = String::from("def main() -> Int:\n");
    for index in 0..count {
        source.push_str(&format!("    value_{index:04} := {index} + 1\n"));
    }
    source.push_str(&format!("    return value_{:04}\n", count - 1));
    source
}

#[test]
fn aggregate_growth_and_invalid_mir_are_rejected_before_unbounded_execution() {
    let (program, types) = executable("def main() -> List[Int]:\n    return [1, 2]\n");
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
    assert!(error.block.is_some());

    let (program, types) = executable(concat!(
        "def identity(values: List[Int]) -> List[Int]:\n",
        "    return values\n",
    ));
    let error = execute_mir(
        &program,
        &types,
        MirFunctionId::from_raw(0),
        &[ExecutionValue::List(vec![
            ExecutionValue::Int(1),
            ExecutionValue::Int(2),
        ])],
        InterpreterLimits {
            max_aggregate_elements: 3,
            ..InterpreterLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(
        error.kind,
        InterpreterErrorKind::LimitExceeded(InterpreterLimit::AggregateElements),
    );
    assert!(error.block.is_some());

    let (mut program, types) = executable("def main() -> Int:\n    return 1\n");
    let block = program.blocks().next().unwrap().0;
    program.block_mut(block).unwrap().terminator =
        Terminator::Return(Some(Operand::Constant(Constant::Bool(false))));
    let error = execute_mir(
        &program,
        &types,
        MirFunctionId::from_raw(0),
        &[],
        InterpreterLimits::default(),
    )
    .unwrap_err();
    assert!(matches!(
        error.kind,
        InterpreterErrorKind::InvalidMir { diagnostics: 1.. }
    ));
    assert_eq!(error.code(), "E4301");
}
