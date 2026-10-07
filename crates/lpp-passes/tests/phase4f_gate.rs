//! Phase 4F exit-gate evidence (MIR-side): fold semantics matched to
//! the oracle bit-for-bit, unoptimized-versus-optimized equivalence
//! through the legacy execution entry, shape stability (no
//! instruction is added, removed, or moved), compile-fail gates
//! (exact manager codes for bad pass output and bad input), level
//! budget semantics, linear growth, and tight-limit runs.
//!
//! Ownership-side evidence (balance proof, ARC equivalence,
//! pinned sets, use-after-move, pass wiring) lives in
//! `crates/lpp-ownership/tests/phase4f_ownership.rs`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lpp_common::OptimizationLevel;
use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    lower_package,
};
use lpp_mir::{
    BinaryOperator, Constant, ExecutionOutcome, ExecutionValue, InstructionId, InstructionKind,
    InterpreterErrorKind, InterpreterLimits, MirBuildOptions, MirFunctionId, MirFunctionKind,
    MirInvariant, MirProgram, Operand, Rvalue, Terminator, UnaryOperator, build_mir,
    execute_mir_with_stats, mir_snapshot, verify_mir,
};
use lpp_passes::{
    MirPass, PassContext, PassFailure, PassManager, PassManagerErrorKind, fold_binary, fold_unary,
    optimization_passes, run_optimization,
};
use lpp_types::{ShadowInferenceOptions, TypeInterner, infer_hir_package};

// ── scaffolding ────────────────────────────────────────────────────────────

#[derive(Debug)]
struct MemoryFileSystem {
    files: BTreeMap<PathBuf, String>,
}

