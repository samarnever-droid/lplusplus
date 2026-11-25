use super::*;

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
        [
            "const-fold",
            "const-prop",
            "algebraic-simplify",
            "branch-fold"
        ]
    );
    assert_eq!(shape(&program), before_shape);
}
