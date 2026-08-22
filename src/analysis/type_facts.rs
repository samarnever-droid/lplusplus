//! Canonical semantic facts for resolved L++ types.
//!
//! Ownership, ABI shape, recursive containment and container policies used to
//! be re-derived with ad-hoc `matches!(TypeRef::...)` expressions throughout
//! MIR and both backends. Keeping those decisions here makes adding a new
//! managed type a compiler-enforced, reviewable change instead of a search for
//! every partial type list.

use crate::types::TypeRef;

/// Runtime lifetime category of a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifetimeClass {
    /// Plain value with no destructor.
    Copy,
    /// One ARC/region-managed reference.
    Managed,
    /// Non-owning view whose source lifetime is checked separately.
    BorrowedView,
}

/// Backend-neutral machine ABI category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbiClass {
    Void,
    I8,
    I16,
    I32,
    I64,
    F64,
    Pointer,
    VectorI64x2,
}

/// Runtime representation and accessor family for one list element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListElementClass {
    Unsupported,
    Scalar,
    Bool,
    Float,
    Arc,
}

impl TypeRef {
    /// The single ownership classification consumed by MIR and backends.
    pub fn lifetime_class(&self) -> LifetimeClass {
        match self {
            TypeRef::Str
            | TypeRef::Custom(_)
            | TypeRef::Generic(_, _)
            | TypeRef::Function
            | TypeRef::Tuple(_)
            | TypeRef::Task(_) => LifetimeClass::Managed,
            TypeRef::StrSlice | TypeRef::Slice(_) => LifetimeClass::BorrowedView,
            TypeRef::Int
            | TypeRef::Float
            | TypeRef::Char
            | TypeRef::Void
            | TypeRef::Bool
            | TypeRef::U8
            | TypeRef::U16
            | TypeRef::U32
            | TypeRef::I8
            | TypeRef::I16
            | TypeRef::I32
            | TypeRef::Unresolved(_)
            | TypeRef::TypeParam(_)
            | TypeRef::VectorI64x2 => LifetimeClass::Copy,
        }
    }

    pub fn is_managed(&self) -> bool {
        self.lifetime_class() == LifetimeClass::Managed
    }

    pub fn is_borrowed_view(&self) -> bool {
        self.lifetime_class() == LifetimeClass::BorrowedView
    }

    pub fn abi_class(&self) -> AbiClass {
        match self {
            TypeRef::Void => AbiClass::Void,
            TypeRef::Bool | TypeRef::U8 | TypeRef::I8 => AbiClass::I8,
            TypeRef::U16 | TypeRef::I16 => AbiClass::I16,
            TypeRef::U32 | TypeRef::I32 => AbiClass::I32,
            TypeRef::Float => AbiClass::F64,
            TypeRef::Int | TypeRef::Char => AbiClass::I64,
            TypeRef::VectorI64x2 => AbiClass::VectorI64x2,
            TypeRef::Str
            | TypeRef::Custom(_)
            | TypeRef::Generic(_, _)
            | TypeRef::Unresolved(_)
            | TypeRef::Function
            | TypeRef::TypeParam(_)
            | TypeRef::Tuple(_)
            | TypeRef::StrSlice
            | TypeRef::Slice(_)
            | TypeRef::Task(_) => AbiClass::Pointer,
        }
    }

    /// Size/alignment of one field in the current 64-bit native ABI.
    pub fn native_size_align(&self) -> (usize, usize) {
        match self.abi_class() {
            AbiClass::I8 => (1, 1),
            AbiClass::I16 => (2, 2),
            AbiClass::I32 => (4, 4),
            AbiClass::VectorI64x2 => (16, 16),
            AbiClass::Void | AbiClass::I64 | AbiClass::F64 | AbiClass::Pointer => (8, 8),
        }
    }

    /// Whether a type recursively contains a single-executor Task value.
    pub fn contains_task(&self) -> bool {
        match self {
            TypeRef::Task(_) => true,
            TypeRef::Tuple(elements) | TypeRef::Generic(_, elements) => {
                elements.iter().any(TypeRef::contains_task)
            }
            TypeRef::Slice(element) => element.contains_task(),
            _ => false,
        }
    }

    /// One canonical list policy shared by type checking, MIR, and backends.
    /// Lists store one 64-bit slot per element; borrowed views, unresolved
    /// types, SIMD vectors and Void cannot safely fit that ownership model.
    pub fn list_element_class(&self) -> ListElementClass {
        match self {
            TypeRef::Bool | TypeRef::U8 | TypeRef::I8 => ListElementClass::Bool,
            TypeRef::Float => ListElementClass::Float,
            TypeRef::Int | TypeRef::Char | TypeRef::U16 | TypeRef::I16 | TypeRef::U32 | TypeRef::I32 => ListElementClass::Scalar,
            TypeRef::Str
            | TypeRef::Custom(_)
            | TypeRef::Generic(_, _)
            | TypeRef::Function
            | TypeRef::Tuple(_)
            | TypeRef::Task(_) => ListElementClass::Arc,
            TypeRef::Void
            | TypeRef::Unresolved(_)
            | TypeRef::TypeParam(_)
            | TypeRef::VectorI64x2
            | TypeRef::StrSlice
            | TypeRef::Slice(_) => ListElementClass::Unsupported,
        }
    }

    pub fn is_list_element_supported(&self) -> bool {
        self.list_element_class() != ListElementClass::Unsupported
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::StructTypeId;

    #[test]
    fn ownership_and_abi_facts_are_centralized() {
        assert_eq!(TypeRef::Int.lifetime_class(), LifetimeClass::Copy);
        assert_eq!(TypeRef::Str.lifetime_class(), LifetimeClass::Managed);
        assert_eq!(
            TypeRef::StrSlice.lifetime_class(),
            LifetimeClass::BorrowedView
        );
        assert_eq!(TypeRef::Bool.abi_class(), AbiClass::I8);
        assert_eq!(
            TypeRef::Tuple(vec![TypeRef::Int, TypeRef::Str]).abi_class(),
            AbiClass::Pointer
        );
        assert!(TypeRef::Custom(StructTypeId(0)).is_managed());
    }

    #[test]
    fn list_policy_is_consistent_for_frontend_and_backends() {
        assert_eq!(TypeRef::Bool.list_element_class(), ListElementClass::Bool);
        assert_eq!(TypeRef::Char.list_element_class(), ListElementClass::Scalar);
        assert_eq!(TypeRef::Float.list_element_class(), ListElementClass::Float);
        assert_eq!(TypeRef::Str.list_element_class(), ListElementClass::Arc);
        assert_eq!(
            TypeRef::Generic("List".to_string(), vec![TypeRef::Int]).list_element_class(),
            ListElementClass::Arc
        );
        assert!(!TypeRef::StrSlice.is_list_element_supported());
        assert!(!TypeRef::VectorI64x2.is_list_element_supported());
    }

    #[test]
    fn recursive_task_fact_sees_nested_containers() {
        let nested = TypeRef::Tuple(vec![
            TypeRef::Int,
            TypeRef::Generic(
                "List".to_string(),
                vec![TypeRef::Task(Box::new(TypeRef::Str))],
            ),
        ]);
        assert!(nested.contains_task());
        assert!(!TypeRef::Slice(Box::new(TypeRef::Int)).contains_task());
    }
}
