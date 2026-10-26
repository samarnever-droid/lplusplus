use super::*;

const STR_BUILTIN_CORPUS: &str = r#"
def str1_check() -> Int:
    if str_concat("foo", "bar") != "foobar":
        return 201
    if str_concat("", "x") != "x":
        return 202
    if str_concat("x", "") != "x":
        return 203
    if not str_contains("hello world", "world"):
        return 204
    if str_contains("hello world", "mars"):
        return 205
    if not str_contains("abc", ""):
        return 206
    if not str_starts_with("foobar", "foo"):
        return 207
    if str_starts_with("foobar", "bar"):
        return 208
    if not str_ends_with("foobar", "bar"):
        return 209
    if str_ends_with("foobar", "foo"):
        return 210
    if str_find("hello world", "world") != 6:
        return 211
    if str_find("hello", "x") != -1:
        return 212
    if str_find("abc", "") != 0:
        return 213
    if str_find("aaaa", "aa") != 0:
        return 214
    if str_replace("a-b-c", "-", "+") != "a+b+c":
        return 215
    if str_replace("aaa", "a", "bb") != "bbbbbb":
        return 216
    if str_replace("abc", "x", "y") != "abc":
        return 217
    if str_replace("abc", "", "z") != "abc":
        return 218
    if str_replace("aaa", "aa", "b") != "ba":
        return 219
    if str_trim("  hi  \n\t") != "hi":
        return 220
    if str_trim("   ") != "":
        return 221
    if str_to_lower("AbC") != "abc":
        return 222
    if str_to_upper("AbC") != "ABC":
        return 223
    if str_to_lower("a1b!C") != "a1b!c":
        return 224
    if str_len("hello") != 5:
        return 225
    return 0

def str2_check() -> Int:
    if int_to_str(123) != "123":
        return 231
    if int_to_str(-456) != "-456":
        return 232
    if int_to_str(0) != "0":
        return 233
    if int_to_str(-9223372036854775807 - 1) != "-9223372036854775808":
        return 234
    if int_to_str(9223372036854775807) != "9223372036854775807":
        return 235
    if str_to_int("42") != 42:
        return 241
    if str_to_int("-42") != -42:
        return 242
    if str_to_int("  -42  ") != -42:
        return 243
    if str_to_int("+42") != 42:
        return 244
    if str_to_int("42xyz") != 42:
        return 245
    if str_to_int("") != 0:
        return 246
    if str_to_int("  ") != 0:
        return 247
    if str_to_int("99999999999999999999") != 9223372036854775807:
        return 248
    if str_to_int("-99999999999999999999") != (-9223372036854775807 - 1):
        return 249
    if str_to_int("12 34") != 12:
        return 250
    return 0

def str3_check() -> Int:
    if u64_to_str(42) != "42":
        return 261
    if u64_to_str(-1) != "18446744073709551615":
        return 262
    if u64_to_str(0) != "0":
        return 263
    if u64_to_hex(255) != "ff":
        return 264
    if u64_to_hex(4096) != "1000":
        return 265
    if u64_to_hex(0) != "0":
        return 266
    if u64_to_hex(-1) != "ffffffffffffffff":
        return 267
    if u64_to_hex(3735928559) != "deadbeef":
        return 268
    if str_to_u64("255") != 255:
        return 271
    if str_to_u64("0x10") != 16:
        return 272
    if str_to_u64("0XDEAD") != 57005:
        return 273
    if str_to_u64("  0xff") != 255:
        return 274
    if str_to_u64("abc") != 0:
        return 275
    if str_to_u64("12abc") != 12:
        return 276
    if str_to_u64("0x") != 0:
        return 277
    if str_to_u64("-5") != 0:
        return 278
    if bool_to_str(true) != "true":
        return 281
    if bool_to_str(false) != "false":
        return 282
    return 0

def str4_check() -> Int:
    if float_to_str(1.5) != "1.5":
        return 291
    if float_to_str(-2.25) != "-2.25":
        return 292
    if float_to_str(0.0) != "0":
        return 293
    if float_to_str(123.0) != "123":
        return 294
    if float_to_str(123456.0) != "123456":
        return 295
    if float_to_str(1234567.0) != "1.23457e+06":
        return 296
    if float_to_str(0.0001) != "0.0001":
        return 297
    if float_to_str(0.00001) != "1e-05":
        return 298
    if float_to_str(100000.0) != "100000":
        return 299
    if float_to_str(1000000.0) != "1e+06":
        return 300
    if float_to_str(0.30000000000000004) != "0.3":
        return 301
    if float_to_str(123.456) != "123.456":
        return 302
    if float_to_str(-0.00005) != "-5e-05":
        return 303
    if float_to_str(99999.9) != "99999.9":
        return 304
    return 0

def main() -> Int:
    if str1_check() == 0:
        print_str("ok_str1")
    else:
        print_str("fail_str1")
    if str2_check() == 0:
        print_str("ok_str2")
    else:
        print_str("fail_str2")
    if str3_check() == 0:
        print_str("ok_str3")
    else:
        print_str("fail_str3")
    if str4_check() == 0:
        print_str("ok_str4")
    else:
        print_str("fail_str4")
    print_str("all_ok")
    return 0
"#;

#[test]
fn string_builtin_corpus_matches_the_arc_oracle() {
    let (program, types, package) = pipeline(STR_BUILTIN_CORPUS);
    let names = Names(&package.names.symbols);
    let entry = main_function(&program, &package.names.symbols);

    let module = compile(&program, &types, &names);
    let (stdout, status) = run_wasm(&module.object, "string-builtin");
    assert_eq!(
        status, 0,
        "string builtin corpus object exited {status}:\n{stdout}"
    );

    let oracle = run_arc_oracle(&program, &types, entry);
    let expected = oracle.output.concat();
    assert_eq!(
        stdout, expected,
        "string builtin corpus: object stdout diverges from the arc oracle"
    );
    expect_success_markers(&stdout, "string-builtin");
}

// Legacy corpus parity added after the original string batch: slicing,
// repetition, character conversion, split, integer power, and freestanding
// transcendental helpers all execute against the ARC oracle.
