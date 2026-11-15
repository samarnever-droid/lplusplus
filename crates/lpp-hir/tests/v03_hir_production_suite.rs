//! Production-grade integration test suite for `lpp-hir`.
//!
//! Covers multi-module package graphs, diamond & cross-package dependency
//! resolution, re-exports, circular import rejection, case sensitivity,
//! declaration & generic lowering, type-ref trees (`Ptr`, `RawPtr`, `List`,
//! `Map`, `Set`, `Deque`, `Option`, `Result`, tuples), operator precedence,
//! exhaustive & guarded match desugaring (`build_variant_clause_chain`,
//! `build_value_match_stmts`), f-string interpolation desugaring, default
//! parameter & variadic lowering, `unsafe` origin tracking, and `E1xxx`/`E3xxx`
//! diagnostics.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use lpp_common::Diagnostic;
use lpp_hir::{
    BinaryOperator, BindingTarget, BodyId, DefId, DefinitionKind, DesugaringKind, Enum, ExprId,
    ExpressionKind, ExternBlock, FileSystem, FileSystemError, Function, GraphBuilder, GraphError,
    GraphRequest, HirItem, HirItemKind, HirPackage, Impl, Literal, LocalKind, ModuleId,
    NameBinding, OriginKind, PackageGraph, PackageSpec, ResolutionMode, Statement, StatementKind,
    StmtId, Struct, Symbol, Trait, TypeRefId, TypeRefKind, UnaryOperator, build_name_index,
    hir_snapshot, lower_package,
};

#[derive(Debug, Default)]
struct MemoryFileSystem {
    files: BTreeMap<PathBuf, String>,
    case_insensitive: bool,
}

impl MemoryFileSystem {
    fn with_files(files: &[(&str, &str)]) -> Self {
        Self {
            files: files
                .iter()
                .map(|(path, source)| (PathBuf::from(path), (*source).to_owned()))
                .collect(),
            case_insensitive: false,
        }
    }

    fn case_insensitive(mut self) -> Self {
        self.case_insensitive = true;
        self
    }

    fn actual_file(&self, requested: &Path) -> Option<&PathBuf> {
        let requested_norm = requested.to_string_lossy().replace('\\', "/");
        if self.case_insensitive {
            let requested_lower = requested_norm.to_lowercase();
            self.files.keys().find(|path| {
                path.to_string_lossy().replace('\\', "/").to_lowercase() == requested_lower
            })
        } else {
            self.files
                .keys()
                .find(|path| path.to_string_lossy().replace('\\', "/") == requested_norm)
        }
    }
}

impl FileSystem for MemoryFileSystem {
    fn is_file(&self, path: &Path) -> Result<bool, FileSystemError> {
        Ok(self.actual_file(path).is_some())
    }

    fn canonicalize(&self, path: &Path) -> Result<PathBuf, FileSystemError> {
        if let Some(actual) = self.actual_file(path) {
            return Ok(actual.clone());
        }
        let path_norm = path.to_string_lossy().replace('\\', "/");
        let dir_prefix = if path_norm.ends_with('/') {
            path_norm.clone()
        } else {
            format!("{path_norm}/")
        };
        if self.case_insensitive {
            let dir_lower = dir_prefix.to_lowercase();
            if self.files.keys().any(|file| {
                file.to_string_lossy()
                    .replace('\\', "/")
                    .to_lowercase()
                    .starts_with(&dir_lower)
            }) {
                return Ok(path.to_owned());
            }
        } else if self.files.keys().any(|file| {
            file.to_string_lossy()
                .replace('\\', "/")
                .starts_with(&dir_prefix)
        }) {
            return Ok(path.to_owned());
        }
        Err(FileSystemError::new("canonicalize", path, "path not found"))
    }

    fn read_to_string(&self, path: &Path) -> Result<String, FileSystemError> {
        let Some(actual) = self.actual_file(path) else {
            return Err(FileSystemError::new("read", path, "file not found"));
        };
        Ok(self.files[actual].clone())
    }
}

fn app_request() -> GraphRequest {
    GraphRequest::new("/app/src/main.lpp", PackageSpec::new("app", "/app/src"))
}

fn build_graph_files(
    files: &[(&str, &str)],
    deps: &[(&str, &str)],
) -> Result<PackageGraph, GraphError> {
    let fs = MemoryFileSystem::with_files(files);
    let mut req = app_request();
    for (name, root) in deps {
        req = req.with_dependency(PackageSpec::new(*name, *root));
    }
    GraphBuilder::new(&fs).build(req)
}

fn lower_single(source: &str) -> HirPackage {
    let graph = build_graph_files(&[("/app/src/main.lpp", source)], &[])
        .unwrap_or_else(|err| panic!("expected graph build success, got {err}"));
    lower_package(&graph, ResolutionMode::Namespaced)
        .unwrap_or_else(|diags| panic!("expected lowering success, got {diags:#?}"))
}

fn lower_single_err(source: &str) -> Vec<Diagnostic> {
    let graph = build_graph_files(&[("/app/src/main.lpp", source)], &[])
        .unwrap_or_else(|err| panic!("expected graph build success, got {err}"));
    lower_package(&graph, ResolutionMode::Namespaced).expect_err("expected lowering failure")
}

fn lower_files(
    files: &[(&str, &str)],
    deps: &[(&str, &str)],
    mode: ResolutionMode,
) -> (PackageGraph, HirPackage) {
    let graph = build_graph_files(files, deps)
        .unwrap_or_else(|err| panic!("expected graph build success, got {err}"));
    let hir = lower_package(&graph, mode)
        .unwrap_or_else(|diags| panic!("expected lowering success, got {diags:#?}"));
    (graph, hir)
}

fn sym_str<'a>(hir: &'a HirPackage, symbol: Symbol) -> &'a str {
    hir.names.symbols.resolve(symbol).expect("interned symbol")
}

fn find_item<'a>(hir: &'a HirPackage, name: &str) -> &'a HirItem {
    let target = hir
        .names
        .symbols
        .get(name)
        .unwrap_or_else(|| panic!("missing symbol '{name}'"));
    hir.items
        .iter()
        .find(|item| item.name == Some(target))
        .unwrap_or_else(|| panic!("missing item '{name}'"))
}

fn find_fn(hir: &HirPackage, name: &str) -> Function {
    match find_item(hir, name).kind {
        HirItemKind::Function(function) => function,
        other => panic!("expected function for '{name}', got {other:?}"),
    }
}

fn find_struct(hir: &HirPackage, name: &str) -> Struct {
    match find_item(hir, name).kind {
        HirItemKind::Struct(s) => s,
        other => panic!("expected struct for '{name}', got {other:?}"),
    }
}

fn find_enum(hir: &HirPackage, name: &str) -> Enum {
    match find_item(hir, name).kind {
        HirItemKind::Enum(e) => e,
        other => panic!("expected enum for '{name}', got {other:?}"),
    }
}

fn find_trait(hir: &HirPackage, name: &str) -> Trait {
    match find_item(hir, name).kind {
        HirItemKind::Trait(t) => t,
        other => panic!("expected trait for '{name}', got {other:?}"),
    }
}

fn find_impls(hir: &HirPackage) -> Vec<Impl> {
    hir.items
        .iter()
        .filter_map(|item| match item.kind {
            HirItemKind::Impl(imp) => Some(imp),
            _ => None,
        })
        .collect()
}

fn find_externs(hir: &HirPackage) -> Vec<ExternBlock> {
    hir.items
        .iter()
        .filter_map(|item| match item.kind {
            HirItemKind::Extern(ext) => Some(ext),
            _ => None,
        })
        .collect()
}

fn body_stmts<'a>(hir: &'a HirPackage, body: BodyId) -> &'a [StmtId] {
    hir.statements(hir.bodies[body].statements)
}

fn collect_all_stmts_in_body(hir: &HirPackage, body: BodyId, out: &mut Vec<Statement>) {
    for &stmt_id in body_stmts(hir, body) {
        let stmt = hir.statements[stmt_id];
        out.push(stmt);
        match stmt.kind {
            StatementKind::If {
                then_body,
                else_body,
                ..
            } => {
                collect_all_stmts_in_body(hir, then_body, out);
                if let Some(eb) = else_body {
                    collect_all_stmts_in_body(hir, eb, out);
                }
            }
            StatementKind::While { body, .. }
            | StatementKind::Unsafe { body }
            | StatementKind::For { body, .. } => {
                collect_all_stmts_in_body(hir, body, out);
            }
            StatementKind::Match { arms, .. } => {
                for &arm in hir.match_arms(arms) {
                    collect_all_stmts_in_body(hir, hir.match_arms[arm].body, out);
                }
            }
            StatementKind::Let { .. }
            | StatementKind::Assign { .. }
            | StatementKind::Expression(_)
            | StatementKind::Return(_)
            | StatementKind::Break
            | StatementKind::Continue => {}
        }
    }
}

