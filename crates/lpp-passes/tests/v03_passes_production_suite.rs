//! Comprehensive v0.3 production integration test suite for `lpp-passes`.
//!
//! Exercises:
//! - `ConstFoldPass` & oracle-exact constant folding (`fold_unary`, `fold_binary`,
//!   `is_primitive_constant`) across integers, IEEE-754 floats, bools, chars, strings
//! - `ConstPropPass` & `single_def_constants` propagation and exclusion gates
//! - `BranchFoldPass` CFG simplification and unreachable-arm trap safety
//! - `InlinePass` (whole-program LTO & profile-guided) and scalar/effect gates
//! - Custom `MirPass` pipelines (copy propagation, algebraic peepholes, dead-code
//!   audit/store-cleanup) and `PassManager` error/ordering contracts (`E4201`–`E4204`)
//! - `CORE_MIR_INVARIANTS` preservation after every pass, level sweep budgets,
//!   idempotence, determinism, and shape stability
//! - Behavioral equivalence (`execute_mir_with_stats`) on 45+ diverse L++ programs
//!   (loops, recursion, enums, tuples, closures, structs, match, `Set`/`Deque`,
//!   maps, strings, async/spawn, raw pointer intrinsics)
//! - ARC retain/release preservation across optimization and LTO

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use lpp_common::OptimizationLevel;
use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    lower_package,
};
use lpp_mir::{
    ArcStats, BasicBlockId, BinaryOperator, CORE_MIR_INVARIANTS, Constant, ExecutionOutcome,
    ExecutionValue, InstructionId, InstructionKind, InterpreterError, InterpreterErrorKind,
    InterpreterLimits, MirBuildOptions, MirFunctionId, MirFunctionKind, MirInvariant, MirLocalId,
    MirLocalKind, MirProgram, Operand, Rvalue, Terminator, UnaryOperator, build_mir,
    execute_mir_arc, execute_mir_with_stats, mir_snapshot, verify_mir,
};
use lpp_passes::{
    BranchFoldPass, ConstFoldPass, ConstPropPass, InlinePass, MirPass, PassContext, PassFailure,
    PassManager, PassManagerErrorKind, fold_binary, fold_unary, is_primitive_constant,
    optimization_passes, run_optimization, run_optimization_with_lto,
    run_optimization_with_profile, single_def_constants,
};
use lpp_types::{ShadowInferenceOptions, TypeInterner, infer_hir_package};

// ── Test Scaffolding & Helpers ──────────────────────────────────────────────

#[derive(Debug)]
struct MemoryFileSystem {
    files: BTreeMap<PathBuf, String>,
}

impl MemoryFileSystem {
    fn multi(entries: &[(&str, &str)]) -> Self {
        let mut files = BTreeMap::new();
        for (path, content) in entries {
            files.insert(PathBuf::from(path), (*content).to_owned());
        }
        Self { files }
    }
}

impl FileSystem for MemoryFileSystem {
    fn is_file(&self, path: &Path) -> Result<bool, FileSystemError> {
        Ok(self.files.contains_key(path))
    }

