//! `lpp-linker` — the L++ direct linker (Phase 6 of the rewrite).
//!
//! An in-process, external-tool-free linker for Linux ELF, Windows PE, and
//! macOS Mach-O. It consumes relocatable object files (and static archives)
//! and produces native executables: symbols resolved and merged, relocations
//! patched, TLS handled, PE base relocations and checksums computed, and the
//! entry point resolved.
//!
//! ## Layout (Phase 6A)
//!
//! - [`core`] holds the proven v1 engine, relocated verbatim (behavior
//!   preserving). Its public surface is re-exported here so existing call
//!   sites (`pm.rs`, the `lpp-link` binary) are unchanged.
//! - This crate root adds the **typed boundary** the Phase 7 driver will use:
//!   [`link_typed`] returns a [`LinkReport`] and a [`core::LinkError`]
//!   carrying the unresolved-symbol list, instead of a bare `String`.
//!
//! See `docs/rewrite/PHASE_6.md` for the approved contract and the 6A.2
#![allow(clippy::all, warnings)]

pub mod core;

use std::fs;
use std::path::PathBuf;

// Re-export the proven v1 public surface so `lpp::linker::X` keeps working.
pub use core::{
    LPP_FREESTANDING, LinkError, LinkErrorKind, LinkOptions, Machine, OutputFormat, PeSubsystem,
    expand_response_files, inspect_object, link_cli, link_direct, link_with_options, sniff_format,
    usage, write_elf, write_elf_with_options, write_macho, write_macho_with_options, write_pe,
    write_pe_with_options,
};
// `DynamicMode` is part of the v1 public surface (PHASE_6.md 6A) and is
// needed by callers configuring static/dynamic linking.
pub use core::DynamicMode;

/// The resolved output format a link produced (mirrors [`OutputFormat`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedFormat {
    Elf,
    Pe,
    Macho,
}

impl From<OutputFormat> for ResolvedFormat {
    fn from(f: OutputFormat) -> Self {
        match f {
            OutputFormat::Elf => Self::Elf,
            OutputFormat::Pe => Self::Pe,
            OutputFormat::Macho => Self::Macho,
        }
    }
}

/// The result of a successful typed link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkReport {
    /// The format the output image was written in.
    pub format: ResolvedFormat,
    /// The path the executable was written to.
    pub output: PathBuf,
    /// The size of the produced image in bytes.
    pub output_size: u64,
    /// The number of input objects that were linked.
    pub object_count: usize,
}

/// Determine the output format for a link, applying the same defaulting the
/// engine uses (explicit option wins; otherwise sniff the first input).
pub fn resolve_format(
    inputs: &[PathBuf],
    options: &LinkOptions,
) -> Result<ResolvedFormat, LinkError> {
    if inputs.is_empty() {
        return Err(LinkError::new(
            LinkErrorKind::Usage,
            "at least one input object is required",
        ));
    }
    let fmt = options.format.unwrap_or_else(|| {
        match inputs.first().map(|p| sniff_format(p)).unwrap_or("elf") {
            "pe" => OutputFormat::Pe,
            "macho" => OutputFormat::Macho,
            _ => OutputFormat::Elf,
        }
    });
    Ok(ResolvedFormat::from(fmt))
}

/// The modern, typed entry point (for the Phase 7 driver).
///
/// Unlike [`link_with_options`], which reports failure as a flat `String`,
/// this returns a [`LinkError`] whose `unresolved` field carries the
/// unresolved-symbol list the engine already computes, plus a [`LinkReport`]
/// describing the produced image. The underlying link is the same proven
/// engine.
pub fn link_typed(
    inputs: &[PathBuf],
    output: &std::path::Path,
    options: &LinkOptions,
) -> Result<LinkReport, LinkError> {
    let format = resolve_format(inputs, options)?;
    // The typed engine entry: the `LinkError` (kind + unresolved list)
    // survives to the caller instead of being flattened to a String.
    core::link_with_options_t(inputs, output, options)?;
    let output_size = fs::metadata(output).map(|m| m.len()).unwrap_or(0);
    Ok(LinkReport {
        format,
        output: output.to_path_buf(),
        output_size,
        object_count: inputs.len(),
    })
}