impl MemoryFileSystem {
    fn new(source: &str) -> Self {
        Self {
            files: BTreeMap::from([(PathBuf::from("/ownership/main.lpp"), source.to_owned())]),
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

/// Lower + type-check + build the program; panics with context on
/// failure.
fn executable(source: &str) -> (MirProgram, TypeInterner) {
    let filesystem = MemoryFileSystem::new(source);
    let graph = GraphBuilder::new(&filesystem)
        .build(GraphRequest::new(
            "/ownership/main.lpp",
            PackageSpec::new("ownership", "/ownership"),
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

fn entry_function(program: &MirProgram) -> MirFunctionId {
    program
        .functions()
        .filter(|(_, function)| function.kind != MirFunctionKind::Closure)
        .map(|(id, _)| id)
        .last()
        .expect("test programs define at least one top-level function")
}

/// Legacy execution with default limits; panics with context on
/// failure.
fn run_legacy(
    program: &MirProgram,
    types: &TypeInterner,
    entry: MirFunctionId,
) -> ExecutionOutcome {
    execute_mir_with_stats(program, types, entry, &[], InterpreterLimits::default())
        .unwrap_or_else(|error| panic!("legacy execution failed: {error}"))
}

fn legacy_with_limits(
    program: &MirProgram,
    types: &TypeInterner,
    entry: MirFunctionId,
    limits: InterpreterLimits,
) -> Result<ExecutionOutcome, lpp_mir::InterpreterError> {
    execute_mir_with_stats(program, types, entry, &[], limits)
}

/// Per-block instruction id sequences: the shape-stability witness.
fn shape(program: &MirProgram) -> Vec<(lpp_mir::BasicBlockId, Vec<InstructionId>)> {
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
    assert_eq!(
        shape(before),
        shape(after),
        "the 4F passes rewrite in place: no block or instruction is added, removed, or moved"
    );
}

/// Collect the constants of one kind that appear as the sole value of
/// an `Assign` in the program (used to feed the pure fold functions
/// without hand-constructing origin-tagged constants).
fn assigned_constants(program: &MirProgram, which: fn(&Constant) -> bool) -> Vec<Constant> {
    program
        .instructions()
        .filter_map(|(_, instruction)| {
            if let InstructionKind::Assign {
                value: Rvalue::Use(Operand::Constant(constant)),
                ..
            } = &instruction.kind
            {
                (which)(constant).then_some(*constant)
            } else {
                None
            }
        })
        .collect()
}

/// Run the level and check the full equivalence gate: value and
/// output identical before and after, steps never increasing, shape
/// stable, MIR still validated. Returns the optimized program and its
/// types.
fn assert_equivalence(
    source: &str,
    level: OptimizationLevel,
    expected: ExecutionValue,
) -> (MirProgram, TypeInterner, ExecutionOutcome) {
    let (unoptimized, types) = executable(source);
    let entry = entry_function(&unoptimized);
    let before = run_legacy(&unoptimized, &types, entry);
    assert_eq!(before.value, expected);

    let mut optimized = unoptimized.clone();
    run_optimization(&mut optimized, &types, level)
        .unwrap_or_else(|error| panic!("optimization failed: {error:?}"));
    assert!(verify_mir(&optimized, &types).is_empty());
    assert_shape_stable(&unoptimized, &optimized);

    let after = run_legacy(&optimized, &types, entry);
    assert_eq!(after.value, before.value, "value changed by optimization");
    assert_eq!(
        after.output, before.output,
        "output changed by optimization"
    );
    assert!(
        after.stats.steps <= before.stats.steps,
        "steps increased: {} -> {}",
        before.stats.steps,
        after.stats.steps
    );
    (optimized, types, after)
}

// ── fold semantics: bit-for-bit against the oracle ─────────────────────────

#[test]
fn int_folds_match_wrapping_semantics() {
    let (program, _types) = executable("def main() -> Int:\n    return 1\n");
    let int = |left, right, operator| fold_binary(&program, operator, &left, &right);

    // Wrapping arithmetic — the oracle wraps, so the fold wraps.
    assert_eq!(
        int(
            Constant::Integer(i64::MAX),
            Constant::Integer(1),
            BinaryOperator::Add
        ),
        Some(Constant::Integer(i64::MIN))
    );
    assert_eq!(
        int(
            Constant::Integer(i64::MIN),
            Constant::Integer(1),
            BinaryOperator::Subtract
        ),
        Some(Constant::Integer(i64::MAX))
    );
    assert_eq!(
        int(
            Constant::Integer(i64::MAX),
            Constant::Integer(2),
            BinaryOperator::Multiply
        ),
        Some(Constant::Integer(-2))
    );
    // i64::MIN / -1 wraps to MIN in the oracle (wrapping_div).
    assert_eq!(
        int(
            Constant::Integer(i64::MIN),
            Constant::Integer(-1),
            BinaryOperator::Divide
        ),
        Some(Constant::Integer(i64::MIN))
    );
    assert_eq!(
        int(
            Constant::Integer(-7),
            Constant::Integer(3),
            BinaryOperator::Modulo
        ),
        Some(Constant::Integer(-1))
    );
    // Division and modulo by zero are runtime errors: not folded.
    assert_eq!(
        int(
            Constant::Integer(1),
            Constant::Integer(0),
            BinaryOperator::Divide
        ),
        None
    );
    assert_eq!(
        int(
            Constant::Integer(1),
            Constant::Integer(0),
            BinaryOperator::Modulo
        ),
        None
    );
    // Shifts use the wrapping semantics (shift counts wrap mod 64).
    assert_eq!(
        int(
            Constant::Integer(1),
            Constant::Integer(64),
            BinaryOperator::ShiftLeft
        ),
        Some(Constant::Integer(1))
    );
    assert_eq!(
        int(
            Constant::Integer(-8),
            Constant::Integer(1),
            BinaryOperator::ShiftRight
        ),
        Some(Constant::Integer(-4))
    );
    // Comparisons fold to bools.
    assert_eq!(
        int(
            Constant::Integer(2),
            Constant::Integer(3),
            BinaryOperator::Less
        ),
        Some(Constant::Bool(true))
    );
    assert_eq!(
        int(
            Constant::Integer(2),
            Constant::Integer(3),
            BinaryOperator::GreaterEqual
        ),
        Some(Constant::Bool(false))
    );
    // Logical operators on ints are runtime errors: not folded.
    assert_eq!(
        int(
            Constant::Integer(1),
            Constant::Integer(2),
            BinaryOperator::LogicalAnd
        ),
        None
    );
    // Negate wraps (i64::MIN negates to itself).
    assert_eq!(
        fold_unary(UnaryOperator::Negate, &Constant::Integer(i64::MIN)),
        Some(Constant::Integer(i64::MIN))
    );
    // Not on an int is a runtime error: not folded.
    assert_eq!(fold_unary(UnaryOperator::Not, &Constant::Integer(0)), None);
}

#[test]
fn float_folds_are_bit_exact_ieee() {
    let (program, _types) = executable("def main() -> Int:\n    return 1\n");
    let float = |left: f64, right: f64, operator| {
        fold_binary(
            &program,
            operator,
            &Constant::FloatBits(left.to_bits()),
            &Constant::FloatBits(right.to_bits()),
        )
    };
    let bits = |constant: &Option<Constant>| match constant {
        Some(Constant::FloatBits(bits)) => *bits,
        other => panic!("expected float bits, got {other:?}"),
    };

    // The fold applies the same IEEE-754 operations as the oracle, so
    // the results agree in kind and in every deterministic bit pattern
    // (infinities, signed zero). NaN payloads are a hardware
    // micro-detail of the runtime operation — rustc's const arithmetic
    // even picks a different payload than the runtime SSE, so the NaN
    // cases assert NaN-ness here, and the e2e
    // `float_nan_inf_equivalence` gate covers exact-bit agreement
    // against the oracle through both execution entries.
    assert!(f64::from_bits(bits(&float(f64::NAN, 1.0, BinaryOperator::Add))).is_nan());
    assert!(f64::from_bits(bits(&float(0.0, 0.0, BinaryOperator::Divide))).is_nan());
    assert_eq!(
        bits(&float(1.0, 0.0, BinaryOperator::Divide)),
        f64::INFINITY.to_bits()
    );
    assert_eq!(
        bits(&float(1e308, 10.0, BinaryOperator::Multiply)),
        f64::INFINITY.to_bits()
    );
    assert_eq!(
        bits(&float(-0.0, 0.0, BinaryOperator::Add)),
        0.0_f64.to_bits()
    );
    // NaN comparisons follow IEEE: NaN == NaN is false.
    assert_eq!(
        float(f64::NAN, f64::NAN, BinaryOperator::Equal),
        Some(Constant::Bool(false))
    );
    assert_eq!(
        float(f64::NAN, 1.0, BinaryOperator::Less),
        Some(Constant::Bool(false))
    );
    assert_eq!(
        float(1.0, 2.0, BinaryOperator::LessEqual),
        Some(Constant::Bool(true))
    );
    // Bitwise operators on floats are runtime errors: not folded.
    assert_eq!(float(1.0, 2.0, BinaryOperator::BitAnd), None);
    // Unary negate is a bit-exact sign flip.
    assert_eq!(
        fold_unary(
            UnaryOperator::Negate,
            &Constant::FloatBits((-2.5_f64).to_bits())
        ),
        Some(Constant::FloatBits(2.5_f64.to_bits()))
    );
    // Not on a float is a runtime error: not folded.
    assert_eq!(
        fold_unary(UnaryOperator::Not, &Constant::FloatBits(1.0_f64.to_bits())),
        None
    );
}

#[test]
fn bool_char_string_equality_folds() {
    let (program, _types) = executable(
        "def main() -> Str:\n    a := \"same\"\n    b := \"same\"\n    c := 'x'\n    d := 'x'\n    return a\n",
    );
    // Bool logical and equality.
    assert_eq!(
        fold_binary(
            &program,
            BinaryOperator::LogicalAnd,
            &Constant::Bool(true),
            &Constant::Bool(false)
        ),
        Some(Constant::Bool(false))
    );
    assert_eq!(
        fold_binary(
            &program,
            BinaryOperator::LogicalOr,
            &Constant::Bool(false),
            &Constant::Bool(false)
        ),
        Some(Constant::Bool(false))
    );
    assert_eq!(
        fold_binary(
            &program,
            BinaryOperator::Equal,
            &Constant::Bool(true),
            &Constant::Bool(true)
        ),
        Some(Constant::Bool(true))
    );
    // Char comparisons, from the two 'x' literal defs.
    let chars = assigned_constants(&program, |c| matches!(c, Constant::Character { .. }));
    assert_eq!(
        chars.len(),
        2,
        "expected two char literal defs, got {chars:?}"
    );
    assert_eq!(
        fold_binary(&program, BinaryOperator::Equal, &chars[0], &chars[1]),
        Some(Constant::Bool(true))
    );
    assert_eq!(
        fold_binary(&program, BinaryOperator::Less, &chars[0], &chars[0]),
        Some(Constant::Bool(false))
    );
    // String equality folds; the two "same" literals interning to
    // equal contents. String Add (concatenation) allocates a heap
    // node and is never folded.
    let strings = assigned_constants(&program, |c| matches!(c, Constant::String { .. }));
    assert_eq!(
        strings.len(),
        2,
        "expected two string literal defs, got {strings:?}"
    );
    assert_eq!(
        fold_binary(&program, BinaryOperator::Equal, &strings[0], &strings[1]),
        Some(Constant::Bool(true))
    );
    assert_eq!(
        fold_binary(&program, BinaryOperator::NotEqual, &strings[0], &strings[1]),
        Some(Constant::Bool(false))
    );
    assert_eq!(
        fold_binary(&program, BinaryOperator::Add, &strings[0], &strings[1]),
        None
    );
}

#[test]
fn never_folds_mixed_type_pairs() {
    let (program, _types) = executable("def main() -> Int:\n    return 1\n");
    // Mixed-type pairs are ruled out by the type checker; the folder
    // declines them so any such pair keeps its runtime behavior.
    assert_eq!(
        fold_binary(
            &program,
            BinaryOperator::Equal,
            &Constant::Integer(1),
            &Constant::FloatBits(1.0_f64.to_bits())
        ),
        None
    );
    assert_eq!(
        fold_binary(
            &program,
            BinaryOperator::Add,
            &Constant::Bool(true),
            &Constant::Integer(1)
        ),
        None
    );
}

// ── sweep and budget semantics ─────────────────────────────────────────────

#[test]
fn const_chain_peels_per_sweep_and_reaches_fixed_point_at_o2() {
    // Two constant-dependent layers: a:=1; b:=a+2; return b.
    // Sweep 1 propagates a into b's binary; sweep 2 folds it and
    // propagates the folded temp; sweep 3 propagates b into the
    // return; sweep 4 is a no-op and the run stops. O2's budget of
    // four sweeps therefore converges, and a second O2 run on its own
    // output changes nothing.
    let source = "def main() -> Int:\n    a := 1\n    b := a + 2\n    return b\n";
    let (optimized, types, after) =
        assert_equivalence(source, OptimizationLevel::O2, ExecutionValue::Int(3));
    assert!(
        mir_snapshot(&optimized).contains("Return(Some(Constant(Integer(3))))"),
        "expected the return to be folded to a constant:\n{}",
        mir_snapshot(&optimized)
    );
    // Idempotence at O2: a second run changes nothing.
    let mut again = optimized.clone();
    run_optimization(&mut again, &types, OptimizationLevel::O2).unwrap();
    assert_eq!(
        mir_snapshot(&again),
        mir_snapshot(&optimized),
        "an O2 run on its own output must change nothing once converged"
    );
    let _ = after;
}

#[test]
fn chain_deeper_than_budget_is_partially_reduced() {
    // Three layers: a:=1; b:=a+2; c:=b+3; return c. O2 (4 sweeps)
    // does not fully fold the three-layer chain; O3 (6 sweeps) does.
    // Values are identical at every level.
    let source = "def main() -> Int:\n    a := 1\n    b := a + 2\n    c := b + 3\n    return c\n";
    let (mut at_o2, types) = executable(source);
    run_optimization(&mut at_o2, &types, OptimizationLevel::O2).unwrap();
    let (mut at_o3, _types3) = executable(source);
    run_optimization(&mut at_o3, &types, OptimizationLevel::O3).unwrap();

    let after_o2 = run_legacy(&at_o2, &types, entry_function(&at_o2));
    let after_o3 = run_legacy(&at_o3, &types, entry_function(&at_o3));
    assert_eq!(after_o2.value, ExecutionValue::Int(6));
    assert_eq!(after_o3.value, ExecutionValue::Int(6));
    assert_eq!(
        after_o2.stats.steps, after_o3.stats.steps,
        "in-place rewrites keep the instruction and block counts, so steps are equal"
    );
    // O3 folds strictly further than O2 on the three-layer chain.
    assert_ne!(mir_snapshot(&at_o2), mir_snapshot(&at_o3));
}

#[test]
fn levels_progress_o0_o1_o2() {
    // A two-layer chain needs three changing sweeps to converge
    // (propagate, fold+propagate, propagate into the return). O0 (1
    // sweep) and O1 (2 sweeps) are strictly less reduced than O2.
    let source = "def main() -> Int:\n    a := 1\n    b := a + 2\n    return b\n";
    let at_level = |level: OptimizationLevel| {
        let (mut program, types) = executable(source);
        run_optimization(&mut program, &types, level).unwrap();
        (program, types)
    };
    let (o0, types0) = at_level(OptimizationLevel::O0);
    let (o1, types1) = at_level(OptimizationLevel::O1);
    let (o2, types2) = at_level(OptimizationLevel::O2);

    for (label, program, types) in [
        ("O0", &o0, &types0),
        ("O1", &o1, &types1),
        ("O2", &o2, &types2),
    ] {
        assert_eq!(
            run_legacy(program, types, entry_function(program)).value,
            ExecutionValue::Int(3),
            "{label} must preserve the value"
        );
    }
    assert_ne!(
        mir_snapshot(&o0),
        mir_snapshot(&o1),
        "O0 and O1 must differ on a two-layer chain"
    );
    assert_ne!(
        mir_snapshot(&o1),
        mir_snapshot(&o2),
        "O1 and O2 must differ on a two-layer chain"
    );
    // The O2 snapshot carries the fully folded return.
    assert!(mir_snapshot(&o2).contains("Return(Some(Constant(Integer(3))))"));
}

// ── unoptimized-versus-optimized equivalence ───────────────────────────────

#[test]
fn int_arithmetic_equivalence() {
    // big := i64::MAX; wrapped := big + 1 wraps to i64::MIN; the whole
    // chain folds at O2 and must compute the identical wrapped value.
    // halved = MIN/2 = -4611686018427387904; shifted = 8;
    // combined = (5<6) && (6<=5) = false, so the else branch adds 1.
    assert_equivalence(
        concat!(
            "def main() -> Int:\n",
            "    big := 9223372036854775807\n",
            "    wrapped := big + 1\n",
            "    halved := wrapped / 2\n",
            "    shifted := 1 << 3\n",
            "    cmp := 5 < 6\n",
            "    combined := cmp && (6 <= 5)\n",
            "    mut total := halved + shifted\n",
            "    if combined:\n",
            "        total = total + 100\n",
            "    else:\n",
            "        total = total + 1\n",
            "    return total\n",
        ),
        OptimizationLevel::O2,
        ExecutionValue::Int(-4611686018427387895),
    );
}

#[test]
fn division_by_zero_survives_optimization() {
    // b := 0 is a single-def constant, so const-prop rewrites the
    // divisor to Constant(0); const-fold must then refuse to fold the
    // division (right operand zero) and the runtime error is kept.
    let source = "def main() -> Int:\n    a := 5\n    b := 0\n    return a / b\n";
    let (program, types) = executable(source);
    let entry = entry_function(&program);
    let before = legacy_with_limits(&program, &types, entry, InterpreterLimits::default());
    assert!(matches!(before, Err(error) if error.kind == InterpreterErrorKind::DivisionByZero));

    let mut optimized = program.clone();
    run_optimization(&mut optimized, &types, OptimizationLevel::O3).unwrap();
    let after = legacy_with_limits(&optimized, &types, entry, InterpreterLimits::default());
    let error = after.expect_err("division by zero must survive optimization");
    assert_eq!(error.code(), "E4303");
    assert!(matches!(error.kind, InterpreterErrorKind::DivisionByZero));
}

#[test]
fn float_nan_inf_equivalence() {
    // NaN and inf flows: the optimized program computes the same
    // IEEE comparisons. `inf == inf` is true (the only branch taken),
    // `nan == 1.0` is false.
    assert_equivalence(
        concat!(
            "def main() -> Int:\n",
            "    inf := 1.0 / 0.0\n",
            "    nan := 0.0 / 0.0\n",
            "    is_nan := nan == 1.0\n",
            "    finite := inf == inf\n",
            "    mut total := 0\n",
            "    if is_nan:\n",
            "        total = total + 1\n",
            "    if finite:\n",
            "        total = total + 2\n",
            "    return total\n",
        ),
        OptimizationLevel::O2,
        ExecutionValue::Int(2),
    );
}

#[test]
fn branch_fold_equivalence_and_terminator_shape() {
    // Two constant-bool branches (true and false), nested: both fold
    // to gotos, the abandoned arms become unreachable (legal), and the
    // value is unchanged.
    let source = concat!(
        "def main() -> Int:\n",
        "    t := true\n",
        "    f := false\n",
        "    if t:\n",
        "        if f:\n",
        "            return 1\n",
        "        else:\n",
        "            return 2\n",
        "    else:\n",
        "        return 3\n",
        "    return 4\n",
    );
    let branches = |program: &MirProgram| {
        program
            .blocks()
            .filter(|(_, block)| matches!(block.terminator, Terminator::Branch { .. }))
            .count()
    };
    let gotos = |program: &MirProgram| {
        program
            .blocks()
            .filter(|(_, block)| matches!(block.terminator, Terminator::Goto(_)))
            .count()
    };
    let (fresh, _types) = executable(source);
    assert_eq!(branches(&fresh), 2, "source has two branches");
    let gotos_before = gotos(&fresh);
    let (optimized, _types_opt, _after) =
        assert_equivalence(source, OptimizationLevel::O2, ExecutionValue::Int(2));
    assert_eq!(branches(&optimized), 0, "both branches must fold to gotos");
    assert_eq!(
        gotos(&optimized),
        gotos_before + 2,
        "each fold adds one goto"
    );
}

#[test]
fn list_len_not_hoisted_shape_stable() {
    // Push, read len, push: the len read sits between the pushes in
    // program order. The passes move no instructions, so the read
    // keeps its instruction and its order relative to the pushes, and
    // the value (1, not 2) proves nothing was re-ordered semantically.
    let source = concat!(
        "def main() -> Int:\n",
        "    xs := list_new()\n",
        "    list_push(xs, 0)\n",
        "    n := list_len(xs)\n",
        "    list_push(xs, 1)\n",
        "    return n\n",
    );
    let (optimized, _types, after) =
        assert_equivalence(source, OptimizationLevel::O2, ExecutionValue::Int(1));
    // The len read is the third instruction and is still a builtin
    // call after optimization.
    let ids = shape(&optimized)[0].1.clone();
    let len_instruction = optimized
        .instruction(ids[2])
        .expect("third instruction exists");
    assert!(
        matches!(
            len_instruction.kind,
            InstructionKind::Assign {
                value: Rvalue::Builtin { .. },
                ..
            }
        ),
        "the list_len read must keep its instruction: {len_instruction:?}"
    );
    assert_eq!(
        after.value,
        ExecutionValue::Int(1),
        "a hoisted len would read 2"
    );
}

#[test]
fn list_operations_equivalence() {
    assert_equivalence(
        concat!(
            "def main() -> Int:\n",
            "    xs := list_new()\n",
            "    list_push(xs, 1)\n",
            "    list_push(xs, 2)\n",
            "    list_set(xs, 0, 7)\n",
            "    return list_get(xs, 0) + list_get(xs, 1) + list_len(xs)\n",
        ),
        OptimizationLevel::O2,
        ExecutionValue::Int(11),
    );
}

#[test]
fn closure_capture_equivalence() {
    // The closure captures base := 10 (a single-def constant). The
    // capture operand lives in the read-only operand list arena and is
    // not propagated; the value is unchanged.
    assert_equivalence(
        concat!(
            "def main() -> Int:\n",
            "    base := 10\n",
            "    add := fn(x: Int) -> Int:\n",
            "        return base + x\n",
            "    first := add(5)\n",
            "    return first + add(10)\n",
        ),
        OptimizationLevel::O2,
        ExecutionValue::Int(35),
    );
}

#[test]
fn async_await_spawn_equivalence() {
    // Async: await + string constant return (strings are never
    // propagated), plus a detached spawn with printed output.
    let source = concat!(
        "async def first() -> Str:\n",
        "    return \"ready\"\n",
        "async def main():\n",
        "    result := first().await\n",
        "    print_str(result)\n",
    );
    let (_optimized, _types, after) =
        assert_equivalence(source, OptimizationLevel::O2, ExecutionValue::Void);
    assert_eq!(after.output, vec!["ready\n".to_owned()]);

    let spawn_source = concat!(
        "def main():\n",
        "    xs := list_new()\n",
        "    spawn fn():\n",
        "        print_str(\"done\")\n",
    );
    let (_o, _t, after_spawn) =
        assert_equivalence(spawn_source, OptimizationLevel::O2, ExecutionValue::Void);
    assert_eq!(after_spawn.output, vec!["done\n".to_owned()]);
}

// ── growth, limits, determinism ────────────────────────────────────────────

#[test]
fn linear_growth_20_vs_200() {
    // A loop over a size parameter N: the pipeline completes at both
    // sizes, equivalence holds, and the optimized step count grows
    // approximately linearly (bounded away from quadratic).
    let generated = |count: i64| {
        format!(
            "def main() -> Int:\n    mut i := 0\n    mut total := 0\n    while i < {count}:\n        total = total + i\n        i = i + 1\n    return total\n"
        )
    };
    let (small, small_types) = executable(&generated(20));
    let (large, large_types) = executable(&generated(200));
    let entry_small = entry_function(&small);
    let entry_large = entry_function(&large);
    let before_small = run_legacy(&small, &small_types, entry_small);
    let before_large = run_legacy(&large, &large_types, entry_large);

    let mut small_opt = small.clone();
    let mut large_opt = large.clone();
    run_optimization(&mut small_opt, &small_types, OptimizationLevel::O2).unwrap();
    run_optimization(&mut large_opt, &large_types, OptimizationLevel::O2).unwrap();
    let after_small = run_legacy(&small_opt, &small_types, entry_small);
    let after_large = run_legacy(&large_opt, &large_types, entry_large);

    assert_eq!(
        after_small.value,
        ExecutionValue::Int((0..20).map(i64::from).sum::<i64>())
    );
    assert_eq!(
        after_large.value,
        ExecutionValue::Int((0..200).map(i64::from).sum::<i64>())
    );
    assert_eq!(after_small.value, before_small.value);
    assert_eq!(after_large.value, before_large.value);

    let ratio = after_large.stats.steps as f64 / after_small.stats.steps as f64;
    assert!(
        (5.0..16.0).contains(&ratio),
        "optimized step ratio 200/20 = {ratio}: expected roughly linear (10x), not quadratic"
    );
}

#[test]
fn tight_limits_exact_step_budget() {
    // Every optimized gate program runs inside its own observed step
    // budget: no hidden step inflation, and the run is deterministic
    // (the second run inside the exact budget also succeeds).
    let source = concat!(
        "def main() -> Int:\n",
        "    a := 40\n",
        "    b := 2\n",
        "    c := a + b\n",
        "    d := c * 2\n",
        "    e := d - 1\n",
        "    f := e + 1\n",
        "    return f\n",
    );
    let (mut program, types) = executable(source);
    let entry = entry_function(&program);
    run_optimization(&mut program, &types, OptimizationLevel::O2).unwrap();
    let observed = run_legacy(&program, &types, entry);
    let exact = InterpreterLimits {
        max_steps: observed.stats.steps,
        ..InterpreterLimits::default()
    };
    assert!(
        legacy_with_limits(&program, &types, entry, exact).is_ok(),
        "the optimized run must fit its exact observed step budget"
    );
    assert!(
        legacy_with_limits(&program, &types, entry, exact).is_ok(),
        "the run must be deterministic inside the exact budget"
    );
}

#[test]
fn pipeline_is_deterministic() {
    // Two fresh builds of the same source optimized at the same level
    // produce identical snapshots.
    let source = concat!(
        "def main() -> Int:\n",
        "    a := 1\n",
        "    b := a + 2\n",
        "    t := a < b\n",
        "    if t:\n",
        "        return b\n",
        "    else:\n",
        "        return a\n",
    );
    let (mut first, types) = executable(source);
    let (mut second, _types2) = executable(source);
    run_optimization(&mut first, &types, OptimizationLevel::O2).unwrap();
    run_optimization(&mut second, &types, OptimizationLevel::O2).unwrap();
    assert_eq!(mir_snapshot(&first), mir_snapshot(&second));
}

// ── compile-fail gates: exact manager codes ────────────────────────────────

/// A stub pass that writes a Bool constant into an Int-typed target:
/// its output must fail the manager's post-pass verification.
struct TypeBreakingPass;

impl MirPass for TypeBreakingPass {
    fn name(&self) -> &'static str {
        "type-breaking-stub"
    }

    fn run(
        &mut self,
        program: &mut MirProgram,
        _context: &PassContext<'_>,
    ) -> Result<(), PassFailure> {
        let (id, target) = program
            .instructions()
            .find_map(|(id, instruction)| {
                if let InstructionKind::Assign {
                    target,
                    value: Rvalue::Use(Operand::Constant(Constant::Integer(_))),
                } = &instruction.kind
                {
                    Some((id, *target))
                } else {
                    None
                }
            })
            .ok_or(PassFailure {
                message: "stub: no integer constant def found",
            })?;
        program
            .instruction_mut(id)
            .expect("instruction exists")
            .kind = InstructionKind::Assign {
            target,
            value: Rvalue::Use(Operand::Constant(Constant::Bool(true))),
        };
        Ok(())
    }
}

/// Apply the type-breaking rewrite by hand (outside the manager).
fn break_types_in_place(program: &mut MirProgram) {
    let (id, target) = program
        .instructions()
        .find_map(|(id, instruction)| {
            if let InstructionKind::Assign {
                target,
                value: Rvalue::Use(Operand::Constant(Constant::Integer(_))),
            } = &instruction.kind
            {
                Some((id, *target))
            } else {
                None
            }
        })
        .expect("test programs contain an integer constant def");
    program
        .instruction_mut(id)
        .expect("instruction exists")
        .kind = InstructionKind::Assign {
        target,
        value: Rvalue::Use(Operand::Constant(Constant::Bool(true))),
    };
}

/// A stub pass that requires `OwnershipBalance`, which the 4F passes
/// never establish.
struct BalanceRequiringPass;

impl MirPass for BalanceRequiringPass {
    fn name(&self) -> &'static str {
        "balance-requiring-stub"
    }

    fn required(&self) -> &'static [MirInvariant] {
        &[MirInvariant::OwnershipBalance]
    }

    fn run(
        &mut self,
        _program: &mut MirProgram,
        _context: &PassContext<'_>,
    ) -> Result<(), PassFailure> {
        Ok(())
    }
}

/// A stub pass that fails.
struct FailingPass;

impl MirPass for FailingPass {
    fn name(&self) -> &'static str {
        "failing-stub"
    }

    fn run(
        &mut self,
        _program: &mut MirProgram,
        _context: &PassContext<'_>,
    ) -> Result<(), PassFailure> {
        Err(PassFailure {
            message: "stub failure",
        })
    }
}

#[test]
fn stub_pass_type_violation_is_e4204() {
    let (mut program, types) = executable("def main() -> Int:\n    a := 1\n    return a\n");
    let mut manager = PassManager::new();
    manager.push(TypeBreakingPass);
    let error = manager
        .run(&mut program, &types)
        .expect_err("a type-violating rewrite must be rejected");
    assert_eq!(error.code(), "E4204");
    assert_eq!(error.pass, Some("type-breaking-stub"));
    match &error.kind {
        PassManagerErrorKind::InvalidPassOutput(diagnostics) => {
            assert!(
                !diagnostics.is_empty(),
                "the post-pass verification must report the type mismatch"
            );
            assert!(
                diagnostics.iter().any(|diagnostic| matches!(
                    diagnostic.kind,
                    lpp_mir::MirVerificationErrorKind::TypeMismatch { .. }
                )),
                "expected a TypeMismatch diagnostic, got {diagnostics:?}"
            );
        }
        other => panic!("expected InvalidPassOutput, got {other:?}"),
    }
}

#[test]
fn stub_pass_unmet_precondition_is_e4202() {
    let (mut program, types) = executable("def main() -> Int:\n    a := 1\n    return a\n");
    let mut manager = PassManager::new();
    manager.push(BalanceRequiringPass);
    let error = manager
        .run(&mut program, &types)
        .expect_err("OwnershipBalance was never established");
    assert_eq!(error.code(), "E4202");
    assert_eq!(error.pass, Some("balance-requiring-stub"));
    assert!(matches!(
        error.kind,
        PassManagerErrorKind::UnmetPrecondition(MirInvariant::OwnershipBalance)
    ));
}

#[test]
fn stub_pass_failure_is_e4203() {
    let (mut program, types) = executable("def main() -> Int:\n    a := 1\n    return a\n");
    let mut manager = PassManager::new();
    manager.push(FailingPass);
    let error = manager
        .run(&mut program, &types)
        .expect_err("the stub must fail the manager run");
    assert_eq!(error.code(), "E4203");
    assert_eq!(error.pass, Some("failing-stub"));
    assert!(matches!(error.kind, PassManagerErrorKind::PassFailed(_)));
}

#[test]
fn invalid_input_is_e4201() {
    let (mut program, types) = executable("def main() -> Int:\n    a := 1\n    return a\n");
    break_types_in_place(&mut program);
    let mut manager = PassManager::new();
    manager.push(TypeBreakingPass);
    let error = manager
        .run(&mut program, &types)
        .expect_err("already-invalid MIR must fail at entry");
    assert_eq!(error.code(), "E4201");
    assert!(matches!(
        error.kind,
        PassManagerErrorKind::InitialVerification(_)
    ));
}

#[test]
fn optimization_pipeline_preserves_shape_on_a_gate_program() {
    // The canonical single-sweep manager (used by the level runs)
    // executes the three named passes and leaves the shape intact.
    let (mut program, types) =
        executable("def main() -> Int:\n    a := 1\n    b := a + 2\n    return b\n");
    let before_shape = shape(&program);
    let outcome = optimization_passes().run(&mut program, &types).unwrap();
    assert_eq!(
        outcome.executed,
        ["const-fold", "const-prop", "branch-fold"]
    );
    assert_eq!(shape(&program), before_shape);
}
