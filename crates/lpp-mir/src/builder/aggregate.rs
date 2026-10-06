use super::*;

impl MirBuilder<'_> {
    pub(super) fn ensure_aggregate(
        &mut self,
        ty: TypeId,
        origin: OriginId,
    ) -> Result<MirAggregateId, MirBuildError> {
        if let Some(aggregate) = self.aggregate_types.get(&ty) {
            return Ok(*aggregate);
        }
        let TypeKind::Nominal {
            definition,
            arguments,
        } = self.types.interner.kind(ty)
        else {
            return Err(self.error(origin, MirBuildErrorKind::InvalidAggregateType(ty)));
        };
        let item_id = self
            .types
            .aggregates
            .item(definition)
            .ok_or_else(|| self.error(origin, MirBuildErrorKind::InvalidAggregateType(ty)))?;
        let item = self.package.items[item_id];
        let (kind, parameters) = match item.kind {
            HirItemKind::Struct(structure) => (MirAggregateKind::Struct, structure.type_parameters),
            HirItemKind::Enum(enumeration) => (MirAggregateKind::Enum, enumeration.type_parameters),
            _ => return Err(self.error(origin, MirBuildErrorKind::InvalidAggregateType(ty))),
        };
        let parameters = self.package.type_parameters(parameters).to_vec();
        let concrete_arguments = self.types.interner.list(arguments).to_vec();
        if parameters.len() != concrete_arguments.len() {
            return Err(self.error(origin, MirBuildErrorKind::InvalidAggregateType(ty)));
        }
        let mut substitution = TypeSubstitution::new();
        for (parameter, argument) in parameters.iter().zip(concrete_arguments) {
            substitution.insert(*parameter, argument);
        }
        self.check_next(
            MirCapacity::Aggregates,
            self.counts.aggregates,
            self.options.max_aggregates,
            origin,
        )?;
        let instance = self
            .types
            .instances
            .instance(InstanceKey::new(item_id, arguments))
            .filter(|instance| {
                self.types.instances.records()[instance.index()].kind
                    == InstanceRequestKind::Required
            });
        let aggregate = self
            .program
            .aggregates
            .alloc(MirAggregate {
                source: item_id,
                definition,
                instance,
                arguments,
                ty,
                kind,
                fields: ListRange::empty(),
                variants: ListRange::empty(),
                origin: item.origin,
            })
            .map_err(|_| self.error(origin, MirBuildErrorKind::Capacity(MirCapacity::Storage)))?;
        self.counts.aggregates += 1;
        self.aggregate_types.insert(ty, aggregate);

        match item.kind {
            HirItemKind::Struct(structure) => {
                let source_fields = self.package.fields(structure.fields).to_vec();
                let mut fields = Vec::with_capacity(source_fields.len());
                for source in source_fields {
                    let field =
                        self.alloc_aggregate_field(aggregate, None, source, &substitution, origin)?;
                    self.aggregate_fields.insert((aggregate, source), field);
                    fields.push(field);
                }
                let fields = self
                    .program
                    .lists
                    .fields
                    .extend(&fields)
                    .map_err(|error| self.storage_error(origin, error))?;
                self.program
                    .aggregate_mut(aggregate)
                    .expect("new aggregate descriptor exists")
                    .fields = fields;
            }
            HirItemKind::Enum(enumeration) => {
                let source_variants = self.package.variants(enumeration.variants).to_vec();
                let mut variants = Vec::with_capacity(source_variants.len());
                for (ordinal, source) in source_variants.into_iter().enumerate() {
                    self.check_next(
                        MirCapacity::AggregateVariants,
                        self.counts.aggregate_variants,
                        self.options.max_aggregate_variants,
                        self.package.variants[source].origin,
                    )?;
                    let variant = self
                        .program
                        .variants
                        .alloc(MirVariant {
                            aggregate,
                            source,
                            ordinal: u32::try_from(ordinal).map_err(|_| {
                                self.error(
                                    origin,
                                    MirBuildErrorKind::Capacity(MirCapacity::AggregateVariants),
                                )
                            })?,
                            fields: ListRange::empty(),
                            origin: self.package.variants[source].origin,
                        })
                        .map_err(|_| {
                            self.error(origin, MirBuildErrorKind::Capacity(MirCapacity::Storage))
                        })?;
                    self.counts.aggregate_variants += 1;
                    self.aggregate_variants.insert((aggregate, source), variant);
                    let source_fields = self
                        .package
                        .fields(self.package.variants[source].fields)
                        .to_vec();
                    let mut fields = Vec::with_capacity(source_fields.len());
                    for source_field in source_fields {
                        let field = self.alloc_aggregate_field(
                            aggregate,
                            Some(variant),
                            source_field,
                            &substitution,
                            origin,
                        )?;
                        self.aggregate_fields
                            .insert((aggregate, source_field), field);
                        fields.push(field);
                    }
                    let fields = self
                        .program
                        .lists
                        .fields
                        .extend(&fields)
                        .map_err(|error| self.storage_error(origin, error))?;
                    self.program
                        .variant_mut(variant)
                        .expect("new variant descriptor exists")
                        .fields = fields;
                    variants.push(variant);
                }
                let variants = self
                    .program
                    .lists
                    .variants
                    .extend(&variants)
                    .map_err(|error| self.storage_error(origin, error))?;
                self.program
                    .aggregate_mut(aggregate)
                    .expect("new aggregate descriptor exists")
                    .variants = variants;
            }
            _ => unreachable!("aggregate descriptors are built only for structs and enums"),
        }
        Ok(aggregate)
    }

    /// Ensure every nominal reachable through a concrete value type has a MIR
    /// descriptor. Some valid programs mention an aggregate only as a
    /// container element or function-signature component and therefore never
    /// trigger constructor/field lowering for that aggregate directly.
    pub(super) fn ensure_type_aggregates(
        &mut self,
        ty: TypeId,
        origin: OriginId,
    ) -> Result<(), MirBuildError> {
        match self.types.interner.kind(ty) {
            TypeKind::Nominal { .. } => {
                self.ensure_aggregate(ty, origin)?;
            }
            TypeKind::List(element) | TypeKind::Slice(element) | TypeKind::Task(element) => {
                self.ensure_type_aggregates(element, origin)?;
            }
            TypeKind::Tuple(elements) => {
                let elements = self.types.interner.list(elements).to_vec();
                for element in elements {
                    self.ensure_type_aggregates(element, origin)?;
                }
            }
            TypeKind::Map { key, value } => {
                self.ensure_type_aggregates(key, origin)?;
                self.ensure_type_aggregates(value, origin)?;
            }
            TypeKind::Function { parameters, result } => {
                let parameters = self.types.interner.list(parameters).to_vec();
                for parameter in parameters {
                    self.ensure_type_aggregates(parameter, origin)?;
                }
                self.ensure_type_aggregates(result, origin)?;
            }
            TypeKind::Primitive(_)
            | TypeKind::Never
            | TypeKind::Error
            | TypeKind::GenericParameter(_)
            | TypeKind::BoundVariable(_)
            | TypeKind::InferenceVariable(_)
            | TypeKind::UnresolvedName { .. } => {}
        }
        Ok(())
    }

    /// Close the aggregate-descriptor set over all concrete types that made it
    /// into MIR. Field recursion handles newly created descriptors, while this
    /// root pass covers signature-only and local-only nominal references.
    pub(super) fn ensure_program_aggregate_types(
        &mut self,
        fallback_origin: OriginId,
    ) -> Result<(), MirBuildError> {
        let mut roots = Vec::new();
        for (_, function) in self.program.functions() {
            roots.push((function.ty, function.origin));
            roots.push((function.return_type, function.origin));
        }
        for (_, local) in self.program.locals() {
            roots.push((local.ty, local.origin));
        }
        for (_, place) in self.program.places() {
            roots.push((place.ty, place.origin));
        }
        if roots.is_empty() {
            roots.push((
                self.types.interner.primitive(PrimitiveType::Void),
                fallback_origin,
            ));
        }
        for (ty, origin) in roots {
            self.ensure_type_aggregates(ty, origin)?;
        }
        Ok(())
    }

    fn alloc_aggregate_field(
        &mut self,
        aggregate: MirAggregateId,
        variant: Option<MirVariantId>,
        source: lpp_hir::FieldId,
        substitution: &TypeSubstitution,
        origin: OriginId,
    ) -> Result<MirFieldId, MirBuildError> {
        let source_field = self.package.fields[source];
        let template = self
            .types
            .assignments
            .type_ref(source_field.type_ref)
            .ok_or_else(|| {
                self.error(
                    source_field.origin,
                    MirBuildErrorKind::GenericTypeMaterialization,
                )
            })?;
        let ty = self.materialize_type(substitution, template, source_field.origin)?;
        // Ownership and backend layout need descriptors for nominal types even
        // when they appear only behind a container (for example
        // `List[Child]` initialized with `[]`). Materialize those transitive
        // aggregate instances while the concrete field type is available.
        self.ensure_type_aggregates(ty, source_field.origin)?;
        self.check_next(
            MirCapacity::AggregateFields,
            self.counts.aggregate_fields,
            self.options.max_aggregate_fields,
            origin,
        )?;
        let field = self
            .program
            .fields
            .alloc(MirField {
                aggregate,
                variant,
                source,
                ty,
                origin: source_field.origin,
            })
            .map_err(|_| self.error(origin, MirBuildErrorKind::Capacity(MirCapacity::Storage)))?;
        self.counts.aggregate_fields += 1;
        Ok(field)
    }
}
