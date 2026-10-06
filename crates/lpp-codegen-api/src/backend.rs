//! The `Backend` trait, `CodegenOptions`, `NameResolver`, and
//! `CompiledModule`.
//!
//! A backend consumes **optimized and revalidated MIR** (the Phase 4
//! exit artifact) and produces a verified, deterministic object file.
//! It never mutates its input, and it never invents names or layouts.

use std::collections::BTreeSet;

use lpp_mir::MirProgram;
use lpp_types::TypeInterner;

use crate::Target;

/// Resolves a MIR symbol raw index (the `Symbol` identity space
/// embedded in `MirFunction::name`) to its source-level name.
///
/// Codegen crates do not import HIR; the symbol table stays with the
/// caller (the HIR name index), and backends ask for names through
/// this boundary. Resolution order is irrelevant; the mapping is a
/// pure function of the raw index.
pub trait NameResolver: Send + Sync {
    fn resolve(&self, symbol_raw: u32) -> Option<&str>;
}

/// The optimization level requested of a backend.
///
/// [`OptLevel::None`] is the raw, deterministic output (the dev default). For
/// the `wasm32` target a non-`None` level runs the object through `wasm-opt`
/// as a one-time production pass (a system tool, like `clang` for the native
/// backends — no new Rust dependency). Native backends currently ignore it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OptLevel {
    /// Raw output; no post-optimization.
    #[default]
    None,
    O1,
    O2,
    O3,
    O4,
}

impl OptLevel {
    /// The `wasm-opt` flag for this level, or `None` for the raw level.
    pub const fn wasm_flag(self) -> Option<&'static str> {
        match self {
            OptLevel::None => None,
            OptLevel::O1 => Some("-O1"),
            OptLevel::O2 => Some("-O2"),
            OptLevel::O3 => Some("-O3"),
            OptLevel::O4 => Some("-O4"),
        }
    }

    pub const fn is_none(self) -> bool {
        matches!(self, OptLevel::None)
    }
}

/// The options for one module compilation.
pub struct CodegenOptions<'a> {
    pub target: Target,
    pub names: &'a dyn NameResolver,
    pub opt_level: OptLevel,
}

impl std::fmt::Debug for CodegenOptions<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CodegenOptions")
            .field("target", &self.target)
            .field("names", &"<NameResolver>")
            .field("opt_level", &self.opt_level)
            .finish()
    }
}

impl<'a> CodegenOptions<'a> {
    #[must_use]
    pub fn new(target: Target, names: &'a dyn NameResolver) -> Self {
        Self {
            target,
            names,
            opt_level: OptLevel::None,
        }
    }

    /// Request a post-optimization level. The wasm backend runs the object
    /// through `wasm-opt`; native backends ignore it.
    #[must_use]
    pub fn with_opt_level(mut self, level: OptLevel) -> Self {
        self.opt_level = level;
        self
    }
}

/// The result of compiling a MIR program to one object file.
///
/// `object` is the full host-format object image (ELF, Mach-O, or COFF for
/// native backends). The symbol census (`exported_symbols`,
/// `imported_symbols`, `entry`) is derived from the module's own
/// declaration record — ordered, deterministic collections — so tests
/// can assert ABI shape without parsing the bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledModule {
    pub target: Target,
    pub object: Vec<u8>,
    pub exported_symbols: BTreeSet<String>,
    pub imported_symbols: BTreeSet<String>,
    /// The C-ABI process entry symbol (`main`) when the program
    /// defines a source-level `main`; `None` for library modules.
    pub entry: Option<String>,
}

impl CompiledModule {
    /// Whether two modules are byte-identical for the same target.
    #[must_use]
    pub fn same_object(&self, other: &Self) -> bool {
        self.target == other.target && self.object == other.object
    }
}

/// A Phase 5 backend: validated MIR in, verified deterministic object
/// out.
pub trait Backend {
    /// The backend's stable name (e.g. `"cranelift"`).
    fn name(&self) -> &'static str;

    /// The targets this backend implements, in deterministic order.
    fn targets(&self) -> &'static [Target];

    /// Compile every function of the program into one object file.
    ///
    /// Functions are lowered in `MirFunctionId` order (the program's
    /// deterministic iteration order), data symbols in first-use
    /// order, and the result must be byte-identical across repeated
    /// compiles of the same input. Anything the backend's slice does
    /// not lower is a typed `CodegenError`, never a partial object.
    fn compile_module(
        &self,
        program: &MirProgram,
        types: &TypeInterner,
        options: &CodegenOptions<'_>,
    ) -> Result<CompiledModule, crate::CodegenError>;
}
