//! `lpp-codegen-api` — the Phase 5A backend-neutral contract.
//!
//! Every Phase 5 backend lowers *optimized and revalidated MIR* into a
//! verified, deterministic object file. This crate holds the shared
//! vocabulary those backends implement against: targets, machine
//! scalar types, builtin lowering derived from the checked-in ABI
//! registry, the `Backend` trait, the `CompiledModule` census, and the
//! `E5xxx` error table.
//!
//! Boundaries: this crate never imports a backend, and it never
//! resolves builtin behavior by ad-hoc string dispatch — builtin facts
//! come from `lpp-runtime-abi`'s generated table via `BuiltinId`.
#![allow(clippy::all, warnings)]

mod backend;
mod builtin;
mod error;
pub mod layout;
mod machine;
mod target;

pub use backend::{Backend, CodegenOptions, CompiledModule, NameResolver, OptLevel};
pub use builtin::{BuiltinLowering, runtime_symbol_lowerable};
pub use error::{CodegenError, CodegenErrorKind};
pub use machine::{MachineType, machine_type_for_abi, semantic_machine_type};
pub use target::Target;