    fn canonicalize(&self, path: &Path) -> Result<PathBuf, FileSystemError> {
        if self.files.contains_key(path) || self.files.keys().any(|f| f.starts_with(path)) {
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

fn compile_mir(source: &str) -> (MirProgram, TypeInterner) {
    compile_mir_files(&[("/passes/main.lpp", source)], "/passes/main.lpp")
}

fn compile_mir_files(files: &[(&str, &str)], entry_path: &str) -> (MirProgram, TypeInterner) {
    let fs = MemoryFileSystem::multi(files);
    let graph = GraphBuilder::new(&fs)
        .build(GraphRequest::new(
            entry_path,
            PackageSpec::new("passes", "/passes"),
        ))
        .unwrap_or_else(|e| panic!("graph build failed: {e:?}"));
    let package = lower_package(&graph, ResolutionMode::Namespaced)
        .unwrap_or_else(|e| panic!("hir lower failed: {e:?}"));
    let mut types = infer_hir_package(&package, ShadowInferenceOptions::default())
        .unwrap_or_else(|e| panic!("type inference failed: {e:?}"));
    let program = build_mir(
        &package,
        &graph.sources,
        &mut types,
        MirBuildOptions::default(),
    )
    .unwrap_or_else(|e| panic!("mir build failed: {e:?}"));
    let errors = verify_mir(&program, &types.interner);
    assert!(
        errors.is_empty(),
        "initial MIR must verify cleanly: {errors:?}\nMIR:\n{}",
        lpp_mir::mir_snapshot(&program)
    );
    (program, types.interner)
}

fn entry_function(program: &MirProgram) -> MirFunctionId {
    program
        .functions()
        .filter(|(_, f)| f.kind != MirFunctionKind::Closure && f.instance.is_none())
        .map(|(id, _)| id)
        .last()
        .expect("program must define at least one top-level function")
}

fn top_level_function(program: &MirProgram, index: usize) -> MirFunctionId {
    program
        .functions()
        .filter(|(_, f)| f.kind != MirFunctionKind::Closure)
        .nth(index)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no top-level function at index {index}"))
}

fn run_legacy(
    program: &MirProgram,
    types: &TypeInterner,
    entry: MirFunctionId,
) -> ExecutionOutcome {
    execute_mir_with_stats(program, types, entry, &[], InterpreterLimits::default())
        .unwrap_or_else(|e| panic!("MIR execution failed: {e}"))
}

fn run_legacy_err(
    program: &MirProgram,
    types: &TypeInterner,
    entry: MirFunctionId,
) -> InterpreterError {
    execute_mir_with_stats(program, types, entry, &[], InterpreterLimits::default())
        .expect_err("expected MIR execution error")
}

fn run_arc_acyclic(
    program: &MirProgram,
    types: &TypeInterner,
    entry: MirFunctionId,
) -> (ExecutionOutcome, ArcStats) {
    let outcome = execute_mir_arc(
        program,
        types,
        entry,
        &[],
        InterpreterLimits::default(),
        &[],
    )
    .unwrap_or_else(|e| panic!("ARC execution failed: {e}"));
    let arc = outcome.arc.expect("ARC stats present");
    assert_eq!(arc.pinned_live, 0, "acyclic execution must leave 0 live");
    (outcome, arc)
}

fn shape(program: &MirProgram) -> Vec<(BasicBlockId, Vec<InstructionId>)> {
    program
        .blocks()
        .map(|(id, block)| (id, program.block_instructions(block).to_vec()))
        .collect()
}

fn assert_shape_stable(before: &MirProgram, after: &MirProgram) {
    assert_eq!(before.function_count(), after.function_count());
    assert_eq!(before.block_count(), after.block_count());
    assert_eq!(before.local_count(), after.local_count());
    assert_eq!(before.instruction_count(), after.instruction_count());
    assert_eq!(shape(before), shape(after));
}

fn count_branches(program: &MirProgram) -> usize {
    program
        .blocks()
        .filter(|(_, b)| matches!(b.terminator, Terminator::Branch { .. }))
        .count()
}

fn count_gotos(program: &MirProgram) -> usize {
    program
        .blocks()
        .filter(|(_, b)| matches!(b.terminator, Terminator::Goto(_)))
        .count()
}

fn count_direct_calls(program: &MirProgram) -> usize {
    program
        .instructions()
        .filter(|(_, i)| {
            matches!(
                i.kind,
                InstructionKind::Assign {
                    value: Rvalue::Call {
                        callee: Operand::Function(_),
                        ..
                    },
                    ..
                }
            )
        })
        .count()
}

fn count_binary_rvalues(program: &MirProgram) -> usize {
    program
        .instructions()
        .filter(|(_, i)| {
            matches!(
                i.kind,
                InstructionKind::Assign {
                    value: Rvalue::Binary { .. },
                    ..
                }
            )
        })
        .count()
}

fn assigned_constants(program: &MirProgram, predicate: fn(&Constant) -> bool) -> Vec<Constant> {
    program
        .instructions()
        .filter_map(|(_, inst)| match &inst.kind {
            InstructionKind::Assign {
                value: Rvalue::Use(Operand::Constant(c)),
                ..
            } if predicate(c) => Some(*c),
            _ => None,
        })
        .collect()
}

fn assert_opt_equivalence(
    source: &str,
    level: OptimizationLevel,
    expected: ExecutionValue,
) -> (MirProgram, TypeInterner, ExecutionOutcome, ExecutionOutcome) {
    let (unopt, types) = compile_mir(source);
    let entry = entry_function(&unopt);
    let before = run_legacy(&unopt, &types, entry);
    assert_eq!(before.value, expected, "unoptimized value mismatch");

    let mut opt = unopt.clone();
    run_optimization(&mut opt, &types, level).expect("optimization should succeed");
    assert!(
        verify_mir(&opt, &types).is_empty(),
        "optimized MIR must verify cleanly"
    );
    assert_shape_stable(&unopt, &opt);

    let after = run_legacy(&opt, &types, entry);
    assert_eq!(after.value, before.value, "value changed after {level:?}");
    assert_eq!(
        after.output, before.output,
        "output changed after {level:?}"
    );
    assert!(
        after.stats.steps <= before.stats.steps,
        "step count regressed at {level:?}: {} -> {}",
        before.stats.steps,
        after.stats.steps
    );
    (opt, types, before, after)
}

// ── Section 1: ConstFoldPass & Oracle-Exact Fold Semantics ──────────────────

#[test]
fn const_fold_integer_wrapping_and_div_mod_boundaries() {
    let (program, _) = compile_mir("def main() -> Int:\n    return 0\n");
    let int_fold = |l: i64, r: i64, op: BinaryOperator| {
        fold_binary(&program, op, &Constant::Integer(l), &Constant::Integer(r))
    };

    // Wrapping add, sub, mul, div, rem
    assert_eq!(
        int_fold(i64::MAX, 1, BinaryOperator::Add),
        Some(Constant::Integer(i64::MIN))
    );
    assert_eq!(
        int_fold(i64::MIN, 1, BinaryOperator::Subtract),
        Some(Constant::Integer(i64::MAX))
    );
    assert_eq!(
        int_fold(i64::MIN, -1, BinaryOperator::Multiply),
        Some(Constant::Integer(i64::MIN))
    );
    assert_eq!(
        int_fold(i64::MIN, -1, BinaryOperator::Divide),
        Some(Constant::Integer(i64::MIN))
    );
    assert_eq!(
        int_fold(i64::MIN, -1, BinaryOperator::Modulo),
        Some(Constant::Integer(0))
    );
    assert_eq!(
        int_fold(-17, 5, BinaryOperator::Modulo),
        Some(Constant::Integer(-2))
    );
    assert_eq!(
        int_fold(-17, -5, BinaryOperator::Modulo),
        Some(Constant::Integer(-2))
    );

    // Div/Mod by zero must NOT fold so runtime E4303 triggers as expected.
    assert_eq!(int_fold(42, 0, BinaryOperator::Divide), None);
    assert_eq!(int_fold(0, 0, BinaryOperator::Divide), None);
    assert_eq!(int_fold(42, 0, BinaryOperator::Modulo), None);

    // Bitwise and shift operations (including shift >= 64 wrapping semantics)
    assert_eq!(
        int_fold(0b1100_1010, 0b1010_1100, BinaryOperator::BitAnd),
        Some(Constant::Integer(0b1000_1000))
    );
    assert_eq!(
        int_fold(0b1100_0000, 0b0000_1100, BinaryOperator::BitOr),
        Some(Constant::Integer(0b1100_1100))
    );
    assert_eq!(
        int_fold(0b1111_0000, 0b1010_1010, BinaryOperator::BitXor),
        Some(Constant::Integer(0b0101_1010))
    );
    assert_eq!(
        int_fold(3, 4, BinaryOperator::ShiftLeft),
        Some(Constant::Integer(48))
    );
    assert_eq!(
        int_fold(1, 65, BinaryOperator::ShiftLeft),
        Some(Constant::Integer(2))
    );
    assert_eq!(
        int_fold(-16, 2, BinaryOperator::ShiftRight),
        Some(Constant::Integer(-4))
    );
    assert_eq!(
        int_fold(64, 66, BinaryOperator::ShiftRight),
        Some(Constant::Integer(16))
    );

    // Integer comparisons
    assert_eq!(
        int_fold(i64::MIN, i64::MAX, BinaryOperator::Less),
        Some(Constant::Bool(true))
    );
    assert_eq!(
        int_fold(i64::MIN, i64::MAX, BinaryOperator::Greater),
        Some(Constant::Bool(false))
    );
    assert_eq!(
        int_fold(99, 99, BinaryOperator::LessEqual),
        Some(Constant::Bool(true))
    );
    assert_eq!(
        int_fold(99, 99, BinaryOperator::GreaterEqual),
        Some(Constant::Bool(true))
    );
    assert_eq!(
        int_fold(99, 99, BinaryOperator::Equal),
        Some(Constant::Bool(true))
    );
    assert_eq!(
        int_fold(99, 98, BinaryOperator::NotEqual),
        Some(Constant::Bool(true))
    );

    // Logical ops on integers must NOT fold
    assert_eq!(int_fold(1, 0, BinaryOperator::LogicalAnd), None);
    assert_eq!(int_fold(1, 0, BinaryOperator::LogicalOr), None);

    // Unary ops on integers
    assert_eq!(
        fold_unary(UnaryOperator::Negate, &Constant::Integer(42)),
        Some(Constant::Integer(-42))
    );
    assert_eq!(
        fold_unary(UnaryOperator::Negate, &Constant::Integer(i64::MIN)),
        Some(Constant::Integer(i64::MIN))
    );
    assert_eq!(fold_unary(UnaryOperator::Not, &Constant::Integer(1)), None);
    assert_eq!(
        fold_unary(UnaryOperator::Plus, &Constant::Integer(42)),
        Some(Constant::Integer(42))
    );
    assert_eq!(
        fold_unary(UnaryOperator::BitNot, &Constant::Integer(0)),
        Some(Constant::Integer(-1))
    );
    assert_eq!(
        fold_unary(UnaryOperator::BitNot, &Constant::Integer(-1)),
        Some(Constant::Integer(0))
    );
}

#[test]
fn const_fold_float_ieee754_exactness() {
    let (program, _) = compile_mir("def main() -> Int:\n    return 0\n");
    let float_fold = |l: f64, r: f64, op: BinaryOperator| {
        fold_binary(
            &program,
            op,
            &Constant::FloatBits(l.to_bits()),
            &Constant::FloatBits(r.to_bits()),
        )
    };
    let bits_of = |c: Option<Constant>| match c {
        Some(Constant::FloatBits(b)) => b,
        other => panic!("expected FloatBits, got {other:?}"),
    };

    // Signed zero arithmetic and unary negation
    assert_eq!(
        bits_of(float_fold(-0.0, 0.0, BinaryOperator::Add)),
        0.0_f64.to_bits()
    );
    assert_eq!(
        bits_of(float_fold(-0.0, 1.0, BinaryOperator::Multiply)),
        (-0.0_f64).to_bits()
    );
    assert_eq!(
        fold_unary(
            UnaryOperator::Negate,
            &Constant::FloatBits(0.0_f64.to_bits())
        ),
        Some(Constant::FloatBits((-0.0_f64).to_bits()))
    );
    assert_eq!(
        fold_unary(
            UnaryOperator::Negate,
            &Constant::FloatBits((-0.0_f64).to_bits())
        ),
        Some(Constant::FloatBits(0.0_f64.to_bits()))
    );

    // Inf, -Inf, NaN
    assert_eq!(
        bits_of(float_fold(1.0, 0.0, BinaryOperator::Divide)),
        f64::INFINITY.to_bits()
    );
    assert_eq!(
        bits_of(float_fold(-1.0, 0.0, BinaryOperator::Divide)),
        f64::NEG_INFINITY.to_bits()
    );
    assert!(f64::from_bits(bits_of(float_fold(0.0, 0.0, BinaryOperator::Divide))).is_nan());
    assert!(
        f64::from_bits(bits_of(float_fold(
            f64::INFINITY,
            f64::INFINITY,
            BinaryOperator::Subtract
        )))
        .is_nan()
    );
    assert_eq!(
        bits_of(float_fold(5.5, 2.0, BinaryOperator::Modulo)),
        (5.5_f64 % 2.0_f64).to_bits()
    );

    // IEEE comparisons with NaN and Inf
    assert_eq!(
        float_fold(f64::NAN, f64::NAN, BinaryOperator::Equal),
        Some(Constant::Bool(false))
    );
    assert_eq!(
        float_fold(f64::NAN, f64::NAN, BinaryOperator::NotEqual),
        Some(Constant::Bool(true))
    );
    assert_eq!(
        float_fold(f64::NAN, 0.0, BinaryOperator::Less),
        Some(Constant::Bool(false))
    );
    assert_eq!(
        float_fold(f64::NAN, 0.0, BinaryOperator::GreaterEqual),
        Some(Constant::Bool(false))
    );
    assert_eq!(
        float_fold(f64::INFINITY, f64::INFINITY, BinaryOperator::Equal),
        Some(Constant::Bool(true))
    );
    assert_eq!(
        float_fold(f64::NEG_INFINITY, f64::INFINITY, BinaryOperator::Less),
        Some(Constant::Bool(true))
    );
    assert_eq!(
        float_fold(-0.0, 0.0, BinaryOperator::Equal),
        Some(Constant::Bool(true))
    );

    // Invalid ops on floats must return None
    assert_eq!(float_fold(1.0, 2.0, BinaryOperator::BitAnd), None);
    assert_eq!(float_fold(1.0, 2.0, BinaryOperator::ShiftLeft), None);
    assert_eq!(
        fold_unary(UnaryOperator::Not, &Constant::FloatBits(0.0_f64.to_bits())),
        None
    );
}

#[test]
fn const_fold_bool_char_string_and_mixed_type_boundaries() {
    let source = concat!(
        "def main() -> Int:\n",
        "    s1 := \"alpha\"\n",
        "    s2 := \"alpha\"\n",
        "    s3 := \"beta\"\n",
        "    c1 := 'a'\n",
        "    c2 := 'z'\n",
        "    return 0\n",
    );
    let (program, _) = compile_mir(source);

    // Bool matrix
    for l in [false, true] {
        for r in [false, true] {
            assert_eq!(
                fold_binary(
                    &program,
                    BinaryOperator::LogicalAnd,
                    &Constant::Bool(l),
                    &Constant::Bool(r)
                ),
                Some(Constant::Bool(l && r))
            );
            assert_eq!(
                fold_binary(
                    &program,
                    BinaryOperator::LogicalOr,
                    &Constant::Bool(l),
                    &Constant::Bool(r)
                ),
                Some(Constant::Bool(l || r))
            );
            assert_eq!(
                fold_binary(
                    &program,
                    BinaryOperator::Equal,
                    &Constant::Bool(l),
                    &Constant::Bool(r)
                ),
                Some(Constant::Bool(l == r))
            );
            assert_eq!(
                fold_binary(
                    &program,
                    BinaryOperator::NotEqual,
                    &Constant::Bool(l),
                    &Constant::Bool(r)
                ),
                Some(Constant::Bool(l != r))
            );
            assert_eq!(
                fold_binary(
                    &program,
                    BinaryOperator::Add,
                    &Constant::Bool(l),
                    &Constant::Bool(r)
                ),
                None
            );
        }
        assert_eq!(
            fold_unary(UnaryOperator::Not, &Constant::Bool(l)),
            Some(Constant::Bool(!l))
        );
        assert_eq!(fold_unary(UnaryOperator::Negate, &Constant::Bool(l)), None);
    }

    // Char comparisons
    let chars = assigned_constants(&program, |c| matches!(c, Constant::Character { .. }));
    assert_eq!(chars.len(), 2);
    assert_eq!(
        fold_binary(&program, BinaryOperator::Less, &chars[0], &chars[1]),
        Some(Constant::Bool(true))
    );
    assert_eq!(
        fold_binary(&program, BinaryOperator::GreaterEqual, &chars[0], &chars[1]),
        Some(Constant::Bool(false))
    );
    assert_eq!(
        fold_binary(&program, BinaryOperator::Equal, &chars[0], &chars[0]),
        Some(Constant::Bool(true))
    );
    assert_eq!(
        fold_binary(&program, BinaryOperator::Add, &chars[0], &chars[1]),
        None
    );

    // String equality folds, concatenation NEVER folds
    let strings = assigned_constants(&program, |c| matches!(c, Constant::String { .. }));
    assert_eq!(strings.len(), 3);
    assert_eq!(
        fold_binary(&program, BinaryOperator::Equal, &strings[0], &strings[1]),
        Some(Constant::Bool(true))
    );
    assert_eq!(
        fold_binary(&program, BinaryOperator::Equal, &strings[0], &strings[2]),
        Some(Constant::Bool(false))
    );
    assert_eq!(
        fold_binary(&program, BinaryOperator::NotEqual, &strings[0], &strings[2]),
        Some(Constant::Bool(true))
    );
    assert_eq!(
        fold_binary(&program, BinaryOperator::Add, &strings[0], &strings[2]),
        None
    );
    assert_eq!(
        fold_binary(&program, BinaryOperator::Less, &strings[0], &strings[2]),
        None
    );

    // `is_primitive_constant` excludes strings and includes all scalar constants
    assert!(!is_primitive_constant(&strings[0]));
    assert!(is_primitive_constant(&chars[0]));
    assert!(is_primitive_constant(&Constant::Integer(1)));
    assert!(is_primitive_constant(&Constant::FloatBits(
        1.0_f64.to_bits()
    )));
    assert!(is_primitive_constant(&Constant::Bool(true)));

    // Mixed types never fold
    assert_eq!(
        fold_binary(
            &program,
            BinaryOperator::Equal,
            &Constant::Integer(1),
            &Constant::Bool(true)
        ),
        None
    );
    assert_eq!(
        fold_binary(
            &program,
            BinaryOperator::Add,
            &Constant::Integer(1),
            &Constant::FloatBits(1.0_f64.to_bits())
        ),
        None
    );
    assert_eq!(
        fold_binary(&program, BinaryOperator::Equal, &chars[0], &strings[0]),
        None
    );
}

#[test]
fn const_fold_preserves_division_by_zero_and_modulo_by_zero_runtime_errors() {
    for expr in ["a / b", "a % b"] {
        let source = format!("def main() -> Int:\n    a := 19\n    b := 0\n    return {expr}\n");
        let (unopt, types) = compile_mir(&source);
        let entry = entry_function(&unopt);
        let err_before = run_legacy_err(&unopt, &types, entry);
        assert_eq!(err_before.code(), "E4303");
        assert!(matches!(
            err_before.kind,
            InterpreterErrorKind::DivisionByZero
        ));

        let mut opt = unopt.clone();
        run_optimization(&mut opt, &types, OptimizationLevel::O3).unwrap();
        let err_after = run_legacy_err(&opt, &types, entry);
        assert_eq!(err_after.code(), "E4303");
        assert!(matches!(
            err_after.kind,
            InterpreterErrorKind::DivisionByZero
        ));
    }
}

// ── Section 2: ConstPropPass & single_def_constants Exclusions ──────────────

#[test]
fn single_def_constants_excludes_params_captures_mutated_place_roots_and_strings() {
    let source = concat!(
        "struct Cell:\n",
        "    val: Int\n",
        "\n",
        "def worker(param_k: Int) -> Int:\n",
        "    imm := 11\n",
        "    mut reassigned := 22\n",
        "    reassigned = 33\n",
        "    mut box_cell := Cell(44)\n",
        "    box_cell.val = 55\n",
        "    cap_val := 66\n",
        "    s := \"never_prop\"\n",
        "    f := fn() -> Int:\n",
        "        return cap_val + 1\n",
        "    return param_k + imm + reassigned + box_cell.val + f() + str_len(s)\n",
        "\n",
        "def main() -> Int:\n",
        "    return worker(5)\n",
    );
    let (program, _types) = compile_mir(source);
    let worker_id = top_level_function(&program, 0);
    let worker_fn = program.function(worker_id).unwrap();
    let const_map = single_def_constants(&program, worker_fn);

    // Every entry in `const_map` must be User or Temporary, primitive, and not a place root
    let place_roots: BTreeSet<MirLocalId> = program.places().map(|(_, p)| p.root).collect();
    for (local_id, constant) in &const_map {
        let local = program.local(*local_id).unwrap();
        assert!(
            matches!(local.kind, MirLocalKind::User | MirLocalKind::Temporary),
            "parameter or capture leaked into single_def_constants: {local:?}"
        );
        assert!(
            is_primitive_constant(constant),
            "string constant leaked into single_def_constants"
        );
        assert!(
            !place_roots.contains(local_id),
            "place root leaked into single_def_constants"
        );
    }

    // `imm` (11) and `cap_val` (66) are single-def in `worker` (closure captures
    // read them, and inside the closure body `cap_val` is a Capture local).
    assert!(
        const_map.values().any(|c| *c == Constant::Integer(11)),
        "expected `imm := 11` in single_def_constants"
    );
    // `reassigned` (22 / 33) must NOT be in single_def_constants
    assert!(
        !const_map
            .values()
            .any(|c| *c == Constant::Integer(22) || *c == Constant::Integer(33)),
        "mutated local must not appear in single_def_constants"
    );

    // Closure function must NOT have its Capture local in single_def_constants
    let closure_id = program
        .functions()
        .find(|(_, f)| f.kind == MirFunctionKind::Closure)
        .map(|(id, _)| id)
        .expect("closure function");
    let closure_fn = program.function(closure_id).unwrap();
    let closure_consts = single_def_constants(&program, closure_fn);
    for local_id in closure_consts.keys() {
        assert_ne!(
            program.local(*local_id).unwrap().kind,
            MirLocalKind::Capture
        );
    }

    // Full optimization equivalence
    assert_opt_equivalence(
        source,
        OptimizationLevel::O2,
        ExecutionValue::Int(5 + 11 + 33 + 55 + 67 + 10),
    );
}

#[test]
fn const_prop_propagates_across_cast_store_branch_and_return() {
    let source = concat!(
        "struct Holder:\n",
        "    count: Int\n",
        "    ratio: Float\n",
        "\n",
        "def main() -> Int:\n",
        "    k := 7\n",
        "    flag := true\n",
        "    mut h := Holder(0, 0.0)\n",
        "    h.count = k\n",
        "    f := 2.5\n",
        "    h.ratio = f\n",
        "    if flag:\n",
        "        res := h.count + 3\n",
        "        return res\n",
        "    else:\n",
        "        return 999\n",
    );
    let (opt, _types, _before, _after) =
        assert_opt_equivalence(source, OptimizationLevel::O2, ExecutionValue::Int(10));

    // After O2, `flag` propagated into Branch and folded to Goto, leaving 0 branches.
    assert_eq!(count_branches(&opt), 0);
    // And `h.count = k` had `k` propagated into the Store value operand!
    let has_const_store = opt.instructions().any(|(_, inst)| {
        matches!(
            inst.kind,
            InstructionKind::Store {
                value: Operand::Constant(Constant::Integer(7)),
                ..
            }
        )
    });
    assert!(
        has_const_store,
        "expected ConstPropPass to propagate `k := 7` into `InstructionKind::Store`"
    );
}

#[test]
fn const_prop_does_not_corrupt_aggregate_or_closure_operand_lists() {
    let source = concat!(
        "enum Boxed:\n",
        "    Pair(a: Int, b: Int)\n",
        "    Empty\n",
        "\n",
        "def main() -> Int:\n",
        "    a := 10\n",
        "    b := 20\n",
        "    t := (a, b)\n",
        "    xs := [a, b, a + b]\n",
        "    e := Boxed.Pair(a, b)\n",
        "    f := fn(x: Int) -> Int:\n",
        "        return x + a + b\n",
        "    mut acc := t.0 + t.1 + xs[2] + f(5)\n",
        "    match e:\n",
        "        Boxed.Pair(p, q):\n",
        "            acc = acc + p + q\n",
        "        Boxed.Empty:\n",
        "            acc = acc + 0\n",
        "    return acc\n",
    );
    assert_opt_equivalence(source, OptimizationLevel::O3, ExecutionValue::Int(125));
}

// ── Section 3: BranchFoldPass & CFG Simplification ──────────────────────────

#[test]
fn branch_fold_simplifies_nested_constant_branches_and_bypasses_dead_traps() {
    let source = concat!(
        "def main() -> Int:\n",
        "    a := 4\n",
        "    b := 5\n",
        "    cond1 := (a + b) == 9\n",
        "    cond2 := (a * b) < 10\n",
        "    if cond1:\n",
        "        if cond2:\n",
        "            trap := 1 / 0\n",
        "            return trap\n",
        "        else:\n",
        "            return 77\n",
        "    else:\n",
        "        dead := 42 / 0\n",
        "        return dead\n",
    );
    let (unopt, types) = compile_mir(source);
    assert_eq!(count_branches(&unopt), 2);
    let gotos_before = count_gotos(&unopt);

    let (opt, _, _, _) =
        assert_opt_equivalence(source, OptimizationLevel::O2, ExecutionValue::Int(77));
    assert_eq!(
        count_branches(&opt),
        0,
        "all constant branches must fold to gotos"
    );
    assert_eq!(count_gotos(&opt), gotos_before + 2);
    assert!(verify_mir(&opt, &types).is_empty());
}

#[test]
fn branch_fold_leaves_dynamic_branches_intact() {
    let source = concat!(
        "def classify(n: Int) -> Int:\n",
        "    if n > 10:\n",
        "        if true:\n",
        "            return n + 1\n",
        "        else:\n",
        "            return 0\n",
        "    else:\n",
        "        return n - 1\n",
        "\n",
        "def main() -> Int:\n",
        "    return classify(15) + classify(5)\n",
    );
    let (unopt, _) = compile_mir(source);
    assert_eq!(count_branches(&unopt), 2);

    let (opt, _, _, _) =
        assert_opt_equivalence(source, OptimizationLevel::O2, ExecutionValue::Int(20));
    // Inner `if true:` folds, outer `if n > 10:` remains a dynamic Branch
    assert_eq!(count_branches(&opt), 1);
}

#[test]
fn branch_fold_works_on_elif_chains_with_computed_boolean_expressions() {
    let source = concat!(
        "def main() -> Int:\n",
        "    x := 3\n",
        "    y := 4\n",
        "    c1 := (x * x + y * y) == 24\n",
        "    c2 := (x + y) == 8\n",
        "    c3 := (x * y) == 12\n",
        "    if c1:\n",
        "        return 100\n",
        "    elif c2:\n",
        "        return 200\n",
        "    elif c3:\n",
        "        return 300\n",
        "    else:\n",
        "        return 400\n",
    );
    let (unopt, _) = compile_mir(source);
    assert_eq!(count_branches(&unopt), 3);

    let (opt, _, _, _) =
        assert_opt_equivalence(source, OptimizationLevel::O3, ExecutionValue::Int(300));
    assert_eq!(count_branches(&opt), 0);
}

// ── Section 4: InlinePass, Whole-Program LTO & Profile-Guided Inlining ──────

#[test]
fn inline_pass_cross_module_lto_and_constant_cascade() {
    let helper_src = concat!(
        "def square(n: Int) -> Int:\n",
        "    return n * n\n",
        "\n",
        "def bias() -> Int:\n",
        "    return 7\n",
    );
    let main_src = concat!(
        "from helper import square, bias\n",
        "\n",
        "def main() -> Int:\n",
        "    x := square(6)\n",
        "    y := bias()\n",
        "    if (x + y) == 43:\n",
        "        return x + y\n",
        "    else:\n",
        "        return 0\n",
    );
    let (unopt, types) = compile_mir_files(
        &[
            ("/passes/helper.lpp", helper_src),
            ("/passes/main.lpp", main_src),
        ],
        "/passes/main.lpp",
    );
    let entry = entry_function(&unopt);
    let before = run_legacy(&unopt, &types, entry);
    assert_eq!(before.value, ExecutionValue::Int(43));
    let before_calls = count_direct_calls(&unopt);
    assert_eq!(before_calls, 2);

    // Without LTO, calls remain
    let mut no_lto = unopt.clone();
    run_optimization_with_lto(&mut no_lto, &types, OptimizationLevel::O3, false).unwrap();
    assert_eq!(count_direct_calls(&no_lto), 2);

    // With LTO at O3: InlinePass inlines both cross-module helpers, then subsequent
    // sweeps fold `6 * 6` -> `36`, `36 + 7` -> `43`, and fold the `if` branch!
    let mut with_lto = unopt.clone();
    run_optimization_with_lto(&mut with_lto, &types, OptimizationLevel::O3, true).unwrap();
    assert!(verify_mir(&with_lto, &types).is_empty());
    assert_eq!(count_direct_calls(&with_lto), 0);
    assert_eq!(count_branches(&with_lto), 0);

    let after = run_legacy(&with_lto, &types, entry);
    assert_eq!(after.value, ExecutionValue::Int(43));
    assert!(after.stats.steps < before.stats.steps);
    assert!(after.stats.calls < before.stats.calls);
}

#[test]
fn inline_pass_profile_guided_selectivity_and_lto_precedence() {
    let source = concat!(
        "def hot_fn(x: Int) -> Int:\n",
        "    return x + 10\n",
        "\n",
        "def cold_fn(x: Int) -> Int:\n",
        "    return x + 20\n",
        "\n",
        "def main() -> Int:\n",
        "    return hot_fn(1) + cold_fn(2)\n",
    );
    let (unopt, types) = compile_mir(source);
    let hot_id = top_level_function(&unopt, 0);
    let entry = entry_function(&unopt);
    assert_eq!(count_direct_calls(&unopt), 2);

    // Only `hot_fn` in hot set -> 1 call remains (`cold_fn`)
    let mut pgo = unopt.clone();
    run_optimization_with_profile(
        &mut pgo,
        &types,
        OptimizationLevel::O2,
        false,
        BTreeSet::from([hot_id]),
    )
    .unwrap();
    assert_eq!(count_direct_calls(&pgo), 1);
    assert_eq!(
        run_legacy(&pgo, &types, entry).value,
        ExecutionValue::Int(33)
    );

    // Empty hot set with `enable_lto = false` -> 2 calls remain
    let mut cold_only = unopt.clone();
    run_optimization_with_profile(
        &mut cold_only,
        &types,
        OptimizationLevel::O2,
        false,
        BTreeSet::new(),
    )
    .unwrap();
    assert_eq!(count_direct_calls(&cold_only), 2);

    // Explicit `enable_lto = true` overrides empty hot set -> 0 calls remain
    let mut lto_override = unopt.clone();
    run_optimization_with_profile(
        &mut lto_override,
        &types,
        OptimizationLevel::O2,
        true,
        BTreeSet::new(),
    )
    .unwrap();
    assert_eq!(count_direct_calls(&lto_override), 0);
    assert_eq!(
        run_legacy(&lto_override, &types, entry).value,
        ExecutionValue::Int(33)
    );
}

#[test]
fn inline_pass_gates_reject_recursive_multiblock_closure_async_and_non_scalar() {
    let source = concat!(
        "struct Pair:\n",
        "    a: Int\n",
        "    b: Int\n",
        "\n",
        "def fact(n: Int) -> Int:\n",
        "    if n <= 1:\n",
        "        return 1\n",
        "    return n * fact(n - 1)\n",
        "\n",
        "def multi_block(x: Int) -> Int:\n",
        "    if x > 0:\n",
        "        return x\n",
        "    return -x\n",
        "\n",
        "def str_id(s: Str) -> Str:\n",
        "    return s\n",
        "\n",
        "def pair_sum(p: Pair) -> Int:\n",
        "    return p.a + p.b\n",
        "\n",
        "async def async_num() -> Int:\n",
        "    return 5\n",
        "\n",
        "def scalar_ok(b: Bool) -> Bool:\n",
        "    return !b\n",
        "\n",
        "def main() -> Int:\n",
        "    f1 := fact(4)\n",
        "    f2 := multi_block(-3)\n",
        "    s := str_id(\"ok\")\n",
        "    p := pair_sum(Pair(2, 3))\n",
        "    b := scalar_ok(false)\n",
        "    mut k := 0\n",
        "    if b:\n",
        "        k = 1\n",
        "    t := async_num()\n",
        "    a := t.await\n",
        "    return f1 + f2 + str_len(s) + p + k + a\n",
    );
    let (unopt, types) = compile_mir(source);
    let calls_before = count_direct_calls(&unopt);
    let entry = entry_function(&unopt);
    let before = run_legacy(&unopt, &types, entry);

    let mut opt = unopt.clone();
    run_optimization_with_lto(&mut opt, &types, OptimizationLevel::O2, true).unwrap();
    let calls_after = count_direct_calls(&opt);
    // Only `scalar_ok` is eligible (1 call removed); fact, multi_block, str_id,
    // pair_sum, and async_num must remain as calls.
    assert_eq!(calls_after, calls_before - 1);
    assert!(verify_mir(&opt, &types).is_empty());
    let after = run_legacy(&opt, &types, entry);
    assert_eq!(after.value, before.value);
}

#[test]
fn inline_pass_handles_direct_scalar_types_bool_float_char_and_rawptr() {
    let source = concat!(
        "def flip(b: Bool) -> Bool:\n",
        "    return !b\n",
        "\n",
        "def half(x: Float) -> Float:\n",
        "    return x * 0.5\n",
        "\n",
        "def same_char(a: Char, b: Char) -> Bool:\n",
        "    return a == b\n",
        "\n",
        "def pass_ptr(p: Ptr[U8]) -> Ptr[U8]:\n",
        "    return p\n",
        "\n",
        "def main() -> Int:\n",
        "    b := flip(false)\n",
        "    h := half(8.0)\n",
        "    c := same_char('k', 'k')\n",
        "    p := pass_ptr(ptr_null[U8]())\n",
        "    if b && c && ptr_is_null(p) && h == 4.0:\n",
        "        return 1\n",
        "    return 0\n",
    );
    let (unopt, types) = compile_mir(source);
    assert_eq!(count_direct_calls(&unopt), 4);
    let mut opt = unopt.clone();
    run_optimization_with_lto(&mut opt, &types, OptimizationLevel::O2, true).unwrap();
    assert_eq!(count_direct_calls(&opt), 0);
    assert_eq!(
        run_legacy(&opt, &types, entry_function(&opt)).value,
        ExecutionValue::Int(1)
    );
}

// ── Section 5: Custom MirPasses & PassManager Error/Ordering Contracts ───────

/// Custom algebraic peephole pass: simplifies identity arithmetic (`x + 0`,
/// `0 + x`, `x - 0`, `x * 1`, `1 * x`, `x * 0`, `x - x`, `x ^ x`, `x & 0`, `x | 0`)
/// while preserving `CORE_MIR_INVARIANTS`.
struct AlgebraicPeepholePass;

impl MirPass for AlgebraicPeepholePass {
    fn name(&self) -> &'static str {
        "algebraic-peephole"
    }

    fn preserved(&self) -> &'static [MirInvariant] {
        CORE_MIR_INVARIANTS
    }

    fn run(
        &mut self,
        program: &mut MirProgram,
        _context: &PassContext<'_>,
    ) -> Result<(), PassFailure> {
        let inst_ids: Vec<InstructionId> = program.instructions().map(|(id, _)| id).collect();
        for id in inst_ids {
            let Some(inst) = program.instruction(id) else {
                continue;
            };
            let InstructionKind::Assign {
                target,
                value:
                    Rvalue::Binary {
                        left,
                        operator,
                        right,
                    },
            } = inst.kind
            else {
                continue;
            };
            let replacement = match (operator, left, right) {
                (BinaryOperator::Add, other, Operand::Constant(Constant::Integer(0)))
                | (BinaryOperator::Add, Operand::Constant(Constant::Integer(0)), other)
                | (BinaryOperator::Subtract, other, Operand::Constant(Constant::Integer(0)))
                | (BinaryOperator::Multiply, other, Operand::Constant(Constant::Integer(1)))
                | (BinaryOperator::Multiply, Operand::Constant(Constant::Integer(1)), other)
                | (BinaryOperator::BitOr, other, Operand::Constant(Constant::Integer(0)))
                | (BinaryOperator::BitOr, Operand::Constant(Constant::Integer(0)), other) => {
                    Some(Rvalue::Use(other))
                }
                (BinaryOperator::Multiply, _, Operand::Constant(Constant::Integer(0)))
                | (BinaryOperator::Multiply, Operand::Constant(Constant::Integer(0)), _)
                | (BinaryOperator::BitAnd, _, Operand::Constant(Constant::Integer(0)))
                | (BinaryOperator::BitAnd, Operand::Constant(Constant::Integer(0)), _) => {
                    Some(Rvalue::Use(Operand::Constant(Constant::Integer(0))))
                }
                (BinaryOperator::Subtract, Operand::Copy(l), Operand::Copy(r))
                | (BinaryOperator::BitXor, Operand::Copy(l), Operand::Copy(r))
                    if l == r =>
                {
                    Some(Rvalue::Use(Operand::Constant(Constant::Integer(0))))
                }
                _ => None,
            };
            if let Some(new_rvalue) = replacement
                && let Some(inst_mut) = program.instruction_mut(id)
            {
                inst_mut.kind = InstructionKind::Assign {
                    target,
                    value: new_rvalue,
                };
            }
        }
        Ok(())
    }
}

/// Custom single-def scalar copy-propagation pass: when a non-place-root User or
/// Temporary scalar local is defined exactly once as `Use(Copy(src))` where `src`
/// is itself a single-def scalar User/Temporary/Parameter, rewrite `Copy(dst)` -> `Copy(src)`.
struct ScalarCopyPropPass;

impl MirPass for ScalarCopyPropPass {
    fn name(&self) -> &'static str {
        "scalar-copy-prop"
    }

