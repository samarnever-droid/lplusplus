use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use lpp_hir::{
    ExpressionKind, FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec,
    ResolutionMode, TypeRefKind, lower_package,
};
use lpp_types::{
    PrimitiveType, ShadowInferenceOptions, TraitGoal, TraitSolution, TraitSolverLimits, TypeError,
    TypeKind, infer_hir_package, semantic_snapshot,
};

#[derive(Debug, Default)]
struct MemoryFileSystem {
    files: BTreeMap<PathBuf, String>,
}

impl MemoryFileSystem {
    fn with_files(files: &[(&str, &str)]) -> Self {
        Self {
            files: files
                .iter()
                .map(|(path, source)| (PathBuf::from(path), (*source).to_owned()))
                .collect(),
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

fn lower_single_source(source: &str) -> lpp_hir::HirPackage {
    let filesystem = MemoryFileSystem::with_files(&[("/app/src/main.lpp", source)]);
    let graph = GraphBuilder::new(&filesystem)
        .build(GraphRequest::new(
            "/app/src/main.lpp",
            PackageSpec::new("app", "/app/src"),
        ))
        .unwrap();
    lower_package(&graph, ResolutionMode::Namespaced).unwrap()
}

fn lower_representative_package() -> lpp_hir::HirPackage {
    let filesystem = MemoryFileSystem::with_files(&[
        (
            "/app/src/main.lpp",
            concat!(
                "from util import identity\n",
                "struct Pair:\n",
                "    number: Int\n",
                "    label: Str\n",
                "def combine(input: Int) -> (Int, Str):\n",
                "    values := [input, 2]\n",
                "    pair := (identity[Int](values[0]), \"ok\")\n",
                "    return pair\n",
            ),
        ),
        (
            "/app/src/util.lpp",
            concat!("def identity[T](value: T) -> T:\n", "    return value\n",),
        ),
    ]);
    let graph = GraphBuilder::new(&filesystem)
        .build(GraphRequest::new(
            "/app/src/main.lpp",
            PackageSpec::new("app", "/app/src"),
        ))
        .unwrap();
    lower_package(&graph, ResolutionMode::Namespaced).unwrap()
}

#[test]
fn assigns_canonical_types_to_representative_multi_file_hir() {
    let package = lower_representative_package();
    let output = infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap();
    let int = output.interner.primitive(PrimitiveType::Int);
    let string = output.interner.primitive(PrimitiveType::String);

    assert!(
        package
            .expressions
            .enumerate()
            .all(|(id, _)| output.assignments.expression(id).is_some())
    );
    assert!(
        package
            .locals
            .enumerate()
            .all(|(id, _)| output.assignments.local(id).is_some())
    );
    assert!(
        package
            .type_refs
            .enumerate()
            .all(|(id, _)| output.assignments.type_ref(id).is_some())
    );
    assert!(
        package
            .items
            .enumerate()
            .all(|(id, _)| output.assignments.item(id).is_some())
    );

    let string_literals = package
        .expressions
        .enumerate()
        .filter_map(|(id, expression)| {
            matches!(
                expression.kind,
                ExpressionKind::Literal(lpp_hir::Literal::String { .. })
            )
            .then_some(id)
        });
    for expression in string_literals {
        assert_eq!(output.assignments.expression(expression), Some(string));
    }

    let tuple_expression = package
        .expressions
        .enumerate()
        .find_map(|(id, expression)| {
            matches!(expression.kind, ExpressionKind::Tuple(_)).then_some(id)
        })
        .unwrap();
    let tuple = output.assignments.expression(tuple_expression).unwrap();
    let TypeKind::Tuple(elements) = output.interner.kind(tuple) else {
        panic!("tuple HIR receives a tuple type");
    };
    assert_eq!(output.interner.list(elements), &[int, string]);

    let pair_reference = package
        .type_refs
        .enumerate()
        .find_map(|(id, reference)| matches!(reference.kind, TypeRefKind::Tuple(_)).then_some(id))
        .unwrap();
    assert_eq!(output.assignments.type_ref(pair_reference), Some(tuple));
}

#[test]
fn string_indexing_and_named_aliases_preserve_v1_types() {
    let package = lower_single_source(concat!(
        "type Name = Str\n",
        "def first(value: Name) -> Str:\n",
        "    return value[0]\n",
    ));
    let output = infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap();
    let string = output.interner.primitive(PrimitiveType::String);
    let index = package
        .expressions
        .enumerate()
        .find_map(|(id, expression)| {
            matches!(expression.kind, ExpressionKind::Index { .. }).then_some(id)
        })
        .unwrap();
    assert_eq!(output.assignments.expression(index), Some(string));
}

#[test]
fn cyclic_type_aliases_fail_with_a_bounded_diagnostic() {
    let package = lower_single_source(concat!(
        "type First = Second\n",
        "type Second = First\n",
        "def consume(value: First):\n",
        "    return\n",
    ));
    let error = infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap_err();
    assert!(matches!(error.error, TypeError::TypeAliasCycle { .. }));
}

#[test]
fn semantic_snapshot_matches_the_checked_in_phase3_contract() {
    let package = lower_representative_package();
    let output = infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap();
    let snapshot = semantic_snapshot(&package, &output);
    if std::env::var_os("LPP_UPDATE_SNAPSHOTS").is_some() {
        fs::write(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/phase3-semantic.snap"),
            &snapshot,
        )
        .unwrap();
    }
    assert_eq!(
        snapshot.replace("\r\n", "\n"),
        include_str!("snapshots/phase3-semantic.snap").replace("\r\n", "\n"),
        "semantic snapshot changed; review IDs, types, traits, and instances together",
    );
}

#[test]
fn repeated_shadow_inference_is_structurally_deterministic() {
    let package = lower_representative_package();
    let first = infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap();
    let second = infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap();
    assert_eq!(first, second);
}

#[test]
fn collects_hir_trait_rules_and_concrete_generic_demands_deterministically() {
    let filesystem = MemoryFileSystem::with_files(&[(
        "/app/src/main.lpp",
        concat!(
            "struct Box[T]:\n",
            "    value: T\n",
            "trait Show:\n",
            "    def show(self) -> Int\n",
            "impl[T] Show for Box[T]:\n",
            "    def show(self) -> Int:\n",
            "        return 1\n",
            "impl Show for Box[Int]:\n",
            "    def show(self) -> Int:\n",
            "        return 2\n",
            "def identity[T](value: T) -> T:\n",
            "    return value\n",
            "def wrap[T](value: T) -> Box[T]:\n",
            "    return Box(value)\n",
            "def deep[T](value: T) -> Box[T]:\n",
            "    return wrap(value)\n",
            "def main():\n",
            "    first := identity(1)\n",
            "    second := identity(\"two\")\n",
            "    deep_int := deep(5)\n",
            "    deep_str := deep(\"six\")\n",
        ),
    )]);
    let graph = GraphBuilder::new(&filesystem)
        .build(GraphRequest::new(
            "/app/src/main.lpp",
            PackageSpec::new("app", "/app/src"),
        ))
        .unwrap();
    let package = lower_package(&graph, ResolutionMode::Namespaced).unwrap();
    let mut output = infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap();

    assert!(output.trait_diagnostics.is_empty());
    assert_eq!(output.trait_index.len(), 2);
    assert!(output.instance_diagnostics.is_empty());
    assert_eq!(
        output.instances.records().len(),
        8,
        "deep[T] must discover wrap[T] and Box[T] transitively for Int and Str",
    );
    assert!(
        output
            .instances
            .records()
            .iter()
            .any(|record| record.depth >= 2),
        "transitive generic discovery must advance the bounded fixed point",
    );
    output.instances.finish().unwrap();

    let module = package.modules[0].module;
    let box_definition = match package.names.resolve(module, "Box").unwrap() {
        lpp_hir::BindingTarget::Definition(definition) => definition,
        lpp_hir::BindingTarget::Module(_) => panic!("Box is a definition"),
    };
    let show_definition = match package.names.resolve(module, "Show").unwrap() {
        lpp_hir::BindingTarget::Definition(definition) => definition,
        lpp_hir::BindingTarget::Module(_) => panic!("Show is a definition"),
    };
    let int = output.interner.primitive(PrimitiveType::Int);
    let arguments = output.interner.intern_list(&[int]).unwrap();
    let int_box = output
        .interner
        .intern(TypeKind::Nominal {
            definition: box_definition,
            arguments,
        })
        .unwrap();
    let empty = output.interner.empty_list();
    let solution = output.trait_index.solve(
        &mut output.interner,
        TraitGoal::new(show_definition, int_box, empty),
        TraitSolverLimits::default(),
    );
    let TraitSolution::Unique(selection) = solution else {
        panic!("Box[Int] must select a unique Show implementation");
    };
    assert_eq!(
        output
            .trait_index
            .rule(selection.implementation)
            .unwrap()
            .self_pattern,
        int_box,
        "the concrete impl is more specific",
    );

    let repeated = infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap();
    assert_eq!(output.instances, repeated.instances);
    assert_eq!(output.trait_index.len(), repeated.trait_index.len());
}

#[test]
fn shadow_inference_enforces_a_global_work_bound() {
    let package = lower_representative_package();
    let error = infer_hir_package(
        &package,
        ShadowInferenceOptions {
            work_units: 1,
            ..ShadowInferenceOptions::default()
        },
    )
    .expect_err("representative inference cannot fit in one work unit");
    assert!(matches!(
        error.error,
        lpp_types::TypeError::WorkLimitExceeded { .. }
    ));
}