fn flatten_type_ref(hir: &HirPackage, id: TypeRefId) -> String {
    match hir.type_refs[id].kind {
        TypeRefKind::Named(sym) => sym_str(hir, sym).to_owned(),
        TypeRefKind::Applied { base, arguments } => {
            let args = hir
                .type_refs(arguments)
                .iter()
                .map(|arg| flatten_type_ref(hir, *arg))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}[{args}]", sym_str(hir, base))
        }
        TypeRefKind::Tuple(elements) => {
            let elems = hir
                .type_refs(elements)
                .iter()
                .map(|elem| flatten_type_ref(hir, *elem))
                .collect::<Vec<_>>()
                .join(", ");
            format!("({elems})")
        }
    }
}

fn flatten_fstring_concat(hir: &HirPackage, expr: ExprId, out: &mut Vec<ExprId>) {
    match hir.expressions[expr].kind {
        ExpressionKind::Binary {
            left,
            operator: BinaryOperator::Add,
            right,
        } => {
            flatten_fstring_concat(hir, left, out);
            out.push(right);
        }
        _ => out.push(expr),
    }
}

fn module_by_suffix<'a>(graph: &'a PackageGraph, suffix: &str) -> ModuleId {
    graph
        .modules
        .iter()
        .find(|m| {
            m.path
                .to_string_lossy()
                .replace('\\', "/")
                .ends_with(suffix)
        })
        .unwrap_or_else(|| panic!("missing module ending with {suffix}"))
        .id
}

// ============================================================================
// Section 1: Multi-Module Graphs, Diamond Dependencies, Cycles & Resolution
// ============================================================================

#[test]
fn diamond_dependency_graph_deduplicates_shared_module_and_preserves_edges() {
    let files = [
        (
            "/app/src/main.lpp",
            "import left\nimport right\ndef main() -> Int:\n    return left.l() + right.r()\n",
        ),
        (
            "/app/src/left.lpp",
            "from core.math import base_val\ndef l() -> Int:\n    return base_val() + 1\n",
        ),
        (
            "/app/src/right.lpp",
            "import core.math as cm\ndef r() -> Int:\n    return cm.base_val() + 2\n",
        ),
        (
            "/app/src/core/math.lpp",
            "pub def base_val() -> Int:\n    return 40\n",
        ),
    ];
    let (graph, hir) = lower_files(&files, &[], ResolutionMode::Namespaced);
    assert_eq!(graph.modules.len(), 4);
    assert_eq!(graph.edges.len(), 4);

    let core_id = module_by_suffix(&graph, "/app/src/core/math.lpp");
    let left_id = module_by_suffix(&graph, "/app/src/left.lpp");
    let right_id = module_by_suffix(&graph, "/app/src/right.lpp");

    let core_importers: BTreeSet<ModuleId> = graph
        .edges
        .iter()
        .filter(|edge| edge.imported == core_id)
        .map(|edge| edge.importer)
        .collect();
    assert_eq!(core_importers, BTreeSet::from([left_id, right_id]));

    // Both `left.l()` and `cm.base_val()` resolve to their target definitions via HirPackage::expression_definition.
    let resolved_defs: Vec<DefId> = hir
        .expressions
        .enumerate()
        .filter_map(|(id, _)| hir.expression_definition(id))
        .collect();
    assert!(resolved_defs.len() >= 3);
}

#[test]
fn relative_importer_directory_and_package_root_fallback_resolve_cleanly() {
    let files = [
        (
            "/app/src/main.lpp",
            "import sub.entry\ndef main() -> Int:\n    return sub.entry.run()\n",
        ),
        (
            "/app/src/sub/entry.lpp",
            "import local_helper\nimport shared.root_helper\npub def run() -> Int:\n    return local_helper.a() + root_helper.b()\n",
        ),
        (
            "/app/src/sub/local_helper.lpp",
            "pub def a() -> Int:\n    return 10\n",
        ),
        (
            "/app/src/shared/root_helper.lpp",
            "pub def b() -> Int:\n    return 20\n",
        ),
    ];
    let (graph, hir) = lower_files(&files, &[], ResolutionMode::Namespaced);
    assert_eq!(graph.modules.len(), 4);
    let sub_entry = module_by_suffix(&graph, "/app/src/sub/entry.lpp");
    assert_eq!(graph.dependencies(sub_entry).count(), 2);
    assert!(matches!(
        hir.names.resolve(sub_entry, "local_helper"),
        Some(BindingTarget::Module(_))
    ));
    assert!(matches!(
        hir.names.resolve(sub_entry, "root_helper"),
        Some(BindingTarget::Module(_))
    ));
}

#[test]
fn cross_package_dependencies_resolve_prefixed_and_unprefixed_and_keep_order() {
    let files = [
        (
            "/app/src/main.lpp",
            concat!(
                "import std.io\n",
                "from net.http import get\n",
                "import crypto\n",
                "def main() -> Int:\n",
                "    io.flush()\n",
                "    return get() + crypto.hash()\n",
            ),
        ),
        ("/deps/std/io.lpp", "pub def flush():\n    return\n"),
        (
            "/deps/net/net/http.lpp",
            "pub def get() -> Int:\n    return 200\n",
        ),
        (
            "/deps/crypto/crypto.lpp",
            "pub def hash() -> Int:\n    return 7\n",
        ),
    ];
    let g1 = build_graph_files(
        &files,
        &[
            ("std", "/deps/std"),
            ("net", "/deps/net"),
            ("crypto", "/deps/crypto"),
        ],
    )
    .unwrap();
    let g2 = build_graph_files(
        &files,
        &[
            ("crypto", "/deps/crypto"),
            ("std", "/deps/std"),
            ("net", "/deps/net"),
        ],
    )
    .unwrap();

    assert_eq!(
        g1.packages.iter().map(|p| &p.name).collect::<Vec<_>>(),
        g2.packages.iter().map(|p| &p.name).collect::<Vec<_>>()
    );
    assert_eq!(
        g1.modules.iter().map(|m| &m.path).collect::<Vec<_>>(),
        g2.modules.iter().map(|m| &m.path).collect::<Vec<_>>()
    );
    assert_eq!(g1.edges, g2.edges);

    let h1 = lower_package(&g1, ResolutionMode::Namespaced).unwrap();
    let h2 = lower_package(&g2, ResolutionMode::Namespaced).unwrap();
    assert_eq!(h1, h2);
}

#[test]
fn reexport_and_module_namespace_field_chains_resolve_targets() {
    // Module `api` imports `impl_mod` and defines `wrapper`; `main` imports `api` as `a`
    // and accesses both `a.wrapper` and `a.impl_mod`.
    let files = [
        (
            "/app/src/main.lpp",
            concat!(
                "import api as a\n",
                "def main() -> Int:\n",
                "    x := a.wrapper()\n",
                "    y := a.impl_mod\n",
                "    return x\n",
            ),
        ),
        (
            "/app/src/api.lpp",
            "import impl_mod\npub def wrapper() -> Int:\n    return impl_mod.core()\n",
        ),
        (
            "/app/src/impl_mod.lpp",
            "pub def core() -> Int:\n    return 99\n",
        ),
    ];
    let (graph, hir) = lower_files(&files, &[], ResolutionMode::Namespaced);
    let impl_id = module_by_suffix(&graph, "/app/src/impl_mod.lpp");

    let field_bindings: Vec<BindingTarget> = hir
        .expressions
        .enumerate()
        .filter_map(|(id, expr)| match expr.kind {
            ExpressionKind::Field { .. } => hir.expression_binding(id),
            _ => None,
        })
        .collect();
    assert!(
        field_bindings.contains(&BindingTarget::Module(impl_id)),
        "a.impl_mod must resolve via ModuleScope::imported to impl_mod ModuleId"
    );
    assert!(
        field_bindings
            .iter()
            .any(|b| matches!(b, BindingTarget::Definition(_)))
    );
}

#[test]
fn circular_imports_are_rejected_deterministically_unless_allow_cycles_is_set() {
    // 1. Self cycle
    let self_err = build_graph_files(
        &[(
            "/app/src/main.lpp",
            "import main\ndef main():\n    return\n",
        )],
        &[],
    )
    .expect_err("self-import must be rejected");
    let GraphError::ImportCycle(self_paths) = self_err else {
        panic!("expected ImportCycle");
    };
    assert_eq!(
        self_paths,
        [
            PathBuf::from("/app/src/main.lpp"),
            PathBuf::from("/app/src/main.lpp"),
        ]
    );

    // 2. 3-node cycle a -> b -> c -> a
    let three_files = [
        ("/app/src/main.lpp", "import a\ndef main():\n    return\n"),
        ("/app/src/a.lpp", "import b\ndef fa():\n    return\n"),
        ("/app/src/b.lpp", "import c\ndef fb():\n    return\n"),
        ("/app/src/c.lpp", "import a\ndef fc():\n    return\n"),
    ];
    let cycle_err = build_graph_files(&three_files, &[]).expect_err("3-node cycle must fail");
    let GraphError::ImportCycle(paths) = cycle_err else {
        panic!("expected ImportCycle");
    };
    assert_eq!(
        paths,
        [
            PathBuf::from("/app/src/a.lpp"),
            PathBuf::from("/app/src/b.lpp"),
            PathBuf::from("/app/src/c.lpp"),
            PathBuf::from("/app/src/a.lpp"),
        ]
    );

    // 3. `with_allow_cycles(true)` permits lowering the cyclic graph.
    let fs = MemoryFileSystem::with_files(&three_files);
    let graph = GraphBuilder::new(&fs)
        .build(app_request().with_allow_cycles(true))
        .expect("allow_cycles must allow cyclic graph");
    assert_eq!(graph.modules.len(), 4);
    let hir = lower_package(&graph, ResolutionMode::Namespaced).unwrap();
    assert_eq!(hir.modules.len(), 4);
}

