use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    lower_package,
};
use lpp_mir::{
    ExecutionValue, InterpreterLimits, MirBuildErrorKind, MirBuildOptions, MirFunctionId,
    build_mir, execute_mir, mir_snapshot, verify_mir,
};
use lpp_types::{
    InstanceKey, InstanceLimits, InstancePlanner, InstanceRequestKind, PrimitiveType,
    ShadowInferenceOptions, ShadowTypeOutput, TypeId, TypeInterner, TypeKind, infer_hir_package,
};

#[derive(Debug)]
struct MemoryFileSystem {
    files: BTreeMap<PathBuf, String>,
}

impl MemoryFileSystem {
    fn new(source: &str) -> Self {
        Self {
            files: BTreeMap::from([(PathBuf::from("/generic/main.lpp"), source.to_owned())]),
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

fn typed(source: &str, options: ShadowInferenceOptions) -> (lpp_hir::HirPackage, ShadowTypeOutput, lpp_common::SourceMap) {
    let filesystem = MemoryFileSystem::new(source);
    let graph = GraphBuilder::new(&filesystem)
        .build(GraphRequest::new(
            "/generic/main.lpp",
            PackageSpec::new("generic", "/generic"),
        ))
        .unwrap();
    let package = lower_package(&graph, ResolutionMode::Namespaced).unwrap();
    let types = infer_hir_package(&package, options).unwrap();
    (package, types, graph.sources.clone())
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

#[test]
fn inferred_and_explicit_demands_emit_concrete_instances_once() {
    let source = concat!(
        "def identity[T](value: T) -> T:\n",
        "    return value\n",
        "def main() -> Int:\n",
        "    first := identity(40)\n",
        "    second := identity[Int](2)\n",
        "    if identity[Bool](false):\n",
        "        return 0\n",
        "    return first + second\n",
    );
    let (package, mut types, sources) = typed(source, ShadowInferenceOptions::default());
    assert!(types.instance_diagnostics.is_empty());
    assert_eq!(types.instances.records().len(), 2);
    let program = build_mir(&package, &sources, &mut types, MirBuildOptions::default()).unwrap();

    assert!(verify_mir(&program, &types.interner).is_empty());
    assert_eq!(
        execute_main(&program, &types.interner),
        ExecutionValue::Int(42)
    );
    assert_eq!(program.function_count(), 3);
    assert_eq!(
        program
            .functions()
            .filter(|(_, function)| function.instance.is_some())
            .count(),
        2,
    );
    assert!(program.functions().all(|(_, function)| {
        let arguments = types.interner.list(function.instance_arguments);
        if function.instance.is_some() {
            arguments.len() == 1
        } else {
            arguments.is_empty()
        }
    }));
    assert_eq!(
        program
            .functions()
            .filter(|(_, function)| function.instance.is_none())
            .count(),
        1,
        "the generic template must not become executable MIR",
    );
    let local_sets = program
        .functions()
        .map(|(_, function)| {
            program
                .function_locals(function)
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
        })
        .collect::<Vec<_>>();
    for left in 0..local_sets.len() {
        for right in left + 1..local_sets.len() {
            assert!(local_sets[left].is_disjoint(&local_sets[right]));
        }
    }

    let mut duplicate = program.clone();
    let instances = duplicate
        .functions()
        .filter_map(|(id, function)| function.instance.map(|instance| (id, instance)))
        .collect::<Vec<_>>();
    duplicate.function_mut(instances[1].0).unwrap().instance = Some(instances[0].1);
    assert!(verify_mir(&duplicate, &types.interner).iter().any(|error| {
        matches!(
            error.kind,
            lpp_mir::MirVerificationErrorKind::DuplicateInstance(_)
        ) && error.code() == "E4101"
    }));

    let mut missing_arguments = program.clone();
    missing_arguments
        .function_mut(instances[0].0)
        .unwrap()
        .instance_arguments = types.interner.empty_list();
    assert!(
        verify_mir(&missing_arguments, &types.interner)
            .iter()
            .any(|error| {
                error.kind == lpp_mir::MirVerificationErrorKind::InvalidInstanceArguments
                    && error.code() == "E4101"
            })
    );

    let mut spurious_arguments = program.clone();
    spurious_arguments
        .function_mut(MirFunctionId::from_raw(0))
        .unwrap()
        .instance_arguments = program.function(instances[0].0).unwrap().instance_arguments;
    assert!(
        verify_mir(&spurious_arguments, &types.interner)
            .iter()
            .any(|error| {
                error.kind == lpp_mir::MirVerificationErrorKind::InvalidInstanceArguments
                    && error.code() == "E4101"
            })
    );

    let error = build_mir(&package, &sources, &mut types,
        MirBuildOptions {
            max_functions: 2,
            ..MirBuildOptions::default()
        },
    )
    .unwrap_err();
    assert_eq!(
        error.kind,
        MirBuildErrorKind::Capacity(lpp_mir::MirCapacity::Functions),
    );
    assert_eq!(error.code(), "E4001");
}

#[test]
fn nested_and_recursive_generic_dependencies_execute_deterministically() {
    let source = concat!(
        "def identity[T](value: T) -> T:\n",
        "    return value\n",
        "def relay[T](value: T) -> T:\n",
        "    pair := (value, value)\n",
        "    values := [value]\n",
        "    return identity[T](value)\n",
        "def recur[T](value: T, again: Bool) -> T:\n",
        "    if again:\n",
        "        return recur(value, false)\n",
        "    return relay(value)\n",
        "def main() -> Int:\n",
        "    return recur[Int](42, true)\n",
    );
    let (package, mut first_types, sources) = typed(source, ShadowInferenceOptions::default());
    let first = build_mir(&package, &sources, &mut first_types, MirBuildOptions::default()).unwrap();
    let repeated = build_mir(&package, &sources, &mut first_types, MirBuildOptions::default()).unwrap();
    let (package, mut second_types, sources) = typed(source, ShadowInferenceOptions::default());
    let second = build_mir(&package, &sources, &mut second_types, MirBuildOptions::default()).unwrap();

    assert_eq!(first, repeated);
    assert_eq!(first, second);
    assert_eq!(mir_snapshot(&first), mir_snapshot(&second));
    assert!(verify_mir(&first, &first_types.interner).is_empty());
    assert_eq!(
        execute_main(&first, &first_types.interner),
        ExecutionValue::Int(42)
    );
    assert_eq!(first.function_count(), 4);
    assert_eq!(
        first
            .functions()
            .filter(|(_, function)| function.instance.is_some())
            .count(),
        3,
    );
    for (_, function) in first.functions() {
        assert!(!contains_generic(function.ty, &first_types.interner));
        assert!(!contains_generic(
            function.return_type,
            &first_types.interner,
        ));
        for local in first.function_locals(function) {
            assert!(!contains_generic(
                first.local(*local).unwrap().ty,
                &first_types.interner,
            ));
        }
    }
}

fn contains_generic(root: TypeId, types: &TypeInterner) -> bool {
    let mut pending = vec![root];
    let mut visited = BTreeSet::new();
    while let Some(ty) = pending.pop() {
        if !visited.insert(ty) {
            continue;
        }
        match types.kind(ty) {
            TypeKind::GenericParameter(_) => return true,
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
            | TypeKind::Never
            | TypeKind::Primitive(_)
            | TypeKind::BoundVariable(_)
            | TypeKind::InferenceVariable(_)
            | TypeKind::UnresolvedName { .. } => {}
        }
    }
    false
}

#[test]
fn optional_records_are_not_executable_and_required_arity_is_validated() {
    let source = concat!(
        "def identity[T](value: T) -> T:\n",
        "    return value\n",
        "def main() -> Int:\n",
        "    return 1\n",
    );
    let (package, mut optional_types, sources) = typed(source, ShadowInferenceOptions::default());
    let int = optional_types.interner.primitive(PrimitiveType::Int);
    let arguments = optional_types.interner.intern_list(&[int]).unwrap();
    let key = InstanceKey::new(lpp_hir::HirItemId::from_raw(0), arguments);
    let mut optional = InstancePlanner::new(InstanceLimits::default());
    optional
        .request_root(key, None, InstanceRequestKind::OptionalSpecialization)
        .unwrap();
    let work = optional.pop_next().unwrap().unwrap();
    optional.complete(work.id).unwrap();
    optional_types.instances = optional;

    let program = build_mir(&package, &sources, &mut optional_types, MirBuildOptions::default()).unwrap();
    assert_eq!(program.function_count(), 1);
    assert!(
        program
            .functions()
            .all(|(_, function)| function.instance.is_none())
    );
    assert!(verify_mir(&program, &optional_types.interner).is_empty());

    let (package, mut malformed_types, sources) = typed(source, ShadowInferenceOptions::default());
    let key = InstanceKey::new(
        lpp_hir::HirItemId::from_raw(0),
        malformed_types.interner.empty_list(),
    );
    let mut malformed = InstancePlanner::new(InstanceLimits::default());
    malformed
        .request_root(key, None, InstanceRequestKind::Required)
        .unwrap();
    let work = malformed.pop_next().unwrap().unwrap();
    malformed.complete(work.id).unwrap();
    malformed_types.instances = malformed;

    let error = build_mir(&package, &sources, &mut malformed_types, MirBuildOptions::default()).unwrap_err();
    assert_eq!(
        error.kind,
        MirBuildErrorKind::InvalidInstanceArity {
            instance: work.id,
            expected: 1,
            actual: 0,
        },
    );
    assert_eq!(error.code(), "E4006");
}

#[test]
fn invalid_instance_plans_and_type_work_exhaustion_are_structured() {
    let source = concat!(
        "def identity[T](value: T) -> T:\n",
        "    return value\n",
        "def main() -> Int:\n",
        "    return identity(1)\n",
    );
    let (package, mut limited_types, sources) = typed(
        source,
        ShadowInferenceOptions {
            instance_limits: InstanceLimits {
                max_instances_global: 0,
                ..InstanceLimits::default()
            },
            ..ShadowInferenceOptions::default()
        },
    );
    assert!(!limited_types.instance_diagnostics.is_empty());
    let error = build_mir(&package, &sources, &mut limited_types, MirBuildOptions::default()).unwrap_err();
    assert!(matches!(
        error.kind,
        MirBuildErrorKind::InvalidInstancePlan { diagnostics: 1.. }
    ));
    assert_eq!(error.code(), "E4006");

    let (package, mut types, sources) = typed(source, ShadowInferenceOptions::default());
    let error = build_mir(&package, &sources, &mut types,
        MirBuildOptions {
            max_type_work: 0,
            ..MirBuildOptions::default()
        },
    )
    .unwrap_err();
    assert_eq!(
        error.kind,
        MirBuildErrorKind::Capacity(lpp_mir::MirCapacity::TypeWork),
    );
    assert_eq!(error.code(), "E4001");

    let (package, mut types, sources) = typed(source, ShadowInferenceOptions::default());
    let error = build_mir(&package, &sources, &mut types,
        MirBuildOptions {
            max_type_depth: 0,
            ..MirBuildOptions::default()
        },
    )
    .unwrap_err();
    assert_eq!(
        error.kind,
        MirBuildErrorKind::Capacity(lpp_mir::MirCapacity::TypeDepth),
    );
    assert_eq!(error.code(), "E4001");
}

#[test]
fn repeated_calls_do_not_duplicate_generic_function_bodies() {
    let small_source = repeated_identity_calls(20);
    let large_source = repeated_identity_calls(200);
    let (small_package, mut small_types, small_sources) = typed(&small_source, ShadowInferenceOptions::default());
    let (large_package, mut large_types, large_sources) = typed(&large_source, ShadowInferenceOptions::default());
    assert_eq!(small_types.instances.records().len(), 1);
    assert_eq!(large_types.instances.records().len(), 1);

    let small = build_mir(&small_package, &small_sources, &mut small_types, MirBuildOptions::default()).unwrap();
    let large = build_mir(&large_package, &large_sources, &mut large_types, MirBuildOptions::default()).unwrap();
    assert_eq!(small.function_count(), 2);
    assert_eq!(large.function_count(), 2);
    assert_eq!(
        small
            .functions()
            .filter(|(_, function)| function.instance.is_some())
            .count(),
        1,
    );
    assert_eq!(
        large
            .functions()
            .filter(|(_, function)| function.instance.is_some())
            .count(),
        1,
    );
}

fn repeated_identity_calls(count: usize) -> String {
    let mut source =
        String::from("def identity[T](value: T) -> T:\n    return value\ndef main() -> Int:\n");
    for index in 0..count {
        source.push_str(&format!("    value_{index:04} := identity({index})\n"));
    }
    source.push_str(&format!("    return value_{:04}\n", count - 1));
    source
}
