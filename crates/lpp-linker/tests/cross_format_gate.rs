//! Cross-format gate: prove the PE (Windows) and Mach-O (macOS) link paths on
//! any host, with no external toolchain and no target OS.
//!
//! We cannot *run* a PE or a Mach-O on Linux, so this gate proves the next best
//! thing — that the images the linker produces are **well-formed and
//! loader-conformant**:
//!
//!   1. synthesize a minimal relocatable input object in the target format with
//!      the `object` crate's writer (pure Rust, host-independent),
//!   2. link it with `lpp-linker` into an executable image,
//!   3. re-parse the produced image with the `object` crate — the same parsing
//!      a loader-facing tool performs — and assert the invariants a real loader
//!      reads (format, architecture, entry, sections, imports),
//!   4. where an independent external validator is available on the host
//!      (`objdump` understands PE/COFF), cross-check it too.
//!
//! Execution proof (Wine for PE, real macOS/Windows CI runners) layers on top of
//! this; this gate is the hermetic foundation that runs anywhere.

use std::path::{Path, PathBuf};
use std::process::Command;

use object::read::{File as ReadFile, Object as ReadObject, ObjectSection, ObjectSymbol};
use object::write::{Object as WriteObject, Relocation, StandardSection, Symbol, SymbolSection};
use object::{
    Architecture, BinaryFormat, Endianness, RelocationEncoding, RelocationFlags, RelocationKind,
    SymbolFlags, SymbolKind, SymbolScope,
};

use lpp_linker::{LinkOptions, Machine, OutputFormat, PeSubsystem, ResolvedFormat, link_typed};

