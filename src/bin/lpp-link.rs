//! `lpp-link` — direct linker for Linux ELF, Windows PE, and macOS Mach-O.
//!
//! Phase 2+: ELF with GOT/rodata merge, PE with full multi-section
//! (.text/.rdata/.data/.bss/.idata), base relocations, and broad AMD64
//! relocation coverage.  Mach-O direct emitter for the verified subset.
//!
//! The linker deliberately grows in small verified slices.  Each format gets
//! exactly the section and relocation support it needs for the verified
//! workload set — nothing more, nothing less.

/// Set freestanding runtime compilation mode flag for direct linker targets.
pub const LPP_FREESTANDING: bool = true;

use object::read::archive::ArchiveFile;
use object::{
    Architecture, BinaryFormat, Object, ObjectSection, ObjectSymbol, RelocationKind,
    RelocationTarget, SymbolSection,
};
use std::collections::{BTreeMap, HashMap};
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
    Tls,
}

struct Relocation {
    offset: usize,
    target: String,
    addend: i64,
    size: u8,
    kind: RelocationKind,
    /// Which merged section this relocation patches (set during COFF parse).
    section_class: SectionClass,
    /// Raw COFF relocation type (IMAGE_REL_AMD64_*), 0 for ELF/Mach-O.
    coff_type: u16,
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
    /// Merged thread-local storage bytes.
    tls: Vec<u8>,
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

fn parse_elf_object(file: &object::File, path: &Path) -> Result<ElfInput, String> {
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

    let mut rodata_map: HashMap<object::SectionIndex, usize> = HashMap::new();
    let mut rodata = Vec::new();
    for sec in file.sections() {
        if let Ok(name) = sec.name() {
            if name == ".rodata" || name.starts_with(".rodata.") {
                let align = usize::try_from(sec.align()).unwrap_or(16).max(1);
                let base = align_up(rodata.len(), align);
                rodata.resize(base, 0);
                rodata_map.insert(sec.index(), base);
                if let Ok(d) = sec.uncompressed_data() {
                    rodata.extend_from_slice(&d);
                }
            }
        }
    }
    let is_rodata = |s: SymbolSection| match s {
        SymbolSection::Section(i) => rodata_map.contains_key(&i),
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
                    let sec_base = match sym.section() {
                        SymbolSection::Section(i) => rodata_map.get(&i).copied().unwrap_or(0),
                        _ => 0,
                    };
                    dst.push((n.to_string(), sec_base as u64 + sym.address()));
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
        let (target, addend) = match sym.section() {
            SymbolSection::Section(i) if i == text_idx => (
                "__self_text__".to_string(),
                rel.addend() + sym.address() as i64,
            ),
            SymbolSection::Section(i) if rodata_map.contains_key(&i) => {
                let sec_base = rodata_map[&i] as i64;
                (
                    "__self_rodata__".to_string(),
                    rel.addend() + sec_base + sym.address() as i64,
                )
            }
            _ => (raw.to_string(), rel.addend()),
        };
        relocs.push(Relocation {
            offset: usize::try_from(off).map_err(|_| "relocation offset overflow")?,
            target,
            addend,
            size: rel.size(),
            kind: rel.kind(),
            section_class: SectionClass::Text,
                coff_type: 0,
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

fn load_elf_inputs(path: &Path, out: &mut Vec<ElfInput>) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|e| format!("read '{}': {e}", path.display()))?;
    if let Ok(archive) = ArchiveFile::parse(&*bytes) {
        for member in archive.members() {
            if let Ok(member) = member {
                if let Ok(data) = member.data(&*bytes) {
                    if let Ok(file) = object::File::parse(data) {
                        if file.format() == BinaryFormat::Elf && file.architecture() == Architecture::X86_64 {
                            let member_name = String::from_utf8_lossy(member.name()).to_string();
                            let member_path = path.join(&member_name);
                            out.push(parse_elf_object(&file, &member_path)?);
                        }
                    }
                }
            }
        }
        return Ok(());
    }

    let file = object::File::parse(&*bytes).map_err(|e| format!("parse '{}': {e}", path.display()))?;
    out.push(parse_elf_object(&file, path)?);
    Ok(())
}

fn write_elf(inputs: &[PathBuf], output: &Path) -> Result<(), String> {
    if inputs.is_empty() {
        return Err("at least one input object is required".to_string());
    }
    let mut objs: Vec<ElfInput> = Vec::new();
    for p in inputs {
        load_elf_inputs(p, &mut objs)?;
    }

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

    let mut got: HashMap<(String, u64, usize), usize> = HashMap::new();
    for (idx, inp) in objs.iter().enumerate() {
        for rel in &inp.relocations {
            if rel.kind == RelocationKind::GotRelative {
                let sym_offset = if rel.target == "__self_rodata__" || rel.target == "__self_text__" {
                    (rel.addend + 4) as u64
                } else {
                    0u64
                };
                let n = got.len();
                got.entry((rel.target.clone(), sym_offset, idx)).or_insert(n);
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
    for ((name, sym_offset, obj_idx), slot) in &got {
        let base_tgt = match name.as_str() {
            "__self_text__" => bases[*obj_idx] as u64,
            "__self_rodata__" => rodata_bases[*obj_idx] as u64,
            _ => *syms
                .get(name)
                .ok_or_else(|| format!("unresolved GOT symbol '{name}'"))?,
        };
        let tgt = base_tgt + sym_offset;
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
            let (tgt, effective_addend) = match rel.kind {
                RelocationKind::GotRelative => {
                    let (sym_offset, instr_addend) =
                        if rel.target == "__self_rodata__" || rel.target == "__self_text__" {
                            ((rel.addend + 4) as u64, -4i64)
                        } else {
                            (0u64, rel.addend)
                        };
                    let slot = *got
                        .get(&(rel.target.clone(), sym_offset, idx))
                        .ok_or_else(|| "missing GOT slot".to_string())?;
                    (
                        u64::try_from(got_off + slot * 8).map_err(|_| "GOT overflow")?,
                        instr_addend,
                    )
                }
                _ if rel.target == "__self_text__" => {
                    (u64::try_from(base).map_err(|_| "text overflow")?, rel.addend)
                }
                _ if rel.target == "__self_rodata__" => {
                    (u64::try_from(rodata_bases[idx]).map_err(|_| "rodata overflow")?, rel.addend)
                }
                _ => (
                    *syms.get(&rel.target).ok_or_else(|| {
                        format!(
                            "'{}': unresolved external relocation to '{}'",
                            inp.path.display(),
                            rel.target
                        )
                    })?,
                    rel.addend,
                ),
            };
            let patch = base + rel.offset;
            if patch + 4 > text.len() {
                return Err(format!("'{}': patch out of range", inp.path.display()));
            }
            if rel.kind == RelocationKind::Absolute {
                let v = ELF_BASE as i64 + CODE_OFFSET as i64 + tgt as i64 + effective_addend;
                text[patch..patch + 4].copy_from_slice(&(v as i32).to_le_bytes());
            } else {
                let d = tgt as i64 + effective_addend - patch as i64;
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
    // Use the raw COFF type when available (avoids misclassification).
    if rel.coff_type != 0 {
        return rel.coff_type as u8;
    }
    // Fallback for ELF/Mach-O objects (not COFF).
    match rel.kind {
        RelocationKind::Absolute if rel.size == 64 => AMD64_ADDR64,
        RelocationKind::Absolute => AMD64_ADDR32,
        RelocationKind::Relative => AMD64_REL32,
        RelocationKind::SectionIndex => AMD64_SECTION,
        RelocationKind::SectionOffset => AMD64_SECREL,
        _ => {
            if rel.size == 64 {
                AMD64_ADDR64
            } else {
                AMD64_REL32
            }
        }
    }
}

fn parse_coff_object(
    file: &object::File,
    path: &Path,
    _bytes: &[u8],
) -> Result<CoffSections, String> {
    let mut text_buf = Vec::new();
    let mut rdata_buf = Vec::new();
    let mut data_buf = Vec::new();
    let mut tls_buf = Vec::new();
    let mut map: Vec<(object::SectionIndex, SectionClass, usize)> = Vec::new();
    let mut relocs = Vec::new();

    for sec in file.sections() {
        let idx = sec.index();
        let name = sec.name().unwrap_or("");

        // Skip non-loadable debug, directive, and exception handling sections.
        // .xdata and .pdata are Windows SEH tables — they cause SECREL
        // relocations that corrupt .text when improperly handled.  The
        // freestanding runtime doesn't need exception unwinding.
        if name.starts_with(".debug")
            || name.starts_with(".drectve")
            || name.starts_with(".comment")
            || name.starts_with(".note")
            || name.starts_with(".xdata")
            || name.starts_with(".pdata")
        {
            continue;
        }

        // Check PE section characteristics: skip IMAGE_SCN_LNK_REMOVE (0x800) or INFO (0x200)
        if let object::SectionFlags::Coff { characteristics } = sec.flags() {
            if (characteristics & 0x00000800) != 0 || (characteristics & 0x00000200) != 0 {
                continue;
            }
        }

        let kind = sec.kind();
        let class = if name.starts_with(".text") || kind == object::SectionKind::Text {
            SectionClass::Text
        } else if name.starts_with(".rdata")
            || name.starts_with(".rodata")
            || name.starts_with(".xdata")
            || name.starts_with(".pdata")
            || kind == object::SectionKind::ReadOnlyData
            || kind == object::SectionKind::ReadOnlyString
        {
            SectionClass::Rodata
        } else if name.starts_with(".tls") || kind == object::SectionKind::UninitializedTls {
            SectionClass::Tls
        } else if name.starts_with(".data")
            || name.starts_with(".bss")
            || kind == object::SectionKind::Data
            || kind == object::SectionKind::UninitializedData
        {
            SectionClass::Data
        } else {
            continue;
        };

        let buf: &mut Vec<u8> = match class {
            SectionClass::Text => &mut text_buf,
            SectionClass::Rodata => &mut rdata_buf,
            SectionClass::Data => &mut data_buf,
            SectionClass::Tls => &mut tls_buf,
        };
        let sec_align = usize::try_from(sec.align()).unwrap_or(16).max(16);
        let base = align_up(buf.len(), sec_align);
        buf.resize(base, if class == SectionClass::Text { 0xCC } else { 0x00 });

        // BSS / uninitialized sections have zero on-disk size; just reserve virtual space.
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
            // COFF uses implicit addends baked into the instruction bytes.
            // The `object` crate reads that implicit value and returns it as
            // rel.addend(), but our patching code overwrites the bytes with
            // the final computed value.  Using the crate's addend would
            // double-count it, so we always zero it for COFF objects.
            let coff_type = match rel.flags() {
                object::RelocationFlags::Coff { typ } => typ,
                _ => 0,
            };
            relocs.push(Relocation {
                offset: base + raw_off,
                target,
                addend: 0,
                size: rel.size(),
                kind: rel.kind(),
                section_class: class,
                coff_type,
            });
        }

        // Pad to alignment for next section of same class
        let padded = align_up(buf.len(), sec_align);
        buf.resize(padded, if class == SectionClass::Text { 0xCC } else { 0x00 });
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
                        && !name.starts_with(".tls")
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
        tls: tls_buf,
        section_map: map,
        symbols: syms,
        relocations: relocs,
    })
}

fn load_coff_inputs(path: &Path, out: &mut Vec<CoffSections>) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|e| format!("read '{}': {e}", path.display()))?;

    // Check if path is a static library archive (.lib / .a)
    if let Ok(archive) = ArchiveFile::parse(&*bytes) {
        for member in archive.members() {
            if let Ok(member) = member {
                if let Ok(data) = member.data(&*bytes) {
                    if let Ok(file) = object::File::parse(data) {
                        if file.format() == BinaryFormat::Coff && file.architecture() == Architecture::X86_64 {
                            let member_name = String::from_utf8_lossy(member.name()).to_string();
                            let member_path = path.join(&member_name);
                            out.push(parse_coff_object(&file, &member_path, data)?);
                        }
                    }
                }
            }
        }
        return Ok(());
    }

    let file = object::File::parse(&*bytes).map_err(|e| format!("parse '{}': {e}", path.display()))?;
    if file.format() != BinaryFormat::Coff || file.architecture() != Architecture::X86_64 {
        return Err(format!("'{}' is not an x86-64 COFF object or library archive", path.display()));
    }
    out.push(parse_coff_object(&file, path, &bytes)?);
    Ok(())
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
        SectionClass::Tls => "tls",
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
    tls_base: usize,
}

fn is_crt_symbol(name: &str) -> bool {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    matches!(
        clean,
        "malloc"
            | "free"
            | "realloc"
            | "calloc"
            | "printf"
            | "puts"
            | "memset"
            | "memcpy"
            | "memmove"
            | "strlen"
            | "strcmp"
            | "strncmp"
            | "strcpy"
            | "strncpy"
            | "strcat"
            | "strchr"
            | "strstr"
            | "sprintf"
            | "sscanf"
            | "exit"
            | "abort"
            | "sin"
            | "cos"
            | "tan"
            | "pow"
            | "sqrt"
            | "ceil"
            | "floor"
            | "fmod"
            | "fabs"
            | "atan2"
            | "log"
            | "exp"
            | "getchar"
            | "putchar"
            | "fopen"
            | "fclose"
            | "fread"
            | "fwrite"
            | "fflush"
            | "fprintf"
            | "fseek"
            | "ftell"
            | "getenv"
            | "system"
            | "time"
            | "clock"
            | "_errno"
            | "__getmainargs"
            | "__set_app_type"
            | "_acmdln"
            | "_initterm"
            | "_initterm_e"
            | "_configthreadlocale"
    )
}

/// Build the combined import descriptor + ILT + IAT + hint/name table for
/// KERNEL32.dll and msvcrt.dll.  Also reserves space for `.refptr.` internal symbols.
struct ImportData {
    data: Vec<u8>,
    iat_rvas: HashMap<String, u32>,
    refptr_offsets: HashMap<String, usize>,
    #[allow(dead_code)]
    ilt_rva: u32,
    iat_rva: u32,
    iat_size: u32,
    #[allow(dead_code)]
    dll_count: usize,
}

fn build_imports(
    raw_imports: &[String],
    refptrs: &[String],
    section_rva: u32,
) -> Result<ImportData, String> {
    let mut kernel_imports = Vec::new();
    let mut crt_imports = Vec::new();

    for imp in raw_imports {
        let clean = imp.strip_prefix("__imp_").unwrap_or(imp).to_string();
        if is_crt_symbol(&clean) {
            if !crt_imports.contains(&clean) {
                crt_imports.push(clean);
            }
        } else {
            if !kernel_imports.contains(&clean) {
                kernel_imports.push(clean);
            }
        }
    }

    let dll_list = [
        ("KERNEL32.dll", &kernel_imports),
        ("msvcrt.dll", &crt_imports),
    ];
    let active_dlls: Vec<(&str, &Vec<String>)> = dll_list
        .into_iter()
        .filter(|(_, funcs)| !funcs.is_empty())
        .collect();

    let dll_count = active_dlls.len();
    let desc_size = if dll_count == 0 { 0 } else { (dll_count + 1) * 20 };

    let mut total_ilt_iat_entries = 0;
    for (_, funcs) in &active_dlls {
        total_ilt_iat_entries += funcs.len() + 1; // including null terminator for each DLL
    }

    let ilt_size = total_ilt_iat_entries * 8;
    let iat_size = total_ilt_iat_entries * 8;

    let ilt_off = align_up(desc_size, 8);
    let iat_off = ilt_off + ilt_size;
    let refptr_off = align_up(iat_off + iat_size, 8);

    let mut data = vec![0u8; refptr_off + refptrs.len() * 8];
    let mut iat_rvas = HashMap::new();
    let mut refptr_offsets = HashMap::new();

    if dll_count > 0 {
        let mut cur_desc_pos = 0;
        let mut cur_ilt_pos = ilt_off;
        let mut cur_iat_pos = iat_off;

        for (dll_name, funcs) in &active_dlls {
            let dll_name_off = data.len();
            data.extend_from_slice(dll_name.as_bytes());
            data.push(0);
            while data.len() % 2 != 0 {
                data.push(0);
            }

            let mut hint_offsets = HashMap::new();
            for f in *funcs {
                let h_off = data.len();
                data.extend_from_slice(&[0u8, 0u8]); // Hint
                data.extend_from_slice(f.as_bytes());
                data.push(0);
                while data.len() % 2 != 0 {
                    data.push(0);
                }
                hint_offsets.insert(f.clone(), h_off);
            }

            let this_ilt_rva = section_rva + cur_ilt_pos as u32;
            let this_iat_rva = section_rva + cur_iat_pos as u32;
            let this_dll_name_rva = section_rva + dll_name_off as u32;

            put_u32(&mut data, cur_desc_pos, this_ilt_rva);
            put_u32(&mut data, cur_desc_pos + 12, this_dll_name_rva);
            put_u32(&mut data, cur_desc_pos + 16, this_iat_rva);
            cur_desc_pos += 20;

            for f in *funcs {
                let name_rva = section_rva + hint_offsets[f] as u32;
                let thunk = name_rva as u64;

                data[cur_ilt_pos..cur_ilt_pos + 8].copy_from_slice(&thunk.to_le_bytes());
                data[cur_iat_pos..cur_iat_pos + 8].copy_from_slice(&thunk.to_le_bytes());

                let iat_entry_rva = section_rva + cur_iat_pos as u32;
                iat_rvas.insert(format!("__imp_{f}"), iat_entry_rva);
                iat_rvas.insert(f.clone(), iat_entry_rva);

                cur_ilt_pos += 8;
                cur_iat_pos += 8;
            }

            // End of ILT / IAT array for this DLL
            cur_ilt_pos += 8;
            cur_iat_pos += 8;
        }
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
        iat_size: iat_size as u32,
        dll_count,
    })
}

fn generate_base_relocs_from_rvas(rvas: &[u32]) -> Vec<u8> {
    if rvas.is_empty() {
        return Vec::new();
    }
    let mut sorted = rvas.to_vec();
    sorted.sort_unstable();
    sorted.dedup();

    let mut reloc = Vec::new();
    let page_size = 0x1000u32;

    let mut i = 0;
    while i < sorted.len() {
        let page_rva = sorted[i] & !(page_size - 1);
        let mut page_entries = Vec::new();
        while i < sorted.len() && (sorted[i] & !(page_size - 1)) == page_rva {
            let offset_in_page = (sorted[i] & (page_size - 1)) as u16;
            let entry = 0xA000u16 | offset_in_page; // IMAGE_REL_BASED_DIR64
            page_entries.push(entry);
            i += 1;
        }

        let entry_bytes_len = page_entries.len() * 2;
        let block_size = 8 + entry_bytes_len;
        let padded_size = align_up(block_size, 4);

        let start = reloc.len();
        reloc.resize(start + padded_size, 0);
        put_u32(&mut reloc, start, page_rva);
        put_u32(&mut reloc, start + 4, padded_size as u32);
        for (idx, e) in page_entries.iter().enumerate() {
            put_u16(&mut reloc, start + 8 + idx * 2, *e);
        }
    }
    reloc
}

/// Full PE32+ linker: .text / .rdata / .data / .idata / .reloc
fn write_pe(inputs: &[PathBuf], output: &Path) -> Result<(), String> {
    if inputs.is_empty() {
        return Err("at least one input object is required".to_string());
    }

    // ── 1. Read & classify all inputs (including .lib / .a archives) ───
    let mut objs: Vec<CoffSections> = Vec::new();
    for p in inputs {
        load_coff_inputs(p, &mut objs)?;
    }

    // ── 2. Merge sections ────────────────────────────────────────────────
    let mut merged_text = Vec::new();
    let mut merged_rdata = Vec::new();
    let mut merged_data = Vec::new();
    let mut merged_tls = Vec::new();

    let mut bases: Vec<SectionBase> = Vec::new();
    let mut global_syms: HashMap<String, (SectionClass, u64)> = HashMap::new();

    for obj in &objs {
        let tb = align_up(merged_text.len(), 16);
        merged_text.resize(tb, 0x90);
        let rb = align_up(merged_rdata.len(), 16);
        merged_rdata.resize(rb, 0x00);
        let db = align_up(merged_data.len(), 16);
        merged_data.resize(db, 0x00);
        let tlsb = align_up(merged_tls.len(), 16);
        merged_tls.resize(tlsb, 0x00);

        bases.push(SectionBase {
            text_base: tb,
            rdata_base: rb,
            data_base: db,
            tls_base: tlsb,
        });

        for (name, class, off) in &obj.symbols {
            let abs = match class {
                SectionClass::Text => tb as u64 + off,
                SectionClass::Rodata => rb as u64 + off,
                SectionClass::Data => db as u64 + off,
                SectionClass::Tls => tlsb as u64 + off,
            };
            if global_syms.insert(name.clone(), (*class, abs)).is_some() {
                return Err(format!("duplicate definition of symbol '{name}'"));
            }
        }

        merged_text.extend_from_slice(&obj.text);
        merged_rdata.extend_from_slice(&obj.rdata);
        merged_data.extend_from_slice(&obj.data);
        merged_tls.extend_from_slice(&obj.tls);
    }

    // ── 3. Collect imports and refptrs ───────────────────────────────────
    let mut raw_imports: Vec<String> = Vec::new();
    let mut refptr_names: Vec<String> = Vec::new();

    for obj in &objs {
        for rel in &obj.relocations {
            if let Some(name) = rel.target.strip_prefix("__imp_") {
                let n = name.to_string();
                if !raw_imports.contains(&n) {
                    raw_imports.push(n);
                }
            } else if is_crt_symbol(&rel.target) && !global_syms.contains_key(&rel.target) {
                // Only import CRT symbols from msvcrt.dll if they are NOT
                // defined locally in any input object.  The freestanding
                // runtime provides its own memcpy/memset/etc.; redirecting
                // those through the IAT would cause the call to land on
                // pointer data instead of executable code (ACCESS_VIOLATION).
                if !raw_imports.contains(&rel.target) {
                    raw_imports.push(rel.target.clone());
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

    let has_tls = !merged_tls.is_empty();
    let tls_rva = pe_align(data_rva as usize + merged_data.len(), PE_SECT_ALIGN) as u32;
    let tls_raw_size = if has_tls {
        pe_align(merged_tls.len(), PE_FILE_ALIGN)
    } else {
        0
    };

    let idata_rva = pe_align(
        if has_tls {
            tls_rva as usize + merged_tls.len()
        } else {
            data_rva as usize + merged_data.len()
        },
        PE_SECT_ALIGN,
    ) as u32;

    // Build TLS Directory if TLS data is present
    let mut tls_dir_rva = 0u32;
    if has_tls {
        let tls_index_data_off = merged_data.len();
        merged_data.resize(tls_index_data_off + 8, 0);

        let tls_dir_off = merged_rdata.len();
        merged_rdata.resize(tls_dir_off + 40, 0);

        tls_dir_rva = rdata_rva + tls_dir_off as u32;
        let start_va = PE_IMAGE_BASE + tls_rva as u64;
        let end_va = start_va + merged_tls.len() as u64;
        let index_va = PE_IMAGE_BASE + data_rva as u64 + tls_index_data_off as u64;

        put_u64(&mut merged_rdata, tls_dir_off, start_va);      // StartAddressOfRawData
        put_u64(&mut merged_rdata, tls_dir_off + 8, end_va);    // EndAddressOfRawData
        put_u64(&mut merged_rdata, tls_dir_off + 16, index_va); // AddressOfIndex
        put_u64(&mut merged_rdata, tls_dir_off + 24, 0);        // AddressOfCallBacks
        put_u32(&mut merged_rdata, tls_dir_off + 32, 0);        // SizeOfZeroFill
        put_u32(&mut merged_rdata, tls_dir_off + 36, 0);        // Characteristics
    }

    // Fill .refptr. slots in .data using the now-known RVAs.
    // Build refptr_offsets mapping ".refptr.X" → full RVA of the slot in .data.
    let mut refptr_rvas: HashMap<String, usize> = HashMap::new();
    for (i, name) in refptr_names.iter().enumerate() {
        let slot_rva = data_rva as usize + refptr_data_off + i * 8;
        refptr_rvas.insert(format!(".refptr.{name}"), slot_rva);
        if let Some((class, abs)) = global_syms.get(name) {
            let rva = match class {
                SectionClass::Text => text_rva as u64 + abs,
                SectionClass::Rodata => rdata_rva as u64 + abs,
                SectionClass::Data => data_rva as u64 + abs,
                SectionClass::Tls => tls_rva as u64 + abs,
            };
            let addr = PE_IMAGE_BASE + rva;
            merged_data[refptr_data_off + i * 8..][..8].copy_from_slice(&addr.to_le_bytes());
        }
    }

    // Build imports (refptrs no longer stored in .idata — pass empty slice)
    let import = build_imports(&raw_imports, &[], idata_rva)?;
    let has_idata = !import.data.is_empty();
    let idata_raw_size = if has_idata {
        pe_align(import.data.len(), PE_FILE_ALIGN)
    } else {
        0
    };

    // ── 5. Resolve relocations ───────────────────────────────────────────
    let mut abs_rvas: Vec<u32> = Vec::new();

    // Record .refptr. data slot RVAs for base relocations (ASLR)
    for (_, slot_rva) in &refptr_rvas {
        abs_rvas.push(*slot_rva as u32);
    }

    for (idx, obj) in objs.iter().enumerate() {
        let b = &bases[idx];
        for rel in &obj.relocations {
            // Use the section class recorded during COFF parse instead of
            // guessing from offset ranges (which fails when text_base ==
            // rdata_base == 0 for the first object).
            // Add the per-object section base to get the merged buffer offset.
            let (patch_buf, patch_rva) = match rel.section_class {
                SectionClass::Text => (&mut merged_text, text_rva),
                SectionClass::Rodata => (&mut merged_rdata, rdata_rva),
                SectionClass::Data => (&mut merged_data, data_rva),
                SectionClass::Tls => (&mut merged_tls, tls_rva),
            };
            let section_base = match rel.section_class {
                SectionClass::Text => b.text_base,
                SectionClass::Rodata => b.rdata_base,
                SectionClass::Data => b.data_base,
                SectionClass::Tls => b.tls_base,
            };
            let patch = section_base + rel.offset;
            let patch_rva_addr = patch_rva as i64 + patch as i64;

            // Watch for corruption of lpp_print_int byte 47
                    patch, patch + 8, rel.target, coff_reloc_number(&rel), rel.section_class);
            }

            // Resolve target
            let target = resolve_pe_target(
                &rel,
                &global_syms,
                &import.iat_rvas,
                &refptr_rvas,
                &bases[idx],
                text_rva,
                rdata_rva,
                data_rva,
                tls_rva,
                idata_rva,
            )?;

            let rnum = coff_reloc_number(rel);

            match rnum {
                AMD64_ADDR64 => {
                    if patch + 8 > patch_buf.len() {
                        return Err(format!("'{}': ADDR64 patch OOB", obj.path.display()));
                    }
                    let abs_addr = PE_IMAGE_BASE + target;
                    patch_buf[patch..patch + 8].copy_from_slice(&abs_addr.to_le_bytes());
                    abs_rvas.push(patch_rva_addr as u32);
                }
                AMD64_ADDR32 => {
                    if patch + 4 > patch_buf.len() {
                        return Err(format!("'{}': ADDR32 patch OOB", obj.path.display()));
                    }
                    let abs32 = (PE_IMAGE_BASE + target) as u32;
                    patch_buf[patch..patch + 4].copy_from_slice(&abs32.to_le_bytes());
                }
                AMD64_ADDR32NB => {
                    // ADDR32NB = address NOT based (image-relative RVA, no PE_IMAGE_BASE)
                    if patch + 4 > patch_buf.len() {
                        return Err(format!("'{}': ADDR32NB patch OOB", obj.path.display()));
                    }
                    let rva32 = target as u32;
                    patch_buf[patch..patch + 4].copy_from_slice(&rva32.to_le_bytes());
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
                    let disp = target as i64 + rel.addend - (patch_rva_addr + 4 + adjustment);
                    if disp < i32::MIN as i64 || disp > i32::MAX as i64 {
                        return Err(format!(
                            "'{}': REL32 displacement overflow ({disp})",
                            obj.path.display()
                        ));
                    }
                    patch_buf[patch..patch + 4].copy_from_slice(&(disp as i32).to_le_bytes());
                }
                AMD64_SECTION => {}
                AMD64_SECREL => {
                    if patch + 4 > patch_buf.len() {
                        return Err(format!("'{}': SECREL patch OOB", obj.path.display()));
                    }
                    // SECREL = offset of target from the beginning of target's section.
                    // resolve_pe_target returns the full RVA; subtract the section base.
                    // Determine which section the target is in by checking RVA ranges.
                    let secrel_val = if target < rdata_rva as u64 {
                        target - text_rva as u64   // target in .text
                    } else if target < data_rva as u64 {
                        target - rdata_rva as u64  // target in .rdata
                    } else if target < idata_rva as u64 {
                        target - data_rva as u64   // target in .data
                    } else {
                        target // fallback
                    };
                    patch_buf[patch..patch + 4].copy_from_slice(&(secrel_val as u32).to_le_bytes());
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

    // Generate base relocations table for ASLR
    let reloc_data = generate_base_relocs_from_rvas(&abs_rvas);
    let reloc_rva = if !reloc_data.is_empty() {
        pe_align(idata_rva as usize + import.data.len(), PE_SECT_ALIGN) as u32
    } else {
        0
    };
    let has_reloc = !reloc_data.is_empty();
    let reloc_raw_size = if has_reloc {
        pe_align(reloc_data.len(), PE_FILE_ALIGN)
    } else {
        0
    };

    // ── 6. (refptr data slots already filled above) ───────────────────────

    // ── 7. Compute raw file offsets and active section count ─────────────
    let has_text = !merged_text.is_empty();
    let has_rdata = !merged_rdata.is_empty();
    let has_data = !merged_data.is_empty() || !refptr_names.is_empty();

    let mut section_count: u16 = 0;
    if has_text { section_count += 1; }
    if has_rdata { section_count += 1; }
    if has_data { section_count += 1; }
    if has_tls { section_count += 1; }
    if has_idata { section_count += 1; }
    if has_reloc { section_count += 1; }

    let nt = 0x80;
    let opt = nt + 24;
    let opt_size: u16 = 0xF0;
    let required_headers_bytes = opt + opt_size as usize + (section_count as usize) * 40;
    let headers_size = pe_align(required_headers_bytes, PE_FILE_ALIGN);

    let text_raw_off = headers_size;
    let rdata_raw_off = text_raw_off + text_raw_size;
    let data_raw_off = rdata_raw_off + rdata_raw_size;
    let tls_raw_off = data_raw_off + data_raw_size;
    let idata_raw_off = tls_raw_off + tls_raw_size;
    let reloc_raw_off = idata_raw_off + idata_raw_size;

    let image_end = if has_reloc {
        reloc_rva as usize + reloc_data.len()
    } else if has_idata {
        idata_rva as usize + import.data.len()
    } else if has_tls {
        tls_rva as usize + merged_tls.len()
    } else if has_data {
        data_rva as usize + merged_data.len()
    } else if has_rdata {
        rdata_rva as usize + merged_rdata.len()
    } else {
        text_rva as usize + merged_text.len()
    };
    let image_size = pe_align(image_end, PE_SECT_ALIGN);
    let file_size = reloc_raw_off + reloc_raw_size;

    let mut pe = vec![0u8; file_size.max(headers_size)];

    // ── 8. DOS + PE headers ──────────────────────────────────────────────
    pe[0..2].copy_from_slice(b"MZ");
    put_u32(&mut pe, 0x3c, 0x80);
    let nt = 0x80;
    pe[nt..nt + 4].copy_from_slice(b"PE\0\0");
    put_u16(&mut pe, nt + 4, 0x8664); // x86-64
    put_u16(&mut pe, nt + 6, section_count);
    let opt_size: u16 = 0xF0;
    put_u16(&mut pe, nt + 20, opt_size);
    put_u16(&mut pe, nt + 22, 0x0022); // EXE, large-address-aware

    let opt = nt + 24;
    put_u16(&mut pe, opt, 0x20b); // PE32+
    put_u32(&mut pe, opt + 4, text_raw_size as u32);
    put_u32(
        &mut pe,
        opt + 8,
        (rdata_raw_size + data_raw_size + tls_raw_size + idata_raw_size) as u32,
    );

    // EntryPoint
    let main_entry = ["mainCRTStartup", "main", "_main", "WinMain", "lpp_main"]
        .iter()
        .find_map(|&name| global_syms.get(name))
        .ok_or_else(|| "required entry symbol ('mainCRTStartup', 'main', '_main', 'WinMain', or 'lpp_main') not found".to_string())?;

    let main_abs = match main_entry.0 {
        SectionClass::Text => text_rva as u64 + main_entry.1,
        SectionClass::Rodata => rdata_rva as u64 + main_entry.1,
        SectionClass::Data => data_rva as u64 + main_entry.1,
        SectionClass::Tls => tls_rva as u64 + main_entry.1,
    };
    put_u32(&mut pe, opt + 16, main_abs as u32);
    put_u32(&mut pe, opt + 20, text_rva);
    put_u64(&mut pe, opt + 24, PE_IMAGE_BASE);
    put_u32(&mut pe, opt + 32, PE_SECT_ALIGN as u32);
    put_u32(&mut pe, opt + 36, PE_FILE_ALIGN as u32);
    put_u16(&mut pe, opt + 40, 6);
    put_u16(&mut pe, opt + 48, 6);
    put_u32(&mut pe, opt + 56, image_size as u32);
    put_u32(&mut pe, opt + 60, headers_size as u32);
    put_u16(&mut pe, opt + 68, 3); // Console subsystem
    put_u16(&mut pe, opt + 70, 0x8160); // NX_COMPAT | DYNAMIC_BASE | HIGH_ENTROPY_VA | TERMINAL_SERVER_AWARE
    put_u64(&mut pe, opt + 72, 0x100000);
    put_u64(&mut pe, opt + 80, 0x1000);
    put_u64(&mut pe, opt + 88, 0x100000);
    put_u64(&mut pe, opt + 96, 0x1000);
    put_u32(&mut pe, opt + 108, 16);

    // Data directories
    let dirs = opt + 112;
    if has_idata {
        put_u32(&mut pe, dirs + 8, idata_rva); // Import directory (index 1)
        put_u32(&mut pe, dirs + 12, import.data.len() as u32);

        put_u32(&mut pe, dirs + 12 * 8, import.iat_rva); // IAT directory (index 12)
        put_u32(
            &mut pe,
            dirs + 12 * 8 + 4,
            import.iat_size,
        );
    }
    if has_reloc {
        put_u32(&mut pe, dirs + 5 * 8, reloc_rva); // Base relocation directory (index 5)
        put_u32(&mut pe, dirs + 5 * 8 + 4, reloc_data.len() as u32);
    }
    if has_tls {
        put_u32(&mut pe, dirs + 9 * 8, tls_dir_rva); // TLS directory (index 9)
        put_u32(&mut pe, dirs + 9 * 8 + 4, 40);
    }

    // ── 9. Section headers ───────────────────────────────────────────────
    let mut sec = opt + opt_size as usize;

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

    if has_text {
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
    }

    if has_rdata {
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

    if has_data {
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

    if has_tls {
        let mut tlsname = [0u8; 8];
        tlsname[..4].copy_from_slice(b".tls");
        emit_section(
            &mut pe,
            &mut sec,
            &tlsname,
            tls_rva,
            tls_raw_size,
            tls_raw_off,
            merged_tls.len(),
            0xC0000040, // RW | CNT_INITIALIZED_DATA | MEM_READ | MEM_WRITE
        );
    }

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
    if has_text {
        pe[text_raw_off..text_raw_off + merged_text.len()].copy_from_slice(&merged_text);
    }
    if has_rdata {
        pe[rdata_raw_off..rdata_raw_off + merged_rdata.len()].copy_from_slice(&merged_rdata);
    }
    if has_data {
        pe[data_raw_off..data_raw_off + merged_data.len()].copy_from_slice(&merged_data);
    }
    if has_tls {
        pe[tls_raw_off..tls_raw_off + merged_tls.len()].copy_from_slice(&merged_tls);
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
/// Resolve a PE relocation target to its RVA (NOT absolute address).  The
/// caller adds PE_IMAGE_BASE for absolute relocation types (ADDR64, ADDR32)
/// and uses the bare RVA for PC-relative computations (REL32).
fn resolve_pe_target(
    rel: &Relocation,
    global_syms: &HashMap<String, (SectionClass, u64)>,
    iat_rvas: &HashMap<String, u32>,
    refptr_offsets: &HashMap<String, usize>,
    bases: &SectionBase,
    text_rva: u32,
    rdata_rva: u32,
    data_rva: u32,
    tls_rva: u32,
    idata_rva: u32,
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
    if rel.target.starts_with("__self_tls__") {
        return Ok(tls_rva as u64 + bases.tls_base as u64);
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
    if let Some(rest) = rel.target.strip_prefix("__ext_tls__") {
        let ext_base: usize = rest
            .parse()
            .map_err(|_| format!("invalid __ext_tls__ tag: {}", rel.target))?;
        return Ok(tls_rva as u64 + ext_base as u64);
    }

    // .refptr. entry — resolve via the linker-managed slot in .data that
    // was filled with (PE_IMAGE_BASE + target_rva).  Do NOT use the original
    // .rdata$.refptr section from the COFF object (it contains zeros).
    if rel.target.starts_with(".refptr.") {
        if let Some(&rva) = refptr_offsets.get(&rel.target) {
            return Ok(rva as u64);
        }
        // Fallback: resolve the underlying symbol directly
        if let Some(name) = rel.target.strip_prefix(".refptr.") {
            if let Some((class, abs)) = global_syms.get(name) {
                let rva = match class {
                    SectionClass::Text => text_rva as u64 + abs,
                    SectionClass::Rodata => rdata_rva as u64 + abs,
                    SectionClass::Data => data_rva as u64 + abs,
                    SectionClass::Tls => tls_rva as u64 + abs,
                };
                return Ok(rva);
            }
        }
    }

    // Global symbol — compute RVA from section base + internal offset.
    // Check local definitions BEFORE IAT entries so that symbols defined
    // in the freestanding runtime (memcpy, memset, __chkstk, etc.) resolve
    // to their actual code rather than to IAT pointer data.  Only explicit
    // __imp_ prefixed references should go through the IAT.
    if !rel.target.starts_with("__imp_") {
        if let Some((class, abs)) = global_syms.get(&rel.target) {
            let rva = match class {
                SectionClass::Text => text_rva as u64 + abs,
                SectionClass::Rodata => rdata_rva as u64 + abs,
                SectionClass::Data => data_rva as u64 + abs,
                SectionClass::Tls => tls_rva as u64 + abs,
            };
            return Ok(rva);
        }
    }

    // IAT entry — returned as bare RVA (for __imp_ symbols and truly
    // external symbols not defined locally)
    if let Some(rva) = iat_rvas.get(&rel.target) {
        return Ok(*rva as u64);
    }

    // .refptr. fallback if not present in refptr_offsets table
    if let Some(name) = rel.target.strip_prefix(".refptr.") {
        if let Some((class, abs)) = global_syms.get(name) {
            let rva = match class {
                SectionClass::Text => text_rva as u64 + abs,
                SectionClass::Rodata => rdata_rva as u64 + abs,
                SectionClass::Data => data_rva as u64 + abs,
                SectionClass::Tls => tls_rva as u64 + abs,
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
            section_class: SectionClass::Text,
                coff_type: 0,
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

fn expand_response_files(args: Vec<String>) -> Result<Vec<String>, String> {
    let mut expanded = Vec::new();
    for arg in args {
        if let Some(rsp_path) = arg.strip_prefix('@') {
            let content = fs::read_to_string(rsp_path)
                .map_err(|e| format!("failed to read response file '@{}': {}", rsp_path, e))?;
            for line in content.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() && !trimmed.starts_with('#') {
                    for token in trimmed.split_whitespace() {
                        expanded.push(token.to_string());
                    }
                }
            }
        } else {
            expanded.push(arg);
        }
    }
    Ok(expanded)
}

fn main() {
    let raw_args: Vec<String> = env::args().skip(1).collect();
    let args = match expand_response_files(raw_args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("lpp-link error: {e}");
            std::process::exit(1);
        }
    };
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
    for path in &inputs {
        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            let ext_lower = ext.to_lowercase();
            if ext_lower == "c" || ext_lower == "cpp" || ext_lower == "cc" || ext_lower == "lpp" {
                eprintln!(
                    "lpp-link error: Input file '{}' is a source code file. lpp-link requires compiled binary object files (.obj or .o).\nPlease compile the runtime source into a COFF object file first (e.g. 'cl /c /DLPP_FREESTANDING {}' or 'gcc -c -DLPP_FREESTANDING {} -o {}.obj').",
                    path.display(),
                    path.display(),
                    path.display(),
                    path.file_stem().unwrap_or_default().to_string_lossy()
                );
                std::process::exit(1);
            }
        }
    }
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
