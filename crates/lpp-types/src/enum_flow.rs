use lpp_hir::{
    ArenaId, ExprId, FieldId, HirItemId, HirPackage, IdRange, LocalId, MatchArmId, StmtId,
    VariantId,
};

use crate::TypeId;

/// Resolved enum semantics for one statement-form `match`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnumMatchFact {
    pub item: HirItemId,
    pub subject_type: TypeId,
    pub arms: IdRange<MatchArmId>,
    pub covered_variants: u32,
    pub total_variants: u32,
    pub wildcard: Option<MatchArmId>,
}

impl EnumMatchFact {
    #[must_use]
    pub const fn exhaustive(self) -> bool {
        self.wildcard.is_some() || self.covered_variants == self.total_variants
    }
}

/// Resolved pattern and payload mapping for one source arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnumMatchArmFact {
    Variant {
        item: HirItemId,
        variant: VariantId,
        fields: IdRange<FieldId>,
        bindings: IdRange<LocalId>,
        reachable: bool,
    },
    Wildcard {
        reachable: bool,
    },
}

impl EnumMatchArmFact {
    #[must_use]
    pub const fn reachable(self) -> bool {
        match self {
            Self::Variant { reachable, .. } | Self::Wildcard { reachable } => reachable,
        }
    }
}

/// Resolved success extraction and residual mapping for one postfix `?`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnumTryFact {
    pub item: HirItemId,
    pub carrier_type: TypeId,
    pub success: VariantId,
    pub success_field: FieldId,
    pub success_type: TypeId,
    pub variants: IdRange<VariantId>,
    pub return_item: HirItemId,
    pub return_type: TypeId,
    pub direct_residual: bool,
}

/// Fixed-size optional fact tables indexed directly by canonical HIR identities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumFlowFacts {
    matches: Vec<Option<EnumMatchFact>>,
    arms: Vec<Option<EnumMatchArmFact>>,
    tries: Vec<Option<EnumTryFact>>,
}

impl EnumFlowFacts {
    pub(crate) fn new(package: &HirPackage) -> Self {
        Self {
            matches: vec![None; package.statements.len()],
            arms: vec![None; package.match_arms.len()],
            tries: vec![None; package.expressions.len()],
        }
    }

    #[must_use]
    pub fn match_statement(&self, statement: StmtId) -> Option<EnumMatchFact> {
        self.matches[statement.index()]
    }

    pub(crate) fn set_match(&mut self, statement: StmtId, fact: EnumMatchFact) {
        self.matches[statement.index()] = Some(fact);
    }

    #[must_use]
    pub fn match_arm(&self, arm: MatchArmId) -> Option<EnumMatchArmFact> {
        self.arms[arm.index()]
    }

    pub(crate) fn set_arm(&mut self, arm: MatchArmId, fact: EnumMatchArmFact) {
        self.arms[arm.index()] = Some(fact);
    }

    #[must_use]
    pub fn try_expression(&self, expression: ExprId) -> Option<EnumTryFact> {
        self.tries[expression.index()]
    }

    pub(crate) fn set_try(&mut self, expression: ExprId, fact: EnumTryFact) {
        self.tries[expression.index()] = Some(fact);
    }

    pub fn matches(&self) -> impl Iterator<Item = (StmtId, EnumMatchFact)> + '_ {
        self.matches.iter().enumerate().filter_map(|(index, fact)| {
            fact.map(|fact| {
                (
                    StmtId::from_index(index).expect("enum match fact index fits StmtId"),
                    fact,
                )
            })
        })
    }

    pub fn arms(&self) -> impl Iterator<Item = (MatchArmId, EnumMatchArmFact)> + '_ {
        self.arms.iter().enumerate().filter_map(|(index, fact)| {
            fact.map(|fact| {
                (
                    MatchArmId::from_index(index).expect("enum arm fact index fits MatchArmId"),
                    fact,
                )
            })
        })
    }

    pub fn tries(&self) -> impl Iterator<Item = (ExprId, EnumTryFact)> + '_ {
        self.tries.iter().enumerate().filter_map(|(index, fact)| {
            fact.map(|fact| {
                (
                    ExprId::from_index(index).expect("enum try fact index fits ExprId"),
                    fact,
                )
            })
        })
    }

    pub(crate) fn matches_mut(&mut self) -> impl Iterator<Item = &mut EnumMatchFact> {
        self.matches.iter_mut().filter_map(Option::as_mut)
    }

    pub(crate) fn tries_mut(&mut self) -> impl Iterator<Item = &mut EnumTryFact> {
        self.tries.iter_mut().filter_map(Option::as_mut)
    }

    #[must_use]
    pub fn match_slot_count(&self) -> usize {
        self.matches.len()
    }

    #[must_use]
    pub fn arm_slot_count(&self) -> usize {
        self.arms.len()
    }

    #[must_use]
    pub fn try_slot_count(&self) -> usize {
        self.tries.len()
    }

    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.match_slot_count()
            .saturating_add(self.arm_slot_count())
            .saturating_add(self.try_slot_count())
    }

    #[must_use]
    pub fn fact_count(&self) -> usize {
        self.matches()
            .count()
            .saturating_add(self.arms().count())
            .saturating_add(self.tries().count())
    }
}