#[test]
fn graph_builder_reports_case_mismatch_missing_ambiguous_duplicate_and_entry_errors() {
    // 1. Missing entry (source root exists, entry file does not)
    let no_entry_fs =
        MemoryFileSystem::with_files(&[("/app/src/other.lpp", "def ok():\n    return\n")]);
    let missing_entry = GraphBuilder::new(&no_entry_fs)
        .build(GraphRequest::new(
            "/app/src/missing.lpp",
            PackageSpec::new("app", "/app/src"),
        ))
        .expect_err("missing entry must fail");
    assert!(matches!(missing_entry, GraphError::MissingEntry(_)));

    // 2. Case mismatch on entry with case-insensitive FS
    let entry_case_fs =
        MemoryFileSystem::with_files(&[("/app/src/Main.lpp", "def main():\n    return\n")])
            .case_insensitive();
    let entry_case_err = GraphBuilder::new(&entry_case_fs)
        .build(app_request())
        .expect_err("entry casing mismatch must fail");
    assert!(matches!(entry_case_err, GraphError::CaseMismatch { .. }));

    // 3. Case mismatch on transitive import with case-insensitive FS
    let imp_case_fs = MemoryFileSystem::with_files(&[
        (
            "/app/src/main.lpp",
            "import sub.Helper\ndef main():\n    return\n",
        ),
        ("/app/src/sub/helper.lpp", "def h():\n    return\n"),
    ])
    .case_insensitive();
    let imp_case_err = GraphBuilder::new(&imp_case_fs)
        .build(app_request())
        .expect_err("import casing mismatch must fail");
    assert!(matches!(imp_case_err, GraphError::CaseMismatch { .. }));

    // 4. Missing module records searched paths
    let missing_mod_err = build_graph_files(
        &[
            (
                "/app/src/main.lpp",
                "import ghost.mod\ndef main():\n    return\n",
            ),
            ("/deps/dep/mod.lpp", "def ok():\n    return\n"),
        ],
        &[("dep", "/deps/dep")],
    )
    .expect_err("missing module must fail");
    assert!(matches!(
        missing_mod_err,
        GraphError::MissingModule {
            ref searched,
            ..
        } if !searched.is_empty()
    ));

    // 5. Ambiguous module across two dependency packages
    let ambig_err = build_graph_files(
        &[
            (
                "/app/src/main.lpp",
                "import util\ndef main():\n    return\n",
            ),
            ("/deps/p1/util.lpp", "def u1():\n    return\n"),
            ("/deps/p2/util.lpp", "def u2():\n    return\n"),
        ],
        &[("p1", "/deps/p1"), ("p2", "/deps/p2")],
    )
    .expect_err("ambiguous dependency module must fail");
    let GraphError::AmbiguousModule { matches, .. } = ambig_err else {
        panic!("expected AmbiguousModule");
    };
    assert_eq!(matches.len(), 2);

    // 6. Invalid package name & duplicate package name
    let inv_err = build_graph_files(
        &[("/app/src/main.lpp", "def main():\n    return\n")],
        &[("", "/deps/empty")],
    )
    .expect_err("empty dependency package name must fail");
    assert!(matches!(inv_err, GraphError::InvalidPackageName));

    let dup_err = build_graph_files(
        &[("/app/src/main.lpp", "def main():\n    return\n")],
        &[("dup", "/deps/a"), ("dup", "/deps/b")],
    )
    .expect_err("duplicate dependency name must fail");
    assert!(matches!(dup_err, GraphError::DuplicatePackage(name) if name == "dup"));
}

// ============================================================================
// Section 2: Declarations, Generics, Traits, Impls, Externs & TypeRef Trees
// ============================================================================

#[test]
fn lowers_structs_enums_traits_impls_externs_consts_and_type_aliases() {
    let source = concat!(
        "@packed\n",
        "@align(64)\n",
        "@section(\".rodata\")\n",
        "pub struct Packet[T: Eq]:\n",
        "    id: Int = 1\n",
        "    payload: T\n",
        "\n",
        "pub enum Option[T]:\n",
        "    Some(value: T)\n",
        "    None\n",
        "\n",
        "pub enum Result[T, E]:\n",
        "    Ok(val: T)\n",
        "    Err(err: E)\n",
        "\n",
        "pub trait Serializer:\n",
        "    def write_byte(self, b: Int) -> Bool\n",
        "    def flush(self)\n",
        "\n",
        "impl[T: Eq] Packet[T]:\n",
        "    def get_id(self) -> Int:\n",
        "        return self.id\n",
        "\n",
        "impl[T: Eq] Serializer for Packet[T]:\n",
        "    def write_byte(self, b: Int) -> Bool:\n",
        "        return true\n",
        "    def flush(self):\n",
        "        return\n",
        "\n",
        "pub extern \"C\" link \"sqlite3\":\n",
        "    def sqlite3_open(path: Str, handle: RawPtr) -> Int\n",
        "    def sqlite3_close(handle: RawPtr) -> Int\n",
        "\n",
        "pub const MAX_FRAMES = 4096\n",
        "pub type FrameId = Int\n",
    );
    let hir = lower_single(source);

    // Struct item & attributes & field default
    let pkt_item = find_item(&hir, "Packet");
    assert!(pkt_item.public);
    let attrs: Vec<&str> = hir
        .symbols(pkt_item.attributes)
        .iter()
        .map(|s| sym_str(&hir, *s))
        .collect();
    assert_eq!(attrs, ["packed", "align(64)", "section(.rodata)"]);
    let pkt = find_struct(&hir, "Packet");
    let pkt_params = hir.type_parameters(pkt.type_parameters);
    assert_eq!(pkt_params.len(), 1);
    assert_eq!(sym_str(&hir, hir.type_parameters[pkt_params[0]].name), "T");
    assert_eq!(
        flatten_type_ref(&hir, hir.type_parameters[pkt_params[0]].bound.unwrap()),
        "Eq"
    );
    let pkt_fields = hir.fields(pkt.fields);
    assert_eq!(pkt_fields.len(), 2);
    assert!(hir.fields[pkt_fields[0]].default.is_some());
    assert!(hir.fields[pkt_fields[1]].default.is_none());

    // Enums
    let opt = find_enum(&hir, "Option");
    let opt_variants = hir.variants(opt.variants);
    assert_eq!(opt_variants.len(), 2);
    assert_eq!(hir.fields(hir.variants[opt_variants[0]].fields).len(), 1);
    assert_eq!(hir.fields(hir.variants[opt_variants[1]].fields).len(), 0);

    let res = find_enum(&hir, "Result");
    assert_eq!(hir.type_parameters(res.type_parameters).len(), 2);
    assert_eq!(hir.variants(res.variants).len(), 2);

    // Trait signature-only methods
    let tr = find_trait(&hir, "Serializer");
    let tr_methods = hir.items(tr.methods);
    assert_eq!(tr_methods.len(), 2);
    for &m in tr_methods {
        let HirItemKind::Function(f) = hir.items[m].kind else {
            panic!("expected trait method function");
        };
        assert!(f.body.is_none(), "trait method must be signature-only");
    }

    // Impls propagate referenced impl type parameters (`T`) to nested methods
    let impls = find_impls(&hir);
    assert_eq!(impls.len(), 2);
    assert!(impls[0].trait_ref.is_none());
    assert!(impls[1].trait_ref.is_some());
    for imp in &impls {
        for &m in hir.items(imp.methods) {
            let HirItemKind::Function(f) = hir.items[m].kind else {
                panic!("expected impl method");
            };
            let m_tparams = hir.type_parameters(f.type_parameters);
            assert_eq!(m_tparams.len(), 1);
            assert_eq!(sym_str(&hir, hir.type_parameters[m_tparams[0]].name), "T");
        }
    }

    // Extern block and top-level DefIds for extern functions
    let externs = find_externs(&hir);
    assert_eq!(externs.len(), 1);
    assert!(externs[0].link_library.is_some());
    assert_eq!(hir.items(externs[0].functions).len(), 2);
    assert!(matches!(
        hir.names
            .resolve(hir.names.modules[0].module, "sqlite3_open"),
        Some(BindingTarget::Definition(_))
    ));
    assert!(matches!(
        hir.names
            .resolve(hir.names.modules[0].module, "sqlite3_close"),
        Some(BindingTarget::Definition(_))
    ));

    // Const & TypeAlias
    assert!(matches!(
        find_item(&hir, "MAX_FRAMES").kind,
        HirItemKind::Const { .. }
    ));
    assert!(matches!(
        find_item(&hir, "FrameId").kind,
        HirItemKind::TypeAlias { .. }
    ));
}

