use super::*;

pub(super) const BUILTIN_CORPUS: &str = r#"
def print_family_check() -> Int:
    print_int(42)
    print_float(8.25)
    print_bool(true)
    print_str("hello_print")
    print(7)
    print(3.5)
    print(false)
    print("direct_print")
    mut s := "consumed"
    write_str(s)
    print_int(1)
    return 0

def string_ops_check() -> Int:
    a := str_concat("hello", " world")
    if str_len(a) != 11:
        print_str("fail_str_len")
    if not str_contains(a, "lo w"):
        print_str("fail_str_contains")
    if not str_starts_with(a, "hello"):
        print_str("fail_str_starts")
    if not str_ends_with(a, "world"):
        print_str("fail_str_ends")
    if str_find(a, "wor") != 6:
        print_str("fail_str_find")
    if str_find(a, "z") != -1:
        print_str("fail_str_find_miss")
    b := str_replace(a, "world", "LPP")
    print_str(b)
    c := str_trim("  pad  ")
    print_str(c)
    print_str(str_lower("MiXeD"))
    print_str(str_upper("mixed"))
    print_str(str_replace("aaa", "a", "bb"))
    print_str(str_replace("abc", "", "x"))
    if str_len(str_trim("\t\n x \r")) != 1:
        print_str("fail_trim_edges")
    return 0

def conversions_check() -> Int:
    if int_to_str(-1234) != "-1234":
        print_str("fail_int_to_str")
    if u64_to_str(-1) != "18446744073709551615":
        print_str("fail_u64_to_str")
    if u64_to_hex(255) != "ff":
        print_str("fail_u64_to_hex")
    if bool_to_str(true) != "true":
        print_str("fail_bool_to_str")
    if float_to_str(0.00001) != "1e-05":
        print_str("fail_float_exp")
    if float_to_str(1234567.0) != "1.23457e+06":
        print_str("fail_float_exp_up")
    if float_to_str(0.0001) != "0.0001":
        print_str("fail_float_decimal")
    if float_to_str(-0.00001) != "-1e-05":
        print_str("fail_float_neg_exp")
    if float_to_str(999999.0) != "999999":
        print_str("fail_float_whole")
    if float_to_str(1.0) != "1":
        print_str("fail_float_one")
    print_str(float_to_str(0.00001))
    print_str(float_to_str(1234567.0))
    if str_to_int("  -42") != -42:
        print_str("fail_str_to_int")
    if str_to_int("99999999999999999999") != 9223372036854775807:
        print_str("fail_str_to_int_saturate")
    if str_to_int("abc") != 0:
        print_str("fail_str_to_int_none")
    if str_to_u64("0x1F") != 31:
        print_str("fail_str_to_u64_hex")
    if str_to_u64("255") != 255:
        print_str("fail_str_to_u64_dec")
    if str_to_u64("ff") != 0:
        print_str("fail_str_to_u64_no_prefix")
    return 0

def integer_helpers_check() -> Int:
    if abs(-5) != 5 or abs(5) != 5:
        print_str("fail_abs")
    if abs(sub_wrap(-9223372036854775807, 1)) != sub_wrap(-9223372036854775807, 1):
        print_str("fail_abs_min")
    if min(-3, 7) != -3 or max(-3, 7) != 7:
        print_str("fail_min_max")
    if min_u(-1, 1) != 1 or max_u(-1, 1) != -1:
        print_str("fail_min_max_u")
    if lt_u(-1, 1) == 1 or ge_u(1, -1) == 1 or gt_u(-1, 1) == 0 or lt_u(1, -1) == 0 or le_u(1, -1) == 0:
        print_str("fail_cmp_u")
    if shr_u(8, 1) != 4 or shl_u(4, 3) != 32:
        print_str("fail_shift")
    if shr_u(1, 64) != 0 or shl_u(1, -1) != 0:
        print_str("fail_shift_bounds")
    if div_u(100, 7) != 14 or rem_u(100, 7) != 2:
        print_str("fail_div_rem")
    if div_u(-7, 2) != 9223372036854775804:
        print_str("fail_div_u_min")
    if popcount64(10) != 2 or clz64(1) != 63 or ctz64(4) != 2:
        print_str("fail_bit_count")
    if clz64(0) != 64 or ctz64(0) != 64:
        print_str("fail_bit_count_zero")
    if bswap16(4660) != 13330:
        print_str("fail_bswap16")
    if bswap32(16909060) != 67305985:
        print_str("fail_bswap32")
    if rotl64(1, 63) != sub_wrap(-9223372036854775807, 1):
        print_str("fail_rotl64")
    if rotl64(3, 1) != 6 or rotr64(6, 1) != 3:
        print_str("fail_rotate64")
    if rotl64(-1, 5) != -1:
        print_str("fail_rotl64_allones")
    if rotl32(1, 31) != 2147483648:
        print_str("fail_rotl32")
    if rotr32(2147483648, 1) != 1073741824:
        print_str("fail_rotr32")
    if trunc_u8(-1) != 255 or trunc_u16(-1) != 65535 or trunc_u32(-1) != 4294967295:
        print_str("fail_trunc_u")
    if trunc_i8(-1) != -1 or trunc_i16(-1) != -1:
        print_str("fail_trunc_i")
    if trunc_i32(2147483648) != -2147483648:
        print_str("fail_trunc_i32")
    if add_checked(10, 20) != 30:
        print_str("fail_add_checked")
    if sub_checked(10, 40) != -30:
        print_str("fail_sub_checked")
    if mul_checked(1000000, 1000000) != 1000000000000:
        print_str("fail_mul_checked")
    if add_wrap(sub_wrap(-9223372036854775807, 1), 1) != -9223372036854775807:
        print_str("fail_add_wrap")
    if sub_wrap(9223372036854775807, 1) != 9223372036854775806:
        print_str("fail_sub_wrap")
    if mul_wrap(3037000500, 3037000500) != -9223372036709301616:
        print_str("fail_mul_wrap")
    return 0

