use super::*;

impl<'hir> TypeChecker<'hir> {
    pub(in crate::check) fn validate_assignment_target(
        &self,
        target: ExprId,
        projected: bool,
        origin: OriginId,
    ) -> Result<(), ShadowTypeError> {
        match self.package.expressions[target].kind {
            ExpressionKind::Name {
                binding: NameBinding::Local(local),
                ..
            } => {
                let local = self.package.locals[local];
                if local.mutable || (projected && local.kind == LocalKind::Parameter) {
                    Ok(())
                } else {
                    Err(self.at(origin, TypeError::ImmutableAssignmentRoot))
                }
            }
            ExpressionKind::Field { base, .. } => {
                self.validate_assignment_target(base, true, origin)
            }
            ExpressionKind::Index { base, .. } => {
                if matches!(
                    self.places.expression(target),
                    Some(PlaceExpressionFact::TupleField { .. })
                ) {
                    return Err(self.at(origin, TypeError::TupleElementAssignment));
                }
                self.validate_assignment_target(base, true, origin)
            }
            _ => Err(self.at(origin, TypeError::InvalidAssignmentTarget)),
        }
    }

    pub(in crate::check) fn infer_field_expression(
        &mut self,
        expression: ExprId,
        base: ExprId,
        name: Symbol,
        parameters: &BTreeMap<Symbol, TypeParamId>,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        if matches!(
            self.package.expression_binding(base),
            Some(BindingTarget::Module(_))
        ) {
            let _ = self.infer_expression(base, parameters)?;
            return match self.package.expression_binding(expression) {
                Some(BindingTarget::Definition(definition)) => {
                    self.instantiate_item(definition, origin)
                }
                Some(BindingTarget::Module(_)) | None => {
                    self.fresh(InferenceLevel::ROOT.child(), origin)
                }
            };
        }
        let enum_item = self
            .package
            .expression_definition(base)
            .and_then(|definition| {
                self.definition_items
                    .get(definition.index())
                    .and_then(|item| *item)
            })
            .filter(|item| matches!(self.package.items[*item].kind, HirItemKind::Enum(_)));
        let base_type = self.infer_expression(base, parameters)?;
        let resolved_base = self
            .inference
            .resolve(&self.interner, base_type, &mut self.budget)
            .map_err(|error| self.at(origin, error))?;

        if let Some(item) = enum_item {
            let Some(variant) = self.aggregates.variant(item, name) else {
                let definition = self.package.items[item]
                    .definition
                    .expect("a top-level enum has a definition");
                return Err(self.at(
                    origin,
                    TypeError::UnknownVariant {
                        definition,
                        variant: name,
                    },
                ));
            };
            if self.package.variants[variant].fields.is_empty() {
                self.aggregates.set_expression(
                    expression,
                    AggregateExpressionFact::UnitVariant { item, variant },
                );
                return Ok(resolved_base);
            }
            let callable = self.variant_constructor_type(item, variant, resolved_base, origin)?;
            self.aggregates.set_expression(
                expression,
                AggregateExpressionFact::VariantConstructor { item, variant },
            );
            return Ok(callable);
        }

        let TypeKind::Nominal {
            definition,
            arguments,
        } = self.interner.kind(resolved_base)
        else {
            return self.fresh(InferenceLevel::ROOT.child(), origin);
        };
        let Some(item) = self.aggregates.item(definition) else {
            return Err(self.at(
                origin,
                TypeError::InvalidAggregateBase {
                    actual: resolved_base,
                },
            ));
        };
        let HirItemKind::Struct(_) = self.package.items[item].kind else {
            return self.fresh(InferenceLevel::ROOT.child(), origin);
        };
        let Some(field) = self.aggregates.field(item, name) else {
            if self.call_callees.contains(&expression) {
                return self.fresh(InferenceLevel::ROOT.child(), origin);
            }
            return Err(self.at(
                origin,
                TypeError::UnknownField {
                    definition,
                    field: name,
                },
            ));
        };
        let field_type = self.nominal_field_type(item, field, arguments, origin)?;
        self.aggregates.set_expression(
            expression,
            AggregateExpressionFact::FieldProjection { item, field },
        );
        Ok(field_type)
    }

    pub(in crate::check) fn variant_constructor_type(
        &mut self,
        item: HirItemId,
        variant: VariantId,
        nominal: TypeId,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        let TypeKind::Nominal {
            definition,
            arguments,
        } = self.interner.kind(nominal)
        else {
            return Err(self.at(origin, TypeError::InvalidAggregateBase { actual: nominal }));
        };
        if self.package.items[item].definition != Some(definition) {
            return Err(self.at(origin, TypeError::InvalidAggregateBase { actual: nominal }));
        }
        let fields = self
            .package
            .fields(self.package.variants[variant].fields)
            .to_vec();
        let mut field_types = Vec::with_capacity(fields.len());
        for field in fields {
            field_types.push(self.nominal_field_type(item, field, arguments, origin)?);
        }
        let parameters = self
            .interner
            .intern_list(&field_types)
            .map_err(|error| self.at(origin, error.into()))?;
        self.interner
            .intern(TypeKind::Function {
                parameters,
                result: nominal,
            })
            .map_err(|error| self.at(origin, error.into()))
    }

    pub(in crate::check) fn nominal_field_type(
        &mut self,
        item: HirItemId,
        field: FieldId,
        arguments: crate::TypeListId,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        let item_value = self.package.items[item];
        let context = self.parameter_context(item);
        let template = self.resolve_type_ref(
            self.package.fields[field].type_ref,
            item_value.module,
            &context,
        )?;
        let parameters = self.item_parameters[item.index()].clone();
        let arguments = self.interner.list(arguments).to_vec();
        if parameters.len() != arguments.len() {
            return Err(self.at(
                origin,
                TypeError::ArityMismatch {
                    expected: parameters.len(),
                    actual: arguments.len(),
                },
            ));
        }
        let mut substitution = TypeSubstitution::new();
        for (parameter, argument) in parameters.iter().zip(arguments) {
            substitution.insert(*parameter, argument);
        }
        substitution
            .apply(&mut self.interner, template, &mut self.budget)
            .map_err(|error| self.at(origin, error))
    }

    pub(in crate::check) fn infer_name(
        &mut self,
        symbol: Symbol,
        binding: NameBinding,
        parameters: &BTreeMap<Symbol, TypeParamId>,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        match binding {
            NameBinding::Local(local) => {
                if let Some(scheme) = self.assignments.local_schemes.get(&local).cloned() {
                    self.inference
                        .instantiate(
                            &mut self.interner,
                            &scheme,
                            InferenceLevel::ROOT.child(),
                            Some(origin),
                            &mut self.budget,
                        )
                        .map_err(|error| self.at(origin, error))
                } else {
                    self.ensure_local(local, parameters)
                }
            }
            NameBinding::Item(BindingTarget::Definition(definition)) => {
                self.instantiate_item(definition, origin)
            }
            NameBinding::Item(BindingTarget::Module(_)) => {
                self.fresh(InferenceLevel::ROOT.child(), origin)
            }
            NameBinding::Unresolved => {
                let spelling = self
                    .package
                    .names
                    .symbols
                    .resolve(symbol)
                    .expect("HIR symbols retain their spelling");
                let builtin_spelling = if spelling == "_getpid" {
                    "thread_id"
                } else {
                    spelling
                };
                if let Some(builtin) = self.builtin_index.lookup(builtin_spelling) {
                    return self.builtin_function_type(builtin, origin);
                }
                self.fresh(InferenceLevel::ROOT.child(), origin)
            }
        }
    }
}