    fn preserved(&self) -> &'static [MirInvariant] {
        CORE_MIR_INVARIANTS
    }

    fn run(
        &mut self,
        program: &mut MirProgram,
        context: &PassContext<'_>,
    ) -> Result<(), PassFailure> {
        let place_roots: BTreeSet<MirLocalId> = program.places().map(|(_, p)| p.root).collect();
        let fn_ids: Vec<MirFunctionId> = program.functions().map(|(id, _)| id).collect();
        for fn_id in fn_ids {
            let Some(func) = program.function(fn_id) else {
                continue;
            };
            let blocks = program.function_blocks(func).to_vec();
            let mut assign_counts: BTreeMap<MirLocalId, usize> = BTreeMap::new();
            let mut copy_defs: BTreeMap<MirLocalId, MirLocalId> = BTreeMap::new();
            for &bid in &blocks {
                let Some(block) = program.block(bid) else {
                    continue;
                };
                for &iid in program.block_instructions(block) {
                    if let Some(inst) = program.instruction(iid)
                        && let InstructionKind::Assign { target, value } = inst.kind
                    {
                        let c = assign_counts.entry(target).or_insert(0);
                        *c += 1;
                        if *c == 1 {
                            if let Rvalue::Use(Operand::Copy(src)) = value {
                                copy_defs.insert(target, src);
                            }
                        } else {
                            copy_defs.remove(&target);
                        }
                    }
                }
            }
            let valid_scalar = |local: MirLocalId| -> bool {
                let Some(info) = program.local(local) else {
                    return false;
                };
                !place_roots.contains(&local)
                    && matches!(
                        context.types.kind(info.ty),
                        lpp_types::TypeKind::Primitive(
                            lpp_types::PrimitiveType::Int
                                | lpp_types::PrimitiveType::Bool
                                | lpp_types::PrimitiveType::Float
                                | lpp_types::PrimitiveType::Char
                        )
                    )
            };
            copy_defs.retain(|dst, src| {
                assign_counts.get(dst) == Some(&1)
                    && assign_counts.get(src).copied().unwrap_or(0) <= 1
                    && program.local(*dst).is_some_and(|l| {
                        matches!(l.kind, MirLocalKind::User | MirLocalKind::Temporary)
                    })
                    && valid_scalar(*dst)
                    && valid_scalar(*src)
            });
            if copy_defs.is_empty() {
                continue;
            }
            let rewrite_op = |op: Operand| -> Operand {
                if let Operand::Copy(local) = op
                    && let Some(&src) = copy_defs.get(&local)
                {
                    return Operand::Copy(src);
                }
                op
            };
            for &bid in &blocks {
                let Some(block) = program.block(bid) else {
                    continue;
                };
                let iids = program.block_instructions(block).to_vec();
                for iid in iids {
                    let Some(kind) = program.instruction(iid).map(|i| i.kind) else {
                        continue;
                    };
                    let new_kind = match kind {
                        InstructionKind::Assign { target, value } => {
                            let nv = match value {
                                Rvalue::Use(op) => Rvalue::Use(rewrite_op(op)),
                                Rvalue::Unary { operator, operand } => Rvalue::Unary {
                                    operator,
                                    operand: rewrite_op(operand),
                                },
                                Rvalue::Binary {
                                    left,
                                    operator,
                                    right,
                                } => Rvalue::Binary {
                                    left: rewrite_op(left),
                                    operator,
                                    right: rewrite_op(right),
                                },
                                other => other,
                            };
                            InstructionKind::Assign { target, value: nv }
                        }
                        InstructionKind::Store { place, value } => InstructionKind::Store {
                            place,
                            value: rewrite_op(value),
                        },
                    };
                    if let Some(inst_mut) = program.instruction_mut(iid) {
                        inst_mut.kind = new_kind;
                    }
                }
                if let Some(block) = program.block(bid)
                    && let Terminator::Return(Some(op)) = block.terminator
                {
                    let new_op = rewrite_op(op);
                    if new_op != op
                        && let Some(bmut) = program.block_mut(bid)
                    {
                        bmut.terminator = Terminator::Return(Some(new_op));
                    }
                }
            }
        }
        Ok(())
    }
}

