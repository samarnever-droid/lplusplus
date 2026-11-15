use super::*;

const DATA_SURFACE_CORPUS: &str = r#"
def factorial(n: Int) -> Int:
    if n <= 1:
        return 1
    return n * factorial(n - 1)

def combine(a: Int, b: Int, c: Int) -> Int:
    return (a + b) * c - c

def check_int():
    if (9223372036854775806 + 1) != 9223372036854775807:
        print_str("fail_int_add")
    if (7 - 10) != -3:
        print_str("fail_int_sub")
    if (6 * 7) != 42:
        print_str("fail_int_mul")
    if (17 / 5) != 3:
        print_str("fail_int_div")
    if (-17 / 5) != -3:
        print_str("fail_int_sdiv")
    if (17 % 5) != 2:
        print_str("fail_int_rem")
    if (-17 % 5) != -2:
        print_str("fail_int_srem")
    if (9223372036854775807 + 1) >= 0:
        print_str("fail_int_wrap")
    if (255 & 160) != 160:
        print_str("fail_int_band")
    if (255 | 160) != 255:
        print_str("fail_int_bor")
    if (255 ^ 160) != 95:
        print_str("fail_int_bxor")
    if (1 << 10) != 1024:
        print_str("fail_int_shl")
    if (123 << 70) != 7872:
        print_str("fail_int_shl_wrap")
    if (123 << -2) != -4611686018427387904:
        print_str("fail_int_shl_neg")
    if ((1 << 63) >> 63) != -1:
        print_str("fail_int_shr_sign")
    if (-8 >> 1) != -4:
        print_str("fail_int_shr_neg")
    if (9223372036854775807 >> 62) != 1:
        print_str("fail_int_shr_max")
    if 3 < 7 == false:
        print_str("fail_int_lt")
    if 7 <= 7 == false:
        print_str("fail_int_le")
    if 7 > 3 == false:
        print_str("fail_int_gt")
    if 8 >= 7 == false:
        print_str("fail_int_ge")
    if 7 == 7 == false:
        print_str("fail_int_eq")
    if 7 != 8 == false:
        print_str("fail_int_ne")
    if -(5) != -5:
        print_str("fail_int_neg")

def check_bool_char():
    t := true
    f := false
    if (t && f) != false:
        print_str("fail_bool_and")
    if (t || f) == false:
        print_str("fail_bool_or")
    if !(t && f) == false:
        print_str("fail_bool_not")
    if (t == f) != false:
        print_str("fail_bool_eq")
    if (t != f) == false:
        print_str("fail_bool_ne")
    c := 'B'
    if c < 'C' == false:
        print_str("fail_char_lt")
    if c == 'A' != false:
        print_str("fail_char_ne")
    if 'a' > 'A' == false:
        print_str("fail_char_gt")

def check_float():
    if (1.5 + 2.25) != 3.75:
        print_str("fail_float_add")
    if (10.0 / 4.0) != 2.5:
        print_str("fail_float_div")
    if (7.5 % 2.0) != 1.5:
        print_str("fail_float_mod")
    if (-3.5 * 2.0) != -7.0:
        print_str("fail_float_mul")
    if (9.0 - 4.0) != 5.0:
        print_str("fail_float_sub")
    if 1.5 < 2.5 == false:
        print_str("fail_float_lt")
    if 2.5 <= 2.5 == false:
        print_str("fail_float_le")
    nan := 0.0 / 0.0
    if nan == nan:
        print_str("fail_float_nan_eq")
    if nan != nan == false:
        print_str("fail_float_nan_ne")

def check_cfg():
    mut i := 0
    mut total := 0
    while i < 10:
        total = total + i
        i = i + 1
    if total != 45:
        print_str("fail_cfg_sum")
    mut k := 0
    mut s := 0
    while k < 4:
        s = s + k * 2
        k = k + 1
    if s != 12:
        print_str("fail_cfg_k")
    x := 7
    mut acc := s
    if x > 5:
        if x > 6:
            acc = acc + 100
        else:
            acc = acc + 200
    else:
        acc = acc + 300
    if acc != 112:
        print_str("fail_cfg_nested_if")
    mut n := 0
    i = 0
    mut j := 0
    while i < 3:
        j = 0
        while j < 3:
            n = n + 1
            j = j + 1
        i = i + 1
    if n != 9:
        print_str("fail_cfg_nested_loop")

