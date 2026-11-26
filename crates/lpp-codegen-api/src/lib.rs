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

mod backend;
mod builtin;
mod error;
pub mod layout;
mod machine;
pub mod ownership_lowering;
mod target;

pub use backend::{
    Backend, CodegenOptions, CompiledModule, DebugSourceResolver, NameResolver, OptLevel,
};
pub use builtin::{BuiltinLowering, runtime_symbol_lowerable};
pub use error::{CodegenError, CodegenErrorKind};
pub use machine::{MachineType, machine_type_for_abi, semantic_machine_type};
pub use ownership_lowering::{
    OwnershipLoweringPlan, build_ownership_lowering_plan, is_managed_value, is_pod_aggregate,
    is_pod_tuple,
};
pub use target::Target;

/// C, POSIX, libm, and runtime symbol names that must never be exported
/// verbatim by unannotated L++ functions or methods (which would otherwise
/// interpose ahead of libc/liblpp_runtime during dynamic linking).
const RESERVED_C_SYMBOLS: &[&str] = &[
    "_exit",
    "abort",
    "abs",
    "atof",
    "atoi",
    "atol",
    "bsearch",
    "calloc",
    "ceil",
    "clock",
    "close",
    "cos",
    "dup",
    "dup2",
    "exit",
    "exp",
    "fabs",
    "fclose",
    "fflush",
    "floor",
    "fmod",
    "fmod",
    "fopen",
    "fprintf",
    "fputc",
    "fputs",
    "fread",
    "free",
    "fstat",
    "fwrite",
    "getchar",
    "getenv",
    "isatty",
    "labs",
    "log",
    "log10",
    "lseek",
    "malloc",
    "memchr",
    "memcmp",
    "memcpy",
    "memmove",
    "memset",
    "open",
    "pow",
    "printf",
    "putchar",
    "puts",
    "qsort",
    "rand",
    "read",
    "realloc",
    "setenv",
    "sin",
    "sleep",
    "snprintf",
    "socket",
    "sprintf",
    "sqrt",
    "srand",
    "stat",
    "strcasecmp",
    "strcat",
    "strchr",
    "strcmp",
    "strcpy",
    "strcspn",
    "strdup",
    "strlen",
    "strncat",
    "strncmp",
    "strncpy",
    "strpbrk",
    "strrchr",
    "strspn",
    "strstr",
    "strtod",
    "strtol",
    "strtoul",
    "system",
    "tan",
    "time",
    "unlink",
    "usleep",
    "vprintf",
    "vsnprintf",
    "wait",
    "waitpid",
    "write",
];

/// Compute a collision-free object export name for a non-`main`, non-`@export_name`
/// L++ function or method while preserving normal L++ function names verbatim.
#[must_use]
pub fn sanitize_internal_export_name(
    source_name: &str,
    mir_id: u32,
    used: &mut std::collections::BTreeSet<String>,
) -> String {
    let needs_prefix =
        source_name.starts_with("lpp_") || RESERVED_C_SYMBOLS.binary_search(&source_name).is_ok();
    let mut export = if needs_prefix {
        format!("lpp_fn_{source_name}")
    } else {
        source_name.to_owned()
    };
    if used.contains(&export) {
        export = format!("{export}_{mir_id}");
    }
    used.insert(export.clone());
    export
}
