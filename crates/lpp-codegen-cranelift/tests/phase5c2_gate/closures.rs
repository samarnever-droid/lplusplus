use super::*;

pub(super) const CLOSURE_TASK_CORPUS: &str = r#"
struct Point:
    x: Int
    y: Int

async def first() -> Str:
    return "ready"

async def second() -> Str:
    value := first().await
    return value

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

def async_value_check() -> Int:
    av := first
    t := av()
    return str_len(t.await)

def cycle_check() -> Int:
    mut total := 0
    xs := list_new()
    f := fn(seed: Int, extra: Int) -> Int:
        total = total + list_len(xs)
        return total + seed + extra
    g := f
    list_push(xs, f)
    return g(100, 7)

def task_chain_check() -> Int:
    t := first()
    a := t.await
    b := t.await
    return str_len(a) + str_len(b)

def spawn_check() -> Int:
    xs := list_new()
    bump := fn():
        list_push(xs, 1)
    spawn bump
    return list_len(xs)

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

async def main():
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
    if async_value_check() == 5:
        print_str("ok_async_value")
    else:
        print_str("fail_async_value")
    if cycle_check() == 108:
        print_str("ok_cycle")
    else:
        print_str("fail_cycle")
    if task_chain_check() == 10:
        print_str("ok_task_chain")
    else:
        print_str("fail_task_chain")
    if spawn_check() == 1:
        print_str("ok_spawn")
    else:
        print_str("fail_spawn")
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
    chain := second().await
    if str_len(chain) == 5:
        print_str("ok_async_chain")
    else:
        print_str("fail_async_chain")
    print_str("all_ok")
"#;

// ── corpus 2: builtin families (Family A) ─────────────────────────────────

#[test]
#[cfg_attr(
    target_os = "windows",
    ignore = "MSVC runtime setup is covered by the Windows driver smoke gate"
)]
fn closure_and_task_corpus_matches_the_arc_oracle() {
    let (program, types, package) = pipeline(CLOSURE_TASK_CORPUS);
    let names = Names(&package.names.symbols);

    // The cycle closure's captures must pin a type: the 4D plan's
    // pinned set is non-empty for this corpus.
    let plan = compute_ownership_plan(&program, &types)
        .unwrap_or_else(|e| panic!("ownership plan: {e:?}"));
    assert!(
        !plan.pinned_types().is_empty(),
        "the cycle corpus pins no type — the corpus rule is broken"
    );

    let module = compile(&program, &types, &names);
    let (stdout, status) = link_and_run(&module.object, "closure_task");
    assert_eq!(status, 0, "closure/task object exited {status}:\n{stdout}");

    let entry = main_function(&program, &package.names.symbols);
    let oracle = run_arc_oracle(&program, &types, entry);
    let expected: String = oracle.output.concat();
    assert_eq!(
        stdout, expected,
        "closure/task corpus: object stdout diverges from the arc oracle"
    );
    expect_success_markers(&stdout, "closure_task");
}

// ── gate 2: differential builtin families ─────────────────────────────────
