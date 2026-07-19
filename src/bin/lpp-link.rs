//! `lpp-link` Phase 2 MVP: direct Linux x86-64 ELF executable emission.
//!
//! Scope is intentionally narrow and explicit:
//! - one Cranelift ELF relocatable object;
//! - a `.text` section only;
//! - internal 32-bit PC-relative calls only;
//! - no runtime or libc imports.
//!
//! This is a real direct executable writer used for linker bring-up tests, not
//! a replacement for the host-link path yet.

use object::{Architecture, BinaryFormat, Object, ObjectSection, ObjectSymbol, RelocationTarget};
use std::env;
use std::fs;
use std::path::Path;

const ELF_BASE: u64 = 0x400000;
const CODE_OFFSET: usize = 0x1000;
const EM_X86_64: u16 = 62;
const PT_LOAD: u32 = 1;
const PF_R_X: u32 = 5;

fn put_u16(buf: &mut [u8], offset: usize, value: u16) {
    buf[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
fn put_u32(buf: &mut [u8], offset: usize, value: u32) {
    buf[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn put_u64(buf: &mut [u8], offset: usize, value: u64) {
    buf[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn symbol_offset(file: &object::File<'_>, name: &str) -> Result<u64, String> {
    file.symbols()
        .find(|symbol| symbol.name().ok() == Some(name))
        .map(|symbol| symbol.address())
        .ok_or_else(|| format!("required symbol '{name}' not found"))
}

fn write_elf(input: &Path, output: &Path) -> Result<(), String> {
    let bytes = fs::read(input).map_err(|error| format!("read '{}': {error}", input.display()))?;
    let file = object::File::parse(&*bytes).map_err(|error| format!("parse object: {error}"))?;
    if file.format() != BinaryFormat::Elf || file.architecture() != Architecture::X86_64 {
        return Err("Phase 2 MVP accepts only x86-64 ELF relocatable objects".to_string());
    }

    let text_section = file
        .section_by_name(".text")
        .ok_or_else(|| "object has no .text section".to_string())?;
    let mut text = text_section
        .uncompressed_data()
        .map_err(|error| format!("read .text: {error}"))?
        .into_owned();
    let lpp_main = symbol_offset(&file, "lpp_main")?;
    let main = symbol_offset(&file, "main")?;

    // Resolve the Cranelift wrapper's internal call(s), e.g. main -> lpp_main.
    for (offset, relocation) in text_section.relocations() {
        if relocation.size() != 32 {
            return Err(format!("unsupported relocation width {}", relocation.size()));
        }
        let RelocationTarget::Symbol(symbol_index) = relocation.target() else {
            return Err("unsupported non-symbol relocation".to_string());
        };
        let symbol = file
            .symbol_by_index(symbol_index)
            .map_err(|error| format!("read relocation symbol: {error}"))?;
        let target_name = symbol.name().map_err(|error| format!("read symbol name: {error}"))?;
        let target = match target_name {
            "lpp_main" => lpp_main,
            "main" => main,
            other => return Err(format!("unresolved external relocation to '{other}'")),
        };
        let place = offset as i64;
        let displacement = target as i64 + relocation.addend() - place;
        let patch = usize::try_from(offset).map_err(|_| "relocation offset overflow")?;
        if patch + 4 > text.len() || displacement < i32::MIN as i64 || displacement > i32::MAX as i64 {
            return Err("PC-relative relocation is out of range".to_string());
        }
        text[patch..patch + 4].copy_from_slice(&(displacement as i32).to_le_bytes());
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
    eprintln!("Usage: lpp-link <input.o> -o <output>");
    eprintln!("Phase 2 MVP: direct Linux x86-64 ELF for runtime-free L++ objects.");
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() != 3 || args[1] != "-o" {
        usage();
        std::process::exit(2);
    }
    if let Err(error) = write_elf(Path::new(&args[0]), Path::new(&args[2])) {
        eprintln!("lpp-link error: {error}");
        std::process::exit(1);
    }
}
