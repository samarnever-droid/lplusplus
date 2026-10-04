use std::collections::BTreeMap;

use lpp_hir::{
    ArenaId, BindingTarget, DefId, ExprId, ExpressionKind, HirItemId, HirItemKind, IdRange,
    NameBinding, OriginId, Symbol, TypeParamId,
};

use crate::{
    AggregateExpressionFact, InferenceLevel, TypeError, TypeId, TypeKind, TypeSubstitution,
};

use super::{ShadowTypeError, TypeChecker};

impl<'hir> TypeChecker<'hir> {
    pub(super) fn infer_call(
        &mut self,
        call_expression: ExprId,
        callee: ExprId,
        arguments: IdRange<ExprId>,
        explicit_arguments: Option<&[TypeId]>,
        parameters: &BTreeMap<Symbol, TypeParamId>,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        // UFCS method dispatch: `receiver.method(args)` where `method` is not a
        // struct field resolves to a free function or `impl` method taking the
        // receiver as its first argument.
        if let ExpressionKind::Field { base, name } = self.package.expressions[callee].kind {
            if let Some(result) = self.infer_method_call(
                call_expression,
                callee,
                base,
                name,
                arguments,
                parameters,
                origin,
            )? {
                return Ok(result);
            }
        }
        let callee_type = if let Some(explicit_arguments) = explicit_arguments {
            match self.package.expressions[callee].kind {
                ExpressionKind::Name {
                    binding: NameBinding::Item(BindingTarget::Definition(definition)),
                    ..
                } => {
                    let type_id =
                        self.instantiate_item_with(definition, explicit_arguments, origin)?;
                    self.assignments.expressions[callee.index()] = Some(type_id);
                    type_id
                }
                _ => self.infer_expression(callee, parameters)?,
            }
        } else {
            self.infer_expression(callee, parameters)?
        };

        let arguments = self.package.expressions(arguments).to_vec();
        if matches!(
            self.aggregates.expression(callee),
            Some(AggregateExpressionFact::UnitVariant { .. })
        ) {
            if arguments.is_empty() {
                return Ok(callee_type);
            }
            return Err(self.at(
                origin,
                TypeError::ArityMismatch {
                    expected: 0,
                    actual: arguments.len(),
                },
            ));
        }
        // v1 struct-constructor arity (verified against the v1 oracle): a
        // struct constructor accepts either every field positionally or zero
        // arguments — `Box()` zero-initializes all fields — while a partial
        // count is a structured arity failure (v1 `E0004`). The zero-argument
        // form yields the constructed nominal directly; the per-field
        // parameter list is never unified, and MIR synthesizes the zero values
        // (see `zero_init_struct_fields`).
        if arguments.is_empty() && self.is_struct_constructor(callee) {
            if let TypeKind::Function { result, .. } = self.interner.kind(callee_type) {
                return Ok(result);
            }
        }
        if self.is_struct_constructor(callee) {
            return self.infer_struct_constructor_call(callee_type, &arguments, parameters, origin);
        }
        if let Some(fixed) = self.variadic_fixed_arity(callee) {
            return self.infer_variadic_call(callee_type, &arguments, fixed, parameters, origin);
        }
        if let Some((required, total)) = self.default_param_arity(callee) {
            if arguments.len() >= required && arguments.len() <= total {
                return self.infer_defaulted_call(callee_type, &arguments, parameters, origin);
            }
        }
        let mut argument_types = Vec::with_capacity(arguments.len());
        for argument in arguments {
            argument_types.push(self.infer_expression(argument, parameters)?);
        }
        // Some builtins are spelled generically but overload on the (now known)
        // argument types — e.g. `slice_get` on a `StrSlice` yields a `Str`
        // instead of an `Int`. Re-resolve the callee's function type from the
        // specialized builtin so the call's result type matches the overload.
        let callee_type =
            self.respecialized_builtin_callee_type(callee, call_expression, callee_type, origin)?;
        let result = self.fresh(InferenceLevel::ROOT.child(), origin)?;
        let argument_types = self
            .interner
            .intern_list(&argument_types)
            .map_err(|error| self.at(origin, error.into()))?;
        let function = self
            .interner
            .intern(TypeKind::Function {
                parameters: argument_types,
                result,
            })
            .map_err(|error| self.at(origin, error.into()))?;
        self.unify(function, callee_type, origin)?;
        Ok(result)
    }

