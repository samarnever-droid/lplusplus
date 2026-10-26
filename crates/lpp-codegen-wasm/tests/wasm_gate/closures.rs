use super::*;

const CLOSURE_CORPUS: &str = r#"
struct Point:
    x: Int
    y: Int

def counter_check() -> Int:
    mut count := 0
    counter := fn() -> Int:
        count = count + 1
        return count
    first_call := counter()
    second_call := counter()
    return count + first_call + second_call

def list_closure_check() -> Int:
    add_one := fn(value: Int) -> Int: value + 1
    callbacks := [add_one]
    callback := list_get(callbacks, 0)
    return callback(41)

def nested_capture_check() -> Int:
    base := 10
    add := fn(x: Int) -> Int:
        return base + x
    first_call := add(5)
    return first_call + add(10)

def managed_capture_check() -> Int:
    mut p := Point(3, 7)
    bump := fn() -> Int:
        p = Point(p.x + 1, p.y)
        return p.y
    a := bump()
    b := bump()
    return a + b + p.x

def sync_value_check() -> Int:
    double := fn(x: Int) -> Int: x * 2
    f := double
    return f(21)

def cycle_check() -> Int:
    mut total := 0
    xs := list_new()
    f := fn(seed: Int, extra: Int) -> Int:
        total = total + list_len(xs)
        return total + seed + extra
    g := f
    list_push(xs, f)
    return g(100, 7)

def managed_read_only() -> Int:
    p := Point(3, 7)
    read := fn() -> Int:
        return p.x + p.y
    return read() + p.x

def minimal_capture() -> Int:
    p := Point(3, 7)
    read := fn() -> Int:
        return p.x
    return read()

def capture_call_twice() -> Int:
    p := Point(3, 7)
    read := fn() -> Int:
        return p.x
    a := read()
    b := read()
    return a + b

def no_closure() -> Int:
    p := Point(3, 7)
    return p.x

def main() -> Int:
    if counter_check() == 5:
        print_str("ok_counter")
    else:
        print_str("fail_counter")
    if list_closure_check() == 42:
        print_str("ok_list_closure")
    else:
        print_str("fail_list_closure")
    if nested_capture_check() == 35:
        print_str("ok_nested")
    else:
        print_str("fail_nested")
    if managed_capture_check() == 19:
        print_str("ok_managed_capture")
    else:
        print_str("fail_managed_capture")
    if sync_value_check() == 42:
        print_str("ok_sync_value")
    else:
        print_str("fail_sync_value")
    if cycle_check() == 108:
        print_str("ok_cycle")
    else:
        print_str("fail_cycle")
    if managed_read_only() == 13:
        print_str("ok_managed_read_only")
    else:
        print_str("fail_managed_read_only")
    if minimal_capture() == 3:
        print_str("ok_minimal")
    else:
        print_str("fail_minimal")
    if capture_call_twice() == 6:
        print_str("ok_call_twice")
    else:
        print_str("fail_call_twice")
    if no_closure() == 3:
        print_str("ok_no_closure")
    else:
        print_str("fail_no_closure")
    print_str("all_ok")
    return 0
"#;

// ── 5D2b slice 2: bare function values ─────────────────────────────────────
//
// A bare function value (`f := add`) is a 16-byte capsule
// `[code, env=NULL]` on the ARC heap; it dispatches through the same
// table and call types as a closure. The oracle (ARC-mode interpreter)
// carries `RuntimeValue::Function`, so the differential gate covers the
// materialization, aliasing, reassignment, and list-element paths.

const FUNCTION_VALUE_CORPUS: &str = r#"
def add(a: Int, b: Int) -> Int:
    return a + b

def mul(a: Int, b: Int) -> Int:
    return a * b

def sub(a: Int, b: Int) -> Int:
    return a - b

def value_alias_check() -> Int:
    f := add
    g := mul
    a := f(1, 2)
    b := g(3, 4)
    h := g
    c := h(5, 6)
    return a + b + c

def value_swap_check() -> Int:
    f := add
    g := mul
    f := g
    g := add
    a := f(2, 3)
    b := g(9, 1)
    return a + b

def value_reassign_check() -> Int:
    f := add
    a := f(1, 1)
    f := sub
    b := f(10, 4)
    return a + b

def value_in_list_check() -> Int:
    ops := [add, mul]
    op0 := list_get(ops, 0)
    op1 := list_get(ops, 1)
    a := op0(7, 1)
    b := op1(2, 5)
    return a + b

def value_call_twice_check() -> Int:
    f := mul
    a := f(2, 3)
    b := f(4, 5)
    return a + b

def main() -> Int:
    if value_alias_check() == 45:
        print_str("ok_value_alias")
    else:
        print_str("fail_value_alias")
    if value_swap_check() == 16:
        print_str("ok_value_swap")
    else:
        print_str("fail_value_swap")
    if value_reassign_check() == 8:
        print_str("ok_value_reassign")
    else:
        print_str("fail_value_reassign")
    if value_in_list_check() == 18:
        print_str("ok_value_list")
    else:
        print_str("fail_value_list")
    if value_call_twice_check() == 26:
        print_str("ok_value_twice")
    else:
        print_str("fail_value_twice")
    print_str("all_ok")
    return 0
"#;

#[test]
fn function_value_corpus_matches_the_arc_oracle() {
    let (program, types, package) = pipeline(FUNCTION_VALUE_CORPUS);
    let names = Names(&package.names.symbols);
    let entry = main_function(&program, &package.names.symbols);

    let module = compile(&program, &types, &names);
    let (stdout, status) = run_wasm(&module.object, "function_value");
    assert_eq!(
        status, 0,
        "function value corpus object exited {status}:\n{stdout}"
    );

    let oracle = run_arc_oracle(&program, &types, entry);
    let expected = oracle.output.concat();
    assert_eq!(
        stdout, expected,
        "function value corpus: object stdout diverges from the arc oracle"
    );
    expect_success_markers(&stdout, "function_value");
}

// ── 5D2b gate 1: differential closure corpus ───────────────────────────────

#[test]
fn closure_corpus_matches_the_arc_oracle() {
    let (program, types, package) = pipeline(CLOSURE_CORPUS);
    let names = Names(&package.names.symbols);
    let entry = main_function(&program, &package.names.symbols);

    let module = compile(&program, &types, &names);
    let (stdout, status) = run_wasm(&module.object, "closure");
    assert_eq!(
        status, 0,
        "closure corpus object exited {status}:\n{stdout}"
    );

    let oracle = run_arc_oracle(&program, &types, entry);
    let expected = oracle.output.concat();
    assert_eq!(
        stdout, expected,
        "closure corpus: object stdout diverges from the arc oracle"
    );
    expect_success_markers(&stdout, "closure");
}
