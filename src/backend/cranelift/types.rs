use crate::typecheck::{StructTypeId, TypeRef, TypeTable};
use cranelift_codegen::ir::types as cl_types;

/// Maps a L++ TypeRef to a Cranelift IR type.
pub fn type_to_cl(ty: &TypeRef) -> cranelift_codegen::ir::Type {
    match ty {
        TypeRef::Int => cl_types::I64,
        TypeRef::Float => cl_types::F64,
        TypeRef::Bool => cl_types::I8,
        TypeRef::Str => cl_types::I64,  // pointer (MVP: null for now)
        TypeRef::Void => cl_types::I64, // dummy; callers check != Void
        TypeRef::Custom(_) => cl_types::I64, // opaque struct pointer
        TypeRef::Generic(_, _) => cl_types::I64, // opaque container pointer
        TypeRef::Unresolved(_) => cl_types::I64, // not yet resolved; treat as ptr
        TypeRef::Function => cl_types::I64, // function pointer placeholder
    }
}

/// Native layout used by the AOT backend. L++ currently targets 64-bit hosts;
/// pointer-like values therefore occupy one 64-bit word. Keep layout decisions
/// here so allocation and field access cannot silently disagree.
#[derive(Debug, Clone, Copy)]
pub struct FieldLayout {
    pub offset: usize,
    pub ty: cranelift_codegen::ir::Type,
}

fn align_up(value: usize, align: usize) -> usize {
    debug_assert!(align.is_power_of_two());
    (value + align - 1) & !(align - 1)
}

pub fn type_size_align(ty: &TypeRef) -> (usize, usize) {
    match ty {
        TypeRef::Bool => (1, 1),
        TypeRef::Int
        | TypeRef::Float
        | TypeRef::Str
        | TypeRef::Custom(_)
        | TypeRef::Generic(_, _)
        | TypeRef::Unresolved(_)
        | TypeRef::Function
        | TypeRef::Void => (8, 8),
    }
}

/// Return each field's offset and machine type, plus the padded allocation size.
pub fn struct_layout(table: &TypeTable, id: StructTypeId) -> (Vec<FieldLayout>, usize) {
    let def = &table.definitions[id.0];
    let mut offset = 0usize;
    let mut struct_align = 1usize;
    let mut fields = Vec::with_capacity(def.fields.len());
    for (_, ty) in &def.fields {
        let (size, align) = type_size_align(ty);
        offset = align_up(offset, align);
        fields.push(FieldLayout {
            offset,
            ty: type_to_cl(ty),
        });
        offset += size;
        struct_align = struct_align.max(align);
    }
    (fields, align_up(offset, struct_align))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_layout_matches_the_aot_abi() {
        assert_eq!(type_size_align(&TypeRef::Bool), (1, 1));
        assert_eq!(type_size_align(&TypeRef::Int), (8, 8));
        assert_eq!(type_size_align(&TypeRef::Float), (8, 8));
        assert_eq!(type_to_cl(&TypeRef::Bool), cl_types::I8);
        assert_eq!(type_to_cl(&TypeRef::Float), cl_types::F64);
    }

    #[test]
    fn alignment_rounds_up_correctly() {
        assert_eq!(align_up(0, 8), 0);
        assert_eq!(align_up(1, 8), 8);
        assert_eq!(align_up(8, 8), 8);
        assert_eq!(align_up(9, 8), 16);
    }
}
