//! Runtime ABI schema and deterministic output generation.

mod generate;
mod model;

pub use generate::{GeneratedAbi, generate};
pub use model::{
    AbiRegistry, AbiType, Builtin, BuiltinNameOverride, LoweringSignature, Ownership, SchemaError,
    SymbolOverride, TargetAvailability,
};

/// Compile-checked descriptors generated from the v1 ABI schema.
pub mod generated {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../abi/generated/builtins.rs"
    ));
}

/// The workspace's checked-in v1 compatibility schema.
pub const BUILTINS_SCHEMA: &str = include_str!("../../../abi/builtins.toml");

pub fn v1_registry() -> Result<AbiRegistry, SchemaError> {
    AbiRegistry::parse(BUILTINS_SCHEMA)
}