#[test]
fn extern_functions_can_be_selectively_imported_across_modules() {
    let files = [
        (
            "/app/src/main.lpp",
            "from ffi import c_strlen\ndef main() -> Int:\n    return c_strlen(\"abc\")\n",
        ),
        (
            "/app/src/ffi.lpp",
            "pub extern \"C\":\n    def c_strlen(s: Str) -> Int\n",
        ),
    ];
    let (graph, hir) = lower_files(&files, &[], ResolutionMode::Namespaced);
    let binding = hir.names.resolve(graph.entry, "c_strlen");
    assert!(matches!(binding, Some(BindingTarget::Definition(_))));
    let calls: Vec<_> = hir
        .expressions
        .iter()
        .filter_map(|e| match e.kind {
            ExpressionKind::Call { callee, .. } => hir.expression_definition(callee),
            _ => None,
        })
        .collect();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        hir.names.definitions[calls[0]].kind,
        DefinitionKind::Function
    );
}

#[test]
fn lowers_all_collection_pointer_and_nested_tuple_type_refs() {
    let source = concat!(
        "struct Complex[T]:\n",
        "    p: Ptr[T]\n",
        "    raw: RawPtr\n",
        "    items: List[T]\n",
        "    table: Map[Str, List[(Int, Option[Ptr[T]])]]\n",
        "    unique: Set[Str]\n",
        "    queue: Deque[Result[T, Int]]\n",
        "    pair16: (Int, Int, Int, Int, Int, Int, Int, Int, Int, Int, Int, Int, Int, Int, Int, Int)\n",
    );
    let hir = lower_single(source);
    let s = find_struct(&hir, "Complex");
    let fields: Vec<(&str, String)> = hir
        .fields(s.fields)
        .iter()
        .map(|&fid| {
            let f = hir.fields[fid];
            (sym_str(&hir, f.name), flatten_type_ref(&hir, f.type_ref))
        })
        .collect();
    assert_eq!(
        fields,
        [
            ("p", "Ptr[T]".to_owned()),
            ("raw", "RawPtr".to_owned()),
            ("items", "List[T]".to_owned()),
            ("table", "Map[Str, List[(Int, Option[Ptr[T]])]]".to_owned()),
            ("unique", "Set[Str]".to_owned()),
            ("queue", "Deque[Result[T, Int]]".to_owned()),
            (
                "pair16",
                "(Int, Int, Int, Int, Int, Int, Int, Int, Int, Int, Int, Int, Int, Int, Int, Int)"
                    .to_owned()
            ),
        ]
    );
}

#[test]
fn impl_only_propagates_referenced_type_parameters_to_methods() {
    let source = concat!(
        "struct Wrapper[T]:\n",
        "    val: T\n",
        "\n",
        "impl[T, Unused] Wrapper[T]:\n",
        "    def get(self) -> T:\n",
        "        return self.val\n",
    );
    let hir = lower_single(source);
    let imps = find_impls(&hir);
    let method_id = hir.items(imps[0].methods)[0];
    let HirItemKind::Function(method) = hir.items[method_id].kind else {
        panic!("expected method");
    };
    let tparams: Vec<&str> = hir
        .type_parameters(method.type_parameters)
        .iter()
        .map(|&tp| sym_str(&hir, hir.type_parameters[tp].name))
        .collect();
    assert_eq!(tparams, ["T"]);
}

#[test]
fn trait_parameter_desugaring_handles_multiple_traits_and_explicit_generics() {
    let source = concat!(
        "trait Read:\n",
        "    def read(self) -> Int\n",
        "trait Write:\n",
        "    def write(self, v: Int)\n",
        "def pipe[K](key: K, src: Read, dst: Write) -> Int:\n",
        "    v := src.read()\n",
        "    dst.write(v)\n",
        "    return v\n",
    );
    let hir = lower_single(source);
    let pipe = find_fn(&hir, "pipe");
    let tparams: Vec<(&str, Option<String>)> = hir
        .type_parameters(pipe.type_parameters)
        .iter()
        .map(|&tp| {
            let p = hir.type_parameters[tp];
            (
                sym_str(&hir, p.name),
                p.bound.map(|b| flatten_type_ref(&hir, b)),
            )
        })
        .collect();
    assert_eq!(
        tparams,
        [
            ("K", None),
            ("$impl0", Some("Read".to_owned())),
            ("$impl1", Some("Write".to_owned())),
        ]
    );
    let param_types: Vec<String> = hir
        .locals(pipe.parameters)
        .iter()
        .map(|&lid| flatten_type_ref(&hir, hir.locals[lid].type_ref.unwrap()))
        .collect();
    assert_eq!(param_types, ["K", "$impl0", "$impl1"]);
}

#[test]
fn generic_calls_and_index_expressions_disambiguate_cleanly() {
    let source = concat!(
        "def id[T](x: T) -> T:\n",
        "    return x\n",
        "def pair[A, B](a: A, b: B) -> (A, B):\n",
        "    return (a, b)\n",
        "def main(arr: List[Int]) -> Int:\n",
        "    a := id::<Int>(10)\n",
        "    b := id::[Int](20)\n",
        "    c := id[Int](30)\n",
        "    d := pair[Int, List[Int]](40, arr)\n",
        "    e := arr[0]\n",
        "    return a + b + c + d.0 + e\n",
    );
    let hir = lower_single(source);
    let generic_calls = hir
        .expressions
        .iter()
        .filter(|e| matches!(e.kind, ExpressionKind::GenericCall { .. }))
        .count();
    assert_eq!(generic_calls, 4);
    let index_exprs = hir
        .expressions
        .iter()
        .filter(|e| matches!(e.kind, ExpressionKind::Index { .. }))
        .count();
    // `d.0` and `arr[0]` both lower to `ExpressionKind::Index`.
    assert_eq!(index_exprs, 2);
}

// ============================================================================
// Section 3: Operator Precedence, Closures, Defaults, Variadics & Shadowing
// ============================================================================

#[test]
fn operator_precedence_tree_respects_all_levels() {
    let source = concat!(
        "def prec(a: Int, b: Int, c: Int, d: Bool) -> Bool:\n",
        "    x := a + b * c - a / b % c\n",
        "    y := a < b + 1 and -c * 2 == -6 or not d\n",
        "    z := (a as Float) + 1.5\n",
        "    return y\n",
    );
    let hir = lower_single(source);
    let f = find_fn(&hir, "prec");
    let stmts = body_stmts(&hir, f.body.unwrap());

    // `x := a + b * c - a / b % c`
    // Precedence: `* / %` (4) > `+ -` (3), left-associative within same level.
    let StatementKind::Let { value: x_val, .. } = hir.statements[stmts[0]].kind else {
        panic!("expected Let for x");
    };
    let ExpressionKind::Binary {
        left: x_left,
        operator: BinaryOperator::Subtract,
        right: x_right,
    } = hir.expressions[x_val].kind
    else {
        panic!("outermost operator of `a + b * c - a / b % c` must be Subtract");
    };
    assert!(matches!(
        hir.expressions[x_left].kind,
        ExpressionKind::Binary {
            operator: BinaryOperator::Add,
            ..
        }
    ));
    assert!(matches!(
        hir.expressions[x_right].kind,
        ExpressionKind::Binary {
            operator: BinaryOperator::Modulo,
            ..
        }
    ));

    // `y := a < b + 1 and -c * 2 == -6 or not d`
    // Precedence: unary (>) > `*` (4) > `+` (3) > `< ==` (2) > `and or` (1, left-assoc).
    let StatementKind::Let { value: y_val, .. } = hir.statements[stmts[1]].kind else {
        panic!("expected Let for y");
    };
    let ExpressionKind::Binary {
        left: y_left,
        operator: BinaryOperator::LogicalOr,
        right: y_right,
    } = hir.expressions[y_val].kind
    else {
        panic!("outermost operator of y must be LogicalOr");
    };
    assert!(matches!(
        hir.expressions[y_left].kind,
        ExpressionKind::Binary {
            operator: BinaryOperator::LogicalAnd,
            ..
        }
    ));
    assert!(matches!(
        hir.expressions[y_right].kind,
        ExpressionKind::Unary {
            operator: UnaryOperator::Not,
            ..
        }
    ));

    // `z := (a as Float) + 1.5`
    let StatementKind::Let { value: z_val, .. } = hir.statements[stmts[2]].kind else {
        panic!("expected Let for z");
    };
    let ExpressionKind::Binary {
        left: z_left,
        operator: BinaryOperator::Add,
        ..
    } = hir.expressions[z_val].kind
    else {
        panic!("expected Add");
    };
    assert!(matches!(
        hir.expressions[z_left].kind,
        ExpressionKind::Cast { .. }
    ));
}

