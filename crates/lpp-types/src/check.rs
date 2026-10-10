mod body;
mod call;
mod signature;

use std::collections::{BTreeMap, BTreeSet};

use lpp_hir::{
    ArenaId, DefId, ExprId, ExpressionKind, HirItemId, HirItemKind, HirPackage, LocalId, ModuleId,
    OriginId, ScopeId, Symbol, TypeParamId, TypeRefId,
};

use crate::{
    AggregateFacts, BuiltinFacts, BuiltinIndex, EnumFlowFacts, InferenceLevel, InferenceTable,
    InstanceCollectionError, InstanceLimits, InstancePlanner, PlaceFacts, PrimitiveType,
    TraitBuildError, TraitIndex, TraitSolverLimits, TypeError, TypeId, TypeInterner, TypeKind,
    TypeScheme, TypeWorkBudget,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadowInferenceOptions {
    pub work_units: usize,
    pub max_type_depth: usize,
    pub trait_limits: TraitSolverLimits,
    pub instance_limits: InstanceLimits,
}

impl Default for ShadowInferenceOptions {
    fn default() -> Self {
        Self {
            work_units: 1_000_000,
            max_type_depth: TypeWorkBudget::DEFAULT_MAX_DEPTH,
            trait_limits: TraitSolverLimits::default(),
            instance_limits: InstanceLimits::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowTypeError {
    pub origin: OriginId,
    pub error: TypeError,
}

impl std::fmt::Display for ShadowTypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeAssignments {
    expressions: Vec<Option<TypeId>>,
    locals: Vec<Option<TypeId>>,
    type_refs: Vec<Option<TypeId>>,
    items: Vec<Option<TypeId>>,
    local_schemes: BTreeMap<LocalId, TypeScheme>,
}

impl TypeAssignments {
    fn new(package: &HirPackage) -> Self {
        Self {
            expressions: vec![None; package.expressions.len()],
            locals: vec![None; package.locals.len()],
            type_refs: vec![None; package.type_refs.len()],
            items: vec![None; package.items.len()],
            local_schemes: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn expression(&self, id: ExprId) -> Option<TypeId> {
        self.expressions[id.index()]
    }

    #[must_use]
    pub fn local(&self, id: LocalId) -> Option<TypeId> {
        self.locals[id.index()]
    }

    #[must_use]
    pub fn type_ref(&self, id: TypeRefId) -> Option<TypeId> {
        self.type_refs[id.index()]
    }

    #[must_use]
    pub fn item(&self, id: HirItemId) -> Option<TypeId> {
        self.items[id.index()]
    }

    #[must_use]
    pub fn local_scheme(&self, id: LocalId) -> Option<&TypeScheme> {
        self.local_schemes.get(&id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowTypeOutput {
    pub interner: TypeInterner,
    pub inference: InferenceTable,
    pub assignments: TypeAssignments,
    pub aggregates: AggregateFacts,
    pub places: PlaceFacts,
    pub enum_flow: EnumFlowFacts,
    pub builtins: BuiltinFacts,
    pub trait_limits: TraitSolverLimits,
    pub trait_index: TraitIndex,
    pub trait_diagnostics: Vec<TraitBuildError>,
    pub instances: InstancePlanner,
    pub instance_diagnostics: Vec<InstanceCollectionError>,
    /// UFCS/method dispatch table exported for MIR: `(receiver nominal
    /// definition, method name)` → the concrete function item that implements
    /// it. MIR uses this to re-resolve a method call by the *concrete* receiver
    /// type at monomorphization time (e.g. a trait-bounded generic body whose
    /// receiver `self` becomes `Box` in one instance and `Buffer` in another).
    pub method_index: BTreeMap<(DefId, Symbol), HirItemId>,
    pub method_candidates: BTreeMap<(DefId, Symbol), Vec<HirItemId>>,
}

pub fn infer_hir_package(
    package: &HirPackage,
    options: ShadowInferenceOptions,
) -> Result<ShadowTypeOutput, ShadowTypeError> {
    TypeChecker::new(package, options).check()
}

struct TypeChecker<'hir> {
    package: &'hir HirPackage,
    interner: TypeInterner,
    inference: InferenceTable,
    assignments: TypeAssignments,
    aggregates: AggregateFacts,
    places: PlaceFacts,
    enum_flow: EnumFlowFacts,
    builtins: BuiltinFacts,
    builtin_index: BuiltinIndex,
    spawn_closure_scope: Option<ScopeId>,
    current_function_result: Option<TypeId>,
    definition_items: Vec<Option<HirItemId>>,
    call_callees: BTreeSet<ExprId>,
    resolving_aliases: BTreeSet<DefId>,
    /// UFCS method dispatch table: `(receiver nominal definition, method name)`
    /// → the function item that implements it. Populated after signature
    /// registration and covers both free functions whose first parameter is a
    /// nominal type and `impl` block methods.
    method_index: BTreeMap<(DefId, Symbol), HirItemId>,
    method_candidates: BTreeMap<(DefId, Symbol), Vec<HirItemId>>,
    /// Generic type parameters in scope for a UFCS-callable method item. For an
    /// `impl` method these are the implementation's parameters followed by the
    /// method's own; for a free function they are the function's parameters.
    /// Used to instantiate the method's signature with fresh inference
    /// variables before unifying at a call site.
    method_generics: BTreeMap<HirItemId, Vec<TypeParamId>>,
    item_parameters: Vec<Vec<TypeParamId>>,
    function_results: Vec<Option<TypeId>>,
    budget: TypeWorkBudget,
    trait_limits: TraitSolverLimits,
    instance_limits: InstanceLimits,
}

impl<'hir> TypeChecker<'hir> {
    fn new(package: &'hir HirPackage, options: ShadowInferenceOptions) -> Self {
        let call_callees = package
            .expressions
            .iter()
            .filter_map(|expression| match expression.kind {
                ExpressionKind::Call { callee, .. }
                | ExpressionKind::GenericCall { callee, .. } => Some(callee),
                _ => None,
            })
            .collect();
        Self {
            package,
            interner: TypeInterner::new(),
            inference: InferenceTable::new(),
            assignments: TypeAssignments::new(package),
            aggregates: AggregateFacts::new(package),
            places: PlaceFacts::new(package),
            enum_flow: EnumFlowFacts::new(package),
            builtins: BuiltinFacts::new(package.expressions.len()),
            builtin_index: BuiltinIndex::from_generated(),
            spawn_closure_scope: None,
            current_function_result: None,
            definition_items: vec![None; package.names.definitions.len()],
            call_callees,
            resolving_aliases: BTreeSet::new(),
            method_index: BTreeMap::new(),
            method_candidates: BTreeMap::new(),
            method_generics: BTreeMap::new(),
            item_parameters: vec![Vec::new(); package.items.len()],
            function_results: vec![None; package.items.len()],
            budget: TypeWorkBudget::with_max_depth(options.work_units, options.max_type_depth),
            trait_limits: options.trait_limits,
            instance_limits: options.instance_limits,
        }
    }

    fn check(mut self) -> Result<ShadowTypeOutput, ShadowTypeError> {
        for (item_id, item) in self.package.items.enumerate() {
            if let Some(definition) = item.definition {
                self.definition_items[definition.index()] = Some(item_id);
            }
        }
        for (item_id, item) in self.package.items.enumerate() {
            let item = *item;
            self.register_item(item_id, item)?;
        }
        self.resolve_impl_self_types()?;
        self.build_method_index();
        for (item_id, item) in self.package.items.enumerate() {
            self.infer_item_expressions(item_id, *item)?;
        }
        for (item_id, item) in self.package.items.enumerate() {
            self.validate_item_safety(item_id, *item)?;
        }
        self.normalize_assignments()?;
        let (trait_index, trait_diagnostics) =
            TraitIndex::from_hir(self.package, &self.assignments, &self.interner);
        let (instances, instance_diagnostics) = InstancePlanner::collect_hir(
            self.package,
            &self.assignments,
            &self.aggregates,
            &mut self.interner,
            self.instance_limits,
        );
        Ok(ShadowTypeOutput {
            interner: self.interner,
            inference: self.inference,
            assignments: self.assignments,
            aggregates: self.aggregates,
            places: self.places,
            enum_flow: self.enum_flow,
            builtins: self.builtins,
            trait_limits: self.trait_limits,
            trait_index,
            trait_diagnostics,
            instances,
            instance_diagnostics,
            method_index: self.method_index.clone(),
            method_candidates: self.method_candidates.clone(),
        })
    }

    /// Bind `Self` inside `impl` methods to the implementation's target type.
    ///
    /// The frontend gives each method's `self` parameter the placeholder type
    /// `Self`, which name resolution leaves as an unresolved name. This pass
    /// rewrites every occurrence of that placeholder — in the method's
    /// parameters, its result, and each parameter local — to the concrete
    /// target nominal type, so method bodies (e.g. `self.field`) type check and
    /// MIR sees the real receiver type.
    fn resolve_impl_self_types(&mut self) -> Result<(), ShadowTypeError> {
        for (impl_item_id, item) in self.package.items.enumerate() {
            let HirItemKind::Impl(impl_) = item.kind else {
                continue;
            };
            let Some(target) = self.assignments.items[impl_item_id.index()] else {
                continue;
            };
            let methods = self.package.items(impl_.methods).to_vec();
            for method in methods {
                let HirItemKind::Function(function) = self.package.items[method].kind else {
                    continue;
                };
                let locals = self.package.locals(function.parameters).to_vec();
                let Some(first) = locals.first() else {
                    continue;
                };
                // The `Self` placeholder is the (interned) type of the first
                // parameter when it is a `self` receiver; identify it so every
                // identical occurrence can be substituted.
                let Some(self_marker) = self.assignments.locals[first.index()] else {
                    continue;
                };
                if !matches!(
                    self.interner.kind(self_marker),
                    TypeKind::UnresolvedName { .. }
                ) {
                    continue;
                }
                // Rewrite the parameter locals.
                for local in &locals {
                    if self.assignments.locals[local.index()] == Some(self_marker) {
                        self.assignments.locals[local.index()] = Some(target);
                    }
                }
                // Rewrite the method's signature type (parameters + result).
                let Some(method_type) = self.assignments.items[method.index()] else {
                    continue;
                };
                let TypeKind::Function { parameters, result } = self.interner.kind(method_type)
                else {
                    continue;
                };
                let mut param_types = self.interner.list(parameters).to_vec();
                for param in &mut param_types {
                    if *param == self_marker {
                        *param = target;
                    }
                }
                let new_result = if result == self_marker {
                    target
                } else {
                    result
                };
                let origin = self.package.items[method].origin;
                let parameters = self
                    .interner
                    .intern_list(&param_types)
                    .map_err(|error| self.at(origin, error.into()))?;
                let new_type = self
                    .interner
                    .intern(TypeKind::Function {
                        parameters,
                        result: new_result,
                    })
                    .map_err(|error| self.at(origin, error.into()))?;
                self.assignments.items[method.index()] = Some(new_type);
            }
        }
        Ok(())
    }

    /// Returns the nominal definition a (signature-time, inference-free) type
    /// resolves to, if any.
    fn nominal_definition(&self, ty: TypeId) -> Option<DefId> {
        match self.interner.kind(ty) {
            TypeKind::Nominal { definition, .. } => Some(definition),
            _ => None,
        }
    }

    /// Populate `method_index` after signatures are registered. Free functions
    /// whose first parameter is a nominal type become UFCS methods on that
    /// type; `impl` block methods take precedence for the same key.
    fn build_method_index(&mut self) {
        for (item_id, item) in self.package.items.enumerate() {
            let HirItemKind::Function(function) = item.kind else {
                continue;
            };
            // Only top-level (definition-bearing) free functions here; impl
            // methods have `definition: None` and are handled below.
            if item.definition.is_none() {
                continue;
            }
            let Some(name) = item.name else {
                continue;
            };
            let locals = self.package.locals(function.parameters);
            let Some(first) = locals.first() else {
                continue;
            };
            let Some(receiver) = self.assignments.locals[first.index()] else {
                continue;
            };
            if let Some(definition) = self.nominal_definition(receiver) {
                if !self.method_index.contains_key(&(definition, name)) {
                    self.method_index.insert((definition, name), item_id);
                    self.method_candidates
                        .entry((definition, name))
                        .or_default()
                        .push(item_id);
                    self.method_generics
                        .insert(item_id, self.item_parameters[item_id.index()].clone());
                }
            }
        }
        for (item_id, item) in self.package.items.enumerate() {
            let HirItemKind::Impl(impl_) = item.kind else {
                continue;
            };
            let Some(target) = self.assignments.items[item_id.index()] else {
                continue;
            };
            let Some(definition) = self.nominal_definition(target) else {
                continue;
            };
            let methods = self.package.items(impl_.methods).to_vec();
            for method in methods {
                let method_item = &self.package.items[method];
                let Some(name) = method_item.name else {
                    continue;
                };
                self.method_index
                    .entry((definition, name))
                    .or_insert(method);
                self.method_candidates
                    .entry((definition, name))
                    .or_default()
                    .push(method);
                // Impl lowering folds the impl parameters into each method's
                // type-parameter list, so the method's own parameter context is
                // the complete in-scope generic set.
                self.method_generics
                    .insert(method, self.item_parameters[method.index()].clone());
            }
        }
    }

    fn normalize_assignments(&mut self) -> Result<(), ShadowTypeError> {
        let fallback = self
            .package
            .items
            .iter()
            .next()
            .map(|item| item.origin)
            .unwrap_or_else(|| {
                self.package
                    .origins
                    .enumerate()
                    .next()
                    .map(|(id, _)| id)
                    .expect("a lowered HIR package has at least one origin")
            });
        // Opaque handle default. A builtin whose result is the `Any` ABI slot
        // (e.g. `map_new()`) is typed as a fresh inference variable. When no
        // later use pins it — the runtime handle is only ever threaded back
        // through other `Any` slots — the variable is still unbound here. Its
        // machine shape is nonetheless known: an i64 heap handle. Grounding
        // such leftover variables in `Int` lets those programs materialize
        // instead of failing MIR with `GenericTypeMaterialization`. This is
        // safe for already-correct programs: any type that reaches MIR lowering
        // must already be concrete, so a program that compiles today carries no
        // unbound variable for this pass to touch.
        let int_type = self.interner.primitive(PrimitiveType::Int);
        for type_id in self
            .assignments
            .expressions
            .iter_mut()
            .chain(&mut self.assignments.locals)
            .chain(&mut self.assignments.type_refs)
            .chain(&mut self.assignments.items)
            .flatten()
        {
            let normalized = self
                .inference
                .normalize(&mut self.interner, *type_id, &mut self.budget)
                .map_err(|error| ShadowTypeError {
                    origin: fallback,
                    error,
                })?;
            *type_id = if matches!(
                self.interner.kind(normalized),
                TypeKind::InferenceVariable(_)
            ) {
                int_type
            } else {
                normalized
            };
        }
        for fact in self.enum_flow.matches_mut() {
            fact.subject_type = self
                .inference
                .normalize(&mut self.interner, fact.subject_type, &mut self.budget)
                .map_err(|error| ShadowTypeError {
                    origin: fallback,
                    error,
                })?;
        }
        for fact in self.enum_flow.tries_mut() {
            for type_id in [
                &mut fact.carrier_type,
                &mut fact.success_type,
                &mut fact.return_type,
            ] {
                *type_id = self
                    .inference
                    .normalize(&mut self.interner, *type_id, &mut self.budget)
                    .map_err(|error| ShadowTypeError {
                        origin: fallback,
                        error,
                    })?;
            }
            fact.direct_residual = fact.carrier_type == fact.return_type;
        }
        Ok(())
    }

    fn unify(
        &mut self,
        expected: TypeId,
        actual: TypeId,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        self.inference
            .unify(&mut self.interner, expected, actual, &mut self.budget)
            .map_err(|error| self.at(origin, error))
    }

    fn fresh(
        &mut self,
        level: InferenceLevel,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        self.inference
            .fresh(&mut self.interner, level, Some(origin))
            .map_err(|error| self.at(origin, error))
    }

    fn module_for_origin(&self, origin: OriginId) -> ModuleId {
        let file = self.package.origins[origin].span.file.raw();
        self.package
            .modules
            .iter()
            .find(|module| module.module.raw() == file)
            .or_else(|| self.package.modules.first())
            .map(|module| module.module)
            .expect("a lowered HIR package has at least one module")
    }

    const fn at(&self, origin: OriginId, error: TypeError) -> ShadowTypeError {
        ShadowTypeError { origin, error }
    }
}
