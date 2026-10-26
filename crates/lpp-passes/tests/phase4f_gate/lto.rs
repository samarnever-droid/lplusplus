use super::*;

use std::collections::BTreeSet;

use lpp_passes::{run_optimization_with_lto, run_optimization_with_profile};

fn direct_call_count(program: &MirProgram) -> usize {
    program
        .instructions()
        .filter(|(_, instruction)| {
            matches!(
                instruction.kind,
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

#[test]
fn whole_program_lto_inlines_a_tiny_scalar_callee_without_changing_results() {
    let source = r#"
def plus_one(value: Int) -> Int:
    return value + 1

def main() -> Int:
    return plus_one(41)
"#;
    let (original, types) = executable(source);
    let entry = entry_function(&original);
    let before = run_legacy(&original, &types, entry);
    let before_calls = direct_call_count(&original);
    assert!(before_calls > 0, "the fixture must contain the direct call");

    let mut ordinary = original.clone();
    run_optimization(&mut ordinary, &types, OptimizationLevel::O2).unwrap();
    assert_eq!(direct_call_count(&ordinary), before_calls);

    let mut optimized = original.clone();
    run_optimization_with_lto(&mut optimized, &types, OptimizationLevel::O2, true).unwrap();
    assert_eq!(direct_call_count(&optimized), 0);
    assert!(verify_mir(&optimized, &types).is_empty());
    let after = run_legacy(&optimized, &types, entry);
    assert_eq!(after.value, before.value);
    assert_eq!(after.output, before.output);
    assert!(after.stats.steps < before.stats.steps);
}

#[test]
fn profile_guided_inlining_only_uses_the_hot_callee_set() {
    let source = r#"
def plus_one(value: Int) -> Int:
    return value + 1

def main() -> Int:
    return plus_one(41)
"#;
    let (original, types) = executable(source);
    let plus_one = original
        .functions()
        .next()
        .map(|(id, _)| id)
        .expect("fixture has a helper function");

    let mut cold = original.clone();
    run_optimization_with_profile(
        &mut cold,
        &types,
        OptimizationLevel::O2,
        false,
        BTreeSet::new(),
    )
    .unwrap();
    assert_eq!(direct_call_count(&cold), direct_call_count(&original));

    let mut hot = original.clone();
    run_optimization_with_profile(
        &mut hot,
        &types,
        OptimizationLevel::O2,
        false,
        BTreeSet::from([plus_one]),
    )
    .unwrap();
    assert_eq!(direct_call_count(&hot), 0);
    assert!(verify_mir(&hot, &types).is_empty());
}

#[test]
fn inliner_keeps_calls_with_effects_or_non_scalar_values() {
    let source = r#"
def identity(value: Str) -> Str:
    return value

def main() -> Int:
    value := identity("kept")
    return 42
"#;
    let (original, types) = executable(source);
    let before_calls = direct_call_count(&original);
    assert!(before_calls > 0);

    let mut optimized = original.clone();
    run_optimization_with_lto(&mut optimized, &types, OptimizationLevel::O2, true).unwrap();
    assert_eq!(direct_call_count(&optimized), before_calls);
    assert!(verify_mir(&optimized, &types).is_empty());
}
