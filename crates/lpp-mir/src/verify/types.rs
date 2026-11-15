use super::*;

mod places;
mod rvalues;

impl Verifier<'_> {
    fn verify_aggregate_operands(
        &mut self,
        site: InstructionSite,
        expected: &[MirFieldId],
        actual: crate::ListRange<Operand>,
        locals: &BTreeSet<MirLocalId>,
    ) -> Option<()> {
        let actual = self.program.operands(actual);
        if expected.len() != actual.len() {
            self.push(
                site.function,
                Some(site.block),
                Some(site.instruction),
                site.origin,
                MirVerificationErrorKind::ArityMismatch {
                    expected: expected.len(),
                    actual: actual.len(),
                },
            );
            return None;
        }
        for (field, operand) in expected.iter().zip(actual) {
            let Some(field) = self.program.field(*field) else {
                self.push(
                    site.function,
                    Some(site.block),
                    Some(site.instruction),
                    site.origin,
                    MirVerificationErrorKind::MissingReference(MirEntity::Field(*field)),
                );
                return None;
            };
            let actual = self.operand_type(
                site.function,
                site.block,
                Some(site.instruction),
                site.origin,
                *operand,
                locals,
            )?;
            if actual != field.ty && !self.integer_operand_assignable(*operand, field.ty) {
                self.push(
                    site.function,
                    Some(site.block),
                    Some(site.instruction),
                    site.origin,
                    MirVerificationErrorKind::TypeMismatch {
                        expected: field.ty,
                        actual,
                    },
                );
                return None;
            }
        }
        Some(())
    }

    pub(super) fn operand_type(
        &mut self,
        function: MirFunctionId,
        block: BasicBlockId,
        instruction: Option<InstructionId>,
        origin: OriginId,
        operand: Operand,
        locals: &BTreeSet<MirLocalId>,
    ) -> Option<TypeId> {
        match operand {
            Operand::Copy(local) => match self.program.local(local) {
                Some(value) if locals.contains(&local) => Some(value.ty),
                Some(_) => {
                    self.push(
                        function,
                        Some(block),
                        instruction,
                        origin,
                        MirVerificationErrorKind::CrossFunctionReference(MirEntity::Local(local)),
                    );
                    None
                }
                None => {
                    self.push(
                        function,
                        Some(block),
                        instruction,
                        origin,
                        MirVerificationErrorKind::MissingReference(MirEntity::Local(local)),
                    );
                    None
                }
            },
            Operand::Function(target) => match self.program.function(target) {
                Some(target) => Some(target.ty),
                None => {
                    self.push(
                        function,
                        Some(block),
                        instruction,
                        origin,
                        MirVerificationErrorKind::MissingReference(MirEntity::Function(target)),
                    );
                    None
                }
            },
            Operand::Constant(constant) => Some(match constant {
                crate::Constant::Integer(_) => self.types.primitive(PrimitiveType::Int),
                crate::Constant::FloatBits(_) => self.types.primitive(PrimitiveType::Float),
                crate::Constant::String { .. } => self.types.primitive(PrimitiveType::String),
                crate::Constant::Character { .. } => self.types.primitive(PrimitiveType::Char),
                crate::Constant::Bool(_) => self.types.primitive(PrimitiveType::Bool),
            }),
        }
    }

    pub(super) fn reference_error(
        &mut self,
        function: MirFunctionId,
        block: BasicBlockId,
        instruction: InstructionId,
        origin: OriginId,
        entity: MirEntity,
        exists: bool,
    ) {
        self.push(
            function,
            Some(block),
            Some(instruction),
            origin,
            if exists {
                MirVerificationErrorKind::CrossFunctionReference(entity)
            } else {
                MirVerificationErrorKind::MissingReference(entity)
            },
        );
    }

    pub(super) fn integer_operand_assignable(&self, operand: Operand, target: TypeId) -> bool {
        match operand {
            Operand::Constant(crate::Constant::Integer(value)) => {
                self.integer_literal_assignable(value, target)
            }
            Operand::Copy(_) | Operand::Function(_) | Operand::Constant(_) => false,
        }
    }

    fn integer_literal_assignable(&self, value: i64, target: TypeId) -> bool {
        match self.types.kind(target) {
            TypeKind::Primitive(PrimitiveType::Int) => true,
            TypeKind::Primitive(PrimitiveType::U8) => u8::try_from(value).is_ok(),
            TypeKind::Primitive(PrimitiveType::U16) => u16::try_from(value).is_ok(),
            TypeKind::Primitive(PrimitiveType::U32) => u32::try_from(value).is_ok(),
            TypeKind::Primitive(PrimitiveType::U64 | PrimitiveType::USize) => value >= 0,
            TypeKind::Primitive(PrimitiveType::I8) => i8::try_from(value).is_ok(),
            TypeKind::Primitive(PrimitiveType::I16) => i16::try_from(value).is_ok(),
            TypeKind::Primitive(PrimitiveType::I32) => i32::try_from(value).is_ok(),
            TypeKind::Primitive(PrimitiveType::ISize) => true,
            TypeKind::Primitive(PrimitiveType::RawPtr) | TypeKind::Ptr(_) => value == 0,
            // Legacy L++ source uses integer zero for a null aggregate edge,
            // most notably as the terminal link of recursive structures.
            TypeKind::Nominal { .. } => value == 0,
            _ => false,
        }
    }

    fn is_numeric(&self, ty: TypeId) -> bool {
        self.is_integral(ty) || self.types.kind(ty) == TypeKind::Primitive(PrimitiveType::Float)
    }

    pub(super) fn is_integral(&self, ty: TypeId) -> bool {
        matches!(
            self.types.kind(ty),
            TypeKind::Primitive(
                PrimitiveType::Int
                    | PrimitiveType::U8
                    | PrimitiveType::U16
                    | PrimitiveType::U32
                    | PrimitiveType::U64
                    | PrimitiveType::I8
                    | PrimitiveType::I16
                    | PrimitiveType::I32
                    | PrimitiveType::USize
                    | PrimitiveType::ISize
            )
        )
    }

    fn is_addable(&self, ty: TypeId) -> bool {
        self.is_numeric(ty) || self.types.kind(ty) == TypeKind::Primitive(PrimitiveType::String)
    }

    fn is_ordered(&self, ty: TypeId) -> bool {
        self.is_addable(ty) || self.types.kind(ty) == TypeKind::Primitive(PrimitiveType::Char)
    }
}
