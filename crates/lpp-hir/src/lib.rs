#![forbid(unsafe_code)]

mod arena;
mod fs;
mod graph;
mod ids;
mod interner;
mod ir;
mod list;
mod lower;
mod resolve;
mod snapshot;

pub use arena::{Arena, ArenaExhausted};
pub use fs::{FileSystem, FileSystemError, OsFileSystem};
pub use graph::{
    GraphBuilder, GraphError, GraphRequest, ImportEdge, Module, ModuleId, Package, PackageGraph,
    PackageId, PackageSpec,
};
pub use ids::{
    ArenaId, BodyId, DefId, ExprId, FieldId, HirItemId, InstanceId, LocalId, MatchArmId, OriginId,
    ScopeId, StmtId, Symbol, TypeParamId, TypeRefId, VariantId,
};
pub use interner::{InternerExhausted, StringInterner};
pub use ir::{
    BinaryOperator, Body, DesugaringKind, Enum, Expression, ExpressionKind, ExternBlock, Field,
    Function, HirItem, HirItemKind, HirModule, HirPackage, Impl, Literal, Local, LocalKind,
    MatchArm, NameBinding, Origin, OriginKind, Scope, Statement, StatementKind, Struct, Trait,
    TypeParameter, TypeRef, TypeRefKind, UnaryOperator, Variant,
};
pub use list::{IdList, IdListExhausted, IdRange};
pub use lower::lower_package;
pub use resolve::{
    BindingTarget, Definition, DefinitionKind, ModuleScope, NameIndex, ResolutionMode,
    build_name_index,
};
pub use snapshot::hir_snapshot;

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use lpp_frontend::ImportKind;

    use super::*;

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
                self.files
                    .keys()
                    .find(|path| path.to_string_lossy().replace('\\', "/").to_lowercase() == requested_lower)
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
            if self.files.keys().any(|file| file.starts_with(path)) {
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

    fn request() -> GraphRequest {
        GraphRequest::new("/app/src/main.lpp", PackageSpec::new("app", "/app/src"))
    }

    #[test]
    fn builds_stable_graph_with_aliases_and_selective_imports() {
        let filesystem = MemoryFileSystem::with_files(&[
            (
                "/app/src/main.lpp",
                "import b as bee\nfrom a import value\ndef main():\n    return value()\n",
            ),
            ("/app/src/a.lpp", "def value() -> Int:\n    return 1\n"),
            ("/app/src/b.lpp", "def other() -> Int:\n    return 2\n"),
        ]);
        let graph = GraphBuilder::new(&filesystem).build(request()).unwrap();

        assert_eq!(graph.modules.len(), 3);
        assert_eq!(graph.modules[0].path, Path::new("/app/src/a.lpp"));
        assert_eq!(graph.modules[1].path, Path::new("/app/src/b.lpp"));
        assert_eq!(graph.modules[2].path, Path::new("/app/src/main.lpp"));
        assert_eq!(graph.entry.raw(), 2);
        assert_eq!(graph.modules[0].file.raw(), graph.modules[0].id.raw());
        assert!(graph.edges.iter().any(|edge| {
            edge.path.components() == ["b"]
                && edge.kind
                    == ImportKind::Module {
                        alias: Some("bee".to_owned()),
                    }
        }));
        assert!(graph.edges.iter().any(|edge| {
            edge.path.components() == ["a"]
                && edge.kind
                    == ImportKind::Selective {
                        names: vec!["value".to_owned()],
                    }
        }));
    }

    #[test]
    fn package_and_module_order_does_not_depend_on_dependency_input_order() {
        let filesystem = MemoryFileSystem::with_files(&[
            (
                "/app/src/main.lpp",
                "import beta.tool\nimport alpha.tool\ndef main():\n    return\n",
            ),
            ("/deps/alpha/tool.lpp", "def alpha_tool():\n    return\n"),
            ("/deps/beta/tool.lpp", "def beta_tool():\n    return\n"),
        ]);
        let left = request()
            .with_dependency(PackageSpec::new("beta", "/deps/beta"))
            .with_dependency(PackageSpec::new("alpha", "/deps/alpha"));
        let right = request()
            .with_dependency(PackageSpec::new("alpha", "/deps/alpha"))
            .with_dependency(PackageSpec::new("beta", "/deps/beta"));

        let left = GraphBuilder::new(&filesystem).build(left).unwrap();
        let right = GraphBuilder::new(&filesystem).build(right).unwrap();
        assert_eq!(
            left.packages
                .iter()
                .map(|package| (&package.name, package.id))
                .collect::<Vec<_>>(),
            right
                .packages
                .iter()
                .map(|package| (&package.name, package.id))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            left.modules
                .iter()
                .map(|module| &module.path)
                .collect::<Vec<_>>(),
            right
                .modules
                .iter()
                .map(|module| &module.path)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn reports_an_explicit_deterministic_cycle() {
        let filesystem = MemoryFileSystem::with_files(&[
            ("/app/src/main.lpp", "import a\ndef main():\n    return\n"),
            ("/app/src/a.lpp", "import b\ndef a():\n    return\n"),
            ("/app/src/b.lpp", "import a\ndef b():\n    return\n"),
        ]);
        let error = GraphBuilder::new(&filesystem)
            .build(request())
            .expect_err("cycle must fail graph construction");
        let GraphError::ImportCycle(paths) = error else {
            panic!("expected import cycle");
        };
        assert_eq!(
            paths,
            [
                PathBuf::from("/app/src/a.lpp"),
                PathBuf::from("/app/src/b.lpp"),
                PathBuf::from("/app/src/a.lpp"),
            ]
        );
    }

    #[test]
    fn enforces_case_sensitive_imports_on_case_insensitive_filesystems() {
        let filesystem = MemoryFileSystem::with_files(&[
            (
                "/app/src/main.lpp",
                "import Util\ndef main():\n    return\n",
            ),
            ("/app/src/util.lpp", "def utility():\n    return\n"),
        ])
        .case_insensitive();
        let error = GraphBuilder::new(&filesystem)
            .build(request())
            .expect_err("wrong-case import must fail");
        assert!(matches!(error, GraphError::CaseMismatch { .. }));
    }

    #[test]
    fn preserves_importer_and_search_evidence_for_missing_modules() {
        let filesystem = MemoryFileSystem::with_files(&[(
            "/app/src/main.lpp",
            "import absent\ndef main():\n    return\n",
        )]);
        let injected: &dyn FileSystem = &filesystem;
        let error = GraphBuilder::new(injected)
            .build(request())
            .expect_err("missing import must fail");
        let GraphError::MissingModule {
            importer,
            module,
            searched,
        } = error
        else {
            panic!("expected missing module error");
        };
        assert_eq!(importer, Path::new("/app/src/main.lpp"));
        assert_eq!(module.components(), ["absent"]);
        assert_eq!(searched, [PathBuf::from("/app/src/absent.lpp")]);
    }

    #[test]
    fn compact_ids_arenas_and_interning_do_not_duplicate_names() {
        assert_eq!(std::mem::size_of::<DefId>(), 4);
        assert_eq!(std::mem::size_of::<Option<DefId>>(), 4);
        assert_eq!(std::mem::size_of::<Symbol>(), 4);
        assert!(DefId::from_index(u32::MAX as usize).is_none());

        let mut interner = StringInterner::new();
        let first = interner.intern("shared_name").unwrap();
        let second = interner.intern("shared_name").unwrap();
        assert_eq!(first, second);
        assert_eq!(interner.len(), 1);

        let mut arena = Arena::<DefId, u64>::new();
        let id = arena.alloc(41).unwrap();
        arena[id] += 1;
        assert_eq!(arena[id], 42);
    }

    #[test]
    fn name_index_uses_module_namespaces_and_resolves_import_bindings() {
        let filesystem = MemoryFileSystem::with_files(&[
            (
                "/app/src/main.lpp",
                "import b as bee\nfrom a import shared\ndef main():\n    bee.shared()\n    return shared()\n",
            ),
            ("/app/src/a.lpp", "def shared() -> Int:\n    return 1\n"),
            ("/app/src/b.lpp", "def shared() -> Int:\n    return 2\n"),
        ]);
        let graph = GraphBuilder::new(&filesystem).build(request()).unwrap();
        let index = build_name_index(&graph, ResolutionMode::Namespaced).unwrap();

        let main = graph.entry;
        assert!(matches!(
            index.resolve(main, "bee"),
            Some(BindingTarget::Module(_))
        ));
        assert!(matches!(
            index.resolve(main, "shared"),
            Some(BindingTarget::Definition(_))
        ));
        assert_eq!(index.definitions.len(), 3);
        let hir = lower_package(&graph, ResolutionMode::Namespaced).unwrap();
        let shared = hir.names.symbols.get("shared").unwrap();
        assert!(hir.expressions.iter().any(|expression| matches!(
            expression.kind,
            ExpressionKind::Name {
                symbol,
                binding: NameBinding::Item(BindingTarget::Definition(_)),
            } if symbol == shared
        )));
        let bee = hir.names.symbols.get("bee").unwrap();
        assert!(hir.expressions.iter().any(|expression| matches!(
            expression.kind,
            ExpressionKind::Name {
                symbol,
                binding: NameBinding::Item(BindingTarget::Module(_)),
            } if symbol == bee
        )));

        let diagnostics = build_name_index(&graph, ResolutionMode::LegacyFlat)
            .expect_err("the edition-1 flat namespace must report duplicate definitions");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.as_str() == "E3004")
        );
    }

    #[test]
    fn selective_imports_require_a_definition_in_the_target_module() {
        let filesystem = MemoryFileSystem::with_files(&[
            (
                "/app/src/main.lpp",
                "from a import missing\ndef main():\n    return\n",
            ),
            ("/app/src/a.lpp", "def present():\n    return\n"),
        ]);
        let graph = GraphBuilder::new(&filesystem).build(request()).unwrap();
        let diagnostics = build_name_index(&graph, ResolutionMode::Namespaced)
            .expect_err("missing selective name must be diagnosed");
        assert_eq!(diagnostics[0].code.as_str(), "E3002");
    }

    #[test]
    fn lowers_declarations_and_bodies_into_compact_origin_preserving_hir() {
        let source = concat!(
            "@inline\n",
            "struct Pair[T]:\n",
            "    left: T\n",
            "    right: T\n",
            "enum Result[T]:\n",
            "    Ok(value: T)\n",
            "    Err(code: Int)\n",
            "trait Show:\n",
            "    def show(self) -> Str\n",
            "impl Show for Pair[Int]:\n",
            "    def show(self) -> Str:\n",
            "        return \"pair\"\n",
            "extern \"C\":\n",
            "    def abs(value: Int) -> Int\n",
            "const ONE = 1\n",
            "type Number = Int\n",
            "async def compute[T: Show](input: T, delta: Int = 1) -> Int:\n",
            "    mut total: Int = delta\n",
            "    total += 2\n",
            "    if total > 2:\n",
            "        total = input.value\n",
            "    elif total == 2:\n",
            "        total = 3\n",
            "    else:\n",
            "        total = 4\n",
            "    for item in [1, 2]:\n",
            "        total += item\n",
            "    match Result.Ok(total):\n",
            "        Result.Ok(value):\n",
            "            total = value\n",
            "        _:\n",
            "            total = 0\n",
            "    callback := fn(x: Int) -> Int: x + total\n",
            "    return callback(total)\n",
        );
        let filesystem = MemoryFileSystem::with_files(&[("/app/src/main.lpp", source)]);
        let graph = GraphBuilder::new(&filesystem).build(request()).unwrap();
        let hir = lower_package(&graph, ResolutionMode::Namespaced).unwrap();

        assert_eq!(hir.modules.len(), 1);
        assert_eq!(hir.items(hir.modules[0].items).len(), 8);
        assert_eq!(hir.fields.len(), 4);
        assert_eq!(hir.variants.len(), 2);
        assert_eq!(hir.type_parameters.len(), 3);
        assert!(hir.expressions.len() > 25);
        assert!(hir.statements.len() > 12);
        assert_eq!(std::mem::size_of::<ExprId>(), 4);
        assert_eq!(std::mem::size_of::<Option<ExprId>>(), 4);
        assert_eq!(std::mem::size_of::<IdRange<ExprId>>(), 8);
        assert_eq!(std::mem::size_of::<Expression>(), 32);
        assert_eq!(std::mem::size_of::<Statement>(), 20);
        assert_eq!(std::mem::size_of::<Origin>(), 20);
        assert_eq!(std::mem::size_of::<Local>(), 24);

        let local_names = hir
            .locals
            .iter()
            .map(|local| hir.names.symbols.resolve(local.name).unwrap())
            .collect::<Vec<_>>();
        assert!(local_names.contains(&"input"));
        assert!(local_names.contains(&"total"));
        assert!(local_names.contains(&"value"));
        assert!(hir.expressions.iter().any(|expression| matches!(
            expression.kind,
            ExpressionKind::Name {
                binding: NameBinding::Local(_),
                ..
            }
        )));

        let augmented = hir
            .expressions
            .iter()
            .find(|expression| {
                matches!(
                    expression.kind,
                    ExpressionKind::Binary {
                        operator: BinaryOperator::Add,
                        ..
                    }
                ) && matches!(
                    hir.origins[expression.origin].kind,
                    OriginKind::Desugared(DesugaringKind::AugmentedAssignment)
                )
            })
            .expect("augmented assignment must retain a desugaring origin");
        let augmented_origin = hir.origins[augmented.origin];
        assert!(augmented_origin.parent.is_some());
        assert!(matches!(
            hir.origins[augmented_origin.parent.unwrap()].kind,
            OriginKind::Source
        ));
        assert!(hir.statements.iter().any(|statement| matches!(
            hir.origins[statement.origin].kind,
            OriginKind::Desugared(DesugaringKind::Elif)
        )));
    }

    #[test]
    fn same_scope_shadowing_allocates_distinct_locals_and_resolves_latest() {
        let filesystem = MemoryFileSystem::with_files(&[(
            "/app/src/main.lpp",
            "def main():\n    value := 1\n    value := 2\n    return value\n",
        )]);
        let graph = GraphBuilder::new(&filesystem).build(request()).unwrap();
        let hir = lower_package(&graph, ResolutionMode::Namespaced).unwrap();
        let value = hir.names.symbols.get("value").unwrap();
        let locals = hir
            .locals
            .enumerate()
            .filter_map(|(id, local)| (local.name == value).then_some(id))
            .collect::<Vec<_>>();
        assert_eq!(locals.len(), 2);
        assert_ne!(locals[0], locals[1]);
        assert!(hir.expressions.iter().any(|expression| {
            matches!(
                expression.kind,
                ExpressionKind::Name {
                    symbol,
                    binding: NameBinding::Local(local),
                } if symbol == value && local == locals[1]
            )
        }));
    }

    #[test]
    fn rejects_non_place_assignment_during_hir_lowering() {
        let filesystem =
            MemoryFileSystem::with_files(&[("/app/src/main.lpp", "def main():\n    1 = 2\n")]);
        let graph = GraphBuilder::new(&filesystem).build(request()).unwrap();
        let diagnostics = lower_package(&graph, ResolutionMode::Namespaced)
            .expect_err("literal assignment target must be rejected");
        assert_eq!(diagnostics[0].code.as_str(), "E3102");
    }
}