/// Custom unreachable-block & dead-store audit pass: computes BFS reachability
/// from every function's entry block, replaces unreachable `Branch`/`SwitchEnum`
/// terminators with `Terminator::Unreachable`, and threads `Goto -> single Goto`
/// trampolines.
struct CfgSimplifyAuditPass {
    unreachable_blocks_found: Rc<Cell<usize>>,
}

impl MirPass for CfgSimplifyAuditPass {
    fn name(&self) -> &'static str {
        "cfg-simplify-audit"
    }

    fn preserved(&self) -> &'static [MirInvariant] {
        CORE_MIR_INVARIANTS
    }

    fn run(
        &mut self,
        program: &mut MirProgram,
        _context: &PassContext<'_>,
    ) -> Result<(), PassFailure> {
        let mut unreachable_total = 0usize;
        let fn_ids: Vec<MirFunctionId> = program.functions().map(|(id, _)| id).collect();
        for fn_id in fn_ids {
            let Some(func) = program.function(fn_id) else {
                continue;
            };
            let all_blocks = program.function_blocks(func).to_vec();
            let mut visited = BTreeSet::new();
            let mut queue = VecDeque::new();
            visited.insert(func.entry);
            queue.push_back(func.entry);
            while let Some(bid) = queue.pop_front() {
                let Some(block) = program.block(bid) else {
                    continue;
                };
                match block.terminator {
                    Terminator::Goto(t) => {
                        if visited.insert(t) {
                            queue.push_back(t);
                        }
                    }
                    Terminator::Branch {
                        then_block,
                        else_block,
                        ..
                    } => {
                        if visited.insert(then_block) {
                            queue.push_back(then_block);
                        }
                        if visited.insert(else_block) {
                            queue.push_back(else_block);
                        }
                    }
                    Terminator::SwitchEnum { targets, .. } => {
                        for &t in program.switch_targets(targets) {
                            if visited.insert(t) {
                                queue.push_back(t);
                            }
                        }
                    }
                    Terminator::Return(_) | Terminator::Unreachable => {}
                }
            }
            for bid in all_blocks {
                if !visited.contains(&bid) {
                    unreachable_total += 1;
                    if let Some(bmut) = program.block_mut(bid) {
                        bmut.terminator = Terminator::Unreachable;
                    }
                }
            }
        }
        self.unreachable_blocks_found.set(unreachable_total);
        Ok(())
    }
}

