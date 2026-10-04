//! Emit a PE and a Mach-O executable image from synthesized objects and print
//! their paths, for manual inspection with `file` / `objdump`.
use object::write::{Object as WriteObject, StandardSection, Symbol, SymbolSection};
use object::{Architecture, BinaryFormat, Endianness, SymbolFlags, SymbolKind, SymbolScope};
use lpp_linker::{link_typed, LinkOptions, Machine, OutputFormat, PeSubsystem};
use std::path::PathBuf;

fn minimal(format: BinaryFormat) -> Vec<u8> {
    let mut obj = WriteObject::new(format, Architecture::X86_64, Endianness::Little);
    let text = obj.section_id(StandardSection::Text);
    let code = [0x31u8, 0xc0, 0xc3];
    let off = obj.append_section_data(text, &code, 1);
    obj.add_symbol(Symbol { name: b"main".to_vec(), value: off, size: code.len() as u64,
        kind: SymbolKind::Text, scope: SymbolScope::Linkage, weak: false,
        section: SymbolSection::Section(text), flags: SymbolFlags::None });
    obj.write().unwrap()
}

fn main() {
    let d = PathBuf::from("/tmp/xf");
    std::fs::create_dir_all(&d).unwrap();
    // PE
    std::fs::write(d.join("in.obj"), minimal(BinaryFormat::Coff)).unwrap();
    let mut po = LinkOptions::default();
    po.format = Some(OutputFormat::Pe); po.machine = Some(Machine::X86_64); po.subsystem = Some(PeSubsystem::Console);
    let r = link_typed(&[d.join("in.obj")], &d.join("hello.exe"), &po).unwrap();
    println!("PE   -> {} ({} bytes)", d.join("hello.exe").display(), r.output_size);
    // Mach-O
    std::fs::write(d.join("in.o"), minimal(BinaryFormat::MachO)).unwrap();
    let mut mo = LinkOptions::default();
    mo.format = Some(OutputFormat::Macho); mo.machine = Some(Machine::X86_64);
    let r = link_typed(&[d.join("in.o")], &d.join("hello.macho"), &mo).unwrap();
    println!("MachO-> {} ({} bytes)", d.join("hello.macho").display(), r.output_size);
}