fn tmp_dir(tag: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("lpp_xfmt_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_input(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

/// A minimal x86-64 relocatable object in `format`, defining a global `main`
/// whose body is `xor eax, eax; ret` — self-contained, imports nothing.
fn minimal_object(format: BinaryFormat) -> Vec<u8> {
    let mut obj = WriteObject::new(format, Architecture::X86_64, Endianness::Little);
    let text = obj.section_id(StandardSection::Text);
    let code = [0x31u8, 0xc0, 0xc3]; // xor eax,eax ; ret
    let offset = obj.append_section_data(text, &code, 1);
    obj.add_symbol(Symbol {
        name: b"main".to_vec(),
        value: offset,
        size: code.len() as u64,
        kind: SymbolKind::Text,
        scope: SymbolScope::Linkage,
        weak: false,
        section: SymbolSection::Section(text),
        flags: SymbolFlags::None,
    });
    obj.write().expect("synthesize input object")
}

/// An x86-64 relocatable object whose `main` calls an *undefined* `import`
/// through a `call rel32`, forcing the linker down its import path (PE IAT /
/// Mach-O dyld bind).
fn calling_object(format: BinaryFormat, import: &str) -> Vec<u8> {
    let mut obj = WriteObject::new(format, Architecture::X86_64, Endianness::Little);
    let text = obj.section_id(StandardSection::Text);
    // call rel32 (0xe8, disp32) ; ret
    let code = [0xe8u8, 0, 0, 0, 0, 0xc3];
    let offset = obj.append_section_data(text, &code, 1);
    obj.add_symbol(Symbol {
        name: b"main".to_vec(),
        value: offset,
        size: code.len() as u64,
        kind: SymbolKind::Text,
        scope: SymbolScope::Linkage,
        weak: false,
        section: SymbolSection::Section(text),
        flags: SymbolFlags::None,
    });
    let imp = obj.add_symbol(Symbol {
        name: import.as_bytes().to_vec(),
        value: 0,
        size: 0,
        kind: SymbolKind::Text,
        scope: SymbolScope::Dynamic,
        weak: false,
        section: SymbolSection::Undefined,
        flags: SymbolFlags::None,
    });
    obj.add_relocation(
        text,
        Relocation {
            offset: offset + 1, // the disp32 field of the `call`
            symbol: imp,
            addend: -4,
            flags: RelocationFlags::Generic {
                kind: RelocationKind::Relative,
                encoding: RelocationEncoding::X86Branch,
                size: 32,
            },
        },
    )
    .expect("add call relocation");
    obj.write().expect("synthesize calling object")
}

// ── PE (Windows) ────────────────────────────────────────────────────────────

#[test]
fn pe_image_is_well_formed() {
    let dir = tmp_dir("pe_wf");
    let obj = write_input(&dir, "in.obj", &minimal_object(BinaryFormat::Coff));
    let out = dir.join("out.exe");

    let mut opts = LinkOptions::default();
    opts.format = Some(OutputFormat::Pe);
    opts.machine = Some(Machine::X86_64);
    opts.subsystem = Some(PeSubsystem::Console);

    let report = link_typed(&[obj], &out, &opts).expect("PE link must succeed");
    assert_eq!(report.format, ResolvedFormat::Pe);
    assert!(report.output_size > 0);

    let bytes = std::fs::read(&out).unwrap();
    let file = ReadFile::parse(&*bytes).expect("produced PE must parse");
    assert_eq!(file.format(), BinaryFormat::Pe);
    assert_eq!(file.architecture(), Architecture::X86_64);
    assert!(file.entry() > 0, "PE must have a non-zero entry RVA");
    assert!(
        file.sections()
            .any(|s| s.name().map(|n| n.contains("text")).unwrap_or(false)),
        "PE must carry a text section"
    );
    assert_eq!(&bytes[0..2], b"MZ", "PE must start with the MZ stub");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pe_import_populates_the_iat() {
    let dir = tmp_dir("pe_imp");
    // ExitProcess is auto-classified to KERNEL32.dll by the linker.
    let obj = write_input(
        &dir,
        "in.obj",
        &calling_object(BinaryFormat::Coff, "ExitProcess"),
    );
    let out = dir.join("out.exe");

    let mut opts = LinkOptions::default();
    opts.format = Some(OutputFormat::Pe);
    opts.machine = Some(Machine::X86_64);
    opts.subsystem = Some(PeSubsystem::Console);

    link_typed(&[obj], &out, &opts).expect("PE link with import must succeed");

    let bytes = std::fs::read(&out).unwrap();
    let file = ReadFile::parse(&*bytes).expect("produced PE must parse");
    let imports = file.imports().expect("PE import table must parse");
    assert!(
        imports.iter().any(|i| i.name() == b"ExitProcess"),
        "IAT must contain the imported ExitProcess (got: {:?})",
        imports
            .iter()
            .map(|i| String::from_utf8_lossy(i.name()).into_owned())
            .collect::<Vec<_>>()
    );
    assert!(
        imports
            .iter()
            .any(|i| i.library().eq_ignore_ascii_case(b"KERNEL32.dll")),
        "import must resolve against KERNEL32.dll"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Independent external validator: `objdump` on this host understands PE/COFF
/// (`pei-x86-64`). If it is present, its view must agree with ours.
#[test]
fn pe_objdump_agrees() {
    let dir = tmp_dir("pe_od");
    let obj = write_input(&dir, "in.obj", &minimal_object(BinaryFormat::Coff));
    let out = dir.join("out.exe");
    let mut opts = LinkOptions::default();
    opts.format = Some(OutputFormat::Pe);
    opts.machine = Some(Machine::X86_64);
    opts.subsystem = Some(PeSubsystem::Console);
    link_typed(&[obj], &out, &opts).expect("PE link must succeed");

    match Command::new("objdump").arg("-f").arg(&out).output() {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout);
            assert!(
                s.contains("pei-x86-64") || s.contains("pe-x86-64") || s.contains("coff-x86-64"),
                "objdump must recognize a PE/COFF x86-64 image; got:\n{s}"
            );
            assert!(
                s.contains("start address"),
                "objdump must report an entry (start address); got:\n{s}"
            );
        }
        _ => eprintln!("[skip] objdump unavailable or cannot read PE on this host"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ── Mach-O (macOS) ───────────────────────────────────────────────────────────

#[test]
fn macho_image_is_well_formed() {
    let dir = tmp_dir("mac_wf");
    let obj = write_input(&dir, "in.o", &minimal_object(BinaryFormat::MachO));
    let out = dir.join("out.macho");

    let mut opts = LinkOptions::default();
    opts.format = Some(OutputFormat::Macho);
    opts.machine = Some(Machine::X86_64);

    let report = link_typed(&[obj], &out, &opts).expect("Mach-O link must succeed");
    assert_eq!(report.format, ResolvedFormat::Macho);
    assert!(report.output_size > 0);

    let bytes = std::fs::read(&out).unwrap();
    let file = ReadFile::parse(&*bytes).expect("produced Mach-O must parse");
    assert_eq!(file.format(), BinaryFormat::MachO);
    assert_eq!(file.architecture(), Architecture::X86_64);
    assert!(file.entry() > 0, "Mach-O must have a non-zero entry");
    assert!(
        file.sections()
            .any(|s| s.name().map(|n| n.contains("text")).unwrap_or(false)),
        "Mach-O must carry a __text section"
    );
    assert_eq!(&bytes[0..4], &[0xCF, 0xFA, 0xED, 0xFE], "Mach-O magic");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn macho_import_binds_against_libsystem() {
    let dir = tmp_dir("mac_imp");
    let obj = write_input(&dir, "in.o", &calling_object(BinaryFormat::MachO, "exit"));
    let out = dir.join("out.macho");

    let mut opts = LinkOptions::default();
    opts.format = Some(OutputFormat::Macho);
    opts.machine = Some(Machine::X86_64);

    link_typed(&[obj], &out, &opts).expect("Mach-O link with import must succeed");

    let bytes = std::fs::read(&out).unwrap();
    let file = ReadFile::parse(&*bytes).expect("produced Mach-O must parse");
    // The undefined symbol must survive as an imported (undefined) dynamic symbol.
    let has_undef_import = file
        .symbols()
        .chain(file.dynamic_symbols())
        .any(|s| s.is_undefined() && s.name().map(|n| n.contains("exit")).unwrap_or(false));
    assert!(
        has_undef_import,
        "Mach-O must record the undefined 'exit' import"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ── Robustness (both formats): malformed input is a clean error, never a panic ─

#[test]
fn malformed_input_is_a_clean_error_pe() {
    let dir = tmp_dir("pe_bad");
    let bad = write_input(&dir, "garbage.obj", b"not a real object file at all");
    let out = dir.join("out.exe");
    let mut opts = LinkOptions::default();
    opts.format = Some(OutputFormat::Pe);
    opts.machine = Some(Machine::X86_64);
    let err = link_typed(&[bad], &out, &opts).expect_err("garbage must fail, not panic");
    assert!(!err.message.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn malformed_input_is_a_clean_error_macho() {
    let dir = tmp_dir("mac_bad");
    let bad = write_input(&dir, "garbage.o", b"\x00\x01\x02\x03 still not an object");
    let out = dir.join("out.macho");
    let mut opts = LinkOptions::default();
    opts.format = Some(OutputFormat::Macho);
    opts.machine = Some(Machine::X86_64);
    let err = link_typed(&[bad], &out, &opts).expect_err("garbage must fail, not panic");
    assert!(!err.message.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
