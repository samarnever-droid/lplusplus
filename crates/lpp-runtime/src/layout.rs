//! Layout constants for the L++ runtime (Phase 6B).
//!
//! These mirror, byte for byte, the v1 C runtime's (`lpp_runtime.c`)
//! `LppArcHeader` and the string-pool entry layout the generated objects
//! assume. The v1 C runtime remains the behavioral reference; any change
//! here must change in lockstep (and the symbol/layout gates will fail
//! first).

/// The ARC header magic, at offset 0 of every ARC-managed object's hidden
/// header. `"ARC1"` in ASCII.
pub const ARC_MAGIC: u32 = 0x4152_4331;

/// Immortal sentinel: a refcount value meaning "static, never freed".
/// Deliberately the same constant as [`ARC_MAGIC`] — a literal prefixed with
/// the constant in BOTH of its first two words is well-formed to this
/// runtime (magic@0, refcount@4) and immortal to it, and stays correct under
/// the freestanding layout (refcount@0) as well.
pub const ARC_IMMORTAL: u32 = 0x4152_4331;

/// Hidden header size. C layout of `LppArcHeader` (verified with
/// `offsetof`): magic(4)@0 + refcount(4)@4 + generation(4)@8 +
/// 4 bytes padding@12 + destructor(8)@16 = 24 bytes. The destructor is a
/// pointer, so the C compiler aligns it to 8 — it sits at offset 16, not 12.
pub const ARC_HEADER_SIZE: usize = 24;

/// Byte offsets inside the hidden header (match C `offsetof` exactly).
pub const ARC_MAGIC_OFFSET: usize = 0;
pub const ARC_REFCOUNT_OFFSET: usize = 4;
pub const ARC_GENERATION_OFFSET: usize = 8;
pub const ARC_DESTRUCTOR_OFFSET: usize = 16;

/// The freestanding (min) runtime puts the refcount at offset 0; both of the
/// first two words of an emitted literal hold [`ARC_IMMORTAL`], so one blob
/// stays immortal no matter which runtime the object is linked against.
pub const FREESTANDING_REFCOUNT_OFFSET: usize = 0;

/// Minimum address accepted as a valid ARC payload pointer (beyond the
/// kernel guard pages; the C runtime uses 0x10000).
pub const MIN_VALID_ADDR: usize = 0x10000;

/// The shared immortal empty string: 16-aligned, 32 words of which the last
/// two hold `""` plus padding. The payload pointer is word 6 (24 bytes in).
pub const EMPTY_STR_BLOB_WORDS: usize = 8;
pub const EMPTY_STR_PAYLOAD_WORD: usize = 6;