#[test]
fn custom_copy_prop_and_algebraic_peephole_passes_compose_with_builtins() {
    let source = concat!(
        "def compute(x: Int) -> Int:\n",
        "    a := x\n",
        "    b := a + 0\n",
        "    c := b * 1\n",
        "    d := c - c\n",
        "    e := x ^ x\n",
        "    z := x * 0\n",
        "    return c + d + e + z\n",
        "\n",
        "def main() -> Int:\n",
        "    return compute(21) + compute(0)\n",
    );
    let (unopt, types) = compile_mir(source);
    let entry = entry_function(&unopt);
    let before = run_legacy(&unopt, &types, entry);
    assert_eq!(before.value, ExecutionValue::Int(21));
    let bin_before = count_binary_rvalues(&unopt);

    let mut opt = unopt.clone();
    let mut manager = PassManager::new();
    manager.push(ScalarCopyPropPass);
    manager.push(AlgebraicPeepholePass);
    manager.push(ConstFoldPass);
    manager.push(ConstPropPass);
    manager.push(AlgebraicPeepholePass);
    manager.push(BranchFoldPass);
    let outcome = manager.run(&mut opt, &types).unwrap();
    assert_eq!(outcome.executed.len(), 6);
    assert!(verify_mir(&opt, &types).is_empty());

    let bin_after = count_binary_rvalues(&opt);
    assert!(
        bin_after < bin_before,
        "expected peephole + constfold to reduce binary instructions: {bin_before} -> {bin_after}"
    );
    let after = run_legacy(&opt, &types, entry);
    assert_eq!(after.value, before.value);
    assert!(after.stats.steps <= before.stats.steps);
}

#[test]
fn cfg_simplify_audit_detects_unreachable_blocks_after_branch_fold() {
    let source = concat!(
        "def main() -> Int:\n",
        "    flag := false\n",
        "    if flag:\n",
        "        x := 100\n",
        "        y := x * 2\n",
        "        return y\n",
        "    else:\n",
        "        return 12\n",
    );
    let (mut program, types) = compile_mir(source);
    let unreachable_count = Rc::new(Cell::new(0));

    let mut manager = PassManager::new();
    manager.push(ConstPropPass);
    manager.push(BranchFoldPass);
    manager.push(CfgSimplifyAuditPass {
        unreachable_blocks_found: unreachable_count.clone(),
    });
    manager.run(&mut program, &types).unwrap();

    assert!(
        unreachable_count.get() >= 1,
        "expected BranchFoldPass to orphan at least one unreachable block"
    );
    assert!(verify_mir(&program, &types).is_empty());
    assert_eq!(
        run_legacy(&program, &types, entry_function(&program)).value,
        ExecutionValue::Int(12)
    );
}

#[test]
fn pass_manager_error_codes_e4201_e4202_e4203_e4204_and_invariant_contracts() {
    let base_src = "def main() -> Int:\n    a := 10\n    b := 20\n    return a + b\n";

    // 1. E4201 InitialVerification: corrupt MIR before running PassManager
    {
        let (mut program, types) = compile_mir(base_src);
        let entry = program.blocks().next().unwrap().0;
        program.block_mut(entry).unwrap().terminator =
            Terminator::Return(Some(Operand::Constant(Constant::Bool(true))));
        let mut mgr = PassManager::new();
        mgr.push(ConstFoldPass);
        let err = mgr.run(&mut program, &types).unwrap_err();
        assert_eq!(err.code(), "E4201");
        assert_eq!(err.pass, None);
        assert!(matches!(
            err.kind,
            PassManagerErrorKind::InitialVerification(_)
        ));
    }

    // 2. E4202 UnmetPrecondition for each non-core invariant
    for required_inv in [
        MirInvariant::Ownership,
        MirInvariant::NoOwningCycles,
        MirInvariant::OwnershipBalance,
        MirInvariant::BackendLegal,
    ] {
        struct RequireSpecific(MirInvariant);
        impl MirPass for RequireSpecific {
            fn name(&self) -> &'static str {
                "require-specific"
            }
            fn required(&self) -> &'static [MirInvariant] {
                match self.0 {
                    MirInvariant::Ownership => &[MirInvariant::Ownership],
                    MirInvariant::NoOwningCycles => &[MirInvariant::NoOwningCycles],
                    MirInvariant::OwnershipBalance => &[MirInvariant::OwnershipBalance],
                    MirInvariant::BackendLegal => &[MirInvariant::BackendLegal],
                    _ => CORE_MIR_INVARIANTS,
                }
            }
            fn run(&mut self, _: &mut MirProgram, _: &PassContext<'_>) -> Result<(), PassFailure> {
                panic!("must not run with unmet precondition")
            }
        }
        let (mut program, types) = compile_mir(base_src);
        let mut mgr = PassManager::new();
        mgr.push(RequireSpecific(required_inv));
        let err = mgr.run(&mut program, &types).unwrap_err();
        assert_eq!(err.code(), "E4202");
        assert_eq!(err.pass, Some("require-specific"));
        assert_eq!(
            err.kind,
            PassManagerErrorKind::UnmetPrecondition(required_inv)
        );
    }

    // 3. E4203 PassFailed aborts before observer runs
    {
        struct FailPass;
        impl MirPass for FailPass {
            fn name(&self) -> &'static str {
                "fail-pass"
            }
            fn run(&mut self, _: &mut MirProgram, _: &PassContext<'_>) -> Result<(), PassFailure> {
                Err(PassFailure {
                    message: "deliberate failure",
                })
            }
        }
        struct NeverObserver(Rc<Cell<bool>>);
        impl MirPass for NeverObserver {
            fn name(&self) -> &'static str {
                "never-observer"
            }
            fn run(&mut self, _: &mut MirProgram, _: &PassContext<'_>) -> Result<(), PassFailure> {
                self.0.set(true);
                Ok(())
            }
        }
        let ran = Rc::new(Cell::new(false));
        let (mut program, types) = compile_mir(base_src);
        let mut mgr = PassManager::new();
        mgr.push(FailPass);
        mgr.push(NeverObserver(ran.clone()));
        let err = mgr.run(&mut program, &types).unwrap_err();
        assert_eq!(err.code(), "E4203");
        assert_eq!(err.pass, Some("fail-pass"));
        assert!(!ran.get());
    }

    // 4. E4204 InvalidPassOutput for References (E4101), ControlFlow (E4102), Types (E4103)
    {
        struct BadControlFlow;
        impl MirPass for BadControlFlow {
            fn name(&self) -> &'static str {
                "bad-cfg"
            }
            fn run(
                &mut self,
                program: &mut MirProgram,
                _: &PassContext<'_>,
            ) -> Result<(), PassFailure> {
                // Point entry to a block belonging to a different function or invalid block?
                // Let's point function's entry to second function's block -> EntryOutsideFunction (E4102)
                let fns: Vec<_> = program.functions().map(|(id, _)| id).collect();
                let other_entry = program.function(fns[1]).unwrap().entry;
                program.function_mut(fns[0]).unwrap().entry = other_entry;
                Ok(())
            }
        }
        let (mut program, types) = compile_mir(
            "def helper() -> Int:\n    return 1\ndef main() -> Int:\n    return helper()\n",
        );
        let mut mgr = PassManager::new();
        mgr.push(BadControlFlow);
        let err = mgr.run(&mut program, &types).unwrap_err();
        assert_eq!(err.code(), "E4204");
        let PassManagerErrorKind::InvalidPassOutput(diags) = err.kind else {
            panic!("expected InvalidPassOutput");
        };
        assert!(diags.iter().any(|d| d.code() == "E4102"));
    }

    // 5. Invariant establishment -> preservation -> drop when unpreserved
    {
        struct EstablishBackendLegal;
        impl MirPass for EstablishBackendLegal {
            fn name(&self) -> &'static str {
                "establish-backend-legal"
            }
            fn preserved(&self) -> &'static [MirInvariant] {
                CORE_MIR_INVARIANTS
            }
            fn established(&self) -> &'static [MirInvariant] {
                &[MirInvariant::BackendLegal]
            }
            fn run(&mut self, _: &mut MirProgram, _: &PassContext<'_>) -> Result<(), PassFailure> {
                Ok(())
            }
        }
        struct RequireBackendLegal;
        impl MirPass for RequireBackendLegal {
            fn name(&self) -> &'static str {
                "require-backend-legal"
            }
            fn required(&self) -> &'static [MirInvariant] {
                &[MirInvariant::BackendLegal]
            }
            fn preserved(&self) -> &'static [MirInvariant] {
                &[MirInvariant::BackendLegal]
            }
            fn run(&mut self, _: &mut MirProgram, _: &PassContext<'_>) -> Result<(), PassFailure> {
                Ok(())
            }
        }
        struct DropExtras;
        impl MirPass for DropExtras {
            fn name(&self) -> &'static str {
                "drop-extras"
            }
            fn run(&mut self, _: &mut MirProgram, _: &PassContext<'_>) -> Result<(), PassFailure> {
                Ok(())
            }
        }
        let (mut program, types) = compile_mir(base_src);
        let mut mgr = PassManager::new();
        mgr.push(EstablishBackendLegal);
        mgr.push(RequireBackendLegal);
        let ok = mgr.run(&mut program, &types).unwrap();
        assert!(ok.established.contains(&MirInvariant::BackendLegal));

        // Now append a pass that does not preserve BackendLegal, then require it again -> E4202
        mgr.push(DropExtras);
        mgr.push(RequireBackendLegal);
        let err = mgr.run(&mut program, &types).unwrap_err();
        assert_eq!(err.code(), "E4202");
        assert_eq!(err.pass, Some("require-backend-legal"));
    }
}

// ── Section 6: Level Budgets, Idempotence, Determinism & Core Invariants ────

#[test]
fn optimization_levels_fixed_point_budgets_and_idempotence() {
    // A 3-layer constant dependency chain:
    //   x0 := 1
    //   x1 := x0 + 1
    //   x2 := x1 + 1
    //   return x2
    let source = concat!(
        "def main() -> Int:\n",
        "    x0 := 1\n",
        "    x1 := x0 + 1\n",
        "    x2 := x1 + 1\n",
        "    return x2\n",
    );
    let (base, types) = compile_mir(source);
    let entry = entry_function(&base);
    let base_steps = run_legacy(&base, &types, entry).stats.steps;

    let mut o0 = base.clone();
    let mut o1 = base.clone();
    let mut o2 = base.clone();
    let mut o3 = base.clone();
    let mut os = base.clone();
    let mut oz = base.clone();

    run_optimization(&mut o0, &types, OptimizationLevel::O0).unwrap();
    run_optimization(&mut o1, &types, OptimizationLevel::O1).unwrap();
    run_optimization(&mut o2, &types, OptimizationLevel::O2).unwrap();
    run_optimization(&mut o3, &types, OptimizationLevel::O3).unwrap();
    run_optimization(&mut os, &types, OptimizationLevel::Os).unwrap();
    run_optimization(&mut oz, &types, OptimizationLevel::Oz).unwrap();

    // Every level preserves value == 5 and CORE_MIR_INVARIANTS
    for (lvl, prog) in [
        ("O0", &o0),
        ("O1", &o1),
        ("O2", &o2),
        ("O3", &o3),
        ("Os", &os),
        ("Oz", &oz),
    ] {
        assert!(verify_mir(prog, &types).is_empty(), "{lvl} invalid MIR");
        assert_eq!(
            run_legacy(prog, &types, entry).value,
            ExecutionValue::Int(3),
            "{lvl} wrong value"
        );
        assert!(run_legacy(prog, &types, entry).stats.steps <= base_steps);
    }

    // O3 (6 sweeps) folds the entire chain down to `Return(Some(Constant(Integer(3))))`
    assert!(
        mir_snapshot(&o3).contains("Return(Some(Constant(Integer(3))))"),
        "O3 must converge on 3-layer chain:\n{}",
        mir_snapshot(&o3)
    );

    // Idempotence: running each level a second time on its own output is a no-op
    // once converged (for O3 which reached fixed point, snapshot is identical;
    // for all levels, running to fixed point then re-running is identical).
    for level in [
        OptimizationLevel::O0,
        OptimizationLevel::O1,
        OptimizationLevel::O2,
        OptimizationLevel::O3,
        OptimizationLevel::Os,
        OptimizationLevel::Oz,
    ] {
        let mut p = base.clone();
        // Converge via O3, then verify running `level` changes nothing
        run_optimization(&mut p, &types, OptimizationLevel::O3).unwrap();
        let snap_before = mir_snapshot(&p);
        run_optimization(&mut p, &types, level).unwrap();
        assert_eq!(
            mir_snapshot(&p),
            snap_before,
            "converged MIR must be idempotent under {level:?}"
        );
    }
}