#[test]
fn postfix_chains_try_await_field_index_call_and_spawn_lower_accurately() {
    let source = concat!(
        "async def worker(x: Int) -> Int:\n",
        "    return x\n",
        "def run(items: List[Int]) -> Int:\n",
        "    t := spawn worker(items[0])\n",
        "    r := t.await?\n",
        "    return r\n",
    );
    let hir = lower_single(source);
    let run = find_fn(&hir, "run");
    let stmts = body_stmts(&hir, run.body.unwrap());

    let StatementKind::Let { value: t_val, .. } = hir.statements[stmts[0]].kind else {
        panic!("expected Let");
    };
    assert!(matches!(
        hir.expressions[t_val].kind,
        ExpressionKind::Spawn(_)
    ));

    let StatementKind::Let { value: r_val, .. } = hir.statements[stmts[1]].kind else {
        panic!("expected Let");
    };
    let ExpressionKind::Try(inner) = hir.expressions[r_val].kind else {
        panic!("expected Try wrapping Await");
    };
    assert!(matches!(
        hir.expressions[inner].kind,
        ExpressionKind::Await(_)
    ));
}

#[test]
fn closures_inline_and_multiline_block_lower_with_correct_scopes_and_origins() {
    let source = concat!(
        "def make(offset: Int) -> Int:\n",
        "    inline := fn(x: Int) -> Int: x + offset\n",
        "    block := fn(x, y: Int) -> Int:\n",
        "        mut acc := x + y\n",
        "        acc += offset\n",
        "        return acc\n",
        "    return inline(1) + block(2, 3)\n",
    );
    let hir = lower_single(source);
    let make = find_fn(&hir, "make");
    let stmts = body_stmts(&hir, make.body.unwrap());

    // Inline closure wraps expression in an implicit Return with DesugaringKind::ImplicitClosureReturn
    let StatementKind::Let { value: c1, .. } = hir.statements[stmts[0]].kind else {
        panic!("expected Let for inline closure");
    };
    let ExpressionKind::Closure {
        parameters: p1,
        return_type: rt1,
        body: b1,
    } = hir.expressions[c1].kind
    else {
        panic!("expected Closure");
    };
    assert_eq!(hir.locals(p1).len(), 1);
    assert!(rt1.is_some());
    let b1_stmts = body_stmts(&hir, b1);
    assert_eq!(b1_stmts.len(), 1);
    assert!(matches!(
        hir.statements[b1_stmts[0]].kind,
        StatementKind::Return(Some(_))
    ));
    assert!(matches!(
        hir.origins[hir.statements[b1_stmts[0]].origin].kind,
        OriginKind::Desugared(DesugaringKind::ImplicitClosureReturn)
    ));

    // Multiline closure preserves all statements and resolves captured `offset` to outer parameter
    let StatementKind::Let { value: c2, .. } = hir.statements[stmts[1]].kind else {
        panic!("expected Let for block closure");
    };
    let ExpressionKind::Closure {
        parameters: p2,
        body: b2,
        ..
    } = hir.expressions[c2].kind
    else {
        panic!("expected Closure");
    };
    assert_eq!(hir.locals(p2).len(), 2);
    assert!(hir.locals[hir.locals(p2)[0]].type_ref.is_none());
    assert!(hir.locals[hir.locals(p2)[1]].type_ref.is_some());
    assert_eq!(body_stmts(&hir, b2).len(), 3);
    assert_eq!(
        hir.scopes[hir.bodies[b2].scope].parent,
        Some(hir.bodies[make.body.unwrap()].scope)
    );
}

#[test]
fn default_parameters_and_variadic_rest_parameters_lower_faithfully() {
    let source = concat!(
        "def connect(host: Str = \"localhost\", port: Int = 8000 + 80, ...tags: Str) -> Int:\n",
        "    return port\n",
    );
    let hir = lower_single(source);
    let f = find_fn(&hir, "connect");
    assert!(f.variadic);
    let params = hir.locals(f.parameters);
    assert_eq!(params.len(), 3);
    assert_eq!(hir.locals[params[0]].kind, LocalKind::Parameter);
    assert!(hir.locals[params[0]].default.is_some());
    assert!(hir.locals[params[1]].default.is_some());
    assert!(matches!(
        hir.expressions[hir.locals[params[1]].default.unwrap()].kind,
        ExpressionKind::Binary {
            operator: BinaryOperator::Add,
            ..
        }
    ));
    assert!(hir.locals[params[2]].default.is_none());
}

#[test]
fn tuple_destructuring_declares_mutable_and_immutable_locals_up_to_16() {
    let source = concat!(
        "def destruct(p: (Int, Int), big: (Int, Int, Int, Int, Int, Int, Int, Int, Int, Int, Int, Int, Int, Int, Int, Int)) -> Int:\n",
        "    (a, b) := p\n",
        "    mut (c, d) := p\n",
        "    (x1, x2, x3, x4, x5, x6, x7, x8, x9, x10, x11, x12, x13, x14, x15, x16) := big\n",
        "    c = d + x16\n",
        "    return a + b + c\n",
    );
    let hir = lower_single(source);
    let f = find_fn(&hir, "destruct");
    let stmts = body_stmts(&hir, f.body.unwrap());

    let StatementKind::Let {
        bindings: b_imm, ..
    } = hir.statements[stmts[0]].kind
    else {
        panic!("expected Let");
    };
    assert_eq!(hir.locals(b_imm).len(), 2);
    assert!(!hir.locals[hir.locals(b_imm)[0]].mutable);

    let StatementKind::Let {
        bindings: b_mut, ..
    } = hir.statements[stmts[1]].kind
    else {
        panic!("expected Let");
    };
    assert_eq!(hir.locals(b_mut).len(), 2);
    assert!(hir.locals[hir.locals(b_mut)[0]].mutable);
    assert!(hir.locals[hir.locals(b_mut)[1]].mutable);

    let StatementKind::Let { bindings: b_16, .. } = hir.statements[stmts[2]].kind else {
        panic!("expected Let");
    };
    assert_eq!(hir.locals(b_16).len(), 16);
}

#[test]
fn nested_scope_shadowing_keeps_outer_local_intact_after_inner_scope_exits() {
    let source = concat!(
        "def shadow(flag: Bool) -> Int:\n",
        "    x := 10\n",
        "    if flag:\n",
        "        x := 20\n",
        "        y := x\n",
        "    return x\n",
    );
    let hir = lower_single(source);
    let f = find_fn(&hir, "shadow");
    let stmts = body_stmts(&hir, f.body.unwrap());

    let StatementKind::Let {
        bindings: outer_b, ..
    } = hir.statements[stmts[0]].kind
    else {
        panic!("expected outer Let");
    };
    let outer_x = hir.locals(outer_b)[0];

    let StatementKind::If { then_body, .. } = hir.statements[stmts[1]].kind else {
        panic!("expected If");
    };
    let inner_stmts = body_stmts(&hir, then_body);
    let StatementKind::Let {
        bindings: inner_b, ..
    } = hir.statements[inner_stmts[0]].kind
    else {
        panic!("expected inner Let");
    };
    let inner_x = hir.locals(inner_b)[0];
    assert_ne!(outer_x, inner_x);

    let StatementKind::Let { value: y_val, .. } = hir.statements[inner_stmts[1]].kind else {
        panic!("expected Let y");
    };
    assert!(matches!(
        hir.expressions[y_val].kind,
        ExpressionKind::Name {
            binding: NameBinding::Local(id),
            ..
        } if id == inner_x
    ));

    let StatementKind::Return(Some(ret_expr)) = hir.statements[stmts[2]].kind else {
        panic!("expected Return");
    };
    assert!(matches!(
        hir.expressions[ret_expr].kind,
        ExpressionKind::Name {
            binding: NameBinding::Local(id),
            ..
        } if id == outer_x
    ));
}

// ============================================================================
// Section 4: F-Strings, Match Desugaring, Control Flow & Unsafe Tracking
// ============================================================================

