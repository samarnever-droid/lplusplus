//! `linker` — in-process direct linker module for Linux ELF, Windows PE, and macOS Mach-O.
//!
//! Exposes direct PE/ELF/Mach-O binary creation directly inside `lpp` compiler
//! binaries without needing subprocess spawns or external `lpp-link.exe` binaries.

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
    section_class: SectionClass,
    coff_type: u16,
}

struct CoffSections {
    path: PathBuf,
    text: Vec<u8>,
    rdata: Vec<u8>,
    data: Vec<u8>,
    tls: Vec<u8>,
    #[allow(dead_code)]
    section_map: Vec<(object::SectionIndex, SectionClass, usize)>,
    symbols: Vec<(String, SectionClass, u64)>,
    relocations: Vec<Relocation>,
}

struct ElfInput {
    path: PathBuf,
    text: Vec<u8>,
    rodata: Vec<u8>,
    text_symbols: Vec<(String, u64)>,
    rodata_symbols: Vec<(String, u64)>,
    data: Vec<u8>,
    data_symbols: Vec<(String, u64)>,
    relocations: Vec<Relocation>,
}

struct MachoInput {
    path: PathBuf,
    text: Vec<u8>,
    text_symbols: Vec<(String, u64)>,
    relocations: Vec<Relocation>,
}

// ═══════════════════════════════════════════════════════════════════════════
//  1.  ELF path
// ═══════════════════════════════════════════════════════════════════════════

