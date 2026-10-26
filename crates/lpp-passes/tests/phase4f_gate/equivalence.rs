use super::*;

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