#[test]
fn fstring_interpolation_desugars_into_left_folded_string_concatenation() {
    let source = concat!(
        "def greet(name: Str, count: Int) -> Str:\n",
        "    empty := f\"\"\n",
        "    lead := f\"{count} items\"\n",
        "    esc := f\"brace=\\{ok\\}, user={name}, next={count + 1}\"\n",
        "    nested := f\"\"\"pair={fmt(f\"({count})\", \"{\")}\"\"\"\n",
        "    return empty + lead + esc + nested\n",
    );
    let hir = lower_single(source);
    let greet = find_fn(&hir, "greet");
    let stmts = body_stmts(&hir, greet.body.unwrap());

    // 1. `f""` lowers to empty formatted String literal
    let StatementKind::Let { value: empty_v, .. } = hir.statements[stmts[0]].kind else {
        panic!("expected Let empty");
    };
    assert!(matches!(
        hir.expressions[empty_v].kind,
        ExpressionKind::Literal(Literal::String {
            formatted: true,
            ..
        })
    ));

    // 2. `f"{count} items"` -> `"" + count + " items"`
    let StatementKind::Let { value: lead_v, .. } = hir.statements[stmts[1]].kind else {
        panic!("expected Let lead");
    };
    let mut parts = Vec::new();
    flatten_fstring_concat(&hir, lead_v, &mut parts);
    assert_eq!(parts.len(), 3);
    assert!(matches!(
        hir.expressions[parts[0]].kind,
        ExpressionKind::Literal(Literal::String {
            formatted: true,
            ..
        })
    ));
    assert!(matches!(
        hir.expressions[parts[1]].kind,
        ExpressionKind::Name {
            binding: NameBinding::Local(_),
            ..
        }
    ));
    assert!(matches!(
        hir.expressions[parts[2]].kind,
        ExpressionKind::Literal(Literal::String {
            formatted: true,
            ..
        })
    ));

    // 3. `f"brace=\{ok\}, user={name}, next={count + 1}"` -> `"" + "brace=\{ok\}, user=" + name + ", next=" + (count + 1)`
    let StatementKind::Let { value: esc_v, .. } = hir.statements[stmts[2]].kind else {
        panic!("expected Let esc");
    };
    let mut esc_parts = Vec::new();
    flatten_fstring_concat(&hir, esc_v, &mut esc_parts);
    assert_eq!(esc_parts.len(), 5);
    assert!(matches!(
        hir.expressions[esc_parts[4]].kind,
        ExpressionKind::Binary {
            operator: BinaryOperator::Add,
            ..
        }
    ));

    // 4. Nested f-string inside triple-quoted f-string parses inner interpolation recursively
    let StatementKind::Let {
        value: nested_v, ..
    } = hir.statements[stmts[3]].kind
    else {
        panic!("expected Let nested");
    };
    let mut nested_parts = Vec::new();
    flatten_fstring_concat(&hir, nested_v, &mut nested_parts);
    assert_eq!(nested_parts.len(), 4);
    assert!(matches!(
        hir.expressions[nested_parts[2]].kind,
        ExpressionKind::Call { .. }
    ));
}

#[test]
fn pure_enum_match_with_or_patterns_flattens_into_match_arms() {
    let source = concat!(
        "enum Color:\n",
        "    Red\n",
        "    Green\n",
        "    Blue\n",
        "    Rgb(r: Int, g: Int, b: Int)\n",
        "def score(c: Color) -> Int:\n",
        "    match c:\n",
        "        Color.Red | Color.Green:\n",
        "            return 1\n",
        "        Color.Rgb(r, g, b):\n",
        "            return r + g + b\n",
        "        _:\n",
        "            return 0\n",
    );
    let hir = lower_single(source);
    let score = find_fn(&hir, "score");
    let stmts = body_stmts(&hir, score.body.unwrap());
    assert_eq!(stmts.len(), 1);
    let StatementKind::Match { arms, .. } = hir.statements[stmts[0]].kind else {
        panic!("expected StatementKind::Match");
    };
    let arm_ids = hir.match_arms(arms);
    assert_eq!(
        arm_ids.len(),
        4,
        "OR-pattern `Red | Green` expands to 2 arms"
    );
    assert!(!hir.match_arms[arm_ids[0]].wildcard);
    assert!(!hir.match_arms[arm_ids[1]].wildcard);
    assert_eq!(hir.locals(hir.match_arms[arm_ids[2]].bindings).len(), 3);
    assert!(hir.match_arms[arm_ids[3]].wildcard);
}

#[test]
fn guarded_enum_match_builds_variant_clause_chain_with_canonical_aliases_and_wildcards() {
    let source = concat!(
        "enum Msg:\n",
        "    Num(v: Int)\n",
        "    Empty\n",
        "def handle(m: Msg, urgent: Bool) -> Int:\n",
        "    match m:\n",
        "        _ if urgent:\n",
        "            return 999\n",
        "        Msg.Num(a) if a > 10:\n",
        "            return a\n",
        "        Msg.Num(b) if b > 0:\n",
        "            return b * 2\n",
        "        Msg.Num(c):\n",
        "            return c\n",
        "        _:\n",
        "            return -1\n",
    );
    let hir = lower_single(source);
    let handle = find_fn(&hir, "handle");
    let stmts = body_stmts(&hir, handle.body.unwrap());
    assert_eq!(stmts.len(), 1);
    let StatementKind::Match { arms, .. } = hir.statements[stmts[0]].kind else {
        panic!("expected guarded enum StatementKind::Match");
    };
    let arm_ids = hir.match_arms(arms);
    // Two buckets/arms: `Msg.Num` and wildcard `_`.
    assert_eq!(arm_ids.len(), 2);

    // First arm is `Msg.Num` with canonical binding `a`
    let num_arm = hir.match_arms[arm_ids[0]];
    assert!(!num_arm.wildcard);
    let num_bindings = hir.locals(num_arm.bindings);
    assert_eq!(num_bindings.len(), 1);
    assert_eq!(sym_str(&hir, hir.locals[num_bindings[0]].name), "a");

    // Inside `num_arm.body`, all statements include the earlier `_ if urgent:` guard,
    // then `a > 10`, then alias `b = a` + `b > 0`, then alias `c = a`, then wildcard fallback!
    let mut nested = Vec::new();
    collect_all_stmts_in_body(&hir, num_arm.body, &mut nested);
    let alias_names: Vec<&str> = nested
        .iter()
        .filter_map(|s| match s.kind {
            StatementKind::Let { bindings, .. } => {
                Some(sym_str(&hir, hir.locals[hir.locals(bindings)[0]].name))
            }
            _ => None,
        })
        .collect();
    assert_eq!(alias_names, ["a", "b", "c"]);

    // Second arm is wildcard `_` which has `if urgent: 999 else: -1`.
    let wild_arm = hir.match_arms[arm_ids[1]];
    assert!(wild_arm.wildcard);
    let wild_stmts = body_stmts(&hir, wild_arm.body);
    assert!(matches!(
        hir.statements[wild_stmts[0]].kind,
        StatementKind::If {
            else_body: Some(_),
            ..
        }
    ));
}

#[test]
fn value_match_desugars_literals_ranges_or_patterns_binders_and_guards() {
    let source = concat!(
        "def classify(x: Int, ready: Bool) -> Int:\n",
        "    match x:\n",
        "        -5:\n",
        "            return 1\n",
        "        0 | 1 | 2:\n",
        "            return 2\n",
        "        3..10:\n",
        "            return 3\n",
        "        10..=20 if ready:\n",
        "            return 4\n",
        "        n if n > 100:\n",
        "            return n\n",
        "        _ if ready:\n",
        "            return 5\n",
        "        other:\n",
        "            return other\n",
    );
    let hir = lower_single(source);
    let classify = find_fn(&hir, "classify");
    let stmts = body_stmts(&hir, classify.body.unwrap());

    // First statement binds `__match_subj_...`, second is the top-level If decision tree.
    assert_eq!(stmts.len(), 2);
    let StatementKind::Let { bindings, .. } = hir.statements[stmts[0]].kind else {
        panic!("expected subject let binding");
    };
    let subj_name = sym_str(&hir, hir.locals[hir.locals(bindings)[0]].name);
    assert!(subj_name.starts_with("__match_subj_"));

    let mut all_stmts = Vec::new();
    collect_all_stmts_in_body(&hir, classify.body.unwrap(), &mut all_stmts);

    // Check that `n` and `other` binder locals were declared as `LocalKind::Pattern`.
    let pattern_locals: Vec<&str> = hir
        .locals
        .iter()
        .filter(|l| l.kind == LocalKind::Pattern)
        .map(|l| sym_str(&hir, l.name))
        .collect();
    assert!(pattern_locals.contains(&"n"));
    assert!(pattern_locals.contains(&"other"));

    // Check that exclusive range `3..10` uses `<` and inclusive range `10..=20` uses `<=`.
    assert!(hir.expressions.iter().any(|e| matches!(
        e.kind,
        ExpressionKind::Binary {
            operator: BinaryOperator::Less,
            ..
        }
    )));
    assert!(hir.expressions.iter().any(|e| matches!(
        e.kind,
        ExpressionKind::Binary {
            operator: BinaryOperator::LessEqual,
            ..
        }
    )));
    // Check that `0 | 1 | 2` produced LogicalOr chain.
    assert!(hir.expressions.iter().any(|e| matches!(
        e.kind,
        ExpressionKind::Binary {
            operator: BinaryOperator::LogicalOr,
            ..
        }
    )));
    // Check that `-5` pattern produced Unary::Negate.
    assert!(hir.expressions.iter().any(|e| matches!(
        e.kind,
        ExpressionKind::Unary {
            operator: UnaryOperator::Negate,
            ..
        }
    )));
}

