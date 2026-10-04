//! `MachineType` — the backend-neutral machine scalar lattice and the
//! total mappings from the ABI registry's type tables into it.
//!
//! The registry has two type views, and both map total here:
//!
//! - `generated::AbiType` (5 variants) is the *machine* ABI a builtin
//!   symbol actually takes; `machine_type_for_abi` maps it. This is the
//!   table a backend uses to build import signatures.
//! - the parsed registry `AbiType` (9 variants) is the *semantic* view
//!   (`Any`, `Str`, `StrSlice`, `VectorI64x2`, …);
//!   `semantic_machine_type` maps it. This is the table 5C's
//!   aggregate/closure lowering reasons with.
//!
//! Both mirrors the v1 cranelift backend exactly
//! (`Bool -> I8`, `Int/Str/Any -> I64`, `Float -> F64`, …). `Void` has
//! no machine value and maps to `None`.

use lpp_runtime_abi::generated::{AbiType, SemanticAbiType};

/// A machine scalar type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MachineType {
    I8,
    I32,
    I64,
    F64,
    I64X2,
}

impl MachineType {
    #[must_use]
    pub const fn all() -> &'static [MachineType] {
        &[
            MachineType::I8,
            MachineType::I32,
            MachineType::I64,
            MachineType::F64,
            MachineType::I64X2,
        ]
    }
}

/// The total mapping from the *machine* ABI type (the generated
/// table's `AbiType`, what a builtin symbol's parameters/results
/// actually are) into the machine lattice. `Void` maps to `None`.
#[must_use]
pub const fn machine_type_for_abi(abi: AbiType) -> Option<MachineType> {
    Some(match abi {
        AbiType::Bool => MachineType::I8,
        AbiType::F64 => MachineType::F64,
        AbiType::I32 => MachineType::I32,
        AbiType::I64 => MachineType::I64,
        AbiType::Void => return None,
    })
}

/// The total mapping from the *semantic* ABI type (parsed registry /
/// `SemanticAbiType`) into the machine lattice. `Void` maps to `None`;
/// `VectorI64x2` is the only variant that lowers to the vector
/// machine type.
#[must_use]
pub const fn semantic_machine_type(semantic: SemanticAbiType) -> Option<MachineType> {
    Some(match semantic {
        SemanticAbiType::Any => MachineType::I64,
        SemanticAbiType::Bool => MachineType::I8,
        SemanticAbiType::F64 => MachineType::F64,
        SemanticAbiType::I32 => MachineType::I32,
        SemanticAbiType::I64 => MachineType::I64,
        SemanticAbiType::Str => MachineType::I64,
        SemanticAbiType::StrSlice => MachineType::I64,
        SemanticAbiType::VectorI64x2 => MachineType::I64X2,
        SemanticAbiType::Void => return None,
    })
}