def check_calls():
    if factorial(5) != 120:
        print_str("fail_call_fact")
    if factorial(0) != 1:
        print_str("fail_call_fact0")
    if combine(3, 4, 5) != 30:
        print_str("fail_call_multi")
    if combine(10, 20, 30) != 870:
        print_str("fail_call_multi2")

def check_strings():
    s := "hello"
    if s == "hello" == false:
        print_str("fail_str_eq")
    if s == "world" != false:
        print_str("fail_str_ne")
    if "abc" == "abc" == false:
        print_str("fail_str_eq_lit")
    if "abc" == "abd" != false:
        print_str("fail_str_ne_lit")
    if str_len("hello") != 5:
        print_str("fail_str_len")
    if str_len("") != 0:
        print_str("fail_str_len_empty")

def check_print():
    print_str("str_ok")
    print_int(12345)
    print_int(-42)
    print_bool(true)
    print_bool(false)
    print('A')
    print(0)
    s := "printed"
    print(s)
    write_str("raw")
    print_int(7)

def main() -> Int:
    check_int()
    check_bool_char()
    check_float()
    check_cfg()
    check_calls()
    check_strings()
    check_print()
    print_str("all_ok")
    return 0
"#;

// ── 5D2a: managed data surface corpus ──────────────────────────────────────
//
// The 5C aggregate corpus (structs, enums, lists, cross-function passing)
// plus the byte-exact `{:.6}` float-print surface, differential-tested
// against the Phase 4 oracle.

const MANAGED_DATA_SURFACE_CORPUS: &str = r#"
struct Point:
    x: Int
    y: Int
struct Box:
    origin: Point
    size: Int
    tag: Char
    flag: Bool
struct Outer:
    inner: Point
    label: Str
    items: List[Point]
struct L1:
    next: L2
struct L2:
    deep: L3
    pad: Int
struct L3:
    value: Int
struct Link:
    value: Int
    next: Link
enum Shape:
    Square(size: Int)
    Circle(radius: Int)
    Blank
enum Wrap:
    Plain(n: Int)
    Fancy(point: Point)
    Empty
def area(kind: Shape) -> Int:
    mut answer := 0
    match kind:
        Square(size):
            answer = size * size
        Circle(radius):
            answer = radius * 3
        Blank:
            answer = 0
    return answer
def wrap_value(w: Wrap) -> Int:
    mut answer := 0
    match w:
        Plain(n):
            answer = n
        Fancy(point):
            answer = point.x + point.y
        Empty:
            answer = 0
    return answer
def check_shapes() -> Int:
    mut total := area(Shape.Square(3))
    total = total + area(Shape.Circle(2))
    total = total + area(Shape.Blank)
    probe := Shape.Square(7)
    total = total + area(probe)
    return total
def check_structs() -> Int:
    p := Point(3, 4)
    mut total := p.x + p.y
    q := p
    total = total + q.x
    mut r := Box(Point(1, 2), 5, 'k', true)
    total = total + r.origin.x
    total = total + r.origin.y
    total = total + r.size
    r.origin = Point(10, 20)
    total = total + r.origin.x
    total = total + r.origin.y
    r.size = 6
    r.tag = 'm'
    if r.tag == 'm':
        total = total + 1
    if r.flag:
        total = total + 1
    if not r.flag:
        total = total + 1000
    return total
def check_managed_struct() -> Int:
    mut s := Outer(Point(1, 2), "hello", [Point(3, 4), Point(5, 6)])
    mut total := s.inner.x
    total = total + s.items[0].y
    total = total + s.items[1].x
    s.inner = Point(9, 9)
    total = total + s.inner.x
    s.items[0] = Point(7, 8)
    total = total + s.items[0].x
    mut t := s
    total = total + t.inner.y
    t.label = "world"
    total = total + s.inner.x
    return total
def check_deep() -> Int:
    mut d := L1(L2(L3(41), 2))
    mut total := d.next.deep.value
    total = total + d.next.pad
    d.next.pad = 5
    total = total + d.next.pad
    return total
def check_recursive_null() -> Int:
    tail := Link(2, 0)
    head := Link(1, tail)
    return head.value + head.next.value
def check_lists() -> Int:
    mut xs := [10, 20, 30]
    mut total := xs[0] + xs[1]
    total = total + xs[2]
    xs[1] = 25
    total = total + xs[1]
    mut sum := 0
    for v in xs:
        sum = sum + v
    total = total + sum
    ys := [4, 5]
    total = total + ys[0] + ys[1]
    return total
