use lpp_hir::{ArenaId, BinaryOperator, ExprId, HirPackage, StmtId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceExpressionFact {
    TupleField { index: u32 },
    ListIndex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AugmentedAssignmentFact {
    pub operator: BinaryOperator,
    pub right: ExprId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceStatementFact {
    Assignment {
        augmented: Option<AugmentedAssignmentFact>,
    },
    TupleDestructure {
        arity: u32,
    },
    ListIteration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceFacts {
    expressions: Vec<Option<PlaceExpressionFact>>,
    statements: Vec<Option<PlaceStatementFact>>,
}

impl PlaceFacts {
    pub(crate) fn new(package: &HirPackage) -> Self {
        Self {
            expressions: vec![None; package.expressions.len()],
            statements: vec![None; package.statements.len()],
        }
    }

    #[must_use]
    pub fn expression(&self, expression: ExprId) -> Option<PlaceExpressionFact> {
        self.expressions[expression.index()]
    }

    pub(crate) fn set_expression(&mut self, expression: ExprId, fact: PlaceExpressionFact) {
        self.expressions[expression.index()] = Some(fact);
    }

    #[must_use]
    pub fn statement(&self, statement: StmtId) -> Option<PlaceStatementFact> {
        self.statements[statement.index()]
    }

    pub(crate) fn set_statement(&mut self, statement: StmtId, fact: PlaceStatementFact) {
        self.statements[statement.index()] = Some(fact);
    }

    pub fn expressions(&self) -> impl Iterator<Item = (ExprId, PlaceExpressionFact)> + '_ {
        self.expressions
            .iter()
            .enumerate()
            .filter_map(|(index, fact)| {
                fact.map(|fact| {
                    (
                        ExprId::from_index(index).expect("place expression fact index fits ExprId"),
                        fact,
                    )
                })
            })
    }

    pub fn statements(&self) -> impl Iterator<Item = (StmtId, PlaceStatementFact)> + '_ {
        self.statements
            .iter()
            .enumerate()
            .filter_map(|(index, fact)| {
                fact.map(|fact| {
                    (
                        StmtId::from_index(index).expect("place statement fact index fits StmtId"),
                        fact,
                    )
                })
            })
    }

    #[must_use]
    pub fn expression_slot_count(&self) -> usize {
        self.expressions.len()
    }

    #[must_use]
    pub fn statement_slot_count(&self) -> usize {
        self.statements.len()
    }

    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.expression_slot_count()
            .saturating_add(self.statement_slot_count())
    }
}
