//! `CodegenError` — the `E5xxx` error table for Phase 5 backends.
//!
//! A backend either lowers a validated MIR program completely or
//! reports a typed error at the exact function. There is no silent
//! fallback to another backend and no reinterpretation of the program
//! (the `OPTIMIZATION_STRATEGY.md` rule: optimizer and codegen bugs
//! fail compilation).

use lpp_mir::MirFunctionId;

use crate::Target;

/// The kind of a codegen failure, mapped one-to-one onto the `E5xxx`
/// table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodegenErrorKind {
    /// A MIR shape the slice does not lower (aggregates, stores,
    /// closures, async, …). `construct` names the rejected shape.
    UnsupportedConstruct { construct: &'static str },
    /// The requested target is not in the backend's `targets()`.
    UnsupportedTarget(Target),
    /// A builtin the slice does not lower, or a builtin that requires
    /// a single runtime import the backend does not provide.
    UnrepresentableBuiltin {
        builtin: lpp_types::BuiltinId,
        reason: &'static str,
    },
    /// The backend's own IR rejected the lowered function.
    IrVerificationFailed(String),
    /// The object emitter failed.
    ObjectEmissionFailed(String),
    /// A registry descriptor contradicts the backend's expectation
    /// (defence in depth at the boundary).
    AbiMismatch {
        symbol: String,
        expected_arity: u32,
        actual_arity: u32,
    },
}

/// One codegen failure, anchored to a function when the failure is
/// function-local.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodegenError {
    pub function: Option<MirFunctionId>,
    pub kind: CodegenErrorKind,
}

impl CodegenError {
    #[must_use]
    pub const fn new(function: Option<MirFunctionId>, kind: CodegenErrorKind) -> Self {
        Self { function, kind }
    }

    /// The stable `E5xxx` diagnostic code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match &self.kind {
            CodegenErrorKind::UnsupportedConstruct { .. } => "E5001",
            CodegenErrorKind::UnsupportedTarget(_) => "E5002",
            CodegenErrorKind::UnrepresentableBuiltin { .. } => "E5003",
            CodegenErrorKind::IrVerificationFailed(_) => "E5004",
            CodegenErrorKind::ObjectEmissionFailed(_) => "E5005",
            CodegenErrorKind::AbiMismatch { .. } => "E5006",
        }
    }
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {:?}", self.code(), self.kind)
    }
}

impl std::error::Error for CodegenError {}
