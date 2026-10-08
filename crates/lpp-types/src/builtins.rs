//! Builtin resolution against the generated ABI registry.
//!
//! The rewrite never resolves builtin names by ad-hoc string dispatch: the
//! checked-in generated table from `lpp-runtime-abi` is the single source of
//! truth, and every resolved call records a compact fact that MIR consumes.

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use lpp_hir::{ArenaId, ExprId};

use crate::{PrimitiveType, TypeId, TypeInterner, TypeKind};

/// Compact identity into the checked-in generated builtin table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BuiltinId(NonZeroU32);

impl BuiltinId {
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        assert!(raw < u32::MAX, "builtin ID exceeds its range");
        Self(NonZeroU32::new(raw + 1).expect("index + 1 is nonzero"))
    }

    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0.get() - 1
    }

    #[must_use]
    pub const fn descriptor(self) -> &'static lpp_runtime_abi::generated::BuiltinAbi {
        &lpp_runtime_abi::generated::BUILTINS[self.raw() as usize]
    }
}

/// Index of builtin source spellings to compact builtin identities.
#[derive(Debug, Default)]
pub struct BuiltinIndex {
    by_name: BTreeMap<&'static str, BuiltinId>,
}

impl BuiltinIndex {
    /// Build the index from the checked-in generated table. Duplicate
    /// spellings (the audited v1 `file_size` pair) keep the first entry, in
    /// deterministic table order.
    #[must_use]
    pub fn from_generated() -> Self {
        let mut by_name = BTreeMap::new();
        for (offset, builtin) in lpp_runtime_abi::generated::BUILTINS.iter().enumerate() {
            by_name.entry(builtin.name).or_insert(Self::id_at(offset));
        }
        Self { by_name }
    }

    fn id_at(offset: usize) -> BuiltinId {
        BuiltinId::from_raw(u32::try_from(offset).expect("the generated table fits the ID space"))
    }

    #[must_use]
    pub fn lookup(&self, name: &str) -> Option<BuiltinId> {
        self.by_name.get(name).copied()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}

/// One resolved builtin call. The full semantic signature is derived from the
/// generated descriptor, so the fact stays a four-byte identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinFact {
    pub builtin: BuiltinId,
}

/// Call expressions that resolved to registry builtins, indexed by the call's
/// expression ID in deterministic HIR order.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BuiltinFacts {
    expressions: Vec<Option<BuiltinFact>>,
}

impl BuiltinFacts {
    pub fn new(expression_count: usize) -> Self {
        Self {
            expressions: vec![None; expression_count],
        }
    }

    pub fn record(&mut self, expression: ExprId, builtin: BuiltinId) {
        self.expressions[expression.index()] = Some(BuiltinFact { builtin });
    }

    #[must_use]
    pub fn expression(&self, expression: ExprId) -> Option<BuiltinFact> {
        self.expressions[expression.index()]
    }

    pub fn expressions(&self) -> impl Iterator<Item = (ExprId, BuiltinFact)> {
        self.expressions
            .iter()
            .enumerate()
            .filter_map(move |(index, fact)| {
                fact.and_then(|fact| ExprId::from_index(index).map(|id| (id, fact)))
            })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.expressions
            .iter()
            .filter(|fact| fact.is_some())
            .count()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.expressions.iter().all(|fact| fact.is_none())
    }
}

