use super::*;

impl Verifier<'_> {
    pub(in crate::verify) fn place_type(
        &mut self,
        site: InstructionSite,
        place_id: MirPlaceId,
        locals: &BTreeSet<MirLocalId>,
        for_store: bool,
    ) -> Option<TypeId> {
        let Some(place) = self.program.place(place_id).copied() else {
            self.push(
                site.function,
                Some(site.block),
                Some(site.instruction),
                site.origin,
                MirVerificationErrorKind::MissingReference(MirEntity::Place(place_id)),
            );
            return None;
        };
        let Some(root) = self.program.local(place.root).copied() else {
            self.reference_error(
                site.function,
                site.block,
                site.instruction,
                site.origin,
                MirEntity::Local(place.root),
                false,
            );
            return None;
        };
        if !locals.contains(&place.root) {
            self.reference_error(
                site.function,
                site.block,
                site.instruction,
                site.origin,
                MirEntity::Local(place.root),
                true,
            );
            return None;
        }
        let projections = self.program.place_projections(&place).to_vec();
        if for_store {
            if projections
                .iter()
                .any(|projection| matches!(projection, PlaceProjection::Downcast(_)))
            {
                self.push(
                    site.function,
                    Some(site.block),
                    Some(site.instruction),
                    site.origin,
                    MirVerificationErrorKind::DowncastStore,
                );
                return None;
            }
            let parameter_projection = !projections.is_empty()
                && self
                    .program
                    .function(site.function)
                    .is_some_and(|function| {
                        self.program
                            .function_parameters(function)
                            .contains(&place.root)
                    });
            if !root.mutable && !parameter_projection {
                self.push(
                    site.function,
                    Some(site.block),
                    Some(site.instruction),
                    site.origin,
                    MirVerificationErrorKind::ImmutableStore(place.root),
                );
                return None;
            }
            if projections
                .iter()
                .any(|projection| matches!(projection, PlaceProjection::TupleField(_)))
            {
                self.push(
                    site.function,
                    Some(site.block),
                    Some(site.instruction),
                    site.origin,
                    MirVerificationErrorKind::TupleElementStore,
                );
                return None;
            }
        }

        let mut current = root.ty;
        let mut downcast = None;
        for projection in projections {
            current = match projection {
                PlaceProjection::Downcast(variant_id) => {
                    let Some(variant) = self.program.variant(variant_id) else {
                        self.push(
                            site.function,
                            Some(site.block),
                            Some(site.instruction),
                            site.origin,
                            MirVerificationErrorKind::MissingReference(MirEntity::Variant(
                                variant_id,
                            )),
                        );
                        return None;
                    };
                    let Some(aggregate) = self.program.aggregate(variant.aggregate) else {
                        self.push(
                            site.function,
                            Some(site.block),
                            Some(site.instruction),
                            site.origin,
                            MirVerificationErrorKind::MissingReference(MirEntity::Aggregate(
                                variant.aggregate,
                            )),
                        );
                        return None;
                    };
                    if downcast.is_some()
                        || current != aggregate.ty
                        || aggregate.kind != crate::MirAggregateKind::Enum
                        || !self
                            .program
                            .aggregate_variants(aggregate)
                            .contains(&variant_id)
                    {
                        self.push(
                            site.function,
                            Some(site.block),
                            Some(site.instruction),
                            site.origin,
                            MirVerificationErrorKind::InvalidAggregateType(current),
                        );
                        return None;
                    }
                    downcast = Some(variant_id);
                    current
                }
                PlaceProjection::Field(field_id) => {
                    let Some(field) = self.program.field(field_id) else {
                        self.push(
                            site.function,
                            Some(site.block),
                            Some(site.instruction),
                            site.origin,
                            MirVerificationErrorKind::MissingReference(MirEntity::Field(field_id)),
                        );
                        return None;
                    };
                    let Some(aggregate) = self.program.aggregate(field.aggregate) else {
                        self.push(
                            site.function,
                            Some(site.block),
                            Some(site.instruction),
                            site.origin,
                            MirVerificationErrorKind::MissingReference(MirEntity::Aggregate(
                                field.aggregate,
                            )),
                        );
                        return None;
                    };
                    let valid = if let Some(variant) = downcast.take() {
                        aggregate.kind == crate::MirAggregateKind::Enum
                            && field.variant == Some(variant)
                            && self.program.variant(variant).is_some_and(|variant| {
                                self.program.variant_fields(variant).contains(&field_id)
                            })
                    } else {
                        aggregate.kind == crate::MirAggregateKind::Struct
                            && field.variant.is_none()
                            && self.program.aggregate_fields(aggregate).contains(&field_id)
                    };
                    if current != aggregate.ty || !valid {
                        self.push(
                            site.function,
                            Some(site.block),
                            Some(site.instruction),
                            site.origin,
                            MirVerificationErrorKind::InvalidAggregateType(current),
                        );
                        return None;
                    }
                    field.ty
                }
                PlaceProjection::TupleField(index) => {
                    let TypeKind::Tuple(elements) = self.types.kind(current) else {
                        self.push(
                            site.function,
                            Some(site.block),
                            Some(site.instruction),
                            site.origin,
                            MirVerificationErrorKind::InvalidTupleType(current),
                        );
                        return None;
                    };
                    let Some(element) = self.types.list(elements).get(index as usize).copied()
                    else {
                        self.push(
                            site.function,
                            Some(site.block),
                            Some(site.instruction),
                            site.origin,
                            MirVerificationErrorKind::InvalidTupleType(current),
                        );
                        return None;
                    };
                    element
                }
                PlaceProjection::ListIndex(index) => {
                    let actual = self.operand_type(
                        site.function,
                        site.block,
                        Some(site.instruction),
                        site.origin,
                        index,
                        locals,
                    )?;
                    let int = self.types.primitive(PrimitiveType::Int);
                    if actual != int {
                        self.push(
                            site.function,
                            Some(site.block),
                            Some(site.instruction),
                            site.origin,
                            MirVerificationErrorKind::TypeMismatch {
                                expected: int,
                                actual,
                            },
                        );
                        return None;
                    }
                    let TypeKind::List(element) = self.types.kind(current) else {
                        self.push(
                            site.function,
                            Some(site.block),
                            Some(site.instruction),
                            site.origin,
                            MirVerificationErrorKind::InvalidListType(current),
                        );
                        return None;
                    };
                    element
                }
            };
        }
        if downcast.is_some() {
            self.push(
                site.function,
                Some(site.block),
                Some(site.instruction),
                site.origin,
                MirVerificationErrorKind::InvalidAggregateType(current),
            );
            return None;
        }
        if current != place.ty {
            self.push(
                site.function,
                Some(site.block),
                Some(site.instruction),
                site.origin,
                MirVerificationErrorKind::TypeMismatch {
                    expected: current,
                    actual: place.ty,
                },
            );
            return None;
        }
        Some(current)
    }
}
