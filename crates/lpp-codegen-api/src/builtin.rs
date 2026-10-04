//! `BuiltinLowering` — derive a builtin's runtime symbol and machine
//! signature from the checked-in ABI registry, never by string
//! dispatch.
//!
//! `BuiltinId::descriptor` is the single source of truth (the
//! compile-checked generated table from `lpp-runtime-abi`). A builtin
//! whose registry `symbol` is empty — e.g. v1 `print`, which the v1
//! backend lowers as a *sequence* of runtime calls rather than one
//! import — has no single runtime symbol and is reported not
//! runtime-lowerable; a backend either handles such a builtin by name
//! within its own documented subset or rejects it with `E5003`.

use lpp_runtime_abi::generated::AbiType as GeneratedAbiType;
use lpp_types::BuiltinId;

use crate::machine::{MachineType, machine_type_for_abi};

/// One builtin's lowering facts, derived from the registry descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltinLowering {
    pub builtin: BuiltinId,
    pub name: &'static str,
    pub symbol: &'static str,
    pub parameters: Vec<MachineType>,
    pub result: Option<MachineType>,
}

impl BuiltinLowering {
    /// Derive the lowering for a runtime-lowerable builtin. Returns
    /// `None` when the registry symbol is empty (no single runtime
    /// import exists) — the caller decides whether the backend has a
    /// by-name lowering or reports `E5003`.
    ///
    /// Fails the process (panic) if a non-`Void` parameter has no
    /// machine type: that would mean the `AbiType -> MachineType`
    /// mapping is no longer total, an invariant the 5A gate asserts.
    #[must_use]
    pub fn from_builtin(builtin: BuiltinId) -> Option<Self> {
        let descriptor = builtin.descriptor();
        (!descriptor.symbol.is_empty()).then(|| {
            let parameters = descriptor
                .parameters
                .iter()
                .copied()
                .map(|abi| {
                    machine_type_for_abi(abi).expect(
                        "AbiType -> MachineType mapping must stay total for parameter types",
                    )
                })
                .collect();
            let result = descriptor.result;
            let result = if result == GeneratedAbiType::Void {
                None
            } else {
                Some(
                    machine_type_for_abi(result)
                        .expect("AbiType -> MachineType mapping must stay total for result types"),
                )
            };
            Self {
                builtin,
                name: descriptor.name,
                symbol: descriptor.symbol,
                parameters,
                result,
            }
        })
    }
}

/// Whether a builtin has a single runtime symbol in the registry (and
/// so is lowerable as one import). `false` for empty-symbol builtins.
#[must_use]
pub fn runtime_symbol_lowerable(builtin: BuiltinId) -> bool {
    !builtin.descriptor().symbol.is_empty()
}
