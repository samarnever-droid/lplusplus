//! `Target` — the targets rewrite codegen addresses in Phase 5.
//!
//! The Cranelift backend (5B) addresses `X86_64` and `Aarch64` (cranelift
//! 0.113 ships those ISAs by default). `Wasm32Wasi` (5D) is Phase 5's
//! first 32-bit target: heap pointers are `i32` linear-memory offsets and
//! the object is a self-contained `.wasm` image. Each backend declares the
//! targets it implements via `Backend::targets`; this type is the shared,
//! bounded lattice.

/// A deterministic codegen target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Target {
    X86_64,
    Aarch64,
    Wasm32Wasi,
}

impl Target {
    /// The fixed, deterministic triple the target compiles to. Objects are
    /// named for the platform the v1 freestanding runtime targets; the
    /// triple is a compile-time constant, never derived from the host, so
    /// output is reproducible.
    #[must_use]
    pub const fn triple(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64-unknown-linux-gnu",
            Self::Aarch64 => "aarch64-unknown-linux-gnu",
            Self::Wasm32Wasi => "wasm32-wasip1",
        }
    }

    /// Pointer width in bits. The native targets are 64-bit; wasm32 is
    /// 32-bit (heap pointers are linear-memory offsets).
    #[must_use]
    pub const fn pointer_bits(self) -> u32 {
        match self {
            Self::Wasm32Wasi => 32,
            Self::X86_64 | Self::Aarch64 => 64,
        }
    }

    /// The full bounded set of Phase 5 targets, in deterministic order.
    #[must_use]
    pub const fn all() -> &'static [Target] {
        &[Target::X86_64, Target::Aarch64, Target::Wasm32Wasi]
    }
}
