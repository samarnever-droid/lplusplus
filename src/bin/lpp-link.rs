//! `lpp-link` Phase 2: direct Linux x86-64 ELF executable emission.
//!
//! The linker deliberately grows in small verified slices. It currently merges
//! `.text` from one or more x86-64 ELF objects and resolves internal 32-bit
//! PC-relative relocations. This is sufficient for Cranelift objects plus the
//! freestanding `lpp_runtime_min.o` print runtime, without invoking a host
//! compiler or linker during the final link step.

use object::{Architecture, BinaryFormat, Object, ObjectSection, ObjectSymbol, RelocationKind, RelocationTarget, SymbolSection};
use std::collections::HashMap;
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
    symbols: Vec<(String, u64)>,
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

    let mut symbols = Vec::new();
    for symbol in file.symbols() {
        if symbol.section() == SymbolSection::Section(text_index) {
            if let Ok(name) = symbol.name() {
                if !name.is_empty() {
                    symbols.push((name.to_string(), symbol.address()));
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
        let target = symbol.name()
            .map_err(|error| format!("read relocation symbol name: {error}"))?
            .to_string();
        relocations.push(Relocation {
            offset: usize::try_from(offset).map_err(|_| "relocation offset overflow")?,
            target,
            addend: relocation.addend(),
            size: relocation.size(),
            kind: relocation.kind(),
        });
    }
    Ok(InputText { path: path.to_path_buf(), text, symbols, relocations })
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
        for (name, offset) in &input.symbols {
            let absolute = u64::try_from(base).map_err(|_| "text offset overflow")? + offset;
            if symbols.insert(name.clone(), absolute).is_some() {
                return Err(format!("duplicate definition of symbol '{name}'"));
            }
        }
        text.extend_from_slice(&input.text);
    }
    let _lpp_main = *symbols.get("lpp_main").ok_or_else(|| "required symbol 'lpp_main' not found".to_string())?;
    let main = *symbols.get("main").ok_or_else(|| "required symbol 'main' not found".to_string())?;

    // PIC Cranelift imports runtime functions through GOTPCREL relocations.
    // Build a tiny read-only GOT immediately after .text; all entries point to
    // symbols supplied by one of the input objects.
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
                _ => *symbols.get(&relocation.target).ok_or_else(|| {
                    format!("'{}': unresolved external relocation to '{}'", input.path.display(), relocation.target)
                })?,
            };
            let patch = base + relocation.offset;
            let displacement = target as i64 + relocation.addend - patch as i64;
            if patch + 4 > text.len() || displacement < i32::MIN as i64 || displacement > i32::MAX as i64 {
                return Err(format!("'{}': PC-relative relocation out of range", input.path.display()));
            }
            text[patch..patch + 4].copy_from_slice(&(displacement as i32).to_le_bytes());
        }
    }

    // Linux `_start`: align stack, call C ABI main, exit(main_status) via syscall.
    let start_offset = text.len();
    let main_address = ELF_BASE + CODE_OFFSET as u64 + main;
    let call_next = ELF_BASE + CODE_OFFSET as u64 + start_offset as u64 + 11;
    let call_displacement = main_address as i64 - call_next as i64;
    if call_displacement < i32::MIN as i64 || call_displacement > i32::MAX as i64 {
        return Err("main is out of range for startup call".to_string());
    }
    let mut start = vec![
        0x31, 0xed,                         // xor ebp, ebp
        0x48, 0x83, 0xe4, 0xf0,             // and rsp, -16
        0xe8, 0, 0, 0, 0,                   // call main
        0x89, 0xc7,                         // mov edi, eax
        0xb8, 60, 0, 0, 0,                  // mov eax, 60 (exit)
        0x0f, 0x05,                         // syscall
    ];
    start[7..11].copy_from_slice(&(call_displacement as i32).to_le_bytes());

    let file_size = CODE_OFFSET + text.len() + start.len();
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
    elf[CODE_OFFSET + text.len()..].copy_from_slice(&start);
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

fn usage() {
    eprintln!("Usage: lpp-link <program.o> [runtime.o ...] -o <output>");
    eprintln!("Phase 2: direct Linux x86-64 ELF linker for internal .text relocations.");
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(output_index) = args.iter().position(|arg| arg == "-o") else {
        usage();
        std::process::exit(2);
    };
    if output_index == 0 || output_index + 2 != args.len() {
        usage();
        std::process::exit(2);
    }
    let inputs = args[..output_index].iter().map(PathBuf::from).collect::<Vec<_>>();
    if let Err(error) = write_elf(&inputs, Path::new(&args[output_index + 1])) {
        eprintln!("lpp-link error: {error}");
        std::process::exit(1);
    }
}