def check_list_float() -> Float:
    mut xs := [1.5, 2.25]
    mut total := xs[0]
    total = total + xs[1]
    xs[0] = 4.5
    total = total + xs[0]
    return total
def check_list_char() -> Int:
    xs := ['a', 'b']
    mut total := 0
    if xs[0] == 'a':
        total = total + 1
    if xs[1] == 'b':
        total = total + 1
    return total
def check_list_bool() -> Int:
    mut xs := [true, false]
    mut total := 0
    if xs[0]:
        total = total + 1
    if not xs[1]:
        total = total + 1
    xs[1] = true
    if xs[1]:
        total = total + 1
    return total
def check_nested() -> Int:
    xs := [[1, 2], [3, 4]]
    mut total := xs[0][0]
    total = total + xs[0][1]
    total = total + xs[1][0]
    total = total + xs[1][1]
    return total
def check_list_struct() -> Int:
    mut xs := [Point(1, 1), Point(2, 2)]
    mut total := xs[0].x
    total = total + xs[0].y
    total = total + xs[1].x
    xs[1] = Point(5, 6)
    total = total + xs[1].y
    return total
def describe(p: Point, xs: List[Int], kind: Shape) -> Int:
    mut total := p.x + p.y
    total = total + xs[0]
    total = total + area(kind)
    return total
def check_calls() -> Int:
    mut total := describe(Point(3, 4), [7, 8], Shape.Square(2))
    total = total + describe(Point(1, 1), [100], Shape.Blank)
    return total
def check_wrap() -> Int:
    mut total := wrap_value(Wrap.Plain(7))
    total = total + wrap_value(Wrap.Fancy(Point(2, 3)))
    total = total + wrap_value(Wrap.Empty)
    held := Wrap.Fancy(Point(4, 5))
    total = total + wrap_value(held)
    return total
def check_float_print():
    print(1.5)
    print(-8.25)
    print(0.1)
    print(8.0 / 3.0)
    print(1000000000000000.0)
    print(-0.0)
    print_float(2.05)
    print_float(-0.0)
    print_float(8.0 / 3.0)

def main() -> Int:
    if check_shapes() == 64:
        print_str("ok_shapes")
    else:
        print_str("fail_shapes")
    if check_structs() == 50:
        print_str("ok_structs")
    else:
        print_str("fail_structs")
    if check_managed_struct() == 44:
        print_str("ok_managed_struct")
    else:
        print_str("fail_managed_struct")
    if check_deep() == 48:
        print_str("ok_deep")
    else:
        print_str("fail_deep")
    if check_recursive_null() == 3:
        print_str("ok_recursive_null")
    else:
        print_str("fail_recursive_null")
    if check_lists() == 159:
        print_str("ok_lists")
    else:
        print_str("fail_lists")
    if check_list_float() == 8.25:
        print_str("ok_list_float")
    else:
        print_str("fail_list_float")
    if check_list_char() == 2:
        print_str("ok_list_char")
    else:
        print_str("fail_list_char")
    if check_list_bool() == 3:
        print_str("ok_list_bool")
    else:
        print_str("fail_list_bool")
    if check_nested() == 10:
        print_str("ok_nested")
    else:
        print_str("fail_nested")
    if check_list_struct() == 10:
        print_str("ok_list_struct")
    else:
        print_str("fail_list_struct")
    if check_calls() == 120:
        print_str("ok_calls")
    else:
        print_str("fail_calls")
    if check_wrap() == 21:
        print_str("ok_wrap")
    else:
        print_str("fail_wrap")
    check_float_print()
    print_str("all_ok")
    return 0
"#;

// ── 5D2a: ARC stress corpus ────────────────────────────────────────────────
//
// The 5C ARC stress programs: aliasing, self-assignment, swaps,
// move-then-reassign, cross-container stores, deep nesting.

const ARC_STRESS_CORPUS: &str = r#"
struct Point:
    x: Int
    y: Int
struct Box:
    origin: Point
    size: Int
    tag: Char
    flag: Bool
struct Holder:
    value: Int
    items: List[Holder]
struct L1:
    next: L2
struct L2:
    deep: L3
    pad: Int
struct L3:
    value: Int
def check_aliasing() -> Int:
    p := Point(5, 5)
    mut total := 0
    q := p
    r := q
    total = total + p.x
    total = total + q.x
    total = total + r.y
    s := Box(p, 1, 'a', false)
    total = total + s.origin.x
    return total
