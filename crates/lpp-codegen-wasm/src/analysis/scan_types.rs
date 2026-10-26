use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SliceKind {
    List(TypeId),
    String,
}

pub(crate) fn slice_kind_of(
    program: &MirProgram,
    types: &TypeInterner,
    operand: &Operand,
    function: MirFunctionId,
) -> Result<SliceKind, CodegenError> {
    let Operand::Copy(local) = operand else {
        return Err(unsupported(
            "slice argument must be a local",
            Some(function),
        ));
    };
    let ty = program
        .local(*local)
        .ok_or_else(|| unsupported("unknown slice local", Some(function)))?
        .ty;
    match types.kind(ty) {
        TypeKind::Slice(element) => Ok(SliceKind::List(element)),
        TypeKind::Primitive(PrimitiveType::StrSlice) => Ok(SliceKind::String),
        _ => Err(unsupported("slice argument expected", Some(function))),
    }
}

/// The element type of a list operand: a `Copy` of a local of list type.
pub(crate) fn list_element_of(
    lower: &ScanLower<'_>,
    operand: &Operand,
) -> Result<TypeId, CodegenError> {
    let Operand::Copy(local) = operand else {
        return Err(unsupported(
            "list argument must be a local",
            Some(lower.fn_id),
        ));
    };
    let ty = lower.program.local(*local).unwrap().ty;
    match lower.types.kind(ty) {
        TypeKind::List(element) => Ok(element),
        _ => Err(unsupported("list argument expected", Some(lower.fn_id))),
    }
}

pub(crate) fn check_terminator(
    lower: &mut ScanLower<'_>,
    terminator: &Terminator,
) -> Result<(), CodegenError> {
    match terminator {
        Terminator::Goto(_) | Terminator::Branch { .. } | Terminator::Unreachable => Ok(()),
        Terminator::Return(operand) => {
            // A returned function value materializes a capsule (the
            // retain/release bookkeeping is generic over managed
            // types); a returned string literal joins the pool.
            if let Some(op) = operand {
                if let Operand::Constant(Constant::String { string, .. }) = op
                    && let Some(content) = lower.program.string(*string)
                {
                    lower.strings.insert(content.clone());
                }
                scan_operand_class(lower, op)?;
            }
            Ok(())
        }
        Terminator::SwitchEnum { subject, .. } => {
            if scan_operand_class(lower, subject)? != Class::Ptr {
                return Err(unsupported("switch on non-managed type", Some(lower.fn_id)));
            }
            *lower.has_managed = true;
            Ok(())
        }
    }
}

pub(crate) fn require_str(
    lower: &mut ScanLower<'_>,
    operand: &Operand,
) -> Result<(), CodegenError> {
    if scan_operand_class(lower, operand)? == Class::Str {
        Ok(())
    } else {
        Err(unsupported("string argument expected", Some(lower.fn_id)))
    }
}

pub(crate) fn require_i64(
    lower: &mut ScanLower<'_>,
    operand: &Operand,
) -> Result<(), CodegenError> {
    if scan_operand_class(lower, operand)? == Class::I64 {
        Ok(())
    } else {
        Err(unsupported("integer argument expected", Some(lower.fn_id)))
    }
}

pub(crate) fn require_bool(
    lower: &mut ScanLower<'_>,
    operand: &Operand,
) -> Result<(), CodegenError> {
    if scan_operand_class(lower, operand)? == Class::Bool {
        Ok(())
    } else {
        Err(unsupported("boolean argument expected", Some(lower.fn_id)))
    }
}

pub(crate) fn require_f64(
    lower: &mut ScanLower<'_>,
    operand: &Operand,
) -> Result<(), CodegenError> {
    if scan_operand_class(lower, operand)? == Class::F64 {
        Ok(())
    } else {
        Err(unsupported("float argument expected", Some(lower.fn_id)))
    }
}

// ---------------------------------------------------------------------------
// Type / class mapping.
// ---------------------------------------------------------------------------
