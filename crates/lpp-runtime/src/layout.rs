//! Native ARC layout constants shared by the Rust runtime and generated code.
//!
//! Every native ARC payload has a 16-byte common header immediately before
//! it: magic@0, refcount@4, generation@8, flags@12. Objects with a destructor
//! carry an additional 8-byte destructor pointer immediately before that
//! common header, so their allocation prefix remains 24 bytes. Objects without
//! a destructor—including ordinary strings—use only the 16-byte header.
//!
//! The freestanding C runtimes encode allocator and arena metadata in the
//! flags word; the hosted runtime uses it for cycle-collector flags. The
//! compiler does not read these fields: payload values remain normal pointers.

/// ARC magic, the first word of every native ARC header (`"ARC1"` in ASCII).
pub const ARC_MAGIC: u32 = 0x4152_4331;

/// Immortal sentinel: a refcount value meaning "static, never freed".
/// The emitted literal stores this in both the magic and refcount words so
/// the read-only blob is recognized consistently by every native runtime.
pub const ARC_IMMORTAL: u32 = 0x4152_4331;

/// Size of the common native ARC header immediately before the payload.
pub const ARC_HEADER_SIZE: usize = 16;

/// Destructor-bearing allocations place the function pointer immediately
/// before the common header. This prefix is absent when there is no destructor.
pub const ARC_DESTRUCTOR_PREFIX_SIZE: usize = 8;

/// Byte offsets inside the common header (match the C layouts exactly).
pub const ARC_MAGIC_OFFSET: usize = 0;
pub const ARC_REFCOUNT_OFFSET: usize = 4;
pub const ARC_GENERATION_OFFSET: usize = 8;
pub const ARC_CYCLE_FLAGS_OFFSET: usize = 12;

/// The object is registered with the cycle collector (its type is a
/// `Shared` cycle member).
pub const ARC_FLAG_COLLECTIBLE: u32 = 1;
/// The object is currently in the possible-cycle-root buffer.
pub const ARC_FLAG_BUFFERED: u32 = 2;
/// An 8-byte destructor pointer is stored immediately before the common
/// header. Kept in the flags word so freeing can recover the true allocation
/// base after cycle-collector invalidation.
pub const ARC_FLAG_HAS_DTOR: u32 = 4;

/// The host, Linux freestanding, and Windows freestanding runtimes now share
/// the same native refcount offset.
pub const FREESTANDING_REFCOUNT_OFFSET: usize = ARC_REFCOUNT_OFFSET;

/// Minimum address accepted as a valid ARC payload pointer (beyond the guard
/// pages; the C runtime uses 0x10000).
pub const MIN_VALID_ADDR: usize = 0x10000;

/// Shared immortal empty-string blob. Keep the allocation larger than needed
/// for alignment; the payload pointer is word 4 (16 bytes in).
pub const EMPTY_STR_BLOB_WORDS: usize = 8;
pub const EMPTY_STR_PAYLOAD_WORD: usize = ARC_HEADER_SIZE / 4;
