use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use lpp_codegen_api::{Backend, CodegenOptions, NameResolver, Target};
use lpp_codegen_wasm::WasmBackend;
use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    StringInterner, Symbol, lower_package,
};
use lpp_mir::{InterpreterLimits, build_mir, execute_mir_arc};
use lpp_ownership::compute_ownership_plan;
use lpp_types::{ShadowInferenceOptions, infer_hir_package};

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

struct MemoryFs(BTreeMap<PathBuf, String>);

impl FileSystem for MemoryFs {
    fn is_file(&self, path: &Path) -> Result<bool, FileSystemError> {
        Ok(self.0.contains_key(path))
    }

    fn canonicalize(&self, path: &Path) -> Result<PathBuf, FileSystemError> {
        if self.0.contains_key(path) || self.0.keys().any(|file| file.starts_with(path)) {
            Ok(path.to_owned())
        } else {
            Err(FileSystemError::new("canonicalize", path, "not found"))
        }
    }

    fn read_to_string(&self, path: &Path) -> Result<String, FileSystemError> {
        self.0
            .get(path)
            .cloned()
            .ok_or_else(|| FileSystemError::new("read", path, "not found"))
    }
}

struct Names<'a>(&'a StringInterner);

impl NameResolver for Names<'_> {
    fn resolve(&self, symbol_raw: u32) -> Option<&str> {
        self.0.resolve(Symbol::from_raw(symbol_raw))
    }
}

fn compile_with_oracle(source: &str, run_oracle: bool) -> (Vec<u8>, Option<String>) {
    let fs = MemoryFs(BTreeMap::from([(
        PathBuf::from("/cycle/main.lpp"),
        source.to_string(),
    )]));
    let graph = GraphBuilder::new(&fs)
        .build(GraphRequest::new(
            "/cycle/main.lpp",
            PackageSpec::new("cycle", "/cycle"),
        ))
        .unwrap();
    let package = lower_package(&graph, ResolutionMode::Namespaced).unwrap();
    let mut inference = infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap();
    let program = build_mir(
        &package,
        &graph.sources,
        &mut inference,
        lpp_mir::MirBuildOptions::default(),
    )
    .unwrap();
    let entry = program
        .functions()
        .find(|(_, function)| {
            function
                .name
                .and_then(|symbol| package.names.symbols.resolve(symbol))
                == Some("main")
        })
        .map(|(id, _)| id)
        .unwrap();

    let oracle_out = if run_oracle {
        let ownership = compute_ownership_plan(&program, &inference.interner).unwrap();
        let oracle = execute_mir_arc(
            &program,
            &inference.interner,
            entry,
            &[],
            InterpreterLimits::default(),
            &ownership.pinned_types(),
        )
        .unwrap();
        Some(oracle.output.concat())
    } else {
        None
    };

    let names = Names(&package.names.symbols);
    let options = CodegenOptions::new(Target::Wasm32Wasi, &names);
    let module = WasmBackend
        .compile_module(&program, &inference.interner, &options)
        .unwrap();
    let repeated = WasmBackend
        .compile_module(&program, &inference.interner, &options)
        .unwrap();
    assert_eq!(
        module.object, repeated.object,
        "WASM emission must be deterministic"
    );
    assert_eq!(module.exported_symbols, repeated.exported_symbols);
    assert_eq!(module.imported_symbols, repeated.imported_symbols);

    for payload in wasmparser::Parser::new(0).parse_all(&module.object) {
        payload.unwrap();
    }

    (module.object, oracle_out)
}

fn run_wasm_and_measure_pages(wasm_bytes: &[u8]) -> (String, usize) {
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let directory =
        std::env::temp_dir().join(format!("lpp-wasm-cycle-{}-{}", std::process::id(), id));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let wasm_path = directory.join("module.wasm");
    let runner_path = directory.join("run.mjs");
    std::fs::write(&wasm_path, wasm_bytes).unwrap();
    std::fs::write(
        &runner_path,
        r#"import { readFileSync } from "node:fs";
import { WASI } from "node:wasi";

const file = process.argv[2];
const wasi = new WASI({
  version: "preview1",
  args: [file],
  env: {},
  returnOnExit: true,
});
const bytes = readFileSync(file);
const mod = await WebAssembly.compile(bytes);
const instance = await WebAssembly.instantiate(mod, {
  wasi_snapshot_preview1: wasi.wasiImport,
});
const code = wasi.start(instance);
const pages = instance.exports.memory.buffer.byteLength >> 16;
process.stderr.write(`PAGES=${pages}\n`);
process.exitCode = code;
"#,
    )
    .unwrap();

    let output = Command::new("node")
        .arg(&runner_path)
        .arg(&wasm_path)
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&directory);
    assert!(
        output.status.success(),
        "WASM exited with {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    let pages = stderr
        .lines()
        .find_map(|line| line.strip_prefix("PAGES="))
        .and_then(|v| v.parse::<usize>().ok())
        .expect("PAGES marker in stderr");
    (stdout, pages)
}