def check_self_assign() -> Int:
    mut p := Point(3, 7)
    mut s := Box(Point(1, 1), 1, 'a', false)
    mut total := 0
    total = total + s.origin.x
    s.origin = s.origin
    total = total + s.origin.y
    q := p
    p = q
    total = total + p.x
    return total
def check_swap() -> Int:
    mut a := Point(1, 10)
    mut b := Point(2, 20)
    mut tmp := a
    a = b
    b = tmp
    mut total := a.x
    total = total + b.x
    total = total + a.y
    total = total + b.y
    return total
def check_move_reassign() -> Int:
    mut p := Point(1, 1)
    mut holder := Box(p, 9, 'z', true)
    mut total := 0
    total = total + holder.origin.x
    holder.origin = Point(4, 5)
    total = total + holder.origin.x
    total = total + holder.origin.y
    p = Point(8, 8)
    total = total + p.x
    return total
def check_xstore() -> Int:
    mut a := Box(Point(1, 10), 1, 'a', false)
    mut b := Box(Point(2, 20), 2, 'b', true)
    mut total := 0
    a.origin = b.origin
    total = total + a.origin.x
    total = total + b.origin.y
    total = total + a.origin.y
    total = total + b.origin.x
    return total
def check_deep_nest() -> Int:
    d := L1(L2(L3(41), 2))
    mut e := L1(L2(L3(100), 3))
    mut total := d.next.deep.value
    total = total + e.next.deep.value
    total = total + d.next.pad + e.next.pad
    return total
def main() -> Int:
    if check_aliasing() == 20:
        print_str("ok_aliasing")
    else:
        print_str("fail_aliasing")
    if check_self_assign() == 5:
        print_str("ok_self_assign")
    else:
        print_str("fail_self_assign")
    if check_swap() == 33:
        print_str("ok_swap")
    else:
        print_str("fail_swap")
    if check_move_reassign() == 18:
        print_str("ok_move_reassign")
    else:
        print_str("fail_move_reassign")
    if check_xstore() == 44:
        print_str("ok_xstore")
    else:
        print_str("fail_xstore")
    if check_deep_nest() == 146:
        print_str("ok_deep_nest")
    else:
        print_str("fail_deep_nest")
    print_str("all_ok")
    return 0
"#;

// ── wasm binary inspection (census) ────────────────────────────────────────

