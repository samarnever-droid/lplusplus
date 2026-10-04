use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    lower_package,
};
use lpp_types::{
    AggregateConstructor, AggregateExpressionFact, PlaceExpressionFact, PlaceStatementFact,
    PrimitiveType, ShadowInferenceOptions, TypeError, infer_hir_package, semantic_snapshot,
};

#[derive(Debug)]
struct MemoryFileSystem {
    files: BTreeMap<PathBuf, String>,
}

impl MemoryFileSystem {
    fn new(source: &str) -> Self {
        Self {
            files: BTreeMap::from([(PathBuf::from("/aggregate/main.lpp"), source.to_owned())]),
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
            "/aggregate/main.lpp",
            PackageSpec::new("aggregate", "/aggregate"),
        ))
        .unwrap();
    lower_package(&graph, ResolutionMode::Namespaced).unwrap()
}

#[test]
fn resolves_struct_fields_and_enum_variant_constructors_deterministically() {
    let source = concat!(
        "struct Pair[T]:\n",
        "    first: T\n",
        "    second: T\n",
        "enum Choice[T]:\n",
        "    Some(value: T)\n",
        "    None\n",
        "def main() -> Int:\n",
        "    pair := Pair[Int](20, 22)\n",
        "    choice := Choice.Some(42)\n",
        "    return pair.first + pair.second\n",
    );
    let package = lower(source);
    let first = infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap();
    let second = infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap();
    assert_eq!(first, second);

    let facts = first.aggregates.expressions().collect::<Vec<_>>();
    assert_eq!(
        facts
            .iter()
            .filter(|(_, fact)| matches!(
                fact,
                AggregateExpressionFact::Constructor(AggregateConstructor::Struct { .. })
            ))
            .count(),
        1,
    );
    assert_eq!(
        facts
            .iter()
            .filter(|(_, fact)| matches!(
                fact,
                AggregateExpressionFact::Constructor(AggregateConstructor::EnumVariant { .. })
            ))
            .count(),
        1,
    );
    assert_eq!(
        facts
            .iter()
            .filter(|(_, fact)| matches!(fact, AggregateExpressionFact::FieldProjection { .. }))
            .count(),
        2,
    );

    let int = first.interner.primitive(PrimitiveType::Int);
    for (expression, fact) in facts {
        if matches!(fact, AggregateExpressionFact::FieldProjection { .. }) {
            assert_eq!(first.assignments.expression(expression), Some(int));
        }
    }
    assert!(first.instance_diagnostics.is_empty());
    assert_eq!(first.instances.records().len(), 2);
}

#[test]
fn nominal_lookup_failures_and_constructor_arity_are_structured() {
    let unknown_field = lower(concat!(
        "struct Point:\n",
        "    x: Int\n",
        "def main() -> Int:\n",
        "    point := Point(1)\n",
        "    return point.y\n",
    ));
    let error = infer_hir_package(&unknown_field, ShadowInferenceOptions::default()).unwrap_err();
    assert!(matches!(error.error, TypeError::UnknownField { .. }));

    let unknown_variant = lower(concat!(
        "enum Choice:\n",
        "    Some(value: Int)\n",
        "def main() -> Choice:\n",
        "    return Choice.None\n",
    ));
    let error = infer_hir_package(&unknown_variant, ShadowInferenceOptions::default()).unwrap_err();
    assert!(matches!(error.error, TypeError::UnknownVariant { .. }));

    let wrong_arity = lower(concat!(
        "struct Point:\n",
        "    x: Int\n",
        "    y: Int\n",
        "def main() -> Point:\n",
        "    return Point(1)\n",
    ));
    let error = infer_hir_package(&wrong_arity, ShadowInferenceOptions::default()).unwrap_err();
    assert_eq!(
        error.error,
        TypeError::ArityMismatch {
            expected: 1,
            actual: 2,
        },
    );
}

#[test]
fn place_facts_cover_projection_assignment_destructuring_and_iteration_deterministically() {
    let source = concat!(
        "def main() -> Int:\n",
        "    pair := (1, 2)\n",
        "    (left, right) := pair\n",
        "    mut values := [left, right]\n",
        "    mut index := 0\n",
        "    values[index] += pair.0\n",
        "    for value in values:\n",
        "        index += value\n",
        "    return pair[1] + values[0] + index\n",
    );
    let package = lower(source);
    let first = infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap();
    let second = infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap();
    assert_eq!(first.places, second.places);

    let expression_facts = first
        .places
        .expressions()
        .map(|(_, fact)| fact)
        .collect::<Vec<_>>();
    assert!(expression_facts.contains(&PlaceExpressionFact::TupleField { index: 0 }));
    assert!(expression_facts.contains(&PlaceExpressionFact::TupleField { index: 1 }));
    assert!(expression_facts.contains(&PlaceExpressionFact::ListIndex));

    let statement_facts = first
        .places
        .statements()
        .map(|(_, fact)| fact)
        .collect::<Vec<_>>();
    assert!(
        statement_facts
            .iter()
            .any(|fact| matches!(fact, PlaceStatementFact::TupleDestructure { arity: 2 }))
    );
    assert!(
        statement_facts
            .iter()
            .any(|fact| matches!(fact, PlaceStatementFact::Assignment { augmented: Some(_) }))
    );
    assert!(statement_facts.contains(&PlaceStatementFact::ListIteration));

    let first_snapshot = semantic_snapshot(&package, &first);
    let second_snapshot = semantic_snapshot(&package, &second);
    assert_eq!(first_snapshot, second_snapshot);
    assert!(first_snapshot.contains("semantic-v4\n"));
    assert!(first_snapshot.contains("place_facts\n"));
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots/phase4c2b-places-semantic.snap");
    if std::env::var_os("LPP_UPDATE_SNAPSHOTS").is_some() {
        fs::write(&fixture, &first_snapshot).unwrap();
    }
    assert_eq!(
        first_snapshot.replace("\r\n", "\n"),
        include_str!("snapshots/phase4c2b-places-semantic.snap").replace("\r\n", "\n")
    );
}