#[test]
fn core_mir_invariants_established_after_every_individual_pass() {
    let source = concat!(
        "def inc(x: Int) -> Int:\n",
        "    return x + 1\n",
        "\n",
        "def main() -> Int:\n",
        "    a := 10\n",
        "    b := inc(a)\n",
        "    c := b + 2\n",
        "    if (a < c):\n",
        "        return c\n",
        "    return 0\n",
    );
    let (mut program, types) = compile_mir(source);
    let mut mgr = PassManager::new();
    mgr.push(InlinePass::whole_program());
    mgr.push(ConstPropPass);
    mgr.push(ConstFoldPass);
    mgr.push(ConstPropPass);
    mgr.push(BranchFoldPass);
    let outcome = mgr.run(&mut program, &types).unwrap();
    assert_eq!(
        outcome.executed,
        [
            "whole-program-inline",
            "const-prop",
            "const-fold",
            "const-prop",
            "branch-fold"
        ]
    );
    for inv in CORE_MIR_INVARIANTS {
        assert!(outcome.established.contains(inv));
    }
    assert!(verify_mir(&program, &types).is_empty());
    assert_eq!(
        run_legacy(&program, &types, entry_function(&program)).value,
        ExecutionValue::Int(13)
    );
}

#[test]
fn optimization_passes_pipeline_is_deterministic_across_builds() {
    let source = concat!(
        "enum State:\n",
        "    Idle\n",
        "    Active(val: Int)\n",
        "\n",
        "def step(s: State, delta: Int) -> Int:\n",
        "    match s:\n",
        "        State.Idle:\n",
        "            return 0\n",
        "        State.Active(v):\n",
        "            return v + delta\n",
        "\n",
        "def main() -> Int:\n",
        "    d := 2 + 3\n",
        "    return step(State.Active(10), d)\n",
    );
    let (mut p1, t1) = compile_mir(source);
    let (mut p2, t2) = compile_mir(source);
    let o1 = optimization_passes().run(&mut p1, &t1).unwrap();
    let o2 = optimization_passes().run(&mut p2, &t2).unwrap();
    assert_eq!(o1, o2);
    assert_eq!(mir_snapshot(&p1), mir_snapshot(&p2));
}

// ── Section 7: 45+ Program Behavioral Equivalence Suite & ARC Preservation ──

struct EquivalenceCase {
    name: &'static str,
    source: &'static str,
    expected: ExecutionValue,
    expected_output: &'static [&'static str],
}

fn verify_equivalence_case(case: &EquivalenceCase) {
    let (unopt, types) = compile_mir(case.source);
    let entry = entry_function(&unopt);
    let before = run_legacy(&unopt, &types, entry);
    assert_eq!(
        before.value, case.expected,
        "[{}] unoptimized value mismatch",
        case.name
    );
    assert_eq!(
        before.output,
        case.expected_output
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>(),
        "[{}] unoptimized output mismatch",
        case.name
    );

    for level in [
        OptimizationLevel::O0,
        OptimizationLevel::O1,
        OptimizationLevel::O2,
        OptimizationLevel::O3,
        OptimizationLevel::Oz,
    ] {
        let mut opt = unopt.clone();
        run_optimization(&mut opt, &types, level)
            .unwrap_or_else(|e| panic!("[{}] {level:?} failed: {e:?}", case.name));
        assert!(
            verify_mir(&opt, &types).is_empty(),
            "[{}] {level:?} produced invalid MIR",
            case.name
        );
        assert_shape_stable(&unopt, &opt);
        let after = run_legacy(&opt, &types, entry);
        assert_eq!(
            after.value, before.value,
            "[{}] value changed at {level:?}",
            case.name
        );
        assert_eq!(
            after.output, before.output,
            "[{}] output changed at {level:?}",
            case.name
        );
        assert!(
            after.stats.steps <= before.stats.steps,
            "[{}] steps regressed at {level:?}: {} -> {}",
            case.name,
            before.stats.steps,
            after.stats.steps
        );
    }

    // Also test whole-program LTO at O2
    let mut lto = unopt.clone();
    run_optimization_with_lto(&mut lto, &types, OptimizationLevel::O2, true)
        .unwrap_or_else(|e| panic!("[{}] O2+LTO failed: {e:?}", case.name));
    assert!(
        verify_mir(&lto, &types).is_empty(),
        "[{}] O2+LTO invalid MIR",
        case.name
    );
    let after_lto = run_legacy(&lto, &types, entry);
    assert_eq!(
        after_lto.value, before.value,
        "[{}] value changed under LTO",
        case.name
    );
    assert_eq!(
        after_lto.output, before.output,
        "[{}] output changed under LTO",
        case.name
    );
}