/// Unsigned LEB128 decode.
fn uleb(bytes: &[u8], at: &mut usize) -> u64 {
    let mut result: u64 = 0;
    let mut shift = 0u32;
    loop {
        let byte = bytes[*at];
        *at += 1;
        result |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    result
}

/// Parse the custom "name" section (function names) and the funcref table
/// size from the raw wasm object.
fn parse_names_and_table(object: &[u8]) -> (BTreeMap<String, u32>, Option<u32>, Option<Vec<u32>>) {
    assert_eq!(&object[0..4], b"\0asm", "not a wasm module");
    let mut at = 8usize; // magic + version
    let mut names: BTreeMap<String, u32> = BTreeMap::new();
    let mut table_min: Option<u32> = None;
    let mut table_elems: Option<Vec<u32>> = None;
    while at < object.len() {
        let id = object[at];
        at += 1;
        let len = uleb(object, &mut at) as usize;
        let payload = &object[at..at + len];
        at += len;
        if id == 0 {
            // Custom section: "name" + payload.
            let mut p = 0usize;
            let name_len = uleb(payload, &mut p) as usize;
            let section_name = &payload[p..p + name_len];
            p += name_len;
            if section_name == b"name" {
                while p < payload.len() {
                    let sub = payload[p];
                    p += 1;
                    let sub_len = uleb(payload, &mut p) as usize;
                    let sub_end = p + sub_len;
                    if sub == 1 {
                        // Function names.
                        let mut q = p;
                        let count = uleb(payload, &mut q) as usize;
                        for _ in 0..count {
                            let func_idx = uleb(payload, &mut q) as u32;
                            let nlen = uleb(payload, &mut q) as usize;
                            let n = String::from_utf8(payload[q..q + nlen].to_vec()).unwrap();
                            q += nlen;
                            names.insert(n, func_idx);
                        }
                    }
                    p = sub_end;
                }
            }
        } else if id == 4 {
            // Table section: one funcref table, min size.
            let mut p = 0usize;
            let count = uleb(payload, &mut p) as usize;
            assert_eq!(count, 1, "exactly one table");
            let elem_type = payload[p];
            p += 1;
            assert_eq!(elem_type, 0x70, "funcref table");
            let flags = payload[p];
            p += 1;
            assert_eq!(flags, 0x00, "min-only table limits");
            table_min = Some(uleb(payload, &mut p) as u32);
        } else if id == 9 {
            // Element section: the table initialization.
            let mut p = 0usize;
            let count = uleb(payload, &mut p) as usize;
            assert_eq!(count, 1, "exactly one element segment");
            let mode = payload[p];
            p += 1;
            assert_eq!(mode, 0x00, "active segment, table 0");
            // Init expr: i32.const 0; end
            let mut q = p;
            assert_eq!(payload[q], 0x41);
            q += 1;
            let mut _s = 0i64;
            let mut byte = payload[q] as i64;
            q += 1;
            _s = (byte & 0x7f) as i64;
            while payload[q - 1] & 0x80 != 0 {
                byte = payload[q] as i64;
                _s |= (byte & 0x7f) << 7;
                q += 1;
            }
            assert_eq!(payload[q], 0x0b, "init expr end");
            q += 1;
            let n = uleb(payload, &mut q) as usize;
            let mut elems = Vec::with_capacity(n);
            for _ in 0..n {
                elems.push(uleb(payload, &mut q) as u32);
            }
            table_elems = Some(elems);
        }
    }
    (names, table_min, table_elems)
}

// ── tests ──────────────────────────────────────────────────────────────────

#[test]
fn data_surface_matches_oracle() {
    let (program, types, package) = pipeline(DATA_SURFACE_CORPUS);
    let names = Names(&package.names.symbols);
    let entry = main_function(&program, &package.names.symbols);

    let oracle = run_oracle(&program, &types, entry);
    let expected = oracle.output.concat();

    let module = compile(&program, &types, &names);
    assert_eq!(module.target, Target::Wasm32Wasi);
    let (stdout, status) = run_wasm(&module.object, "data_surface");
    assert_eq!(status, 0, "wasm exited non-zero:\n{stdout}");
    expect_success_markers(&stdout, "data_surface");
    assert_eq!(
        stdout, expected,
        "object stdout diverged from the reference oracle"
    );
}

#[test]
fn deterministic_object() {
    for (label, corpus) in [
        ("data_surface", DATA_SURFACE_CORPUS),
        ("managed", MANAGED_DATA_SURFACE_CORPUS),
        ("arc_stress", ARC_STRESS_CORPUS),
    ] {
        let (program, types, package) = pipeline(corpus);
        let names = Names(&package.names.symbols);
        let a = compile(&program, &types, &names);
        let b = compile(&program, &types, &names);
        assert_eq!(
            a.object, b.object,
            "{label}: two compiles differ byte-for-byte"
        );
        assert_eq!(
            a.exported_symbols, b.exported_symbols,
            "{label}: export censuses differ"
        );
        assert_eq!(
            a.imported_symbols, b.imported_symbols,
            "{label}: import censuses differ"
        );
    }
}

// ── 5D2a gate 1: differential managed corpus ───────────────────────────────

#[test]
fn managed_corpus_matches_the_reference_oracle() {
    let (program, types, package) = pipeline(MANAGED_DATA_SURFACE_CORPUS);
    let names = Names(&package.names.symbols);
    let entry = main_function(&program, &package.names.symbols);

    let module = compile(&program, &types, &names);
    let (stdout, status) = run_wasm(&module.object, "managed");
    assert_eq!(
        status, 0,
        "managed corpus object exited {status}:\n{stdout}"
    );

    let oracle = run_oracle(&program, &types, entry);
    let expected = oracle.output.concat();
    assert_eq!(
        stdout, expected,
        "managed corpus: object stdout diverges from the oracle"
    );
    expect_success_markers(&stdout, "managed");
}

// ── 5D2a gate 2: differential ARC stress corpus ────────────────────────────

#[test]
fn arc_stress_corpus_matches_the_arc_oracle() {
    let (program, types, package) = pipeline(ARC_STRESS_CORPUS);
    let names = Names(&package.names.symbols);
    let entry = main_function(&program, &package.names.symbols);

    let module = compile(&program, &types, &names);
    let (stdout, status) = run_wasm(&module.object, "arc_stress");
    assert_eq!(status, 0, "arc stress object exited {status}:\n{stdout}");

    let oracle = run_arc_oracle(&program, &types, entry);
    let expected = oracle.output.concat();
    assert_eq!(
        stdout, expected,
        "arc stress corpus: object stdout diverges from the arc oracle"
    );
    expect_success_markers(&stdout, "arc stress");
}

// ── 5D2a gate 3: structure census ──────────────────────────────────────────

#[test]
fn object_census_matches_the_contract() {
    let (program, types, package) = pipeline(MANAGED_DATA_SURFACE_CORPUS);
    let names = Names(&package.names.symbols);
    let module = compile(&program, &types, &names);

    // Exports: _start, memory, and the user functions — nothing else.
    let mut expected_exports = BTreeSet::from(["_start".to_string(), "memory".to_string()]);
    for (_, function) in program.functions() {
        if let Some(symbol) = function.name
            && let Some(name) = package.names.symbols.resolve(symbol)
        {
            expected_exports.insert(name.to_string());
        }
    }
    assert_eq!(
        module.exported_symbols, expected_exports,
        "export census diverged"
    );
    assert_eq!(
        module.imported_symbols,
        BTreeSet::from([
            "wasi_snapshot_preview1.fd_write".to_string(),
            "wasi_snapshot_preview1.proc_exit".to_string(),
        ]),
        "import census diverged"
    );

    // The name section: one destructor per nominal, in MirAggregateId
    // order, plus the list destroyer and the no-op.
    let (name_section, table_min, table_elems) = parse_names_and_table(&module.object);
    let aggregates: Vec<MirAggregateId> = program.aggregates().map(|(id, _)| id).collect();
    let kinds: Vec<_> = aggregates
        .iter()
        .map(|&id| program.aggregate(id).unwrap().kind)
        .collect();
    let expected_drops: BTreeSet<String> = aggregates
        .iter()
        .enumerate()
        .map(|(pos, &id)| {
            let tag = match kinds[pos] {
                lpp_mir::MirAggregateKind::Struct => 's',
                lpp_mir::MirAggregateKind::Enum => 'e',
            };
            format!("lpp_drop_{tag}{}", id.raw())
        })
        .chain(std::iter::once("lpp_drop_list".to_string()))
        .chain(std::iter::once("lpp_drop_none".to_string()))
        .collect();
    let drop_names_in_section: BTreeSet<String> = name_section
        .keys()
        .filter(|name| name.starts_with("lpp_drop_"))
        .cloned()
        .collect();
    assert_eq!(
        drop_names_in_section, expected_drops,
        "drop function names diverged"
    );

    // The table: [no-op, drops in MirAggregateId order, list].
    let expected_table_len = 1 + aggregates.len() as u32 + 1;
    assert_eq!(table_min, Some(expected_table_len), "funcref table size");
    let elems = table_elems.expect("element section initializes the table");
    assert_eq!(
        elems.len() as u32,
        expected_table_len,
        "table element count"
    );
    assert_eq!(
        name_section.get("lpp_drop_none"),
        Some(&elems[0]),
        "table slot 0 is the no-op"
    );
    for (pos, &id) in aggregates.iter().enumerate() {
        let tag = match kinds[pos] {
            lpp_mir::MirAggregateKind::Struct => 's',
            lpp_mir::MirAggregateKind::Enum => 'e',
        };
        let name = format!("lpp_drop_{tag}{}", id.raw());
        assert_eq!(
            name_section.get(&name),
            Some(&elems[pos + 1]),
            "drop {name} is not tabled in MirAggregateId order"
        );
    }
    // Table = [no-op, drop0..drop_{n-1}, list]: the list destroyer
    // occupies slot 1 + n.
    assert_eq!(
        name_section.get("lpp_drop_list"),
        Some(&elems[aggregates.len() + 1]),
        "list destroyer is the last table entry"
    );
}

// ── 5D2b slice 1: closure corpus (closures + capsule calls) ────────────────
//
// The synchronous subset of the 5C2 closure corpus: value captures
// (cells for mutables), capsule copies called through the dispatch
// table, closures stored in lists, and two-parameter closure calls.
// Expected marker values (hand-computed, identical to the 5C2 gate):
//   counter 5, list_closure 42, nested 35, managed_capture 19,
//   sync_value 42, cycle 108, managed_read_only 13, minimal 3,
//   call_twice 6, no_closure 3.
//
// Corpus-domain rule (from the 5C2 contract): the cycle closure's
// function type (two parameters) is shared by no other closure in
// the program, so the 4D-pinned type cannot pin an unrelated capsule.
