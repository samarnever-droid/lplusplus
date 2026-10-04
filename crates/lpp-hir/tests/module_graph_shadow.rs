use std::path::Path;

use lpp_hir::{
    BindingTarget, ExpressionKind, GraphBuilder, GraphError, GraphRequest, NameBinding,
    OsFileSystem, PackageSpec, ResolutionMode, build_name_index, lower_package,
};

fn repository_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("HIR crate must live under <root>/crates")
}

#[test]
fn legacy_multi_file_fixture_builds_as_a_real_graph() {
    let root = repository_root();
    let source_root = root.join("tests/modules");
    let request = GraphRequest::new(
        source_root.join("test_full_project.lpp"),
        PackageSpec::new("module-fixture", &source_root),
    );
    let graph = GraphBuilder::new(&OsFileSystem)
        .build(request.clone())
        .unwrap();
    let repeated = GraphBuilder::new(&OsFileSystem).build(request).unwrap();

    assert_eq!(graph.modules.len(), 4);
    assert_eq!(graph.edges.len(), 3);
    for module in &graph.modules {
        let source = graph.sources.file(module.file).unwrap();
        assert!(std::ptr::eq(source.text(), module.syntax.source.as_ref()));
    }
    assert_eq!(
        graph
            .modules
            .iter()
            .map(|module| module.path.clone())
            .collect::<Vec<_>>(),
        repeated
            .modules
            .iter()
            .map(|module| module.path.clone())
            .collect::<Vec<_>>()
    );
    assert!(graph.modules.iter().any(|module| {
        module
            .path
            .ends_with(Path::new("tests/modules/utils/helpers.lpp"))
    }));

    let namespaced = build_name_index(&graph, ResolutionMode::Namespaced).unwrap();
    assert!(namespaced.resolve(graph.entry, "add").is_none());
    let legacy = build_name_index(&graph, ResolutionMode::LegacyFlat).unwrap();
    assert!(matches!(
        legacy.resolve(graph.entry, "add"),
        Some(BindingTarget::Definition(_))
    ));

    let hir = lower_package(&graph, ResolutionMode::LegacyFlat).unwrap();
    let repeated_hir = lower_package(&repeated, ResolutionMode::LegacyFlat).unwrap();
    assert_eq!(hir, repeated_hir);
    let add = hir.names.symbols.get("add").unwrap();
    assert!(hir.expressions.iter().any(|expression| matches!(
        expression.kind,
        ExpressionKind::Name {
            symbol,
            binding: NameBinding::Item(BindingTarget::Definition(_)),
        } if symbol == add
    )));
}

#[test]
fn entry_graph_surfaces_syntax_debt_in_a_transitive_module() {
    let root = repository_root();
    let source_root = root.join("pm/src");
    let request = GraphRequest::new(
        source_root.join("main.lpp"),
        PackageSpec::new("lpp-pm", &source_root),
    )
    .with_dependency(PackageSpec::new("stdlib", root.join("stdlib")));
    let error = GraphBuilder::new(&OsFileSystem)
        .build(request)
        .expect_err("the package manager publisher module still contains brace syntax");

    let GraphError::Frontend { path, diagnostics } = error else {
        panic!("expected a transitive frontend error, got {error}");
    };
    assert!(path.ends_with(Path::new("pm/src/publisher.lpp")));
    assert_eq!(diagnostics[0].code.as_str(), "E1001");
}
