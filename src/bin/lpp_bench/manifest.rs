use super::*;

// ═══════════════════════════════════════════════════════════════════════════
//  King20 manifest
// ═══════════════════════════════════════════════════════════════════════════

pub(super) fn load_king20() -> Vec<King20Case> {
    let root = repo_root();
    vec![
        (1, "recursive-fib-35", "benchmarks/bench_fib.lpp", "9227465"),
        (2, "loop-10m", "benchmarks/bench_loop.lpp", "49999995000000"),
        (
            3,
            "call-chain-1m",
            "benchmarks/bench_calls.lpp",
            "500000500000",
        ),
        (4, "integer-arithmetic", "tests/arith.lpp", "15\n5\n50\n2"),
        (5, "conditional-branches", "tests/branches.lpp", "1\n0\n1"),
        (6, "nested-direct-calls", "tests/nested_calls.lpp", "120"),
        (7, "immutable-closure", "tests/closure_test.lpp", "52"),
        (8, "arc-list-int", "tests/list_safety.lpp", "3\n5\n13"),
        (9, "owned-struct-return", "tests/owned_return.lpp", "1"),
        (
            10,
            "branch-owned-return",
            "tests/arc_branch_return.lpp",
            "1",
        ),
        (
            11,
            "nested-struct-destructor",
            "tests/arc_nested_struct.lpp",
            "1",
        ),
        (12, "direct-arc-alias", "tests/arc_direct_alias.lpp", "1"),
        (
            13,
            "closure-arc-capture",
            "tests/arc_closure_capture.lpp",
            "0",
        ),
        (
            14,
            "borrowed-parameter-return",
            "tests/arc_borrowed_return.lpp",
            "1",
        ),
        (
            15,
            "borrowed-field-return",
            "tests/arc_borrowed_field_return.lpp",
            "1",
        ),
        (16, "field-alias", "tests/arc_field_alias.lpp", "1"),
        (17, "list-int-alias", "tests/arc_list_alias.lpp", "7"),
        (
            18,
            "list-custom-ownership",
            "tests/arc_list_custom.lpp",
            "1",
        ),
        (
            19,
            "nested-branch-alias",
            "tests/arc_nested_branch_alias.lpp",
            "1",
        ),
        (
            20,
            "closure-branch-capture",
            "tests/arc_closure_branch_capture.lpp",
            "0",
        ),
    ]
    .into_iter()
    .map(|(id, name, src, exp)| King20Case {
        id,
        name: name.to_string(),
        source: root.join(src),
        expected: exp.to_string(),
    })
    .collect()
}
