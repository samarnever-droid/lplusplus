use super::*;

const EXTENDED_LEGACY_BUILTIN_CORPUS: &str = r#"
def echo(s: Str) -> Void:
    print_str(s)

def main() -> Int:
    word := "borrowed"
    echo(word)
    print_int(str_len(word))
    print_str(str_substr("Hello", 1, 3))
    print_str(str_substr("Hello", 99, 3))
    print_str(str_repeat("ab", 3))
    print_str(char_at("hello", 1))
    print_int(ord("A"))
    print_str(chr(66))
    print_int(ord("🙂"))
    print_str(chr(128578))
    print_int(lpp_str_eq("same", "same"))
    kept := str_repeat("kept", 1)
    alias := str_replace(kept, "", "unused")
    print_str(kept)
    print_str(alias)
    parts := str_split("a,b,c", ord(","))
    print_int(list_len(parts))
    for part in parts:
        print_str(part)
    print_int(int_pow(2, 10))
    print_int(int_pow(2, -1))
    print_float(lpp_pow(2.0, 0.5))
    print_float(sin(1.0))
    print_float(cos(1.0))
    return 0
"#;

#[test]
fn extended_legacy_builtins_and_borrowed_calls_match_the_arc_oracle() {
    let (program, types, package) = pipeline(EXTENDED_LEGACY_BUILTIN_CORPUS);
    let names = Names(&package.names.symbols);
    let entry = main_function(&program, &package.names.symbols);

    let module = compile(&program, &types, &names);
    let (stdout, status) = run_wasm(&module.object, "extended-legacy-builtins");
    assert_eq!(
        status, 0,
        "extended legacy builtin corpus exited {status}:\n{stdout}"
    );

    let oracle = run_arc_oracle(&program, &types, entry);
    assert_eq!(
        stdout,
        oracle.output.concat(),
        "extended legacy builtins diverged from the ARC oracle"
    );
}

#[test]
fn arc_heap_reuses_freed_blocks_under_repeated_managed_churn() {
    fn compile_churn(iterations: usize, label: &str) -> (String, i32, u32) {
        let source = format!(
            "struct Box:\n    value: Int\n\ndef main() -> Int:\n    mut total := 0\n    mut i := 0\n    while i < {iterations}:\n        xs := [Box(i), Box(i + 1)]\n        value := list_get(xs, 0)\n        total = total + value.value - i\n        ys := [i, i + 1]\n        total = total + list_get(ys, 0) - i\n        text := str_concat(\"id:\", int_to_str(i))\n        if str_len(text) > 0:\n            total = total + 1\n        i = i + 1\n    print(total)\n    return 0\n"
        );
        let (program, types, package) = pipeline(&source);
        let names = Names(&package.names.symbols);
        let module = compile(&program, &types, &names);
        run_wasm_memory_pages(&module.object, label)
    }

    let (small_out, small_status, small_pages) = compile_churn(100, "heap_reuse_small");
    let (large_out, large_status, large_pages) = compile_churn(100_000, "heap_reuse_large");
    assert_eq!((small_out.as_str(), small_status), ("100\n", 0));
    assert_eq!((large_out.as_str(), large_status), ("100000\n", 0));
    eprintln!(
        "WASM ARC churn: 100 iterations={small_pages} pages; 100,000 iterations={large_pages} pages"
    );
    assert!(
        large_pages <= small_pages + 1,
        "ARC churn should reuse freed blocks: memory grew from {small_pages} pages at 100 iterations to {large_pages} pages at 100,000"
    );
}

#[test]
fn integer_division_panic_and_overflow_match_the_oracle() {
    for (operator, label) in [("/", "div_zero"), ("%", "mod_zero")] {
        let zero = format!(
            "def f(n: Int) -> Int:\n    return 100 {operator} n\n\ndef main() -> Int:\n    print(f(0))\n    return 0\n"
        );
        let (program, types, package) = pipeline(&zero);
        let names = Names(&package.names.symbols);
        let module = compile(&program, &types, &names);
        let (_, status) = run_wasm(&module.object, label);
        assert_eq!(
            status, 101,
            "integer {operator}-by-zero must exit via WASI status 101"
        );
    }

    let wrapping = "def main() -> Int:\n    m := 0 - 9223372036854775807 - 1\n    print(m / (0 - 1))\n    print(m % (0 - 1))\n    print(100 / 7)\n    print(100 % 7)\n    return 0\n";
    let (program, types, package) = pipeline(wrapping);
    let names = Names(&package.names.symbols);
    let module = compile(&program, &types, &names);
    let (stdout, status) = run_wasm(&module.object, "div_wrap");
    assert_eq!(status, 0);
    assert_eq!(stdout, "-9223372036854775808\n0\n14\n2\n");
}
