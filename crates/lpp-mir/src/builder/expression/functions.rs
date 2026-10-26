use super::*;

impl FunctionBuilder<'_, '_> {
    /// If `definition` names a `const`, returns its value expression so a
    /// reference can be inlined at the use site.
    pub(in crate::builder) fn const_value(&self, definition: lpp_hir::DefId) -> Option<ExprId> {
        let item = self
            .core
            .definition_items
            .get(definition.index())
            .and_then(|item| *item)?;
        match self.core.package.items[item].kind {
            HirItemKind::Const { value } => Some(value),
            _ => None,
        }
    }

    pub(in crate::builder) fn lower_function_name(
        &mut self,
        expression: ExprId,
        definition: lpp_hir::DefId,
        explicit_arguments: Option<IdRange<TypeRefId>>,
    ) -> Result<MirFunctionId, MirBuildError> {
        let origin = self.core.package.expressions[expression].origin;
        let Some(item_id) = self
            .core
            .definition_items
            .get(definition.index())
            .and_then(|item| *item)
        else {
            return Err(self.core.error(
                origin,
                MirBuildErrorKind::Unsupported(UnsupportedConstruct::ExternalFunction),
            ));
        };
        let HirItemKind::Function(function) = self.core.package.items[item_id].kind else {
            return Err(self.core.error(
                origin,
                MirBuildErrorKind::Unsupported(UnsupportedConstruct::ExternalFunction),
            ));
        };
        if function.type_parameters.is_empty() {
            return self
                .core
                .definition_functions
                .get(definition.index())
                .and_then(|function| *function)
                .ok_or_else(|| {
                    self.core.error(
                        origin,
                        MirBuildErrorKind::Unsupported(UnsupportedConstruct::ExternalFunction),
                    )
                });
        }

        if let Some(type_arguments) = explicit_arguments {
            let type_arguments = self.core.package.type_refs(type_arguments).to_vec();
            let mut concrete_arguments = Vec::with_capacity(type_arguments.len());
            for type_reference in type_arguments {
                let ty = self
                    .core
                    .types
                    .assignments
                    .type_ref(type_reference)
                    .ok_or_else(|| {
                        self.core
                            .error(origin, MirBuildErrorKind::GenericTypeMaterialization)
                    })?;
                concrete_arguments.push(self.concrete_type(ty, origin)?);
            }
            let arguments = self
                .core
                .types
                .interner
                .intern_list(&concrete_arguments)
                .map_err(|_| {
                    self.core
                        .error(origin, MirBuildErrorKind::GenericTypeMaterialization)
                })?;
            let instance = self
                .core
                .types
                .instances
                .instance(InstanceKey::new(item_id, arguments))
                .ok_or_else(|| {
                    self.core.error(
                        origin,
                        MirBuildErrorKind::MissingExplicitInstance { item: item_id },
                    )
                })?;
            return self.core.instance_functions[instance.index()].ok_or_else(|| {
                self.core.error(
                    origin,
                    MirBuildErrorKind::MissingExplicitInstance { item: item_id },
                )
            });
        }

        let ty = self
            .core
            .types
            .assignments
            .expression(expression)
            .ok_or_else(|| {
                self.core.error(
                    origin,
                    MirBuildErrorKind::MissingType(TypedEntity::Expression(expression)),
                )
            })?;
        let ty = self.concrete_type(ty, origin)?;
        self.core
            .instance_type_functions
            .get(&(item_id, ty))
            .and_then(|function| *function)
            .ok_or_else(|| {
                self.core.error(
                    origin,
                    MirBuildErrorKind::MissingFunctionInstance { item: item_id, ty },
                )
            })
    }
}
