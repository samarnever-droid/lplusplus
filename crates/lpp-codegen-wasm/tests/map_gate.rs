use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use lpp_codegen_api::{Backend, CodegenOptions, NameResolver, Target};
use lpp_codegen_wasm::WasmBackend;
use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    StringInterner, Symbol, lower_package,
};
use lpp_mir::{
    InterpreterErrorKind, InterpreterLimit, InterpreterLimits, build_mir, execute_mir_arc,
};
use lpp_ownership::compute_ownership_plan;
use lpp_types::{ShadowInferenceOptions, infer_hir_package};

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

fn compile_and_run(source: &str) -> String {
    let fs = MemoryFs(BTreeMap::from([(
        PathBuf::from("/map/main.lpp"),
        source.to_string(),
    )]));
    let graph = GraphBuilder::new(&fs)
        .build(GraphRequest::new(
            "/map/main.lpp",
            PackageSpec::new("map", "/map"),
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
    let heap_error = execute_mir_arc(
        &program,
        &inference.interner,
        entry,
        &[],
        InterpreterLimits {
            max_heap_nodes: 0,
            ..InterpreterLimits::default()
        },
        &ownership.pinned_types(),
    )
    .unwrap_err();
    assert_eq!(
        heap_error.kind,
        InterpreterErrorKind::LimitExceeded(InterpreterLimit::HeapNodes)
    );
    let entry_error = execute_mir_arc(
        &program,
        &inference.interner,
        entry,
        &[],
        InterpreterLimits {
            max_aggregate_elements: 2,
            ..InterpreterLimits::default()
        },
        &ownership.pinned_types(),
    )
    .unwrap_err();
    assert_eq!(
        entry_error.kind,
        InterpreterErrorKind::LimitExceeded(InterpreterLimit::AggregateElements)
    );

    let names = Names(&package.names.symbols);
    let options = CodegenOptions::new(Target::Wasm32Wasi, &names);
    let module = WasmBackend
        .compile_module(&program, &inference.interner, &options)
        .unwrap();
    let repeated = WasmBackend
        .compile_module(&program, &inference.interner, &options)
        .unwrap();
    assert_eq!(module.object, repeated.object);
    assert_eq!(module.exported_symbols, repeated.exported_symbols);
    assert_eq!(module.imported_symbols, repeated.imported_symbols);
    for payload in wasmparser::Parser::new(0).parse_all(&module.object) {
        payload.unwrap();
    }
    let directory = std::env::temp_dir().join(format!("lpp-wasm-map-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let wasm = directory.join("module.wasm");
    std::fs::write(&wasm, module.object).unwrap();
    let host = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("wasm_run.mjs");
    let output = Command::new("node").arg(host).arg(&wasm).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let actual = String::from_utf8(output.stdout).unwrap();
    assert_eq!(actual, oracle.output.concat());
    let _ = std::fs::remove_dir_all(directory);
    actual
}

#[test]
fn integer_and_string_maps_cover_growth_update_lookup_and_remove() {
    let source = concat!(
        "def test_ints() -> Void:\n",
        "    mut ints := map_new()\n",
        "    map_put(ints, 0, 100)\n",
        "    map_put(ints, 1, 101)\n",
        "    map_put(ints, 2, 102)\n",
        "    map_put(ints, 3, 103)\n",
        "    map_put(ints, 4, 104)\n",
        "    map_put(ints, 5, 105)\n",
        "    map_put(ints, 6, 106)\n",
        "    map_put(ints, 7, 107)\n",
        "    map_put(ints, 8, 108)\n",
        "    map_put(ints, 9, 109)\n",
        "    map_put(ints, 5, 505)\n",
        "    print_int(map_len(ints))\n",
        "    print_int(map_get(ints, 5))\n",
        "    print_int(map_get(ints, 99))\n",
        "    if map_has(ints, 8):\n",
        "        print_int(1)\n",
        "    else:\n",
        "        print_int(0)\n",
        "    if map_has(ints, 99):\n",
        "        print_int(1)\n",
        "    else:\n",
        "        print_int(0)\n",
        "    map_remove(ints, 3)\n",
        "    print_int(map_len(ints))\n",
        "    print_int(map_get(ints, 9))\n",
        "\n",
        "def test_words() -> Void:\n",
        "    mut words := map_new()\n",
        "    map_put(words, \"apple\", 10)\n",
        "    map_put(words, \"banana\", 20)\n",
        "    map_put(words, \"apple\", 15)\n",
        "    print_int(map_get(words, \"apple\"))\n",
        "    if map_has(words, \"banana\"):\n",
        "        print_int(1)\n",
        "    else:\n",
        "        print_int(0)\n",
        "    map_remove(words, \"apple\")\n",
        "    print_int(map_len(words))\n",
        "    print_int(map_get(words, \"apple\"))\n",
        "\n",
        "def test_float_ints() -> Void:\n",
        "    mut values := map_new()\n",
        "    lpp_map_put_float(values, 7, 1.25)\n",
        "    lpp_map_put_float(values, 7, 2.5)\n",
        "    print_float(lpp_map_get_float(values, 7))\n",
        "    print_float(lpp_map_get_float(values, 99))\n",
        "\n",
        "def test_float_words() -> Void:\n",
        "    mut values := map_new()\n",
        "    lpp_map_put_str_float(values, \"pi\", 3.5)\n",
        "    print_float(lpp_map_get_str_float(values, \"pi\"))\n",
        "\n",
        "def main() -> Int:\n",
        "    test_ints()\n",
        "    test_words()\n",
        "    test_float_ints()\n",
        "    test_float_words()\n",
        "    return 0\n",
    );

    assert_eq!(
        compile_and_run(source),
        "10\n505\n0\n1\n0\n9\n109\n15\n1\n1\n0\n2.500000\n0.000000\n3.500000\n"
    );
}
