//! `lpp-link` — direct linker for Linux ELF, Windows PE, and macOS Mach-O.
//!
//! Phase 2+: ELF with GOT/rodata merge, PE with full multi-section
//! (.text/.rdata/.data/.bss/.idata), base relocations, and broad AMD64
//! relocation coverage.  Mach-O direct emitter for the verified subset.
//!
//! The linker deliberately grows in small verified slices.  Each format gets
//! exactly the section and relocation support it needs for the verified
//! workload set — nothing more, nothing less.

use object::{
    Architecture, BinaryFormat, Object, ObjectSection, ObjectSymbol, RelocationKind,
    RelocationTarget, SymbolSection,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

// ── Little-endian helpers ──────────────────────────────────────────────────

fn put_u16(buf: &mut [u8], offset: usize, value: u16) {
    buf[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
fn put_u32(buf: &mut [u8], offset: usize, value: u32) {
    buf[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn put_u64(buf: &mut [u8], offset: usize, value: u64) {
    buf[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}
fn align_up(value: usize, alignment: usize) -> usize {
    (value + alignment - 1) & !(alignment - 1)
}

// ═══════════════════════════════════════════════════════════════════════════
// Shared types
// ═══════════════════════════════════════════════════════════════════════════

/// What kind of section a relocation lives in — used so we can resolve
/// self-references even when Cranelift emits anonymous section symbols.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SectionClass {
    Text,
    Rodata,
    Data,
}

struct Relocation {
    offset: usize,
    target: String,
    addend: i64,
    size: u8,
    kind: RelocationKind,
}

/// Merged input data for one object file, split by section class so the
/// linker can lay out .text, .rdata, .data separately.
struct CoffSections {
    path: PathBuf,
    /// Merged code bytes (all Text-kind sections from this input).
    text: Vec<u8>,
    /// Merged read-only data bytes.
    rdata: Vec<u8>,
    /// Merged writable data bytes.
    data: Vec<u8>,
    /// Each element: (section_index, class, base_offset_within_class_buffer).
    #[allow(dead_code)]
    section_map: Vec<(object::SectionIndex, SectionClass, usize)>,
    /// Global symbols, keyed by name → offset *within its section class buffer*.
    symbols: Vec<(String, SectionClass, u64)>,
    /// All relocations from every section.
    relocations: Vec<Relocation>,
}

/// ELF-only aggregated input (kept mostly for the existing ELF path).
struct ElfInput {
    path: PathBuf,
    text: Vec<u8>,
    rodata: Vec<u8>,
    text_symbols: Vec<(String, u64)>,
    rodata_symbols: Vec<(String, u64)>,
    relocations: Vec<Relocation>,
}

/// Mach-O aggregated input.
struct MachoInput {
    path: PathBuf,
    text: Vec<u8>,
    text_symbols: Vec<(String, u64)>,
    relocations: Vec<Relocation>,
}

// ═══════════════════════════════════════════════════════════════════════════
//  1.  ELF path  (kept stable, minor cleanups)
// ═══════════════════════════════════════════════════════════════════════════

const ELF_BASE: u64 = 0x400000;
const CODE_OFFSET: usize = 0x1000;
const EM_X86_64: u16 = 62;
const PT_LOAD: u32 = 1;
const PF_R_X: u32 = 5;

fn read_elf_input(path: &Path) -> Result<ElfInput, String> {
    let bytes = fs::read(path).map_err(|e| format!("read '{}': {e}", path.display()))?;
    let file =
        object::File::parse(&*bytes).map_err(|e| format!("parse '{}': {e}", path.display()))?;
    if file.format() != BinaryFormat::Elf || file.architecture() != Architecture::X86_64 {
        return Err(format!(
            "'{}' is not an x86-64 ELF relocatable object",
            path.display()
        ));
    }
    let text_sec = file
        .section_by_name(".text")
        .ok_or_else(|| format!("'{}' has no .text section", path.display()))?;
    let text_idx = text_sec.index();
    let text = text_sec
        .uncompressed_data()
        .map_err(|e| format!("read .text from '{}': {e}", path.display()))?
        .into_owned();

    let mut rodata_idxs = HashSet::new();
    let mut rodata = Vec::new();
    for sec in file.sections() {
        if let Ok(name) = sec.name() {
            if name == ".rodata" || name.starts_with(".rodata.") {
                rodata_idxs.insert(sec.index());
                if let Ok(d) = sec.uncompressed_data() {
                    rodata.extend_from_slice(&d);
                }
            }
        }
    }
    let is_rodata = |s: SymbolSection| match s {
        SymbolSection::Section(i) => rodata_idxs.contains(&i),
        _ => false,
    };

    let mut text_syms = Vec::new();
    let mut rodata_syms = Vec::new();
    for sym in file.symbols() {
        let dst = if sym.section() == SymbolSection::Section(text_idx) {
            Some(&mut text_syms)
        } else if is_rodata(sym.section()) {
            Some(&mut rodata_syms)
        } else {
            None
        };
        if let Some(dst) = dst {
            if let Ok(n) = sym.name() {
                if !n.is_empty() {
                    dst.push((n.to_string(), sym.address()));
                }
            }
        }
    }

    let mut relocs = Vec::new();
    for (off, rel) in text_sec.relocations() {
        let RelocationTarget::Symbol(si) = rel.target() else {
            return Err(format!(
                "'{}' has unsupported non-symbol relocation",
                path.display()
            ));
        };
        let sym = file
            .symbol_by_index(si)
            .map_err(|e| format!("read relocation symbol: {e}"))?;
        let raw = sym
            .name()
            .map_err(|e| format!("read relocation symbol name: {e}"))?;
        let is_section = raw.is_empty()
            || sym.kind() == object::SymbolKind::Section
            || raw.starts_with(".rodata")
            || raw.starts_with(".text");
        let target = if is_section && sym.section() == SymbolSection::Section(text_idx) {
            "__self_text__".to_string()
        } else if is_section && is_rodata(sym.section()) {
            "__self_rodata__".to_string()
        } else {
            raw.to_string()
        };
        relocs.push(Relocation {
            offset: usize::try_from(off).map_err(|_| "relocation offset overflow")?,
            target,
            addend: rel.addend(),
            size: rel.size(),
            kind: rel.kind(),
        });
    }
    Ok(ElfInput {
        path: path.to_path_buf(),
        text,
        rodata,
        text_symbols: text_syms,
        rodata_symbols: rodata_syms,
        relocations: relocs,
    })
}

fn write_elf(inputs: &[PathBuf], output: &Path) -> Result<(), String> {
    if inputs.is_empty() {
        return Err("at least one input object is required".to_string());
    }
    let objs: Vec<ElfInput> = inputs
        .iter()
        .map(|p| read_elf_input(p))
        .collect::<Result<_, _>>()?;

    let mut text = Vec::new();
    let mut bases = Vec::new();
    let mut syms: HashMap<String, u64> = HashMap::new();
    for inp in &objs {
        let base = align_up(text.len(), 16);
        text.resize(base, 0x90);
        bases.push(base);
        for (n, o) in &inp.text_symbols {
            let abs = u64::try_from(base).map_err(|_| "text offset overflow")? + o;
            if syms.insert(n.clone(), abs).is_some() {
                return Err(format!("duplicate definition of symbol '{n}'"));
            }
        }
        text.extend_from_slice(&inp.text);
    }
    // A runtime-free ELF may have `main` but no `lpp_main`.  Use `main` as
    // the entry if present and fall back to `lpp_main` otherwise.
    let has_main = syms.contains_key("main");
    let has_lpp = syms.contains_key("lpp_main");
    let entry = if has_main {
        syms.get("main")
    } else if has_lpp {
        syms.get("lpp_main")
    } else {
        None
    };
    let entry =
        entry.ok_or_else(|| "required symbol 'main' (or 'lpp_main') not found".to_string())?;
    let entry = *entry; // deref: &u64 → u64

    let start_off = text.len();
    let entry_addr = ELF_BASE + CODE_OFFSET as u64 + entry;
    let call_next = ELF_BASE + CODE_OFFSET as u64 + start_off as u64 + 11;
    let disp = entry_addr as i64 - call_next as i64;
    if disp < i32::MIN as i64 || disp > i32::MAX as i64 {
        return Err("entry point out of range for startup call".to_string());
    }
    let mut start = vec![
        0x31, 0xed, 0x48, 0x83, 0xe4, 0xf0, // xor ebp; and rsp,-16
        0xe8, 0, 0, 0, 0, // call main
        0x89, 0xc7, 0xb8, 60, 0, 0, 0, 0x0f, 0x05, // exit
    ];
    start[7..11].copy_from_slice(&(disp as i32).to_le_bytes());
    text.extend_from_slice(&start);

    let mut got: HashMap<String, usize> = HashMap::new();
    for inp in &objs {
        for rel in &inp.relocations {
            if rel.kind == RelocationKind::GotRelative {
                let n = got.len();
                got.entry(rel.target.clone()).or_insert(n);
            }
        }
    }
    let got_off = align_up(text.len(), 8);
    text.resize(got_off + got.len() * 8, 0);

    let mut rodata_bases = Vec::new();
    let mut rodata_off = align_up(text.len(), 16);
    text.resize(rodata_off, 0);
    for inp in &objs {
        let base = rodata_off;
        rodata_bases.push(base);
        for (n, o) in &inp.rodata_symbols {
            let abs = u64::try_from(base).map_err(|_| "rodata offset overflow")? + o;
            if syms.insert(n.clone(), abs).is_some() {
                return Err(format!("duplicate definition of symbol '{n}'"));
            }
        }
        text.extend_from_slice(&inp.rodata);
        rodata_off = align_up(text.len(), 16);
        text.resize(rodata_off, 0);
    }
    for (name, slot) in &got {
        let tgt = *syms
            .get(name)
            .ok_or_else(|| format!("unresolved GOT symbol '{name}'"))?;
        let loc = got_off + slot * 8;
        let addr = ELF_BASE + CODE_OFFSET as u64 + tgt;
        text[loc..loc + 8].copy_from_slice(&addr.to_le_bytes());
    }

    for (idx, inp) in objs.iter().enumerate() {
        let base = bases[idx];
        for rel in &inp.relocations {
            if rel.size != 32 {
                return Err(format!(
                    "'{}': unsupported relocation width {}",
                    inp.path.display(),
                    rel.size
                ));
            }
            let tgt = match rel.kind {
                RelocationKind::GotRelative => {
                    let slot = *got
                        .get(&rel.target)
                        .ok_or_else(|| "missing GOT slot".to_string())?;
                    u64::try_from(got_off + slot * 8).map_err(|_| "GOT overflow")?
                }
                _ if rel.target == "__self_text__" => {
                    u64::try_from(base).map_err(|_| "text overflow")?
                }
                _ if rel.target == "__self_rodata__" => {
                    u64::try_from(rodata_bases[idx]).map_err(|_| "rodata overflow")?
                }
                _ => *syms.get(&rel.target).ok_or_else(|| {
                    format!(
                        "'{}': unresolved external relocation to '{}'",
                        inp.path.display(),
                        rel.target
                    )
                })?,
            };
            let patch = base + rel.offset;
            if patch + 4 > text.len() {
                return Err(format!("'{}': patch out of range", inp.path.display()));
            }
            if rel.kind == RelocationKind::Absolute {
                let v = ELF_BASE as i64 + CODE_OFFSET as i64 + tgt as i64 + rel.addend;
                text[patch..patch + 4].copy_from_slice(&(v as i32).to_le_bytes());
            } else {
                let d = tgt as i64 + rel.addend - patch as i64;
                text[patch..patch + 4].copy_from_slice(&(d as i32).to_le_bytes());
            }
        }
    }

    let fsize = CODE_OFFSET + text.len();
    let mut elf = vec![0u8; fsize];
    elf[0..4].copy_from_slice(b"\x7fELF");
    elf[4] = 2;
    elf[5] = 1;
    elf[6] = 1;
    put_u16(&mut elf, 16, 2);
    put_u16(&mut elf, 18, EM_X86_64);
    put_u32(&mut elf, 20, 1);
    put_u64(
        &mut elf,
        24,
        ELF_BASE + CODE_OFFSET as u64 + start_off as u64,
    );
    put_u64(&mut elf, 32, 64);
    put_u16(&mut elf, 52, 64);
    put_u16(&mut elf, 54, 56);
    put_u16(&mut elf, 56, 1);
    let ph = 64;
    put_u32(&mut elf, ph, PT_LOAD);
    put_u32(&mut elf, ph + 4, PF_R_X);
    put_u64(&mut elf, ph + 8, 0);
    put_u64(&mut elf, ph + 16, ELF_BASE);
    put_u64(&mut elf, ph + 24, ELF_BASE);
    put_u64(&mut elf, ph + 32, fsize as u64);
    put_u64(&mut elf, ph + 40, fsize as u64);
    put_u64(&mut elf, ph + 48, 0x1000);
    elf[CODE_OFFSET..CODE_OFFSET + text.len()].copy_from_slice(&text);
    fs::write(output, elf).map_err(|e| format!("write '{}': {e}", output.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = fs::metadata(output)
            .map_err(|e| format!("stat '{}': {e}", output.display()))?
            .permissions();
        perm.set_mode(0o755);
        fs::set_permissions(output, perm)
            .map_err(|e| format!("chmod '{}': {e}", output.display()))?;
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
//  2.  Windows PE path  —  full multi-section linker
// ═══════════════════════════════════════════════════════════════════════════

const PE_IMAGE_BASE: u64 = 0x140000000;
const PE_SECTION_RVA: u32 = 0x1000;
const PE_FILE_ALIGN: usize = 0x200;
const PE_SECT_ALIGN: usize = 0x1000;

/// IMAGE_REL_AMD64_* constants we handle.
const AMD64_ADDR64: u8 = 1;
const AMD64_ADDR32: u8 = 2;
const AMD64_ADDR32NB: u8 = 3;
const AMD64_REL32: u8 = 4;
const AMD64_REL32_1: u8 = 5;
const AMD64_REL32_2: u8 = 6;
const AMD64_REL32_3: u8 = 7;
const AMD64_REL32_4: u8 = 8;
const AMD64_REL32_5: u8 = 9;
const AMD64_SECTION: u8 = 10;
const AMD64_SECREL: u8 = 11;

/// Map the `object` crate's generic `RelocationKind` back to the concrete
/// AMD64 relocation number when possible.  Falls back to treating `Absolute`
/// as ADDR32 and `Relative` as REL32.
fn coff_reloc_number(rel: &Relocation) -> u8 {
    // The object crate exposes the raw COFF relocation type through its
    // `RelocationKind` discriminant.  We can't directly access it, but
    // Cranelift only emits a few kinds, so we classify heuristically.
    match rel.kind {
        RelocationKind::Absolute if rel.size == 64 => AMD64_ADDR64,
        RelocationKind::Absolute => AMD64_ADDR32,
        RelocationKind::Relative => AMD64_REL32,
        RelocationKind::SectionIndex => AMD64_SECTION,
        RelocationKind::SectionOffset => AMD64_SECREL,
        _ => {
            // Unknown — treat 64-bit as ADDR64, 32-bit as REL32 (safe default)
            if rel.size == 64 {
                AMD64_ADDR64
            } else {
                AMD64_REL32
            }
        }
    }
}

/// Read one COFF object, splitting its sections into text / rdata / data
/// classes so the linker can lay them out independently.
fn read_coff_full(path: &Path) -> Result<CoffSections, String> {
    let bytes = fs::read(path).map_err(|e| format!("read '{}': {e}", path.display()))?;
    let file =
        object::File::parse(&*bytes).map_err(|e| format!("parse '{}': {e}", path.display()))?;
    if file.format() != BinaryFormat::Coff || file.architecture() != Architecture::X86_64 {
        return Err(format!("'{}' is not an x86-64 COFF object", path.display()));
    }

    let mut text_buf = Vec::new();
    let mut rdata_buf = Vec::new();
    let mut data_buf = Vec::new();
    let mut map: Vec<(object::SectionIndex, SectionClass, usize)> = Vec::new();
    let mut relocs = Vec::new();

    for sec in file.sections() {
        let idx = sec.index();
        let kind = sec.kind();

        // Skip non-loadable sections entirely — their symbols are local anchors,
        // not global definitions, and their data should not appear in the image.
        if kind == object::SectionKind::Debug
            || kind == object::SectionKind::Linker
            || kind == object::SectionKind::Metadata
            || kind == object::SectionKind::Other
        {
            continue;
        }

        let class = match kind {
            object::SectionKind::Text => SectionClass::Text,
            object::SectionKind::ReadOnlyData | object::SectionKind::ReadOnlyString => {
                SectionClass::Rodata
            }
            object::SectionKind::UninitializedData
            | object::SectionKind::UninitializedTls
            | object::SectionKind::Data => SectionClass::Data,
            _ => SectionClass::Data,
        };
        let buf: &mut Vec<u8> = match class {
            SectionClass::Text => &mut text_buf,
            SectionClass::Rodata => &mut rdata_buf,
            SectionClass::Data => &mut data_buf,
        };
        let base = align_up(buf.len(), 16);
        buf.resize(base, 0x00);

        // BSS / uninitialized sections have zero on-disk size; just reserve
        // virtual space.  `uncompressed_data()` panics for these in the object crate.
        let is_zero_fill = matches!(
            kind,
            object::SectionKind::UninitializedData | object::SectionKind::UninitializedTls
        );
        if is_zero_fill {
            let sz = sec.size() as usize;
            buf.resize(buf.len() + sz, 0x00);
            map.push((idx, class, base));
            let padded = align_up(buf.len(), 16);
            buf.resize(padded, 0x00);
            continue;
        }

        let data = sec
            .uncompressed_data()
            .map_err(|e| format!("read section from '{}': {e}", path.display()))?
            .into_owned();
        buf.extend_from_slice(&data);
        map.push((idx, class, base));

        for (off, rel) in sec.relocations() {
            let raw_off = usize::try_from(off).map_err(|_| "reloc offset overflow")?;
            let RelocationTarget::Symbol(si) = rel.target() else {
                return Err(format!(
                    "'{}' has unsupported non-symbol relocation",
                    path.display()
                ));
            };
            let sym = file
                .symbol_by_index(si)
                .map_err(|e| format!("read relocation symbol: {e}"))?;
            let raw_name = sym
                .name()
                .map_err(|e| format!("read relocation symbol name: {e}"))?;
            let target = resolve_coff_target(&raw_name, &sym, &map, class);
            relocs.push(Relocation {
                offset: base + raw_off,
                target,
                addend: rel.addend(),
                size: rel.size(),
                kind: rel.kind(),
            });
        }

        // Pad to alignment for next section of same class
        let padded = align_up(buf.len(), 16);
        buf.resize(padded, 0x00);
    }

    let mut syms = Vec::new();
    for sym in file.symbols() {
        if let SymbolSection::Section(idx) = sym.section() {
            if let Some((_, class, base)) = map.iter().find(|(i, _, _)| *i == idx) {
                if let Ok(name) = sym.name() {
                    if !name.is_empty()
                        && !name.starts_with(".text")
                        && !name.starts_with(".rdata")
                        && !name.starts_with(".data")
                        && !name.starts_with(".bss")
                        && !name.starts_with(".xdata")
                        && !name.starts_with(".pdata")
                        && !name.starts_with(".debug")
                        && !name.starts_with(".drectve")
                        && !name.starts_with('$')
                    {
                        syms.push((name.to_string(), *class, *base as u64 + sym.address()));
                    }
                }
            }
        }
    }

    Ok(CoffSections {
        path: path.to_path_buf(),
        text: text_buf,
        rdata: rdata_buf,
        data: data_buf,
        section_map: map,
        symbols: syms,
        relocations: relocs,
    })
}

fn resolve_coff_target(
    raw_name: &str,
    sym: &object::Symbol<'_, '_>,
    map: &[(object::SectionIndex, SectionClass, usize)],
    self_class: SectionClass,
) -> String {
    let is_anonymous = raw_name.is_empty()
        || sym.kind() == object::SymbolKind::Section
        || raw_name.starts_with(".text")
        || raw_name.starts_with(".rdata")
        || raw_name.starts_with(".data")
        || raw_name.starts_with(".bss")
        || raw_name.starts_with(".xdata")
        || raw_name.starts_with(".pdata")
        || raw_name.starts_with(".debug")
        || raw_name.starts_with(".drectve")
        || raw_name.starts_with('$');

    if is_anonymous {
        if let SymbolSection::Section(idx) = sym.section() {
            if let Some((_, sclass, base)) = map.iter().find(|(i, _, _)| *i == idx) {
                if *sclass == self_class {
                    return format!("__self_{}__", section_class_tag(self_class));
                }
                return format!("__ext_{}__{}", section_class_tag(*sclass), base);
            }
        }
        // Section symbol pointing to an undefined external
        return "__self_text__".to_string();
    }
    raw_name.to_string()
}

fn section_class_tag(c: SectionClass) -> &'static str {
    match c {
        SectionClass::Text => "text",
        SectionClass::Rodata => "rdata",
        SectionClass::Data => "data",
    }
}

fn pe_align(v: usize, a: usize) -> usize {
    (v + a - 1) & !(a - 1)
}

/// Base offsets for one input's section contributions in the merged buffers.
struct SectionBase {
    text_base: usize,
    rdata_base: usize,
    data_base: usize,
}

/// Build the combined import descriptor + ILT + IAT + hint/name table for
/// KERNEL32.dll.  Also reserves space for `.refptr.` internal symbols.
struct ImportData {
    data: Vec<u8>,
    iat_rvas: HashMap<String, u32>,
    refptr_offsets: HashMap<String, usize>,
    #[allow(dead_code)]
    ilt_rva: u32,
    iat_rva: u32,
    #[allow(dead_code)]
    dll_count: usize,
}

fn build_imports(
    kernel_imports: &[String],
    refptrs: &[String],
    section_rva: u32,
) -> Result<ImportData, String> {
    let count = kernel_imports.len();
    // IMAGE_IMPORT_DESCRIPTOR is 20 bytes; we need count+1 (terminator).
    let desc_count = if count == 0 { 0 } else { count + 1 };
    let desc_size = desc_count * 20;
    // ILT (Import Lookup Table) and IAT each have (count+1) × 8 bytes.
    let ilt_count = if count == 0 { 0 } else { count + 1 };
    let ilt_size = ilt_count * 8;
    let iat_size = ilt_count * 8;

    let ilt_off = align_up(desc_size, 8);
    let iat_off = ilt_off + ilt_size;
    let refptr_off = align_up(iat_off + iat_size, 8);

    let mut data = vec![0u8; refptr_off + refptrs.len() * 8];
    let mut iat_rvas = HashMap::new();
    let mut refptr_offsets = HashMap::new();

    // DLL name and hint/name entries come after the tables.
    if count > 0 {
        let dll_off = data.len();
        data.extend_from_slice(b"KERNEL32.dll\0");
        while data.len() % 2 != 0 {
            data.push(0);
        }
        let mut hint_names: HashMap<String, usize> = HashMap::new();
        for imp in kernel_imports {
            let off = data.len();
            data.extend_from_slice(&[0u8, 0u8]); // Hint
            data.extend_from_slice(imp.as_bytes());
            data.push(0);
            while data.len() % 2 != 0 {
                data.push(0);
            }
            hint_names.insert(imp.clone(), off);
        }

        for (i, imp) in kernel_imports.iter().enumerate() {
            let name_rva = section_rva + hint_names[imp] as u32;
            let thunk = name_rva as u64;
            let ilt_pos = ilt_off + i * 8;
            let iat_pos = iat_off + i * 8;
            data[ilt_pos..ilt_pos + 8].copy_from_slice(&thunk.to_le_bytes());
            data[iat_pos..iat_pos + 8].copy_from_slice(&thunk.to_le_bytes());
            iat_rvas.insert(format!("__imp_{imp}"), section_rva + iat_pos as u32);
        }
        // IMAGE_IMPORT_DESCRIPTOR
        put_u32(&mut data, 0, section_rva + ilt_off as u32); // OriginalFirstThunk
        put_u32(&mut data, 12, section_rva + dll_off as u32); // Name
        put_u32(&mut data, 16, section_rva + iat_off as u32); // FirstThunk
    }

    for (i, name) in refptrs.iter().enumerate() {
        refptr_offsets.insert(format!(".refptr.{name}"), refptr_off + i * 8);
    }

    Ok(ImportData {
        data,
        iat_rvas,
        refptr_offsets,
        ilt_rva: section_rva + ilt_off as u32,
        iat_rva: section_rva + iat_off as u32,
        dll_count: if count > 0 { 1 } else { 0 },
    })
}

/// Generate base relocations (`.reloc` section) for a writable block.
fn generate_base_relocs(data: &[u8], section_rva: u32) -> Vec<u8> {
    let page_size = 0x1000usize;
    let mut reloc = Vec::new();

    let mut page = 0usize;
    let mut entries_for_page: Vec<u16> = Vec::new();

    let flush_page = |page: usize, entries: &mut Vec<u16>, out: &mut Vec<u8>| {
        if entries.is_empty() {
            return;
        }
        let block_size = 8 + entries.len() * 2;
        // Align block to 4 bytes
        let padded = align_up(block_size, 4);
        let start = out.len();
        out.resize(start + padded, 0);
        put_u32(out, start, (section_rva as usize + page) as u32);
        put_u32(out, start + 4, padded as u32);
        for (i, e) in entries.iter().enumerate() {
            put_u16(out, start + 8 + i * 2, *e);
        }
        entries.clear();
    };

    // For each 8-byte aligned address in the data, check if it might be an
    // absolute pointer. Only entries that look like image-base-relative
    // addresses (>= PE_IMAGE_BASE, < PE_IMAGE_BASE + 4GB) need relocs.
    for off in (0..data.len()).step_by(8) {
        if off + 8 > data.len() {
            break;
        }
        let val = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
        if val >= PE_IMAGE_BASE && val < PE_IMAGE_BASE + 0x100000000 {
            let cur_page = off & !(page_size - 1);
            if cur_page != page {
                flush_page(page, &mut entries_for_page, &mut reloc);
                page = cur_page;
            }
            let entry = 0xA000u16 | ((off - cur_page) as u16); // IMAGE_REL_BASED_DIR64
            entries_for_page.push(entry);
        }
    }
    flush_page(page, &mut entries_for_page, &mut reloc);
    reloc
}

/// Full PE32+ linker: .text / .rdata / .data / .idata / .reloc
fn write_pe(inputs: &[PathBuf], output: &Path) -> Result<(), String> {
    if inputs.is_empty() {
        return Err("at least one input object is required".to_string());
    }

    // ── 1. Read & classify all inputs ────────────────────────────────────
    let objs: Vec<CoffSections> = inputs
        .iter()
        .map(|p| read_coff_full(p))
        .collect::<Result<_, _>>()?;

    // ── 2. Merge sections ────────────────────────────────────────────────
    let mut merged_text = Vec::new();
    let mut merged_rdata = Vec::new();
    let mut merged_data = Vec::new();

    let mut bases: Vec<SectionBase> = Vec::new();
    let mut global_syms: HashMap<String, (SectionClass, u64)> = HashMap::new();

    for obj in &objs {
        let tb = align_up(merged_text.len(), 16);
        merged_text.resize(tb, 0x90);
        let rb = align_up(merged_rdata.len(), 16);
        merged_rdata.resize(rb, 0x00);
        let db = align_up(merged_data.len(), 16);
        merged_data.resize(db, 0x00);

        bases.push(SectionBase {
            text_base: tb,
            rdata_base: rb,
            data_base: db,
        });

        for (name, class, off) in &obj.symbols {
            let abs = match class {
                SectionClass::Text => tb as u64 + off,
                SectionClass::Rodata => rb as u64 + off,
                SectionClass::Data => db as u64 + off,
            };
            if global_syms.insert(name.clone(), (*class, abs)).is_some() {
                return Err(format!("duplicate definition of symbol '{name}'"));
            }
        }

        merged_text.extend_from_slice(&obj.text);
        merged_rdata.extend_from_slice(&obj.rdata);
        merged_data.extend_from_slice(&obj.data);
    }

    // ── 3. Collect imports and refptrs ───────────────────────────────────
    let mut kernel_imports: Vec<String> = Vec::new();
    let mut refptr_names: Vec<String> = Vec::new();

    for obj in &objs {
        for rel in &obj.relocations {
            if let Some(name) = rel.target.strip_prefix("__imp_") {
                let n = name.to_string();
                if !kernel_imports.contains(&n) {
                    kernel_imports.push(n);
                }
            } else if let Some(name) = rel.target.strip_prefix(".refptr.") {
                let n = name.to_string();
                if !refptr_names.contains(&n) {
                    refptr_names.push(n);
                }
            }
        }
    }

    // Resolve .refptr. entries AFTER section layout is known, so we can
    // compute correct RVAs from the global symbol offsets.
    // .refptr. slots live in the .data section.
    let refptr_data_off = merged_data.len();
    merged_data.resize(refptr_data_off + refptr_names.len() * 8, 0);

    // ── 4. Layout ────────────────────────────────────────────────────────
    let text_rva = PE_SECTION_RVA;
    let text_raw_size = pe_align(merged_text.len(), PE_FILE_ALIGN);

    let rdata_rva = pe_align(text_rva as usize + merged_text.len(), PE_SECT_ALIGN) as u32;
    let rdata_raw_size = pe_align(merged_rdata.len(), PE_FILE_ALIGN);

    let data_rva = pe_align(rdata_rva as usize + merged_rdata.len(), PE_SECT_ALIGN) as u32;
    let data_raw_size = pe_align(merged_data.len(), PE_FILE_ALIGN);

    // Now fill .refptr. slots using the now-known RVAs.
    for (i, name) in refptr_names.iter().enumerate() {
        if let Some((class, abs)) = global_syms.get(name) {
            let rva = match class {
                SectionClass::Text => text_rva as u64 + abs,
                SectionClass::Rodata => rdata_rva as u64 + abs,
                SectionClass::Data => data_rva as u64 + abs,
            };
            let addr = PE_IMAGE_BASE + rva;
            merged_data[refptr_data_off + i * 8..][..8].copy_from_slice(&addr.to_le_bytes());
        }
    }

    // Build imports
    let idata_rva = pe_align(data_rva as usize + merged_data.len(), PE_SECT_ALIGN) as u32;
    let import = build_imports(&kernel_imports, &refptr_names, idata_rva)?;
    let has_idata = !import.data.is_empty();
    let idata_raw_size = if has_idata {
        pe_align(import.data.len(), PE_FILE_ALIGN)
    } else {
        0
    };

    // Base relocations on .data + .idata
    let mut all_writable = merged_data.clone();
    if has_idata {
        all_writable.extend_from_slice(&import.data);
    }
    let reloc_data = generate_base_relocs(&all_writable, data_rva);
    let reloc_rva = if !reloc_data.is_empty() {
        pe_align(
            if has_idata {
                idata_rva as usize + import.data.len()
            } else {
                data_rva as usize + merged_data.len()
            },
            PE_SECT_ALIGN,
        ) as u32
    } else {
        0
    };
    let has_reloc = !reloc_data.is_empty();
    let reloc_raw_size = if has_reloc {
        pe_align(reloc_data.len(), PE_FILE_ALIGN)
    } else {
        0
    };

    // ── 5. Resolve relocations ───────────────────────────────────────────
    // We need to apply relocations into the merged buffers.
    // First, build a lookup: symbol name → (class, rva-relative offset)

    for (idx, obj) in objs.iter().enumerate() {
        let b = &bases[idx];
        for rel in &obj.relocations {
            let patch_class = section_class_for_offset(
                rel.offset,
                b.text_base,
                merged_text.len(),
                b.rdata_base,
                merged_rdata.len(),
                b.data_base,
                merged_data.len(),
            );
            let (patch_buf, patch_rva) = match patch_class {
                SectionClass::Text => (&mut merged_text, text_rva),
                SectionClass::Rodata => (&mut merged_rdata, rdata_rva),
                SectionClass::Data => (&mut merged_data, data_rva),
            };
            let patch = rel.offset;
            let patch_rva_addr = patch_rva as i64 + patch as i64;

            // Resolve target
            let target = resolve_pe_target(
                &rel,
                &global_syms,
                &import.iat_rvas,
                &import.refptr_offsets,
                &bases[idx],
                text_rva,
                rdata_rva,
                data_rva,
                idata_rva,
            )?;

            let rnum = coff_reloc_number(rel);

            match rnum {
                AMD64_ADDR64 => {
                    if patch + 8 > patch_buf.len() {
                        return Err(format!("'{}': ADDR64 patch OOB", obj.path.display()));
                    }
                    // ADDR64 stores a 64-bit absolute virtual address.
                    let abs_addr = PE_IMAGE_BASE + target;
                    patch_buf[patch..patch + 8].copy_from_slice(&abs_addr.to_le_bytes());
                }
                AMD64_ADDR32 | AMD64_ADDR32NB => {
                    if patch + 4 > patch_buf.len() {
                        return Err(format!("'{}': ADDR32 patch OOB", obj.path.display()));
                    }
                    // ADDR32 stores a 32-bit truncated absolute virtual address.
                    let abs32 = (PE_IMAGE_BASE + target) as u32;
                    patch_buf[patch..patch + 4].copy_from_slice(&abs32.to_le_bytes());
                }
                AMD64_REL32 | AMD64_REL32_1 | AMD64_REL32_2 | AMD64_REL32_3 | AMD64_REL32_4
                | AMD64_REL32_5 => {
                    if patch + 4 > patch_buf.len() {
                        return Err(format!("'{}': REL32 patch OOB", obj.path.display()));
                    }
                    let adjustment: i64 = match rnum {
                        AMD64_REL32_1 => 1,
                        AMD64_REL32_2 => 2,
                        AMD64_REL32_3 => 3,
                        AMD64_REL32_4 => 4,
                        AMD64_REL32_5 => 5,
                        _ => 0,
                    };
                    // REL32 displacement = target_RVA - (next_instruction_RVA)
                    let disp = target as i64 + rel.addend - (patch_rva_addr + 4 + adjustment);
                    if disp < i32::MIN as i64 || disp > i32::MAX as i64 {
                        return Err(format!(
                            "'{}': REL32 displacement overflow ({disp})",
                            obj.path.display()
                        ));
                    }
                    patch_buf[patch..patch + 4].copy_from_slice(&(disp as i32).to_le_bytes());
                }
                AMD64_SECTION => {
                    // Section index reloc — not needed in executable, skip
                }
                AMD64_SECREL => {
                    if patch + 4 > patch_buf.len() {
                        return Err(format!("'{}': SECREL patch OOB", obj.path.display()));
                    }
                    // SECREL: stored as 32-bit unsigned value.
                    let val32 = target as u32;
                    patch_buf[patch..patch + 4].copy_from_slice(&val32.to_le_bytes());
                }
                _ => {
                    return Err(format!(
                        "'{}': unsupported COFF relocation type {rnum}",
                        obj.path.display()
                    ));
                }
            }
        }
    }

    // ── 6. Resolve .refptr. data section entries ─────────────────────────
    for (i, name) in refptr_names.iter().enumerate() {
        let pos = refptr_data_off + i * 8;
        if pos + 8 > merged_data.len() {
            continue;
        }
        let val = u64::from_le_bytes(merged_data[pos..pos + 8].try_into().unwrap());
        if val != 0 {
            continue; // Already resolved
        }
        if let Some((class, abs)) = global_syms.get(name) {
            let rva = match class {
                SectionClass::Text => text_rva as u64 + abs,
                SectionClass::Rodata => rdata_rva as u64 + abs,
                SectionClass::Data => data_rva as u64 + abs,
            };
            let addr = PE_IMAGE_BASE + rva;
            merged_data[pos..pos + 8].copy_from_slice(&addr.to_le_bytes());
        }
    }

    // ── 7. Compute raw file offsets ──────────────────────────────────────
    let headers_size = PE_FILE_ALIGN;
    let text_raw_off = headers_size;
    let rdata_raw_off = text_raw_off + text_raw_size;
    let data_raw_off = rdata_raw_off + rdata_raw_size;
    let idata_raw_off = data_raw_off + data_raw_size;
    let reloc_raw_off = idata_raw_off + idata_raw_size;

    // Section count
    let mut section_count: u16 = 1; // .text always present
    if !merged_rdata.is_empty() {
        section_count += 1;
    }
    if !merged_data.is_empty() || !refptr_names.is_empty() {
        section_count += 1;
    }
    if has_idata {
        section_count += 1;
    }
    if has_reloc {
        section_count += 1;
    }

    let image_end = reloc_rva as usize + if has_reloc { reloc_data.len() } else { 0 };
    let image_size = pe_align(
        if image_end > 0 {
            image_end
        } else {
            data_rva as usize + merged_data.len()
        },
        PE_SECT_ALIGN,
    );
    let file_size = reloc_raw_off + reloc_raw_size;

    let mut pe = vec![0u8; file_size.max(headers_size)];

    // ── 8. DOS + PE headers ──────────────────────────────────────────────
    pe[0..2].copy_from_slice(b"MZ");
    put_u32(&mut pe, 0x3c, 0x80);
    let nt = 0x80;
    pe[nt..nt + 4].copy_from_slice(b"PE\0\0");
    put_u16(&mut pe, nt + 4, 0x8664); // x86-64
    put_u16(&mut pe, nt + 6, section_count);
    // SizeOfOptionalHeader
    let opt_size: u16 = 0xF0;
    put_u16(&mut pe, nt + 20, opt_size);
    put_u16(&mut pe, nt + 22, 0x0022); // EXE, large-address-aware

    let opt = nt + 24;
    put_u16(&mut pe, opt, 0x20b); // PE32+
    // SizeOfCode
    put_u32(&mut pe, opt + 4, text_raw_size as u32);
    // SizeOfInitializedData
    put_u32(
        &mut pe,
        opt + 8,
        (rdata_raw_size + data_raw_size + idata_raw_size) as u32,
    );
    // SizeOfUninitializedData = 0 (bss is merged into .data)
    // EntryPoint
    let main_abs = global_syms
        .get("main")
        .map(|(c, a)| match c {
            SectionClass::Text => text_rva as u64 + a,
            _ => text_rva as u64 + a,
        })
        .ok_or_else(|| "required symbol 'main' not found".to_string())?;
    put_u32(&mut pe, opt + 16, main_abs as u32);
    // BaseOfCode
    put_u32(&mut pe, opt + 20, text_rva);
    // ImageBase
    put_u64(&mut pe, opt + 24, PE_IMAGE_BASE);
    // SectionAlignment / FileAlignment
    put_u32(&mut pe, opt + 32, PE_SECT_ALIGN as u32);
    put_u32(&mut pe, opt + 36, PE_FILE_ALIGN as u32);
    // MajorOSVersion / MinorOSVersion
    put_u16(&mut pe, opt + 40, 6);
    put_u16(&mut pe, opt + 48, 6);
    // SizeOfImage
    put_u32(&mut pe, opt + 56, image_size as u32);
    // SizeOfHeaders
    put_u32(&mut pe, opt + 60, headers_size as u32);
    // Subsystem = console
    put_u16(&mut pe, opt + 68, 3);
    // DLL characteristics
    put_u16(&mut pe, opt + 70, 0x8100); // NX_COMPAT | HIGH_ENTROPY_VA (fixed base address)
    // Stack reserve / commit
    put_u64(&mut pe, opt + 72, 0x100000);
    put_u64(&mut pe, opt + 80, 0x1000);
    // Heap reserve / commit
    put_u64(&mut pe, opt + 88, 0x100000);
    put_u64(&mut pe, opt + 96, 0x1000);
    // NumberOfRvaAndSizes
    put_u32(&mut pe, opt + 108, 16);

    // Data directories
    let dirs = opt + 112;
    // Import directory (index 1)
    if has_idata {
        put_u32(&mut pe, dirs + 8, idata_rva);
        // Size of Import Directory Table array is 40 bytes (1 descriptor for KERNEL32.dll + 1 NULL descriptor)
        put_u32(&mut pe, dirs + 12, 40);
        // IAT directory (index 12)
        put_u32(&mut pe, dirs + 12 * 8, import.iat_rva);
        put_u32(
            &mut pe,
            dirs + 12 * 8 + 4,
            ((kernel_imports.len() + 1) * 8) as u32,
        );
    }
    // Base relocation directory (index 5)
    if has_reloc {
        put_u32(&mut pe, dirs + 5 * 8, reloc_rva);
        put_u32(&mut pe, dirs + 5 * 8 + 4, reloc_data.len() as u32);
    }

    // ── 9. Section headers ───────────────────────────────────────────────
    let mut sec = opt + opt_size as usize;

    // Helper to emit a section header.
    let emit_section = |pe: &mut [u8],
                        sec: &mut usize,
                        name: &[u8; 8],
                        rva: u32,
                        raw_size: usize,
                        raw_off: usize,
                        virt_size: usize,
                        characteristics: u32| {
        pe[*sec..*sec + 8].copy_from_slice(name);
        put_u32(pe, *sec + 8, virt_size as u32);
        put_u32(pe, *sec + 12, rva);
        put_u32(pe, *sec + 16, raw_size as u32);
        put_u32(pe, *sec + 20, raw_off as u32);
        put_u32(pe, *sec + 36, characteristics);
        *sec += 40;
    };

    // .text
    let mut tname = [0u8; 8];
    tname[..5].copy_from_slice(b".text");
    emit_section(
        &mut pe,
        &mut sec,
        &tname,
        text_rva,
        text_raw_size,
        text_raw_off,
        merged_text.len(),
        0x60000020, // RX | CNT_CODE | MEM_EXECUTE | MEM_READ
    );

    // .rdata
    if !merged_rdata.is_empty() {
        let mut rname = [0u8; 8];
        rname[..6].copy_from_slice(b".rdata");
        emit_section(
            &mut pe,
            &mut sec,
            &rname,
            rdata_rva,
            rdata_raw_size,
            rdata_raw_off,
            merged_rdata.len(),
            0x40000040, // R | CNT_INITIALIZED_DATA | MEM_READ
        );
    }

    // .data
    if !merged_data.is_empty() || !refptr_names.is_empty() {
        let mut dname = [0u8; 8];
        dname[..5].copy_from_slice(b".data");
        emit_section(
            &mut pe,
            &mut sec,
            &dname,
            data_rva,
            data_raw_size,
            data_raw_off,
            merged_data.len(),
            0xC0000040, // RW | CNT_INITIALIZED_DATA | MEM_READ | MEM_WRITE
        );
    }

    // .idata
    if has_idata {
        let mut iname = [0u8; 8];
        iname[..6].copy_from_slice(b".idata");
        emit_section(
            &mut pe,
            &mut sec,
            &iname,
            idata_rva,
            idata_raw_size,
            idata_raw_off,
            import.data.len(),
            0xC0000040, // RW | CNT_INITIALIZED_DATA
        );
    }

    // .reloc
    if has_reloc {
        let mut rlname = [0u8; 8];
        rlname[..6].copy_from_slice(b".reloc");
        emit_section(
            &mut pe,
            &mut sec,
            &rlname,
            reloc_rva,
            reloc_raw_size,
            reloc_raw_off,
            reloc_data.len(),
            0x42000040, // R | CNT_INITIALIZED_DATA | MEM_READ | MEM_DISCARDABLE
        );
    }

    // ── 10. Write section data ──────────────────────────────────────────
    pe[text_raw_off..text_raw_off + merged_text.len()].copy_from_slice(&merged_text);
    if !merged_rdata.is_empty() {
        pe[rdata_raw_off..rdata_raw_off + merged_rdata.len()].copy_from_slice(&merged_rdata);
    }
    if !merged_data.is_empty() || !refptr_names.is_empty() {
        pe[data_raw_off..data_raw_off + merged_data.len()].copy_from_slice(&merged_data);
    }
    if has_idata {
        pe[idata_raw_off..idata_raw_off + import.data.len()].copy_from_slice(&import.data);
    }
    if has_reloc {
        pe[reloc_raw_off..reloc_raw_off + reloc_data.len()].copy_from_slice(&reloc_data);
    }

    fs::write(output, pe).map_err(|e| format!("write '{}': {e}", output.display()))?;
    Ok(())
}

/// Find which section class an offset (relative to the merged buffer start)
/// belongs to.
fn section_class_for_offset(
    offset: usize,
    text_base: usize,
    text_len: usize,
    rdata_base: usize,
    rdata_len: usize,
    data_base: usize,
    data_len: usize,
) -> SectionClass {
    if offset >= text_base && offset < text_base + text_len {
        SectionClass::Text
    } else if offset >= rdata_base && offset < rdata_base + rdata_len {
        SectionClass::Rodata
    } else if offset >= data_base && offset < data_base + data_len {
        SectionClass::Data
    } else {
        SectionClass::Text // fallback
    }
}

/// Resolve a PE relocation target to its RVA (NOT absolute address).  The
/// caller adds PE_IMAGE_BASE for absolute relocation types (ADDR64, ADDR32)
/// and uses the bare RVA for PC-relative computations (REL32).
fn resolve_pe_target(
    rel: &Relocation,
    global_syms: &HashMap<String, (SectionClass, u64)>,
    iat_rvas: &HashMap<String, u32>,
    _refptr_offsets: &HashMap<String, usize>,
    bases: &SectionBase,
    text_rva: u32,
    rdata_rva: u32,
    data_rva: u32,
    _idata_rva: u32,
) -> Result<u64, String> {
    // Self-references — return the RVA of the section base within this input
    if rel.target.starts_with("__self_text__") {
        return Ok(text_rva as u64 + bases.text_base as u64);
    }
    if rel.target.starts_with("__self_rdata__") {
        return Ok(rdata_rva as u64 + bases.rdata_base as u64);
    }
    if rel.target.starts_with("__self_data__") {
        return Ok(data_rva as u64 + bases.data_base as u64);
    }

    // External section references ("__ext_text__<base>", etc.)
    if let Some(rest) = rel.target.strip_prefix("__ext_text__") {
        let ext_base: usize = rest
            .parse()
            .map_err(|_| format!("invalid __ext_text__ tag: {}", rel.target))?;
        return Ok(text_rva as u64 + ext_base as u64);
    }
    if let Some(rest) = rel.target.strip_prefix("__ext_rdata__") {
        let ext_base: usize = rest
            .parse()
            .map_err(|_| format!("invalid __ext_rdata__ tag: {}", rel.target))?;
        return Ok(rdata_rva as u64 + ext_base as u64);
    }
    if let Some(rest) = rel.target.strip_prefix("__ext_data__") {
        let ext_base: usize = rest
            .parse()
            .map_err(|_| format!("invalid __ext_data__ tag: {}", rel.target))?;
        return Ok(data_rva as u64 + ext_base as u64);
    }

    // IAT entry — returned as bare RVA
    if let Some(rva) = iat_rvas.get(&rel.target) {
        return Ok(*rva as u64);
    }

    // Global symbol — compute RVA from section base + internal offset
    if let Some((class, abs)) = global_syms.get(&rel.target) {
        let rva = match class {
            SectionClass::Text => text_rva as u64 + abs,
            SectionClass::Rodata => rdata_rva as u64 + abs,
            SectionClass::Data => data_rva as u64 + abs,
        };
        return Ok(rva);
    }

    // .refptr. symbols — fallback to the underlying symbol's RVA
    if let Some(name) = rel.target.strip_prefix(".refptr.") {
        if let Some((class, abs)) = global_syms.get(name) {
            let rva = match class {
                SectionClass::Text => text_rva as u64 + abs,
                SectionClass::Rodata => rdata_rva as u64 + abs,
                SectionClass::Data => data_rva as u64 + abs,
            };
            return Ok(rva);
        }
    }

    // Legacy __coff_ section markers from old format
    if let Some(rest) = rel.target.strip_prefix("__coff_text_section_") {
        let off: u32 = rest
            .parse()
            .map_err(|_| format!("invalid __coff_text_section_ tag"))?;
        return Ok(text_rva as u64 + off as u64);
    }

    Err(format!("unresolved external COFF symbol '{}'", rel.target))
}

// ═══════════════════════════════════════════════════════════════════════════
//  3.  Mach-O path  (Phase M2 — direct emitter, kept stable)
// ═══════════════════════════════════════════════════════════════════════════

fn read_macho_input(path: &Path) -> Result<MachoInput, String> {
    let bytes = fs::read(path).map_err(|e| format!("read '{}': {e}", path.display()))?;
    let file =
        object::File::parse(&*bytes).map_err(|e| format!("parse '{}': {e}", path.display()))?;
    if file.format() != BinaryFormat::MachO {
        return Err(format!(
            "'{}' is not a Mach-O relocatable object",
            path.display()
        ));
    }
    let mut text = Vec::new();
    let mut sec_bases: Vec<(object::SectionIndex, usize)> = Vec::new();
    let mut sec_relocs = Vec::new();

    for sec in file.sections() {
        if sec.kind() != object::SectionKind::Text {
            continue;
        }
        let base = align_up(text.len(), 16);
        text.resize(base, 0x90);
        let idx = sec.index();
        let data = sec
            .uncompressed_data()
            .map_err(|e| format!("read text from '{}': {e}", path.display()))?
            .into_owned();
        text.extend_from_slice(&data);
        sec_bases.push((idx, base));
        for (off, rel) in sec.relocations() {
            sec_relocs.push((idx, base, off, rel));
        }
    }
    if sec_bases.is_empty() {
        return Err(format!(
            "'{}' has no executable Mach-O text section",
            path.display()
        ));
    }
    let find_base = |idx: object::SectionIndex| -> Option<usize> {
        sec_bases.iter().find(|(i, _)| *i == idx).map(|(_, b)| *b)
    };

    let mut text_syms = Vec::new();
    for sym in file.symbols() {
        if let SymbolSection::Section(idx) = sym.section() {
            if let Some(base) = find_base(idx) {
                if let Ok(name) = sym.name() {
                    let clean = name.strip_prefix('_').unwrap_or(name);
                    if !clean.is_empty() {
                        text_syms.push((clean.to_string(), base as u64 + sym.address()));
                    }
                }
            }
        }
    }

    let mut relocs = Vec::new();
    for (_, base, off, rel) in sec_relocs {
        let RelocationTarget::Symbol(si) = rel.target() else {
            return Err(format!(
                "'{}' has unsupported non-symbol relocation",
                path.display()
            ));
        };
        let sym = file
            .symbol_by_index(si)
            .map_err(|e| format!("read relocation symbol: {e}"))?;
        let raw_name = sym
            .name()
            .map_err(|e| format!("read relocation symbol name: {e}"))?;
        let clean = raw_name.strip_prefix('_').unwrap_or(raw_name);
        let target = if clean.is_empty() {
            match sym.section() {
                SymbolSection::Section(idx) if find_base(idx).is_some() => {
                    format!("__macho_text_section_{}", find_base(idx).unwrap())
                }
                _ => {
                    return Err(format!(
                        "'{}' has unresolved anonymous Mach-O relocation",
                        path.display()
                    ));
                }
            }
        } else {
            clean.to_string()
        };
        relocs.push(Relocation {
            offset: base + usize::try_from(off).map_err(|_| "relocation offset overflow")?,
            target,
            addend: rel.addend(),
            size: rel.size(),
            kind: rel.kind(),
        });
    }
    Ok(MachoInput {
        path: path.to_path_buf(),
        text,
        text_symbols: text_syms,
        relocations: relocs,
    })
}

fn write_macho(inputs: &[PathBuf], output: &Path) -> Result<(), String> {
    if inputs.is_empty() {
        return Err("at least one input object is required".to_string());
    }
    let objs: Vec<MachoInput> = inputs
        .iter()
        .map(|p| read_macho_input(p))
        .collect::<Result<_, _>>()?;

    let mut text = Vec::new();
    let mut bases = Vec::new();
    let mut syms: HashMap<String, u64> = HashMap::new();
    for inp in &objs {
        let base = align_up(text.len(), 16);
        text.resize(base, 0x90);
        bases.push(base);
        for (n, o) in &inp.text_symbols {
            let abs = base as u64 + o;
            if syms.insert(n.clone(), abs).is_some() {
                return Err(format!("duplicate definition of symbol '{n}'"));
            }
        }
        text.extend_from_slice(&inp.text);
    }
    let main = *syms
        .get("main")
        .ok_or_else(|| "required symbol 'main' or '_main' not found".to_string())?;

    for (idx, inp) in objs.iter().enumerate() {
        let base = bases[idx];
        for rel in &inp.relocations {
            let tgt_off = if rel.target == "__self_text__" {
                base as u64
            } else if let Some(off) = rel.target.strip_prefix("__macho_text_section_") {
                off.parse::<u64>()
                    .map_err(|_| "invalid Mach-O section relocation")?
            } else {
                *syms.get(&rel.target).ok_or_else(|| {
                    format!(
                        "'{}': unresolved external symbol '{}'",
                        inp.path.display(),
                        rel.target
                    )
                })?
            };
            let patch = base + rel.offset;
            if patch + 4 > text.len() {
                return Err(format!(
                    "'{}': relocation patch out of range",
                    inp.path.display()
                ));
            }
            let disp = tgt_off as i64 + rel.addend - patch as i64;
            if disp < i32::MIN as i64 || disp > i32::MAX as i64 {
                return Err(format!(
                    "'{}': PC-relative relocation out of range",
                    inp.path.display()
                ));
            }
            text[patch..patch + 4].copy_from_slice(&(disp as i32).to_le_bytes());
        }
    }

    let text_page = align_up(text.len(), 4096);
    let mut header = Vec::new();
    header.extend_from_slice(&0xfeedfacfu32.to_le_bytes());
    header.extend_from_slice(&0x01000007u32.to_le_bytes());
    header.extend_from_slice(&3u32.to_le_bytes());
    header.extend_from_slice(&2u32.to_le_bytes());
    header.extend_from_slice(&2u32.to_le_bytes());
    let sizeofcmds = (72 + 152) as u32;
    header.extend_from_slice(&sizeofcmds.to_le_bytes());
    header.extend_from_slice(&0x00200085u32.to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes());

    // PAGEZERO
    header.extend_from_slice(&0x19u32.to_le_bytes());
    header.extend_from_slice(&72u32.to_le_bytes());
    let mut pz = [0u8; 16];
    pz[..10].copy_from_slice(b"__PAGEZERO");
    header.extend_from_slice(&pz);
    header.extend_from_slice(&0u64.to_le_bytes());
    header.extend_from_slice(&0x100000000u64.to_le_bytes());
    header.extend_from_slice(&0u64.to_le_bytes());
    header.extend_from_slice(&0u64.to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes());

    // TEXT
    header.extend_from_slice(&0x19u32.to_le_bytes());
    header.extend_from_slice(&152u32.to_le_bytes());
    let mut ts = [0u8; 16];
    ts[..6].copy_from_slice(b"__TEXT");
    header.extend_from_slice(&ts);
    header.extend_from_slice(&0x100000000u64.to_le_bytes());
    header.extend_from_slice(&(4096 + text_page as u64).to_le_bytes());
    header.extend_from_slice(&0u64.to_le_bytes());
    header.extend_from_slice(&(4096 + text.len() as u64).to_le_bytes());
    header.extend_from_slice(&7u32.to_le_bytes());
    header.extend_from_slice(&5u32.to_le_bytes());
    header.extend_from_slice(&1u32.to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes());

    let mut tn = [0u8; 16];
    tn[..6].copy_from_slice(b"__text");
    header.extend_from_slice(&tn);
    header.extend_from_slice(&ts);
    header.extend_from_slice(&(0x100000000u64 + 4096 + main).to_le_bytes());
    header.extend_from_slice(&(text.len() as u64).to_le_bytes());
    header.extend_from_slice(&4096u32.to_le_bytes());
    header.extend_from_slice(&4u32.to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes());
    header.extend_from_slice(&0x80000400u32.to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes());

    let mut bin = vec![0u8; 4096];
    bin[..header.len()].copy_from_slice(&header);
    bin.extend_from_slice(&text);
    fs::write(output, bin)
        .map_err(|e| format!("write Mach-O binary '{}': {e}", output.display()))?;
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
//  4.  inspect  (cross-format object introspection)
// ═══════════════════════════════════════════════════════════════════════════

fn inspect_object(input: &Path) -> Result<(), String> {
    let bytes = fs::read(input).map_err(|e| format!("read '{}': {e}", input.display()))?;
    let file =
        object::File::parse(&*bytes).map_err(|e| format!("parse '{}': {e}", input.display()))?;
    let mut reloc_count = 0usize;
    let mut reloc_kinds: BTreeMap<String, usize> = BTreeMap::new();
    println!("format: {:?}", file.format());
    println!("architecture: {:?}", file.architecture());
    println!("sections:");
    for sec in file.sections() {
        for (_, rel) in sec.relocations() {
            reloc_count += 1;
            *reloc_kinds.entry(format!("{:?}", rel.kind())).or_default() += 1;
        }
        println!(
            "  {} size={} kind={:?}",
            sec.name().unwrap_or("<unnamed>"),
            sec.size(),
            sec.kind()
        );
    }
    let defined = file.symbols().filter(|s| !s.is_undefined()).count();
    let undefined = file.symbols().filter(|s| s.is_undefined()).count();
    println!("symbols: defined={defined} undefined={undefined}");
    println!("relocations: {reloc_count}");
    println!("relocation-kinds:");
    for (k, c) in reloc_kinds {
        println!("  {k}={c}");
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
//  5.  main
// ═══════════════════════════════════════════════════════════════════════════

fn usage() {
    eprintln!("Usage: lpp-link <program.o> [runtime.o ...] -o <output>");
    eprintln!("       lpp-link pe <program.obj> [runtime.obj ...] -o <output.exe>");
    eprintln!("       lpp-link macho <program.o> [runtime.o ...] -o <output>");
    eprintln!("       lpp-link inspect <object.o>");
    eprintln!(
        "Phases: direct Linux x86-64 ELF linker; Windows PE COFF linker; macOS Mach-O direct emitter."
    );
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("inspect") {
        if args.len() != 2 {
            usage();
            std::process::exit(2);
        }
        if let Err(e) = inspect_object(Path::new(&args[1])) {
            eprintln!("lpp-link inspect error: {e}");
            std::process::exit(1);
        }
        return;
    }
    let pe_mode = args.first().map(String::as_str) == Some("pe");
    let macho_mode = args.first().map(String::as_str) == Some("macho");
    let offset = if pe_mode || macho_mode { 1 } else { 0 };
    let Some(output_rel) = args[offset..].iter().position(|a| a == "-o") else {
        usage();
        std::process::exit(2);
    };
    let out_idx = offset + output_rel;
    if out_idx == offset || out_idx + 2 != args.len() {
        usage();
        std::process::exit(2);
    }
    let inputs: Vec<PathBuf> = args[offset..out_idx].iter().map(PathBuf::from).collect();
    let result = if pe_mode {
        write_pe(&inputs, Path::new(&args[out_idx + 1]))
    } else if macho_mode {
        write_macho(&inputs, Path::new(&args[out_idx + 1]))
    } else {
        write_elf(&inputs, Path::new(&args[out_idx + 1]))
    };
    if let Err(e) = result {
        eprintln!("lpp-link error: {e}");
        std::process::exit(1);
    }
}
