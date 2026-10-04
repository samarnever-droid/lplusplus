//! `lpp-runtime` — the L++ runtime library as first-class Rust code
//! (Phase 6B, `docs/rewrite/PHASE_6.md`).
//!
//! The v1 C runtime (`lpp_runtime.c`) remains the **behavioral reference**:
//! every symbol here re-implements a C primitive with the identical ABI
//! (same header/struct layout, same sentinel, same memory ordering, same
//! allocator), and the gate suite runs differential sequences against both
//! implementations plus a symbol census against the C object.
//!
//! Slices:
//! - **6B.1:** layout constants + the ARC core (alloc, retain/release atomic
//!   + local fast path, weak generations, immortal empty string, closure
//!   destroy).
//! - **6B.2:** the panic path, `List[T]` (value + ARC-owning,
//!   push/get/set for int/float/bool/arc, len/pop/free/reserve/capacity/
//!   clear), and numeric `Slice[T]` (init/len/get/get_float/get_bool).
//! - **6B.3a:** the pure-numeric builtins (abs/min/max, float
//!   sqrt/floor/ceil/pow, unsigned shift/div/rem/compare, rotates, bit
//!   counts, byte swaps, truncations, checked + wrapping arithmetic).
//! - **6B.3b/c (this release):** the string builtins (concat/find/replace/
//!   trim/contains/starts_with/ends_with/upper/lower/eq/len, int/float/
//!   bool/u64 to-str, str-to-int/u64, u64-to-hex, string slices) and the IO
//!   builtins (print_int/float/bool/str, write_str, eprint_str).
//! - **6B.3d (this release):** tasks (new/poll/await/destroy), the
//!   structural tuple, and `vec_i64_checksum` — completing the 95-symbol
//!   `c_shim.c` link surface.
//! - **6B.3e (next):** arena regions (internal; not part of the c_shim
//!   surface) and the freestanding `lpp_runtime_min.c` variants.
//!
//! The crate builds as `rlib` (in-process tests), `staticlib` and
//! `cdylib` (linking for generated objects and the census gate).

pub mod layout;

pub mod arc;
pub mod clock_rng;
pub mod concur;
pub mod io;
pub mod list;
pub mod map;
pub mod net;
pub mod numeric;
pub mod panic;
pub mod slice;
pub mod string;
pub mod task;
pub mod tuple;

pub use layout::{
    ARC_DESTRUCTOR_OFFSET, ARC_GENERATION_OFFSET, ARC_HEADER_SIZE, ARC_IMMORTAL, ARC_MAGIC,
    ARC_MAGIC_OFFSET, ARC_REFCOUNT_OFFSET, FREESTANDING_REFCOUNT_OFFSET, MIN_VALID_ADDR,
};