    /// Some builtins overload their *result type* on the (now known) argument
    /// types even though they keep the same builtin id and runtime dispatch is
    /// resolved later by codegen. Currently only `slice_get` needs this: on a
    /// `StrSlice` receiver it yields a `Str` (a fresh 1-char string) rather than
    /// an `Int`. Return an adjusted callee function type in that case; otherwise
    /// return `callee_type` unchanged.
    fn respecialized_builtin_callee_type(
        &mut self,
        callee: ExprId,
        call_expression: ExprId,
        callee_type: TypeId,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        let ExpressionKind::Name {
            symbol,
            binding: NameBinding::Unresolved,
        } = self.package.expressions[callee].kind
        else {
            return Ok(callee_type);
        };
        let Some(spelling) = self.package.names.symbols.resolve(symbol) else {
            return Ok(callee_type);
        };
        if spelling != "slice_get" || !self.slice_get_on_str_slice(call_expression) {
            return Ok(callee_type);
        }
        // Rebuild the function type with a `Str` result, keeping the parameter
        // list intact so the argument unification is unchanged.
        let TypeKind::Function { parameters, .. } = self.interner.kind(callee_type) else {
            return Ok(callee_type);
        };
        let string = self.interner.primitive(crate::PrimitiveType::String);
        self.interner
            .intern(TypeKind::Function {
                parameters,
                result: string,
            })
            .map_err(|error| self.at(origin, error.into()))
    }

    /// When `callee` names a variadic function, returns the count of fixed
    /// (leading, non-rest) parameters. The rest parameter collects every
    /// trailing argument into its `List[element]`.
    fn variadic_fixed_arity(&self, callee: ExprId) -> Option<usize> {
        let ExpressionKind::Name {
            binding: NameBinding::Item(BindingTarget::Definition(definition)),
            ..
        } = self.package.expressions[callee].kind
        else {
            return None;
        };
        let item = self
            .definition_items
            .get(definition.index())
            .and_then(|i| *i)?;
        let HirItemKind::Function(function) = self.package.items[item].kind else {
            return None;
        };
        if !function.variadic {
            return None;
        }
        Some(
            self.package
                .locals(function.parameters)
                .len()
                .saturating_sub(1),
        )
    }

    /// Type-check a call to a variadic function: unify each leading argument
    /// with its fixed parameter, then unify every trailing argument with the
    /// rest parameter's list-element type. Zero trailing arguments is legal.
    fn infer_variadic_call(
        &mut self,
        callee_type: TypeId,
        arguments: &[ExprId],
        fixed: usize,
        parameters: &BTreeMap<Symbol, TypeParamId>,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        let TypeKind::Function {
            parameters: param_list,
            result,
        } = self.interner.kind(callee_type)
        else {
            return Err(self.at(
                origin,
                TypeError::ArityMismatch {
                    expected: fixed,
                    actual: arguments.len(),
                },
            ));
        };
        let param_types = self.interner.list(param_list).to_vec();
        if arguments.len() < fixed || param_types.len() != fixed + 1 {
            return Err(self.at(
                origin,
                TypeError::ArityMismatch {
                    expected: fixed,
                    actual: arguments.len(),
                },
            ));
        }
        for index in 0..fixed {
            let argument_type = self.infer_expression(arguments[index], parameters)?;
            self.unify(param_types[index], argument_type, origin)?;
        }
        let element = match self.interner.kind(param_types[fixed]) {
            TypeKind::List(element) => element,
            _ => param_types[fixed],
        };
        for argument in &arguments[fixed..] {
            let argument_type = self.infer_expression(*argument, parameters)?;
            self.unify(element, argument_type, origin)?;
        }
        Ok(result)
    }

