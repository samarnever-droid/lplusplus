use super::*;

impl<'hir> TypeChecker<'hir> {
    pub(in crate::check) fn infer_match_statement(
        &mut self,
        statement: StmtId,
        subject: ExprId,
        arms: IdRange<MatchArmId>,
        expected_result: TypeId,
        parameters: &BTreeMap<Symbol, TypeParamId>,
        origin: OriginId,
    ) -> Result<(), ShadowTypeError> {
        let subject_type = self.infer_expression(subject, parameters)?;
        let subject_type = self
            .inference
            .resolve(&self.interner, subject_type, &mut self.budget)
            .map_err(|error| self.at(origin, error))?;
        let TypeKind::Nominal {
            definition,
            arguments,
        } = self.interner.kind(subject_type)
        else {
            return Err(self.at(
                origin,
                TypeError::InvalidMatchSubject {
                    actual: subject_type,
                },
            ));
        };
        let Some(item) = self.aggregates.item(definition) else {
            return Err(self.at(
                origin,
                TypeError::InvalidMatchSubject {
                    actual: subject_type,
                },
            ));
        };
        let HirItemKind::Enum(enumeration) = self.package.items[item].kind else {
            return Err(self.at(
                origin,
                TypeError::InvalidMatchSubject {
                    actual: subject_type,
                },
            ));
        };
        let variants = self.package.variants(enumeration.variants).to_vec();
        let total_variants = u32::try_from(variants.len()).map_err(|_| {
            self.at(
                origin,
                TypeError::ArityMismatch {
                    expected: u32::MAX as usize,
                    actual: variants.len(),
                },
            )
        })?;
        let item_name = self.package.items[item]
            .name
            .expect("top-level enums retain their names");
        let mut covered = BTreeSet::new();
        let mut wildcard = None;
        let mut wildcard_seen = false;
        let arm_ids = self.package.match_arms(arms).to_vec();

        for arm_id in arm_ids {
            let arm = self.package.match_arms[arm_id];
            let path = self.package.symbols(arm.path);
            if arm.wildcard {
                if path.len() != 1 || !self.package.locals(arm.bindings).is_empty() {
                    return Err(self.at(arm.origin, TypeError::InvalidMatchPattern));
                }
                let reachable = !wildcard_seen && covered.len() < variants.len();
                if wildcard.is_none() {
                    wildcard = Some(arm_id);
                }
                wildcard_seen = true;
                self.enum_flow
                    .set_arm(arm_id, EnumMatchArmFact::Wildcard { reachable });
            } else {
                let variant_name = match path {
                    [variant] => *variant,
                    [qualifier, variant] if *qualifier == item_name => *variant,
                    _ => return Err(self.at(arm.origin, TypeError::InvalidMatchPattern)),
                };
                let Some(variant) = self.aggregates.variant(item, variant_name) else {
                    return Err(self.at(
                        arm.origin,
                        TypeError::UnknownVariant {
                            definition,
                            variant: variant_name,
                        },
                    ));
                };
                let field_range = self.package.variants[variant].fields;
                let source_fields = self.package.fields(field_range).to_vec();
                let binding_range = arm.bindings;
                let bindings = self.package.locals(binding_range).to_vec();
                if bindings.len() != source_fields.len() {
                    return Err(self.at(
                        arm.origin,
                        TypeError::MatchBindingArity {
                            variant,
                            expected: source_fields.len(),
                            actual: bindings.len(),
                        },
                    ));
                }
                for (binding, field) in bindings.into_iter().zip(source_fields) {
                    let binding_type = self.ensure_local(binding, parameters)?;
                    let field_type = self.nominal_field_type(item, field, arguments, arm.origin)?;
                    self.unify(field_type, binding_type, arm.origin)?;
                }
                let reachable = !wildcard_seen && covered.insert(variant);
                self.enum_flow.set_arm(
                    arm_id,
                    EnumMatchArmFact::Variant {
                        item,
                        variant,
                        fields: field_range,
                        bindings: binding_range,
                        reachable,
                    },
                );
            }
            self.infer_body(arm.body, expected_result, parameters)?;
        }

        let covered_variants = u32::try_from(covered.len()).map_err(|_| {
            self.at(
                origin,
                TypeError::ArityMismatch {
                    expected: u32::MAX as usize,
                    actual: covered.len(),
                },
            )
        })?;
        self.enum_flow.set_match(
            statement,
            EnumMatchFact {
                item,
                subject_type,
                arms,
                covered_variants,
                total_variants,
                wildcard,
            },
        );
        Ok(())
    }

