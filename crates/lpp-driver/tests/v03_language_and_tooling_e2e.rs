use std::io::{ErrorKind, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use lpp_common::OptimizationLevel;
use lpp_driver::fmt::{format_source, run_fmt};
use lpp_driver::frontend_eval::check_in_memory;
use lpp_driver::test_runner::run_tests_cli;
use lpp_driver::{
    BackendChoice, CompileOptions, build_executable_configured, runtime_library_path,
};
use lpp_hir::Symbol;
use lpp_mir::{InterpreterLimits, execute_mir_with_stats};

fn workdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lpp_v03_e2e_{}_{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_on_backend(
    name: &str,
    source: &str,
    backend: BackendChoice,
    opt: OptimizationLevel,
) -> String {
    let tag = match (backend, opt) {
        (BackendChoice::Cranelift, _) => "cranelift",
        (BackendChoice::Llvm, OptimizationLevel::O3) => "llvm_o3",
        (BackendChoice::Llvm, _) => "llvm_o0",
        (BackendChoice::Wasm, _) => "wasm",
    };
    let dir = workdir(&format!("{name}_{tag}"));
    let entry = dir.join("main.lpp");
    let exe = dir.join(format!("main{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&entry, source).unwrap();

    let runtime = runtime_library_path().expect("runtime library built");
    let mut opts = CompileOptions::for_backend(backend);
    opts.optimization = opt;
    build_executable_configured(
        &entry,
        "v03_e2e",
        backend,
        opts,
        &exe,
        runtime.parent().unwrap(),
        Some("cc"),
    )
    .unwrap_or_else(|e| panic!("build failed ({tag}) for {name}: {e:?}"));

    let out = lpp_driver::exec_output_retry(&exe).unwrap();
    assert!(
        out.status.success(),
        "run failed ({tag}) for {name}: status={:?}, stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn run_on_mir(name: &str, source: &str) -> String {
    let dir = workdir(&format!("{name}_mir"));
    let entry = dir.join("main.lpp");
    let checked = check_in_memory(source, &entry)
        .unwrap_or_else(|diags| panic!("MIR check failed for {name}: {diags:?}"));
    let main_id = checked
        .mir
        .functions()
        .find_map(|(id, f)| {
            (f.name.and_then(|s| {
                checked
                    .package
                    .names
                    .symbols
                    .resolve(Symbol::from_raw(s.raw()))
            }) == Some("main"))
            .then_some(id)
        })
        .expect("main present");
    let outcome = execute_mir_with_stats(
        &checked.mir,
        &checked.inference.interner,
        main_id,
        &[],
        InterpreterLimits::default(),
    )
    .unwrap_or_else(|e| panic!("MIR exec failed for {name}: {e}"));
    outcome.output.concat()
}

fn assert_all_engines_parity(name: &str, source: &str, expected: &str) {
    let mir_out = run_on_mir(name, source);
    let cranelift_out = run_on_backend(
        name,
        source,
        BackendChoice::Cranelift,
        OptimizationLevel::O0,
    );
    let llvm_o0_out = run_on_backend(name, source, BackendChoice::Llvm, OptimizationLevel::O0);
    let llvm_o3_out = run_on_backend(name, source, BackendChoice::Llvm, OptimizationLevel::O3);
    assert_eq!(mir_out, expected, "{name}: MIR output mismatch");
    assert_eq!(cranelift_out, expected, "{name}: Cranelift output mismatch");
    assert_eq!(llvm_o0_out, expected, "{name}: LLVM -O0 output mismatch");
    assert_eq!(llvm_o3_out, expected, "{name}: LLVM -O3 output mismatch");
}

#[test]
fn tuples_16_fstrings_ranges_and_enhanced_match_parity() {
    let source = r#"
struct Boxed:
    val: Int

enum Msg:
    Ping(n: Int)
    Pair(a: Int, b: Int)
    Quit

def helper(x: Int) -> Int:
    return x * 3 + 1

def classify(n: Int) -> Str:
    match n:
        -1 | -2:
            return "neg_small"
        0..5:
            return "low"
        5..=10 if n % 2 == 0:
            return "even_mid"
        5..=10:
            return "odd_mid"
        v if v > 100:
            return "huge"
        _:
            return "other"

def classify_msg(m: Msg) -> Int:
    match m:
        Msg.Ping(n) if n > 10:
            return n * 10
        Msg.Ping(n):
            return n
        Msg.Pair(a, b) if a == b:
            return a + b + 100
        Msg.Pair(a, b):
            return a + b
        Msg.Quit:
            return -99

def main():
    t16 := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16)
    print(t16.0 + t16.7 + t16.15)

    bx := Boxed(7)
    tag := "hi"
    msg := f"sum={10 + 20},bx={bx.val},fn={helper(4)},s={tag}"
    print_str(msg)

    mut excl := 0
    for i in 0..5:
        excl = excl + i
    mut incl := 0
    for j in 1..=4:
        incl = incl + j
    print(excl)
    print(incl)

    print_str(classify(-2))
    print_str(classify(3))
    print_str(classify(8))
    print_str(classify(7))
    print_str(classify(200))
    print_str(classify(42))

    print(classify_msg(Msg.Ping(12)))
    print(classify_msg(Msg.Ping(4)))
    print(classify_msg(Msg.Pair(5, 5)))
    print(classify_msg(Msg.Pair(3, 4)))
    print(classify_msg(Msg.Quit))

    mut kind := 0
    match 'k':
        'a'..='z':
            kind = 1
        _:
            kind = 2
    print(kind)
"#;

    let expected = "\
25
sum=30,bx=7,fn=13,s=hi
10
10
neg_small
low
even_mid
odd_mid
huge
other
120
4
110
7
-99
1
";
    assert_all_engines_parity("tuples_fstrings_ranges_match", source, expected);
}

#[test]
fn tuple_17_is_rejected() {
    let source = r#"
def main():
    bad := (1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17)
    print(bad.0)
"#;
    let err = check_in_memory(source, std::path::Path::new("/tmp/bad17.lpp"))
        .err()
        .expect("17-tuple should fail type/MIR/HIR lowering");
    assert!(!err.is_empty());
}

#[test]
fn set_deque_and_stdlib_simd_sync_parity() {
    let source = r#"
from stdlib.collections import IntSet, IntDeque
from stdlib.simd import VectorI64x4, vec_i64x4, vec_i64x4_splat
from stdlib.sync import AtomicInt, atomic_int_new, SyncQueue, sync_queue_new
from stdlib.math import lcm

def main():
    s1 := int_set_new()
    s1.add(10)
    s1.add(20)
    s1.add(10)
    s1.add_str("alpha")
    s1.add_str("beta")
    print(s1.len())
    print_bool(s1.has(20))
    print_bool(s1.has_str("alpha"))
    s1.remove(10)
    print_bool(s1.has(10))

    s2 := set_new()
    set_add(s2, 20)
    set_add(s2, 30)
    su := set_union(s1.handle, s2)
    si := set_intersect(s1.handle, s2)
    print(set_len(su))
    print(set_len(si))
    set_free(s2)
    set_free(su)
    set_free(si)
    s1.free()

    dq := int_deque_new()
    dq.push_back(2)
    dq.push_front(1)
    dq.push_back(3)
    print(dq.len())
    print(dq.peek_front())
    print(dq.get(1))
    print(dq.peek_back())
    print(dq.pop_front())
    print(dq.pop_back())
    print(dq.len())
    dq.free()

    va := vec_i64x4(1, 2, 3, 4)
    vb := vec_i64x4_splat(5)
    vc := va.add(vb)
    print(vc.sum())
    print(va.dot(vb))
    print(vc.lane(2))

    print(lcm(12, 18))

    mut out := 0
    atom := atomic_int_new(10)
    out = atom.fetch_add(5)
    print(out)
    print(atom.load())
    atom.free()

    q := sync_queue_new()
    q.push(77)
    q.push(88)
    print(q.pop_or(-1))
    print(q.pop_or(-1))
    print(q.pop_or(-1))
    q.free()
"#;

    let expected = "\
4
1
1
0
4
1
3
1
2
3
1
3
1
30
50
8
36
10
15
77
88
-1
";
    // Test Set + Deque + math + simd across all 4 engines, and Atomics + SyncQueue on native Cranelift + LLVM!
    let cranelift_out = run_on_backend(
        "collections_sync_simd",
        source,
        BackendChoice::Cranelift,
        OptimizationLevel::O0,
    );
    let llvm_o0_out = run_on_backend(
        "collections_sync_simd",
        source,
        BackendChoice::Llvm,
        OptimizationLevel::O0,
    );
    let llvm_o3_out = run_on_backend(
        "collections_sync_simd",
        source,
        BackendChoice::Llvm,
        OptimizationLevel::O3,
    );
    assert_eq!(cranelift_out, expected);
    assert_eq!(llvm_o0_out, expected);
    assert_eq!(llvm_o3_out, expected);

    // Pure Set + Deque + lcm MIR parity (without host atomic intrinsics)
    let mir_src = r#"
from stdlib.collections import IntSet, IntDeque
from stdlib.math import lcm

def main():
    s := int_set_new()
    s.add(5)
    s.add_str("k")
    s.add_str("k")
    print(s.len())
    print_bool(s.has_str("k"))
    dq := int_deque_new()
    dq.push_back(9)
    dq.push_front(4)
    print(dq.pop_front())
    print(dq.pop_back())
    print(lcm(6, 15))
    s.free()
    dq.free()
"#;
    assert_eq!(run_on_mir("collections_mir", mir_src), "2\n1\n4\n9\n30\n");
}

#[test]
fn fmt_test_runner_repl_and_lsp_roundtrip() {
    let dir = workdir("tooling");

    // 1. `lpp fmt` idempotency + check + write
    let unformatted = "def main():   \n\tif true:\n\t\tx:=1   \n\telse:\n\t\tx:=2\n\n\n\n";
    let formatted_once = format_source(unformatted);
    let formatted_twice = format_source(&formatted_once);
    assert_eq!(
        formatted_once, formatted_twice,
        "formatter must be idempotent"
    );
    assert!(formatted_once.contains("    if true:\n        x:=1\n"));

    let fmt_file = dir.join("dirty.lpp");
    std::fs::write(&fmt_file, unformatted).unwrap();
    assert_eq!(
        run_fmt(&["--check".into(), fmt_file.display().to_string()], &dir),
        1
    );
    assert_eq!(
        run_fmt(&["--write".into(), fmt_file.display().to_string()], &dir),
        0
    );
    assert_eq!(
        run_fmt(&["--check".into(), fmt_file.display().to_string()], &dir),
        0
    );

    // 2. `lpp test` runner (`@test` and `test_*`, native + `--mir`, `--filter`)
    let test_file = dir.join("unit_suite.lpp");
    std::fs::write(
        &test_file,
        r#"
@test
def check_math() -> Bool:
    return 2 + 2 == 4

def test_fstring():
    s := f"val={6 * 7}"
    if !str_contains(s, "42"):
        exit(1)
"#,
    )
    .unwrap();
    assert_eq!(run_tests_cli(&[test_file.display().to_string()], &dir), 0);
    assert_eq!(
        run_tests_cli(
            &[
                "--mir".into(),
                "--filter".into(),
                "check_math".into(),
                test_file.display().to_string(),
            ],
            &dir
        ),
        0
    );

    // 3. `lpp repl` piped session via `lpp` binary (if built alongside target/debug)
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let lpp_bin = repo_root
        .join("target/debug")
        .join(format!("lpp{}", std::env::consts::EXE_SUFFIX));
    let lsp_bin = repo_root
        .join("target/debug")
        .join(format!("lpp-lsp{}", std::env::consts::EXE_SUFFIX));

    if lpp_bin.exists() {
        let mut child = Command::new(&lpp_bin)
            .arg("repl")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        {
            let stdin = child.stdin.as_mut().unwrap();
            stdin
                .write_all(b"x := 20 + 22\nx\nf\"ans={x}\"\n:quit\n")
                .unwrap();
        }
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "repl stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("42"), "repl output missing 42: {stdout}");
        assert!(
            stdout.contains("ans=42"),
            "repl output missing ans=42: {stdout}"
        );
    }

    if lsp_bin.exists() {
        let mut child = Command::new(&lsp_bin)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let send = |w: &mut std::process::ChildStdin, payload: &str| {
            let hdr = format!("Content-Length: {}\r\n\r\n", payload.len());
            w.write_all(hdr.as_bytes()).unwrap();
            w.write_all(payload.as_bytes()).unwrap();
            w.flush().unwrap();
        };
        send(
            &mut stdin,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        );
        send(
            &mut stdin,
            r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///tmp/lsp_test.lpp","text":"def main():\n    x: Int = \"oops\"\n"}}}"#,
        );
        send(
            &mut stdin,
            r#"{"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":"file:///tmp/lsp_test.lpp"},"contentChanges":[{"text":"def add(a: Int, b: Int) -> Int:\n    return a + b\ndef main():\n    print(add(1, 2))\n"}]}}"#,
        );
        send(
            &mut stdin,
            r#"{"jsonrpc":"2.0","id":2,"method":"textDocument/documentSymbol","params":{"textDocument":{"uri":"file:///tmp/lsp_test.lpp"}}}"#,
        );
        send(
            &mut stdin,
            r#"{"jsonrpc":"2.0","id":3,"method":"textDocument/formatting","params":{"textDocument":{"uri":"file:///tmp/lsp_test.lpp"}}}"#,
        );
        send(
            &mut stdin,
            r#"{"jsonrpc":"2.0","id":4,"method":"shutdown"}"#,
        );
        send(&mut stdin, r#"{"jsonrpc":"2.0","method":"exit"}"#);
        drop(stdin);

        let mut stdout_bytes = Vec::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_end(&mut stdout_bytes)
            .unwrap();
        let status = child.wait().unwrap();
        assert!(status.success());
        let text = String::from_utf8_lossy(&stdout_bytes);
        assert!(
            text.contains("\"code\":\"E3100\""),
            "expected semantic type error diagnostic from LSP: {text}"
        );
        assert!(
            text.contains("\"name\":\"add\"") && text.contains("\"name\":\"main\""),
            "expected documentSymbol results: {text}"
        );
        assert!(
            text.contains("\"newText\""),
            "expected formatting edit: {text}"
        );
    }
}

#[cfg(unix)]
#[test]
fn dwarf_debug_info_and_debugger_cli() {
    let dir = workdir("dwarf_debug_info");
    let src = dir.join("main.lpp");
    std::fs::write(
        &src,
        r#"def add_values(a: Int, b: Int) -> Int:
    sum := a + b
    return sum

def main():
    res := add_values(19, 23)
    print(res)
"#,
    )
    .expect("write source");

    let runtime = runtime_library_path().expect("runtime library built");
    let rt_dir = runtime.parent().expect("runtime parent dir");

    for (tag, backend) in [
        ("cranelift", BackendChoice::Cranelift),
        ("llvm", BackendChoice::Llvm),
    ] {
        let exe = dir.join(format!("app_{tag}"));
        let mut opts = CompileOptions::for_backend(backend);
        opts.debug_info = true;
        build_executable_configured(&src, "pkg", backend, opts, &exe, rt_dir, Some("cc"))
            .unwrap_or_else(|e| panic!("build {tag} -g failed: {e}"));

        let run_out = lpp_driver::exec_output_retry(&exe).expect("run debug binary");
        assert!(run_out.status.success(), "{tag} binary failed");
        assert_eq!(String::from_utf8_lossy(&run_out.stdout).trim(), "42");

        match Command::new("readelf")
            .args(["--debug-dump=info,line"])
            .arg(&exe)
            .output()
        {
            Ok(readelf) => {
                assert!(readelf.status.success(), "readelf failed for {tag}");
                let dump = String::from_utf8_lossy(&readelf.stdout);
                assert!(
                    dump.contains("main.lpp"),
                    "expected main.lpp in DWARF dump for {tag}: {dump}"
                );
                assert!(
                    dump.contains("add_values")
                        || dump.contains("lpp_add_values")
                        || dump.contains("main"),
                    "expected function DIEs in DWARF dump for {tag}: {dump}"
                );
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => panic!("failed running readelf for {tag}: {e}"),
        }

        match Command::new("gdb")
            .args([
                "-batch",
                "-ex",
                "set pagination off",
                "-ex",
                "b main.lpp:6",
                "-ex",
                "run",
                "-ex",
                "bt",
                "-ex",
                "continue",
            ])
            .arg(&exe)
            .output()
        {
            Ok(gdb_out) => {
                let gdb_stdout = String::from_utf8_lossy(&gdb_out.stdout);
                let gdb_stderr = String::from_utf8_lossy(&gdb_out.stderr);
                assert!(
                    gdb_out.status.success(),
                    "gdb failed for {tag}: stdout={gdb_stdout} stderr={gdb_stderr}"
                );
                assert!(
                    gdb_stdout.contains("main.lpp:6"),
                    "expected gdb breakpoint/backtrace on main.lpp:6 for {tag}: {gdb_stdout}"
                );
                assert!(
                    gdb_stdout.contains("42"),
                    "expected program output 42 under gdb for {tag}: {gdb_stdout}"
                );
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => panic!("failed running gdb for {tag}: {e}"),
        }
    }

    // Verify `lpp debug` CLI routing with LPP_DEBUGGER override.
    let mut session = lpp_driver::CompilerSession::new(lpp_driver::RewriteEngine);
    let req = lpp_driver::DriverRequest::from_args(
        [
            "lpp".to_string(),
            "debug".to_string(),
            src.to_string_lossy().into_owned(),
        ],
        &dir,
    )
    .expect("driver request");
    let orig_dbg = std::env::var_os("LPP_DEBUGGER");
    unsafe {
        std::env::set_var("LPP_DEBUGGER", "/bin/echo");
    }
    let outcome = session.execute(&req);
    match orig_dbg {
        Some(v) => unsafe { std::env::set_var("LPP_DEBUGGER", v) },
        None => unsafe { std::env::remove_var("LPP_DEBUGGER") },
    }
    assert_eq!(outcome.exit_code(), 0, "lpp debug CLI failed: {outcome:?}");

    // Verify LPP_BACKTRACE=1 produces runtime panic backtrace.
    let panic_src = dir.join("panic_div.lpp");
    std::fs::write(
        &panic_src,
        r#"def boom(a: Int, b: Int) -> Int:
    return a / b

def main():
    print(boom(10, 0))
"#,
    )
    .expect("write panic_div");
    let panic_exe = dir.join("panic_div");
    let mut panic_opts = CompileOptions::for_backend(BackendChoice::Cranelift);
    panic_opts.debug_info = true;
    build_executable_configured(
        &panic_src,
        "pkg",
        BackendChoice::Cranelift,
        panic_opts,
        &panic_exe,
        rt_dir,
        Some("cc"),
    )
    .expect("build panic_div");
    let mut panic_out = None;
    for attempt in 0..5 {
        match Command::new(&panic_exe).env("LPP_BACKTRACE", "1").output() {
            Ok(out) => {
                panic_out = Some(out);
                break;
            }
            Err(e) if e.raw_os_error() == Some(26) && attempt < 4 => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(e) => panic!("run panic_div failed: {e}"),
        }
    }
    let panic_out = panic_out.expect("panic_div output");
    assert!(
        !panic_out.status.success(),
        "expected non-zero exit on div by zero"
    );
    let panic_err = String::from_utf8_lossy(&panic_out.stderr);
    assert!(
        panic_err.contains("L++ RUNTIME PANIC") && panic_err.contains("Backtrace:"),
        "expected panic + backtrace, got stderr: {panic_err}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
