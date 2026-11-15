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
    X86_64None,
    Aarch64None,
    Wasm32Wasi,
}

impl Target {
    /// Deterministic architecture triple for the build host's object format.
    /// Native architecture selection is shared across platforms, while the OS
    /// component follows the compiler host so emitted objects can be linked by
    /// that host's toolchain. Bare-metal kernel targets (`*-unknown-none`)
    /// always emit ELF objects regardless of host OS.
    #[must_use]
    pub const fn triple(self) -> &'static str {
        match self {
            Self::X86_64 if cfg!(target_os = "windows") => "x86_64-pc-windows-msvc",
            Self::X86_64 if cfg!(target_os = "macos") => "x86_64-apple-darwin",
            Self::X86_64 => "x86_64-unknown-linux-gnu",
            Self::Aarch64 if cfg!(target_os = "windows") => "aarch64-pc-windows-msvc",
            Self::Aarch64 if cfg!(target_os = "macos") => "aarch64-apple-darwin",
            Self::Aarch64 => "aarch64-unknown-linux-gnu",
            Self::X86_64None => "x86_64-unknown-none",
            Self::Aarch64None => "aarch64-unknown-none",
            Self::Wasm32Wasi => "wasm32-wasip1",
        }
    }

    /// True when the target produces freestanding bare-metal kernel ELF images
    /// with no host OS ABI, C runtime, or dynamic linker.
    #[must_use]
    pub const fn is_freestanding(self) -> bool {
        matches!(self, Self::X86_64None | Self::Aarch64None)
    }

    /// Pointer width in bits. The native/bare-metal targets are 64-bit; wasm32 is
    /// 32-bit (heap pointers are linear-memory offsets).
    #[must_use]
    pub const fn pointer_bits(self) -> u32 {
        match self {
            Self::Wasm32Wasi => 32,
            Self::X86_64 | Self::Aarch64 | Self::X86_64None | Self::Aarch64None => 64,
        }
    }

    /// The default target matching the current build host.
    #[must_use]
    pub const fn host() -> Self {
        if cfg!(target_arch = "aarch64") {
            Self::Aarch64
        } else {
            Self::X86_64
        }
    }

    /// The full bounded set of Phase 5 targets, in deterministic order.
    #[must_use]
    pub const fn all() -> &'static [Target] {
        &[
            Target::X86_64,
            Target::Aarch64,
            Target::X86_64None,
            Target::Aarch64None,
            Target::Wasm32Wasi,
        ]
    }
}
