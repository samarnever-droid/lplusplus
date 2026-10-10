use std::fmt;
use std::sync::Arc;

use lpp_hir::{DefId, LocalId, OriginId, Symbol, VariantId};

use crate::ids::{InferVarId, TypeId};
use crate::interner::TypeInternerExhausted;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InferenceLevel(u32);

impl InferenceLevel {
    pub const ROOT: Self = Self(0);

    #[must_use]
    pub const fn new(level: u32) -> Self {
        Self(level)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    #[must_use]
    pub const fn child(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InferenceVariable {
    pub parent: InferVarId,
    pub type_id: TypeId,
    pub level: InferenceLevel,
    pub origin: Option<OriginId>,
    pub binding: Option<TypeId>,
    pub(super) rank: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeneralizedVariable {
    pub source: InferVarId,
    pub level: InferenceLevel,
    pub origin: Option<OriginId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeScheme {
    pub body: TypeId,
    pub(super) variables: Arc<[GeneralizedVariable]>,
}

impl TypeScheme {
    #[must_use]
    pub fn monomorphic(body: TypeId) -> Self {
        Self {
            body,
            variables: Arc::from([]),
        }
    }

    #[must_use]
    pub fn variables(&self) -> &[GeneralizedVariable] {
        &self.variables
    }

    #[must_use]
    pub fn quantified_count(&self) -> usize {
        self.variables.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeWorkBudget {
    remaining: usize,
    max_depth: usize,
}

impl TypeWorkBudget {
    pub const DEFAULT_MAX_DEPTH: usize = 256;

    #[must_use]
    pub const fn new(units: usize) -> Self {
        Self::with_max_depth(units, Self::DEFAULT_MAX_DEPTH)
    }

    #[must_use]
    pub const fn with_max_depth(units: usize, max_depth: usize) -> Self {
        Self {
            remaining: units,
            max_depth,
        }
    }

    #[must_use]
    pub const fn remaining(self) -> usize {
        self.remaining
    }

    #[must_use]
    pub const fn max_depth(self) -> usize {
        self.max_depth
    }

    pub(crate) fn consume(&mut self, operation: &'static str) -> Result<(), TypeError> {
        self.remaining = self
            .remaining
            .checked_sub(1)
            .ok_or(TypeError::WorkLimitExceeded { operation })?;
        Ok(())
    }

    pub(crate) fn check_depth(
        &self,
        depth: usize,
        operation: &'static str,
    ) -> Result<(), TypeError> {
        if depth > self.max_depth {
            return Err(TypeError::DepthLimitExceeded {
                operation,
                limit: self.max_depth,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeError {
    InternerExhausted,
    InferenceVariablesExhausted,
    Mismatch {
        expected: TypeId,
        actual: TypeId,
    },
    ArityMismatch {
        expected: usize,
        actual: usize,
    },
    OccursCheck {
        variable: InferVarId,
        within: TypeId,
    },
    WorkLimitExceeded {
        operation: &'static str,
    },
    DepthLimitExceeded {
        operation: &'static str,
        limit: usize,
    },
    InvalidBoundVariable {
        index: u32,
        available: usize,
    },
    TypeAliasCycle {
        definition: DefId,
    },
    UnknownField {
        definition: DefId,
        field: Symbol,
    },
    UnknownVariant {
        definition: DefId,
        variant: Symbol,
    },
    InvalidAggregateBase {
        actual: TypeId,
    },
    InvalidAssignmentTarget,
    ImmutableAssignmentRoot,
    TupleIndexMustBeStatic,
    TupleIndexOutOfBounds {
        index: i64,
        arity: usize,
    },
    TupleElementAssignment,
    InvalidMatchSubject {
        actual: TypeId,
    },
    InvalidMatchPattern,
    MatchBindingArity {
        variant: VariantId,
        expected: usize,
        actual: usize,
    },
    InvalidTryCarrier {
        actual: TypeId,
    },
    TrySuccessPayloadArity {
        actual: usize,
    },
    InvalidTryReturn {
        carrier: TypeId,
        function_result: TypeId,
    },
    InvalidTryResidual {
        variant: VariantId,
    },
    SpawnCaptureMutation {
        local: LocalId,
    },
    UnsafeAsyncBlocking {
        builtin: &'static str,
    },
    UnsafeAsyncTaskCapture,
    BorrowedSliceEscape {
        reason: &'static str,
    },
}

impl fmt::Display for TypeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InternerExhausted => {
                formatter.write_str("type interner exhausted its compact ID space")
            }
            Self::InferenceVariablesExhausted => {
                formatter.write_str("inference arena exhausted its compact ID space")
            }
            Self::Mismatch { expected, actual } => {
                write!(
                    formatter,
                    "type mismatch: expected {expected:?}, found {actual:?}"
                )
            }
            Self::ArityMismatch { expected, actual } => {
                write!(
                    formatter,
                    "type arity mismatch: expected {expected}, found {actual}"
                )
            }
            Self::OccursCheck { variable, within } => write!(
                formatter,
                "infinite type: {variable:?} occurs within {within:?}"
            ),
            Self::WorkLimitExceeded { operation } => {
                write!(
                    formatter,
                    "bounded type operation '{operation}' exhausted its work budget"
                )
            }
            Self::DepthLimitExceeded { operation, limit } => write!(
                formatter,
                "bounded type operation '{operation}' exceeded its depth limit of {limit}"
            ),
            Self::InvalidBoundVariable { index, available } => write!(
                formatter,
                "bound type variable {index} is outside the scheme's {available} variables"
            ),
            Self::TypeAliasCycle { definition } => {
                write!(formatter, "cyclic type alias at definition {definition:?}")
            }
            Self::UnknownField { definition, field } => {
                write!(
                    formatter,
                    "unknown field {field:?} on nominal {definition:?}"
                )
            }
            Self::UnknownVariant {
                definition,
                variant,
            } => write!(
                formatter,
                "unknown variant {variant:?} on enum {definition:?}"
            ),
            Self::InvalidAggregateBase { actual } => {
                write!(formatter, "type {actual:?} is not a nominal aggregate")
            }
            Self::InvalidAssignmentTarget => {
                formatter.write_str("assignment target is not a mutable local place")
            }
            Self::ImmutableAssignmentRoot => {
                formatter.write_str("assignment target is rooted in an immutable local")
            }
            Self::TupleIndexMustBeStatic => {
                formatter.write_str("tuple index must be a static integer literal")
            }
            Self::TupleIndexOutOfBounds { index, arity } => write!(
                formatter,
                "tuple index {index} is outside a tuple with {arity} elements"
            ),
            Self::TupleElementAssignment => {
                formatter.write_str("tuple element assignment is not supported")
            }
            Self::InvalidMatchSubject { actual } => {
                write!(formatter, "match subject {actual:?} is not an enum")
            }
            Self::InvalidMatchPattern => formatter.write_str(
                "match pattern must be '_', a variant, or that enum's qualified variant",
            ),
            Self::MatchBindingArity {
                variant,
                expected,
                actual,
            } => write!(
                formatter,
                "match variant {variant:?} binds {actual} values but declares {expected} payloads"
            ),
            Self::InvalidTryCarrier { actual } => {
                write!(formatter, "try operand {actual:?} is not a supported enum")
            }
            Self::TrySuccessPayloadArity { actual } => write!(
                formatter,
                "try success variant must have exactly one payload, found {actual}"
            ),
            Self::InvalidTryReturn {
                carrier,
                function_result,
            } => write!(
                formatter,
                "try carrier {carrier:?} cannot propagate through function result {function_result:?}"
            ),
            Self::SpawnCaptureMutation { local } => {
                write!(
                    formatter,
                    "Cannot mutate captured variable {local:?} inside a spawned closure: this would cause a data race"
                )
            }
            Self::UnsafeAsyncBlocking { builtin } => write!(
                formatter,
                "blocking builtin `{builtin}` cannot be called from an async function"
            ),
            Self::UnsafeAsyncTaskCapture => formatter.write_str(
                "task await handles cannot escape into closures; await them directly in the async body",
            ),
            Self::BorrowedSliceEscape { reason } => {
                write!(formatter, "borrowed slice cannot escape: {reason}")
            }
            Self::InvalidTryResidual { variant } => write!(
                formatter,
                "try residual payloads for variant {variant:?} do not match the function result"
            ),
        }
    }
}

impl std::error::Error for TypeError {}

impl From<TypeInternerExhausted> for TypeError {
    fn from(_: TypeInternerExhausted) -> Self {
        Self::InternerExhausted
    }
}
