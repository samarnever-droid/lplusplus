use super::*;

impl<'hir> TypeChecker<'hir> {
    /// Returns a static diagnostic label when `name` is an unsafe low-level
    /// intrinsic that must execute inside `unsafe:` or `unsafe def`.
    #[must_use]
    pub(in crate::check) fn unsafe_intrinsic_label(name: &str) -> Option<&'static str> {
        Some(match name {
            "ptr_read" => "ptr_read",
            "ptr_write" => "ptr_write",
            "ptr_offset" => "ptr_offset",
            "ptr_add" => "ptr_add",
            "mmio_read8" => "mmio_read8",
            "mmio_read16" => "mmio_read16",
            "mmio_read32" => "mmio_read32",
            "mmio_read64" => "mmio_read64",
            "mmio_write8" => "mmio_write8",
            "mmio_write16" => "mmio_write16",
            "mmio_write32" => "mmio_write32",
            "mmio_write64" => "mmio_write64",
            "port_in8" => "port_in8",
            "port_in16" => "port_in16",
            "port_in32" => "port_in32",
            "port_out8" => "port_out8",
            "port_out16" => "port_out16",
            "port_out32" => "port_out32",
            "cpu_cli" => "cpu_cli",
            "cpu_sti" => "cpu_sti",
            "cpu_hlt" => "cpu_hlt",
            "memory_barrier" => "memory_barrier",
            "rdtsc" => "rdtsc",
            "read_cr0" => "read_cr0",
            "read_cr2" => "read_cr2",
            "read_cr3" => "read_cr3",
            "write_cr3" => "write_cr3",
            "read_cr4" => "read_cr4",
            "write_cr4" => "write_cr4",
            "rdmsr" => "rdmsr",
            "wrmsr" => "wrmsr",
            "lgdt" => "lgdt",
            "lidt" => "lidt",
            "invlpg" => "invlpg",
            "memcpy_raw" => "memcpy_raw",
            "memset_raw" => "memset_raw",
            "memmove_raw" => "memmove_raw",
            "memcmp_raw" => "memcmp_raw",
            _ => return None,
        })
    }

    fn make_fn_type(
        &mut self,
        params: &[TypeId],
        result: TypeId,
        origin: OriginId,
    ) -> Result<Option<TypeId>, ShadowTypeError> {
        let parameters = self
            .interner
            .intern_list(params)
            .map_err(|error| self.at(origin, error.into()))?;
        let ty = self
            .interner
            .intern(TypeKind::Function { parameters, result })
            .map_err(|error| self.at(origin, error.into()))?;
        Ok(Some(ty))
    }

    fn single_intrinsic_type_arg(
        &mut self,
        explicit_type_args: Option<&[TypeId]>,
        origin: OriginId,
    ) -> Result<TypeId, ShadowTypeError> {
        if let Some(args) = explicit_type_args {
            if args.len() != 1 {
                return Err(self.at(
                    origin,
                    TypeError::ArityMismatch {
                        expected: 1,
                        actual: args.len(),
                    },
                ));
            }
            Ok(args[0])
        } else {
            self.fresh(InferenceLevel::ROOT.child(), origin)
        }
    }

    /// Resolve compiler-native low-level/kernel intrinsics (type-level signatures).
    /// Returns `Ok(None)` when `name` is not a compiler intrinsic.
    pub(in crate::check) fn intrinsic_function_type(
        &mut self,
        name: &str,
        explicit_type_args: Option<&[TypeId]>,
        origin: OriginId,
    ) -> Result<Option<TypeId>, ShadowTypeError> {
        match name {
            "size_of" | "align_of" => {
                let _elem = self.single_intrinsic_type_arg(explicit_type_args, origin)?;
                let usize_ty = self.interner.primitive(PrimitiveType::USize);
                self.make_fn_type(&[], usize_ty, origin)
            }
            "ptr_null" => {
                let elem = self.single_intrinsic_type_arg(explicit_type_args, origin)?;
                let ptr_ty = self
                    .interner
                    .intern(TypeKind::Ptr(elem))
                    .map_err(|error| self.at(origin, error.into()))?;
                self.make_fn_type(&[], ptr_ty, origin)
            }
            "ptr_is_null" => {
                let elem = self.single_intrinsic_type_arg(explicit_type_args, origin)?;
                let ptr_ty = self
                    .interner
                    .intern(TypeKind::Ptr(elem))
                    .map_err(|error| self.at(origin, error.into()))?;
                let bool_ty = self.interner.primitive(PrimitiveType::Bool);
                self.make_fn_type(&[ptr_ty], bool_ty, origin)
            }
            "ptr_read" => {
                let elem = self.single_intrinsic_type_arg(explicit_type_args, origin)?;
                let ptr_ty = self
                    .interner
                    .intern(TypeKind::Ptr(elem))
                    .map_err(|error| self.at(origin, error.into()))?;
                self.make_fn_type(&[ptr_ty], elem, origin)
            }
            "ptr_write" => {
                let elem = self.single_intrinsic_type_arg(explicit_type_args, origin)?;
                let ptr_ty = self
                    .interner
                    .intern(TypeKind::Ptr(elem))
                    .map_err(|error| self.at(origin, error.into()))?;
                let void_ty = self.interner.primitive(PrimitiveType::Void);
                self.make_fn_type(&[ptr_ty, elem], void_ty, origin)
            }
            "ptr_offset" => {
                let elem = self.single_intrinsic_type_arg(explicit_type_args, origin)?;
                let ptr_ty = self
                    .interner
                    .intern(TypeKind::Ptr(elem))
                    .map_err(|error| self.at(origin, error.into()))?;
                let isize_ty = self.interner.primitive(PrimitiveType::ISize);
                self.make_fn_type(&[ptr_ty, isize_ty], ptr_ty, origin)
            }
            "ptr_add" => {
                let elem = self.single_intrinsic_type_arg(explicit_type_args, origin)?;
                let ptr_ty = self
                    .interner
                    .intern(TypeKind::Ptr(elem))
                    .map_err(|error| self.at(origin, error.into()))?;
                let usize_ty = self.interner.primitive(PrimitiveType::USize);
                self.make_fn_type(&[ptr_ty, usize_ty], ptr_ty, origin)
            }
            _ => {
                if let Some((params, ret)) = monomorphic_intrinsic_sig(name) {
                    if let Some(args) = explicit_type_args
                        && !args.is_empty()
                    {
                        return Err(self.at(
                            origin,
                            TypeError::ArityMismatch {
                                expected: 0,
                                actual: args.len(),
                            },
                        ));
                    }
                    let param_tys: Vec<TypeId> =
                        params.iter().map(|p| self.interner.primitive(*p)).collect();
                    let ret_ty = self.interner.primitive(ret);
                    return self.make_fn_type(&param_tys, ret_ty, origin);
                }
                if name == "memcpy_raw" || name == "memmove_raw" {
                    let u8_ty = self.interner.primitive(PrimitiveType::U8);
                    let ptr_u8 = self
                        .interner
                        .intern(TypeKind::Ptr(u8_ty))
                        .map_err(|error| self.at(origin, error.into()))?;
                    let usize_ty = self.interner.primitive(PrimitiveType::USize);
                    let void_ty = self.interner.primitive(PrimitiveType::Void);
                    return self.make_fn_type(&[ptr_u8, ptr_u8, usize_ty], void_ty, origin);
                }
                if name == "memset_raw" {
                    let u8_ty = self.interner.primitive(PrimitiveType::U8);
                    let ptr_u8 = self
                        .interner
                        .intern(TypeKind::Ptr(u8_ty))
                        .map_err(|error| self.at(origin, error.into()))?;
                    let usize_ty = self.interner.primitive(PrimitiveType::USize);
                    let void_ty = self.interner.primitive(PrimitiveType::Void);
                    return self.make_fn_type(&[ptr_u8, u8_ty, usize_ty], void_ty, origin);
                }
                if name == "memcmp_raw" {
                    let u8_ty = self.interner.primitive(PrimitiveType::U8);
                    let ptr_u8 = self
                        .interner
                        .intern(TypeKind::Ptr(u8_ty))
                        .map_err(|error| self.at(origin, error.into()))?;
                    let usize_ty = self.interner.primitive(PrimitiveType::USize);
                    let int_ty = self.interner.primitive(PrimitiveType::Int);
                    return self.make_fn_type(&[ptr_u8, ptr_u8, usize_ty], int_ty, origin);
                }
                Ok(None)
            }
        }
    }
}