    /// Type-check a struct constructor's positional arguments field-by-field.
    /// A struct-typed field accepts the integer literal `0` as a null reference
    /// (the v1 idiom for optional / cycle-breaking links), so recursive shapes
    /// like `TreeNode(1, 0, 0)` construct without a spurious Int/Nominal
    /// mismatch. Every other argument unifies with its field type as usual.
    fn infer_struct_constructor_call(
        &mut self,
        callee_type: TypeId,
        arguments: &[ExprId],
        parameters: &BTreeMap<Symbol, TypeParamId>,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        let TypeKind::Function {
            parameters: field_list,
            result,
        } = self.interner.kind(callee_type)
        else {
            return Err(self.at(
                origin,
                TypeError::ArityMismatch {
                    expected: arguments.len(),
                    actual: arguments.len(),
                },
            ));
        };
        let field_types = self.interner.list(field_list).to_vec();
        if arguments.len() != field_types.len() {
            // Match the established convention (expected = supplied argument
            // count, actual = declared field count) used by the general
            // function-unification arity check.
            return Err(self.at(
                origin,
                TypeError::ArityMismatch {
                    expected: arguments.len(),
                    actual: field_types.len(),
                },
            ));
        }
        for (field_type, argument) in field_types.iter().zip(arguments.iter()) {
            let argument_type = self.infer_expression(*argument, parameters)?;
            if self.is_zero_literal(*argument) && self.is_nominal(*field_type) {
                continue;
            }
            self.unify(*field_type, argument_type, origin)?;
        }
        Ok(result)
    }

    /// Whether `expression` is the integer literal `0`.
    fn is_zero_literal(&self, expression: ExprId) -> bool {
        matches!(
            self.package.expressions[expression].kind,
            ExpressionKind::Literal(lpp_hir::Literal::Integer(0))
        )
    }

    /// Whether `ty` resolves to a nominal (struct/enum) type.
    fn is_nominal(&self, ty: TypeId) -> bool {
        matches!(self.interner.kind(ty), TypeKind::Nominal { .. })
    }

    /// When `callee` names a function with default parameters, returns
    /// `(required, total)`: the count of leading required parameters (no
    /// default) and the total parameter count. A call is well-formed when its
    /// argument count is in `required..=total`.
    fn default_param_arity(&self, callee: ExprId) -> Option<(usize, usize)> {
        let ExpressionKind::Name {
            binding: NameBinding::Item(BindingTarget::Definition(definition)),
            ..
        } = self.package.expressions[callee].kind
        else {
            return None;
        };
        let item = self
            .definition_items
            .get(definition.index())
            .and_then(|i| *i)?;
        let HirItemKind::Function(function) = self.package.items[item].kind else {
            return None;
        };
        let locals = self.package.locals(function.parameters);
        let total = locals.len();
        let with_default = locals
            .iter()
            .filter(|local| self.package.locals[**local].default.is_some())
            .count();
        if with_default == 0 {
            return None;
        }
        Some((total - with_default, total))
    }

    /// Type-check a call that omits trailing defaulted arguments: unify each
    /// supplied argument with its parameter; omitted parameters keep their
    /// (already type-checked) default.
    fn infer_defaulted_call(
        &mut self,
        callee_type: TypeId,
        arguments: &[ExprId],
        parameters: &BTreeMap<Symbol, TypeParamId>,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        let TypeKind::Function {
            parameters: param_list,
            result,
        } = self.interner.kind(callee_type)
        else {
            return Err(self.at(
                origin,
                TypeError::ArityMismatch {
                    expected: arguments.len(),
                    actual: arguments.len(),
                },
            ));
        };
        let param_types = self.interner.list(param_list).to_vec();
        if arguments.len() > param_types.len() {
            return Err(self.at(
                origin,
                TypeError::ArityMismatch {
                    expected: param_types.len(),
                    actual: arguments.len(),
                },
            ));
        }
        for (param_type, argument) in param_types.iter().zip(arguments.iter()) {
            let argument_type = self.infer_expression(*argument, parameters)?;
            self.unify(*param_type, argument_type, origin)?;
        }
        Ok(result)
    }