def float_ops_check() -> Int:
    if floor(3.7) != 3.0:
        print_str("fail_floor")
    if ceil(3.1) != 4.0:
        print_str("fail_ceil")
    if pow(2.0, 3.0) != 8.0:
        print_str("fail_pow")
    if sqrt(16.0) != 4.0:
        print_str("fail_sqrt")
    if fmod(7.5, 2.0) != 1.5:
        print_str("fail_fmod")
    if fmod(-7.5, 2.0) != 0.5:
        print_str("fail_fmod_negative")
    print_float(fmod(10.75, 2.0))
    return 0

def list_family_check() -> Int:
    xs := list_new()
    list_push(xs, 10)
    list_push(xs, 20)
    list_set(xs, 0, 15)
    if list_get(xs, 0) != 15 or list_get(xs, 1) != 20 or list_len(xs) != 2:
        print_str("fail_list_int")
    fs := list_new()
    list_push(fs, 1.5)
    list_set(fs, 0, 2.25)
    if list_get(fs, 0) != 2.25:
        print_str("fail_list_float")
    bs := list_new()
    list_push(bs, true)
    list_set(bs, 0, false)
    if list_get(bs, 0):
        print_str("fail_list_bool")
    ms := list_new()
    list_push(ms, "one")
    list_set(ms, 0, "two")
    mut s := "moved_in"
    list_push(ms, s)
    if list_get(ms, 0) != "two" or list_get(ms, 1) != "moved_in" or list_len(ms) != 2:
        print_str("fail_list_str")
    return 0

def main() -> Int:
    print_family_check()
    string_ops_check()
    conversions_check()
    integer_helpers_check()
    float_ops_check()
    list_family_check()
    print_str("all_ok")
    return 0
"#;

// ── corpus 3: slices + SIMD (self-checking) ───────────────────────────────
//
// Hand-computed stdout (the interpreter reports `UnsupportedBuiltin`
// for both families, so this corpus is verified against its exact
// expected output):
//   movemask 516 (byte 2 of lane 0, byte 9 of lane 1 carry 0x6e)
//   add-sum 7237123, extract-1 of sub 28158, mul-sum 14474240,
//   not-sum -7237122, splat-sum 220, shr0 3604480, xor-sum 9,
//   and-sum 8, or-sum 10, shr_var-sum 3618560, checksum(10) 143,
//   slice len 2 / element 30, bool slice true, "world", str-slice len 5.

#[test]
#[cfg_attr(
    target_os = "windows",
    ignore = "MSVC runtime setup is covered by the Windows driver smoke gate"
)]
fn builtin_family_corpus_matches_the_arc_oracle() {
    let (program, types, package) = pipeline(BUILTIN_CORPUS);
    let names = Names(&package.names.symbols);
    let module = compile(&program, &types, &names);
    let (stdout, status) = link_and_run(&module.object, "builtin");
    assert_eq!(status, 0, "builtin object exited {status}:\n{stdout}");

    let entry = main_function(&program, &package.names.symbols);
    let oracle = run_arc_oracle(&program, &types, entry);
    let expected: String = oracle.output.concat();
    assert_eq!(
        stdout, expected,
        "builtin corpus: object stdout diverges from the arc oracle"
    );
    expect_success_markers(&stdout, "builtin");
}

// ── gate 3: self-check slices + SIMD ──────────────────────────────────────