const ELF_BASE: u64 = 0x400000;
const CODE_OFFSET: usize = 0x1000;
const EM_X86_64: u16 = 62;
const PT_LOAD: u32 = 1;
const _PF_R_X: u32 = 5;
const PF_R_W_X: u32 = 7;

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
    let mut data_map: HashMap<object::SectionIndex, usize> = HashMap::new();
    let mut data_image = Vec::new();
    for sec in file.sections() {
        if let Ok(name) = sec.name() {
            let is_data = name == ".data" || name.starts_with(".data.");
            let is_bss = name == ".bss" || name.starts_with(".bss.");
            if is_data || is_bss {
                let align = usize::try_from(sec.align()).unwrap_or(16).max(1);
                let base = align_up(data_image.len(), align);
                data_image.resize(base, 0);
                data_map.insert(sec.index(), base);
                if is_bss {
                    let n = usize::try_from(sec.size()).unwrap_or(0);
                    data_image.resize(base + n, 0);
                } else if let Ok(d) = sec.uncompressed_data() {
                    data_image.extend_from_slice(&d);
                }
            }
        }
    }

    let is_rodata = |s: SymbolSection| match s {
        SymbolSection::Section(i) => rodata_map.contains_key(&i),
        _ => false,
    };
    let is_data_sec = |s: SymbolSection| match s {
        SymbolSection::Section(i) => data_map.contains_key(&i),
        _ => false,
    };

    let mut text_syms = Vec::new();
    let mut rodata_syms = Vec::new();
    let mut data_syms = Vec::new();
    for sym in file.symbols() {
        let dst = if sym.section() == SymbolSection::Section(text_idx) {
            Some(&mut text_syms)
        } else if is_rodata(sym.section()) {
            Some(&mut rodata_syms)
        } else if is_data_sec(sym.section()) {
            Some(&mut data_syms)
        } else {
            None
        };
        if let Some(dst) = dst {
            if let Ok(n) = sym.name() {
                if !n.is_empty() {
                    let sec_base = match sym.section() {
                        SymbolSection::Section(i) => rodata_map
                            .get(&i)
                            .or_else(|| data_map.get(&i))
                            .copied()
                            .unwrap_or(0),
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
            SymbolSection::Section(i) if data_map.contains_key(&i) => {
                let sec_base = data_map[&i] as i64;
                (
                    "__self_data__".to_string(),
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
        data: data_image,
        data_symbols: data_syms,
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

pub fn write_elf(inputs: &[PathBuf], output: &Path) -> Result<(), String> {
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
    let entry = *entry;

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
        0x89, 0xc7, 0xb8, 60, 0, 0, 0, 0x0f, 0x05, 0xeb, 0xfe, // exit + jmp .
    ];
    start[7..11].copy_from_slice(&(disp as i32).to_le_bytes());
    text.extend_from_slice(&start);

    let mut got: HashMap<(String, u64, usize), usize> = HashMap::new();
    for (idx, inp) in objs.iter().enumerate() {
        for rel in &inp.relocations {
            if rel.kind == RelocationKind::GotRelative {
                let sym_offset = (rel.addend + 4) as u64;
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
    let mut data_bases = Vec::new();
    let mut data_off = align_up(text.len(), 16);
    text.resize(data_off, 0);
    for inp in &objs {
        let base = data_off;
        data_bases.push(base);
        for (n, o) in &inp.data_symbols {
            let abs = u64::try_from(base).map_err(|_| "data offset overflow")? + o;
            if syms.insert(n.clone(), abs).is_some() {
                return Err(format!("duplicate definition of symbol '{n}'"));
            }
        }
        text.extend_from_slice(&inp.data);
        data_off = align_up(text.len(), 16);
        text.resize(data_off, 0);
    }

    for (key, &idx) in &got {
        let (target, sym_offset, inp_idx) = key;
        let target_addr = if target == "__self_rodata__" {
            rodata_bases[*inp_idx] as u64 + sym_offset
        } else if target == "__self_data__" {
            data_bases[*inp_idx] as u64 + sym_offset
        } else if target == "__self_text__" {
            bases[*inp_idx] as u64 + sym_offset
        } else {
            *syms.get(target).ok_or_else(|| {
                format!(
                    "'{}': unresolved GOT symbol '{target}'",
                    objs[*inp_idx].path.display()
                )
            })? + sym_offset
        };
        let val = ELF_BASE + CODE_OFFSET as u64 + target_addr;
        let pos = got_off + idx * 8;
        text[pos..pos + 8].copy_from_slice(&val.to_le_bytes());
    }

    for (idx, inp) in objs.iter().enumerate() {
        let base = bases[idx];
        let rodata_base = rodata_bases[idx];
        let data_base = data_bases[idx];
        for rel in &inp.relocations {
            let (target_off, is_got) = if rel.target == "__self_text__" {
                (base as u64, false)
            } else if rel.target == "__self_rodata__" {
                (rodata_base as u64, false)
            } else if rel.target == "__self_data__" {
                (data_base as u64, false)
            } else if rel.kind == RelocationKind::GotRelative {
                let sym_offset = (rel.addend + 4) as u64;
                let idx = got[&(rel.target.clone(), sym_offset, idx)];
                (got_off as u64 + idx as u64 * 8, true)
            } else {
                let addr = *syms.get(&rel.target).ok_or_else(|| {
                    format!(
                        "'{}': unresolved external symbol '{}'",
                        inp.path.display(),
                        rel.target
                    )
                })?;
                (addr, false)
            };
            let patch = match rel.section_class {
                SectionClass::Text => base,
                SectionClass::Rodata => rodata_base,
                SectionClass::Data | SectionClass::Tls => data_base,
            } + rel.offset;
            let patch_len = if rel.kind == RelocationKind::Absolute && rel.size == 64 { 8 } else { 4 };
            if patch + patch_len > text.len() {
                return Err(format!(
                    "'{}': relocation patch out of range",
                    inp.path.display()
                ));
            }
            let addend = if is_got {
                -4i64
            } else {
                rel.addend
            };
            if rel.kind == RelocationKind::Absolute {
                let abs = (ELF_BASE + CODE_OFFSET as u64 + target_off).wrapping_add_signed(addend);
                if patch_len == 8 {
                    text[patch..patch + 8].copy_from_slice(&abs.to_le_bytes());
                } else {
                    text[patch..patch + 4].copy_from_slice(&(abs as u32).to_le_bytes());
                }
            } else {
                let disp = target_off as i64 + addend - patch as i64;
                if disp < i32::MIN as i64 || disp > i32::MAX as i64 {
                    return Err(format!(
                        "'{}': PC-relative relocation out of range",
                        inp.path.display()
                    ));
                }
                text[patch..patch + 4].copy_from_slice(&(disp as i32).to_le_bytes());
            }
        }
    }

    let start_entry = ELF_BASE + CODE_OFFSET as u64 + start_off as u64;
    let fsize = CODE_OFFSET + text.len();
    let mut elf = vec![0u8; fsize];
    elf[0..4].copy_from_slice(b"\x7fELF");
    elf[4] = 2; // 64-bit
    elf[5] = 1; // Little endian
    elf[6] = 1; // ELF version
    put_u16(&mut elf, 16, 2); // ET_EXEC
    put_u16(&mut elf, 18, EM_X86_64);
    put_u32(&mut elf, 20, 1);
    put_u64(&mut elf, 24, start_entry);
    put_u64(&mut elf, 32, 64);
    put_u16(&mut elf, 52, 64);
    put_u16(&mut elf, 54, 56);
    put_u16(&mut elf, 56, 1);
    let ph = 64;
    put_u32(&mut elf, ph, PT_LOAD);
    put_u32(&mut elf, ph + 4, PF_R_W_X);
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
//  2.  Windows PE path
// ═══════════════════════════════════════════════════════════════════════════

const PE_IMAGE_BASE: u64 = 0x140000000;
const PE_SECTION_RVA: u32 = 0x1000;
const PE_FILE_ALIGN: usize = 0x200;
const PE_SECT_ALIGN: usize = 0x1000;

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

fn coff_reloc_number(rel: &Relocation) -> u8 {
    if rel.coff_type != 0 {
        return rel.coff_type as u8;
    }
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

        if name.starts_with(".debug")
            || name.starts_with(".drectve")
            || name.starts_with(".comment")
            || name.starts_with(".note")
            || name.starts_with(".xdata")
            || name.starts_with(".pdata")
        {
            continue;
        }

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
            | "abs"
            | "labs"
            | "llabs"
            | "getpid"
            | "_getpid"
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
            | "lpp_c_malloc"
            | "lpp_c_free"
            | "lpp_c_load_u8"
            | "lpp_c_store_u8"
            | "lpp_c_load_i32"
            | "lpp_c_store_i32"
            | "lpp_c_load_i64"
            | "lpp_c_store_i64"
            | "dlopen"
            | "dlsym"
            | "dlclose"
            | "dlerror"
    )
}

fn is_kernel32_symbol(name: &str) -> bool {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    matches!(
        clean,
        "ExitProcess"
            | "GetTickCount64"
            | "LoadLibraryA"
            | "GetProcAddress"
            | "GetStdHandle"
            | "WriteFile"
            | "ReadFile"
            | "VirtualAlloc"
            | "VirtualFree"
            | "CreateThread"
            | "WaitForSingleObject"
            | "CloseHandle"
            | "CreateFileA"
            | "GetFileSize"
            | "SetFilePointer"
            | "DeleteFileA"
            | "MoveFileA"
            | "GetFileAttributesA"
            | "CreateDirectoryA"
            | "RemoveDirectoryA"
            | "FindFirstFileA"
            | "FindNextFileA"
            | "FindClose"
            | "Sleep"
            | "CreateProcessA"
            | "GetExitCodeProcess"
            | "CreatePipe"
            | "GetEnvironmentVariableA"
            | "SetEnvironmentVariableA"
            | "GetModuleFileNameA"
            | "GetModuleHandleA"
            | "GetLastError"
            | "QueryPerformanceCounter"
            | "QueryPerformanceFrequency"
    )
}

fn is_user32_symbol(name: &str) -> bool {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    matches!(
        clean,
        "CreateWindowExA"
            | "DestroyWindow"
            | "DefWindowProcA"
            | "PostQuitMessage"
            | "RegisterClassA"
            | "GetDC"
            | "ReleaseDC"
            | "LoadCursorA"
            | "PeekMessageA"
            | "TranslateMessage"
            | "DispatchMessageA"
            | "GetAsyncKeyState"
            | "GetCursorPos"
            | "ScreenToClient"
            | "FillRect"
            | "ShowWindow"
            | "UpdateWindow"
            | "SetForegroundWindow"
            | "MessageBoxA"
            | "LoadIconA"
            | "SetWindowPos"
            | "BringWindowToTop"
            | "BeginPaint"
            | "EndPaint"
            | "SetProcessDPIAware"
            | "AdjustWindowRectEx"
    )
}

fn is_gdi32_symbol(name: &str) -> bool {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    matches!(
        clean,
        "CreateCompatibleDC"
            | "CreateCompatibleBitmap"
            | "SelectObject"
            | "DeleteDC"
            | "DeleteObject"
            | "CreateSolidBrush"
            | "CreatePen"
            | "RoundRect"
            | "TextOutA"
            | "SetBkMode"
            | "SetTextColor"
            | "BitBlt"
            | "Ellipse"
            | "MoveToEx"
            | "LineTo"
            | "GetTextExtentPoint32A"
            | "CreateFontA"
            | "SetStretchBltMode"
            | "SetBrushOrgEx"
            | "SetMapMode"
            | "SetGraphicsMode"
            | "SetTextCharacterExtra"
            | "SetTextAlign"
            | "SetLayout"
            | "GetStockObject"
    )
}

fn is_ws2_32_symbol(name: &str) -> bool {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    matches!(
        clean,
        "WSAStartup"
            | "WSACleanup"
            | "WSAGetLastError"
            | "WSAIoctl"
            | "WSASocketA"
            | "WSARecv"
            | "WSASend"
            | "socket"
            | "bind"
            | "listen"
            | "accept"
            | "connect"
            | "send"
            | "recv"
            | "sendto"
            | "recvfrom"
            | "closesocket"
            | "shutdown"
            | "select"
            | "htons"
            | "htonl"
            | "ntohs"
            | "ntohl"
            | "getaddrinfo"
            | "freeaddrinfo"
            | "gethostname"
            | "getsockname"
            | "getpeername"
            | "setsockopt"
            | "getsockopt"
            | "ioctlsocket"
            | "inet_ntoa"
            | "inet_addr"
    )
}

struct ImportData {
    data: Vec<u8>,
    iat_rvas: HashMap<String, u32>,
    _refptr_offsets: HashMap<String, usize>,
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
    let mut user32_imports = Vec::new();
    let mut gdi32_imports = Vec::new();
    let mut ws2_32_imports = Vec::new();
    let mut crt_imports = Vec::new();

    for imp in raw_imports {
        let clean = imp.strip_prefix("__imp_").unwrap_or(imp).to_string();
        if is_user32_symbol(&clean) {
            if !user32_imports.contains(&clean) {
                user32_imports.push(clean);
            }
        } else if is_gdi32_symbol(&clean) {
            if !gdi32_imports.contains(&clean) {
                gdi32_imports.push(clean);
            }
        } else if is_ws2_32_symbol(&clean) {
            if !ws2_32_imports.contains(&clean) {
                ws2_32_imports.push(clean);
            }
        } else if is_crt_symbol(&clean) {
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
        ("USER32.dll", &user32_imports),
        ("GDI32.dll", &gdi32_imports),
        ("WS2_32.dll", &ws2_32_imports),
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
        total_ilt_iat_entries += funcs.len() + 1;
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
                data.extend_from_slice(&[0u8, 0u8]);
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
        _refptr_offsets: refptr_offsets,
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
            let entry = 0xA000u16 | offset_in_page;
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

pub fn write_pe(inputs: &[PathBuf], output: &Path) -> Result<(), String> {
    if inputs.is_empty() {
        return Err("at least one input object is required".to_string());
    }

    let mut objs: Vec<CoffSections> = Vec::new();
    for p in inputs {
        load_coff_inputs(p, &mut objs)?;
    }

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

    let mut raw_imports: Vec<String> = Vec::new();
    let mut refptr_names: Vec<String> = Vec::new();

    for obj in &objs {
        for rel in &obj.relocations {
            if let Some(name) = rel.target.strip_prefix("__imp_") {
                let n = name.to_string();
                if !raw_imports.contains(&n) {
                    raw_imports.push(n);
                }
            } else if (is_kernel32_symbol(&rel.target) || is_user32_symbol(&rel.target) || is_gdi32_symbol(&rel.target) || is_ws2_32_symbol(&rel.target)) && !global_syms.contains_key(&rel.target) {
                if !raw_imports.contains(&rel.target) {
                    raw_imports.push(rel.target.clone());
                }
            } else if is_crt_symbol(&rel.target) && !global_syms.contains_key(&rel.target) {
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

    let refptr_data_off = merged_data.len();
    merged_data.resize(refptr_data_off + refptr_names.len() * 8, 0);

    let text_rva = PE_SECTION_RVA;
    let thunks_len = raw_imports.len() * 6;
    let total_text_len = merged_text.len() + thunks_len;

    let rdata_rva = pe_align(text_rva as usize + total_text_len, PE_SECT_ALIGN) as u32;
    let data_rva = pe_align(rdata_rva as usize + merged_rdata.len(), PE_SECT_ALIGN) as u32;

    let has_tls = !merged_tls.is_empty();
    let tls_rva = pe_align(data_rva as usize + merged_data.len(), PE_SECT_ALIGN) as u32;

    let idata_rva = pe_align(
        if has_tls {
            tls_rva as usize + merged_tls.len()
        } else {
            data_rva as usize + merged_data.len()
        },
        PE_SECT_ALIGN,
    ) as u32;

    let import = build_imports(&raw_imports, &[], idata_rva)?;

    // Generate 6-byte import thunks in .text for direct call relocations to DLL imports
    let mut thunk_rvas: HashMap<String, u32> = HashMap::new();
    for imp in &raw_imports {
        if let Some(&iat_entry_rva) = import.iat_rvas.get(imp) {
            let thunk_rva = text_rva + merged_text.len() as u32;
            let next_ip = thunk_rva + 6;
            let disp = iat_entry_rva as i64 - next_ip as i64;
            let mut thunk_bytes = [0xFFu8, 0x25, 0, 0, 0, 0];
            thunk_bytes[2..6].copy_from_slice(&(disp as i32).to_le_bytes());
            merged_text.extend_from_slice(&thunk_bytes);
            thunk_rvas.insert(imp.clone(), thunk_rva);
        }
    }

    let text_raw_size = pe_align(merged_text.len(), PE_FILE_ALIGN);
    let rdata_raw_size = pe_align(merged_rdata.len(), PE_FILE_ALIGN);
    let data_raw_size = pe_align(merged_data.len(), PE_FILE_ALIGN);
    let tls_raw_size = if has_tls {
        pe_align(merged_tls.len(), PE_FILE_ALIGN)
    } else {
        0
    };
    let has_idata = !import.data.is_empty();
    let idata_raw_size = if has_idata {
        pe_align(import.data.len(), PE_FILE_ALIGN)
    } else {
        0
    };

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

        put_u64(&mut merged_rdata, tls_dir_off, start_va);
        put_u64(&mut merged_rdata, tls_dir_off + 8, end_va);
        put_u64(&mut merged_rdata, tls_dir_off + 16, index_va);
        put_u64(&mut merged_rdata, tls_dir_off + 24, 0);
        put_u32(&mut merged_rdata, tls_dir_off + 32, 0);
        put_u32(&mut merged_rdata, tls_dir_off + 36, 0);
    }

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

    let mut abs_rvas: Vec<u32> = Vec::new();

    for (_, slot_rva) in &refptr_rvas {
        abs_rvas.push(*slot_rva as u32);
    }

    for (idx, obj) in objs.iter().enumerate() {
        let b = &bases[idx];
        for rel in &obj.relocations {
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

            let target = resolve_pe_target(
                &rel,
                &global_syms,
                &import.iat_rvas,
                &thunk_rvas,
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
                    let secrel_val = if target < rdata_rva as u64 {
                        target - text_rva as u64
                    } else if target < data_rva as u64 {
                        target - rdata_rva as u64
                    } else if target < idata_rva as u64 {
                        target - data_rva as u64
                    } else {
                        target
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

    pe[0..2].copy_from_slice(b"MZ");
    put_u32(&mut pe, 0x3c, 0x80);
    let nt = 0x80;
    pe[nt..nt + 4].copy_from_slice(b"PE\0\0");
    put_u16(&mut pe, nt + 4, 0x8664);
    put_u16(&mut pe, nt + 6, section_count);
    let opt_size: u16 = 0xF0;
    put_u16(&mut pe, nt + 20, opt_size);
    put_u16(&mut pe, nt + 22, 0x0022);

    let opt = nt + 24;
    put_u16(&mut pe, opt, 0x20b);
    put_u32(&mut pe, opt + 4, text_raw_size as u32);
    put_u32(
        &mut pe,
        opt + 8,
        (rdata_raw_size + data_raw_size + tls_raw_size + idata_raw_size) as u32,
    );

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
    put_u16(&mut pe, opt + 70, 0x8160);
    put_u64(&mut pe, opt + 72, 0x100000);
    put_u64(&mut pe, opt + 80, 0x1000);
    put_u64(&mut pe, opt + 88, 0x100000);
    put_u64(&mut pe, opt + 96, 0x1000);
    put_u32(&mut pe, opt + 108, 16);

    let dirs = opt + 112;
    if has_idata {
        put_u32(&mut pe, dirs + 8, idata_rva);
        put_u32(&mut pe, dirs + 12, import.data.len() as u32);

        put_u32(&mut pe, dirs + 12 * 8, import.iat_rva);
        put_u32(
            &mut pe,
            dirs + 12 * 8 + 4,
            import.iat_size,
        );
    }
    if has_reloc {
        put_u32(&mut pe, dirs + 5 * 8, reloc_rva);
        put_u32(&mut pe, dirs + 5 * 8 + 4, reloc_data.len() as u32);
    }
    if has_tls {
        put_u32(&mut pe, dirs + 9 * 8, tls_dir_rva);
        put_u32(&mut pe, dirs + 9 * 8 + 4, 40);
    }

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
            0x60000020,
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
            0x40000040,
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
            0xC0000040,
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
            0xC0000040,
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
            0xC0000040,
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
            0x42000040,
        );
    }

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

fn resolve_pe_target(
    rel: &Relocation,
    global_syms: &HashMap<String, (SectionClass, u64)>,
    iat_rvas: &HashMap<String, u32>,
    thunk_rvas: &HashMap<String, u32>,
    refptr_offsets: &HashMap<String, usize>,
    bases: &SectionBase,
    text_rva: u32,
    rdata_rva: u32,
    data_rva: u32,
    tls_rva: u32,
    _idata_rva: u32,
) -> Result<u64, String> {
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

    if rel.target.starts_with(".refptr.") {
        if let Some(&rva) = refptr_offsets.get(&rel.target) {
            return Ok(rva as u64);
        }
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
        // Direct call to an imported DLL function — resolve to the import thunk in .text
        if let Some(&trva) = thunk_rvas.get(&rel.target) {
            return Ok(trva as u64);
        }
    }

    if let Some(rva) = iat_rvas.get(&rel.target) {
        return Ok(*rva as u64);
    }

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

    if let Some(rest) = rel.target.strip_prefix("__coff_text_section_") {
        let off: u32 = rest
            .parse()
            .map_err(|_| format!("invalid __coff_text_section_ tag"))?;
        return Ok(text_rva as u64 + off as u64);
    }

    if rel.target == "__ImageBase" {
        return Ok(0u64);
    }

    Err(format!(
        "unresolved external COFF symbol '{}' — not defined by any input object and not a known DLL import. \
         Link with the host linker (LPP_LINKER=host or --linker host) for full C library support.",
        rel.target
    ))
}

// ═══════════════════════════════════════════════════════════════════════════
//  3.  Mach-O path
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

pub fn write_macho(inputs: &[PathBuf], output: &Path) -> Result<(), String> {
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
    header.extend_from_slice(&3u32.to_le_bytes());
    let sizeofcmds = (72 + 152 + 24) as u32;
    header.extend_from_slice(&sizeofcmds.to_le_bytes());
    header.extend_from_slice(&0x00200085u32.to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes());

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

    header.extend_from_slice(&0x80000028u32.to_le_bytes());
    header.extend_from_slice(&24u32.to_le_bytes());
    header.extend_from_slice(&(4096 + main).to_le_bytes());
    header.extend_from_slice(&0u64.to_le_bytes());

    let mut bin = vec![0u8; 4096];
    bin[..header.len()].copy_from_slice(&header);
    bin.extend_from_slice(&text);
    fs::write(output, bin)
        .map_err(|e| format!("write Mach-O binary '{}': {e}", output.display()))?;
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
//  4.  inspect
// ═══════════════════════════════════════════════════════════════════════════

pub fn inspect_object(input: &Path) -> Result<(), String> {
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
//  5. High-level entrypoints and CLI runner
// ═══════════════════════════════════════════════════════════════════════════

pub fn usage() {
    eprintln!("lpp-link {} — L++ direct native linker", env!("CARGO_PKG_VERSION"));
    eprintln!();
    eprintln!("Usage: lpp-link <program.o> [runtime.o ...] -o <output>");
    eprintln!("       lpp-link pe <program.obj> [runtime.obj ...] -o <output.exe>");
    eprintln!("       lpp-link macho <program.o> [runtime.o ...] -o <output>");
    eprintln!("       lpp-link inspect <object.o>");
    eprintln!();
    eprintln!("Without an explicit 'pe'/'macho' mode the output format is detected");
    eprintln!("from the first input object (ELF / COFF / Mach-O).");
    eprintln!("Arguments may also be passed through a response file: lpp-link @args.rsp");
    eprintln!();
    eprintln!("Modes: direct Linux x86-64 ELF linker; Windows PE COFF linker; macOS Mach-O direct emitter.");
}

pub fn sniff_format(path: &Path) -> &'static str {
    let Ok(bytes) = fs::read(path) else {
        return "elf";
    };
    if bytes.len() >= 4 {
        if &bytes[0..4] == b"\x7fELF" {
            return "elf";
        }
        let be = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let le = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        const MACHO_MAGICS: [u32; 4] = [0xFEEDFACE, 0xFEEDFACF, 0xCAFEBABE, 0xCAFED00D];
        if MACHO_MAGICS.contains(&be) || MACHO_MAGICS.contains(&le) {
            return "macho";
        }
        let machine = u16::from_le_bytes([bytes[0], bytes[1]]);
        if machine == 0x8664 {
            return "pe";
        }
    }
    "elf"
}

pub fn link_direct(inputs: &[PathBuf], output: &Path) -> Result<(), String> {
    if inputs.is_empty() {
        return Err("at least one input object is required".to_string());
    }
    match inputs.first().map(|p| sniff_format(p)).unwrap_or("elf") {
        "pe" => write_pe(inputs, output),
        "macho" => write_macho(inputs, output),
        _ => write_elf(inputs, output),
    }
}

pub fn expand_response_files(args: Vec<String>) -> Result<Vec<String>, String> {
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

pub fn link_cli(args: &[String]) -> Result<(), String> {
    let args = expand_response_files(args.to_vec())?;
    if args.first().map(String::as_str) == Some("inspect") {
        if args.len() != 2 {
            usage();
            return Err("inspect requires exactly one object file argument".to_string());
        }
        return inspect_object(Path::new(&args[1]));
    }
    match args.first().map(String::as_str) {
        Some("-h") | Some("--help") | Some("help") => {
            usage();
            return Ok(());
        }
        Some("-V") | Some("--version") | Some("version") => {
            println!("lpp-link {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        _ => {}
    }
    let pe_mode = args.first().map(String::as_str) == Some("pe");
    let macho_mode = args.first().map(String::as_str) == Some("macho");
    let offset = if pe_mode || macho_mode { 1 } else { 0 };
    let Some(output_rel) = args[offset..].iter().position(|a| a == "-o") else {
        usage();
        return Err("missing '-o <output>' argument".to_string());
    };
    let out_idx = offset + output_rel;
    if out_idx == offset || out_idx + 2 != args.len() {
        usage();
        return Err("invalid output specification".to_string());
    }
    let inputs: Vec<PathBuf> = args[offset..out_idx].iter().map(PathBuf::from).collect();
    for path in &inputs {
        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            let ext_lower = ext.to_lowercase();
            if ext_lower == "c" || ext_lower == "cpp" || ext_lower == "cc" || ext_lower == "lpp" {
                return Err(format!(
                    "Input file '{}' is a source code file. lpp-link requires compiled binary object files (.obj or .o).\nPlease compile the runtime source into a COFF object file first.",
                    path.display()
                ));
            }
        }
    }
    if pe_mode {
        write_pe(&inputs, Path::new(&args[out_idx + 1]))
    } else if macho_mode {
        write_macho(&inputs, Path::new(&args[out_idx + 1]))
    } else {
        link_direct(&inputs, Path::new(&args[out_idx + 1]))
    }
}
