use std::collections::BTreeMap;

use lpp_hir::{
    ArenaId, DefId, ExprId, FieldId, HirItemId, HirItemKind, HirPackage, Symbol, VariantId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregateConstructor {
    Struct { item: HirItemId },
    EnumVariant { item: HirItemId, variant: VariantId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregateExpressionFact {
    Constructor(AggregateConstructor),
    VariantConstructor {
        item: HirItemId,
        variant: VariantId,
    },
    UnitVariant {
        item: HirItemId,
        variant: VariantId,
    },
    FieldProjection {
        item: HirItemId,
        field: FieldId,
    },
    /// A UFCS method call `receiver.method(..)`: `method` is the resolved
    /// function item (a free function or an `impl` method) that receives the
    /// receiver as its first argument.
    MethodCall {
        method: HirItemId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregateFacts {
    definition_items: Vec<Option<HirItemId>>,
    fields: BTreeMap<(HirItemId, Symbol), FieldId>,
    variants: BTreeMap<(HirItemId, Symbol), VariantId>,
    expressions: Vec<Option<AggregateExpressionFact>>,
}

impl AggregateFacts {
    pub(crate) fn new(package: &HirPackage) -> Self {
        let mut facts = Self {
            definition_items: vec![None; package.names.definitions.len()],
            fields: BTreeMap::new(),
            variants: BTreeMap::new(),
            expressions: vec![None; package.expressions.len()],
        };
        for (item_id, item) in package.items.enumerate() {
            if let Some(definition) = item.definition {
                facts.definition_items[definition.index()] = Some(item_id);
            }
            match item.kind {
                HirItemKind::Struct(structure) => {
                    for field in package.fields(structure.fields) {
                        facts
                            .fields
                            .insert((item_id, package.fields[*field].name), *field);
                    }
                }
                HirItemKind::Enum(enumeration) => {
                    for variant in package.variants(enumeration.variants) {
                        facts
                            .variants
                            .insert((item_id, package.variants[*variant].name), *variant);
                    }
                }
                HirItemKind::Function(_)
                | HirItemKind::Trait(_)
                | HirItemKind::Impl(_)
                | HirItemKind::Extern(_)
                | HirItemKind::Const { .. }
                | HirItemKind::TypeAlias { .. } => {}
            }
        }
        facts
    }

    #[must_use]
    pub fn item(&self, definition: DefId) -> Option<HirItemId> {
        self.definition_items
            .get(definition.index())
            .and_then(|item| *item)
    }

    #[must_use]
    pub fn field(&self, item: HirItemId, name: Symbol) -> Option<FieldId> {
        self.fields.get(&(item, name)).copied()
    }

    #[must_use]
    pub fn variant(&self, item: HirItemId, name: Symbol) -> Option<VariantId> {
        self.variants.get(&(item, name)).copied()
    }

    #[must_use]
    pub fn expression(&self, expression: ExprId) -> Option<AggregateExpressionFact> {
        self.expressions[expression.index()]
    }

    pub(crate) fn set_expression(&mut self, expression: ExprId, fact: AggregateExpressionFact) {
        self.expressions[expression.index()] = Some(fact);
    }

    pub fn expressions(&self) -> impl Iterator<Item = (ExprId, AggregateExpressionFact)> + '_ {
        self.expressions
            .iter()
            .enumerate()
            .filter_map(|(index, fact)| {
                fact.map(|fact| {
                    (
                        ExprId::from_index(index).expect("aggregate fact index fits ExprId"),
                        fact,
                    )
                })
            })
    }

    #[must_use]
    pub fn expression_slot_count(&self) -> usize {
        self.expressions.len()
    }
}
