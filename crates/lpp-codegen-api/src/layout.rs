//! Shared aggregate layout for the managed data surface (5C native,
//! 5D2a wasm).
//!
//! Struct payloads follow the v1 `struct_layout` rule: fields in
//! declaration order with native alignment from offset 0. Enum payloads
//! are rewrite-defined (v1 predates enums): a variant-ordinal tag at
//! offset 0 (stored as `I64` on native, as `I32` on wasm — bytes 4..8
//! are padding either way), then the variant's fields laid out from
//! offset 8. List payloads are runtime-managed (`LppList`); the
//! backend only holds the pointer.

use std::collections::BTreeMap;

use lpp_mir::{MirAggregate, MirAggregateId, MirAggregateKind, MirFieldId, MirProgram};
use lpp_types::{TypeId, TypeInterner};

/// One field slot: its field, payload offset, and value type.
#[derive(Debug, Clone, Copy)]
pub struct FieldSlot {
    pub field: MirFieldId,
    pub offset: u32,
    pub ty: TypeId,
}

/// The resolved layout of one nominal aggregate.
#[derive(Debug, Clone)]
pub struct AggregateLayout {
    pub kind: MirAggregateKind,
    /// The allocation size (the 24-byte ARC header is runtime-side).
    pub total_size: u32,
    /// Struct fields, in declaration order (offsets from 0).
    pub struct_fields: Vec<FieldSlot>,
    /// Enum variant fields indexed by variant ordinal (offsets from
    /// 8). The tag itself occupies bytes 0..8.
    pub variant_fields: Vec<Vec<FieldSlot>>,
}

const ENUM_TAG_OFFSET: u32 = 8;

fn type_size_align(types: &TypeInterner, ty: TypeId) -> (u32, u32) {
    match types.kind(ty) {
        lpp_types::TypeKind::Primitive(lpp_types::PrimitiveType::Int)
        | lpp_types::TypeKind::Primitive(lpp_types::PrimitiveType::Float)
        | lpp_types::TypeKind::Primitive(lpp_types::PrimitiveType::String)
        | lpp_types::TypeKind::List(_)
        | lpp_types::TypeKind::Nominal { .. } => (8, 8),
        lpp_types::TypeKind::Primitive(lpp_types::PrimitiveType::Bool)
        | lpp_types::TypeKind::Primitive(lpp_types::PrimitiveType::U8)
        | lpp_types::TypeKind::Primitive(lpp_types::PrimitiveType::I8) => (1, 1),
        lpp_types::TypeKind::Primitive(lpp_types::PrimitiveType::U16)
        | lpp_types::TypeKind::Primitive(lpp_types::PrimitiveType::I16) => (2, 2),
        lpp_types::TypeKind::Primitive(lpp_types::PrimitiveType::Char)
        | lpp_types::TypeKind::Primitive(lpp_types::PrimitiveType::U32)
        | lpp_types::TypeKind::Primitive(lpp_types::PrimitiveType::I32) => (4, 4),
        lpp_types::TypeKind::Tuple(elements) => {
            // A tuple is a flat record of its elements: each is aligned to its
            // own alignment and laid out in declaration order, and the whole is
            // rounded up to the widest element alignment. This mirrors
            // `field_layouts` so a tuple nested in a struct/enum payload has a
            // well-defined size instead of tripping the unreachable below.
            let mut offset = 0u32;
            let mut aggregate_align = 1u32;
            for &element in types.list(elements) {
                let (size, align) = type_size_align(types, element);
                offset = align_up(offset, align);
                offset += size;
                aggregate_align = aggregate_align.max(align);
            }
            (align_up(offset, aggregate_align), aggregate_align)
        }
        _ => unreachable!("5C value types are machine-representable"),
    }
}

fn align_up(value: u32, align: u32) -> u32 {
    (value + align - 1) & !(align - 1)
}

/// The layout of a structural tuple type: `(offset, element_type)` for each
/// element (a flat record from offset 0, the same rule as struct payloads) plus
/// the total allocation size. Returns `None` when `ty` is not a tuple.
#[must_use]
pub fn tuple_layout(types: &TypeInterner, ty: TypeId) -> Option<(Vec<(u32, TypeId)>, u32)> {
    let lpp_types::TypeKind::Tuple(elements) = types.kind(ty) else {
        return None;
    };
    let mut offset = 0u32;
    let mut aggregate_align = 1u32;
    let mut slots = Vec::new();
    for &element in types.list(elements) {
        let (size, align) = type_size_align(types, element);
        offset = align_up(offset, align);
        slots.push((offset, element));
        offset += size;
        aggregate_align = aggregate_align.max(align);
    }
    Some((slots, align_up(offset, aggregate_align)))
}

fn field_layouts(
    types: &TypeInterner,
    fields: &[(MirFieldId, TypeId)],
    start: u32,
) -> (Vec<FieldSlot>, u32) {
    let mut offset = start;
    // A payload that starts at 0 aligns to 1 (the v1 rule); an offset
    // payload (enum fields) is 8-aligned.
    let mut aggregate_align = if start == 0 { 1 } else { 8 };
    let mut slots = Vec::with_capacity(fields.len());
    for &(field, ty) in fields {
        let (size, align) = type_size_align(types, ty);
        offset = align_up(offset, align);
        slots.push(FieldSlot { field, offset, ty });
        offset += size;
        aggregate_align = aggregate_align.max(align);
    }
    (slots, align_up(offset, aggregate_align))
}

