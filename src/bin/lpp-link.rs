//! `lpp-link` Phase 2: direct Linux x86-64 ELF executable emission.
//!
//! The linker deliberately grows in small verified slices. It currently merges
//! `.text` from one or more x86-64 ELF objects and resolves internal 32-bit
//! PC-relative relocations. This is sufficient for Cranelift objects plus the
//! freestanding `lpp_runtime_min.o` print runtime, without invoking a host
//! compiler or linker during the final link step.

use object::{Architecture, BinaryFormat, Object, ObjectSection, ObjectSymbol, RelocationKind, RelocationTarget, SymbolSection};
use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const ELF_BASE: u64 = 0x400000;
const CODE_OFFSET: usize = 0x1000;
const EM_X86_64: u16 = 62;
const PT_LOAD: u32 = 1;
const PF_R_X: u32 = 5;

struct Relocation {
    offset: usize,
    target: String,
    addend: i64,
    size: u8,
    kind: RelocationKind,
}

struct InputText {
    path: PathBuf,
    text: Vec<u8>,
    rodata: Vec<u8>,
    text_symbols: Vec<(String, u64)>,
    rodata_symbols: Vec<(String, u64)>,
    relocations: Vec<Relocation>,
}

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

fn read_input(path: &Path) -> Result<InputText, String> {
    let bytes = fs::read(path).map_err(|error| format!("read '{}': {error}", path.display()))?;
    let file = object::File::parse(&*bytes).map_err(|error| format!("parse '{}': {error}", path.display()))?;
    if file.format() != BinaryFormat::Elf || file.architecture() != Architecture::X86_64 {
        return Err(format!("'{}' is not an x86-64 ELF relocatable object", path.display()));
    }
    let text_section = file.section_by_name(".text")
        .ok_or_else(|| format!("'{}' has no .text section", path.display()))?;
    let text_index = text_section.index();
    let text = text_section.uncompressed_data()
        .map_err(|error| format!("read .text from '{}': {error}", path.display()))?
        .into_owned();
    let (rodata_index, rodata) = if let Some(section) = file.section_by_name(".rodata") {
        let index = section.index();
        let data = section.uncompressed_data()
            .map_err(|error| format!("read .rodata from '{}': {error}", path.display()))?
            .into_owned();
        (Some(index), data)
    } else {
        (None, Vec::new())
    };

    let mut text_symbols = Vec::new();
    let mut rodata_symbols = Vec::new();
    for symbol in file.symbols() {
        let destination = if symbol.section() == SymbolSection::Section(text_index) {
            Some(&mut text_symbols)
        } else if rodata_index.is_some_and(|index| symbol.section() == SymbolSection::Section(index)) {
            Some(&mut rodata_symbols)
        } else {
            None
        };
        if let Some(destination) = destination {
            if let Ok(name) = symbol.name() {
                if !name.is_empty() {
                    destination.push((name.to_string(), symbol.address()));
                }
            }
        }
    }

    let mut relocations = Vec::new();
    for (offset, relocation) in text_section.relocations() {
        let RelocationTarget::Symbol(symbol_index) = relocation.target() else {
            return Err(format!("'{}' has unsupported non-symbol relocation", path.display()));
        };
        let symbol = file.symbol_by_index(symbol_index)
            .map_err(|error| format!("read relocation symbol: {error}"))?;
        let raw_name = symbol.name()
            .map_err(|error| format!("read relocation symbol name: {error}"))?;
        // GCC may target a local section symbol (whose printable name is
        // empty) for absolute function-pointer relocations.
        let target = if raw_name.is_empty() && symbol.section() == SymbolSection::Section(text_index) {
            "__self_text__".to_string()
        } else if raw_name.is_empty() && rodata_index.is_some_and(|index| symbol.section() == SymbolSection::Section(index)) {
            "__self_rodata__".to_string()
        } else {
            raw_name.to_string()
        };
        relocations.push(Relocation {
            offset: usize::try_from(offset).map_err(|_| "relocation offset overflow")?,
            target,
            addend: relocation.addend(),
            size: relocation.size(),
            kind: relocation.kind(),
        });
    }
    Ok(InputText {
        path: path.to_path_buf(),
        text,
        rodata,
        text_symbols,
        rodata_symbols,
        relocations,
    })
}