#[test]
fn value_match_supports_char_bool_and_string_literal_patterns() {
    let source = concat!(
        "def check(c: Char, b: Bool, s: Str) -> Int:\n",
        "    mut acc := 0\n",
        "    match c:\n",
        "        'a'..'z':\n",
        "            acc += 1\n",
        "        _:\n",
        "            acc += 0\n",
        "    match b:\n",
        "        true:\n",
        "            acc += 10\n",
        "        false:\n",
        "            acc += 20\n",
        "    match s:\n",
        "        \"ok\" | \"yes\":\n",
        "            acc += 100\n",
        "        _:\n",
        "            acc += 0\n",
        "    return acc\n",
    );
    let hir = lower_single(source);
    let check = find_fn(&hir, "check");
    let mut all = Vec::new();
    collect_all_stmts_in_body(&hir, check.body.unwrap(), &mut all);
    // 3 value matches -> 3 `__match_subj_` bindings.
    let subj_count = all
        .iter()
        .filter(|s| matches!(
            s.kind,
            StatementKind::Let { bindings, .. }
            if sym_str(&hir, hir.locals[hir.locals(bindings)[0]].name).starts_with("__match_subj_")
        ))
        .count();
    assert_eq!(subj_count, 3);
}

#[test]
fn for_ranges_elif_chains_and_augmented_assignments_preserve_desugaring_origins() {
    let source = concat!(
        "def loops(n: Int) -> Int:\n",
        "    mut total := 0\n",
        "    for i in 0..n:\n",
        "        total += i\n",
        "    for j in 1..=n:\n",
        "        total *= 2\n",
        "        total -= 1\n",
        "        total /= 1\n",
        "        total %= 97\n",
        "    if total == 0:\n",
        "        return 1\n",
        "    elif total < 10:\n",
        "        return 2\n",
        "    elif total < 20:\n",
        "        return 3\n",
        "    else:\n",
        "        return total\n",
    );
    let hir = lower_single(source);
    let loops = find_fn(&hir, "loops");
    let stmts = body_stmts(&hir, loops.body.unwrap());

    // `0..n` -> `range(0, n)` ; `1..=n` -> `range(1, n + 1)`
    let StatementKind::For { iterable: it1, .. } = hir.statements[stmts[1]].kind else {
        panic!("expected For 1");
    };
    let ExpressionKind::Call {
        callee: r1,
        arguments: args1,
    } = hir.expressions[it1].kind
    else {
        panic!("expected range call 1");
    };
    assert!(matches!(
        hir.expressions[r1].kind,
        ExpressionKind::Name { symbol, .. } if sym_str(&hir, symbol) == "range"
    ));
    assert_eq!(hir.expressions(args1).len(), 2);

    let StatementKind::For { iterable: it2, .. } = hir.statements[stmts[2]].kind else {
        panic!("expected For 2");
    };
    let ExpressionKind::Call {
        arguments: args2, ..
    } = hir.expressions[it2].kind
    else {
        panic!("expected range call 2");
    };
    let hi2 = hir.expressions(args2)[1];
    assert!(matches!(
        hir.expressions[hi2].kind,
        ExpressionKind::Binary {
            operator: BinaryOperator::Add,
            ..
        }
    ));

    // 5 augmented assignments (`+=`, `*=`, `-=`, `/=`, `%=`) all carry `DesugaringKind::AugmentedAssignment`
    let aug_ops: Vec<BinaryOperator> = hir
        .expressions
        .iter()
        .filter_map(|expr| match (expr.kind, hir.origins[expr.origin].kind) {
            (
                ExpressionKind::Binary { operator, .. },
                OriginKind::Desugared(DesugaringKind::AugmentedAssignment),
            ) => Some(operator),
            _ => None,
        })
        .collect();
    assert_eq!(
        aug_ops,
        [
            BinaryOperator::Add,
            BinaryOperator::Multiply,
            BinaryOperator::Subtract,
            BinaryOperator::Divide,
            BinaryOperator::Modulo,
        ]
    );

    // 2 `elif` branches carry `DesugaringKind::Elif` with `OriginKind::Source` parents
    let elif_count = hir
        .statements
        .iter()
        .filter(|stmt| {
            let origin = hir.origins[stmt.origin];
            matches!(origin.kind, OriginKind::Desugared(DesugaringKind::Elif))
                && origin
                    .parent
                    .is_some_and(|p| matches!(hir.origins[p].kind, OriginKind::Source))
        })
        .count();
    assert_eq!(elif_count, 2);
}

#[test]
fn unsafe_functions_and_unsafe_blocks_track_flags_scopes_and_source_origins() {
    let source = concat!(
        "unsafe async def dma_transfer(addr: RawPtr) -> Int:\n",
        "    return 1\n",
        "\n",
        "def safe_wrapper() -> Int:\n",
        "    outer := 10\n",
        "    unsafe:\n",
        "        inner := outer + 5\n",
        "        return inner\n",
        "    return outer\n",
    );
    let hir = lower_single(source);

    let dma = find_fn(&hir, "dma_transfer");
    assert!(dma.is_unsafe);
    assert!(dma.is_async);

    let safe = find_fn(&hir, "safe_wrapper");
    assert!(!safe.is_unsafe);
    assert!(!safe.is_async);

    let safe_body = safe.body.unwrap();
    let stmts = body_stmts(&hir, safe_body);
    let StatementKind::Unsafe { body: unsafe_body } = hir.statements[stmts[1]].kind else {
        panic!("expected StatementKind::Unsafe");
    };
    let unsafe_origin = hir.origins[hir.statements[stmts[1]].origin];
    assert_eq!(unsafe_origin.kind, OriginKind::Source);
    assert_eq!(
        &source[unsafe_origin.span.start as usize..unsafe_origin.span.end as usize],
        "unsafe:\n        inner := outer + 5\n        return inner"
    );
    assert_eq!(
        hir.scopes[hir.bodies[unsafe_body].scope].parent,
        Some(hir.bodies[safe_body].scope)
    );
}

#[test]
fn hir_snapshot_is_deterministic_across_repeated_graph_and_hir_builds() {
    let files = [
        (
            "/app/src/main.lpp",
            concat!(
                "import lib.math as m\n",
                "def main() -> Int:\n",
                "    msg := f\"sum={m.add(1, 2)}\"\n",
                "    return m.add(3, 4)\n",
            ),
        ),
        (
            "/app/src/lib/math.lpp",
            "pub def add(a: Int, b: Int = 0) -> Int:\n    return a + b\n",
        ),
    ];
    let (_, hir1) = lower_files(&files, &[], ResolutionMode::Namespaced);
    let (_, hir2) = lower_files(&files, &[], ResolutionMode::Namespaced);
    assert_eq!(hir_snapshot(&hir1), hir_snapshot(&hir2));
}

// ============================================================================
// Section 5: Diagnostic Error Matrix (E1xxx via GraphBuilder, E3xxx in HIR)
// ============================================================================

#[test]
fn graph_builder_surfaces_e1xxx_frontend_errors_on_entry_and_dependencies() {
    let entry_err = build_graph_files(&[("/app/src/main.lpp", "def bad() {\n}\n")], &[])
        .expect_err("brace syntax in entry must surface Frontend error");
    let GraphError::Frontend { path, diagnostics } = entry_err else {
        panic!("expected GraphError::Frontend");
    };
    assert_eq!(path, Path::new("/app/src/main.lpp"));
    assert_eq!(diagnostics[0].code.as_str(), "E1001");

    let dep_err = build_graph_files(
        &[
            (
                "/app/src/main.lpp",
                "import broken\ndef main():\n    return\n",
            ),
            ("/app/src/broken.lpp", "def bad(\n    return\n"),
        ],
        &[],
    )
    .expect_err("unclosed paren in transitive dep must surface Frontend error");
    let GraphError::Frontend { path, diagnostics } = dep_err else {
        panic!("expected GraphError::Frontend");
    };
    assert_eq!(path, Path::new("/app/src/broken.lpp"));
    assert_eq!(diagnostics[0].code.as_str(), "E1101");
}