/// The layout of `aggregate` (struct or enum), from the program's own
/// field and variant descriptors.
pub fn aggregate_layout(
    program: &MirProgram,
    types: &TypeInterner,
    aggregate: MirAggregateId,
) -> AggregateLayout {
    let descriptor: &MirAggregate = program
        .aggregate(aggregate)
        .expect("validated MIR retains every aggregate");
    let kind = descriptor.kind;
    match kind {
        MirAggregateKind::Struct => {
            let fields = program.aggregate_fields(descriptor);
            let tys: Vec<(MirFieldId, TypeId)> = fields
                .iter()
                .map(|&field| {
                    let field_descriptor = program
                        .field(field)
                        .expect("validated MIR retains every field");
                    (field, field_descriptor.ty)
                })
                .collect();
            let (slots, total) = field_layouts(types, &tys, 0);
            AggregateLayout {
                kind,
                total_size: total,
                struct_fields: slots,
                variant_fields: Vec::new(),
            }
        }
        MirAggregateKind::Enum => {
            let variants = program.aggregate_variants(descriptor);
            let variant_count = variants.len();
            let mut variant_slots: Vec<Vec<FieldSlot>> = vec![Vec::new(); variant_count];
            let mut total = ENUM_TAG_OFFSET;
            for &variant_id in variants {
                let variant = program
                    .variant(variant_id)
                    .expect("validated MIR retains every variant");
                let ordinal = variant.ordinal as usize;
                assert!(
                    ordinal < variant_count && variant_slots[ordinal].is_empty(),
                    "variant ordinals are dense and unique"
                );
                let fields = program.variant_fields(variant);
                let tys: Vec<(MirFieldId, TypeId)> = fields
                    .iter()
                    .map(|&field| {
                        let field_descriptor = program
                            .field(field)
                            .expect("validated MIR retains every field");
                        (field, field_descriptor.ty)
                    })
                    .collect();
                let (slots, variant_size) = field_layouts(types, &tys, ENUM_TAG_OFFSET);
                total = total.max(variant_size);
                variant_slots[ordinal] = slots;
            }
            AggregateLayout {
                kind,
                total_size: total,
                struct_fields: Vec::new(),
                variant_fields: variant_slots,
            }
        }
    }
}

/// Indexes used by place resolution: nominal type id to aggregate
/// instance, and per-variant field position by field id.
#[derive(Debug, Default, Clone)]
pub struct AggregateIndex {
    pub by_type: BTreeMap<TypeId, MirAggregateId>,
}

impl AggregateIndex {
    pub fn build(program: &MirProgram) -> Self {
        let mut by_type = BTreeMap::new();
        for (id, aggregate) in program.aggregates() {
            by_type.entry(aggregate.ty).or_insert(id);
        }
        Self { by_type }
    }

    pub fn aggregate_for(&self, ty: TypeId) -> Option<MirAggregateId> {
        self.by_type.get(&ty).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lpp_mir::MirFieldId;
    use lpp_types::{PrimitiveType, TypeInterner, TypeKind};

    fn types() -> TypeInterner {
        let mut interner = TypeInterner::new();
        interner
            .intern(TypeKind::Primitive(PrimitiveType::Int))
            .unwrap();
        interner
            .intern(TypeKind::Primitive(PrimitiveType::Float))
            .unwrap();
        interner
            .intern(TypeKind::Primitive(PrimitiveType::Bool))
            .unwrap();
        interner
            .intern(TypeKind::Primitive(PrimitiveType::Char))
            .unwrap();
        interner
    }

    #[test]
    fn scalar_size_align_matches_the_machine_abi() {
        let mut t = types();
        let int = t.intern(TypeKind::Primitive(PrimitiveType::Int)).unwrap();
        let f64 = t.intern(TypeKind::Primitive(PrimitiveType::Float)).unwrap();
        let bool = t.intern(TypeKind::Primitive(PrimitiveType::Bool)).unwrap();
        let char = t.intern(TypeKind::Primitive(PrimitiveType::Char)).unwrap();
        assert_eq!(type_size_align(&t, int), (8, 8));
        assert_eq!(type_size_align(&t, f64), (8, 8));
        assert_eq!(type_size_align(&t, bool), (1, 1));
        assert_eq!(type_size_align(&t, char), (4, 4));
    }

    #[test]
    fn mixed_width_layout_respects_alignment() {
        let mut t = types();
        let bool = t.intern(TypeKind::Primitive(PrimitiveType::Bool)).unwrap();
        let int = t.intern(TypeKind::Primitive(PrimitiveType::Int)).unwrap();
        let char = t.intern(TypeKind::Primitive(PrimitiveType::Char)).unwrap();
        // [Bool, Bool, Char, Int] from 0: 0, 1, 4, 8; size 16.
        let fields = vec![
            (MirFieldId::from_raw(0), bool),
            (MirFieldId::from_raw(1), bool),
            (MirFieldId::from_raw(2), char),
            (MirFieldId::from_raw(3), int),
        ];
        let (slots, total) = field_layouts(&t, &fields, 0);
        assert_eq!(
            slots.iter().map(|s| s.offset).collect::<Vec<_>>(),
            vec![0, 1, 4, 8]
        );
        assert_eq!(total, 16);
        // The enum form starts at 8 and is 8-aligned: 8, 9, 12, 16; size 24.
        let (slots, total) = field_layouts(&t, &fields, ENUM_TAG_OFFSET);
        assert_eq!(
            slots.iter().map(|s| s.offset).collect::<Vec<_>>(),
            vec![8, 9, 12, 16]
        );
        assert_eq!(total, 24);
    }
}
