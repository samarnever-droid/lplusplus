//! `core` — the proven v1 direct-linker engine (Phase 6A port, verbatim).
//!
//! In-process direct linker for Linux ELF, Windows PE, and macOS Mach-O.
//! Production-grade drop-in for `lpp` compiler binaries. Links relocatable
//! objects and static archives without spawning an external linker.
//!
//! This module is a behavior-preserving relocation of the v1
//! `src/linker.rs` engine into the `lpp-linker` crate (Phase 6A,
//! `docs/rewrite/PHASE_6.md`). Layout math follows the System V ABI,
//! PE/COFF spec, and Mach-O loader contract: `p_vaddr % p_align ==
//! p_offset % p_align`, PE `SizeOfImage` is section-aligned, Mach-O
//! `sizeofcmds` equals the sum of load commands. The 6A.2 slice will
//! thread `LinkError` through the internal signatures.

pub const LPP_FREESTANDING: bool = true;

use object::read::archive::ArchiveFile;
use object::{
    Architecture, BinaryFormat, Object, ObjectSection, ObjectSymbol, RelocationKind,
    RelocationTarget, SymbolKind, SymbolSection,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

mod elf_image;
mod elf_relocations;
use elf_image::{ElfImageContext, finish_elf_image};
use elf_relocations::apply_elf_relocations;

// These shared implementation fragments stay in `core`'s privacy scope so
// ELF, PE, Mach-O, and the CLI can use the same internal object model without
// widening its visibility. Format-specific writers live in normal submodules.
include!("core/integer_helpers.rs");
include!("core/shared_types.rs");
include!("core/relocations.rs");
include!("core/object_images.rs");
include!("core/archives.rs");
include!("core/symbol_resolution.rs");
include!("core/aarch64_patch.rs");
include!("core/elf_support.rs");

mod cli;
mod elf_writer;
mod macho_writer;
mod pe_support;
mod pe_writer;

use pe_support::*;

pub use cli::{
    expand_response_files, expand_response_files_t, inspect_object, inspect_object_t, link_cli,
    link_cli_t, link_direct, link_with_options, link_with_options_t, sniff_format, usage,
};
pub use elf_writer::{write_elf, write_elf_with_options, write_elf_with_options_t};
pub use macho_writer::{write_macho, write_macho_with_options, write_macho_with_options_t};
pub use pe_writer::{write_pe, write_pe_with_options, write_pe_with_options_t};

#[cfg(test)]
mod tests;
