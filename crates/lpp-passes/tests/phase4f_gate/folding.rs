use super::*;

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
