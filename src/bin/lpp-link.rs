//! `lpp-link` — direct linker for Linux ELF, Windows PE, and macOS Mach-O.
//!
//! Phase 3+: auto-detects target format from the host OS.  Explicit `pe` /
//! `macho` subcommands preserved.  ELF with GOT/rodata merge, PE with
//! multi-section layout (.text/.rdata/.data/.idata/.reloc), base relocations,
//! and full AMD64 relocation coverage.  Mach-O direct emitter.
//!
//! The linker grows in small verified slices — each format gets exactly the
//! section and relocation support it needs for the verified workload set.

use object::{
    Architecture, BinaryFormat, Object, ObjectSection, ObjectSymbol,
    RelocationKind, RelocationTarget, SymbolSection,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat { Elf, PE, MachO }

impl OutputFormat {
    fn for_host() -> Self {
        if cfg!(target_os = "windows") { OutputFormat::PE }
        else if cfg!(target_os = "macos") { OutputFormat::MachO }
        else { OutputFormat::Elf }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SectionClass { Text, Rodata, Data }

struct Relocation {
    offset: usize, target: String, addend: i64, size: u8, kind: RelocationKind,
}

struct CoffSections {
    #[allow(dead_code)] path: PathBuf, text: Vec<u8>, rdata: Vec<u8>, data: Vec<u8>,
    #[allow(dead_code)] section_map: Vec<(object::SectionIndex, SectionClass, usize)>,
    symbols: Vec<(String, SectionClass, u64)>, relocations: Vec<Relocation>,
}

struct ElfInput {
    path: PathBuf, text: Vec<u8>, rodata: Vec<u8>,
    text_symbols: Vec<(String, u64)>, rodata_symbols: Vec<(String, u64)>,
    relocations: Vec<Relocation>,
}

struct MachoInput {
    #[allow(dead_code)] path: PathBuf, text: Vec<u8>,
    text_symbols: Vec<(String, u64)>, relocations: Vec<Relocation>,
}

// ═══════════════════════════════════════════════════════════════════════════
//  1.  ELF path
// ═══════════════════════════════════════════════════════════════════════════

const ELF_BASE: u64 = 0x400000;
const CODE_OFFSET: usize = 0x1000;
const EM_X86_64: u16 = 62;
const PT_LOAD: u32 = 1;
const PF_R_X: u32 = 5;

fn read_elf_input(path: &Path) -> Result<ElfInput, String> {
    let bytes = fs::read(path).map_err(|e| format!("read '{}': {e}", path.display()))?;
    let file = object::File::parse(&*bytes)
        .map_err(|e| format!("parse '{}': {e}", path.display()))?;
    if file.format() != BinaryFormat::Elf || file.architecture() != Architecture::X86_64 {
        return Err(format!("'{}' is not an x86-64 ELF relocatable object", path.display()));
    }
    let text_sec = file.section_by_name(".text")
        .ok_or_else(|| format!("'{}' has no .text section", path.display()))?;
    let text_idx = text_sec.index();
    let text = text_sec.uncompressed_data()
        .map_err(|e| format!("read .text from '{}': {e}", path.display()))?.into_owned();

    let mut rodata_idxs = HashSet::new();
    let mut rodata = Vec::new();
    for sec in file.sections() {
        if let Ok(name) = sec.name() {
            if name == ".rodata" || name.starts_with(".rodata.") {
                rodata_idxs.insert(sec.index());
                if let Ok(d) = sec.uncompressed_data() { rodata.extend_from_slice(&d); }
            }
        }
    }
    let is_rodata = |s: SymbolSection| match s { SymbolSection::Section(i) => rodata_idxs.contains(&i), _ => false };

    let mut text_syms = Vec::new(); let mut rodata_syms = Vec::new();
    for sym in file.symbols() {
        let dst = if sym.section() == SymbolSection::Section(text_idx) { Some(&mut text_syms) }
            else if is_rodata(sym.section()) { Some(&mut rodata_syms) } else { None };
        if let Some(dst) = dst {
            if let Ok(n) = sym.name() { if !n.is_empty() { dst.push((n.to_string(), sym.address())); } }
        }
    }

    let mut relocs = Vec::new();
    for (off, rel) in text_sec.relocations() {
        let RelocationTarget::Symbol(si) = rel.target() else {
            return Err(format!("'{}' has unsupported non-symbol relocation", path.display()));
        };
        let sym = file.symbol_by_index(si).map_err(|e| format!("read relocation symbol: {e}"))?;
        let raw = sym.name().map_err(|e| format!("read relocation symbol name: {e}"))?;
        let is_section = raw.is_empty() || sym.kind() == object::SymbolKind::Section
            || raw.starts_with(".rodata") || raw.starts_with(".text");
        let target = if is_section && sym.section() == SymbolSection::Section(text_idx) {
            "__self_text__".to_string()
        } else if is_section && is_rodata(sym.section()) {
            "__self_rodata__".to_string()
        } else { raw.to_string() };
        relocs.push(Relocation {
            offset: usize::try_from(off).map_err(|_| "relocation offset overflow")?,
            target, addend: rel.addend(), size: rel.size(), kind: rel.kind(),
        });
    }
    Ok(ElfInput { path: path.to_path_buf(), text, rodata, text_symbols: text_syms, rodata_symbols: rodata_syms, relocations: relocs })
}

fn write_elf(inputs: &[PathBuf], output: &Path) -> Result<(), String> {
    if inputs.is_empty() { return Err("at least one input object is required".to_string()); }
    let objs: Vec<ElfInput> = inputs.iter().map(|p| read_elf_input(p)).collect::<Result<_, _>>()?;

    let mut text = Vec::new(); let mut bases = Vec::new();
    let mut syms: HashMap<String, u64> = HashMap::new();
    for inp in &objs {
        let base = align_up(text.len(), 16); text.resize(base, 0x90); bases.push(base);
        for (n, o) in &inp.text_symbols {
            let abs = u64::try_from(base).map_err(|_| "text offset overflow")? + o;
            if syms.insert(n.clone(), abs).is_some() { return Err(format!("duplicate definition of symbol '{n}'")); }
        }
        text.extend_from_slice(&inp.text);
    }

    // Prefer `main` (C ABI entry wrapper); accept `lpp_main` for runtime-free
    // programs where Cranelift may emit one or both.
    let has_main = syms.contains_key("main");
    let has_lpp = syms.contains_key("lpp_main");
    let call_target = if has_main { syms.get("main") }
        else if has_lpp { syms.get("lpp_main") }
        else { None };
    let call_target = call_target.ok_or_else(|| "required symbol 'main' (or 'lpp_main') not found".to_string())?;

    let start_off = text.len();
    let entry_addr = ELF_BASE + CODE_OFFSET as u64 + call_target;
    let call_next = ELF_BASE + CODE_OFFSET as u64 + start_off as u64 + 11;
    let disp = entry_addr as i64 - call_next as i64;
    if disp < i32::MIN as i64 || disp > i32::MAX as i64 {
        return Err("entry point out of range for startup call".to_string());
    }
    let mut start = vec![
        0x31, 0xed, 0x48, 0x83, 0xe4, 0xf0, // xor ebp; and rsp,-16
        0xe8, 0, 0, 0, 0,                   // call entry
        0x89, 0xc7, 0xb8, 60, 0, 0, 0, 0x0f, 0x05, // exit
    ];
    start[7..11].copy_from_slice(&(disp as i32).to_le_bytes());
    text.extend_from_slice(&start);

    let _ = (has_main, has_lpp); // suppress "unused" when only one branch matters

    let mut got: HashMap<String, usize> = HashMap::new();
    for inp in &objs {
        for rel in &inp.relocations {
            if rel.kind == RelocationKind::GotRelative {
                let n = got.len(); got.entry(rel.target.clone()).or_insert(n);
            }
        }
    }
    let got_off = align_up(text.len(), 8); text.resize(got_off + got.len() * 8, 0);

    let mut rodata_bases = Vec::new();
    let mut rodata_off = align_up(text.len(), 16); text.resize(rodata_off, 0);
    for inp in &objs {
        let base = rodata_off; rodata_bases.push(base);
        for (n, o) in &inp.rodata_symbols {
            let abs = u64::try_from(base).map_err(|_| "rodata offset overflow")? + o;
            if syms.insert(n.clone(), abs).is_some() { return Err(format!("duplicate definition of symbol '{n}'")); }
        }
        text.extend_from_slice(&inp.rodata);
        rodata_off = align_up(text.len(), 16); text.resize(rodata_off, 0);
    }
    for (name, slot) in &got {
        let tgt = *syms.get(name).ok_or_else(|| format!("unresolved GOT symbol '{name}'"))?;
        let loc = got_off + slot * 8;
        text[loc..loc + 8].copy_from_slice(&(ELF_BASE + CODE_OFFSET as u64 + tgt).to_le_bytes());
    }

    for (idx, inp) in objs.iter().enumerate() {
        let base = bases[idx];
        for rel in &inp.relocations {
            if rel.size != 32 { return Err(format!("'{}': unsupported relocation width {}", inp.path.display(), rel.size)); }
            let tgt = match rel.kind {
                RelocationKind::GotRelative => {
                    let slot = *got.get(&rel.target).ok_or_else(|| "missing GOT slot".to_string())?;
                    u64::try_from(got_off + slot * 8).map_err(|_| "GOT overflow")?
                }
                _ if rel.target == "__self_text__" => u64::try_from(base).map_err(|_| "text overflow")?,
                _ if rel.target == "__self_rodata__" => u64::try_from(rodata_bases[idx]).map_err(|_| "rodata overflow")?,
                _ => *syms.get(&rel.target).ok_or_else(||
                    format!("'{}': unresolved external relocation to '{}'", inp.path.display(), rel.target)
                )?,
            };
            let patch = base + rel.offset;
            if patch + 4 > text.len() { return Err(format!("'{}': relocation patch out of range", inp.path.display())); }
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
    elf[0..4].copy_from_slice(b"\x7fELF"); elf[4] = 2; elf[5] = 1; elf[6] = 1;
    put_u16(&mut elf, 16, 2); put_u16(&mut elf, 18, EM_X86_64); put_u32(&mut elf, 20, 1);
    put_u64(&mut elf, 24, ELF_BASE + CODE_OFFSET as u64 + start_off as u64);
    put_u64(&mut elf, 32, 64);
    put_u16(&mut elf, 52, 64); put_u16(&mut elf, 54, 56); put_u16(&mut elf, 56, 1);
    let ph = 64;
    put_u32(&mut elf, ph, PT_LOAD); put_u32(&mut elf, ph + 4, PF_R_X);
    put_u64(&mut elf, ph + 8, 0); put_u64(&mut elf, ph + 16, ELF_BASE);
    put_u64(&mut elf, ph + 24, ELF_BASE);
    put_u64(&mut elf, ph + 32, fsize as u64); put_u64(&mut elf, ph + 40, fsize as u64);
    put_u64(&mut elf, ph + 48, 0x1000);
    elf[CODE_OFFSET..CODE_OFFSET + text.len()].copy_from_slice(&text);
    fs::write(output, elf).map_err(|e| format!("write '{}': {e}", output.display()))?;
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = fs::metadata(output).map_err(|e| format!("stat '{}': {e}", output.display()))?.permissions();
        perm.set_mode(0o755);
        fs::set_permissions(output, perm).map_err(|e| format!("chmod '{}': {e}", output.display()))?;
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
    match rel.kind {
        RelocationKind::Absolute if rel.size == 64 => AMD64_ADDR64,
        RelocationKind::Absolute => AMD64_ADDR32,
        RelocationKind::Relative => AMD64_REL32,
        RelocationKind::SectionIndex => AMD64_SECTION,
        RelocationKind::SectionOffset => AMD64_SECREL,
        _ => if rel.size == 64 { AMD64_ADDR64 } else { AMD64_REL32 },
    }
}

fn read_coff_full(path: &Path) -> Result<CoffSections, String> {
    let bytes = fs::read(path).map_err(|e| format!("read '{}': {e}", path.display()))?;
    let file = object::File::parse(&*bytes).map_err(|e| format!("parse '{}': {e}", path.display()))?;
    if file.format() != BinaryFormat::Coff || file.architecture() != Architecture::X86_64 {
        return Err(format!("'{}' is not an x86-64 COFF object", path.display()));
    }
    let mut text_buf = Vec::new(); let mut rdata_buf = Vec::new(); let mut data_buf = Vec::new();
    let mut map: Vec<(object::SectionIndex, SectionClass, usize)> = Vec::new();
    let mut relocs = Vec::new();

    for sec in file.sections() {
        let idx = sec.index();
        let kind = sec.kind();
        // Skip debug, directive, and other non-loadable sections so their
        // anonymous symbols don't pollute the merged output.
        if kind == object::SectionKind::Debug
            || kind == object::SectionKind::Linker
            || kind == object::SectionKind::Metadata
            || kind == object::SectionKind::Other
        {
            continue;
        }
        let class = match kind {
            object::SectionKind::Text => SectionClass::Text,
            object::SectionKind::ReadOnlyData
            | object::SectionKind::ReadOnlyString => SectionClass::Rodata,
            object::SectionKind::UninitializedData
            | object::SectionKind::UninitializedTls => SectionClass::Data,
            _ => SectionClass::Data,
        };
        let buf: &mut Vec<u8> = match class {
            SectionClass::Text => &mut text_buf, SectionClass::Rodata => &mut rdata_buf, SectionClass::Data => &mut data_buf,
        };
        let base = align_up(buf.len(), 16);
        buf.resize(base, 0x00);
        // BSS / uninitialized sections have zero on-disk size; just reserve
        // virtual space. `uncompressed_data()` may fail for these.
        let is_zero_fill = matches!(kind,
            object::SectionKind::UninitializedData
            | object::SectionKind::UninitializedTls);
        if is_zero_fill {
            let sz = sec.size() as usize;
            buf.resize(buf.len() + sz, 0x00);
            map.push((idx, class, base));
            let padded = align_up(buf.len(), 16);
            buf.resize(padded, 0x00);
            continue;
        }
        let data = sec.uncompressed_data()
            .map_err(|e| format!("read section: {e}"))?
            .into_owned();
        buf.extend_from_slice(&data);
        map.push((idx, class, base));

        for (off, rel) in sec.relocations() {
            let raw_off = usize::try_from(off).map_err(|_| "reloc offset overflow")?;
            let RelocationTarget::Symbol(si) = rel.target() else {
                return Err(format!("'{}' has unsupported non-symbol relocation", path.display()));
            };
            let sym = file.symbol_by_index(si).map_err(|e| format!("read relocation symbol: {e}"))?;
            let raw_name = sym.name().map_err(|e| format!("read relocation symbol name: {e}"))?;
            let target = resolve_coff_target(&raw_name, &sym, &map, class);
            relocs.push(Relocation { offset: base + raw_off, target, addend: rel.addend(), size: rel.size(), kind: rel.kind() });
        }
        let padded = align_up(buf.len(), 16); buf.resize(padded, 0x00);
    }

    let mut syms = Vec::new();
    for sym in file.symbols() {
        if let SymbolSection::Section(idx) = sym.section() {
            if let Some((_, class, base)) = map.iter().find(|(i, _, _)| *i == idx) {
                if let Ok(name) = sym.name() {
                    if !name.is_empty() && !name.starts_with(".text") && !name.starts_with(".rdata")
                        && !name.starts_with(".data") && !name.starts_with(".bss") && !name.starts_with('$')
                    { syms.push((name.to_string(), *class, *base as u64 + sym.address())); }
                }
            }
        }
    }
    Ok(CoffSections { path: path.to_path_buf(), text: text_buf, rdata: rdata_buf, data: data_buf, section_map: map, symbols: syms, relocations: relocs })
}

fn resolve_coff_target(raw_name: &str, sym: &object::Symbol<'_, '_>,
    map: &[(object::SectionIndex, SectionClass, usize)], self_class: SectionClass) -> String {
    let anon = raw_name.is_empty() || sym.kind() == object::SymbolKind::Section
        || raw_name.starts_with(".text") || raw_name.starts_with(".rdata")
        || raw_name.starts_with(".data") || raw_name.starts_with('$');
    if anon {
        if let SymbolSection::Section(idx) = sym.section() {
            if let Some((_, sc, base)) = map.iter().find(|(i, _, _)| *i == idx) {
                return if *sc == self_class { format!("__self_{}__", section_class_tag(self_class)) }
                    else { format!("__ext_{}__{}", section_class_tag(*sc), base) };
            }
        }
        return "__self_text__".to_string();
    }
    raw_name.to_string()
}

fn section_class_tag(c: SectionClass) -> &'static str {
    match c { SectionClass::Text => "text", SectionClass::Rodata => "rdata", SectionClass::Data => "data" }
}
fn pe_align(v: usize, a: usize) -> usize { (v + a - 1) & !(a - 1) }

struct SectionBase { text_base: usize, rdata_base: usize, data_base: usize }

struct ImportData {
    data: Vec<u8>, iat_rvas: HashMap<String, u32>, refptr_offsets: HashMap<String, usize>,
    #[allow(dead_code)] ilt_rva: u32, iat_rva: u32, #[allow(dead_code)] dll_count: usize,
}

fn build_imports(kernel_imports: &[String], refptrs: &[String], section_rva: u32) -> Result<ImportData, String> {
    let count = kernel_imports.len();
    let desc_count = if count == 0 { 0 } else { count + 1 }; let desc_size = desc_count * 20;
    let ilt_count = if count == 0 { 0 } else { count + 1 }; let ilt_size = ilt_count * 8; let iat_size = ilt_count * 8;
    let ilt_off = align_up(desc_size, 8); let iat_off = ilt_off + ilt_size;
    let refptr_off = align_up(iat_off + iat_size, 8);
    let mut data = vec![0u8; refptr_off + refptrs.len() * 8];
    let mut iat_rvas = HashMap::new(); let mut refptr_offsets = HashMap::new();

    if count > 0 {
        let dll_off = data.len(); data.extend_from_slice(b"KERNEL32.dll\0"); while data.len() % 2 != 0 { data.push(0); }
        let mut hint_names: HashMap<String, usize> = HashMap::new();
        for imp in kernel_imports {
            let off = data.len(); data.extend_from_slice(&[0u8, 0u8]); data.extend_from_slice(imp.as_bytes());
            data.push(0); while data.len() % 2 != 0 { data.push(0); }
            hint_names.insert(imp.clone(), off);
        }
        for (i, imp) in kernel_imports.iter().enumerate() {
            let name_rva = section_rva + hint_names[imp] as u32; let thunk = name_rva as u64;
            let ilt_pos = ilt_off + i * 8; let iat_pos = iat_off + i * 8;
            data[ilt_pos..ilt_pos + 8].copy_from_slice(&thunk.to_le_bytes());
            data[iat_pos..iat_pos + 8].copy_from_slice(&thunk.to_le_bytes());
            iat_rvas.insert(format!("__imp_{imp}"), section_rva + iat_pos as u32);
        }
        put_u32(&mut data, 0, section_rva + ilt_off as u32); put_u32(&mut data, 12, section_rva + dll_off as u32);
        put_u32(&mut data, 16, section_rva + iat_off as u32);
    }
    for (i, name) in refptrs.iter().enumerate() { refptr_offsets.insert(format!(".refptr.{name}"), refptr_off + i * 8); }
    Ok(ImportData { data, iat_rvas, refptr_offsets, ilt_rva: section_rva + ilt_off as u32, iat_rva: section_rva + iat_off as u32, dll_count: if count > 0 { 1 } else { 0 } })
}

fn generate_base_relocs(data: &[u8], section_rva: u32) -> Vec<u8> {
    let page_size = 0x1000usize; let mut reloc = Vec::new();
    let mut page = 0usize; let mut entries_for_page: Vec<u16> = Vec::new();
    let flush_page = |page: usize, entries: &mut Vec<u16>, out: &mut Vec<u8>| {
        if entries.is_empty() { return; }
        let padded = align_up(8 + entries.len() * 2, 4); let start = out.len(); out.resize(start + padded, 0);
        put_u32(out, start, (section_rva as usize + page) as u32); put_u32(out, start + 4, padded as u32);
        for (i, e) in entries.iter().enumerate() { put_u16(out, start + 8 + i * 2, *e); }
        entries.clear();
    };
    for off in (0..data.len()).step_by(8) {
        if off + 8 > data.len() { break; }
        let val = u64::from_le_bytes(data[off..off + 8].try_into().unwrap());
        if val >= PE_IMAGE_BASE && val < PE_IMAGE_BASE + 0x100000000 {
            let cur_page = off & !(page_size - 1);
            if cur_page != page { flush_page(page, &mut entries_for_page, &mut reloc); page = cur_page; }
            entries_for_page.push(0xA000u16 | ((off - cur_page) as u16));
        }
    }
    flush_page(page, &mut entries_for_page, &mut reloc);
    reloc
}

fn write_pe(inputs: &[PathBuf], output: &Path) -> Result<(), String> {
    if inputs.is_empty() { return Err("at least one input object is required".to_string()); }
    let objs: Vec<CoffSections> = inputs.iter().map(|p| read_coff_full(p)).collect::<Result<_, _>>()?;
    let mut merged_text = Vec::new(); let mut merged_rdata = Vec::new(); let mut merged_data = Vec::new();
    let mut bases: Vec<SectionBase> = Vec::new();
    let mut global_syms: HashMap<String, (SectionClass, u64)> = HashMap::new();
    for obj in &objs {
        let tb = align_up(merged_text.len(), 16); merged_text.resize(tb, 0x90);
        let rb = align_up(merged_rdata.len(), 16); merged_rdata.resize(rb, 0x00);
        let db = align_up(merged_data.len(), 16); merged_data.resize(db, 0x00);
        bases.push(SectionBase { text_base: tb, rdata_base: rb, data_base: db });
        for (name, class, off) in &obj.symbols {
            let abs = match class { SectionClass::Text => tb as u64 + off, SectionClass::Rodata => rb as u64 + off, SectionClass::Data => db as u64 + off };
            if global_syms.insert(name.clone(), (*class, abs)).is_some() { return Err(format!("duplicate definition of symbol '{name}'")); }
        }
        merged_text.extend_from_slice(&obj.text); merged_rdata.extend_from_slice(&obj.rdata); merged_data.extend_from_slice(&obj.data);
    }
    let mut kernel_imports: Vec<String> = Vec::new(); let mut refptr_names: Vec<String> = Vec::new();
    for obj in &objs { for rel in &obj.relocations {
        if let Some(name) = rel.target.strip_prefix("__imp_") { let n = name.to_string(); if !kernel_imports.contains(&n) { kernel_imports.push(n); } }
        else if let Some(name) = rel.target.strip_prefix(".refptr.") { let n = name.to_string(); if !refptr_names.contains(&n) { refptr_names.push(n); } }
    }}
    let refptr_data_off = merged_data.len(); merged_data.resize(refptr_data_off + refptr_names.len() * 8, 0);
    for (i, name) in refptr_names.iter().enumerate() {
        if let Some((_class, abs)) = global_syms.get(name) {
            let addr = PE_IMAGE_BASE + PE_SECTION_RVA as u64 + *abs;
            merged_data[refptr_data_off + i * 8..][..8].copy_from_slice(&addr.to_le_bytes());
        }
    }
    let text_rva = PE_SECTION_RVA; let text_raw_size = pe_align(merged_text.len(), PE_FILE_ALIGN);
    let rdata_rva = pe_align(text_rva as usize + merged_text.len(), PE_SECT_ALIGN) as u32;
    let rdata_raw_size = pe_align(merged_rdata.len(), PE_FILE_ALIGN);
    let data_rva = pe_align(rdata_rva as usize + merged_rdata.len(), PE_SECT_ALIGN) as u32;
    let data_raw_size = pe_align(merged_data.len(), PE_FILE_ALIGN);
    let idata_rva = pe_align(data_rva as usize + merged_data.len(), PE_SECT_ALIGN) as u32;
    let import = build_imports(&kernel_imports, &refptr_names, idata_rva)?;
    let has_idata = !import.data.is_empty();
    let idata_raw_size = if has_idata { pe_align(import.data.len(), PE_FILE_ALIGN) } else { 0 };

    let mut all_writable = merged_data.clone(); if has_idata { all_writable.extend_from_slice(&import.data); }
    let reloc_data = generate_base_relocs(&all_writable, data_rva);
    let reloc_rva = if !reloc_data.is_empty() { pe_align(if has_idata { idata_rva as usize + import.data.len() } else { data_rva as usize + merged_data.len() }, PE_SECT_ALIGN) as u32 } else { 0 };
    let has_reloc = !reloc_data.is_empty(); let reloc_raw_size = if has_reloc { pe_align(reloc_data.len(), PE_FILE_ALIGN) } else { 0 };

    for (idx, obj) in objs.iter().enumerate() {
        let b = &bases[idx];
        for rel in &obj.relocations {
            let patch_class = section_class_for_offset(rel.offset, b.text_base, merged_text.len(), b.rdata_base, merged_rdata.len(), b.data_base, merged_data.len());
            let (patch_buf, patch_rva) = match patch_class { SectionClass::Text => (&mut merged_text, text_rva), SectionClass::Rodata => (&mut merged_rdata, rdata_rva), SectionClass::Data => (&mut merged_data, data_rva) };
            let patch = rel.offset; let patch_rva_addr = patch_rva as i64 + patch as i64;
            let target = resolve_pe_target(rel, &global_syms, &import.iat_rvas, &import.refptr_offsets, &bases[idx], text_rva, rdata_rva, data_rva, idata_rva)?;
            let rnum = coff_reloc_number(rel);
            match rnum {
                AMD64_ADDR64 => { if patch + 8 > patch_buf.len() { return Err("ADDR64 patch OOB".into()); } patch_buf[patch..patch + 8].copy_from_slice(&(target as u64).to_le_bytes()); }
                AMD64_ADDR32 | AMD64_ADDR32NB => { if patch + 4 > patch_buf.len() { return Err("ADDR32 patch OOB".into()); } let v = target; if v < i32::MIN as u64 || v > i32::MAX as u64 { return Err("ADDR32 overflow".into()); } patch_buf[patch..patch + 4].copy_from_slice(&(v as i32).to_le_bytes()); }
                AMD64_REL32 | AMD64_REL32_1 | AMD64_REL32_2 | AMD64_REL32_3 | AMD64_REL32_4 | AMD64_REL32_5 => {
                    if patch + 4 > patch_buf.len() { return Err("REL32 patch OOB".into()); }
                    let adj: i64 = match rnum { AMD64_REL32_1=>1,AMD64_REL32_2=>2,AMD64_REL32_3=>3,AMD64_REL32_4=>4,AMD64_REL32_5=>5,_=>0 };
                    let disp = target as i64 + rel.addend - (patch_rva_addr + 4 + adj);
                    patch_buf[patch..patch + 4].copy_from_slice(&(disp as i32).to_le_bytes());
                }
                AMD64_SECTION => {}
                AMD64_SECREL => { if patch + 4 > patch_buf.len() { return Err("SECREL patch OOB".into()); } patch_buf[patch..patch + 4].copy_from_slice(&(target as u32).to_le_bytes()); }
                _ => return Err(format!("unsupported COFF relocation type {rnum}")),
            }
        }
    }
    for (i, name) in refptr_names.iter().enumerate() {
        let pos = refptr_data_off + i * 8; if pos + 8 > merged_data.len() { continue; }
        let val = u64::from_le_bytes(merged_data[pos..pos + 8].try_into().unwrap()); if val != 0 { continue; }
        if let Some((class, abs)) = global_syms.get(name) {
            let rva = match class { SectionClass::Text=>text_rva as u64+abs, SectionClass::Rodata=>rdata_rva as u64+abs, SectionClass::Data=>data_rva as u64+abs };
            merged_data[pos..pos + 8].copy_from_slice(&(PE_IMAGE_BASE + rva).to_le_bytes());
        }
    }
    let headers_size = PE_FILE_ALIGN; let text_raw_off = headers_size;
    let rdata_raw_off = text_raw_off + text_raw_size; let data_raw_off = rdata_raw_off + rdata_raw_size;
    let idata_raw_off = data_raw_off + data_raw_size; let reloc_raw_off = idata_raw_off + idata_raw_size;
    let mut section_count: u16 = 1;
    if !merged_rdata.is_empty() { section_count += 1; }
    if !merged_data.is_empty() || !refptr_names.is_empty() { section_count += 1; }
    if has_idata { section_count += 1; } if has_reloc { section_count += 1; }
    let image_end = reloc_rva as usize + if has_reloc { reloc_data.len() } else { 0 };
    let image_size = pe_align(if image_end > 0 { image_end } else { data_rva as usize + merged_data.len() }, PE_SECT_ALIGN);
    let file_size = reloc_raw_off + reloc_raw_size; let mut pe = vec![0u8; file_size.max(headers_size)];

    pe[0..2].copy_from_slice(b"MZ"); put_u32(&mut pe, 0x3c, 0x80);
    let nt = 0x80; pe[nt..nt + 4].copy_from_slice(b"PE\0\0"); put_u16(&mut pe, nt + 4, 0x8664);
    put_u16(&mut pe, nt + 6, section_count); let opt_size: u16 = 0xF0;
    put_u16(&mut pe, nt + 20, opt_size); put_u16(&mut pe, nt + 22, 0x0022);
    let opt = nt + 24; put_u16(&mut pe, opt, 0x20b);
    put_u32(&mut pe, opt + 4, text_raw_size as u32);
    put_u32(&mut pe, opt + 8, (rdata_raw_size + data_raw_size + idata_raw_size) as u32);
    let main_abs = global_syms.get("main").map(|(c, a)| match c { SectionClass::Text => text_rva as u64 + a, _ => text_rva as u64 + a })
        .ok_or_else(|| "required symbol 'main' not found".to_string())?;
    put_u32(&mut pe, opt + 16, main_abs as u32); put_u32(&mut pe, opt + 20, text_rva);
    put_u64(&mut pe, opt + 24, PE_IMAGE_BASE);
    put_u32(&mut pe, opt + 32, PE_SECT_ALIGN as u32); put_u32(&mut pe, opt + 36, PE_FILE_ALIGN as u32);
    put_u16(&mut pe, opt + 40, 6); put_u16(&mut pe, opt + 48, 6);
    put_u32(&mut pe, opt + 56, image_size as u32); put_u32(&mut pe, opt + 60, headers_size as u32);
    put_u16(&mut pe, opt + 68, 3); put_u16(&mut pe, opt + 70, 0x8140);
    put_u64(&mut pe, opt + 72, 0x100000); put_u64(&mut pe, opt + 80, 0x1000);
    put_u64(&mut pe, opt + 88, 0x100000); put_u64(&mut pe, opt + 96, 0x1000);
    put_u32(&mut pe, opt + 108, 16);
    let dirs = opt + 112;
    if has_idata { put_u32(&mut pe, dirs + 8, idata_rva); put_u32(&mut pe, dirs + 12, import.data.len() as u32); put_u32(&mut pe, dirs + 12*8, import.iat_rva); put_u32(&mut pe, dirs + 12*8 + 4, ((kernel_imports.len()+1)*8) as u32); }
    if has_reloc { put_u32(&mut pe, dirs + 5*8, reloc_rva); put_u32(&mut pe, dirs + 5*8 + 4, reloc_data.len() as u32); }

    let emit_section = |pe: &mut [u8], sec: &mut usize, name: &[u8;8], rva: u32, raw_size: usize, raw_off: usize, virt_size: usize, ch: u32| {
        pe[*sec..*sec + 8].copy_from_slice(name); put_u32(pe, *sec + 8, virt_size as u32);
        put_u32(pe, *sec + 12, rva); put_u32(pe, *sec + 16, raw_size as u32); put_u32(pe, *sec + 20, raw_off as u32); put_u32(pe, *sec + 36, ch); *sec += 40;
    };
    let mut sec = opt + opt_size as usize;
    let mut tname = [0u8;8]; tname[..5].copy_from_slice(b".text"); emit_section(&mut pe, &mut sec, &tname, text_rva, text_raw_size, text_raw_off, merged_text.len(), 0x60000020);
    if !merged_rdata.is_empty() { let mut rname = [0u8;8]; rname[..6].copy_from_slice(b".rdata"); emit_section(&mut pe, &mut sec, &rname, rdata_rva, rdata_raw_size, rdata_raw_off, merged_rdata.len(), 0x40000040); }
    if !merged_data.is_empty() || !refptr_names.is_empty() { let mut dname = [0u8;8]; dname[..5].copy_from_slice(b".data"); emit_section(&mut pe, &mut sec, &dname, data_rva, data_raw_size, data_raw_off, merged_data.len(), 0xC0000040); }
    if has_idata { let mut iname = [0u8;8]; iname[..6].copy_from_slice(b".idata"); emit_section(&mut pe, &mut sec, &iname, idata_rva, idata_raw_size, idata_raw_off, import.data.len(), 0xC0000040); }
    if has_reloc { let mut rlname = [0u8;8]; rlname[..6].copy_from_slice(b".reloc"); emit_section(&mut pe, &mut sec, &rlname, reloc_rva, reloc_raw_size, reloc_raw_off, reloc_data.len(), 0x42000040); }

    pe[text_raw_off..text_raw_off + merged_text.len()].copy_from_slice(&merged_text);
    if !merged_rdata.is_empty() { pe[rdata_raw_off..rdata_raw_off + merged_rdata.len()].copy_from_slice(&merged_rdata); }
    if !merged_data.is_empty() || !refptr_names.is_empty() { pe[data_raw_off..data_raw_off + merged_data.len()].copy_from_slice(&merged_data); }
    if has_idata { pe[idata_raw_off..idata_raw_off + import.data.len()].copy_from_slice(&import.data); }
    if has_reloc { pe[reloc_raw_off..reloc_raw_off + reloc_data.len()].copy_from_slice(&reloc_data); }
    fs::write(output, pe).map_err(|e| format!("write '{}': {e}", output.display()))?;
    Ok(())
}

fn section_class_for_offset(offset: usize, text_base: usize, text_len: usize, rdata_base: usize, rdata_len: usize, data_base: usize, data_len: usize) -> SectionClass {
    if offset >= text_base && offset < text_base + text_len { SectionClass::Text }
    else if offset >= rdata_base && offset < rdata_base + rdata_len { SectionClass::Rodata }
    else if offset >= data_base && offset < data_base + data_len { SectionClass::Data } else { SectionClass::Text }
}

fn resolve_pe_target(rel: &Relocation, global_syms: &HashMap<String, (SectionClass, u64)>, iat_rvas: &HashMap<String, u32>, _refptr_offsets: &HashMap<String, usize>, bases: &SectionBase, text_rva: u32, rdata_rva: u32, data_rva: u32, _idata_rva: u32) -> Result<u64, String> {
    if rel.target.starts_with("__self_text__") { return Ok(PE_IMAGE_BASE + text_rva as u64 + bases.text_base as u64); }
    if rel.target.starts_with("__self_rdata__") { return Ok(PE_IMAGE_BASE + rdata_rva as u64 + bases.rdata_base as u64); }
    if rel.target.starts_with("__self_data__") { return Ok(PE_IMAGE_BASE + data_rva as u64 + bases.data_base as u64); }
    if let Some(rest) = rel.target.strip_prefix("__ext_text__") { let eb: usize = rest.parse().map_err(|_| "invalid")?; return Ok(PE_IMAGE_BASE + text_rva as u64 + eb as u64); }
    if let Some(rest) = rel.target.strip_prefix("__ext_rdata__") { let eb: usize = rest.parse().map_err(|_| "invalid")?; return Ok(PE_IMAGE_BASE + rdata_rva as u64 + eb as u64); }
    if let Some(rest) = rel.target.strip_prefix("__ext_data__") { let eb: usize = rest.parse().map_err(|_| "invalid")?; return Ok(PE_IMAGE_BASE + data_rva as u64 + eb as u64); }
    if let Some(rva) = iat_rvas.get(&rel.target) { return Ok(PE_IMAGE_BASE + *rva as u64); }
    if let Some((class, abs)) = global_syms.get(&rel.target) { let rva = match class { SectionClass::Text=>text_rva as u64+abs, SectionClass::Rodata=>rdata_rva as u64+abs, SectionClass::Data=>data_rva as u64+abs }; return Ok(PE_IMAGE_BASE + rva); }
    if let Some(name) = rel.target.strip_prefix(".refptr.") { if let Some((class, abs)) = global_syms.get(name) { let rva = match class { SectionClass::Text=>text_rva as u64+abs, SectionClass::Rodata=>rdata_rva as u64+abs, SectionClass::Data=>data_rva as u64+abs }; return Ok(PE_IMAGE_BASE + rva); } }
    if let Some(rest) = rel.target.strip_prefix("__coff_text_section_") { let off: u32 = rest.parse().map_err(|_| "invalid")?; return Ok(PE_IMAGE_BASE + text_rva as u64 + off as u64); }
    Err(format!("unresolved external COFF symbol '{}'", rel.target))
}

// ═══════════════════════════════════════════════════════════════════════════
//  3.  Mach-O path  (kept stable)
// ═══════════════════════════════════════════════════════════════════════════

fn read_macho_input(path: &Path) -> Result<MachoInput, String> {
    let bytes = fs::read(path).map_err(|e| format!("read '{}': {e}", path.display()))?;
    let file = object::File::parse(&*bytes).map_err(|e| format!("parse '{}': {e}", path.display()))?;
    if file.format() != BinaryFormat::MachO { return Err(format!("'{}' is not a Mach-O relocatable object", path.display())); }
    let mut text = Vec::new(); let mut sec_bases: Vec<(object::SectionIndex, usize)> = Vec::new(); let mut sec_relocs = Vec::new();
    for sec in file.sections() {
        if sec.kind() != object::SectionKind::Text { continue; }
        let base = align_up(text.len(), 16); text.resize(base, 0x90); let idx = sec.index();
        let data = sec.uncompressed_data().map_err(|e| format!("read section from {}: {e}", path.display()))?.into_owned(); text.extend_from_slice(&data);
        sec_bases.push((idx, base));
        for (off, rel) in sec.relocations() { sec_relocs.push((idx, base, off, rel)); }
    }
    if sec_bases.is_empty() { return Err(format!("'{}' has no executable Mach-O text section", path.display())); }
    let find_base = |idx: object::SectionIndex| -> Option<usize> { sec_bases.iter().find(|(i, _)| *i == idx).map(|(_, b)| *b) };
    let mut text_syms = Vec::new();
    for sym in file.symbols() { if let SymbolSection::Section(idx) = sym.section() { if let Some(base) = find_base(idx) { if let Ok(name) = sym.name() { let clean = name.strip_prefix('_').unwrap_or(name); if !clean.is_empty() { text_syms.push((clean.to_string(), base as u64 + sym.address())); } } } } }
    let mut relocs = Vec::new();
    for (_, base, off, rel) in sec_relocs {
        let RelocationTarget::Symbol(si) = rel.target() else { return Err("unsupported non-symbol reloc".into()); };
        let sym = file.symbol_by_index(si).map_err(|e| format!("symbol: {e}"))?; let raw_name = sym.name().map_err(|e| format!("name: {e}"))?;
        let clean = raw_name.strip_prefix('_').unwrap_or(raw_name);
        let target = if clean.is_empty() { match sym.section() { SymbolSection::Section(idx) if find_base(idx).is_some() => format!("__macho_text_section_{}", find_base(idx).unwrap()), _ => return Err("unresolved anonymous Mach-O relocation".into()), } } else { clean.to_string() };
        relocs.push(Relocation { offset: base + usize::try_from(off).map_err(|_| "overflow")?, target, addend: rel.addend(), size: rel.size(), kind: rel.kind() });
    }
    Ok(MachoInput { path: path.to_path_buf(), text, text_symbols: text_syms, relocations: relocs })
}

fn write_macho(inputs: &[PathBuf], output: &Path) -> Result<(), String> {
    if inputs.is_empty() { return Err("at least one input required".into()); }
    let objs: Vec<MachoInput> = inputs.iter().map(|p| read_macho_input(p)).collect::<Result<_, _>>()?;
    let mut text = Vec::new(); let mut bases = Vec::new(); let mut syms: HashMap<String, u64> = HashMap::new();
    for inp in &objs { let base = align_up(text.len(), 16); text.resize(base, 0x90); bases.push(base); for (n, o) in &inp.text_symbols { let abs = base as u64 + o; if syms.insert(n.clone(), abs).is_some() { return Err(format!("duplicate '{n}'")); } } text.extend_from_slice(&inp.text); }
    let main = *syms.get("main").or_else(|| syms.get("_main")).ok_or_else(|| "required symbol 'main' or '_main' not found".to_string())?;
    for (idx, inp) in objs.iter().enumerate() { let base = bases[idx]; for rel in &inp.relocations {
        let tgt_off = if rel.target == "__self_text__" { base as u64 } else if let Some(off) = rel.target.strip_prefix("__macho_text_section_") { off.parse::<u64>().map_err(|_| "invalid")? } else { *syms.get(&rel.target).ok_or_else(|| format!("unresolved '{}'", rel.target))? };
        let patch = base + rel.offset; if patch + 4 > text.len() { return Err("patch OOB".into()); } text[patch..patch + 4].copy_from_slice(&((tgt_off as i64 + rel.addend - patch as i64) as i32).to_le_bytes());
    }}
    let mut header = Vec::new();
    header.extend_from_slice(&0xfeedfacfu32.to_le_bytes()); header.extend_from_slice(&0x01000007u32.to_le_bytes());
    header.extend_from_slice(&3u32.to_le_bytes()); header.extend_from_slice(&2u32.to_le_bytes()); header.extend_from_slice(&2u32.to_le_bytes());
    header.extend_from_slice(&((72+152) as u32).to_le_bytes()); header.extend_from_slice(&0x00200085u32.to_le_bytes()); header.extend_from_slice(&0u32.to_le_bytes());
    // PAGEZERO
    header.extend_from_slice(&0x19u32.to_le_bytes()); header.extend_from_slice(&72u32.to_le_bytes());
    let mut pz = [0u8;16]; pz[..10].copy_from_slice(b"__PAGEZERO"); header.extend_from_slice(&pz);
    header.extend_from_slice(&0u64.to_le_bytes()); header.extend_from_slice(&0x100000000u64.to_le_bytes());
    header.extend_from_slice(&0u64.to_le_bytes()); header.extend_from_slice(&0u64.to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes()); header.extend_from_slice(&0u32.to_le_bytes()); header.extend_from_slice(&0u32.to_le_bytes()); header.extend_from_slice(&0u32.to_le_bytes());
    // TEXT
    header.extend_from_slice(&0x19u32.to_le_bytes()); header.extend_from_slice(&152u32.to_le_bytes());
    let mut ts = [0u8;16]; ts[..6].copy_from_slice(b"__TEXT"); header.extend_from_slice(&ts);
    header.extend_from_slice(&0x100000000u64.to_le_bytes()); header.extend_from_slice(&(4096 + align_up(text.len(), 4096) as u64).to_le_bytes());
    header.extend_from_slice(&0u64.to_le_bytes()); header.extend_from_slice(&(4096 + text.len() as u64).to_le_bytes());
    header.extend_from_slice(&7u32.to_le_bytes()); header.extend_from_slice(&5u32.to_le_bytes()); header.extend_from_slice(&1u32.to_le_bytes()); header.extend_from_slice(&0u32.to_le_bytes());
    let mut tn = [0u8;16]; tn[..6].copy_from_slice(b"__text"); header.extend_from_slice(&tn); header.extend_from_slice(&ts);
    header.extend_from_slice(&(0x100000000u64 + 4096 + main).to_le_bytes()); header.extend_from_slice(&(text.len() as u64).to_le_bytes());
    header.extend_from_slice(&4096u32.to_le_bytes()); header.extend_from_slice(&4u32.to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes()); header.extend_from_slice(&0u32.to_le_bytes()); header.extend_from_slice(&0x80000400u32.to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes()); header.extend_from_slice(&0u32.to_le_bytes()); header.extend_from_slice(&0u32.to_le_bytes());
    let mut bin = vec![0u8; 4096]; bin[..header.len()].copy_from_slice(&header); bin.extend_from_slice(&text);
    fs::write(output, bin).map_err(|e| format!("write Mach-O binary: {e}"))?;
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
//  4.  inspect + main
// ═══════════════════════════════════════════════════════════════════════════

fn inspect_object(input: &Path) -> Result<(), String> {
    let bytes = fs::read(input).map_err(|e| format!("read '{}': {e}", input.display()))?;
    let file = object::File::parse(&*bytes).map_err(|e| format!("parse '{}': {e}", input.display()))?;
    let mut reloc_count = 0usize; let mut reloc_kinds: BTreeMap<String, usize> = BTreeMap::new();
    println!("format: {:?}", file.format()); println!("architecture: {:?}", file.architecture()); println!("sections:");
    for sec in file.sections() { for (_, rel) in sec.relocations() { reloc_count += 1; *reloc_kinds.entry(format!("{:?}", rel.kind())).or_default() += 1; } println!("  {} size={} kind={:?}", sec.name().unwrap_or("<unnamed>"), sec.size(), sec.kind()); }
    println!("symbols: defined={} undefined={}", file.symbols().filter(|s| !s.is_undefined()).count(), file.symbols().filter(|s| s.is_undefined()).count());
    println!("relocations: {reloc_count}"); println!("relocation-kinds:");
    for (k, c) in reloc_kinds { println!("  {k}={c}"); }
    Ok(())
}

fn usage() {
    eprintln!("Usage: lpp-link <program.o> [runtime.o ...] -o <output>");
    eprintln!("       lpp-link pe <program.obj> [runtime.obj ...] -o <output.exe>");
    eprintln!("       lpp-link macho <program.o> [runtime.o ...] -o <output>");
    eprintln!("       lpp-link inspect <object.o>");
    eprintln!("       lpp-link --help");
    eprintln!("Default mode auto-detects: ELF on Linux, PE on Windows, Mach-O on macOS.");
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("inspect") {
        if args.len() != 2 { usage(); std::process::exit(2); }
        if let Err(e) = inspect_object(Path::new(&args[1])) { eprintln!("lpp-link inspect error: {e}"); std::process::exit(1); }
        return;
    }
    if args.first().map(String::as_str) == Some("--help") || args.first().map(String::as_str) == Some("-h") { usage(); return; }
    let (mode, offset) = match args.first().map(String::as_str) { Some("pe") => (OutputFormat::PE, 1), Some("macho") => (OutputFormat::MachO, 1), _ => (OutputFormat::for_host(), 0) };
    let Some(output_rel) = args[offset..].iter().position(|a| a == "-o") else { usage(); std::process::exit(2); };
    let out_idx = offset + output_rel;
    if out_idx == offset || out_idx + 2 != args.len() { usage(); std::process::exit(2); }
    let inputs: Vec<PathBuf> = args[offset..out_idx].iter().map(PathBuf::from).collect();
    let result = match mode { OutputFormat::PE => write_pe(&inputs, Path::new(&args[out_idx + 1])), OutputFormat::MachO => write_macho(&inputs, Path::new(&args[out_idx + 1])), OutputFormat::Elf => write_elf(&inputs, Path::new(&args[out_idx + 1])) };
    if let Err(e) = result { eprintln!("lpp-link error: {e}"); std::process::exit(1); }
}