    /// UFCS method dispatch. Given the callee `receiver.name` of a call,
    /// attempts to resolve `name` to a method (a free function whose first
    /// parameter is the receiver's nominal type, or an `impl` method) and type
    /// checks the call as `name(receiver, args..)`. Returns `Ok(None)` when the
    /// callee is not a method call (e.g. `name` is a real struct field, or the
    /// receiver is not a nominal type), letting the caller fall back to normal
    /// call inference.
    fn infer_method_call(
        &mut self,
        call_expression: ExprId,
        callee: ExprId,
        base: ExprId,
        name: Symbol,
        arguments: IdRange<ExprId>,
        parameters: &BTreeMap<Symbol, TypeParamId>,
        origin: OriginId,
    ) -> Result<Option<TypeId>, ShadowTypeError> {
        let base_type = self.infer_expression(base, parameters)?;
        let resolved_base = self
            .inference
            .resolve(&self.interner, base_type, &mut self.budget)
            .map_err(|error| self.at(origin, error))?;
        // A trait-bounded generic receiver (`a.byte_size()` where `a: T` and
        // `T: Sizer`): resolve the method through the type parameter's trait
        // bound for typing. MIR re-resolves it to the concrete `impl` method
        // once the instance's receiver type is known.
        if let TypeKind::GenericParameter(param_id) = self.interner.kind(resolved_base) {
            return self.infer_trait_bounded_method_call(
                call_expression,
                callee,
                param_id,
                name,
                arguments,
                parameters,
                origin,
            );
        }
        let TypeKind::Nominal { definition, .. } = self.interner.kind(resolved_base) else {
            return Ok(None);
        };
        // A real struct field takes precedence: `value.field()` calls the value
        // stored in the field, not a method named `field`.
        if let Some(item) = self.aggregates.item(definition) {
            if matches!(self.package.items[item].kind, HirItemKind::Struct(_))
                && self.aggregates.field(item, name).is_some()
            {
                return Ok(None);
            }
        }
        let Some(candidates) = self.method_candidates.get(&(definition, name)).cloned() else {
            return Ok(None);
        };
        let mut candidates = candidates;
        candidates.sort_by_key(|method| self.item_parameters[method.index()].len());
        let argument_ids = self.package.expressions(arguments).to_vec();
        for method_item in candidates {
            // Instantiate the method's in-scope generics with fresh inference
            // variables so a generic receiver or argument unifies at the call
            // site. Candidates are tried most-specific first (fewer generics).
            let generics = self
                .method_generics
                .get(&method_item)
                .cloned()
                .unwrap_or_default();
            let mut fresh_arguments = Vec::with_capacity(generics.len());
            for _ in &generics {
                fresh_arguments.push(self.fresh(InferenceLevel::ROOT.child(), origin)?);
            }
            let method_type =
                self.substitute_item(method_item, &generics, &fresh_arguments, origin)?;
            let TypeKind::Function {
                parameters: param_list,
                result,
            } = self.interner.kind(method_type)
            else {
                continue;
            };
            let result_type = result;
            let param_types = self.interner.list(param_list).to_vec();
            let expected_arguments = param_types.len().saturating_sub(1);
            let snapshot = self.inference.clone();
            let mut fits = argument_ids.len() == expected_arguments;
            if fits {
                if let Some(receiver_type) = param_types.first() {
                    fits = self.unify(*receiver_type, resolved_base, origin).is_ok();
                }
            }
            if fits {
                for (param_type, argument) in param_types.iter().skip(1).zip(argument_ids.iter()) {
                    let argument_type = match self.infer_expression(*argument, parameters) {
                        Ok(argument_type) => argument_type,
                        Err(_) => {
                            fits = false;
                            break;
                        }
                    };
                    if self.unify(*param_type, argument_type, origin).is_err() {
                        fits = false;
                        break;
                    }
                }
            }
            if fits {
                self.assignments.expressions[callee.index()] = Some(method_type);
                self.aggregates.set_expression(
                    call_expression,
                    AggregateExpressionFact::MethodCall {
                        method: method_item,
                    },
                );
                return Ok(Some(result_type));
            }
            self.inference = snapshot;
        }
        Ok(None)
    }

    /// Find the declared method `name` on the trait defined by `trait_def`, if
    /// any. Trait declarations carry their method signatures as nested function
    /// items whose bodies may be absent (`signature_only`).
    fn trait_method_item(&self, trait_def: DefId, name: Symbol) -> Option<HirItemId> {
        let item_id = (*self.definition_items.get(trait_def.index())?)
            .as_ref()
            .copied()?;
        let HirItemKind::Trait(trait_) = self.package.items[item_id].kind else {
            return None;
        };
        for method in self.package.items(trait_.methods) {
            if self.package.items[*method].name == Some(name) {
                return Some(*method);
            }
        }
        None
    }