fn write_elf(inputs: &[PathBuf], output: &Path) -> Result<(), String> {
    if inputs.is_empty() {
        return Err("at least one input object is required".to_string());
    }
    let objects: Vec<InputText> = inputs.iter().map(|path| read_input(path)).collect::<Result<_, _>>()?;

    let mut text = Vec::new();
    let mut bases = Vec::new();
    let mut symbols: HashMap<String, u64> = HashMap::new();
    for input in &objects {
        let base = align_up(text.len(), 16);
        text.resize(base, 0x90); // NOP padding between object text sections.
        bases.push(base);
        for (name, offset) in &input.text_symbols {
            let absolute = u64::try_from(base).map_err(|_| "text offset overflow")? + offset;
            if symbols.insert(name.clone(), absolute).is_some() {
                return Err(format!("duplicate definition of symbol '{name}'"));
            }
        }
        text.extend_from_slice(&input.text);
    }
    let _lpp_main = *symbols.get("lpp_main").ok_or_else(|| "required symbol 'lpp_main' not found".to_string())?;
    let main = *symbols.get("main").ok_or_else(|| "required symbol 'main' not found".to_string())?;

    // Linux `_start`: align stack, call C ABI main, exit(main_status) via syscall.
    let start_offset = text.len();
    let main_address = ELF_BASE + CODE_OFFSET as u64 + main;
    let call_next = ELF_BASE + CODE_OFFSET as u64 + start_offset as u64 + 11;
    let call_displacement = main_address as i64 - call_next as i64;
    if call_displacement < i32::MIN as i64 || call_displacement > i32::MAX as i64 {
        return Err("main is out of range for startup call".to_string());
    }
    let mut start = vec![
        0x31, 0xed, 0x48, 0x83, 0xe4, 0xf0, // xor ebp; and rsp,-16
        0xe8, 0, 0, 0, 0,                   // call main
        0x89, 0xc7, 0xb8, 60, 0, 0, 0, 0x0f, 0x05, // exit syscall
    ];
    start[7..11].copy_from_slice(&(call_displacement as i32).to_le_bytes());
    text.extend_from_slice(&start);

    // PIC Cranelift imports runtime functions and readonly data through GOTPCREL.
    let mut got_slots: HashMap<String, usize> = HashMap::new();
    for input in &objects {
        for relocation in &input.relocations {
            if relocation.kind == RelocationKind::GotRelative {
                let next = got_slots.len();
                got_slots.entry(relocation.target.clone()).or_insert(next);
            }
        }
    }
    let got_offset = align_up(text.len(), 8);
    text.resize(got_offset + got_slots.len() * 8, 0);

    // Merge readonly data after GOT. It stays in the same read/execute load
    // segment for this MVP; writable data gets a separate segment later.
    let mut rodata_bases = Vec::new();
    let mut rodata_offset = align_up(text.len(), 16);
    text.resize(rodata_offset, 0);
    for input in &objects {
        let base = rodata_offset;
        rodata_bases.push(base);
        for (name, offset) in &input.rodata_symbols {
            let absolute = u64::try_from(base).map_err(|_| "rodata offset overflow")? + offset;
            if symbols.insert(name.clone(), absolute).is_some() {
                return Err(format!("duplicate definition of symbol '{name}'"));
            }
        }
        text.extend_from_slice(&input.rodata);
        rodata_offset = align_up(text.len(), 16);
        text.resize(rodata_offset, 0);
    }
    for (name, slot) in &got_slots {
        let target = *symbols.get(name).ok_or_else(|| {
            format!("unresolved GOT symbol '{name}'")
        })?;
        let location = got_offset + slot * 8;
        let address = ELF_BASE + CODE_OFFSET as u64 + target;
        text[location..location + 8].copy_from_slice(&address.to_le_bytes());
    }

    for (index, input) in objects.iter().enumerate() {
        let base = bases[index];
        for relocation in &input.relocations {
            if relocation.size != 32 {
                return Err(format!("'{}': unsupported relocation width {}", input.path.display(), relocation.size));
            }
            let target = match relocation.kind {
                RelocationKind::GotRelative => {
                    let slot = *got_slots.get(&relocation.target).ok_or_else(|| "missing GOT slot".to_string())?;
                    u64::try_from(got_offset + slot * 8).map_err(|_| "GOT offset overflow")?
                }
                _ if relocation.target == "__self_text__" => {
                    u64::try_from(base).map_err(|_| "text offset overflow")?
                }
                _ if relocation.target == "__self_rodata__" => {
                    u64::try_from(rodata_bases[index]).map_err(|_| "rodata offset overflow")?
                }
                _ => *symbols.get(&relocation.target).ok_or_else(|| {
                    format!("'{}': unresolved external relocation to '{}'", input.path.display(), relocation.target)
                })?,
            };
            let patch = base + relocation.offset;
            if patch + 4 > text.len() {
                return Err(format!("'{}': relocation patch out of range", input.path.display()));
            }
            if relocation.kind == RelocationKind::Absolute {
                let value = ELF_BASE as i64 + CODE_OFFSET as i64 + target as i64 + relocation.addend;
                if value < i32::MIN as i64 || value > i32::MAX as i64 {
                    return Err(format!("'{}': absolute relocation out of range", input.path.display()));
                }
                text[patch..patch + 4].copy_from_slice(&(value as i32).to_le_bytes());
            } else {
                let displacement = target as i64 + relocation.addend - patch as i64;
                if displacement < i32::MIN as i64 || displacement > i32::MAX as i64 {
                    return Err(format!("'{}': PC-relative relocation out of range", input.path.display()));
                }
                text[patch..patch + 4].copy_from_slice(&(displacement as i32).to_le_bytes());
            }
        }
    }

    let file_size = CODE_OFFSET + text.len();
    let mut elf = vec![0_u8; file_size];
    elf[0..4].copy_from_slice(b"\x7fELF");
    elf[4] = 2; // ELFCLASS64
    elf[5] = 1; // little endian
    elf[6] = 1; // ELF version
    put_u16(&mut elf, 16, 2); // ET_EXEC
    put_u16(&mut elf, 18, EM_X86_64);
    put_u32(&mut elf, 20, 1);
    put_u64(&mut elf, 24, ELF_BASE + CODE_OFFSET as u64 + start_offset as u64);
    put_u64(&mut elf, 32, 64); // program header offset
    put_u16(&mut elf, 52, 64); // ELF header size
    put_u16(&mut elf, 54, 56); // program header size
    put_u16(&mut elf, 56, 1);  // one program header

    let ph = 64;
    put_u32(&mut elf, ph, PT_LOAD);
    put_u32(&mut elf, ph + 4, PF_R_X);
    put_u64(&mut elf, ph + 8, 0);
    put_u64(&mut elf, ph + 16, ELF_BASE);
    put_u64(&mut elf, ph + 24, ELF_BASE);
    put_u64(&mut elf, ph + 32, file_size as u64);
    put_u64(&mut elf, ph + 40, file_size as u64);
    put_u64(&mut elf, ph + 48, 0x1000);

    elf[CODE_OFFSET..CODE_OFFSET + text.len()].copy_from_slice(&text);
    fs::write(output, elf).map_err(|error| format!("write '{}': {error}", output.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(output)
            .map_err(|error| format!("stat '{}': {error}", output.display()))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(output, permissions)
            .map_err(|error| format!("chmod '{}': {error}", output.display()))?;
    }
    Ok(())
}


const PE_IMAGE_BASE: u64 = 0x140000000;
const PE_SECTION_RVA: u32 = 0x1000;
const PE_FILE_ALIGNMENT: usize = 0x200;
const PE_SECTION_ALIGNMENT: usize = 0x1000;

fn read_coff_input(path: &Path) -> Result<InputText, String> {
    let bytes = fs::read(path).map_err(|error| format!("read '{}': {error}", path.display()))?;
    let file = object::File::parse(&*bytes).map_err(|error| format!("parse '{}': {error}", path.display()))?;
    if file.format() != BinaryFormat::Coff || file.architecture() != Architecture::X86_64 {
        return Err(format!("'{}' is not an x86-64 COFF object", path.display()));
    }

    // MSVC often emits one COMDAT text section per function (`.text$mn`,
    // `.text$...`) rather than one literal `.text`. Merge every text-kind
    // section into the linker input while retaining per-section offsets.
    let mut text = Vec::new();
    let mut section_bases = Vec::new();
    let mut section_relocations = Vec::new();
    for section in file.sections() {
        if section.kind() != object::SectionKind::Text {
            continue;
        }
        let base = align_up(text.len(), 16);
        text.resize(base, 0x90);
        let index = section.index();
        let data = section.uncompressed_data()
            .map_err(|error| format!("read text from '{}': {error}", path.display()))?
            .into_owned();
        text.extend_from_slice(&data);
        section_bases.push((index, base));
        for (offset, relocation) in section.relocations() {
            section_relocations.push((index, base, offset, relocation));
        }
    }
    if section_bases.is_empty() {
        return Err(format!("'{}' has no executable COFF text section", path.display()));
    }
    let section_base = |index| section_bases.iter().find(|(candidate, _)| *candidate == index).map(|(_, base)| *base);

    let mut text_symbols = Vec::new();
    for symbol in file.symbols() {
        if let SymbolSection::Section(index) = symbol.section() {
            if let Some(base) = section_base(index) {
                if let Ok(name) = symbol.name() {
                    // COFF emits local section symbols such as `.text$mn`.
                    // They are relocation anchors, not linkable global names.
                    if !name.is_empty() && !name.starts_with(".text") && !name.starts_with('$') {
                        text_symbols.push((name.to_string(), base as u64 + symbol.address()));
                    }
                }
            }
        }
    }

    let mut relocations = Vec::new();
    for (_, base, offset, relocation) in section_relocations {
        let RelocationTarget::Symbol(symbol_index) = relocation.target() else {
            return Err(format!("'{}' has unsupported non-symbol relocation", path.display()));
        };
        let symbol = file.symbol_by_index(symbol_index)
            .map_err(|error| format!("read relocation symbol: {error}"))?;
        let raw_name = symbol.name()
            .map_err(|error| format!("read relocation symbol name: {error}"))?;
        let target = if raw_name.is_empty() {
            match symbol.section() {
                SymbolSection::Section(index) if section_base(index).is_some() => {
                    format!("__coff_text_section_{}", section_base(index).unwrap())
                }
                _ => return Err(format!("'{}' has unresolved anonymous COFF relocation", path.display())),
            }
        } else {
            raw_name.to_string()
        };
        relocations.push(Relocation {
            offset: base + usize::try_from(offset).map_err(|_| "relocation offset overflow")?,
            target,
            addend: relocation.addend(),
            size: relocation.size(),
            kind: relocation.kind(),
        });
    }
    Ok(InputText {
        path: path.to_path_buf(),
        text,
        rodata: Vec::new(),
        text_symbols,
        rodata_symbols: Vec::new(),
        relocations,
    })
}

fn pe_align(value: usize, alignment: usize) -> usize {
    (value + alignment - 1) & !(alignment - 1)
}

/// Phase W2 PE MVP: merge runtime-free COFF `.text` sections into a console
/// x86-64 PE executable. Runtime imports, data sections, and base relocations
/// intentionally remain unsupported until W2 section/relocation coverage grows.
fn build_kernel32_imports(imports: &[String], section_rva: u32) -> Result<(Vec<u8>, HashMap<String, u32>), String> {
    if imports.is_empty() {
        return Ok((Vec::new(), HashMap::new()));
    }
    let count = imports.len();
    let descriptor_size = 40usize; // descriptor + terminating zero descriptor
    let ilt_offset = align_up(descriptor_size, 8);
    let iat_offset = ilt_offset + (count + 1) * 8;
    let dll_offset = iat_offset + (count + 1) * 8;
    let mut data = vec![0_u8; dll_offset];
    data.extend_from_slice(b"KERNEL32.dll\0");
    while data.len() % 2 != 0 { data.push(0); }
    let mut names = HashMap::new();
    for import in imports {
        let offset = data.len();
        data.extend_from_slice(&[0, 0]); // hint
        data.extend_from_slice(import.as_bytes());
        data.push(0);
        while data.len() % 2 != 0 { data.push(0); }
        names.insert(import.clone(), offset);
    }
    let mut iat_rvas = HashMap::new();
    for (index, import) in imports.iter().enumerate() {
        let name_rva = section_rva + names[import] as u32;
        let thunk = name_rva as u64;
        let ilt = ilt_offset + index * 8;
        let iat = iat_offset + index * 8;
        data[ilt..ilt + 8].copy_from_slice(&thunk.to_le_bytes());
        data[iat..iat + 8].copy_from_slice(&thunk.to_le_bytes());
        iat_rvas.insert(format!("__imp_{import}"), section_rva + iat as u32);
    }
    // IMAGE_IMPORT_DESCRIPTOR for KERNEL32.dll.
    put_u32(&mut data, 0, section_rva + ilt_offset as u32); // OriginalFirstThunk
    put_u32(&mut data, 12, section_rva + dll_offset as u32); // Name
    put_u32(&mut data, 16, section_rva + iat_offset as u32); // FirstThunk
    Ok((data, iat_rvas))
}

/// Phase W2: merge x86-64 COFF text and emit a console PE32+ image. The
/// runtime imports are limited to KERNEL32.dll and use a generated import/IAT
/// section. This is intentionally still a narrow static-layout linker.
fn write_pe(inputs: &[PathBuf], output: &Path) -> Result<(), String> {
    if inputs.is_empty() {
        return Err("at least one input object is required".to_string());
    }
    let objects: Vec<InputText> = inputs.iter().map(|path| read_coff_input(path)).collect::<Result<_, _>>()?;
    let mut text = Vec::new();
    let mut bases = Vec::new();
    let mut symbols: HashMap<String, u64> = HashMap::new();
    let mut imports = Vec::new();
    for input in &objects {
        let base = align_up(text.len(), 16);
        text.resize(base, 0x90);
        bases.push(base);
        for (name, offset) in &input.text_symbols {
            let absolute = base as u64 + offset;
            if symbols.insert(name.clone(), absolute).is_some() {
                return Err(format!("duplicate definition of symbol '{name}'"));
            }
        }
        for relocation in &input.relocations {
            if relocation.target.starts_with("__imp_") {
                let name = relocation.target.trim_start_matches("__imp_").to_string();
                if !imports.contains(&name) { imports.push(name); }
            }
        }
        text.extend_from_slice(&input.text);
    }
    let main = *symbols.get("main").ok_or_else(|| "required symbol 'main' not found".to_string())?;
    let _lpp_main = symbols.get("lpp_main").ok_or_else(|| "required symbol 'lpp_main' not found".to_string())?;

    let headers_size = PE_FILE_ALIGNMENT;
    let text_raw_size = pe_align(text.len(), PE_FILE_ALIGNMENT);
    let idata_rva = pe_align(PE_SECTION_RVA as usize + text.len(), PE_SECTION_ALIGNMENT) as u32;
    let (idata, iat_rvas) = build_kernel32_imports(&imports, idata_rva)?;
    let has_imports = !idata.is_empty();

    for (index, input) in objects.iter().enumerate() {
        let base = bases[index];
        for relocation in &input.relocations {
            if relocation.size != 32 {
                return Err(format!("'{}': unsupported COFF relocation width {}", input.path.display(), relocation.size));
            }
            let target_rva: u32 = if relocation.target == "__self_text__" {
                PE_SECTION_RVA + base as u32
            } else if let Some(offset) = relocation.target.strip_prefix("__coff_text_section_") {
                PE_SECTION_RVA + offset.parse::<u32>().map_err(|_| "invalid COFF section relocation")?
            } else if let Some(rva) = iat_rvas.get(&relocation.target) {
                *rva
            } else {
                PE_SECTION_RVA + *symbols.get(&relocation.target).ok_or_else(|| {
                    format!("'{}': unresolved external COFF symbol '{}'", input.path.display(), relocation.target)
                })? as u32
            };
            let patch = base + relocation.offset;
            if patch + 4 > text.len() {
                return Err(format!("'{}': relocation patch out of range", input.path.display()));
            }
            let patch_rva = PE_SECTION_RVA as i64 + patch as i64;
            if relocation.kind == RelocationKind::Absolute {
                let value = PE_IMAGE_BASE as i64 + target_rva as i64 + relocation.addend;
                if value < i32::MIN as i64 || value > i32::MAX as i64 {
                    return Err("COFF absolute relocation out of range; PE base relocations are not implemented".to_string());
                }
                text[patch..patch + 4].copy_from_slice(&(value as i32).to_le_bytes());
            } else {
                let displacement = target_rva as i64 + relocation.addend - patch_rva;
                if displacement < i32::MIN as i64 || displacement > i32::MAX as i64 {
                    return Err("COFF PC-relative relocation out of range".to_string());
                }
                text[patch..patch + 4].copy_from_slice(&(displacement as i32).to_le_bytes());
            }
        }
    }

    let idata_raw_size = if has_imports { pe_align(idata.len(), PE_FILE_ALIGNMENT) } else { 0 };
    let idata_raw_offset = headers_size + text_raw_size;
    let section_count = if has_imports { 2 } else { 1 };
    let image_end = if has_imports {
        idata_rva as usize + idata.len()
    } else {
        PE_SECTION_RVA as usize + text.len()
    };
    let image_size = pe_align(image_end, PE_SECTION_ALIGNMENT);
    let mut pe = vec![0_u8; headers_size + text_raw_size + idata_raw_size];
    pe[0..2].copy_from_slice(b"MZ");
    put_u32(&mut pe, 0x3c, 0x80);
    let nt = 0x80;
    pe[nt..nt + 4].copy_from_slice(b"PE\0\0");
    put_u16(&mut pe, nt + 4, 0x8664);
    put_u16(&mut pe, nt + 6, section_count);
    put_u16(&mut pe, nt + 20, 0xF0);
    put_u16(&mut pe, nt + 22, 0x0022);
    let opt = nt + 24;
    put_u16(&mut pe, opt, 0x20b);
    put_u32(&mut pe, opt + 4, text_raw_size as u32);
    put_u32(&mut pe, opt + 8, idata_raw_size as u32);
    put_u32(&mut pe, opt + 16, PE_SECTION_RVA + main as u32);
    put_u32(&mut pe, opt + 20, PE_SECTION_RVA);
    put_u64(&mut pe, opt + 24, PE_IMAGE_BASE);
    put_u32(&mut pe, opt + 32, PE_SECTION_ALIGNMENT as u32);
    put_u32(&mut pe, opt + 36, PE_FILE_ALIGNMENT as u32);
    put_u16(&mut pe, opt + 40, 6);
    put_u16(&mut pe, opt + 48, 6);
    put_u32(&mut pe, opt + 56, image_size as u32);
    put_u32(&mut pe, opt + 60, headers_size as u32);
    put_u16(&mut pe, opt + 68, 3); // console
    put_u64(&mut pe, opt + 72, 0x100000);
    put_u64(&mut pe, opt + 80, 0x1000);
    put_u64(&mut pe, opt + 88, 0x100000);
    put_u64(&mut pe, opt + 96, 0x1000);
    put_u32(&mut pe, opt + 108, 16);
    if has_imports {
        let directories = opt + 112;
        put_u32(&mut pe, directories + 8, idata_rva); // import table directory #1
        put_u32(&mut pe, directories + 12, 40);       // descriptor + terminator
        let first_iat_rva = iat_rvas.values().copied().min().unwrap_or(idata_rva);
        put_u32(&mut pe, directories + 12 * 8, first_iat_rva); // IAT directory #12
        put_u32(&mut pe, directories + 12 * 8 + 4, (imports.len() as u32 + 1) * 8);
    }
    let section = opt + 0xF0;
    pe[section..section + 5].copy_from_slice(b".text");
    put_u32(&mut pe, section + 8, text.len() as u32);
    put_u32(&mut pe, section + 12, PE_SECTION_RVA);
    put_u32(&mut pe, section + 16, text_raw_size as u32);
    put_u32(&mut pe, section + 20, headers_size as u32);
    put_u32(&mut pe, section + 36, 0x60000020);
    if has_imports {
        let idata_section = section + 40;
        pe[idata_section..idata_section + 6].copy_from_slice(b".idata");
        put_u32(&mut pe, idata_section + 8, idata.len() as u32);
        put_u32(&mut pe, idata_section + 12, idata_rva);
        put_u32(&mut pe, idata_section + 16, idata_raw_size as u32);
        put_u32(&mut pe, idata_section + 20, idata_raw_offset as u32);
        put_u32(&mut pe, idata_section + 36, 0xC0000040);
    }
    pe[headers_size..headers_size + text.len()].copy_from_slice(&text);
    if has_imports {
        pe[idata_raw_offset..idata_raw_offset + idata.len()].copy_from_slice(&idata);
    }
    fs::write(output, pe).map_err(|error| format!("write '{}': {error}", output.display()))?;
    Ok(())
}

fn inspect_object(input: &Path) -> Result<(), String> {
    let bytes = fs::read(input).map_err(|error| format!("read '{}': {error}", input.display()))?;
    let file = object::File::parse(&*bytes).map_err(|error| format!("parse '{}': {error}", input.display()))?;
    let mut relocations = 0usize;
    let mut relocation_kinds: BTreeMap<String, usize> = BTreeMap::new();
    println!("format: {:?}", file.format());
    println!("architecture: {:?}", file.architecture());
    println!("sections:");
    for section in file.sections() {
        for (_, relocation) in section.relocations() {
            relocations += 1;
            *relocation_kinds.entry(format!("{:?}", relocation.kind())).or_default() += 1;
        }
        println!("  {} size={} kind={:?}", section.name().unwrap_or("<unnamed>"), section.size(), section.kind());
    }
    let defined = file.symbols().filter(|symbol| !symbol.is_undefined()).count();
    let undefined = file.symbols().filter(|symbol| symbol.is_undefined()).count();
    println!("symbols: defined={} undefined={}", defined, undefined);
    println!("relocations: {}", relocations);
    println!("relocation-kinds:");
    for (kind, count) in relocation_kinds {
        println!("  {}={}", kind, count);
    }
    Ok(())
}

fn usage() {
    eprintln!("Usage: lpp-link <program.o> [runtime.o ...] -o <output>\n       lpp-link pe <program.obj> [runtime.obj ...] -o <output.exe>");
    eprintln!("       lpp-link inspect <object.o>");
    eprintln!("Phase 2: direct Linux x86-64 ELF linker; Windows W1 COFF inspection.");
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("inspect") {
        if args.len() != 2 {
            usage();
            std::process::exit(2);
        }
        if let Err(error) = inspect_object(Path::new(&args[1])) {
            eprintln!("lpp-link inspect error: {error}");
            std::process::exit(1);
        }
        return;
    }
    let pe_mode = args.first().map(String::as_str) == Some("pe");
    let offset = if pe_mode { 1 } else { 0 };
    let Some(output_relative) = args[offset..].iter().position(|arg| arg == "-o") else {
        usage();
        std::process::exit(2);
    };
    let output_index = offset + output_relative;
    if output_index == offset || output_index + 2 != args.len() {
        usage();
        std::process::exit(2);
    }
    let inputs = args[offset..output_index].iter().map(PathBuf::from).collect::<Vec<_>>();
    let result = if pe_mode {
        write_pe(&inputs, Path::new(&args[output_index + 1]))
    } else {
        write_elf(&inputs, Path::new(&args[output_index + 1]))
    };
    if let Err(error) = result {
        eprintln!("lpp-link error: {error}");
        std::process::exit(1);
    }
}
