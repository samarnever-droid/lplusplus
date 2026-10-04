use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use lpp::lexer::Lexer;
use lpp::monomorph::Monomorphizer;
use lpp::parser::Parser;
use lpp::semantic::Resolver;
use lpp::typecheck::TypeChecker;
use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    lower_package,
};
use lpp_types::{ShadowInferenceOptions, TypeError, infer_hir_package};

const POSITIVE_CORPUS: &[&str] = &[
    "tests/arith.lpp",
    "tests/bool_test.lpp",
    "tests/branches.lpp",
    "tests/closure_test.lpp",
    "tests/fib.lpp",
    "tests/float_test.lpp",
    "tests/for_range.lpp",
    "tests/generic_trait_impls.lpp",
    "tests/generics_full.lpp",
    "tests/generics_turbofish.lpp",
    "tests/list_safety.lpp",
    "tests/loop.lpp",
    "tests/nested_calls.lpp",
    "tests/test_augmented_assign.lpp",
    "tests/test_break_continue.lpp",
    "tests/test_const.lpp",
    "tests/test_index.lpp",
    "tests/test_list_literal.lpp",
    "tests/test_logical.lpp",
    "tests/test_struct_constructor.lpp",
    "tests/test_traits.lpp",
    "tests/test_type_alias.lpp",
    "tests/test_unary.lpp",
    "tests/tuple_scalars.lpp",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiagnosticClass {
    TypeMismatch,
}

#[derive(Debug)]
struct LegacyFailure {
    stage: &'static str,
    message: String,
}

#[derive(Debug, Default)]
struct MemoryFileSystem {
    files: BTreeMap<PathBuf, String>,
}

impl MemoryFileSystem {
    fn with_source(source: &str) -> Self {
        Self {
            files: BTreeMap::from([(PathBuf::from("/compat/main.lpp"), source.to_owned())]),
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
            .ok_or_else(|| FileSystemError::new("read", path, "path not found"))
    }
}

#[test]
fn representative_v1_type_acceptance_matches_the_rewrite() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut failures = Vec::new();
    for relative in POSITIVE_CORPUS {
        let source = fs::read_to_string(root.join(relative)).unwrap();
        if let Err(error) = legacy_check(&source) {
            failures.push(format!("legacy rejected {relative}: {error:?}"));
            continue;
        }
        if let Err(error) = rewrite_check(&source) {
            failures.push(format!("rewrite rejected {relative}: {error:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "Phase 3 positive compatibility drift:\n{}",
        failures.join("\n"),
    );
}

#[test]
fn representative_v1_type_diagnostics_match_by_stable_class() {
    let cases = [
        (
            "return-type-mismatch",
            "def wrong() -> Int:\n    return \"wrong\"\n",
            DiagnosticClass::TypeMismatch,
        ),
        (
            "generic-return-mismatch",
            concat!(
                "def wrong[T](value: T) -> T:\n",
                "    return \"wrong\"\n",
                "def main():\n",
                "    value := wrong[Int](1)\n",
            ),
            DiagnosticClass::TypeMismatch,
        ),
        (
            "nominal-return-mismatch",
            concat!(
                "struct Box:\n",
                "    value: Int\n",
                "def wrong() -> Box:\n",
                "    return \"wrong\"\n",
            ),
            DiagnosticClass::TypeMismatch,
        ),
    ];
    for (name, source, expected) in cases {
        let legacy = legacy_check(source).expect_err("legacy must reject negative case");
        let rewrite = rewrite_check(source).expect_err("rewrite must reject negative case");
        assert_eq!(
            classify_legacy(&legacy),
            Some(expected),
            "legacy diagnostic changed for {name}: {legacy:?}",
        );
        assert_eq!(
            classify_rewrite(&rewrite),
            Some(expected),
            "rewrite diagnostic changed for {name}: {rewrite:?}",
        );
    }
}

fn legacy_check(source: &str) -> Result<(), LegacyFailure> {
    let tokens = Lexer::new(source)
        .tokenize()
        .map_err(|message| LegacyFailure {
            stage: "lex",
            message,
        })?;
    let mut ast = Parser::new(tokens)
        .parse()
        .map_err(|message| LegacyFailure {
            stage: "parse",
            message,
        })?;
    Monomorphizer::process_program(&mut ast).map_err(|message| LegacyFailure {
        stage: "monomorph",
        message,
    })?;
    let mut resolver = Resolver::new();
    resolver
        .resolve_program(&mut ast)
        .map_err(|message| LegacyFailure {
            stage: "resolve",
            message,
        })?;
    TypeChecker::new(&mut resolver.table)
        .check_program(&ast)
        .map_err(|message| LegacyFailure {
            stage: "type",
            message,
        })
}

fn rewrite_check(source: &str) -> Result<(), TypeError> {
    let filesystem = MemoryFileSystem::with_source(source);
    let graph = GraphBuilder::new(&filesystem)
        .build(GraphRequest::new(
            "/compat/main.lpp",
            PackageSpec::new("phase3-compat", "/compat"),
        ))
        .expect("compatibility source must pass the Phase 2 frontend");
    let package = lower_package(&graph, ResolutionMode::LegacyFlat)
        .expect("compatibility source must lower to HIR");
    infer_hir_package(&package, ShadowInferenceOptions::default())
        .map(|_| ())
        .map_err(|error| error.error)
}

fn classify_legacy(failure: &LegacyFailure) -> Option<DiagnosticClass> {
    let lower = failure.message.to_ascii_lowercase();
    if failure.stage == "type"
        && (lower.contains("type mismatch")
            || lower.contains("return type")
            || lower.contains("list element")
            || lower.contains("expected"))
    {
        Some(DiagnosticClass::TypeMismatch)
    } else {
        None
    }
}

const fn classify_rewrite(error: &TypeError) -> Option<DiagnosticClass> {
    match error {
        TypeError::Mismatch { .. } => Some(DiagnosticClass::TypeMismatch),
        _ => None,
    }
}
