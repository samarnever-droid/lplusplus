use super::*;

const BUILTIN_CORPUS: &str = r#"
def bits_check() -> Int:
    if bswap64(258) != 144396663052566528:
        return 101
    if bswap32(65794) != 33620224:
        return 102
    if bswap16(65794) != 513:
        return 103
    if bswap64(65794) != 144397762564194304:
        return 104
    if clz64(1) != 63:
        return 105
    if clz64(4611686018427387904) != 1:
        return 106
    if ctz64(8) != 3:
        return 107
    if ctz64(1) != 0:
        return 108
    if popcount64(255) != 8:
        return 109
    if popcount64(4294967295) != 32:
        return 110
    return 0

def rot_check() -> Int:
    if rotl64(1, 60) != 1152921504606846976:
        return 111
    if rotr64(1152921504606846976, 60) != 1:
        return 112
    if rotl64(5, 64) != 5:
        return 113
    if rotl64(5, 65) != 10:
        return 114
    if rotl64(-1, 32) != -1:
        return 115
    if rotl32(1, 30) != 1073741824:
        return 116
    if rotr32(1073741824, 30) != 1:
        return 117
    if rotl32(2147483648, 1) != 1:
        return 118
    if rotr32(-1, 1) != 4294967295:
        return 119
    return 0

def shift_check() -> Int:
    if shl_u(1, 62) != 4611686018427387904:
        return 121
    if shr_u(4611686018427387904, 62) != 1:
        return 122
    if shl_u(1, 64) != 0:
        return 123
    if shl_u(1, 100) != 0:
        return 124
    if shl_u(1, -1) != 0:
        return 125
    if shr_u(65535, 100) != 0:
        return 126
    if shr_u(-8, 1) != 9223372036854775804:
        return 127
    return 0

def div_check() -> Int:
    if div_u(10, 3) != 3:
        return 131
    if rem_u(10, 3) != 1:
        return 132
    if div_u(7, 2) != 3:
        return 133
    if rem_u(7, 2) != 1:
        return 134
    if div_u(-10, 3) != 6148914691236517202:
        return 135
    if rem_u(-10, 3) != 0:
        return 136
    return 0

def order_check() -> Int:
    if min(3, 7) != 3:
        return 141
    if max(3, 7) != 7:
        return 142
    if min(-5, 7) != -5:
        return 143
    if max(-5, 7) != 7:
        return 144
    if min_u(-1, 1) != 1:
        return 145
    if max_u(-1, 1) != -1:
        return 146
    if lt_u(-1, 1):
        return 147
    if not le_u(1, 1):
        return 148
    if gt_u(0, -1):
        return 149
    if not ge_u(-1, -1):
        return 150
    return 0

def trunc_check() -> Int:
    if trunc_u8(300) != 44:
        return 151
    if trunc_u16(100000) != 34464:
        return 152
    if trunc_u32(5000000000) != 705032704:
        return 153
    if trunc_i8(200) != -56:
        return 154
    if trunc_i16(49152) != -16384:
        return 155
    if trunc_i32(5000000000) != 705032704:
        return 156
    return 0

def wrap_check() -> Int:
    if add_wrap(9223372036854775807, 1) != (-9223372036854775807 - 1):
        return 161
    if sub_wrap(-9223372036854775807 - 1, 1) != 9223372036854775807:
        return 162
    if mul_wrap(4611686018427387904, 2) != (-9223372036854775807 - 1):
        return 163
    if add_checked(1, 2) != 3:
        return 164
    if sub_checked(1, 2) != -1:
        return 165
    if mul_checked(1000000, 1000000) != 1000000000000:
        return 166
    if add_checked(-9223372036854775807 - 1, 0) != (-9223372036854775807 - 1):
        return 167
    if abs(-5) != 5:
        return 168
    if abs(5) != 5:
        return 169
    return 0

def float_check() -> Int:
    print_float(floor(-2.5))
    print_float(ceil(-2.5))
    print_float(sqrt(144.0))
    print_float(fmod(7.5, 2.0))
    print_float(fmod(-7.5, 2.0))
    return 0

async def main():
    if bits_check() == 0:
        print_str("ok_bits")
    else:
        print_str("fail_bits")
    if rot_check() == 0:
        print_str("ok_rot")
    else:
        print_str("fail_rot")
    if shift_check() == 0:
        print_str("ok_shift")
    else:
        print_str("fail_shift")
    if div_check() == 0:
        print_str("ok_div")
    else:
        print_str("fail_div")
    if order_check() == 0:
        print_str("ok_order")
    else:
        print_str("fail_order")
    if trunc_check() == 0:
        print_str("ok_trunc")
    else:
        print_str("fail_trunc")
    if wrap_check() == 0:
        print_str("ok_wrap")
    else:
        print_str("fail_wrap")
    if float_check() == 0:
        print_str("ok_float")
    else:
        print_str("fail_float")
    print_str("all_ok")
"#;

#[test]
fn builtin_corpus_matches_the_arc_oracle() {
    let (program, types, package) = pipeline(BUILTIN_CORPUS);
    let names = Names(&package.names.symbols);
    let entry = main_function(&program, &package.names.symbols);

    let module = compile(&program, &types, &names);
    let (stdout, status) = run_wasm(&module.object, "builtin");
    assert_eq!(
        status, 0,
        "builtin corpus object exited {status}:\n{stdout}"
    );

    let oracle = run_arc_oracle(&program, &types, entry);
    let expected = oracle.output.concat();
    assert_eq!(
        stdout, expected,
        "builtin corpus: object stdout diverges from the arc oracle"
    );
    expect_success_markers(&stdout, "builtin");
}

// The 5D2b slice 4, batch 3 corpus: every string builtin, asserted
// against the ARC oracle (stdout diff, like the other corpora).