fn monomorphic_intrinsic_sig(name: &str) -> Option<(&'static [PrimitiveType], PrimitiveType)> {
    use PrimitiveType::*;
    Some(match name {
        "mmio_read8" => (&[USize], U8),
        "mmio_read16" => (&[USize], U16),
        "mmio_read32" => (&[USize], U32),
        "mmio_read64" => (&[USize], U64),
        "mmio_write8" => (&[USize, U8], Void),
        "mmio_write16" => (&[USize, U16], Void),
        "mmio_write32" => (&[USize, U32], Void),
        "mmio_write64" => (&[USize, U64], Void),
        "port_in8" => (&[U16], U8),
        "port_in16" => (&[U16], U16),
        "port_in32" => (&[U16], U32),
        "port_out8" => (&[U16, U8], Void),
        "port_out16" => (&[U16, U16], Void),
        "port_out32" => (&[U16, U32], Void),
        "cpu_cli" | "cpu_sti" | "cpu_hlt" | "memory_barrier" => (&[], Void),
        "rdtsc" | "read_cr0" | "read_cr2" | "read_cr3" | "read_cr4" => (&[], U64),
        "write_cr3" | "write_cr4" => (&[U64], Void),
        "rdmsr" => (&[U32], U64),
        "wrmsr" => (&[U32, U64], Void),
        "lgdt" | "lidt" | "invlpg" => (&[USize], Void),
        _ => return None,
    })
}