/// Check whether a concrete type matches one semantic ABI kind.
#[must_use]
pub fn type_matches_semantic(
    ty: TypeId,
    kind: lpp_runtime_abi::generated::SemanticAbiType,
    types: &TypeInterner,
) -> bool {
    use lpp_runtime_abi::generated::SemanticAbiType;
    matches!(
        (types.kind(ty), kind),
        (_, SemanticAbiType::Any)
            | (
                TypeKind::Primitive(PrimitiveType::Bool),
                SemanticAbiType::Bool
            )
            | (
                TypeKind::Primitive(PrimitiveType::Float),
                SemanticAbiType::F64
            )
            | (
                TypeKind::Primitive(PrimitiveType::Int),
                SemanticAbiType::I32 | SemanticAbiType::I64
            )
            | (
                TypeKind::Primitive(PrimitiveType::String),
                SemanticAbiType::Str
            )
            | (
                TypeKind::Primitive(PrimitiveType::StrSlice),
                SemanticAbiType::StrSlice
            )
            | (
                TypeKind::Primitive(PrimitiveType::VectorI64x2),
                SemanticAbiType::VectorI64x2
            )
            | (
                TypeKind::Primitive(PrimitiveType::Void),
                SemanticAbiType::Void
            )
    )
}

/// The concrete L++ type a semantic ABI kind denotes, or `None` for kinds
/// the shadow stage leaves unconstrained: `Any`, and the raw integer slots
/// (`I32`/`I64`), which may carry a heap handle, an element, or a count and
/// therefore resolve to whatever type unification pins them to.
#[must_use]
pub fn semantic_result_type(
    kind: lpp_runtime_abi::generated::SemanticAbiType,
    types: &TypeInterner,
) -> Option<TypeId> {
    use lpp_runtime_abi::generated::SemanticAbiType;
    Some(match kind {
        SemanticAbiType::Any | SemanticAbiType::I32 | SemanticAbiType::I64 => return None,
        SemanticAbiType::Bool => types.primitive(PrimitiveType::Bool),
        SemanticAbiType::F64 => types.primitive(PrimitiveType::Float),
        SemanticAbiType::Str => types.primitive(PrimitiveType::String),
        SemanticAbiType::StrSlice => types.primitive(PrimitiveType::StrSlice),
        SemanticAbiType::VectorI64x2 => types.primitive(PrimitiveType::VectorI64x2),
        SemanticAbiType::Void => types.primitive(PrimitiveType::Void),
    })
}

/// Stable spelling of a semantic ABI kind for diagnostics.
#[must_use]
pub fn semantic_spelling(kind: lpp_runtime_abi::generated::SemanticAbiType) -> &'static str {
    use lpp_runtime_abi::generated::SemanticAbiType;
    match kind {
        SemanticAbiType::Any => "Any",
        SemanticAbiType::Bool => "Bool",
        SemanticAbiType::F64 => "F64",
        SemanticAbiType::I32 => "I32",
        SemanticAbiType::I64 => "I64",
        SemanticAbiType::Str => "Str",
        SemanticAbiType::StrSlice => "StrSlice",
        SemanticAbiType::VectorI64x2 => "VectorI64x2",
        SemanticAbiType::Void => "Void",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_every_generated_builtin_spelling() {
        let index = BuiltinIndex::from_generated();
        assert!(!index.is_empty());
        let unique = lpp_runtime_abi::generated::BUILTINS
            .iter()
            .map(|builtin| builtin.name)
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        assert_eq!(index.len(), unique);
        let print_str = index
            .lookup("print_str")
            .expect("print_str is a registry builtin");
        assert_eq!(print_str.descriptor().symbol, "lpp_print_str");
        assert_eq!(
            print_str.descriptor().semantic_parameters,
            &[lpp_runtime_abi::generated::SemanticAbiType::Str]
        );
        assert!(index.lookup("definitely_not_a_builtin").is_none());
    }

    #[test]
    fn facts_record_and_retrieve_one_fact_per_call_expression() {
        let mut facts = BuiltinFacts::new(4);
        assert!(facts.is_empty());
        let first = BuiltinId::from_raw(0);
        facts.record(ExprId::from_raw(2), first);
        assert_eq!(facts.len(), 1);
        assert_eq!(
            facts.expression(ExprId::from_raw(2)).unwrap().builtin,
            first
        );
        assert!(facts.expression(ExprId::from_raw(3)).is_none());
    }
}