#[test]
fn name_resolution_diagnoses_e3001_e3002_e3003_and_e3004() {
    // E3001: duplicate definition in same module (including extern fn collision)
    let dup_graph = build_graph_files(
        &[(
            "/app/src/main.lpp",
            "def f() -> Int:\n    return 1\ndef f() -> Int:\n    return 2\n",
        )],
        &[],
    )
    .unwrap();
    let e3001 = build_name_index(&dup_graph, ResolutionMode::Namespaced).unwrap_err();
    assert_eq!(e3001[0].code.as_str(), "E3001");

    let extern_dup_graph = build_graph_files(
        &[(
            "/app/src/main.lpp",
            "def puts(s: Str) -> Int:\n    return 0\nextern \"C\":\n    def puts(s: Str) -> Int\n",
        )],
        &[],
    )
    .unwrap();
    let e3001_ext = build_name_index(&extern_dup_graph, ResolutionMode::Namespaced).unwrap_err();
    assert_eq!(e3001_ext[0].code.as_str(), "E3001");

    // E3002: missing selective import symbol
    let missing_sel_graph = build_graph_files(
        &[
            (
                "/app/src/main.lpp",
                "from dep import nope\ndef main():\n    return\n",
            ),
            ("/app/src/dep.lpp", "pub def yes():\n    return\n"),
        ],
        &[],
    )
    .unwrap();
    let e3002 = build_name_index(&missing_sel_graph, ResolutionMode::Namespaced).unwrap_err();
    assert_eq!(e3002[0].code.as_str(), "E3002");

    // E3003: import name conflicts with local definition or second import
    let conflict_local_graph = build_graph_files(
        &[
            ("/app/src/main.lpp", "import dep\ndef dep():\n    return\n"),
            ("/app/src/dep.lpp", "pub def x():\n    return\n"),
        ],
        &[],
    )
    .unwrap();
    let e3003_local =
        build_name_index(&conflict_local_graph, ResolutionMode::Namespaced).unwrap_err();
    assert_eq!(e3003_local[0].code.as_str(), "E3003");

    let conflict_imports_graph = build_graph_files(
        &[
            (
                "/app/src/main.lpp",
                "import a.shared\nimport b.shared\ndef main():\n    return\n",
            ),
            ("/app/src/a/shared.lpp", "pub def one():\n    return\n"),
            ("/app/src/b/shared.lpp", "pub def two():\n    return\n"),
        ],
        &[],
    )
    .unwrap();
    let e3003_imp =
        build_name_index(&conflict_imports_graph, ResolutionMode::Namespaced).unwrap_err();
    assert_eq!(e3003_imp[0].code.as_str(), "E3003");

    // E3004: LegacyFlat duplicate across modules
    let flat_dup_graph = build_graph_files(
        &[
            (
                "/app/src/main.lpp",
                "import other\ndef helper():\n    return\n",
            ),
            ("/app/src/other.lpp", "pub def helper():\n    return\n"),
        ],
        &[],
    )
    .unwrap();
    let e3004 = build_name_index(&flat_dup_graph, ResolutionMode::LegacyFlat).unwrap_err();
    assert_eq!(e3004[0].code.as_str(), "E3004");
    assert!(!e3004[0].notes.is_empty());
}

#[test]
fn hir_lowering_diagnoses_e3100_malformed_constructs() {
    let cases = [
        // Empty tuple expression
        "def f():\n    x := ()\n",
        // Missing parameter type
        "def f(x):\n    return\n",
        // Variadic parameter with default
        "def f(...args: Int = 1):\n    return\n",
        // Tuple destructuring with 1 name
        "def f(p: (Int, Int)):\n    (a) := p\n",
        // Missing operand after binary operator
        "def f():\n    x := 1 +\n",
        // Missing field name after '.'
        "def f(p: Int):\n    x := p.\n",
        // Tuple type with 1 element (missing comma)
        "type Single = (Int)\n",
        // `mut` without binding initializer
        "def f():\n    mut x = 1\n",
        // Empty closure without body
        "def f():\n    c := (fn(x: Int) -> Int:)\n",
        // Unclosed / trailing tokens in expression
        "def f():\n    x := 1 2\n",
        // Variant pattern mixed into scalar match after literal
        "def f(x: Int):\n    match x:\n        1:\n            return\n        Opt.Some(v):\n            return\n",
        // Inconsistent variant arity across guarded enum match arms
        "enum E:\n    Pair(a: Int, b: Int)\ndef f(e: E, flag: Bool):\n    match e:\n        E.Pair(a, b) if flag:\n            return\n        E.Pair(only_one):\n            return\n",
    ];
    for (idx, src) in cases.iter().enumerate() {
        let diags = lower_single_err(src);
        assert_eq!(
            diags[0].code.as_str(),
            "E3100",
            "E3100 case #{idx} failed for:\n{src}\ngot: {diags:#?}"
        );
    }
}

#[test]
fn hir_lowering_diagnoses_e3101_e3102_e3103_e3104_and_e3105() {
    // E3101: declaration inside function body
    assert_eq!(
        lower_single_err("def f():\n    struct Inner:\n        x: Int\n")[0]
            .code
            .as_str(),
        "E3101"
    );
    // E3101: trait method with a body
    assert_eq!(
        lower_single_err("trait Bad:\n    def m(self):\n        return\n")[0]
            .code
            .as_str(),
        "E3101"
    );
    // E3101: extern function with a body
    assert_eq!(
        lower_single_err("extern \"C\":\n    def c_fn():\n        return\n")[0]
            .code
            .as_str(),
        "E3101"
    );

    // E3102: non-place assignment targets
    for bad_assign in [
        "def f():\n    1 = 2\n",
        "def f(a: Int, b: Int):\n    (a + b) = 3\n",
        "def f():\n    foo() = 4\n",
    ] {
        assert_eq!(
            lower_single_err(bad_assign)[0].code.as_str(),
            "E3102",
            "expected E3102 for:\n{bad_assign}"
        );
    }

    // E3103: orphaned elif / else / match arm
    for bad_branch in [
        "def f():\n    elif true:\n        return\n",
        "def f():\n    else:\n        return\n",
        "def f(x: Int):\n    match x:\n        if true:\n            return\n",
    ] {
        assert_eq!(
            lower_single_err(bad_branch)[0].code.as_str(),
            "E3103",
            "expected E3103 for:\n{bad_branch}"
        );
    }

    // E3104: unterminated interpolation inside f-string
    assert_eq!(
        lower_single_err("def f(x: Int):\n    s := f\"hello {x\"\n")[0]
            .code
            .as_str(),
        "E3104"
    );
    // E3104: unterminated nested brace inside f-string interpolation
    assert_eq!(
        lower_single_err("def f(x: Int):\n    s := f\"hello {x + {1}\"\n")[0]
            .code
            .as_str(),
        "E3104"
    );

    // E3105: empty interpolation in f-string
    assert_eq!(
        lower_single_err("def f():\n    s := f\"value: {   }\"\n")[0]
            .code
            .as_str(),
        "E3105"
    );
    // E3105: lexically invalid interpolation inside f-string
    assert_eq!(
        lower_single_err("def f():\n    s := f\"value: {$x}\"\n")[0]
            .code
            .as_str(),
        "E3105"
    );
}

// ============================================================================
// Section 6: End-to-End Realistic Multi-Module Package Lowering & Invariants
// ============================================================================

#[test]
fn end_to_end_multi_module_kernel_and_app_package_lowers_with_full_arena_integrity() {
    let files = [
        (
            "/app/src/main.lpp",
            concat!(
                "import hal.mmio as mmio\n",
                "from hal.types import PhysAddr, Device, Status\n",
                "from std.ds import Ring\n",
                "\n",
                "@inline\n",
                "@no_mangle\n",
                "pub unsafe def kernel_main(dev: Device, q: Ring[Int]) -> Int:\n",
                "    mut status := Status.Ready(0)\n",
                "    unsafe:\n",
                "        val := mmio.read32(dev.base)\n",
                "        status = Status.Ready(val)\n",
                "    match status:\n",
                "        Status.Ready(code) if code > 0:\n",
                "            return code\n",
                "        Status.Ready(code):\n",
                "            return code + 1\n",
                "        Status.Error(msg):\n",
                "            log := f\"err={msg}\"\n",
                "            return -1\n",
                "        _:\n",
                "            return 0\n",
            ),
        ),
        (
            "/app/src/hal/mmio.lpp",
            concat!(
                "from hal.types import PhysAddr\n",
                "pub unsafe def read32(addr: PhysAddr) -> Int:\n",
                "    return 42\n",
            ),
        ),
        (
            "/app/src/hal/types.lpp",
            concat!(
                "pub type PhysAddr = RawPtr\n",
                "pub struct Device:\n",
                "    base: PhysAddr\n",
                "    irq: Int = 14\n",
                "pub enum Status:\n",
                "    Ready(code: Int)\n",
                "    Error(msg: Str)\n",
                "    Idle\n",
            ),
        ),
        (
            "/deps/std/ds.lpp",
            concat!(
                "pub struct Ring[T]:\n",
                "    buf: Deque[T]\n",
                "    cap: Int = 16\n",
            ),
        ),
    ];
    let (graph, hir) = lower_files(&files, &[("std", "/deps/std")], ResolutionMode::Namespaced);
    assert_eq!(graph.modules.len(), 4);
    assert_eq!(hir.modules.len(), 4);

    // Every item's origin span file matches its owning module's FileId.
    for item in hir.items.iter() {
        let module = graph.module(item.module).expect("valid ModuleId");
        assert_eq!(hir.origins[item.origin].span.file, module.file);
    }

    // Verify all expression spans fall within their owning source file length.
    for expr in hir.expressions.iter() {
        let span = hir.origins[expr.origin].span;
        let file = graph.sources.file(span.file).expect("valid FileId");
        assert!(file.validate_span(span).is_ok());
    }
    for stmt in hir.statements.iter() {
        let span = hir.origins[stmt.origin].span;
        let file = graph.sources.file(span.file).expect("valid FileId");
        assert!(file.validate_span(span).is_ok());
    }
}
