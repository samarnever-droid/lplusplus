//! Production WebAssembly (`wasm32-wasip1`) test suite for L++ v0.3.0.
//!
//! Exercises `WasmBackend` compilation, `wasmparser` bytecode validation,
//! `wasmprinter` WAT inspection, WASI preview1 imports, linear memory layout,
//! deterministic module emission, scalars, strings/f-strings, lists/maps,
//! structs/enums/guarded & exhaustive match, closures, tuples 2..16,
//! cycle collection, and structured negative `E5xxx` rejections.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lpp_codegen_api::{
    Backend, CodegenErrorKind, CodegenOptions, CompiledModule, NameResolver, OptLevel, Target,
};
use lpp_codegen_wasm::WasmBackend;
use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    StringInterner, Symbol, lower_package,
};
use lpp_mir::{InterpreterLimits, MirBuildOptions, MirFunctionId, MirProgram, build_mir};
use lpp_ownership::compute_ownership_plan;
use lpp_types::{ShadowInferenceOptions, TypeInterner, infer_hir_package};

#[derive(Debug)]
struct MemoryFileSystem {
    files: BTreeMap<PathBuf, String>,
}

impl MemoryFileSystem {
    fn new(source: &str) -> Self {
        Self {
            files: BTreeMap::from([(PathBuf::from("/wasm_v03/main.lpp"), source.to_owned())]),
        }
    }
}

impl FileSystem for MemoryFileSystem {
    fn is_file(&self, path: &Path) -> Result<bool, FileSystemError> {
        Ok(self.files.contains_key(path))
    }

    fn canonicalize(&self, path: &Path) -> Result<PathBuf, FileSystemError> {
        if self.files.contains_key(path) || self.files.keys().any(|file| file.starts_with(path)) {
            Ok(path.to_owned())
        } else {
            Err(FileSystemError::new("canonicalize", path, "path not found"))
        }
    }

    fn read_to_string(&self, path: &Path) -> Result<String, FileSystemError> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| FileSystemError::new("read", path, "path not found"))
    }
}

struct Names<'a>(&'a StringInterner);

impl NameResolver for Names<'_> {
    fn resolve(&self, symbol_raw: u32) -> Option<&str> {
        self.0.resolve(Symbol::from_raw(symbol_raw))
    }
}

fn pipeline(source: &str) -> (MirProgram, TypeInterner, lpp_hir::HirPackage) {
    let fs = MemoryFileSystem::new(source);
    let graph = GraphBuilder::new(&fs)
        .build(GraphRequest::new(
            "/wasm_v03/main.lpp",
            PackageSpec::new("wasm_v03", "/wasm_v03"),
        ))
        .unwrap_or_else(|e| panic!("graph: {e:?}"));
    let package = lower_package(&graph, ResolutionMode::Namespaced)
        .unwrap_or_else(|e| panic!("lower: {e:?}"));
    let mut types = infer_hir_package(&package, ShadowInferenceOptions::default())
        .unwrap_or_else(|e| panic!("types: {e:?}"));
    let program = build_mir(
        &package,
        &graph.sources,
        &mut types,
        MirBuildOptions::default(),
    )
    .unwrap_or_else(|e| panic!("mir: {e:?}"));
    (program, types.interner, package)
}

fn compile_wasm_checked(source: &str, label: &str) -> (CompiledModule, String) {
    let (program, types, package) = pipeline(source);
    let names = Names(&package.names.symbols);
    let options = CodegenOptions::new(Target::Wasm32Wasi, &names);
    let module = WasmBackend
        .compile_module(&program, &types, &options)
        .unwrap_or_else(|e| panic!("{label}: compile_module failed: {e:?}"));

    assert!(
        module.object.starts_with(b"\0asm\x01\0\0\0"),
        "{label}: invalid wasm magic/version header"
    );
    for payload in wasmparser::Parser::new(0).parse_all(&module.object) {
        if let Err(err) = payload {
            panic!("{label}: wasmparser rejected module: {err}");
        }
    }
    let wat = wasmprinter::print_bytes(&module.object)
        .unwrap_or_else(|e| panic!("{label}: wasmprinter failed: {e}"));
    (module, wat)
}

fn main_fn(program: &MirProgram, names: &StringInterner) -> MirFunctionId {
    program
        .functions()
        .find(|(_, f)| f.name.and_then(|s| names.resolve(s)) == Some("main"))
        .map(|(id, _)| id)
        .expect("defines main")
}

#[test]
fn wasm_scalars_control_flow_and_recursion_validate_and_are_deterministic() {
    let src = r#"
def gcd(a: Int, b: Int) -> Int:
    mut x := a
    mut y := b
    while y != 0:
        t := x % y
        x = y
        y = t
    return x

def fib(n: Int) -> Int:
    if n <= 1:
        return n
    return fib(n - 1) + fib(n - 2)

def main() -> Int:
    mut acc := 0
    for i in 1..=10:
        acc = acc + gcd(i * 12, 18)
    print_int(acc + fib(8))
    return 0
"#;
    let (m1, wat1) = compile_wasm_checked(src, "scalars_1");
    let (m2, wat2) = compile_wasm_checked(src, "scalars_2");
    assert_eq!(
        m1.object, m2.object,
        "WASM emission must be bit-for-bit deterministic"
    );
    assert_eq!(wat1, wat2);
    assert!(wat1.contains("(export \"_start\""));
    assert!(wat1.contains("(memory"));
}

