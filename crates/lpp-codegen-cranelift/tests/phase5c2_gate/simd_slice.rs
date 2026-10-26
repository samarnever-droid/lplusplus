use super::*;

pub(super) const SIMD_SLICE_CORPUS: &str = r#"
def simd_self() -> Int:
    v := vec_i64x2(7208960, 28160)
    probe := vec_u8x16_splat(110)
    matches := vec_u8x16_eq(v, probe)
    print_int(vec_u8x16_movemask(matches))
    print_int(vec_i64x2_sum(vec_i64x2_add(v, vec_i64x2(1, 2))))
    print_int(vec_i64x2_extract(vec_i64x2_sub(v, vec_i64x2(1, 2)), 1))
    print_int(vec_i64x2_sum(vec_i64x2_mul(v, vec_i64x2(2, 2))))
    print_int(vec_i64x2_sum(vec_i64x2_not(v)))
    print_int(vec_i64x2_sum(vec_i64x2_splat(110)))
    print_int(vec_i64x2_extract(vec_i64x2_shr(v, 1), 0))
    m := vec_i64x2(3, 5)
    print_int(vec_i64x2_sum(vec_i64x2_xor(m, vec_i64x2(1, 2))))
    print_int(vec_i64x2_sum(vec_i64x2_and(m, vec_i64x2(3, 7))))
    print_int(vec_i64x2_sum(vec_i64x2_or(m, vec_i64x2(1, 2))))
    print_int(vec_i64x2_sum(vec_i64x2_shr_var(v, vec_i64x2(1, 1))))
    print_int(vec_i64_checksum(10))
    return 0

def slice_self() -> Int:
    xs := [10, 20, 30, 40]
    s := slice(xs, 1, 2)
    print_int(slice_len(s))
    print_int(slice_get(s, 1))
    yb := [true, false, true]
    sb := slice(yb, 0, 3)
    if lpp_slice_get_bool(sb, 2):
        print_str("ok_bool_true")
    text := "hello world"
    ss := str_slice(text, 6, 5)
    print_str(str_slice_to_str(ss))
    print_int(slice_len(ss))
    return 0

def main() -> Int:
    simd_self()
    slice_self()
    return 0
"#;

const SIMD_SLICE_EXPECTED: &str = "\
516
7237123
28158
14474240
-7237122
220
3604480
9
8
10
3618560
143
2
30
ok_bool_true
world
5
";

// ── the closed runtime-symbol universe (the import_signature arms) ────────

#[test]
#[cfg_attr(
    target_os = "windows",
    ignore = "MSVC runtime setup is covered by the Windows driver smoke gate"
)]
fn simd_and_slice_selfcheck_produces_the_hand_computed_output() {
    let (program, types, package) = pipeline(SIMD_SLICE_CORPUS);
    let names = Names(&package.names.symbols);
    let module = compile(&program, &types, &names);
    let (stdout, status) = link_and_run(&module.object, "simd_slice");
    assert_eq!(status, 0, "simd/slice object exited {status}:\n{stdout}");
    assert_eq!(
        stdout, SIMD_SLICE_EXPECTED,
        "simd/slice corpus: object stdout diverges from the hand-computed output"
    );
}

// ── gate 4: determinism + census ──────────────────────────────────────────
