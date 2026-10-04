use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    lower_package,
};
use lpp_types::{
    EnumMatchArmFact, EnumMatchFact, EnumTryFact, PrimitiveType, ShadowInferenceOptions, TypeError,
    TypeKind, infer_hir_package, semantic_snapshot,
};

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

fn lower(source: &str) -> lpp_hir::HirPackage {
    let filesystem = MemoryFileSystem::new(source);
    let graph = GraphBuilder::new(&filesystem)
        .build(GraphRequest::new(
            "/enum-flow/main.lpp",
            PackageSpec::new("enum-flow", "/enum-flow"),
        ))
        .unwrap();
    lower_package(&graph, ResolutionMode::Namespaced).unwrap()
}

#[test]
fn enum_flow_facts_remain_compact_inline_records() {
    assert!(std::mem::size_of::<EnumMatchFact>() <= 32);
    assert!(std::mem::size_of::<EnumMatchArmFact>() <= 32);
    assert!(std::mem::size_of::<EnumTryFact>() <= 48);
    assert!(std::mem::size_of::<Option<EnumMatchFact>>() <= 36);
    assert!(std::mem::size_of::<Option<EnumMatchArmFact>>() <= 36);
    assert!(std::mem::size_of::<Option<EnumTryFact>>() <= 52);
}

#[test]
fn match_facts_resolve_all_payloads_and_preserve_first_arm_precedence() {
    let package = lower(concat!(
        "enum Event:\n",
        "    Pair(left: Int, right: Int)\n",
        "    Empty\n",
        "def inspect(event: Event) -> Int:\n",
        "    match event:\n",
        "        Event.Pair(left, right):\n",
        "            return left + right\n",
        "        Pair(ignored_left, ignored_right):\n",
        "            return 0\n",
        "        _:\n",
        "            return 7\n",
        "    return 9\n",
    ));
    let first = infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap();
    let second = infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap();
    assert_eq!(first, second);

    let matches = first.enum_flow.matches().collect::<Vec<_>>();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].1.covered_variants, 1);
    assert_eq!(matches[0].1.total_variants, 2);
    assert!(matches[0].1.exhaustive());

    let arms = first.enum_flow.arms().collect::<Vec<_>>();
    assert_eq!(arms.len(), 3);
    assert!(matches!(
        arms[0].1,
        EnumMatchArmFact::Variant {
            reachable: true,
            fields,
            bindings,
            ..
        } if fields.len() == 2 && bindings.len() == 2
    ));
    assert!(matches!(
        arms[1].1,
        EnumMatchArmFact::Variant {
            reachable: false,
            ..
        }
    ));
    assert!(matches!(
        arms[2].1,
        EnumMatchArmFact::Wildcard { reachable: true }
    ));
    assert!(semantic_snapshot(&package, &first).contains("semantic-v4\n"));
}

#[test]
fn try_facts_record_direct_and_cross_instance_residual_modes() {
    let package = lower(concat!(
        "enum Result[T, E]:\n",
        "    Ok(value: T)\n",
        "    Err(error: E)\n",
        "def direct(value: Result[Int, Int]) -> Result[Int, Int]:\n",
        "    payload := value?\n",
        "    return Result.Ok(payload)\n",
        "def convert(value: Result[Int, Int]) -> Result[Bool, Int]:\n",
        "    payload := value?\n",
        "    return Result.Ok(payload == 0)\n",
    ));
    let output = infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap();
    let facts = output.enum_flow.tries().collect::<Vec<_>>();
    assert_eq!(facts.len(), 2);
    assert!(facts[0].1.direct_residual);
    assert!(!facts[1].1.direct_residual);
    assert_eq!(facts[0].1.success, facts[1].1.success);
    assert_eq!(facts[0].1.success_field, facts[1].1.success_field);
    assert_eq!(facts[0].1.variants.len(), 2);
}