#[test]
fn wasm_structs_enums_and_guarded_pattern_matching_validate() {
    let src = r#"
enum Shape:
    Circle(r: Float)
    Rect(w: Float, h: Float)
    Tag(code: Int, label: Str)

struct Canvas:
    id: Int
    primary: Shape

def eval_shape(s: Shape) -> Int:
    match s:
        Shape.Circle(r) if r > 5.0:
            return 100
        Shape.Circle(_):
            return 10
        Shape.Rect(w, h) if w == h:
            return 50
        Shape.Rect(_, _):
            return 25
        Shape.Tag(code, label):
            return code + str_len(label)

def main() -> Int:
    c1 := Canvas(1, Shape.Circle(10.0))
    c2 := Canvas(2, Shape.Rect(4.0, 4.0))
    c3 := Canvas(3, Shape.Tag(7, "wasm"))
    total := eval_shape(c1.primary) + eval_shape(c2.primary) + eval_shape(c3.primary)
    print_int(total)
    return 0
"#;
    let (program, types, package) = pipeline(src);
    let plan = compute_ownership_plan(&program, &types).unwrap();
    let entry = main_fn(&program, &package.names.symbols);
    let oracle = lpp_mir::execute_mir_arc(
        &program,
        &types,
        entry,
        &[],
        InterpreterLimits::default(),
        &plan.pinned_types(),
    )
    .unwrap();
    assert_eq!(oracle.output.concat().trim(), "161");

    let (_module, wat) = compile_wasm_checked(src, "wasm_adts");
    assert!(wat.contains("fd_write"));
}

#[test]
fn wasm_strings_fstrings_slices_lists_and_maps_validate() {
    let src = r#"
def format_entry(k: Str, v: Int) -> Str:
    return f"{k}={int_to_str(v)}"

def main() -> Int:
    mut items := [format_entry("a", 10)]
    list_push(items, format_entry("b", 20))
    joined := items[0] + ";" + items[1]
    upper := str_to_upper(joined)
    print_str(upper)

    mut m := map_new()
    map_put(m, 1, 100)
    map_put(m, 2, 200)
    if map_has(m, 1):
        print_int(map_get(m, 1) + map_get(m, 2))
    return 0
"#;
    let (_module, wat) = compile_wasm_checked(src, "wasm_strings_collections");
    assert!(wat.contains("wasi_snapshot_preview1"));
}

#[test]
fn wasm_tuples_2_through_16_and_closures_validate() {
    let src = r#"
def main() -> Int:
    t4 := (10, 20, 30, 40)
    t8 := (1, 2, 3, 4, 5, 6, 7, 8)
    t12 := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12)
    t16 := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16)
    mut base := t4.0 + t8.7 + t12.11 + t16.15
    f := fn(x: Int) -> Int:
        base = base + x
        return base
    print_int(f(5) + f(10))
    return 0
"#;
    let (_module, wat) = compile_wasm_checked(src, "wasm_tuples_closures");
    assert!(wat.contains("call_indirect"));
}

#[test]
fn wasm_cycle_collection_and_wasi_filesystem_builtins_validate() {
    let src = r#"
enum MaybeNode:
    None
    Some(node: Node)

struct Node:
    id: Int
    next: MaybeNode

def main() -> Int:
    keep := [Node(1, MaybeNode.None)]
    mut b := Node(2, MaybeNode.Some(list_get(keep, 0)))
    mut a := list_get(keep, 0)
    a.next = MaybeNode.Some(b)
    exists := file_exists("nonexistent_wasm_probe.txt")
    if exists:
        print_int(0)
    else:
        print_int(a.id + b.id)
    return 0
"#;
    let (_module, wat) = compile_wasm_checked(src, "wasm_cycles_wasi_fs");
    assert!(wat.contains("wasi_snapshot_preview1"));
}

#[test]
fn wasm_rejects_unsupported_targets_and_bare_metal_intrinsics_cleanly() {
    let (program, types, package) = pipeline("def main() -> Int:\n    return 0\n");
    let names = Names(&package.names.symbols);
    let bad_target = CodegenOptions::new(Target::X86_64, &names);
    let err = WasmBackend
        .compile_module(&program, &types, &bad_target)
        .expect_err("WasmBackend must reject X86_64 target");
    assert!(matches!(err.kind, CodegenErrorKind::UnsupportedTarget(_)));

    let kernel_src = r#"
unsafe def port_probe() -> Int:
    port_out8(0x3F8 as U16, 65 as U8)
    b := port_in8(0x3F8 as U16)
    return b as Int

def main() -> Int:
    unsafe:
        return port_probe()
"#;
    let (kprog, ktypes, kpkg) = pipeline(kernel_src);
    let knames = Names(&kpkg.names.symbols);
    let kopts = CodegenOptions::new(Target::Wasm32Wasi, &knames).with_opt_level(OptLevel::None);
    let kerr = WasmBackend
        .compile_module(&kprog, &ktypes, &kopts)
        .expect_err("WasmBackend must reject x86 port I/O intrinsics");
    assert_eq!(kerr.code(), "E5001");
}