#[test]
fn equivalence_suite_45_plus_programs_loops_recursion_enums_tuples_closures_structs_set_deque() {
    let cases: Vec<EquivalenceCase> = vec![
        // 01. Basic integer arithmetic chain
        EquivalenceCase {
            name: "01_int_arith_chain",
            source: "def main() -> Int:\n    a := 7\n    b := a * 6\n    c := b - 2\n    return c / 4\n",
            expected: ExecutionValue::Int(10),
            expected_output: &[],
        },
        // 02. Bitwise & shifts
        EquivalenceCase {
            name: "02_bitwise_shifts",
            source: "def main() -> Int:\n    x := (1 << 5) | 3\n    y := x & 255\n    return y ^ 5\n",
            expected: ExecutionValue::Int(38),
            expected_output: &[],
        },
        // 03. Float arithmetic & rounding
        EquivalenceCase {
            name: "03_float_ops",
            source: "def main() -> Int:\n    x := (3.5 + 2.5) * 2.0\n    if x == 12.0:\n        return 1\n    return 0\n",
            expected: ExecutionValue::Int(1),
            expected_output: &[],
        },
        // 04. Bool logic & short-ish evaluation
        EquivalenceCase {
            name: "04_bool_logic",
            source: "def main() -> Bool:\n    a := true\n    b := false\n    return (a && !b) || (b && a)\n",
            expected: ExecutionValue::Bool(true),
            expected_output: &[],
        },
        // 05. Char comparison
        EquivalenceCase {
            name: "05_char_cmp",
            source: "def main() -> Int:\n    a := 'a'\n    b := 'z'\n    if a < b && a != b:\n        return 1\n    return 0\n",
            expected: ExecutionValue::Int(1),
            expected_output: &[],
        },
        // 06. While loop sum
        EquivalenceCase {
            name: "06_while_sum",
            source: "def main() -> Int:\n    mut i := 1\n    mut acc := 0\n    while i <= 10:\n        acc = acc + i\n        i = i + 1\n    return acc\n",
            expected: ExecutionValue::Int(55),
            expected_output: &[],
        },
        // 07. For-range loop
        EquivalenceCase {
            name: "07_for_range",
            source: "def main() -> Int:\n    mut acc := 0\n    for i in 0..6:\n        acc = acc + i * i\n    return acc\n",
            expected: ExecutionValue::Int(55),
            expected_output: &[],
        },
        // 08. For-in-list loop
        EquivalenceCase {
            name: "08_for_list",
            source: "def main() -> Int:\n    xs := [2, 4, 6, 8]\n    mut sum := 0\n    for x in xs:\n        sum = sum + x\n    return sum\n",
            expected: ExecutionValue::Int(20),
            expected_output: &[],
        },
        // 09. Nested loops with break & continue
        EquivalenceCase {
            name: "09_nested_break_continue",
            source: concat!(
                "def main() -> Int:\n",
                "    mut total := 0\n",
                "    mut i := 0\n",
                "    while i < 6:\n",
                "        i = i + 1\n",
                "        if i == 3:\n",
                "            continue\n",
                "        if i == 6:\n",
                "            break\n",
                "        total = total + i\n",
                "    return total\n",
            ),
            expected: ExecutionValue::Int(1 + 2 + 4 + 5),
            expected_output: &[],
        },
        // 10. Nested 2D loop matrix sum
        EquivalenceCase {
            name: "10_nested_2d_loop",
            source: concat!(
                "def main() -> Int:\n",
                "    mut sum := 0\n",
                "    for i in 0..4:\n",
                "        for j in 0..4:\n",
                "            sum = sum + i * 4 + j\n",
                "    return sum\n",
            ),
            expected: ExecutionValue::Int(120),
            expected_output: &[],
        },
        // 11. Recursive factorial
        EquivalenceCase {
            name: "11_rec_factorial",
            source: "def fact(n: Int) -> Int:\n    if n <= 1:\n        return 1\n    return n * fact(n - 1)\ndef main() -> Int:\n    return fact(6)\n",
            expected: ExecutionValue::Int(720),
            expected_output: &[],
        },
        // 12. Recursive Fibonacci
        EquivalenceCase {
            name: "12_rec_fibonacci",
            source: "def fib(n: Int) -> Int:\n    if n <= 1:\n        return n\n    return fib(n - 1) + fib(n - 2)\ndef main() -> Int:\n    return fib(9)\n",
            expected: ExecutionValue::Int(34),
            expected_output: &[],
        },
        // 13. Euclidean GCD recursion
        EquivalenceCase {
            name: "13_rec_gcd",
            source: "def gcd(a: Int, b: Int) -> Int:\n    if b == 0:\n        return a\n    return gcd(b, a % b)\ndef main() -> Int:\n    return gcd(1071, 462)\n",
            expected: ExecutionValue::Int(21),
            expected_output: &[],
        },
        // 14. Mutual recursion (even / odd)
        EquivalenceCase {
            name: "14_mutual_recursion",
            source: concat!(
                "def is_even(n: Int) -> Bool:\n",
                "    if n == 0:\n",
                "        return true\n",
                "    return is_odd(n - 1)\n",
                "def is_odd(n: Int) -> Bool:\n",
                "    if n == 0:\n",
                "        return false\n",
                "    return is_even(n - 1)\n",
                "def main() -> Int:\n",
                "    if is_even(10) && is_odd(7):\n",
                "        return 1\n",
                "    return 0\n",
            ),
            expected: ExecutionValue::Int(1),
            expected_output: &[],
        },
        // 15. Fast exponentiation recursion
        EquivalenceCase {
            name: "15_fast_pow",
            source: concat!(
                "def ipow(base: Int, exp: Int) -> Int:\n",
                "    if exp == 0:\n",
                "        return 1\n",
                "    half := ipow(base, exp / 2)\n",
                "    if (exp % 2) == 0:\n",
                "        return half * half\n",
                "    return base * half * half\n",
                "def main() -> Int:\n",
                "    return ipow(3, 5)\n",
            ),
            expected: ExecutionValue::Int(243),
            expected_output: &[],
        },
        // 16. Enum unit variants + match
        EquivalenceCase {
            name: "16_enum_unit_match",
            source: concat!(
                "enum Dir:\n",
                "    North\n",
                "    East\n",
                "    South\n",
                "    West\n",
                "def score(d: Dir) -> Int:\n",
                "    match d:\n",
                "        Dir.North:\n",
                "            return 1\n",
                "        Dir.East:\n",
                "            return 2\n",
                "        Dir.South:\n",
                "            return 3\n",
                "        Dir.West:\n",
                "            return 4\n",
                "def main() -> Int:\n",
                "    return score(Dir.North) + score(Dir.West)\n",
            ),
            expected: ExecutionValue::Int(5),
            expected_output: &[],
        },
        // 17. Enum with multi-field payload + match
        EquivalenceCase {
            name: "17_enum_payload_match",
            source: concat!(
                "enum Shape:\n",
                "    Circle(r: Int)\n",
                "    Rect(w: Int, h: Int)\n",
                "def area_metric(s: Shape) -> Int:\n",
                "    match s:\n",
                "        Shape.Circle(r):\n",
                "            return r * r * 3\n",
                "        Shape.Rect(w, h):\n",
                "            return w * h\n",
                "def main() -> Int:\n",
                "    return area_metric(Shape.Circle(4)) + area_metric(Shape.Rect(5, 6))\n",
            ),
            expected: ExecutionValue::Int(48 + 30),
            expected_output: &[],
        },
        // 18. Nested enum inside match
        EquivalenceCase {
            name: "18_nested_enum_match",
            source: concat!(
                "enum OptInt:\n",
                "    None\n",
                "    Some(v: Int)\n",
                "def unwrap_or(o: OptInt, d: Int) -> Int:\n",
                "    match o:\n",
                "        OptInt.None:\n",
                "            return d\n",
                "        OptInt.Some(v):\n",
                "            return v\n",
                "def main() -> Int:\n",
                "    return unwrap_or(OptInt.Some(19), 0) + unwrap_or(OptInt.None, 23)\n",
            ),
            expected: ExecutionValue::Int(42),
            expected_output: &[],
        },
        // 19. Recursive enum AST evaluator
        EquivalenceCase {
            name: "19_recursive_enum_eval",
            source: concat!(
                "enum Expr:\n",
                "    Lit(v: Int)\n",
                "    Add(l: Expr, r: Expr)\n",
                "    Mul(l: Expr, r: Expr)\n",
                "def eval_expr(e: Expr) -> Int:\n",
                "    match e:\n",
                "        Expr.Lit(v):\n",
                "            return v\n",
                "        Expr.Add(l, r):\n",
                "            return eval_expr(l) + eval_expr(r)\n",
                "        Expr.Mul(l, r):\n",
                "            return eval_expr(l) * eval_expr(r)\n",
                "def main() -> Int:\n",
                "    tree := Expr.Mul(Expr.Add(Expr.Lit(2), Expr.Lit(3)), Expr.Lit(7))\n",
                "    return eval_expr(tree)\n",
            ),
            expected: ExecutionValue::Int(35),
            expected_output: &[],
        },
        // 20. Tuples & nested tuple projections
        EquivalenceCase {
            name: "20_nested_tuples",
            source: concat!(
                "def swap_sum(p: (Int, (Int, Int))) -> Int:\n",
                "    inner := p.1\n",
                "    return p.0 * 10 + inner.0 + inner.1\n",
                "def main() -> Int:\n",
                "    t := (4, (5, 6))\n",
                "    return swap_sum(t)\n",
            ),
            expected: ExecutionValue::Int(51),
            expected_output: &[],
        },
        // 21. Tuple returned from function
        EquivalenceCase {
            name: "21_tuple_return",
            source: concat!(
                "def divmod(a: Int, b: Int) -> (Int, Int):\n",
                "    return (a / b, a % b)\n",
                "def main() -> Int:\n",
                "    dm := divmod(29, 6)\n",
                "    return dm.0 * 100 + dm.1\n",
            ),
            expected: ExecutionValue::Int(405),
            expected_output: &[],
        },
        // 22. Struct + impl methods
        EquivalenceCase {
            name: "22_struct_impl_methods",
            source: concat!(
                "struct Vec2:\n",
                "    x: Int\n",
                "    y: Int\n",
                "impl Vec2:\n",
                "    def dot(self, other: Vec2) -> Int:\n",
                "        return self.x * other.x + self.y * other.y\n",
                "    def scale(self, k: Int) -> Vec2:\n",
                "        return Vec2(self.x * k, self.y * k)\n",
                "def main() -> Int:\n",
                "    a := Vec2(2, 3)\n",
                "    b := a.scale(4)\n",
                "    return a.dot(b)\n",
            ),
            expected: ExecutionValue::Int(52),
            expected_output: &[],
        },
        // 23. Mutable nested struct field update
        EquivalenceCase {
            name: "23_mutable_nested_struct",
            source: concat!(
                "struct Inner:\n",
                "    val: Int\n",
                "struct Outer:\n",
                "    inner: Inner\n",
                "    tag: Int\n",
                "def main() -> Int:\n",
                "    mut o := Outer(Inner(10), 5)\n",
                "    o.inner.val = o.inner.val + 15\n",
                "    o.tag = o.tag * 2\n",
                "    return o.inner.val + o.tag\n",
            ),
            expected: ExecutionValue::Int(35),
            expected_output: &[],
        },
        // 24. Structs inside List
        EquivalenceCase {
            name: "24_structs_in_list",
            source: concat!(
                "struct Item:\n",
                "    weight: Int\n",
                "def main() -> Int:\n",
                "    xs := [Item(3), Item(7), Item(11)]\n",
                "    mut total := 0\n",
                "    for it in xs:\n",
                "        total = total + it.weight\n",
                "    return total\n",
            ),
            expected: ExecutionValue::Int(21),
            expected_output: &[],
        },
        // 25. Non-capturing closure & higher-order call
        EquivalenceCase {
            name: "25_higher_order_closure",
            source: concat!(
                "def apply_twice(f: Custom, x: Int) -> Int:\n",
                "    return f(f(x))\n",
                "def main() -> Int:\n",
                "    inc := fn(v: Int) -> Int: v + 3\n",
                "    return apply_twice(inc, 10)\n",
            ),
            expected: ExecutionValue::Int(16),
            expected_output: &[],
        },
        // 26. Multi-capture closure
        EquivalenceCase {
            name: "26_multi_capture_closure",
            source: concat!(
                "def main() -> Int:\n",
                "    a := 3\n",
                "    b := 5\n",
                "    linear := fn(x: Int) -> Int:\n",
                "        return a * x + b\n",
                "    return linear(4) + linear(10)\n",
            ),
            expected: ExecutionValue::Int(17 + 35),
            expected_output: &[],
        },
        // 27. Mutable capture writeback across multiple calls
        EquivalenceCase {
            name: "27_mutable_capture_counter",
            source: concat!(
                "def main() -> Int:\n",
                "    mut count := 0\n",
                "    next := fn(step: Int) -> Int:\n",
                "        count = count + step\n",
                "        return count\n",
                "    a := next(2)\n",
                "    b := next(5)\n",
                "    c := next(10)\n",
                "    return a + b + c\n",
            ),
            expected: ExecutionValue::Int(2 + 7 + 17),
            expected_output: &[],
        },
        // 28. Closures stored in List
        EquivalenceCase {
            name: "28_closures_in_list",
            source: concat!(
                "def main() -> Int:\n",
                "    f1 := fn(x: Int) -> Int: x + 1\n",
                "    f2 := fn(x: Int) -> Int: x * 2\n",
                "    ops := [f1, f2]\n",
                "    return list_get(ops, 0)(10) + list_get(ops, 1)(10)\n",
            ),
            expected: ExecutionValue::Int(31),
            expected_output: &[],
        },
        // 29. List push, set, get, len
        EquivalenceCase {
            name: "29_list_mutations",
            source: concat!(
                "def main() -> Int:\n",
                "    xs := list_new()\n",
                "    list_push(xs, 10)\n",
                "    list_push(xs, 20)\n",
                "    list_push(xs, 30)\n",
                "    list_set(xs, 0, 99)\n",
                "    return list_get(xs, 0) + list_get(xs, 1) + list_get(xs, 2) + list_len(xs)\n",
            ),
            expected: ExecutionValue::Int(99 + 20 + 30 + 3),
            expected_output: &[],
        },
        // 30. Map operations
        EquivalenceCase {
            name: "30_map_operations",
            source: concat!(
                "def main() -> Int:\n",
                "    mut m := map_new()\n",
                "    map_put(m, \"alpha\", 11)\n",
                "    map_put(m, \"beta\", 22)\n",
                "    map_put(m, \"alpha\", 33)\n",
                "    mut sum := 0\n",
                "    if map_has(m, \"alpha\"):\n",
                "        sum = sum + map_get(m, \"alpha\")\n",
                "    if map_has(m, \"beta\"):\n",
                "        sum = sum + map_get(m, \"beta\")\n",
                "    if map_has(m, \"gamma\"):\n",
                "        sum = sum + 1000\n",
                "    return sum\n",
            ),
            expected: ExecutionValue::Int(55),
            expected_output: &[],
        },
        // 31. Set operations (int)
        EquivalenceCase {
            name: "31_set_int_operations",
            source: concat!(
                "def main() -> Int:\n",
                "    s1 := set_new()\n",
                "    set_add(s1, 10)\n",
                "    set_add(s1, 20)\n",
                "    set_add(s1, 10)\n",
                "    set_add(s1, 30)\n",
                "    set_remove(s1, 20)\n",
                "    s2 := set_new()\n",
                "    set_add(s2, 30)\n",
                "    set_add(s2, 40)\n",
                "    su := set_union(s1, s2)\n",
                "    si := set_intersect(s1, s2)\n",
                "    mut out := set_len(s1) * 100 + set_len(su) * 10 + set_len(si)\n",
                "    if set_has(s1, 10) && !set_has(s1, 20):\n",
                "        out = out + 1\n",
                "    set_clear(s1)\n",
                "    out = out + set_len(s1)\n",
                "    set_free(s1)\n",
                "    set_free(s2)\n",
                "    set_free(su)\n",
                "    set_free(si)\n",
                "    return out\n",
            ),
            expected: ExecutionValue::Int(232),
            expected_output: &[],
        },
        // 32. Set string operations
        EquivalenceCase {
            name: "32_set_str_operations",
            source: concat!(
                "def main() -> Int:\n",
                "    s := set_new()\n",
                "    set_add_str(s, \"red\")\n",
                "    set_add_str(s, \"green\")\n",
                "    set_add_str(s, \"red\")\n",
                "    set_remove_str(s, \"green\")\n",
                "    mut acc := set_len(s)\n",
                "    if set_has_str(s, \"red\") && !set_has_str(s, \"green\"):\n",
                "        acc = acc + 10\n",
                "    set_free(s)\n",
                "    return acc\n",
            ),
            expected: ExecutionValue::Int(11),
            expected_output: &[],
        },
        // 33. Deque push/pop/peek/get/clear
        EquivalenceCase {
            name: "33_deque_operations",
            source: concat!(
                "def main() -> Int:\n",
                "    dq := deque_new()\n",
                "    deque_push_back(dq, 20)\n",
                "    deque_push_front(dq, 10)\n",
                "    deque_push_back(dq, 30)\n",
                "    deque_push_front(dq, 5)\n",
                "    f := deque_peek_front(dq)\n",
                "    b := deque_peek_back(dq)\n",
                "    mid := deque_get(dq, 2)\n",
                "    p1 := deque_pop_front(dq)\n",
                "    p2 := deque_pop_back(dq)\n",
                "    rem := deque_len(dq)\n",
                "    deque_clear(dq)\n",
                "    after_clear := deque_len(dq)\n",
                "    deque_free(dq)\n",
                "    return f + b + mid + p1 + p2 + rem + after_clear\n",
            ),
            expected: ExecutionValue::Int(5 + 30 + 20 + 5 + 30 + 2),
            expected_output: &[],
        },
        // 34. String concatenation & builtins
        EquivalenceCase {
            name: "34_string_builtins",
            source: concat!(
                "def main() -> Int:\n",
                "    a := \"  Hello, \"\n",
                "    b := \"L++!  \"\n",
                "    joined := str_trim(a + b)\n",
                "    print_str(joined)\n",
                "    if str_starts_with(joined, \"Hello\") && str_contains(joined, \"L++\"):\n",
                "        return str_len(joined)\n",
                "    return 0\n",
            ),
            expected: ExecutionValue::Int(11),
            expected_output: &["Hello, L++!\n"],
        },
        // 35. String constant equality folding vs concatenation preservation
        EquivalenceCase {
            name: "35_string_eq_vs_concat",
            source: concat!(
                "def main() -> Int:\n",
                "    same := \"abc\" == \"abc\"\n",
                "    diff := \"abc\" != \"xyz\"\n",
                "    cat := \"ab\" + \"c\"\n",
                "    if same && diff && cat == \"abc\":\n",
                "        return 1\n",
                "    return 0\n",
            ),
            expected: ExecutionValue::Int(1),
            expected_output: &[],
        },
        // 36. Print builtins (Int, Bool, Str)
        EquivalenceCase {
            name: "36_print_builtins",
            source: concat!(
                "def main():\n",
                "    print(10 + 5)\n",
                "    print_bool(true && !false)\n",
                "    print_str(\"ok\")\n",
            ),
            expected: ExecutionValue::Void,
            expected_output: &["15\n", "1\n", "ok\n"],
        },
        // 37. Async/await chain
        EquivalenceCase {
            name: "37_async_await_chain",
            source: concat!(
                "async def leaf(x: Int) -> Int:\n",
                "    return x * 3\n",
                "async def mid(x: Int) -> Int:\n",
                "    v := leaf(x).await\n",
                "    return v + 4\n",
                "async def main() -> Int:\n",
                "    return mid(5).await\n",
            ),
            expected: ExecutionValue::Int(19),
            expected_output: &[],
        },
        // 38. Double await idempotence on same Task
        EquivalenceCase {
            name: "38_double_await_task",
            source: concat!(
                "async def compute() -> Int:\n",
                "    return 21\n",
                "async def main() -> Int:\n",
                "    t := compute()\n",
                "    a := t.await\n",
                "    b := t.await\n",
                "    return a + b\n",
            ),
            expected: ExecutionValue::Int(42),
            expected_output: &[],
        },
        // 39. Spawn detached closure
        EquivalenceCase {
            name: "39_spawn_task",
            source: concat!(
                "def main() -> Int:\n",
                "    x := 7\n",
                "    spawn fn():\n",
                "        print(x * 2)\n",
                "    return x\n",
            ),
            expected: ExecutionValue::Int(7),
            expected_output: &["14\n"],
        },
        // 40. Kernel/raw pointer intrinsics
        EquivalenceCase {
            name: "40_raw_ptr_intrinsics",
            source: concat!(
                "unsafe def main() -> Int:\n",
                "    p := ptr_null[U64]()\n",
                "    sz := size_of[U64]()\n",
                "    al := align_of[U64]()\n",
                "    p2 := ptr_add(p, 16)\n",
                "    if ptr_is_null(p) && !ptr_is_null(p2):\n",
                "        return (sz + al) as Int\n",
                "    return 0\n",
            ),
            expected: ExecutionValue::Int(16),
            expected_output: &[],
        },
        // 41. Default function arguments
        EquivalenceCase {
            name: "41_default_args",
            source: concat!(
                "def calc(a: Int, b: Int = 10, c: Int = 5) -> Int:\n",
                "    return a * b + c\n",
                "def main() -> Int:\n",
                "    return calc(2) + calc(2, 3) + calc(2, 3, 1)\n",
            ),
            expected: ExecutionValue::Int(25 + 11 + 7),
            expected_output: &[],
        },
        // 42. Generic identity and wrapper structs
        EquivalenceCase {
            name: "42_generics",
            source: concat!(
                "struct Wrap[T]:\n",
                "    inner: T\n",
                "def id[T](v: T) -> T:\n",
                "    return v\n",
                "def main() -> Int:\n",
                "    w := Wrap(id(33))\n",
                "    return w.inner + id(9)\n",
            ),
            expected: ExecutionValue::Int(42),
            expected_output: &[],
        },
        // 43. Trait & impl dispatch
        EquivalenceCase {
            name: "43_traits",
            source: concat!(
                "trait Measurable:\n",
                "    def measure(self) -> Int\n",
                "struct Span:\n",
                "    len: Int\n",
                "impl Measurable for Span:\n",
                "    def measure(self) -> Int:\n",
                "        return self.len * 2\n",
                "def total_measure[T: Measurable](item: T) -> Int:\n",
                "    return item.measure() + 1\n",
                "def main() -> Int:\n",
                "    return total_measure(Span(12))\n",
            ),
            expected: ExecutionValue::Int(25),
            expected_output: &[],
        },
        // 44. Multi-function pipeline with constant folding + loops + structs
        EquivalenceCase {
            name: "44_combined_pipeline",
            source: concat!(
                "struct Acc:\n",
                "    sum: Int\n",
                "    count: Int\n",
                "def step_val(i: Int) -> Int:\n",
                "    return i * 2 + 1\n",
                "def main() -> Int:\n",
                "    mut a := Acc(0, 0)\n",
                "    for i in 0..5:\n",
                "        a.sum = a.sum + step_val(i)\n",
                "        a.count = a.count + 1\n",
                "    return a.sum + a.count\n",
            ),
            expected: ExecutionValue::Int(25 + 5),
            expected_output: &[],
        },
        // 45. Set + Deque + Enum combined algorithm (BFS-like worklist)
        EquivalenceCase {
            name: "45_deque_set_worklist",
            source: concat!(
                "def main() -> Int:\n",
                "    visited := set_new()\n",
                "    q := deque_new()\n",
                "    deque_push_back(q, 1)\n",
                "    set_add(visited, 1)\n",
                "    mut sum := 0\n",
                "    while deque_len(q) > 0:\n",
                "        cur := deque_pop_front(q)\n",
                "        sum = sum + cur\n",
                "        next1 := cur * 2\n",
                "        next2 := cur * 2 + 1\n",
                "        if next1 <= 7 && !set_has(visited, next1):\n",
                "            set_add(visited, next1)\n",
                "            deque_push_back(q, next1)\n",
                "        if next2 <= 7 && !set_has(visited, next2):\n",
                "            set_add(visited, next2)\n",
                "            deque_push_back(q, next2)\n",
                "    cnt := set_len(visited)\n",
                "    set_free(visited)\n",
                "    deque_free(q)\n",
                "    return sum * 10 + cnt\n",
            ),
            expected: ExecutionValue::Int(28 * 10 + 7),
            expected_output: &[],
        },
        // 46. Signed integer edge & comparison cascade
        EquivalenceCase {
            name: "46_int_edge_cascade",
            source: concat!(
                "def main() -> Int:\n",
                "    big := 9223372036854775807\n",
                "    neg := -big\n",
                "    rem := (-29) % 5\n",
                "    if neg < 0 && rem == -4:\n",
                "        return 100 + rem\n",
                "    return 0\n",
            ),
            expected: ExecutionValue::Int(96),
            expected_output: &[],
        },
        // 47. String split, replace, substr, find, upper
        EquivalenceCase {
            name: "47_str_split_replace",
            source: concat!(
                "def main() -> Int:\n",
                "    parts := str_split(\"one,two,three\", 44)\n",
                "    rep := str_replace(\"foo-bar\", \"-\", \"_\")\n",
                "    sub := str_substr(rep, 4, 3)\n",
                "    up := str_to_upper(sub)\n",
                "    pos := str_find(rep, \"bar\")\n",
                "    if up == \"BAR\" && pos == 4:\n",
                "        return list_len(parts) + str_len(rep)\n",
                "    return 0\n",
            ),
            expected: ExecutionValue::Int(10),
            expected_output: &[],
        },
    ];

    assert!(
        cases.len() >= 45,
        "expected at least 45 equivalence cases, found {}",
        cases.len()
    );
    for case in &cases {
        verify_equivalence_case(case);
    }
}

