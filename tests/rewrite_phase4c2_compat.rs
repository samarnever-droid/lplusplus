use std::collections::BTreeMap;
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
use lpp_mir::{
    ExecutionValue, InterpreterLimits, MirBuildOptions, MirFunctionId, build_mir, execute_mir,
    verify_mir,
};
use lpp_types::{ShadowInferenceOptions, infer_hir_package};

#[derive(Debug)]
struct MemoryFileSystem {
    files: BTreeMap<PathBuf, String>,
}

impl MemoryFileSystem {
    fn new(source: &str) -> Self {
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

fn legacy_accepts(source: &str) -> Result<(), String> {
    let tokens = Lexer::new(source).tokenize()?;
    let mut ast = Parser::new(tokens).parse()?;
    Monomorphizer::process_program(&mut ast)?;
    let mut resolver = Resolver::new();
    resolver.resolve_program(&mut ast)?;
    TypeChecker::new(&mut resolver.table).check_program(&ast)
}

fn rewrite_return(source: &str) -> ExecutionValue {
    let filesystem = MemoryFileSystem::new(source);
    let graph = GraphBuilder::new(&filesystem)
        .build(GraphRequest::new(
            "/compat/main.lpp",
            PackageSpec::new("phase4c2-compat", "/compat"),
        ))
        .unwrap();
    let package = lower_package(&graph, ResolutionMode::LegacyFlat).unwrap();
    let mut types = infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap();
    let program = build_mir(&package, &graph.sources, &mut types, MirBuildOptions::default()).unwrap();
    assert!(verify_mir(&program, &types.interner).is_empty());
    execute_mir(
        &program,
        &types.interner,
        MirFunctionId::from_raw(u32::try_from(program.function_count() - 1).unwrap()),
        &[],
        InterpreterLimits::default(),
    )
    .unwrap()
}

#[test]
fn legacy_accepted_struct_programs_keep_their_rewrite_return_values() {
    let cases = [
        (
            "plain-struct",
            concat!(
                "struct Pair:\n",
                "    first: Int\n",
                "    second: Int\n",
                "def main() -> Int:\n",
                "    pair := Pair(20, 22)\n",
                "    return pair.first + pair.second\n",
            ),
        ),
        (
            "generic-struct",
            concat!(
                "struct Pair[T]:\n",
                "    first: T\n",
                "    second: T\n",
                "def main() -> Int:\n",
                "    pair := Pair[Int](20, 22)\n",
                "    return pair.first + pair.second\n",
            ),
        ),
        (
            "nested-struct",
            concat!(
                "struct Pair:\n",
                "    first: Int\n",
                "    second: Int\n",
                "struct Box:\n",
                "    value: Pair\n",
                "def main() -> Int:\n",
                "    pair := Pair(20, 22)\n",
                "    box := Box(pair)\n",
                "    return box.value.first + box.value.second\n",
            ),
        ),
    ];

    for (name, source) in cases {
        legacy_accepts(source).unwrap_or_else(|error| panic!("legacy rejected {name}: {error}"));
        assert_eq!(
            rewrite_return(source),
            ExecutionValue::Int(42),
            "rewrite changed the legacy return value for {name}",
        );
    }
}

#[test]
fn legacy_accepted_enum_match_payload_and_try_programs_keep_exact_results() {
    let cases = [
        (
            "multi-payload-match",
            concat!(
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
            ),
            42,
        ),
        (
            "wildcard-first-wins",
            concat!(
                "enum Choice:\n",
                "    Some(value: Int)\n",
                "    None\n",
                "def main() -> Int:\n",
                "    value := Choice.Some(7)\n",
                "    match value:\n",
                "        _:\n",
                "            return 42\n",
                "        Some(ignored):\n",
                "            return 0\n",
                "    return 1\n",
            ),
            42,
        ),
        (
            "try-residual-propagation",
            concat!(
                "enum Result:\n",
                "    Ok(value: Int)\n",
                "    Err(code: Int)\n",
                "def propagate(value: Result) -> Result:\n",
                "    payload := value?\n",
                "    return Result.Ok(payload)\n",
                "def main() -> Int:\n",
                "    result := propagate(Result.Err(42))\n",
                "    match result:\n",
                "        Ok(value):\n",
                "            return value\n",
                "        Err(code):\n",
                "            return code\n",
                "    return 0\n",
            ),
            42,
        ),
    ];

    for (name, source, expected) in cases {
        legacy_accepts(source).unwrap_or_else(|error| panic!("legacy rejected {name}: {error}"));
        assert_eq!(
            rewrite_return(source),
            ExecutionValue::Int(expected),
            "rewrite changed the legacy enum-flow result for {name}",
        );
    }
}

#[test]
fn legacy_accepted_places_destructuring_and_list_loops_keep_exact_results() {
    let cases = [
        (
            "list-alias-store",
            concat!(
                "def main() -> Int:\n",
                "    mut values := [1, 2]\n",
                "    alias := values\n",
                "    values[0] += 4\n",
                "    values[1] = alias[0] + 5\n",
                "    return alias[0] + alias[1]\n",
            ),
            15,
        ),
        (
            "tuple-projection-destructure",
            concat!(
                "def main() -> Int:\n",
                "    pair := (20, 22)\n",
                "    (left, right) := pair\n",
                "    return pair[0] + pair[1] + left + right\n",
            ),
            84,
        ),
        (
            "list-loop-break-continue",
            concat!(
                "def main() -> Int:\n",
                "    mut total := 0\n",
                "    for value in [1, 2, 3, 4]:\n",
                "        if value == 2:\n",
                "            continue\n",
                "        if value == 4:\n",
                "            break\n",
                "        total += value\n",
                "    return total\n",
            ),
            4,
        ),
        (
            "struct-alias-store",
            concat!(
                "struct Box:\n",
                "    value: Int\n",
                "def main() -> Int:\n",
                "    mut box := Box(40)\n",
                "    mut alias := box\n",
                "    alias.value += 2\n",
                "    return box.value\n",
            ),
            42,
        ),
    ];

    for (name, source, expected) in cases {
        legacy_accepts(source).unwrap_or_else(|error| panic!("legacy rejected {name}: {error}"));
        assert_eq!(
            rewrite_return(source),
            ExecutionValue::Int(expected),
            "rewrite changed the legacy result for {name}",
        );
    }
}
