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
    I64,
    F64,
    Pointer,
    VectorI64x2,
}

/// Existing list accessor policy. `Bool` is retained here for behavior parity
/// with the current runtime call selection; changing it is a separate semantic
/// bug-fix, not part of this refactor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListAccessorClass {
    Scalar,
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
            TypeRef::Bool => AbiClass::I8,
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

    /// Frontend list-literal element boundary, centralized without changing
    /// the currently accepted language subset.
    pub fn is_frontend_list_element(&self) -> bool {
        matches!(
            self,
            TypeRef::Int | TypeRef::Float | TypeRef::Bool | TypeRef::Char
        ) || self.is_managed()
    }

    /// Existing AOT allocation boundary for list element types.
    pub fn is_aot_list_element(&self) -> bool {
        matches!(
            self,
            TypeRef::Int
                | TypeRef::Float
                | TypeRef::Custom(_)
                | TypeRef::Str
                | TypeRef::Bool
                | TypeRef::Tuple(_)
                | TypeRef::Task(_)
        )
    }

    /// Runtime list getter/pusher symbol family used by the current lowering.
    pub fn list_accessor_class(&self) -> ListAccessorClass {
        match self {
            TypeRef::Float => ListAccessorClass::Float,
            TypeRef::Custom(_) | TypeRef::Str | TypeRef::Bool => ListAccessorClass::Arc,
            _ => ListAccessorClass::Scalar,
        }
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