    pub(in crate::check) fn infer_try_expression(
        &mut self,
        expression: ExprId,
        inner: ExprId,
        parameters: &BTreeMap<Symbol, TypeParamId>,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        let carrier = self.infer_expression(inner, parameters)?;
        let carrier = self
            .inference
            .resolve(&self.interner, carrier, &mut self.budget)
            .map_err(|error| self.at(origin, error))?;
        let TypeKind::Nominal {
            definition,
            arguments,
        } = self.interner.kind(carrier)
        else {
            return Err(self.at(origin, TypeError::InvalidTryCarrier { actual: carrier }));
        };
        let Some(item) = self.aggregates.item(definition) else {
            return Err(self.at(origin, TypeError::InvalidTryCarrier { actual: carrier }));
        };
        let HirItemKind::Enum(enumeration) = self.package.items[item].kind else {
            return Err(self.at(origin, TypeError::InvalidTryCarrier { actual: carrier }));
        };
        let variants = self.package.variants(enumeration.variants).to_vec();
        let Some(success) = variants.first().copied() else {
            return Err(self.at(origin, TypeError::InvalidTryCarrier { actual: carrier }));
        };
        let success_fields = self
            .package
            .fields(self.package.variants[success].fields)
            .to_vec();
        if success_fields.len() != 1 {
            return Err(self.at(
                origin,
                TypeError::TrySuccessPayloadArity {
                    actual: success_fields.len(),
                },
            ));
        }
        let success_type = self.nominal_field_type(item, success_fields[0], arguments, origin)?;
        let Some(function_result) = self.current_function_result else {
            return Err(self.at(
                origin,
                TypeError::InvalidTryReturn {
                    carrier,
                    function_result: self.interner.primitive(PrimitiveType::Void),
                },
            ));
        };
        let function_result = self
            .inference
            .resolve(&self.interner, function_result, &mut self.budget)
            .map_err(|error| self.at(origin, error))?;
        let TypeKind::Nominal {
            definition: result_definition,
            arguments: result_arguments,
        } = self.interner.kind(function_result)
        else {
            return Err(self.at(
                origin,
                TypeError::InvalidTryReturn {
                    carrier,
                    function_result,
                },
            ));
        };
        if result_definition != definition {
            return Err(self.at(
                origin,
                TypeError::InvalidTryReturn {
                    carrier,
                    function_result,
                },
            ));
        }
        let return_item = self
            .aggregates
            .item(result_definition)
            .expect("the matching nominal enum definition has an aggregate item");
        for variant in variants.iter().copied().skip(1) {
            for field in self
                .package
                .fields(self.package.variants[variant].fields)
                .iter()
                .copied()
            {
                let source = self.nominal_field_type(item, field, arguments, origin)?;
                let target = self.nominal_field_type(item, field, result_arguments, origin)?;
                self.unify(source, target, origin)
                    .map_err(|_| self.at(origin, TypeError::InvalidTryResidual { variant }))?;
            }
        }
        self.enum_flow.set_try(
            expression,
            EnumTryFact {
                item,
                carrier_type: carrier,
                success,
                success_field: success_fields[0],
                success_type,
                variants: enumeration.variants,
                return_item,
                return_type: function_result,
                direct_residual: carrier == function_result,
            },
        );
        Ok(success_type)
    }

    pub(in crate::check) fn infer_index_expression(
        &mut self,
        expression: ExprId,
        base_expression: ExprId,
        index_expression: ExprId,
        parameters: &BTreeMap<Symbol, TypeParamId>,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        let base = self.infer_expression(base_expression, parameters)?;
        let base = self
            .inference
            .resolve(&self.interner, base, &mut self.budget)
            .map_err(|error| self.at(origin, error))?;
        let index_type = self.infer_expression(index_expression, parameters)?;
        let int = self.interner.primitive(PrimitiveType::Int);
        self.unify(int, index_type, origin)?;

        match self.interner.kind(base) {
            TypeKind::Tuple(elements) => {
                let ExpressionKind::Literal(Literal::Integer(index)) =
                    self.package.expressions[index_expression].kind
                else {
                    return Err(self.at(origin, TypeError::TupleIndexMustBeStatic));
                };
                let elements = self.interner.list(elements);
                let Ok(index_usize) = usize::try_from(index) else {
                    return Err(self.at(
                        origin,
                        TypeError::TupleIndexOutOfBounds {
                            index,
                            arity: elements.len(),
                        },
                    ));
                };
                let Some(element) = elements.get(index_usize).copied() else {
                    return Err(self.at(
                        origin,
                        TypeError::TupleIndexOutOfBounds {
                            index,
                            arity: elements.len(),
                        },
                    ));
                };
                let compact_index = u32::try_from(index_usize).map_err(|_| {
                    self.at(
                        origin,
                        TypeError::TupleIndexOutOfBounds {
                            index,
                            arity: elements.len(),
                        },
                    )
                })?;
                self.places.set_expression(
                    expression,
                    PlaceExpressionFact::TupleField {
                        index: compact_index,
                    },
                );
                Ok(element)
            }
            TypeKind::List(element) => {
                self.places
                    .set_expression(expression, PlaceExpressionFact::ListIndex);
                Ok(element)
            }
            TypeKind::Slice(element) => {
                // A slice element read `view[i]` is a value, not an assignable
                // place: it lowers to the `slice_get` runtime call. (For a
                // `StrSlice`, codegen dispatches `slice_get` to the
                // string-slice entry point by receiver type.)
                if let Some(slice_get) = self.builtin_index.lookup("slice_get") {
                    self.builtins.record(expression, slice_get);
                }
                Ok(element)
            }
            _ if base == self.interner.primitive(PrimitiveType::StrSlice) => {
                // `sview[i]` reads a character and yields a fresh one-character
                // string. It must use `slice_get` (dispatched to
                // `lpp_str_slice_get`), never `char_at`, which would misread the
                // view record as a string.
                if let Some(slice_get) = self.builtin_index.lookup("slice_get") {
                    self.builtins.record(expression, slice_get);
                }
                Ok(self.interner.primitive(PrimitiveType::String))
            }
            _ if base == self.interner.primitive(PrimitiveType::String) => {
                // String indexing is not an lvalue/place: `s[i]` produces a new
                // one-character string. Record a `char_at` builtin fact so MIR
                // lowers it to the runtime call instead of a place load.
                if let Some(char_at) = self.builtin_index.lookup("char_at") {
                    self.builtins.record(expression, char_at);
                }
                Ok(self.interner.primitive(PrimitiveType::String))
            }
            _ => {
                let element = self.fresh(InferenceLevel::ROOT.child(), origin)?;
                let list = self
                    .interner
                    .intern(TypeKind::List(element))
                    .map_err(|error| self.at(origin, error.into()))?;
                self.unify(list, base, origin)?;
                self.places
                    .set_expression(expression, PlaceExpressionFact::ListIndex);
                Ok(element)
            }
        }
    }
}