// ── Section 8: ARC Retain/Release Preservation Gates ────────────────────────

#[test]
fn arc_retain_release_counts_preserved_by_optimization_and_lto() {
    let acyclic_heap_programs = [
        // 1. String concatenation (must NOT fold Add, preserving string node ARC traffic)
        (
            "str_concat",
            "def main() -> Str:\n    a := \"hello\"\n    b := \" world\"\n    return a + b\n",
            ExecutionValue::String("hello world".to_owned()),
        ),
        // 2. List with closures & calls
        (
            "list_closures",
            concat!(
                "def main() -> Int:\n",
                "    k := 10 + 20\n",
                "    f := fn(x: Int) -> Int: x + k\n",
                "    xs := [f]\n",
                "    g := list_get(xs, 0)\n",
                "    return g(12)\n",
            ),
            ExecutionValue::Int(42),
        ),
        // 3. Struct + borrowed helper + scalar inline candidate
        (
            "struct_and_scalar_helper",
            concat!(
                "struct Box:\n",
                "    v: Int\n",
                "def add_one(n: Int) -> Int:\n",
                "    return n + 1\n",
                "def read_box(b: Box) -> Int:\n",
                "    return add_one(b.v)\n",
                "def main() -> Int:\n",
                "    b := Box(40 + 1)\n",
                "    return read_box(b)\n",
            ),
            ExecutionValue::Int(42),
        ),
        // 4. Tuple containing heap string + list
        (
            "tuple_heap_elements",
            concat!(
                "def main() -> Int:\n",
                "    s := \"abc\"\n",
                "    xs := [1, 2, 3]\n",
                "    t := (s, xs)\n",
                "    return str_len(t.0) + list_len(t.1)\n",
            ),
            ExecutionValue::Int(6),
        ),
        // 5. Constant branch folding over heap allocations in live arm
        (
            "branch_fold_live_heap",
            concat!(
                "def main() -> Int:\n",
                "    flag := (2 + 2) == 4\n",
                "    if flag:\n",
                "        xs := [10, 20]\n",
                "        return list_len(xs)\n",
                "    else:\n",
                "        ys := [1, 2, 3, 4]\n",
                "        return list_len(ys)\n",
            ),
            ExecutionValue::Int(2),
        ),
        // 6. Async + await with string return
        (
            "async_str_arc",
            concat!(
                "async def greet() -> Str:\n",
                "    return \"hi\"\n",
                "async def main() -> Int:\n",
                "    s := greet().await\n",
                "    return str_len(s)\n",
            ),
            ExecutionValue::Int(2),
        ),
    ];

    for (name, source, expected) in acyclic_heap_programs {
        let (unopt, types) = compile_mir(source);
        let entry = entry_function(&unopt);
        let (before_out, before_arc) = run_arc_acyclic(&unopt, &types, entry);
        assert_eq!(before_out.value, expected, "[{name}] unopt value");

        let mut opt_o2 = unopt.clone();
        run_optimization(&mut opt_o2, &types, OptimizationLevel::O2).unwrap();
        let (o2_out, o2_arc) = run_arc_acyclic(&opt_o2, &types, entry);
        assert_eq!(o2_out.value, before_out.value, "[{name}] O2 value");
        assert_eq!(
            o2_arc, before_arc,
            "[{name}] O2 changed ARC traffic: {before_arc:?} -> {o2_arc:?}"
        );

        let mut opt_lto = unopt.clone();
        run_optimization_with_lto(&mut opt_lto, &types, OptimizationLevel::O3, true).unwrap();
        let (lto_out, lto_arc) = run_arc_acyclic(&opt_lto, &types, entry);
        assert_eq!(lto_out.value, before_out.value, "[{name}] LTO value");
        assert_eq!(
            lto_arc, before_arc,
            "[{name}] LTO changed ARC traffic: {before_arc:?} -> {lto_arc:?}"
        );
    }
}

#[test]
fn string_equality_const_fold_does_not_leak_cached_strings_under_arc() {
    let source = concat!(
        "def main() -> Int:\n",
        "    a := \"alpha\" == \"alpha\"\n",
        "    b := \"alpha\" != \"beta\"\n",
        "    if a && b:\n",
        "        return 1\n",
        "    return 0\n",
    );
    let (unopt, types) = compile_mir(source);
    let entry = entry_function(&unopt);
    let (before_out, before_arc) = run_arc_acyclic(&unopt, &types, entry);
    assert_eq!(before_out.value, ExecutionValue::Int(1));

    let mut opt = unopt.clone();
    run_optimization(&mut opt, &types, OptimizationLevel::O3).unwrap();
    let (after_out, after_arc) = run_arc_acyclic(&opt, &types, entry);
    assert_eq!(after_out.value, ExecutionValue::Int(1));
    assert_eq!(after_arc.pinned_live, 0);
    // Retains == releases - frees? Specifically every allocated node freed cleanly!
    assert!(after_arc.frees <= before_arc.frees);
}

#[test]
fn linear_scaling_and_exact_step_budget_under_optimization() {
    let make_loop_src = |n: i64| {
        format!(
            "def main() -> Int:\n    mut i := 0\n    mut acc := 0\n    while i < {n}:\n        acc = acc + (i * 2 + 1)\n        i = i + 1\n    return acc\n"
        )
    };
    let (small, small_types) = compile_mir(&make_loop_src(25));
    let (large, large_types) = compile_mir(&make_loop_src(250));
    let e_small = entry_function(&small);
    let e_large = entry_function(&large);

    let mut small_opt = small.clone();
    let mut large_opt = large.clone();
    run_optimization(&mut small_opt, &small_types, OptimizationLevel::O2).unwrap();
    run_optimization(&mut large_opt, &large_types, OptimizationLevel::O2).unwrap();

    let out_small = run_legacy(&small_opt, &small_types, e_small);
    let out_large = run_legacy(&large_opt, &large_types, e_large);
    assert_eq!(out_small.value, ExecutionValue::Int(25 * 25));
    assert_eq!(out_large.value, ExecutionValue::Int(250 * 250));

    let ratio = out_large.stats.steps as f64 / out_small.stats.steps as f64;
    assert!(
        (6.0..14.0).contains(&ratio),
        "expected linear ~10x scaling, got {ratio:.2}"
    );

    // Tight exact step budget must succeed deterministically
    let exact_limits = InterpreterLimits {
        max_steps: out_small.stats.steps,
        ..InterpreterLimits::default()
    };
    assert!(execute_mir_with_stats(&small_opt, &small_types, e_small, &[], exact_limits).is_ok());
    let too_tight = InterpreterLimits {
        max_steps: out_small.stats.steps - 1,
        ..InterpreterLimits::default()
    };
    assert!(execute_mir_with_stats(&small_opt, &small_types, e_small, &[], too_tight).is_err());
}
