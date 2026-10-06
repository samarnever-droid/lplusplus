use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use lpp_codegen_api::{Backend, CodegenOptions, NameResolver, Target};
use lpp_codegen_wasm::WasmBackend;
use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    StringInterner, Symbol, lower_package,
};
use lpp_mir::{InterpreterErrorKind, InterpreterLimits, build_mir, execute_mir_arc};
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

fn compile_and_run(source: &str, test_id: &str) -> String {
    let fs = MemoryFs(BTreeMap::from([(
        PathBuf::from("/slice/main.lpp"),
        source.to_string(),
    )]));
    let graph = GraphBuilder::new(&fs)
        .build(GraphRequest::new(
            "/slice/main.lpp",
            PackageSpec::new("slice", "/slice"),
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

    let directory =
        std::env::temp_dir().join(format!("lpp-wasm-slice-{}-{test_id}", std::process::id()));
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

fn assert_slice_trap(source: &str, test_id: &str) {
    let fs = MemoryFs(BTreeMap::from([(
        PathBuf::from("/slice-trap/main.lpp"),
        source.to_string(),
    )]));
    let graph = GraphBuilder::new(&fs)
        .build(GraphRequest::new(
            "/slice-trap/main.lpp",
            PackageSpec::new("slice-trap", "/slice-trap"),
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
    let oracle_error = execute_mir_arc(
        &program,
        &inference.interner,
        entry,
        &[],
        InterpreterLimits::default(),
        &ownership.pinned_types(),
    )
    .unwrap_err();
    assert!(matches!(
        oracle_error.kind,
        InterpreterErrorKind::IndexOutOfBounds { .. }
    ));

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

    let directory = std::env::temp_dir().join(format!(
        "lpp-wasm-slice-trap-{}-{test_id}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let wasm = directory.join("module.wasm");
    std::fs::write(&wasm, module.object).unwrap();
    let host = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("wasm_run.mjs");
    let output = Command::new("node").arg(host).arg(&wasm).output().unwrap();
    assert!(!output.status.success(), "invalid slice unexpectedly ran");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("RuntimeError: unreachable"),
        "unexpected Node failure: {stderr}"
    );
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn list_and_string_slices_execute_across_element_classes() {
    let source = concat!(
        "def main() -> Int:\n",
        "    numbers := [10, 20, 30, 40, 50]\n",
        "    view := slice(numbers, 1, 3)\n",
        "    print_int(slice_len(view))\n",
        "    print_int(slice_get(view, 0))\n",
        "    print_int(slice_get(view, 1))\n",
        "    flags := [false, true, false]\n",
        "    bview := slice(flags, 1, 1)\n",
        "    print_bool(lpp_slice_get_bool(bview, 0))\n",
        "    text := \"Bhopal\"\n",
        "    sview := str_slice(text, 1, 3)\n",
        "    print_int(slice_len(sview))\n",
        "    print_str(slice_get(sview, 0))\n",
        "    print_str(str_slice_to_str(sview))\n",
        "    empty := str_slice(text, 6, 0)\n",
        "    print_str(slice_to_str(empty))\n",
        "    return 0\n",
    );

    assert_eq!(
        compile_and_run(source, "classes"),
        "3\n20\n30\n1\n3\nh\nhop\n\n"
    );
}

#[test]
fn invalid_slice_range_traps_like_the_oracle() {
    assert_slice_trap(
        concat!(
            "def main() -> Int:\n",
            "    numbers := [10, 20, 30]\n",
            "    invalid := slice(numbers, 2, 2)\n",
            "    print_int(slice_len(invalid))\n",
            "    return 0\n",
        ),
        "range",
    );
}

#[test]
fn invalid_slice_access_traps_like_the_oracle() {
    assert_slice_trap(
        concat!(
            "def main() -> Int:\n",
            "    numbers := [10, 20, 30]\n",
            "    view := slice(numbers, 0, 2)\n",
            "    print_int(slice_get(view, 2))\n",
            "    return 0\n",
        ),
        "access",
    );
}
