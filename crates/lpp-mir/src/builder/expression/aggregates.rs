use super::*;

impl FunctionBuilder<'_, '_> {
    /// Synthesize the zero value for every field of a struct aggregate — the
    /// MIR side of v1's zero-argument constructor (`Box()` zero-initializes
    /// each field, verified against the v1 oracle: Int->0, Float->0.0,
    /// Bool->false, Str->""). Primitive fields get their zero constant; a
    /// non-primitive field (nested struct, list, map, …) is a structured
    /// `Unsupported` failure for now, keeping this slice bounded.
    pub(in crate::builder) fn zero_init_struct_fields(
        &mut self,
        aggregate: MirAggregateId,
        origin: OriginId,
    ) -> Result<ListRange<Operand>, MirBuildError> {
        // Phase 1: read the concrete field types (immutable borrows only).
        let field_types: Vec<TypeId> = {
            let descriptor = self
                .core
                .program
                .aggregate(aggregate)
                .expect("the constructor's aggregate descriptor was just ensured");
            let fields = self.core.program.aggregate_fields(descriptor);
            let mut types = Vec::with_capacity(fields.len());
            for &field in fields {
                let field = self
                    .core
                    .program
                    .field(field)
                    .expect("aggregate fields reference live field descriptors");
                types.push(field.ty);
            }
            types
        };
        // Phase 2: build one zero value per field (recursively, so a nested
        // struct / list / tuple field gets a real default rather than a
        // rejection).
        let mut zeros = Vec::with_capacity(field_types.len());
        for ty in field_types {
            zeros.push(self.zero_value(ty, origin)?);
        }
        self.store_operands(&zeros, origin)
    }

    /// The default ("zero") value for a type, matching v1's zero-argument
    /// constructor: Int→0, Float→0.0, Bool→false, Char→'\0', Str→"",
    /// List→empty, a nested struct→its own zero-initialized value, and a
    /// tuple→a tuple of its elements' zero values. Non-defaultable types
    /// (enums, maps, …) remain a structured `Unsupported(Field)` failure.
    pub(in crate::builder) fn zero_value(
        &mut self,
        ty: TypeId,
        origin: OriginId,
    ) -> Result<Operand, MirBuildError> {
        let ty = self.concrete_type(ty, origin)?;
        match self.core.types.interner.kind(ty) {
            TypeKind::Primitive(PrimitiveType::Int) => Ok(Operand::Constant(Constant::Integer(0))),
            TypeKind::Primitive(PrimitiveType::Float) => {
                Ok(Operand::Constant(Constant::FloatBits(0)))
            }
            TypeKind::Primitive(PrimitiveType::Bool) => {
                Ok(Operand::Constant(Constant::Bool(false)))
            }
            TypeKind::Primitive(PrimitiveType::Char) => {
                Ok(Operand::Constant(Constant::Character {
                    origin,
                    character: '\0',
                }))
            }
            TypeKind::Primitive(PrimitiveType::String) => {
                let string = self.core.intern_string(String::new(), origin)?;
                Ok(Operand::Constant(Constant::String { origin, string }))
            }
            TypeKind::List(_) => {
                let operands = self.store_operands(&[], origin)?;
                self.assign_temporary(ty, Rvalue::List(operands), origin)
            }
            TypeKind::Nominal { .. } => {
                let aggregate = self.core.ensure_aggregate(ty, origin)?;
                let kind = self
                    .core
                    .program
                    .aggregate(aggregate)
                    .expect("ensure_aggregate yields a live aggregate")
                    .kind;
                if kind != MirAggregateKind::Struct {
                    return Err(self.core.error(
                        origin,
                        MirBuildErrorKind::Unsupported(UnsupportedConstruct::Field),
                    ));
                }
                let fields = self.zero_init_struct_fields(aggregate, origin)?;
                self.assign_temporary(ty, Rvalue::ConstructStruct { aggregate, fields }, origin)
            }
            TypeKind::Tuple(elements) => {
                let element_types = self.core.types.interner.list(elements).to_vec();
                let mut operands = Vec::with_capacity(element_types.len());
                for element in element_types {
                    operands.push(self.zero_value(element, origin)?);
                }
                let operands = self.store_operands(&operands, origin)?;
                self.assign_temporary(ty, Rvalue::Tuple(operands), origin)
            }
            _ => Err(self.core.error(
                origin,
                MirBuildErrorKind::Unsupported(UnsupportedConstruct::Field),
            )),
        }
    }
}