    /// Type-check a method call whose receiver is a trait-bounded type parameter
    /// (`a.m(..)` with `a: T`, `T: Trait`). The call is typed from the trait's
    /// declared method signature; the recorded `MethodCall` fact points at the
    /// trait method item, and MIR re-resolves it to the concrete `impl` method
    /// using the monomorphized receiver type. Returns `Ok(None)` (falling back
    /// to ordinary inference) when the bound, the trait, or the method cannot be
    /// resolved.
    fn infer_trait_bounded_method_call(
        &mut self,
        call_expression: ExprId,
        callee: ExprId,
        param_id: TypeParamId,
        name: Symbol,
        arguments: IdRange<ExprId>,
        parameters: &BTreeMap<Symbol, TypeParamId>,
        origin: OriginId,
    ) -> Result<Option<TypeId>, ShadowTypeError> {
        let Some(bound_ref) = self.package.type_parameters[param_id].bound else {
            return Ok(None);
        };
        let Some(bound_type) = self.assignments.type_ref(bound_ref) else {
            return Ok(None);
        };
        let TypeKind::Nominal {
            definition: trait_def,
            ..
        } = self.interner.kind(bound_type)
        else {
            return Ok(None);
        };
        let Some(trait_method) = self.trait_method_item(trait_def, name) else {
            return Ok(None);
        };
        let Some(method_type) = self.assignments.items[trait_method.index()] else {
            return Ok(None);
        };
        let TypeKind::Function {
            parameters: param_list,
            result,
        } = self.interner.kind(method_type)
        else {
            return Ok(None);
        };
        let param_types = self.interner.list(param_list).to_vec();
        let expected_arguments = param_types.len().saturating_sub(1);
        let argument_ids = self.package.expressions(arguments).to_vec();
        if argument_ids.len() != expected_arguments {
            return Ok(None);
        }
        // Type the explicit arguments. The trait signature's parameter types may
        // mention `Self`, so unification is best-effort here — the concrete
        // `impl` method (re-resolved by MIR) carries the authoritative types.
        for (param_type, argument) in param_types.iter().skip(1).zip(argument_ids.iter()) {
            let argument_type = self.infer_expression(*argument, parameters)?;
            let _ = self.unify(*param_type, argument_type, origin);
        }
        self.assignments.expressions[callee.index()] = Some(method_type);
        self.aggregates.set_expression(
            call_expression,
            AggregateExpressionFact::MethodCall {
                method: trait_method,
            },
        );
        Ok(Some(result))
    }

    /// Whether `callee` is a bare struct name used as a constructor (a `Name`
    /// bound to a `Struct` definition). Mirrors `record_constructor_call`'s
    /// struct detection so the zero-argument type result and the recorded
    /// aggregate fact always agree.
    fn is_struct_constructor(&self, callee: ExprId) -> bool {
        match self.package.expressions[callee].kind {
            ExpressionKind::Name {
                binding: NameBinding::Item(BindingTarget::Definition(definition)),
                ..
            } => self
                .definition_items
                .get(definition.index())
                .and_then(|item| *item)
                .is_some_and(|item| {
                    matches!(self.package.items[item].kind, HirItemKind::Struct(_))
                }),
            _ => false,
        }
    }

    pub(super) fn instantiate_item(
        &mut self,
        definition: DefId,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        let Some(item) = self.definition_items[definition.index()] else {
            return Ok(self.interner.error());
        };
        let parameters = self.item_parameters[item.index()].clone();
        let mut arguments = Vec::with_capacity(parameters.len());
        for _ in &parameters {
            arguments.push(self.fresh(InferenceLevel::ROOT.child(), origin)?);
        }
        self.substitute_item(item, &parameters, &arguments, origin)
    }

    fn instantiate_item_with(
        &mut self,
        definition: DefId,
        arguments: &[TypeId],
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        let Some(item) = self.definition_items[definition.index()] else {
            return Ok(self.interner.error());
        };
        let parameters = self.item_parameters[item.index()].clone();
        if parameters.len() != arguments.len() {
            return Err(self.at(
                origin,
                TypeError::ArityMismatch {
                    expected: parameters.len(),
                    actual: arguments.len(),
                },
            ));
        }
        self.substitute_item(item, &parameters, arguments, origin)
    }

    fn substitute_item(
        &mut self,
        item: lpp_hir::HirItemId,
        parameters: &[TypeParamId],
        arguments: &[TypeId],
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        let type_id = self.assignments.items[item.index()].unwrap_or_else(|| self.interner.error());
        if parameters.is_empty() {
            return Ok(type_id);
        }
        let mut substitution = TypeSubstitution::new();
        for (parameter, argument) in parameters.iter().zip(arguments) {
            substitution.insert(*parameter, *argument);
        }
        substitution
            .apply(&mut self.interner, type_id, &mut self.budget)
            .map_err(|error| self.at(origin, error))
    }
}
