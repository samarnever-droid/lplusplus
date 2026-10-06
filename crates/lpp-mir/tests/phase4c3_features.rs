//! Phase 4C3 exit-gate evidence: source-backed strings/chars, the
//! deterministic ABI builtin subset, closures with shared capture cells,
//! and async/await/spawn execution semantics.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    lower_package,
};
use lpp_mir::{
    ExecutionValue, InterpreterErrorKind, InterpreterLimits, MirBuildErrorKind, MirBuildOptions,
    MirFunctionId, MirFunctionKind, MirProgram, build_mir, execute_mir, execute_mir_with_stats,
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

/// Lower + type-check + build + verify; panics with context on failure.
fn executable(source: &str) -> (lpp_mir::MirProgram, TypeInterner) {
    let filesystem = MemoryFileSystem::new(source);
    let graph = GraphBuilder::new(&filesystem)
        .build(GraphRequest::new(
            "/execute/main.lpp",
            PackageSpec::new("execute", "/execute"),
        ))
        .unwrap();
    let package = lower_package(&graph, ResolutionMode::Namespaced).unwrap();
    let mut types = infer_hir_package(&package, ShadowInferenceOptions::default())
        .unwrap_or_else(|error| panic!("type stage: {error:?}"));
    let program = build_mir(
        &package,
        &graph.sources,
        &mut types,
        MirBuildOptions::default(),
    )
    .unwrap_or_else(|error| panic!("build: {error:?}"));
    (program, types.interner)
}

/// Type-check only, exposing the structured type-stage error.
fn type_error(source: &str) -> lpp_types::ShadowTypeError {
    let filesystem = MemoryFileSystem::new(source);
    let graph = GraphBuilder::new(&filesystem)
        .build(GraphRequest::new(
            "/execute/main.lpp",
            PackageSpec::new("execute", "/execute"),
        ))
        .unwrap();
    let package = lower_package(&graph, ResolutionMode::Namespaced).unwrap();
    infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap_err()
}

fn build_error(source: &str) -> lpp_mir::MirBuildError {
    let filesystem = MemoryFileSystem::new(source);
    let graph = GraphBuilder::new(&filesystem)
        .build(GraphRequest::new(
            "/execute/main.lpp",
            PackageSpec::new("execute", "/execute"),
        ))
        .unwrap();
    let package = lower_package(&graph, ResolutionMode::Namespaced).unwrap();
    let mut types = infer_hir_package(&package, ShadowInferenceOptions::default())
        .unwrap_or_else(|error| panic!("type stage: {error:?}"));
    build_mir(
        &package,
        &graph.sources,
        &mut types,
        MirBuildOptions::default(),
    )
    .unwrap_err()
}

/// Test-program entry: the last top-level function. Closure bodies are
/// lowered after their enclosing function, so closures may trail `main`
/// in function-ID order.
fn entry_function(program: &MirProgram) -> MirFunctionId {
    program
        .functions()
        .filter(|(_, function)| function.kind != MirFunctionKind::Closure)
        .map(|(id, _)| id)
        .last()
        .expect("test programs define at least one top-level function")
}

fn execute_last(source: &str) -> ExecutionValue {
    let (program, types) = executable(source);
    let entry = entry_function(&program);
    execute_mir(&program, &types, entry, &[], InterpreterLimits::default()).unwrap()
}

fn execute_last_with_stats(source: &str) -> lpp_mir::ExecutionOutcome {
    let (program, types) = executable(source);
    let entry = entry_function(&program);
    execute_mir_with_stats(&program, &types, entry, &[], InterpreterLimits::default()).unwrap()
}

// ── 4C3A: strings, chars, and the deterministic builtin subset ─────────────

#[test]
fn string_and_char_literals_execute_with_escapes() {
    assert_eq!(
        execute_last("def main() -> Str:\n    return \"hello\"\n"),
        ExecutionValue::String("hello".to_owned()),
    );
    assert_eq!(
        execute_last("def main() -> Str:\n    return \"tab\\tnewline\\nquote\\\"backslash\\\\\"\n"),
        ExecutionValue::String("tab\tnewline\nquote\"backslash\\".to_owned()),
    );
    assert_eq!(
        execute_last("def main() -> Str:\n    return \"\"\n"),
        ExecutionValue::String(String::new()),
    );
    assert_eq!(
        execute_last("def main() -> Char:\n    return 'x'\n"),
        ExecutionValue::Char('x'),
    );
    assert_eq!(
        execute_last("def main() -> Char:\n    return '\\n'\n"),
        ExecutionValue::Char('\n'),
    );
}

#[test]
fn string_concatenation_and_equality_execute_exactly() {
    assert_eq!(
        execute_last("def main() -> Str:\n    return \"ab\" + \"cd\"\n"),
        ExecutionValue::String("abcd".to_owned()),
    );
    assert_eq!(
        execute_last("def main() -> Bool:\n    return \"abc\" == \"abc\"\n"),
        ExecutionValue::Bool(true),
    );
    assert_eq!(
        execute_last("def main() -> Bool:\n    return \"abc\" == \"abd\"\n"),
        ExecutionValue::Bool(false),
    );
    assert_eq!(
        execute_last("def main() -> Bool:\n    return \"abc\" != \"abd\"\n"),
        ExecutionValue::Bool(true),
    );
    assert_eq!(
        execute_last("def main() -> Bool:\n    return 'a' == 'a'\n"),
        ExecutionValue::Bool(true),
    );
    assert_eq!(
        execute_last("def main() -> Bool:\n    return 'a' != 'b'\n"),
        ExecutionValue::Bool(true),
    );
}

#[test]
fn deterministic_builtin_subset_executes_to_exact_values() {
    // String operations.
    assert_eq!(
        execute_last("def main() -> Str:\n    return str_concat(\"a\", \"b\")\n"),
        ExecutionValue::String("ab".to_owned()),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    return str_len(\"hello\")\n"),
        ExecutionValue::Int(5),
    );
    assert_eq!(
        execute_last("def main() -> Bool:\n    return str_contains(\"hello\", \"ell\")\n"),
        ExecutionValue::Bool(true),
    );
    assert_eq!(
        execute_last("def main() -> Bool:\n    return str_starts_with(\"hello\", \"he\")\n"),
        ExecutionValue::Bool(true),
    );
    assert_eq!(
        execute_last("def main() -> Bool:\n    return str_ends_with(\"hello\", \"lo\")\n"),
        ExecutionValue::Bool(true),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    return str_find(\"hello\", \"l\")\n"),
        ExecutionValue::Int(2),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    return str_find(\"hello\", \"z\")\n"),
        ExecutionValue::Int(-1),
    );
    assert_eq!(
        execute_last("def main() -> Str:\n    return str_replace(\"a-b-c\", \"-\", \"+\")\n"),
        ExecutionValue::String("a+b+c".to_owned()),
    );
    assert_eq!(
        execute_last("def main() -> Str:\n    return str_to_lower(\"AbC\")\n"),
        ExecutionValue::String("abc".to_owned()),
    );
    assert_eq!(
        execute_last("def main() -> Str:\n    return str_to_upper(\"AbC\")\n"),
        ExecutionValue::String("ABC".to_owned()),
    );
    assert_eq!(
        execute_last("def main() -> Str:\n    return str_trim(\"  hi \\t\\n\")\n"),
        ExecutionValue::String("hi".to_owned()),
    );

    // Conversions.
    assert_eq!(
        execute_last("def main() -> Str:\n    return int_to_str(-42)\n"),
        ExecutionValue::String("-42".to_owned()),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    return str_to_int(\"-123abc\")\n"),
        ExecutionValue::Int(-123),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    return str_to_int(\"nope\")\n"),
        ExecutionValue::Int(0),
    );
    assert_eq!(
        execute_last("def main() -> Str:\n    return float_to_str(100000.0)\n"),
        ExecutionValue::String("100000".to_owned()),
    );
    assert_eq!(
        execute_last("def main() -> Str:\n    return float_to_str(0.0001)\n"),
        ExecutionValue::String("0.0001".to_owned()),
    );
    assert_eq!(
        execute_last("def main() -> Str:\n    return float_to_str(123456789.0)\n"),
        ExecutionValue::String("1.23457e+08".to_owned()),
    );
    assert_eq!(
        execute_last("def main() -> Str:\n    return bool_to_str(true)\n"),
        ExecutionValue::String("true".to_owned()),
    );
    assert_eq!(
        execute_last("def main() -> Str:\n    return u64_to_str(-1)\n"),
        ExecutionValue::String("18446744073709551615".to_owned()),
    );
    assert_eq!(
        execute_last("def main() -> Str:\n    return u64_to_hex(255)\n"),
        ExecutionValue::String("ff".to_owned()),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    return str_to_u64(\"0xff\")\n"),
        ExecutionValue::Int(255),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    return str_to_u64(\"42\")\n"),
        ExecutionValue::Int(42),
    );

    // Integer helpers.
    assert_eq!(
        execute_last("def main() -> Int:\n    return abs(-7)\n"),
        ExecutionValue::Int(7),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    return min(3, 9)\n"),
        ExecutionValue::Int(3),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    return max(3, 9)\n"),
        ExecutionValue::Int(9),
    );
    // `lt_u`/`ge_u` compare unsigned, matching v1 `lpp_lt_u`/`lpp_ge_u`, which
    // return an integer 0/1 rather than a Bool (the corpus uses them C-style,
    // e.g. `lt_u(a, b) == 1`). `-1` as u64 is the max value, so it is not
    // less-than 1 (=> 0) and is greater-or-equal to 1 (=> 1).
    assert_eq!(
        execute_last("def main() -> Int:\n    return lt_u(-1, 1)\n"),
        ExecutionValue::Int(0),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    return ge_u(-1, 1)\n"),
        ExecutionValue::Int(1),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    return shr_u(4096, 2)\n"),
        ExecutionValue::Int(1024),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    return shl_u(1, 4)\n"),
        ExecutionValue::Int(16),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    return div_u(17, 5)\n"),
        ExecutionValue::Int(3),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    return rem_u(17, 5)\n"),
        ExecutionValue::Int(2),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    return popcount64(0b1011)\n"),
        ExecutionValue::Int(3),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    return clz64(1)\n"),
        ExecutionValue::Int(63),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    return ctz64(8)\n"),
        ExecutionValue::Int(3),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    return bswap16(0x1234)\n"),
        ExecutionValue::Int(0x3412),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    return bswap32(0x12345678)\n"),
        ExecutionValue::Int(0x78563412),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    return rotl64(1, 1)\n"),
        ExecutionValue::Int(2),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    return rotr64(1, 1)\n"),
        ExecutionValue::Int(i64::MIN),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    return trunc_i8(200)\n"),
        ExecutionValue::Int(-56),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    return add_checked(1, 2)\n"),
        ExecutionValue::Int(3),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    return sub_checked(5, 2)\n"),
        ExecutionValue::Int(3),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    return mul_checked(3, 4)\n"),
        ExecutionValue::Int(12),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    return add_wrap(1, -2)\n"),
        ExecutionValue::Int(-1),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    return mul_wrap(2, 3)\n"),
        ExecutionValue::Int(6),
    );

    // Float math.
    assert_eq!(
        execute_last("def main() -> Float:\n    return floor(2.7)\n"),
        ExecutionValue::FloatBits(2.0_f64.to_bits()),
    );
    assert_eq!(
        execute_last("def main() -> Float:\n    return ceil(2.1)\n"),
        ExecutionValue::FloatBits(3.0_f64.to_bits()),
    );
    assert_eq!(
        execute_last("def main() -> Float:\n    return sqrt(9.0)\n"),
        ExecutionValue::FloatBits(3.0_f64.to_bits()),
    );
    assert_eq!(
        execute_last("def main() -> Float:\n    return pow(2.0, 3.0)\n"),
        ExecutionValue::FloatBits(8.0_f64.to_bits()),
    );
    assert_eq!(
        execute_last("def main() -> Float:\n    return fmod(7.5, 2.0)\n"),
        ExecutionValue::FloatBits(1.5_f64.to_bits()),
    );

    // Structural list operations.
    assert_eq!(
        execute_last(
            "def main() -> Int:\n    xs := list_new()\n    list_push(xs, 10)\n    list_push(xs, 20)\n    list_set(xs, 0, 5)\n    return list_get(xs, 1) + list_len(xs)\n",
        ),
        ExecutionValue::Int(22),
    );
    assert_eq!(
        execute_last("def main() -> Int:\n    xs := list_new()\n    return list_len(xs)\n",),
        ExecutionValue::Int(0),
    );
}

#[test]
fn printing_builtins_capture_output_in_order() {
    let source = concat!(
        "def main():\n",
        "    print_str(\"a\")\n",
        "    print_int(7)\n",
        "    print(1.5)\n",
        "    print(true)\n",
        "    print(\"hi\")\n",
        "    write_str(\"tail\")\n",
    );
    let outcome = execute_last_with_stats(source);
    assert_eq!(outcome.value, ExecutionValue::Void);
    assert_eq!(
        outcome.output,
        vec![
            "a\n".to_owned(),
            "7\n".to_owned(),
            "1.500000\n".to_owned(),
            "1\n".to_owned(),
            "hi\n".to_owned(),
            "tail".to_owned(),
        ]
    );
}

#[test]
fn f_string_interpolates_scalar_values() {
    // An interpolated Int/Float/Bool is coerced to a string via the matching
    // *_to_str builtin, and a leading interpolation still concatenates.
    let source = concat!(
        "def main() -> Str:\n",
        "    count := 42\n",
        "    return f\"{count} items\"\n",
    );
    assert_eq!(
        execute_last(source),
        ExecutionValue::String("42 items".to_owned()),
    );
}

#[test]
fn string_plus_scalar_concatenates() {
    let source = concat!(
        "def main() -> Str:\n",
        "    n := 7\n",
        "    return \"n=\" + n\n",
    );
    assert_eq!(
        execute_last(source),
        ExecutionValue::String("n=7".to_owned()),
    );
}

#[test]
fn f_string_desugars_to_string_concatenation() {
    // A formatted string is lowered in HIR into `+` concatenation of its
    // literal segments and interpolated identifiers, so by the time MIR runs it
    // is an ordinary string expression that builds and executes.
    let source = concat!(
        "def main() -> Str:\n",
        "    name := \"world\"\n",
        "    return f\"hello {name}!\"\n",
    );
    assert_eq!(
        execute_last(source),
        ExecutionValue::String("hello world!".to_owned()),
    );
}

#[test]
fn os_effect_builtins_fail_at_execution_structurally() {
    let source = "def main() -> Int:\n    return random()\n";
    let (program, types) = executable(source);
    let entry = entry_function(&program);
    let error =
        execute_mir(&program, &types, entry, &[], InterpreterLimits::default()).unwrap_err();
    assert_eq!(error.kind, InterpreterErrorKind::UnsupportedBuiltin);
    assert_eq!(error.code(), "E4303");

    let source = "def main() -> Int:\n    return time_ms()\n";
    let (program, types) = executable(source);
    let entry = entry_function(&program);
    let error =
        execute_mir(&program, &types, entry, &[], InterpreterLimits::default()).unwrap_err();
    assert_eq!(error.kind, InterpreterErrorKind::UnsupportedBuiltin);
}

#[test]
fn identical_string_literals_share_one_arena_entry() {
    let source = concat!(
        "def main() -> Str:\n",
        "    a := \"shared\"\n",
        "    b := \"shared\"\n",
        "    c := \"shared\"\n",
        "    return a\n",
    );
    let (program, _types) = executable(source);
    assert_eq!(program.string_count(), 1);
    assert_eq!(
        execute_last(source),
        ExecutionValue::String("shared".to_owned()),
    );
}

#[test]
fn builtin_string_work_scales_linearly_from_20_to_200() {
    let generated = |count: usize| {
        let mut source = String::from("def main() -> Str:\n    mut s := \"\"\n");
        for index in 0..count {
            source.push_str(&format!("    s = str_concat(s, \"{index}\")\n"));
        }
        source.push_str("    return s\n");
        source
    };
    let baseline = execute_last_with_stats(&generated(0));
    let small = execute_last_with_stats(&generated(20));
    let large = execute_last_with_stats(&generated(200));
    // `calls` counts function entries (main only); the builtin work shows
    // up in steps.
    assert_eq!(small.stats.calls, 1);
    assert_eq!(large.stats.calls, 1);
    assert!(large.stats.steps > small.stats.steps);
    // Each iteration costs exactly one builtin call and a constant number
    // of steps, so the per-iteration marginal cost is scale-invariant.
    let per_small = (small.stats.steps - baseline.stats.steps) / 20;
    let per_large = (large.stats.steps - baseline.stats.steps) / 200;
    assert!(per_small > 0);
    assert_eq!(per_small, per_large);
    assert_eq!(
        small.value,
        ExecutionValue::String((0..20).map(|index| index.to_string()).collect::<String>()),
    );
}

// ── 4C3B: closures and shared capture cells ────────────────────────────────

#[test]
fn stateful_counter_closure_executes_1_2_3() {
    // v1 `test_mutable_closure.lpp` shape.
    let source = concat!(
        "def main():\n",
        "    mut count := 0\n",
        "    counter := fn() -> Int:\n",
        "        count = count + 1\n",
        "        return count\n",
        "    print(counter())\n",
        "    print(counter())\n",
        "    print(counter())\n",
    );
    let outcome = execute_last_with_stats(source);
    assert_eq!(outcome.value, ExecutionValue::Void);
    assert_eq!(
        outcome.output,
        vec!["1\n".to_owned(), "2\n".to_owned(), "3\n".to_owned()],
    );
}

#[test]
fn closure_stored_in_list_and_retrieved_executes() {
    // v1 `list_closures.lpp` shape.
    let source = concat!(
        "def main() -> Int:\n",
        "    add_one := fn(value: Int) -> Int: value + 1\n",
        "    callbacks := [add_one]\n",
        "    callback := list_get(callbacks, 0)\n",
        "    return callback(41)\n",
    );
    assert_eq!(execute_last(source), ExecutionValue::Int(42));
}

#[test]
fn closure_capture_mutation_is_by_reference_cell() {
    // By-reference capture cell (5C2, superseding 4C3B value-at-creation):
    // the captured local is a single-element list shared between the
    // enclosing frame and the closure, so the closure's write updates
    // the enclosing local.
    let source = concat!(
        "def main() -> Int:\n",
        "    mut count := 0\n",
        "    bump := fn() -> Int:\n",
        "        count = count + 1\n",
        "        return count\n",
        "    first := bump()\n",
        "    second := bump()\n",
        "    return count + first + second\n",
    );
    // The shared cell accumulates 1 then 2; the enclosing `count` reads
    // the same cell, so count = 2, first = 1, second = 2.
    assert_eq!(execute_last(source), ExecutionValue::Int(5));
}

#[test]
fn nested_closures_capture_enclosing_frame_deterministically() {
    // A closure defined inside a function body captures the enclosing
    // frame's locals. (The v1 parser cannot name the closure's type in a
    // return position, so the capture is exercised in the defining
    // frame.)
    let source = concat!(
        "def main() -> Int:\n",
        "    base := 10\n",
        "    add := fn(x: Int) -> Int:\n",
        "        return base + x\n",
        "    first := add(5)\n",
        "    return first + add(10)\n",
    );
    assert_eq!(execute_last(source), ExecutionValue::Int(35));
    assert_eq!(execute_last(source), ExecutionValue::Int(35));
}

#[test]
fn closure_parameter_defaults_are_rejected_but_definition_defaults_work() {
    // The v1 parser still rejects closure parameters with defaults outright.
    let source = "def main() -> Int:\n    f := fn(x: Int = 1) -> Int: x\n    return f(2)\n";
    let filesystem = MemoryFileSystem::new(source);
    let graph = GraphBuilder::new(&filesystem)
        .build(GraphRequest::new(
            "/execute/main.lpp",
            PackageSpec::new("execute", "/execute"),
        ))
        .unwrap();
    let diagnostics = lower_package(&graph, ResolutionMode::Namespaced).unwrap_err();
    assert_eq!(diagnostics[0].code.as_str(), "E3100");

    // A default parameter on a plain definition now lowers and executes: the
    // supplied argument overrides the default, and an omitted argument uses it.
    let supplied =
        "def f(x: Int = 1) -> Int:\n    return x\n\ndef main() -> Int:\n    return f(2)\n";
    assert_eq!(execute_last(supplied), ExecutionValue::Int(2));
    let defaulted =
        "def f(x: Int = 7) -> Int:\n    return x\n\ndef main() -> Int:\n    return f()\n";
    assert_eq!(execute_last(defaulted), ExecutionValue::Int(7));
}

#[test]
fn closure_work_scales_linearly_from_20_to_200() {
    let generated = |count: usize| {
        let mut source = String::from("def main() -> Int:\n    mut total := 0\n");
        for index in 0..count {
            source.push_str(&format!("    f{index} := fn() -> Int: {index}\n"));
            source.push_str(&format!("    total = total + f{index}()\n"));
        }
        source.push_str("    return total\n");
        source
    };
    let small = execute_last_with_stats(&generated(20));
    let large = execute_last_with_stats(&generated(200));
    assert_eq!(
        small.value,
        ExecutionValue::Int((0..20).map(i64::from).sum())
    );
    assert_eq!(
        large.value,
        ExecutionValue::Int((0..200).map(i64::from).sum())
    );
    // One entry for main plus one call per generated closure.
    assert_eq!(small.stats.calls, 1 + 20);
    assert_eq!(large.stats.calls, 1 + 200);
    assert!(large.stats.heap_nodes > small.stats.heap_nodes);
}

// ── 4C3C: async functions, tasks, await, and spawn ─────────────────────────

#[test]
fn async_await_chain_executes_to_exact_value() {
    // v1 `async_await_chain.lpp` shape.
    let source = concat!(
        "async def first() -> Str:\n",
        "    return \"ready\"\n",
        "async def second() -> Str:\n",
        "    value := first().await\n",
        "    return value\n",
        "async def main():\n",
        "    result := second().await\n",
        "    print_str(result)\n",
    );
    let outcome = execute_last_with_stats(source);
    assert_eq!(outcome.value, ExecutionValue::Void);
    assert_eq!(outcome.output, vec!["ready\n".to_owned()]);
}

#[test]
fn double_await_is_idempotent() {
    // v1 `async_double_await.lpp` shape.
    let source = concat!(
        "async def value() -> Str:\n",
        "    return \"twice\"\n",
        "async def main():\n",
        "    task := value()\n",
        "    first := task.await\n",
        "    second := task.await\n",
        "    print_str(first)\n",
        "    print_str(second)\n",
    );
    let outcome = execute_last_with_stats(source);
    assert_eq!(
        outcome.output,
        vec!["twice\n".to_owned(), "twice\n".to_owned()],
    );
}

#[test]
fn repeated_await_allocates_no_extra_heap_nodes() {
    let once = concat!(
        "async def value() -> Str:\n",
        "    return \"twice\"\n",
        "async def main():\n",
        "    task := value()\n",
        "    print_str(task.await)\n",
    );
    let thrice = concat!(
        "async def value() -> Str:\n",
        "    return \"twice\"\n",
        "async def main():\n",
        "    task := value()\n",
        "    print_str(task.await)\n",
        "    print_str(task.await)\n",
        "    print_str(task.await)\n",
    );
    let single = execute_last_with_stats(once);
    let repeated = execute_last_with_stats(thrice);
    assert_eq!(single.stats.heap_nodes, repeated.stats.heap_nodes);
    assert_eq!(
        repeated.output,
        vec![
            "twice\n".to_owned(),
            "twice\n".to_owned(),
            "twice\n".to_owned(),
        ],
    );
}

#[test]
fn async_main_auto_drains() {
    let outcome = execute_last_with_stats(concat!(
        "async def main():\n",
        "    print(\"async immediate\")\n",
    ));
    assert_eq!(outcome.value, ExecutionValue::Void);
    assert_eq!(outcome.output, vec!["async immediate\n".to_owned()]);
}

#[test]
fn spawn_runs_closure_exactly_once_and_returns_void() {
    let source = concat!(
        "def main():\n",
        "    spawn fn():\n",
        "        print_str(\"once\")\n",
    );
    let outcome = execute_last_with_stats(source);
    assert_eq!(outcome.value, ExecutionValue::Void);
    assert_eq!(outcome.output, vec!["once\n".to_owned()]);
}

#[test]
fn spawn_of_non_closure_fails_structurally() {
    let error = build_error("def main():\n    spawn 5\n");
    assert!(
        matches!(error.kind, MirBuildErrorKind::InvalidSpawnTarget(_)),
        "expected InvalidSpawnTarget, got {error:?}",
    );
    assert_eq!(error.code(), "E4006");
}

#[test]
fn await_of_non_task_fails_structurally() {
    let source = concat!("async def main():\n", "    x := 5\n", "    y := x.await\n",);
    let error = type_error(source);
    assert!(
        matches!(error.error, lpp_types::TypeError::Mismatch { .. }),
        "expected a structured type mismatch, got {error:?}"
    );
}
