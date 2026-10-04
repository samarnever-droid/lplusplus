use std::collections::BTreeMap;

use lpp_common::{Diagnostic, Span};
use lpp_frontend::{ImportKind, ItemKind};

use crate::arena::{Arena, ArenaExhausted};
use crate::graph::{ModuleId, PackageGraph};
use crate::ids::{DefId, Symbol};
use crate::interner::{InternerExhausted, StringInterner};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionMode {
    Namespaced,
    LegacyFlat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefinitionKind {
    Function,
    Struct,
    Enum,
    Trait,
    Const,
    TypeAlias,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Definition {
    pub module: ModuleId,
    pub name: Symbol,
    pub kind: DefinitionKind,
    pub public: bool,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingTarget {
    Definition(DefId),
    Module(ModuleId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleScope {
    pub module: ModuleId,
    definitions: BTreeMap<Symbol, DefId>,
    imports: BTreeMap<Symbol, BindingTarget>,
}

impl ModuleScope {
    #[must_use]
    pub fn definition(&self, name: Symbol) -> Option<DefId> {
        self.definitions.get(&name).copied()
    }

    #[must_use]
    pub fn imported(&self, name: Symbol) -> Option<BindingTarget> {
        self.imports.get(&name).copied()
    }

    pub fn definitions(&self) -> impl ExactSizeIterator<Item = (Symbol, DefId)> + '_ {
        self.definitions.iter().map(|(name, id)| (*name, *id))
    }

    pub fn imports(&self) -> impl ExactSizeIterator<Item = (Symbol, BindingTarget)> + '_ {
        self.imports.iter().map(|(name, target)| (*name, *target))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameIndex {
    pub symbols: StringInterner,
    pub definitions: Arena<DefId, Definition>,
    pub modules: Vec<ModuleScope>,
    legacy_flat: Option<BTreeMap<Symbol, DefId>>,
}

impl NameIndex {
    #[must_use]
    pub fn resolve(&self, module: ModuleId, name: &str) -> Option<BindingTarget> {
        self.resolve_symbol(module, self.symbols.get(name)?)
    }

    #[must_use]
    pub fn resolve_symbol(&self, module: ModuleId, symbol: Symbol) -> Option<BindingTarget> {
        let scope = self.modules.get(module.raw() as usize)?;
        scope
            .definition(symbol)
            .map(BindingTarget::Definition)
            .or_else(|| scope.imported(symbol))
            .or_else(|| {
                self.legacy_flat
                    .as_ref()?
                    .get(&symbol)
                    .copied()
                    .map(BindingTarget::Definition)
            })
    }
}

pub fn build_name_index(
    graph: &PackageGraph,
    mode: ResolutionMode,
) -> Result<NameIndex, Vec<Diagnostic>> {
    let expected_definitions = graph
        .modules
        .iter()
        .map(|module| module.syntax.items.len())
        .sum();
    let mut symbols = StringInterner::with_capacity(expected_definitions);
    let mut definitions = Arena::with_capacity(expected_definitions);
    let mut modules = graph
        .modules
        .iter()
        .map(|module| ModuleScope {
            module: module.id,
            definitions: BTreeMap::new(),
            imports: BTreeMap::new(),
        })
        .collect::<Vec<_>>();
    let mut diagnostics = Vec::new();

    for module in &graph.modules {
        let scope = &mut modules[module.id.raw() as usize];
        for item in &module.syntax.items {
            let Some(kind) = definition_kind(item.kind) else {
                continue;
            };
            let Some(name) = item.name.as_deref() else {
                continue;
            };
            let symbol = match symbols.intern(name) {
                Ok(symbol) => symbol,
                Err(error) => return Err(vec![capacity_diagnostic(error, item.span)]),
            };
            let definition = Definition {
                module: module.id,
                name: symbol,
                kind,
                public: item.public,
                span: item.span,
            };
            let id = match definitions.alloc(definition) {
                Ok(id) => id,
                Err(error) => return Err(vec![arena_diagnostic(error, item.span)]),
            };
            if scope.definitions.insert(symbol, id).is_some() {
                diagnostics.push(diagnostic(
                    "E3001",
                    format!("duplicate definition '{name}' in one module"),
                    item.span,
                ));
            }
        }
    }

    for edge in &graph.edges {
        match &edge.kind {
            ImportKind::Module { alias } => {
                let local_name = alias
                    .as_deref()
                    .or_else(|| edge.path.components().last().map(String::as_str))
                    .expect("module paths are nonempty");
                let symbol = match symbols.intern(local_name) {
                    Ok(symbol) => symbol,
                    Err(error) => return Err(vec![capacity_diagnostic(error, edge.span)]),
                };
                let scope = &mut modules[edge.importer.raw() as usize];
                insert_import(
                    scope,
                    symbol,
                    BindingTarget::Module(edge.imported),
                    local_name,
                    edge.span,
                    &mut diagnostics,
                );
            }
            ImportKind::Selective { names } => {
                for name in names {
                    let symbol = match symbols.intern(name) {
                        Ok(symbol) => symbol,
                        Err(error) => return Err(vec![capacity_diagnostic(error, edge.span)]),
                    };
                    let target_scope = &modules[edge.imported.raw() as usize];
                    let Some(target) = target_scope.definition(symbol) else {
                        diagnostics.push(diagnostic(
                            "E3002",
                            format!(
                                "module '{}' does not define '{name}'",
                                edge.path.components().join(".")
                            ),
                            edge.span,
                        ));
                        continue;
                    };
                    let scope = &mut modules[edge.importer.raw() as usize];
                    insert_import(
                        scope,
                        symbol,
                        BindingTarget::Definition(target),
                        name,
                        edge.span,
                        &mut diagnostics,
                    );
                }
            }
        }
    }

    let legacy_flat = if mode == ResolutionMode::LegacyFlat {
        let mut flat = BTreeMap::new();
        for (id, definition) in definitions.enumerate() {
            if let Some(previous) = flat.insert(definition.name, id) {
                let name = symbols
                    .resolve(definition.name)
                    .expect("definition symbols are interned");
                diagnostics.push(
                    diagnostic(
                        "E3004",
                        format!("legacy flat namespace contains duplicate '{name}'"),
                        definition.span,
                    )
                    .with_note(format!(
                        "first definition is in module {}",
                        definitions[previous].module.raw()
                    )),
                );
            }
        }
        Some(flat)
    } else {
        None
    };

    if diagnostics.is_empty() {
        Ok(NameIndex {
            symbols,
            definitions,
            modules,
            legacy_flat,
        })
    } else {
        Err(diagnostics)
    }
}

fn insert_import(
    scope: &mut ModuleScope,
    symbol: Symbol,
    target: BindingTarget,
    name: &str,
    span: Span,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if scope.definitions.contains_key(&symbol) || scope.imports.insert(symbol, target).is_some() {
        diagnostics.push(diagnostic(
            "E3003",
            format!("import binding '{name}' conflicts with another name"),
            span,
        ));
    }
}

const fn definition_kind(kind: ItemKind) -> Option<DefinitionKind> {
    Some(match kind {
        ItemKind::Function => DefinitionKind::Function,
        ItemKind::Struct => DefinitionKind::Struct,
        ItemKind::Enum => DefinitionKind::Enum,
        ItemKind::Trait => DefinitionKind::Trait,
        ItemKind::Const => DefinitionKind::Const,
        ItemKind::TypeAlias => DefinitionKind::TypeAlias,
        ItemKind::Impl | ItemKind::Extern | ItemKind::Import => return None,
    })
}

fn diagnostic(code: &str, message: impl Into<String>, span: Span) -> Diagnostic {
    Diagnostic::error(code, message)
        .expect("HIR diagnostic codes are valid")
        .with_primary_span(span)
}

fn capacity_diagnostic(error: InternerExhausted, span: Span) -> Diagnostic {
    diagnostic("E3099", error.to_string(), span)
}

fn arena_diagnostic(error: ArenaExhausted, span: Span) -> Diagnostic {
    diagnostic("E3099", error.to_string(), span)
}
