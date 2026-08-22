//! Backend-neutral native aggregate layout.
//!
//! Cranelift used to own the canonical struct/tuple offsets and LLVM imported
//! them from the Cranelift module. That inverted dependency made backend parity
//! accidental. Both backends now consume this analysis-layer layout contract.

use crate::type_facts::AbiClass;
use crate::types::{StructTypeId, TypeRef, TypeTable};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldLayout {
    pub offset: usize,
    pub size: usize,
    pub align: usize,
    pub abi: AbiClass,
}

fn align_up(value: usize, align: usize) -> usize {
    debug_assert!(align.is_power_of_two());
    (value + align - 1) & !(align - 1)
}

pub fn type_size_align(ty: &TypeRef) -> (usize, usize) {
    ty.native_size_align()
}

fn fields_layout<'a>(
    types: impl IntoIterator<Item = &'a TypeRef>,
    start: usize,
) -> (Vec<FieldLayout>, usize) {
    let mut offset = start;
    let mut aggregate_align = if start == 0 { 1 } else { 8 };
    let mut fields = Vec::new();
    for ty in types {
        let (size, align) = type_size_align(ty);
        offset = align_up(offset, align);
        fields.push(FieldLayout {
            offset,
            size,
            align,
            abi: ty.abi_class(),
        });
        offset += size;
        aggregate_align = aggregate_align.max(align);
    }
    (fields, align_up(offset, aggregate_align))
}

/// Tuple payload layout. The first two words are runtime ownership metadata.
pub fn tuple_layout(types: &[TypeRef]) -> (Vec<FieldLayout>, usize) {
    fields_layout(types.iter(), 16)
}

/// ARC mask and four packed 16-bit offsets consumed by `lpp_tuple_destroy`.
pub fn tuple_runtime_metadata(types: &[TypeRef]) -> (u64, u64) {
    let (layout, _) = tuple_layout(types);
    let mut mask = 0u64;
    let mut offsets = 0u64;
    for (index, (ty, field)) in types.iter().zip(layout.iter()).enumerate() {
        if ty.is_managed() {
            mask |= 1u64 << index;
        }
        debug_assert!(field.offset <= u16::MAX as usize);
        offsets |= (field.offset as u64) << (index * 16);
    }
    (mask, offsets)
}

pub fn struct_layout(table: &TypeTable, id: StructTypeId) -> (Vec<FieldLayout>, usize) {
    let def = &table.definitions[id.0];
    if def.repr_exact {
        let mut offset = 0;
        let mut fields = Vec::new();
        for (_, ty) in &def.fields {
            let (size, _) = type_size_align(ty);
            fields.push(FieldLayout {
                offset,
                size,
                align: 1,
                abi: ty.abi_class(),
            });
            offset += size;
        }
        let total_align = def.align.unwrap_or(1);
        (fields, align_up(offset, total_align))
    } else {
        let (fields, size) = fields_layout(def.fields.iter().map(|(_, ty)| ty), 0);
        if let Some(align) = def.align {
            (fields, align_up(size, align))
        } else {
            (fields, size)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_layout_matches_the_native_abi() {
        assert_eq!(type_size_align(&TypeRef::Bool), (1, 1));
        assert_eq!(type_size_align(&TypeRef::Int), (8, 8));
        assert_eq!(type_size_align(&TypeRef::Float), (8, 8));
    }

    #[test]
    fn tuple_layout_respects_mixed_width_alignment() {
        let (fields, size) = tuple_layout(&[TypeRef::Bool, TypeRef::Bool, TypeRef::Int]);
        assert_eq!(fields[0].offset, 16);
        assert_eq!(fields[1].offset, 17);
        assert_eq!(fields[2].offset, 24);
        assert_eq!(size, 32);
    }

    #[test]
    fn tuple_metadata_marks_only_managed_children() {
        let (mask, offsets) = tuple_runtime_metadata(&[TypeRef::Int, TypeRef::Str]);
        assert_eq!(mask, 0b10);
        assert_eq!((offsets >> 16) & 0xffff, 24);
    }

    #[test]
    fn struct_layout_exact_and_align() {
        let mut table = TypeTable::new();
        let id = table.register_struct("Bucket".to_string());
        table.definitions[id.0].fields = vec![
            ("tag".to_string(), TypeRef::Int),
            ("key".to_string(), TypeRef::Int),
            ("flag".to_string(), TypeRef::Bool),
        ];
        table.definitions[id.0].repr_exact = true;
        table.definitions[id.0].align = Some(64);

        let (fields, total_size) = struct_layout(&table, id);
        assert_eq!(fields[0].offset, 0);
        assert_eq!(fields[1].offset, 8);
        assert_eq!(fields[2].offset, 16);
        assert_eq!(total_size, 64);
    }
}