fn verify_against_oracle(source: &str) -> (String, usize) {
    let (wasm, oracle) = compile_with_oracle(source, true);
    let (stdout, pages) = run_wasm_and_measure_pages(&wasm);
    assert_eq!(stdout, oracle.unwrap());
    (stdout, pages)
}

#[test]
fn struct_self_two_three_mutual_and_diamond_cycles_match_oracle() {
    let source = concat!(
        "struct Node:\n",
        "    val: Int\n",
        "    next: Node\n",
        "\n",
        "enum Link:\n",
        "    Nil\n",
        "    Some(node: ENode)\n",
        "\n",
        "struct ENode:\n",
        "    val: Int\n",
        "    next: Link\n",
        "\n",
        "struct Parent:\n",
        "    id: Int\n",
        "    child: Child\n",
        "\n",
        "struct Child:\n",
        "    id: Int\n",
        "    parent: Parent\n",
        "\n",
        "struct Diamond:\n",
        "    val: Int\n",
        "    left: Diamond\n",
        "    right: Diamond\n",
        "\n",
        "def make_self() -> Int:\n",
        "    mut a := Node(11, 0)\n",
        "    a.next = a\n",
        "    return a.val + a.next.val\n",
        "\n",
        "def make_two() -> Int:\n",
        "    mut a := Node(10, 0)\n",
        "    mut b := Node(20, 0)\n",
        "    a.next = b\n",
        "    b.next = a\n",
        "    return a.val + a.next.val\n",
        "\n",
        "def make_three() -> Int:\n",
        "    mut a := Node(1, 0)\n",
        "    mut b := Node(2, 0)\n",
        "    mut c := Node(3, 0)\n",
        "    a.next = b\n",
        "    b.next = c\n",
        "    c.next = a\n",
        "    return a.val + b.val + c.val\n",
        "\n",
        "def make_mutual() -> Int:\n",
        "    mut p := Parent(40, 0)\n",
        "    mut c := Child(5, 0)\n",
        "    p.child = c\n",
        "    c.parent = p\n",
        "    return p.id + p.child.id\n",
        "\n",
        "def make_diamond() -> Int:\n",
        "    mut top := Diamond(100, 0, 0)\n",
        "    mut left := Diamond(10, 0, 0)\n",
        "    mut right := Diamond(20, 0, 0)\n",
        "    mut bottom := Diamond(3, 0, 0)\n",
        "    top.left = left\n",
        "    top.right = right\n",
        "    left.left = bottom\n",
        "    right.right = bottom\n",
        "    bottom.left = top\n",
        "    bottom.right = top\n",
        "    return top.val + top.left.left.val\n",
        "\n",
        "def make_enum_cycle() -> Int:\n",
        "    keep := [ENode(17, Link.Nil)]\n",
        "    mut b := ENode(25, Link.Some(list_get(keep, 0)))\n",
        "    mut a := list_get(keep, 0)\n",
        "    a.next = Link.Some(b)\n",
        "    match a.next:\n",
        "        Nil:\n",
        "            return -1\n",
        "        Some(nxt):\n",
        "            return a.val + nxt.val\n",
        "\n",
        "def main() -> Void:\n",
        "    print(make_self())\n",
        "    print(make_two())\n",
        "    print(make_three())\n",
        "    print(make_mutual())\n",
        "    print(make_diamond())\n",
        "    print(make_enum_cycle())\n",
    );
    let (out, _) = verify_against_oracle(source);
    assert_eq!(out, "22\n30\n6\n45\n103\n42\n");
}

#[test]
fn live_root_cycle_survives_heavy_dead_cycle_churn() {
    let source = concat!(
        "struct Node:\n",
        "    val: Int\n",
        "    next: Node\n",
        "\n",
        "def churn_one(i: Int) -> Void:\n",
        "    mut x := Node(i, 0)\n",
        "    mut y := Node(i + 1, 0)\n",
        "    x.next = y\n",
        "    y.next = x\n",
        "\n",
        "def main() -> Void:\n",
        "    mut live_a := Node(700, 0)\n",
        "    mut live_b := Node(77, 0)\n",
        "    live_a.next = live_b\n",
        "    live_b.next = live_a\n",
        "    mut i := 0\n",
        "    while i < 1400:\n",
        "        churn_one(i)\n",
        "        i = i + 1\n",
        "    print(live_a.val + live_a.next.val)\n",
    );
    let (out, _) = verify_against_oracle(source);
    assert_eq!(out, "777\n");
}