#[test]
fn enum_flow_fact_types_normalize_after_later_arm_constraints() {
    let package = lower(concat!(
        "enum Choice[T]:\n",
        "    Some(value: T)\n",
        "    None\n",
        "def inspect() -> Int:\n",
        "    value := Choice.None\n",
        "    match value:\n",
        "        Some(payload):\n",
        "            return payload + 1\n",
        "        None:\n",
        "            return 42\n",
        "    return 0\n",
    ));
    let output = infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap();
    let fact = output.enum_flow.matches().next().unwrap().1;
    let TypeKind::Nominal { arguments, .. } = output.interner.kind(fact.subject_type) else {
        panic!("match facts retain a normalized nominal enum type")
    };
    assert_eq!(
        output.interner.list(arguments),
        [output.interner.primitive(PrimitiveType::Int)]
    );
}

#[test]
fn phase4c2c_semantic_snapshot_is_stable() {
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
    let package = lower(source);
    let output = infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap();
    let actual = semantic_snapshot(&package, &output);
    let path = Path::new("tests/snapshots/phase4c2c-enum-flow-semantic.snap");
    if std::env::var_os("LPP_UPDATE_SNAPSHOTS").is_some() {
        fs::write(path, &actual).unwrap();
    }
    assert_eq!(
        actual.replace("\r\n", "\n"),
        include_str!("snapshots/phase4c2c-enum-flow-semantic.snap").replace("\r\n", "\n"),
        "Phase 4C2C semantic snapshot changed",
    );
}

#[test]
fn invalid_match_and_try_shapes_are_structured() {
    let bad_match = lower(concat!(
        "enum Pair:\n",
        "    Both(left: Int, right: Int)\n",
        "def inspect(value: Pair) -> Int:\n",
        "    match value:\n",
        "        Both(only):\n",
        "            return only\n",
        "    return 0\n",
    ));
    let error = infer_hir_package(&bad_match, ShadowInferenceOptions::default()).unwrap_err();
    assert!(matches!(error.error, TypeError::MatchBindingArity { .. }));

    let bad_try = lower(concat!(
        "enum Result:\n",
        "    Ok\n",
        "    Err(code: Int)\n",
        "def inspect(value: Result) -> Result:\n",
        "    ignored := value?\n",
        "    return Result.Err(ignored)\n",
    ));
    let error = infer_hir_package(&bad_try, ShadowInferenceOptions::default()).unwrap_err();
    assert_eq!(error.error, TypeError::TrySuccessPayloadArity { actual: 0 });

    let wrong_qualifier = lower(concat!(
        "enum First:\n",
        "    Item\n",
        "enum Second:\n",
        "    Item\n",
        "def inspect(value: First) -> Int:\n",
        "    match value:\n",
        "        Second.Item:\n",
        "            return 1\n",
        "    return 0\n",
    ));
    let error = infer_hir_package(&wrong_qualifier, ShadowInferenceOptions::default()).unwrap_err();
    assert_eq!(error.error, TypeError::InvalidMatchPattern);

    let scalar_match = lower(concat!(
        "def inspect(value: Int) -> Int:\n",
        "    match value:\n",
        "        _:\n",
        "            return 1\n",
        "    return 0\n",
    ));
    let error = infer_hir_package(&scalar_match, ShadowInferenceOptions::default()).unwrap_err();
    assert!(matches!(error.error, TypeError::InvalidMatchSubject { .. }));

    let residual_mismatch = lower(concat!(
        "enum Result[T, E]:\n",
        "    Ok(value: T)\n",
        "    Err(error: E)\n",
        "def convert(value: Result[Int, Int]) -> Result[Bool, Bool]:\n",
        "    payload := value?\n",
        "    return Result.Ok(payload == 0)\n",
    ));
    let error =
        infer_hir_package(&residual_mismatch, ShadowInferenceOptions::default()).unwrap_err();
    assert!(matches!(error.error, TypeError::InvalidTryResidual { .. }));

    let scalar_try = lower(concat!(
        "def inspect(value: Int) -> Int:\n",
        "    return value?\n",
    ));
    let error = infer_hir_package(&scalar_try, ShadowInferenceOptions::default()).unwrap_err();
    assert!(matches!(error.error, TypeError::InvalidTryCarrier { .. }));
}
