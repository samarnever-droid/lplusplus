use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use lpp_codegen_api::{Backend, CodegenOptions, NameResolver, Target};
use lpp_codegen_wasm::WasmBackend;
use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    StringInterner, Symbol, lower_package,
};
use lpp_mir::{InterpreterLimits, build_mir, execute_mir_arc};
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

fn assert_wasm_matches_arc(source: &str, test_id: &str) -> String {
    let fs = MemoryFs(BTreeMap::from([(
        PathBuf::from("/tuple/main.lpp"),
        source.to_string(),
    )]));
    let graph = GraphBuilder::new(&fs)
        .build(GraphRequest::new(
            "/tuple/main.lpp",
            PackageSpec::new("tuple", "/tuple"),
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
    let plan = compute_ownership_plan(&program, &inference.interner).unwrap();
    let oracle = execute_mir_arc(
        &program,
        &inference.interner,
        entry,
        &[],
        InterpreterLimits::default(),
        &plan.pinned_types(),
    )
    .unwrap();

    let module = WasmBackend
        .compile_module(
            &program,
            &inference.interner,
            &CodegenOptions::new(Target::Wasm32Wasi, &Names(&package.names.symbols)),
        )
        .unwrap();
    for payload in wasmparser::Parser::new(0).parse_all(&module.object) {
        payload.unwrap();
    }

    let directory =
        std::env::temp_dir().join(format!("lpp-wasm-tuple-{}-{test_id}", std::process::id()));
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
fn tuple_projection_and_dynamic_string_concat_match_the_arc_oracle() {
    let source = concat!(
        "def main() -> Int:\n",
        "    left := \"hello \"\n",
        "    right := \"world\"\n",
        "    pair := (20, 22)\n",
        "    words := (left + right, \"!\")\n",
        "    print_str(words.0 + words[1])\n",
        "    print_int(pair.0 + pair[1])\n",
        "    return 0\n",
    );
    assert_eq!(
        assert_wasm_matches_arc(source, "baseline"),
        "hello world!\n42\n"
    );
}

#[test]
fn nested_tuples_and_replacement_stress_match_the_arc_oracle() {
    let source = concat!(
        "def main() -> Int:\n",
        "    inner := (\"deep\" + \"ly\", 5)\n",
        "    outer := (inner, \"own\" + \"ed\", 7)\n",
        "    nested := outer.0\n",
        "    print_str(nested.0)\n",
        "    print_int(nested.1 + outer.2)\n",
        "    print_str(outer.1 + nested.0)\n",
        "    mut pair := (\"s\" + \"0\", 0)\n",
        "    mut i := 1\n",
        "    while i < 8:\n",
        "        pair = (pair.0 + int_to_str(i), i)\n",
        "        i = i + 1\n",
        "    print_str(pair.0)\n",
        "    print_int(pair.1)\n",
        "    return 0\n",
    );
    assert_eq!(
        assert_wasm_matches_arc(source, "nested-replace"),
        "deeply\n12\nowneddeeply\ns01234567\n7\n"
    );
}