#[test]
fn acyclic_tail_and_shared_external_survive_cycle_collection() {
    let source = concat!(
        "struct Tail:\n",
        "    label: Str\n",
        "    nums: List[Int]\n",
        "\n",
        "struct SharedBox:\n",
        "    tag: Str\n",
        "    count: Int\n",
        "\n",
        "struct CNode:\n",
        "    tail: Tail\n",
        "    shared: SharedBox\n",
        "    next: CNode\n",
        "\n",
        "def make_cyclic_with_tails(s: SharedBox, k: Int) -> Void:\n",
        "    keep_s := [s]\n",
        "    mut a := CNode(Tail(\"dead-a\", [k, k + 1]), list_get(keep_s, 0), 0)\n",
        "    mut b := CNode(Tail(\"dead-b\", [k + 2]), list_get(keep_s, 0), 0)\n",
        "    a.next = b\n",
        "    b.next = a\n",
        "\n",
        "def main() -> Void:\n",
        "    sb_holder := [SharedBox(\"kept\", 42)]\n",
        "    mut i := 0\n",
        "    while i < 800:\n",
        "        make_cyclic_with_tails(list_get(sb_holder, 0), i)\n",
        "        i = i + 1\n",
        "    sb := list_get(sb_holder, 0)\n",
        "    print_str(sb.tag)\n",
        "    print(sb.count)\n",
    );
    let (out, _) = verify_against_oracle(source);
    assert_eq!(out, "kept\n42\n");
}

#[test]
fn list_tuple_and_closure_cycles_match_oracle() {
    let source = concat!(
        "struct Bag:\n",
        "    val: Int\n",
        "    items: List[Bag]\n",
        "\n",
        "struct H:\n",
        "    t: (TNode, Int)\n",
        "\n",
        "struct TNode:\n",
        "    val: Int\n",
        "    h: H\n",
        "\n",
        "def run_list_cycle() -> Int:\n",
        "    mut b1 := Bag(10, list_new())\n",
        "    mut b2 := Bag(25, list_new())\n",
        "    list_push(b1.items, b2)\n",
        "    got := list_get(b1.items, 0)\n",
        "    list_push(got.items, b1)\n",
        "    return got.val + list_get(got.items, 0).val\n",
        "\n",
        "def run_tuple_cycle() -> Int:\n",
        "    mut a := TNode(55, 0)\n",
        "    mut b := TNode(9, 0)\n",
        "    a.h = H((b, 10))\n",
        "    mut bb := a.h.t.0\n",
        "    bb.h = H((a, 20))\n",
        "    return bb.h.t.0.val + bb.val\n",
        "\n",
        "def run_closure_cycle() -> Int:\n",
        "    mut fns := list_new()\n",
        "    reader := fn() -> Int:\n",
        "        return list_len(fns) + 100\n",
        "    list_push(fns, reader)\n",
        "    f := list_get(fns, 0)\n",
        "    return f()\n",
        "\n",
        "def main() -> Void:\n",
        "    print(run_list_cycle())\n",
        "    print(run_tuple_cycle())\n",
        "    print(run_closure_cycle())\n",
    );
    let (out, _) = verify_against_oracle(source);
    assert_eq!(out, "35\n64\n101\n");
}

#[test]
fn recursive_enum_acyclic_and_cyclic_match_oracle() {
    let source = concat!(
        "enum Expr:\n",
        "    Lit(val: Int)\n",
        "    Add(left: Expr, right: Expr)\n",
        "\n",
        "def eval_expr(e: Expr) -> Int:\n",
        "    match e:\n",
        "        Lit(v):\n",
        "            return v\n",
        "        Add(l, r):\n",
        "            return eval_expr(l) + eval_expr(r)\n",
        "\n",
        "def main() -> Void:\n",
        "    e := Expr.Add(Expr.Lit(19), Expr.Add(Expr.Lit(20), Expr.Lit(3)))\n",
        "    print(eval_expr(e))\n",
    );
    let (out, _) = verify_against_oracle(source);
    assert_eq!(out, "42\n");
}

#[test]
fn cyclic_churn_keeps_wasm_memory_pages_bounded() {
    let source_for = |iters: usize| {
        format!(
            concat!(
                "struct Node:\n",
                "    val: Int\n",
                "    next: Node\n",
                "\n",
                "struct Bag:\n",
                "    id: Int\n",
                "    items: List[Bag]\n",
                "\n",
                "def churn_step(i: Int) -> Void:\n",
                "    mut a := Node(i, 0)\n",
                "    mut b := Node(i + 1, 0)\n",
                "    a.next = b\n",
                "    b.next = a\n",
                "    mut bag1 := Bag(i, list_new())\n",
                "    mut bag2 := Bag(i + 2, list_new())\n",
                "    list_push(bag1.items, bag2)\n",
                "    got := list_get(bag1.items, 0)\n",
                "    list_push(got.items, bag1)\n",
                "\n",
                "def main() -> Void:\n",
                "    mut i := 0\n",
                "    while i < {iters}:\n",
                "        churn_step(i)\n",
                "        i = i + 1\n",
                "    print(1)\n",
            ),
            iters = iters
        )
    };

    let (small_wasm, small_oracle) = compile_with_oracle(&source_for(100), true);
    let (small_out, small_pages) = run_wasm_and_measure_pages(&small_wasm);
    assert_eq!(small_out, small_oracle.unwrap());

    let (large_wasm, _) = compile_with_oracle(&source_for(40_000), false);
    let (large_out, large_pages) = run_wasm_and_measure_pages(&large_wasm);
    assert_eq!(large_out, "1\n");
    assert!(
        large_pages <= small_pages + 2,
        "cyclic churn leaked WASM memory: 100 iters used {small_pages} pages, 40000 iters used {large_pages} pages"
    );
}
