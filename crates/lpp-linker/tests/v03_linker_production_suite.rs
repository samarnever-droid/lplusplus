//! Comprehensive v0.3 production integration test suite for `lpp-linker`.
//!
//! Exercises ELF64 (`x86_64`, `aarch64`), PE/COFF (`x86_64`, `aarch64`),
//! Mach-O (`x86_64`, `aarch64`), bare-metal static ELF kernels (`.multiboot2`
//! placement, `freestanding_arch_object`, custom linker image bases),
//! multi-object relocation resolution, symbol visibility/binding rules, static
//! archives, and negative error paths — all hermetically via pure-Rust
//! `object::write` synthesis and `object::read` + raw binary verification.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use object::read::{File as ReadFile, Object as ReadObject, ObjectSection, ObjectSymbol};
use object::write::{Object as WriteObject, Relocation, StandardSection, Symbol, SymbolSection};
use object::{
    Architecture, BinaryFormat, Endianness, RelocationEncoding, RelocationFlags, RelocationKind,
    SectionKind, SymbolFlags, SymbolKind, SymbolScope, elf,
};

use lpp_linker::{
    DynamicMode, LinkErrorKind, LinkOptions, Machine, OutputFormat, PeSubsystem, ResolvedFormat,
    expand_response_files, freestanding_arch_object, inspect_object, link_cli, link_direct,
    link_typed, link_with_options, multiboot2_header_object, resolve_format, sniff_format,
};

// ── Test Workspace & Binary Parsing Helpers ─────────────────────────────────

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "lpp_v03_linker_{tag}_{}_{}",
            std::process::id(),
            id
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp test dir");
        Self { path }
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let p = self.path.join(name);
        std::fs::write(&p, bytes).expect("write test input");
        p
    }

    fn out(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn le_u16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(b[off..off + 2].try_into().unwrap())
}

fn le_u32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}

fn le_i32(b: &[u8], off: usize) -> i32 {
    i32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}

fn le_u64(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().unwrap())
}

#[derive(Debug, Clone, Copy)]
struct ElfHdr {
    e_type: u16,
    e_machine: u16,
    e_entry: u64,
    e_phoff: u64,
    e_shoff: u64,
    e_phnum: u16,
    e_shnum: u16,
    e_shstrndx: u16,
}

#[derive(Debug, Clone, Copy)]
struct ElfPhdr {
    p_type: u32,
    p_flags: u32,
    p_offset: u64,
    p_vaddr: u64,
    p_paddr: u64,
    p_filesz: u64,
    p_memsz: u64,
    p_align: u64,
}

#[derive(Debug, Clone)]
struct ElfShdr {
    name: String,
    sh_type: u32,
    sh_flags: u64,
    sh_addr: u64,
    sh_offset: u64,
    sh_size: u64,
    sh_link: u32,
}

fn parse_elf_hdr(b: &[u8]) -> ElfHdr {
    assert!(b.len() >= 64, "ELF header truncated");
    assert_eq!(&b[0..4], b"\x7fELF", "ELF magic");
    assert_eq!(b[4], 2, "ELFCLASS64");
    assert_eq!(b[5], 1, "ELFDATA2LSB");
    ElfHdr {
        e_type: le_u16(b, 16),
        e_machine: le_u16(b, 18),
        e_entry: le_u64(b, 24),
        e_phoff: le_u64(b, 32),
        e_shoff: le_u64(b, 40),
        e_phnum: le_u16(b, 56),
        e_shnum: le_u16(b, 60),
        e_shstrndx: le_u16(b, 62),
    }
}

fn parse_elf_phdrs(b: &[u8], h: &ElfHdr) -> Vec<ElfPhdr> {
    let mut out = Vec::with_capacity(h.e_phnum as usize);
    for i in 0..h.e_phnum as usize {
        let o = h.e_phoff as usize + i * 56;
        out.push(ElfPhdr {
            p_type: le_u32(b, o),
            p_flags: le_u32(b, o + 4),
            p_offset: le_u64(b, o + 8),
            p_vaddr: le_u64(b, o + 16),
            p_paddr: le_u64(b, o + 24),
            p_filesz: le_u64(b, o + 32),
            p_memsz: le_u64(b, o + 40),
            p_align: le_u64(b, o + 48),
        });
    }
    out
}

fn parse_elf_shdrs(b: &[u8], h: &ElfHdr) -> Vec<ElfShdr> {
    let strtab_sh_off = h.e_shoff as usize + (h.e_shstrndx as usize) * 64;
    let strtab_off = le_u64(b, strtab_sh_off + 24) as usize;
    let strtab_size = le_u64(b, strtab_sh_off + 32) as usize;
    let shstrtab = &b[strtab_off..strtab_off + strtab_size];

    let mut out = Vec::with_capacity(h.e_shnum as usize);
    for i in 0..h.e_shnum as usize {
        let o = h.e_shoff as usize + i * 64;
        let name_idx = le_u32(b, o) as usize;
        let end = shstrtab[name_idx..]
            .iter()
            .position(|&c| c == 0)
            .map(|p| name_idx + p)
            .unwrap_or(shstrtab.len());
        let name = String::from_utf8_lossy(&shstrtab[name_idx..end]).into_owned();
        out.push(ElfShdr {
            name,
            sh_type: le_u32(b, o + 4),
            sh_flags: le_u64(b, o + 8),
            sh_addr: le_u64(b, o + 16),
            sh_offset: le_u64(b, o + 24),
            sh_size: le_u64(b, o + 32),
            sh_link: le_u32(b, o + 40),
        });
    }
    out
}

fn parse_elf_dynamic_tags(b: &[u8], dyn_sh: &ElfShdr) -> Vec<(i64, u64)> {
    let mut tags = Vec::new();
    let start = dyn_sh.sh_offset as usize;
    let end = start + dyn_sh.sh_size as usize;
    let mut pos = start;
    while pos + 16 <= end {
        let tag = le_u64(b, pos) as i64;
        let val = le_u64(b, pos + 8);
        tags.push((tag, val));
        if tag == 0 {
            break;
        }
        pos += 16;
    }
    tags
}

fn read_cstr(b: &[u8], off: usize) -> String {
    let end = b[off..]
        .iter()
        .position(|&c| c == 0)
        .map(|p| off + p)
        .unwrap_or(b.len());
    String::from_utf8_lossy(&b[off..end]).into_owned()
}

fn add_sym(
    obj: &mut WriteObject<'_>,
    name: &[u8],
    sec: SymbolSection,
    value: u64,
    size: u64,
    kind: SymbolKind,
    scope: SymbolScope,
    weak: bool,
) -> object::write::SymbolId {
    obj.add_symbol(Symbol {
        name: name.to_vec(),
        value,
        size,
        kind,
        scope,
        weak,
        section: sec,
        flags: SymbolFlags::None,
    })
}

fn undef_sym(obj: &mut WriteObject<'_>, name: &[u8], kind: SymbolKind) -> object::write::SymbolId {
    add_sym(
        obj,
        name,
        SymbolSection::Undefined,
        0,
        0,
        kind,
        SymbolScope::Dynamic,
        false,
    )
}

fn build_ar_archive(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = b"!<arch>\n".to_vec();
    for (name, data) in members {
        let mut hdr = [b' '; 60];
        let tag = format!("{name}/");
        let n = tag.len().min(16);
        hdr[..n].copy_from_slice(&tag.as_bytes()[..n]);
        // mtime=0, uid=0, gid=0, mode=100644
        hdr[16..17].copy_from_slice(b"0");
        hdr[28..29].copy_from_slice(b"0");
        hdr[34..35].copy_from_slice(b"0");
        hdr[40..46].copy_from_slice(b"100644");
        let sz = format!("{}", data.len());
        hdr[48..48 + sz.len()].copy_from_slice(sz.as_bytes());
        hdr[58..60].copy_from_slice(b"`\n");
        out.extend_from_slice(&hdr);
        out.extend_from_slice(data);
        if data.len() % 2 == 1 {
            out.push(b'\n');
        }
    }
    out
}

/// Execute a linked Linux ELF binary and return its exit code (retrying ETXTBSY).
fn exec_linux_binary(path: &Path) -> i32 {
    assert!(cfg!(target_os = "linux"), "Linux-only execution helper");
    let mut attempts = 0;
    let status = loop {
        match Command::new(path).status() {
            Ok(s) => break s,
            Err(e) if e.raw_os_error() == Some(26) && attempts < 10 => {
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Err(e) => panic!("failed to execute {}: {e}", path.display()),
        }
    };
    status.code().expect("process must exit normally")
}

/// Build an x86_64 ELF object defining `_start` that exits with `exit_code` (0..=255).
fn make_x64_exit_start_obj(exit_code: u8) -> Vec<u8> {
    let mut obj = WriteObject::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let text = obj.section_id(StandardSection::Text);
    // mov eax, 60 ; mov edi, exit_code ; syscall
    let code = [
        0xb8, 60, 0, 0, 0, // mov eax, 60
        0xbf, exit_code, 0, 0, 0, // mov edi, exit_code
        0x0f, 0x05, // syscall
    ];
    let off = obj.append_section_data(text, &code, 16);
    add_sym(
        &mut obj,
        b"_start",
        SymbolSection::Section(text),
        off,
        code.len() as u64,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    obj.write().unwrap()
}

/// Build an aarch64 ELF object defining `_start` that exits with `exit_code`.
fn make_a64_exit_start_obj(exit_code: u16) -> Vec<u8> {
    let mut obj = WriteObject::new(BinaryFormat::Elf, Architecture::Aarch64, Endianness::Little);
    let text = obj.section_id(StandardSection::Text);
    // mov x0, #exit_code; mov x8, #93; svc #0
    let mov_x0 = 0xd280_0000u32 | ((exit_code as u32) << 5);
    let mov_x8 = 0xd280_0008u32 | (93u32 << 5);
    let svc_0 = 0xd400_0001u32;
    let mut code = Vec::new();
    for insn in [mov_x0, mov_x8, svc_0] {
        code.extend_from_slice(&insn.to_le_bytes());
    }
    let off = obj.append_section_data(text, &code, 16);
    add_sym(
        &mut obj,
        b"_start",
        SymbolSection::Section(text),
        off,
        code.len() as u64,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    obj.write().unwrap()
}

/// Build a minimal relocatable object with `main` returning 0 in any format/arch.
fn make_minimal_main_obj(fmt: BinaryFormat, arch: Architecture) -> Vec<u8> {
    let mut obj = WriteObject::new(fmt, arch, Endianness::Little);
    let text = obj.section_id(StandardSection::Text);
    let code: Vec<u8> = match arch {
        Architecture::X86_64 => vec![0x31, 0xc0, 0xc3], // xor eax, eax; ret
        Architecture::Aarch64 => {
            let mut v = Vec::new();
            v.extend_from_slice(&0xd280_0000u32.to_le_bytes()); // mov x0, #0
            v.extend_from_slice(&0xd65f_03c0u32.to_le_bytes()); // ret
            v
        }
        _ => unreachable!(),
    };
    let off = obj.append_section_data(text, &code, 16);
    add_sym(
        &mut obj,
        b"main",
        SymbolSection::Section(text),
        off,
        code.len() as u64,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    obj.write().unwrap()
}

// ═══════════════════════════════════════════════════════════════════════════
// 1. ELF64 (x86_64 & aarch64) — Headers, PHDRs, SHDRs, PIE, Shared, TLS, BSS
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn elf64_x86_64_static_headers_phdrs_shdrs_and_congruence() {
    let td = TempDir::new("elf_x64_hdr");
    let mut obj = WriteObject::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let text = obj.section_id(StandardSection::Text);
    let rodata = obj.section_id(StandardSection::ReadOnlyData);
    let data = obj.section_id(StandardSection::Data);
    let bss = obj.section_id(StandardSection::UninitializedData);

    let t_off =
        obj.append_section_data(text, &[0xb8, 60, 0, 0, 0, 0xbf, 7, 0, 0, 0, 0x0f, 0x05], 16);
    add_sym(
        &mut obj,
        b"_start",
        SymbolSection::Section(text),
        t_off,
        12,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    let r_off = obj.append_section_data(rodata, b"LPP_RODATA\0", 16);
    add_sym(
        &mut obj,
        b"ro_msg",
        SymbolSection::Section(rodata),
        r_off,
        11,
        SymbolKind::Data,
        SymbolScope::Linkage,
        false,
    );
    let d_off = obj.append_section_data(data, &0x1122_3344_5566_7788u64.to_le_bytes(), 16);
    add_sym(
        &mut obj,
        b"rw_val",
        SymbolSection::Section(data),
        d_off,
        8,
        SymbolKind::Data,
        SymbolScope::Linkage,
        false,
    );
    let b_off = obj.append_section_bss(bss, 128, 16);
    add_sym(
        &mut obj,
        b"bss_buf",
        SymbolSection::Section(bss),
        b_off,
        128,
        SymbolKind::Data,
        SymbolScope::Linkage,
        false,
    );

    let in_obj = td.write("prog.o", &obj.write().unwrap());
    let out = td.out("prog");
    let opts = LinkOptions {
        format: Some(OutputFormat::Elf),
        machine: Some(Machine::X86_64),
        dynamic: DynamicMode::Static,
        ..Default::default()
    };
    let report = link_typed(&[in_obj], &out, &opts).expect("static ELF64 link");
    assert_eq!(report.format, ResolvedFormat::Elf);

    let bytes = std::fs::read(&out).unwrap();
    let hdr = parse_elf_hdr(&bytes);
    assert_eq!(hdr.e_type, elf::ET_EXEC);
    assert_eq!(hdr.e_machine, elf::EM_X86_64);
    assert!(hdr.e_entry >= 0x400000);

    let phdrs = parse_elf_phdrs(&bytes, &hdr);
    let loads: Vec<&ElfPhdr> = phdrs.iter().filter(|p| p.p_type == elf::PT_LOAD).collect();
    assert_eq!(loads.len(), 3, "expect R, RX, RW PT_LOAD segments");
    for seg in &loads {
        assert_eq!(
            seg.p_vaddr % seg.p_align,
            seg.p_offset % seg.p_align,
            "ELF congruence p_vaddr % p_align == p_offset % p_align violated"
        );
        assert_eq!(seg.p_paddr, seg.p_vaddr, "p_paddr tracks p_vaddr");
        assert!(seg.p_memsz >= seg.p_filesz);
    }
    // RX segment contains entry point
    let rx = loads.iter().find(|p| p.p_flags & elf::PF_X != 0).unwrap();
    assert!(hdr.e_entry >= rx.p_vaddr && hdr.e_entry < rx.p_vaddr + rx.p_memsz);
    // RW segment carries .bss in memsz - filesz
    let rw = loads
        .iter()
        .find(|p| (p.p_flags & elf::PF_W != 0) && (p.p_flags & elf::PF_X == 0))
        .unwrap();
    assert!(rw.p_memsz >= rw.p_filesz + 128);
    // GNU_STACK is RW (non-executable)
    let stack = phdrs
        .iter()
        .find(|p| p.p_type == elf::PT_GNU_STACK)
        .expect("PT_GNU_STACK present");
    assert_eq!(stack.p_flags, elf::PF_R | elf::PF_W);
    // No PT_INTERP or PT_DYNAMIC in static link
    assert!(!phdrs.iter().any(|p| p.p_type == elf::PT_INTERP));
    assert!(!phdrs.iter().any(|p| p.p_type == elf::PT_DYNAMIC));
    // Build-id note present
    assert!(phdrs.iter().any(|p| p.p_type == elf::PT_NOTE));

    let shdrs = parse_elf_shdrs(&bytes, &hdr);
    let sh_map: HashMap<&str, &ElfShdr> = shdrs.iter().map(|s| (s.name.as_str(), s)).collect();
    let sh_text = sh_map[".text"];
    assert_eq!(
        sh_text.sh_flags & (elf::SHF_ALLOC as u64 | elf::SHF_EXECINSTR as u64),
        elf::SHF_ALLOC as u64 | elf::SHF_EXECINSTR as u64
    );
    let sh_data = sh_map[".data"];
    assert_eq!(
        sh_data.sh_flags & (elf::SHF_ALLOC as u64 | elf::SHF_WRITE as u64),
        elf::SHF_ALLOC as u64 | elf::SHF_WRITE as u64
    );
    let sh_bss = sh_map[".bss"];
    assert_eq!(sh_bss.sh_type, elf::SHT_NOBITS);
    assert_eq!(sh_bss.sh_offset, 0);
    assert!(sh_bss.sh_size >= 128);
    assert!(
        sh_bss.sh_addr >= sh_data.sh_addr + sh_data.sh_size,
        ".bss VA (0x{:x}) must follow .data end (0x{:x})",
        sh_bss.sh_addr,
        sh_data.sh_addr + sh_data.sh_size
    );
    let symtab_idx_link = sh_map[".symtab"].sh_link as usize;
    assert_eq!(shdrs[symtab_idx_link].name, ".strtab");

    let parsed = ReadFile::parse(&*bytes).expect("object::read parses ELF");
    assert_eq!(parsed.architecture(), Architecture::X86_64);
    assert_eq!(parsed.entry(), hdr.e_entry);

    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        assert_eq!(exec_linux_binary(&out), 7);
    }
}

#[test]
fn elf64_x86_64_pie_and_stripped_options() {
    let td = TempDir::new("elf_pie_strip");
    let obj = td.write("s.o", &make_x64_exit_start_obj(19));
    let out = td.out("pie_bin");
    let opts = LinkOptions {
        format: Some(OutputFormat::Elf),
        machine: Some(Machine::X86_64),
        pie: true,
        strip: true,
        build_id: false,
        dynamic: DynamicMode::Static,
        ..Default::default()
    };
    link_typed(&[obj], &out, &opts).expect("PIE stripped link");

    let bytes = std::fs::read(&out).unwrap();
    let hdr = parse_elf_hdr(&bytes);
    assert_eq!(hdr.e_type, elf::ET_DYN, "PIE emits ET_DYN");
    // Base is 0 for PIE so entry < 0x10000
    assert!(hdr.e_entry > 0 && hdr.e_entry < 0x10000);

    let phdrs = parse_elf_phdrs(&bytes, &hdr);
    assert!(!phdrs.iter().any(|p| p.p_type == elf::PT_NOTE));

    let shdrs = parse_elf_shdrs(&bytes, &hdr);
    assert!(!shdrs.iter().any(|s| s.name == ".symtab"));
    assert!(!shdrs.iter().any(|s| s.name == ".strtab"));
    assert!(shdrs.iter().any(|s| s.name == ".text"));

    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        assert_eq!(exec_linux_binary(&out), 19);
    }
}

#[test]
fn elf64_x86_64_shared_library_exports_and_soname() {
    let td = TempDir::new("elf_shared");
    let mut obj = WriteObject::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let text = obj.section_id(StandardSection::Text);
    let data = obj.section_id(StandardSection::Data);

    let fn_off = obj.append_section_data(text, &[0xb8, 42, 0, 0, 0, 0xc3], 16);
    add_sym(
        &mut obj,
        b"lpp_exported_fn",
        SymbolSection::Section(text),
        fn_off,
        6,
        SymbolKind::Text,
        SymbolScope::Dynamic,
        false,
    );
    let var_off = obj.append_section_data(data, &999u64.to_le_bytes(), 8);
    add_sym(
        &mut obj,
        b"lpp_exported_var",
        SymbolSection::Section(data),
        var_off,
        8,
        SymbolKind::Data,
        SymbolScope::Dynamic,
        false,
    );

    let in_obj = td.write("lib.o", &obj.write().unwrap());
    let out = td.out("liblpp_test.so");
    let opts = LinkOptions {
        format: Some(OutputFormat::Elf),
        machine: Some(Machine::X86_64),
        shared: true,
        soname: Some("liblpp_test.so.1".into()),
        rpath: vec!["$ORIGIN".into(), "/opt/lpp/lib".into()],
        ..Default::default()
    };
    link_typed(&[in_obj], &out, &opts).expect("shared library link");

    let bytes = std::fs::read(&out).unwrap();
    let hdr = parse_elf_hdr(&bytes);
    assert_eq!(hdr.e_type, elf::ET_DYN);
    let phdrs = parse_elf_phdrs(&bytes, &hdr);
    assert!(phdrs.iter().any(|p| p.p_type == elf::PT_DYNAMIC));
    assert!(!phdrs.iter().any(|p| p.p_type == elf::PT_INTERP));

    let shdrs = parse_elf_shdrs(&bytes, &hdr);
    let dyn_sh = shdrs.iter().find(|s| s.name == ".dynamic").unwrap();
    let dynstr_sh = shdrs.iter().find(|s| s.name == ".dynstr").unwrap();
    let dynstr_slice =
        &bytes[dynstr_sh.sh_offset as usize..(dynstr_sh.sh_offset + dynstr_sh.sh_size) as usize];
    let tags = parse_elf_dynamic_tags(&bytes, dyn_sh);

    let soname_off = tags
        .iter()
        .find(|(t, _)| *t == elf::DT_SONAME as i64)
        .map(|(_, v)| *v as usize)
        .expect("DT_SONAME present");
    assert_eq!(read_cstr(dynstr_slice, soname_off), "liblpp_test.so.1");

    let runpath_off = tags
        .iter()
        .find(|(t, _)| *t == elf::DT_RUNPATH as i64)
        .map(|(_, v)| *v as usize)
        .expect("DT_RUNPATH present");
    assert_eq!(read_cstr(dynstr_slice, runpath_off), "$ORIGIN:/opt/lpp/lib");

    // DF_1_PIE must NOT be set on shared libraries
    let flags1 = tags
        .iter()
        .find(|(t, _)| *t == elf::DT_FLAGS_1 as i64)
        .map(|(_, v)| *v)
        .unwrap_or(0);
    assert_eq!(flags1 & 0x0800_0000, 0);

    let parsed = ReadFile::parse(&*bytes).unwrap();
    let dyn_syms: Vec<String> = parsed
        .dynamic_symbols()
        .filter_map(|s| s.name().ok().map(str::to_string))
        .collect();
    assert!(dyn_syms.contains(&"lpp_exported_fn".to_string()));
    assert!(dyn_syms.contains(&"lpp_exported_var".to_string()));
}

#[test]
fn elf64_x86_64_dynamic_plt_got_interp_and_rpath() {
    let td = TempDir::new("elf_dyn_x64");
    let mut obj = WriteObject::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let text = obj.section_id(StandardSection::Text);
    // call exit@PLT ; xor eax, eax ; ret
    let off = obj.append_section_data(text, &[0xe8, 0, 0, 0, 0, 0x31, 0xc0, 0xc3], 16);
    add_sym(
        &mut obj,
        b"main",
        SymbolSection::Section(text),
        off,
        8,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    let exit_sym = undef_sym(&mut obj, b"exit", SymbolKind::Text);
    obj.add_relocation(
        text,
        Relocation {
            offset: off + 1,
            symbol: exit_sym,
            addend: -4,
            flags: RelocationFlags::Elf {
                r_type: elf::R_X86_64_PLT32,
            },
        },
    )
    .unwrap();

    let in_obj = td.write("dyn.o", &obj.write().unwrap());
    let out = td.out("dyn_exec");
    let opts = LinkOptions {
        format: Some(OutputFormat::Elf),
        machine: Some(Machine::X86_64),
        pie: true,
        dynamic: DynamicMode::Auto,
        needed: vec!["libm.so.6".into()],
        rpath: vec!["/usr/local/lib".into()],
        ..Default::default()
    };
    link_typed(&[in_obj], &out, &opts).expect("dynamic x86_64 ELF link");

    let bytes = std::fs::read(&out).unwrap();
    let hdr = parse_elf_hdr(&bytes);
    let phdrs = parse_elf_phdrs(&bytes, &hdr);
    let interp = phdrs
        .iter()
        .find(|p| p.p_type == elf::PT_INTERP)
        .expect("PT_INTERP present");
    let interp_str = read_cstr(
        &bytes[interp.p_offset as usize..(interp.p_offset + interp.p_filesz) as usize],
        0,
    );
    assert_eq!(interp_str, "/lib64/ld-linux-x86-64.so.2");

    let shdrs = parse_elf_shdrs(&bytes, &hdr);
    let sh_map: HashMap<&str, &ElfShdr> = shdrs.iter().map(|s| (s.name.as_str(), s)).collect();
    assert!(sh_map.contains_key(".plt"));
    assert!(sh_map.contains_key(".got.plt"));
    assert!(sh_map.contains_key(".rela.plt"));
    assert!(sh_map.contains_key(".dynsym"));
    assert!(sh_map.contains_key(".dynstr"));
    assert!(sh_map.contains_key(".hash"));

    // Verify DF_1_PIE is set on dynamic PIE executable
    let dyn_sh = sh_map[".dynamic"];
    let tags = parse_elf_dynamic_tags(&bytes, dyn_sh);
    let flags1 = tags
        .iter()
        .find(|(t, _)| *t == elf::DT_FLAGS_1 as i64)
        .map(|(_, v)| *v)
        .unwrap();
    assert_ne!(flags1 & 0x0800_0000, 0, "DF_1_PIE must be set for PIE");

    // Verify PLT call displacement in main points into .plt
    let sh_text = sh_map[".text"];
    let sh_plt = sh_map[".plt"];
    let parsed = ReadFile::parse(&*bytes).unwrap();
    let main_sym = parsed
        .symbols()
        .find(|s| s.name() == Ok("main"))
        .expect("main in symtab");
    let main_file_off =
        sh_text.sh_offset as usize + (main_sym.address() - sh_text.sh_addr) as usize;
    let disp = le_i32(&bytes, main_file_off + 1);
    let call_target_va = (main_sym.address() as i64 + 5 + disp as i64) as u64;
    assert!(
        call_target_va >= sh_plt.sh_addr && call_target_va < sh_plt.sh_addr + sh_plt.sh_size,
        "call exit target 0x{call_target_va:x} must land inside .plt [0x{:x}..0x{:x})",
        sh_plt.sh_addr,
        sh_plt.sh_addr + sh_plt.sh_size
    );
}

#[test]
fn elf64_aarch64_static_and_dynamic_invariants() {
    let td = TempDir::new("elf_a64");
    // Static with explicit _start (syscall exit)
    let obj_start = td.write("a64_start.o", &make_a64_exit_start_obj(9));
    let out_start = td.out("a64_start_bin");
    link_typed(
        &[obj_start],
        &out_start,
        &LinkOptions {
            format: Some(OutputFormat::Elf),
            machine: Some(Machine::Aarch64),
            dynamic: DynamicMode::Static,
            ..Default::default()
        },
    )
    .expect("aarch64 _start ELF link");
    let sbytes = std::fs::read(&out_start).unwrap();
    assert_eq!(parse_elf_hdr(&sbytes).e_machine, elf::EM_AARCH64);

    // Static with synthesized _start calling main
    let obj_main = td.write(
        "main.o",
        &make_minimal_main_obj(BinaryFormat::Elf, Architecture::Aarch64),
    );
    let out_static = td.out("a64_static");
    let opts_static = LinkOptions {
        format: Some(OutputFormat::Elf),
        machine: Some(Machine::Aarch64),
        dynamic: DynamicMode::Static,
        ..Default::default()
    };
    link_typed(&[obj_main], &out_static, &opts_static).expect("aarch64 static ELF link");

    let bytes = std::fs::read(&out_static).unwrap();
    let hdr = parse_elf_hdr(&bytes);
    assert_eq!(hdr.e_machine, elf::EM_AARCH64);
    assert_eq!(hdr.e_type, elf::ET_EXEC);
    let phdrs = parse_elf_phdrs(&bytes, &hdr);
    let rx = phdrs
        .iter()
        .find(|p| p.p_type == elf::PT_LOAD && (p.p_flags & elf::PF_X != 0))
        .unwrap();
    // Default aarch64 ELF page size is 64KB (0x10000)
    assert_eq!(rx.p_align, 0x10000);
    assert!(hdr.e_entry >= rx.p_vaddr && hdr.e_entry < rx.p_vaddr + rx.p_memsz);

    // Dynamic aarch64 with external call to puts
    let mut dobj = WriteObject::new(BinaryFormat::Elf, Architecture::Aarch64, Endianness::Little);
    let text = dobj.section_id(StandardSection::Text);
    // bl puts; ret
    let mut code = Vec::new();
    code.extend_from_slice(&0x9400_0000u32.to_le_bytes());
    code.extend_from_slice(&0xd65f_03c0u32.to_le_bytes());
    let off = dobj.append_section_data(text, &code, 16);
    add_sym(
        &mut dobj,
        b"_start",
        SymbolSection::Section(text),
        off,
        8,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    let puts_sym = undef_sym(&mut dobj, b"puts", SymbolKind::Text);
    dobj.add_relocation(
        text,
        Relocation {
            offset: off,
            symbol: puts_sym,
            addend: 0,
            flags: RelocationFlags::Elf {
                r_type: elf::R_AARCH64_CALL26,
            },
        },
    )
    .unwrap();
    let in_dyn = td.write("a64_dyn.o", &dobj.write().unwrap());
    let out_dyn = td.out("a64_dyn");
    let opts_dyn = LinkOptions {
        format: Some(OutputFormat::Elf),
        machine: Some(Machine::Aarch64),
        dynamic: DynamicMode::Auto,
        page_size: Some(0x1000),
        ..Default::default()
    };
    link_typed(&[in_dyn], &out_dyn, &opts_dyn).expect("aarch64 dynamic ELF link");

    let dbytes = std::fs::read(&out_dyn).unwrap();
    let dhdr = parse_elf_hdr(&dbytes);
    let dphdrs = parse_elf_phdrs(&dbytes, &dhdr);
    let interp = dphdrs
        .iter()
        .find(|p| p.p_type == elf::PT_INTERP)
        .expect("aarch64 PT_INTERP");
    let interp_str = read_cstr(
        &dbytes[interp.p_offset as usize..(interp.p_offset + interp.p_filesz) as usize],
        0,
    );
    assert_eq!(interp_str, "/lib/ld-linux-aarch64.so.1");
}

#[test]
fn elf64_tls_init_fini_common_and_map_file() {
    let td = TempDir::new("elf_tls_common");
    let mut obj1 = WriteObject::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let text1 = obj1.section_id(StandardSection::Text);
    let tdata1 = obj1.section_id(StandardSection::Tls);
    let tbss1 = obj1.section_id(StandardSection::UninitializedTls);
    let init1 = obj1.add_section(Vec::new(), b".init_array".to_vec(), SectionKind::Data);
    let fini1 = obj1.add_section(Vec::new(), b".fini_array".to_vec(), SectionKind::Data);

    // _start reads tpoff32 of tls_var into eax and exits with 0
    let mut code = vec![0xb8, 0, 0, 0, 0]; // mov eax, imm32 (patched by TPOFF32)
    code.extend_from_slice(&[0xb8, 60, 0, 0, 0, 0x31, 0xff, 0x0f, 0x05]);
    let s_off = obj1.append_section_data(text1, &code, 16);
    add_sym(
        &mut obj1,
        b"_start",
        SymbolSection::Section(text1),
        s_off,
        code.len() as u64,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    let init_fn_off = obj1.append_section_data(text1, &[0xc3], 16);
    let init_fn_sym = add_sym(
        &mut obj1,
        b"my_init",
        SymbolSection::Section(text1),
        init_fn_off,
        1,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    let i_off = obj1.append_section_data(init1, &[0u8; 8], 8);
    obj1.add_relocation(
        init1,
        Relocation {
            offset: i_off,
            symbol: init_fn_sym,
            addend: 0,
            flags: RelocationFlags::Elf {
                r_type: elf::R_X86_64_64,
            },
        },
    )
    .unwrap();
    let f_off = obj1.append_section_data(fini1, &[0u8; 8], 8);
    obj1.add_relocation(
        fini1,
        Relocation {
            offset: f_off,
            symbol: init_fn_sym,
            addend: 0,
            flags: RelocationFlags::Elf {
                r_type: elf::R_X86_64_64,
            },
        },
    )
    .unwrap();

    let td_off = obj1.append_section_data(tdata1, &0xCAFE_BABEu64.to_le_bytes(), 8);
    let tls_sym = add_sym(
        &mut obj1,
        b"tls_var",
        SymbolSection::Section(tdata1),
        td_off,
        8,
        SymbolKind::Tls,
        SymbolScope::Linkage,
        false,
    );
    let tb_off = obj1.append_section_bss(tbss1, 16, 8);
    add_sym(
        &mut obj1,
        b"tbss_var",
        SymbolSection::Section(tbss1),
        tb_off,
        16,
        SymbolKind::Tls,
        SymbolScope::Linkage,
        false,
    );
    obj1.add_relocation(
        text1,
        Relocation {
            offset: s_off + 1,
            symbol: tls_sym,
            addend: 0,
            flags: RelocationFlags::Elf {
                r_type: elf::R_X86_64_TPOFF32,
            },
        },
    )
    .unwrap();
    // COMMON symbol size 16 in obj1
    add_sym(
        &mut obj1,
        b"shared_comm",
        SymbolSection::Common,
        8,
        16,
        SymbolKind::Data,
        SymbolScope::Linkage,
        false,
    );

    // obj2 declares COMMON shared_comm with larger size 64
    let mut obj2 = WriteObject::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    add_sym(
        &mut obj2,
        b"shared_comm",
        SymbolSection::Common,
        16,
        64,
        SymbolKind::Data,
        SymbolScope::Linkage,
        false,
    );

    let p1 = td.write("o1.o", &obj1.write().unwrap());
    let p2 = td.write("o2.o", &obj2.write().unwrap());
    let out = td.out("tls_comm");
    let map = td.out("link.map");
    let opts = LinkOptions {
        format: Some(OutputFormat::Elf),
        machine: Some(Machine::X86_64),
        dynamic: DynamicMode::Static,
        map_path: Some(map.clone()),
        ..Default::default()
    };
    link_typed(&[p1, p2], &out, &opts).expect("TLS + init/fini + COMMON link");

    let bytes = std::fs::read(&out).unwrap();
    let hdr = parse_elf_hdr(&bytes);
    let phdrs = parse_elf_phdrs(&bytes, &hdr);
    let pt_tls = phdrs
        .iter()
        .find(|p| p.p_type == elf::PT_TLS)
        .expect("PT_TLS present");
    assert_eq!(pt_tls.p_filesz, 8);
    assert_eq!(pt_tls.p_memsz, 24); // 8 tdata + 16 tbss

    let shdrs = parse_elf_shdrs(&bytes, &hdr);
    assert!(shdrs.iter().any(|s| s.name == ".init_array"));
    assert!(shdrs.iter().any(|s| s.name == ".fini_array"));
    assert!(shdrs.iter().any(|s| s.name == ".tdata"));
    assert!(shdrs.iter().any(|s| s.name == ".tbss"));

    // Verify TPOFF32 patched value: x86_64 variant II = offset - align_up(tls_memsz, align)
    let sh_text = shdrs.iter().find(|s| s.name == ".text").unwrap();
    let tpoff_val = le_i32(&bytes, sh_text.sh_offset as usize + 1);
    assert!(
        tpoff_val < 0,
        "x86_64 TPOFF32 must be negative: {tpoff_val}"
    );

    // Verify COMMON resolved to max size 64 in .bss
    let parsed = ReadFile::parse(&*bytes).unwrap();
    let comm_sym = parsed
        .symbols()
        .find(|s| s.name() == Ok("shared_comm"))
        .expect("shared_comm in symtab");
    assert_eq!(comm_sym.size(), 64);

    let map_text = std::fs::read_to_string(&map).expect("map file written");
    assert!(map_text.contains("_start"));
    assert!(map_text.contains("shared_comm"));
    assert!(map_text.contains("tls_var"));
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. Bare-Metal Static ELF64 Kernel Linking & Multiboot2 Invariants
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn bare_metal_x86_64_kernel_multiboot2_and_freestanding_arch() {
    let td = TempDir::new("bm_x64");
    let mb2_path = td.write("mb2.o", &multiboot2_header_object(Machine::X86_64));
    let arch_path = td.write("arch.o", &freestanding_arch_object(Machine::X86_64));

    // Kernel object referencing lpp_arch_* and defining _start with .bss
    let mut kobj = WriteObject::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let text = kobj.section_id(StandardSection::Text);
    let data = kobj.section_id(StandardSection::Data);
    let bss = kobj.section_id(StandardSection::UninitializedData);

    // _start: call lpp_arch_cpu_cli ; call lpp_arch_rdtsc ; call lpp_arch_cpu_hlt ; jmp .
    let code = [
        0xe8, 0, 0, 0, 0, // call lpp_arch_cpu_cli (+1)
        0xe8, 0, 0, 0, 0, // call lpp_arch_rdtsc (+6)
        0xe8, 0, 0, 0, 0, // call lpp_arch_cpu_hlt (+11)
        0xeb, 0xfe, // jmp $
    ];
    let k_off = kobj.append_section_data(text, &code, 16);
    add_sym(
        &mut kobj,
        b"_start",
        SymbolSection::Section(text),
        k_off,
        code.len() as u64,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    for (rel_off, name) in [
        (k_off + 1, b"lpp_arch_cpu_cli".as_slice()),
        (k_off + 6, b"lpp_arch_rdtsc".as_slice()),
        (k_off + 11, b"lpp_arch_cpu_hlt".as_slice()),
    ] {
        let sym = undef_sym(&mut kobj, name, SymbolKind::Text);
        kobj.add_relocation(
            text,
            Relocation {
                offset: rel_off,
                symbol: sym,
                addend: -4,
                flags: RelocationFlags::Elf {
                    r_type: elf::R_X86_64_PLT32,
                },
            },
        )
        .unwrap();
    }
    let d_off = kobj.append_section_data(data, b"KERNEL_DATA\0", 16);
    add_sym(
        &mut kobj,
        b"kernel_banner",
        SymbolSection::Section(data),
        d_off,
        12,
        SymbolKind::Data,
        SymbolScope::Linkage,
        false,
    );
    let b_off = kobj.append_section_bss(bss, 4096, 16);
    add_sym(
        &mut kobj,
        b"kernel_stack",
        SymbolSection::Section(bss),
        b_off,
        4096,
        SymbolKind::Data,
        SymbolScope::Linkage,
        false,
    );

    let k_path = td.write("kernel.o", &kobj.write().unwrap());
    let out = td.out("kernel.elf");

    let opts = LinkOptions {
        format: Some(OutputFormat::Elf),
        machine: Some(Machine::X86_64),
        entry: Some("_start".into()),
        image_base: Some(0x0010_0000),
        pie: false,
        dynamic: DynamicMode::Static,
        ..Default::default()
    };
    link_typed(&[mb2_path, k_path, arch_path], &out, &opts)
        .expect("bare-metal x86_64 kernel link must succeed");

    let bytes = std::fs::read(&out).unwrap();
    let hdr = parse_elf_hdr(&bytes);
    assert_eq!(hdr.e_type, elf::ET_EXEC);
    assert_eq!(hdr.e_machine, elf::EM_X86_64);
    assert!(hdr.e_entry >= 0x0010_1000);

    let phdrs = parse_elf_phdrs(&bytes, &hdr);
    assert!(!phdrs.iter().any(|p| p.p_type == elf::PT_INTERP));
    assert!(!phdrs.iter().any(|p| p.p_type == elf::PT_DYNAMIC));

    // Verify RX PT_LOAD is 4KB-aligned at p_offset = 0x1000, p_vaddr = 0x0010_1000
    let rx = phdrs
        .iter()
        .find(|p| p.p_type == elf::PT_LOAD && (p.p_flags & elf::PF_X != 0))
        .expect("RX PT_LOAD");
    assert_eq!(rx.p_align, 0x1000);
    assert_eq!(rx.p_offset, 0x1000);
    assert_eq!(rx.p_vaddr, 0x0010_1000);

    // Verify Multiboot2 header magic (0xE852_50D6) and valid checksum inside first 8KB
    const MB2_MAGIC: u32 = 0xE852_50D6;
    let search_end = bytes.len().min(8192);
    let mb2_pos = bytes[..search_end]
        .windows(4)
        .position(|w| u32::from_le_bytes(w.try_into().unwrap()) == MB2_MAGIC)
        .expect("Multiboot2 magic must reside within first 8KB of kernel ELF");
    assert_eq!(mb2_pos % 8, 0, "Multiboot2 header must be 8-byte aligned");
    assert!(mb2_pos >= rx.p_offset as usize);
    let arch = le_u32(&bytes, mb2_pos + 4);
    let hlen = le_u32(&bytes, mb2_pos + 8);
    let csum = le_u32(&bytes, mb2_pos + 12);
    assert_eq!(arch, 0);
    assert_eq!(hlen, 24);
    assert_eq!(
        MB2_MAGIC
            .wrapping_add(arch)
            .wrapping_add(hlen)
            .wrapping_add(csum),
        0
    );

    // Verify all freestanding arch symbols and kernel_stack exist
    let parsed = ReadFile::parse(&*bytes).unwrap();
    for expected in [
        "__lpp_multiboot2_header",
        "_start",
        "lpp_arch_cpu_cli",
        "lpp_arch_rdtsc",
        "lpp_arch_cpu_hlt",
        "lpp_panic_div_zero",
        "kernel_stack",
    ] {
        assert!(
            parsed.symbols().any(|s| s.name() == Ok(expected)),
            "missing bare-metal symbol '{expected}'"
        );
    }
}

#[test]
fn bare_metal_aarch64_kernel_with_4kb_page_and_no_startup() {
    let td = TempDir::new("bm_a64");
    let mb2_path = td.write("mb2.o", &multiboot2_header_object(Machine::Aarch64));
    let arch_path = td.write("arch.o", &freestanding_arch_object(Machine::Aarch64));

    let mut kobj = WriteObject::new(BinaryFormat::Elf, Architecture::Aarch64, Endianness::Little);
    let text = kobj.section_id(StandardSection::Text);
    // kernel_main: bl lpp_arch_cpu_cli; bl lpp_arch_invlpg; b .
    let mut code = Vec::new();
    code.extend_from_slice(&0x9400_0000u32.to_le_bytes()); // bl lpp_arch_cpu_cli
    code.extend_from_slice(&0x9400_0000u32.to_le_bytes()); // bl lpp_arch_invlpg
    code.extend_from_slice(&0x1400_0000u32.to_le_bytes()); // b .
    let off = kobj.append_section_data(text, &code, 16);
    add_sym(
        &mut kobj,
        b"kernel_main",
        SymbolSection::Section(text),
        off,
        code.len() as u64,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    let cli_sym = undef_sym(&mut kobj, b"lpp_arch_cpu_cli", SymbolKind::Text);
    let invlpg_sym = undef_sym(&mut kobj, b"lpp_arch_invlpg", SymbolKind::Text);
    kobj.add_relocation(
        text,
        Relocation {
            offset: off,
            symbol: cli_sym,
            addend: 0,
            flags: RelocationFlags::Elf {
                r_type: elf::R_AARCH64_CALL26,
            },
        },
    )
    .unwrap();
    kobj.add_relocation(
        text,
        Relocation {
            offset: off + 4,
            symbol: invlpg_sym,
            addend: 0,
            flags: RelocationFlags::Elf {
                r_type: elf::R_AARCH64_CALL26,
            },
        },
    )
    .unwrap();

    let k_path = td.write("k64.o", &kobj.write().unwrap());
    let out = td.out("kernel_a64.elf");

    let opts = LinkOptions {
        format: Some(OutputFormat::Elf),
        machine: Some(Machine::Aarch64),
        entry: Some("kernel_main".into()),
        image_base: Some(0x4008_0000),
        page_size: Some(0x1000),
        no_startup: true,
        pie: false,
        dynamic: DynamicMode::Static,
        ..Default::default()
    };
    link_typed(&[mb2_path, k_path, arch_path], &out, &opts)
        .expect("bare-metal aarch64 kernel link");

    let bytes = std::fs::read(&out).unwrap();
    let hdr = parse_elf_hdr(&bytes);
    assert_eq!(hdr.e_machine, elf::EM_AARCH64);
    assert_eq!(hdr.e_type, elf::ET_EXEC);

    let parsed = ReadFile::parse(&*bytes).unwrap();
    let km = parsed
        .symbols()
        .find(|s| s.name() == Ok("kernel_main"))
        .unwrap();
    assert_eq!(hdr.e_entry, km.address());
    assert!(hdr.e_entry >= 0x4008_1000);

    // Check Multiboot2 header is inside first 8KB because page_size=0x1000
    let mb2_pos = bytes[..8192]
        .windows(4)
        .position(|w| u32::from_le_bytes(w.try_into().unwrap()) == 0xE852_50D6);
    assert!(mb2_pos.is_some(), "Multiboot2 header must be in first 8KB");

    // Verify BL instructions in kernel_main actually point to lpp_arch_cpu_cli & invlpg
    let shdrs = parse_elf_shdrs(&bytes, &hdr);
    let sh_text = shdrs.iter().find(|s| s.name == ".text").unwrap();
    let km_file = sh_text.sh_offset as usize + (km.address() - sh_text.sh_addr) as usize;
    let bl_cli = le_u32(&bytes, km_file);
    assert_eq!(bl_cli & 0xFC00_0000, 0x9400_0000);
    let imm26_cli = ((bl_cli & 0x03FF_FFFF) as i32) << 6 >> 6;
    let target_cli = (km.address() as i64 + (imm26_cli as i64) * 4) as u64;
    let expected_cli = parsed
        .symbols()
        .find(|s| s.name() == Ok("lpp_arch_cpu_cli"))
        .unwrap()
        .address();
    assert_eq!(target_cli, expected_cli);
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. Multi-Object Relocations, Weak/Local/Global Symbols & Static Archives
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn elf64_x86_64_multi_object_pc32_abs64_got_size_and_execution() {
    let td = TempDir::new("elf_x64_reloc");

    // Object A: defines helper(x) returning x + ro_bias + rw_counter + bss_slot
    // and exercises local symbol isolation (`local_helper`).
    let mut obj_a = WriteObject::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let ta = obj_a.section_id(StandardSection::Text);
    let ra = obj_a.section_id(StandardSection::ReadOnlyData);
    let da = obj_a.section_id(StandardSection::Data);

    // local_helper: mov eax, 10 ; ret
    let lh_off = obj_a.append_section_data(ta, &[0xb8, 10, 0, 0, 0, 0xc3], 16);
    let lh_sym = add_sym(
        &mut obj_a,
        b"local_helper",
        SymbolSection::Section(ta),
        lh_off,
        6,
        SymbolKind::Text,
        SymbolScope::Compilation,
        false,
    );

    // ro_bias = 5 (4 bytes)
    let ro_off = obj_a.append_section_data(ra, &5u32.to_le_bytes(), 4);
    let ro_sym = add_sym(
        &mut obj_a,
        b"ro_bias",
        SymbolSection::Section(ra),
        ro_off,
        4,
        SymbolKind::Data,
        SymbolScope::Linkage,
        false,
    );

    // ptr_to_ro: 8-byte R_X86_64_64 pointing to ro_bias
    let ptr_off = obj_a.append_section_data(da, &[0u8; 8], 8);
    add_sym(
        &mut obj_a,
        b"ptr_to_ro",
        SymbolSection::Section(da),
        ptr_off,
        8,
        SymbolKind::Data,
        SymbolScope::Linkage,
        false,
    );
    obj_a
        .add_relocation(
            da,
            Relocation {
                offset: ptr_off,
                symbol: ro_sym,
                addend: 0,
                flags: RelocationFlags::Elf {
                    r_type: elf::R_X86_64_64,
                },
            },
        )
        .unwrap();

    // size_of_ro: 4-byte R_X86_64_SIZE32 of ro_bias
    let sz_off = obj_a.append_section_data(da, &[0u8; 4], 4);
    add_sym(
        &mut obj_a,
        b"size_of_ro",
        SymbolSection::Section(da),
        sz_off,
        4,
        SymbolKind::Data,
        SymbolScope::Linkage,
        false,
    );
    obj_a
        .add_relocation(
            da,
            Relocation {
                offset: sz_off,
                symbol: ro_sym,
                addend: 0,
                flags: RelocationFlags::Elf {
                    r_type: elf::R_X86_64_SIZE32,
                },
            },
        )
        .unwrap();

    // fn compute_a():
    //   call local_helper            ; eax = 10
    //   mov rcx, [rip + ptr_to_ro]   ; rcx = &ro_bias (PC32)
    //   add eax, [rcx]               ; eax = 15
    //   mov rdx, [rip + got(ro_bias)]; rdx = &ro_bias via GOT (GOTPCREL)
    //   add eax, [rdx]               ; eax = 20
    //   add eax, [rip + size_of_ro]  ; eax = 24
    //   ret
    let comp_code = [
        0xe8, 0, 0, 0, 0, // +0: call local_helper (rel @ +1)
        0x48, 0x8b, 0x0d, 0, 0, 0, 0, // +5: mov rcx, [rip + disp32] (rel @ +8)
        0x03, 0x01, // +12: add eax, [rcx]
        0x48, 0x8b, 0x15, 0, 0, 0, 0, // +14: mov rdx, [rip + disp32] (GOTPCREL @ +17)
        0x03, 0x02, // +21: add eax, [rdx]
        0x03, 0x05, 0, 0, 0, 0,    // +23: add eax, [rip + disp32] (size_of_ro @ +25)
        0xc3, // +29: ret
    ];
    let comp_off = obj_a.append_section_data(ta, &comp_code, 16);
    add_sym(
        &mut obj_a,
        b"compute_a",
        SymbolSection::Section(ta),
        comp_off,
        comp_code.len() as u64,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    let ptr_sym = obj_a.symbol_id(b"ptr_to_ro").unwrap();
    let sz_sym = obj_a.symbol_id(b"size_of_ro").unwrap();
    for (rel_off, sym, rtype) in [
        (comp_off + 1, lh_sym, elf::R_X86_64_PLT32),
        (comp_off + 8, ptr_sym, elf::R_X86_64_PC32),
        (comp_off + 17, ro_sym, elf::R_X86_64_GOTPCREL),
        (comp_off + 25, sz_sym, elf::R_X86_64_PC32),
    ] {
        obj_a
            .add_relocation(
                ta,
                Relocation {
                    offset: rel_off,
                    symbol: sym,
                    addend: -4,
                    flags: RelocationFlags::Elf { r_type: rtype },
                },
            )
            .unwrap();
    }

    // Object B: also defines a local symbol named `local_helper` returning 7,
    // plus `_start` calling `compute_a` + `local_helper`, then syscall exit(eax).
    let mut obj_b = WriteObject::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let tb = obj_b.section_id(StandardSection::Text);
    let lh_b_off = obj_b.append_section_data(tb, &[0xb8, 7, 0, 0, 0, 0xc3], 16);
    let lh_b_sym = add_sym(
        &mut obj_b,
        b"local_helper",
        SymbolSection::Section(tb),
        lh_b_off,
        6,
        SymbolKind::Text,
        SymbolScope::Compilation,
        false,
    );
    let comp_ext = undef_sym(&mut obj_b, b"compute_a", SymbolKind::Text);
    // _start:
    //   call compute_a      ; eax = 24
    //   mov ebx, eax        ; ebx = 24
    //   call local_helper   ; eax = 7 (must call obj_b's local_helper, NOT obj_a's!)
    //   add eax, ebx        ; eax = 31
    //   mov edi, eax        ; exit status = 31
    //   mov eax, 60
    //   syscall
    let start_code = [
        0xe8, 0, 0, 0, 0, // +0: call compute_a (@ +1)
        0x89, 0xc3, // +5: mov ebx, eax
        0xe8, 0, 0, 0, 0, // +7: call local_helper (@ +8)
        0x01, 0xd8, // +12: add eax, ebx
        0x89, 0xc7, // +14: mov edi, eax
        0xb8, 60, 0, 0, 0, // +16: mov eax, 60
        0x0f, 0x05, // +21: syscall
    ];
    let sb_off = obj_b.append_section_data(tb, &start_code, 16);
    add_sym(
        &mut obj_b,
        b"_start",
        SymbolSection::Section(tb),
        sb_off,
        start_code.len() as u64,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    obj_b
        .add_relocation(
            tb,
            Relocation {
                offset: sb_off + 1,
                symbol: comp_ext,
                addend: -4,
                flags: RelocationFlags::Elf {
                    r_type: elf::R_X86_64_PLT32,
                },
            },
        )
        .unwrap();
    obj_b
        .add_relocation(
            tb,
            Relocation {
                offset: sb_off + 8,
                symbol: lh_b_sym,
                addend: -4,
                flags: RelocationFlags::Elf {
                    r_type: elf::R_X86_64_PC32,
                },
            },
        )
        .unwrap();

    let pa = td.write("a.o", &obj_a.write().unwrap());
    let pb = td.write("b.o", &obj_b.write().unwrap());
    let out = td.out("multi_x64");
    let opts = LinkOptions {
        format: Some(OutputFormat::Elf),
        machine: Some(Machine::X86_64),
        dynamic: DynamicMode::Static,
        ..Default::default()
    };
    link_typed(&[pa, pb], &out, &opts).expect("multi-object x86_64 link");

    let bytes = std::fs::read(&out).unwrap();
    let parsed = ReadFile::parse(&*bytes).unwrap();
    let ro_va = parsed
        .symbols()
        .find(|s| s.name() == Ok("ro_bias"))
        .unwrap()
        .address();
    let ptr_va = parsed
        .symbols()
        .find(|s| s.name() == Ok("ptr_to_ro"))
        .unwrap()
        .address();
    let sz_va = parsed
        .symbols()
        .find(|s| s.name() == Ok("size_of_ro"))
        .unwrap()
        .address();

    let hdr = parse_elf_hdr(&bytes);
    let shdrs = parse_elf_shdrs(&bytes, &hdr);
    let sh_data = shdrs.iter().find(|s| s.name == ".data").unwrap();
    let ptr_file = sh_data.sh_offset as usize + (ptr_va - sh_data.sh_addr) as usize;
    let sz_file = sh_data.sh_offset as usize + (sz_va - sh_data.sh_addr) as usize;
    assert_eq!(le_u64(&bytes, ptr_file), ro_va, "R_X86_64_64 patched");
    assert_eq!(le_u32(&bytes, sz_file), 4, "R_X86_64_SIZE32 patched");

    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        assert_eq!(exec_linux_binary(&out), 31);
    }
}

#[test]
fn elf64_aarch64_multi_object_call26_adrp_add_lo12_ldst_and_got() {
    let td = TempDir::new("elf_a64_reloc");

    // Object A: defines global_data (u64) in .data and helper() in .text
    let mut obj_a = WriteObject::new(BinaryFormat::Elf, Architecture::Aarch64, Endianness::Little);
    let ta = obj_a.section_id(StandardSection::Text);
    let da = obj_a.section_id(StandardSection::Data);

    let h_off = obj_a.append_section_data(ta, &0xd65f_03c0u32.to_le_bytes(), 16); // ret
    add_sym(
        &mut obj_a,
        b"helper_a64",
        SymbolSection::Section(ta),
        h_off,
        4,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    let gd_off = obj_a.append_section_data(da, &0xDEAD_BEEF_CAFE_1234u64.to_le_bytes(), 8);
    add_sym(
        &mut obj_a,
        b"global_data",
        SymbolSection::Section(da),
        gd_off,
        8,
        SymbolKind::Data,
        SymbolScope::Linkage,
        false,
    );

    // Object B: references helper_a64 (CALL26), global_data (ADR_PREL_PG_HI21 + ADD_ABS_LO12_NC
    // + LDST64_ABS_LO12_NC), and GOT load of global_data (ADR_GOT_PAGE + LD64_GOT_LO12_NC).
    let mut obj_b = WriteObject::new(BinaryFormat::Elf, Architecture::Aarch64, Endianness::Little);
    let tb = obj_b.section_id(StandardSection::Text);
    let mut code = Vec::new();
    code.extend_from_slice(&0x9400_0000u32.to_le_bytes()); // +0:  bl helper_a64
    code.extend_from_slice(&0x9000_0000u32.to_le_bytes()); // +4:  adrp x0, global_data
    code.extend_from_slice(&0x9100_0000u32.to_le_bytes()); // +8:  add  x0, x0, :lo12:global_data
    code.extend_from_slice(&0xf940_0021u32.to_le_bytes()); // +12: ldr  x1, [x1, :lo12:global_data]
    code.extend_from_slice(&0x9000_0002u32.to_le_bytes()); // +16: adrp x2, :got:global_data
    code.extend_from_slice(&0xf940_0042u32.to_le_bytes()); // +20: ldr  x2, [x2, :got_lo12:global_data]
    code.extend_from_slice(&0xd65f_03c0u32.to_le_bytes()); // +24: ret
    let sb_off = obj_b.append_section_data(tb, &code, 16);
    add_sym(
        &mut obj_b,
        b"_start",
        SymbolSection::Section(tb),
        sb_off,
        code.len() as u64,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    let h_ext = undef_sym(&mut obj_b, b"helper_a64", SymbolKind::Text);
    let gd_ext = undef_sym(&mut obj_b, b"global_data", SymbolKind::Data);

    for (off, sym, rtype) in [
        (sb_off, h_ext, elf::R_AARCH64_CALL26),
        (sb_off + 4, gd_ext, elf::R_AARCH64_ADR_PREL_PG_HI21),
        (sb_off + 8, gd_ext, elf::R_AARCH64_ADD_ABS_LO12_NC),
        (sb_off + 12, gd_ext, elf::R_AARCH64_LDST64_ABS_LO12_NC),
        (sb_off + 16, gd_ext, elf::R_AARCH64_ADR_GOT_PAGE),
        (sb_off + 20, gd_ext, elf::R_AARCH64_LD64_GOT_LO12_NC),
    ] {
        obj_b
            .add_relocation(
                tb,
                Relocation {
                    offset: off,
                    symbol: sym,
                    addend: 0,
                    flags: RelocationFlags::Elf { r_type: rtype },
                },
            )
            .unwrap();
    }

    let pa = td.write("a.o", &obj_a.write().unwrap());
    let pb = td.write("b.o", &obj_b.write().unwrap());
    let out = td.out("a64_reloc");
    let opts = LinkOptions {
        format: Some(OutputFormat::Elf),
        machine: Some(Machine::Aarch64),
        dynamic: DynamicMode::Static,
        ..Default::default()
    };
    link_typed(&[pa, pb], &out, &opts).expect("aarch64 multi-object relocations");

    let bytes = std::fs::read(&out).unwrap();
    let hdr = parse_elf_hdr(&bytes);
    let shdrs = parse_elf_shdrs(&bytes, &hdr);
    let sh_text = shdrs.iter().find(|s| s.name == ".text").unwrap();

    let parsed = ReadFile::parse(&*bytes).unwrap();
    let start_va = parsed
        .symbols()
        .find(|s| s.name() == Ok("_start"))
        .unwrap()
        .address();
    let helper_va = parsed
        .symbols()
        .find(|s| s.name() == Ok("helper_a64"))
        .unwrap()
        .address();
    let gd_va = parsed
        .symbols()
        .find(|s| s.name() == Ok("global_data"))
        .unwrap()
        .address();

    let s_file = sh_text.sh_offset as usize + (start_va - sh_text.sh_addr) as usize;

    // 1. Check BL helper_a64
    let bl = le_u32(&bytes, s_file);
    let imm26 = ((bl & 0x03FF_FFFF) as i32) << 6 >> 6;
    assert_eq!((start_va as i64 + (imm26 as i64) * 4) as u64, helper_va);

    // 2. Check ADRP x0, global_data
    let adrp = le_u32(&bytes, s_file + 4);
    let immlo = ((adrp >> 29) & 3) as i64;
    let immhi = ((adrp >> 5) & 0x7_ffff) as i64;
    let imm21 = (((immhi << 2) | immlo) << 43) >> 43;
    let adrp_page = ((start_va + 4) & !0xfff) as i64 + (imm21 << 12);
    assert_eq!(adrp_page as u64, gd_va & !0xfff);

    // 3. Check ADD x0, x0, :lo12:global_data
    let add_lo12 = le_u32(&bytes, s_file + 8);
    assert_eq!(((add_lo12 >> 10) & 0xfff) as u64, gd_va & 0xfff);

    // 4. Check LDR x1, [x1, :lo12:global_data] (scaled by 8 -> shift 3)
    let ldr_lo12 = le_u32(&bytes, s_file + 12);
    assert_eq!(((ldr_lo12 >> 10) & 0xfff) as u64, (gd_va & 0xfff) >> 3);

    // 5. Check GOT page + GOT lo12 point to a GOT slot holding gd_va
    let got_adrp = le_u32(&bytes, s_file + 16);
    let got_immlo = ((got_adrp >> 29) & 3) as i64;
    let got_immhi = ((got_adrp >> 5) & 0x7_ffff) as i64;
    let got_imm21 = (((got_immhi << 2) | got_immlo) << 43) >> 43;
    let got_page = ((start_va + 16) & !0xfff) as i64 + (got_imm21 << 12);
    let got_ldr = le_u32(&bytes, s_file + 20);
    let got_off12 = (((got_ldr >> 10) & 0xfff) as i64) << 3;
    let got_slot_va = (got_page + got_off12) as u64;
    // Locate GOT slot in RW PT_LOAD
    let phdrs = parse_elf_phdrs(&bytes, &hdr);
    let rw = phdrs
        .iter()
        .find(|p| p.p_type == elf::PT_LOAD && (p.p_flags & elf::PF_W != 0))
        .unwrap();
    let slot_file = rw.p_offset as usize + (got_slot_va - rw.p_vaddr) as usize;
    assert_eq!(le_u64(&bytes, slot_file), gd_va, "GOT slot holds target VA");
}

#[test]
fn weak_symbol_overridden_by_global_and_weak_vs_weak() {
    let td = TempDir::new("weak_sym");

    // obj_weak: weak `get_val` returning 11
    let mut obj_w1 = WriteObject::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let tw1 = obj_w1.section_id(StandardSection::Text);
    let ow1 = obj_w1.append_section_data(tw1, &[0xb8, 11, 0, 0, 0, 0xc3], 16);
    add_sym(
        &mut obj_w1,
        b"get_val",
        SymbolSection::Section(tw1),
        ow1,
        6,
        SymbolKind::Text,
        SymbolScope::Linkage,
        true,
    );

    // obj_weak2: weak `get_val` returning 22
    let mut obj_w2 = WriteObject::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let tw2 = obj_w2.section_id(StandardSection::Text);
    let ow2 = obj_w2.append_section_data(tw2, &[0xb8, 22, 0, 0, 0, 0xc3], 16);
    add_sym(
        &mut obj_w2,
        b"get_val",
        SymbolSection::Section(tw2),
        ow2,
        6,
        SymbolKind::Text,
        SymbolScope::Linkage,
        true,
    );

    // obj_strong: strong `get_val` returning 77
    let mut obj_s = WriteObject::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let ts = obj_s.section_id(StandardSection::Text);
    let os = obj_s.append_section_data(ts, &[0xb8, 77, 0, 0, 0, 0xc3], 16);
    add_sym(
        &mut obj_s,
        b"get_val",
        SymbolSection::Section(ts),
        os,
        6,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );

    // caller: _start calls get_val and exits with eax
    let mut obj_c = WriteObject::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let tc = obj_c.section_id(StandardSection::Text);
    let code = [
        0xe8, 0, 0, 0, 0, // call get_val
        0x89, 0xc7, // mov edi, eax
        0xb8, 60, 0, 0, 0, // mov eax, 60
        0x0f, 0x05, // syscall
    ];
    let oc = obj_c.append_section_data(tc, &code, 16);
    add_sym(
        &mut obj_c,
        b"_start",
        SymbolSection::Section(tc),
        oc,
        code.len() as u64,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    let gv = undef_sym(&mut obj_c, b"get_val", SymbolKind::Text);
    obj_c
        .add_relocation(
            tc,
            Relocation {
                offset: oc + 1,
                symbol: gv,
                addend: -4,
                flags: RelocationFlags::Elf {
                    r_type: elf::R_X86_64_PLT32,
                },
            },
        )
        .unwrap();

    let pw1 = td.write("w1.o", &obj_w1.write().unwrap());
    let pw2 = td.write("w2.o", &obj_w2.write().unwrap());
    let ps = td.write("strong.o", &obj_s.write().unwrap());
    let pc = td.write("caller.o", &obj_c.write().unwrap());

    let opts = LinkOptions {
        format: Some(OutputFormat::Elf),
        machine: Some(Machine::X86_64),
        dynamic: DynamicMode::Static,
        ..Default::default()
    };

    // Case 1: weak + strong -> strong wins (77)
    let out_strong = td.out("weak_strong");
    link_typed(&[pw1.clone(), ps, pc.clone()], &out_strong, &opts)
        .expect("weak + strong must resolve to strong");
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        assert_eq!(exec_linux_binary(&out_strong), 77);
    }

    // Case 2: weak + weak -> first weak wins (11), no duplicate error
    let out_weak = td.out("weak_weak");
    link_typed(&[pw1, pw2, pc], &out_weak, &opts).expect("weak + weak must succeed");
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        assert_eq!(exec_linux_binary(&out_weak), 11);
    }
}

#[test]
fn static_archive_selective_and_chained_member_extraction() {
    let td = TempDir::new("ar_extract");

    // Member 1: defines `lib_a` which calls `lib_b`
    let mut m1 = WriteObject::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let t1 = m1.section_id(StandardSection::Text);
    let o1 = m1.append_section_data(t1, &[0xe9, 0, 0, 0, 0], 16); // jmp lib_b
    add_sym(
        &mut m1,
        b"lib_a",
        SymbolSection::Section(t1),
        o1,
        5,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    let lb_undef = m1.add_symbol(Symbol {
        name: b"lib_b".to_vec(),
        value: 0,
        size: 0,
        kind: SymbolKind::Text,
        scope: SymbolScope::Dynamic,
        weak: false,
        section: SymbolSection::Undefined,
        flags: SymbolFlags::Elf {
            st_info: (elf::STB_GLOBAL << 4) | elf::STT_FUNC,
            st_other: 0,
        },
    });
    m1.add_relocation(
        t1,
        Relocation {
            offset: o1 + 1,
            symbol: lb_undef,
            addend: -4,
            flags: RelocationFlags::Elf {
                r_type: elf::R_X86_64_PLT32,
            },
        },
    )
    .unwrap();

    // Member 2: defines `lib_b` returning 55
    let mut m2 = WriteObject::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let t2 = m2.section_id(StandardSection::Text);
    let o2 = m2.append_section_data(t2, &[0xb8, 55, 0, 0, 0, 0xc3], 16);
    add_sym(
        &mut m2,
        b"lib_b",
        SymbolSection::Section(t2),
        o2,
        6,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );

    // Member 3: unused member with an unresolved symbol `never_needed`
    // (must NOT be pulled in, proving selective extraction!)
    let mut m3 = WriteObject::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let t3 = m3.section_id(StandardSection::Text);
    let o3 = m3.append_section_data(t3, &[0xe8, 0, 0, 0, 0, 0xc3], 16);
    add_sym(
        &mut m3,
        b"unused_fn",
        SymbolSection::Section(t3),
        o3,
        6,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    let nn = undef_sym(&mut m3, b"never_needed", SymbolKind::Text);
    m3.add_relocation(
        t3,
        Relocation {
            offset: o3 + 1,
            symbol: nn,
            addend: -4,
            flags: RelocationFlags::Elf {
                r_type: elf::R_X86_64_PLT32,
            },
        },
    )
    .unwrap();

    // Reverse order inside archive (m2 before m1) to test fixed-point archive loop
    let ar_bytes = build_ar_archive(&[
        ("m3.o", &m3.write().unwrap()),
        ("m2.o", &m2.write().unwrap()),
        ("m1.o", &m1.write().unwrap()),
    ]);
    let ar_path = td.write("libtest.a", &ar_bytes);

    // Caller _start calls lib_a and exits with eax
    let mut main_o = WriteObject::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let tm = main_o.section_id(StandardSection::Text);
    let code = [
        0xe8, 0, 0, 0, 0, // call lib_a
        0x89, 0xc7, // mov edi, eax
        0xb8, 60, 0, 0, 0, // mov eax, 60
        0x0f, 0x05, // syscall
    ];
    let om = main_o.append_section_data(tm, &code, 16);
    add_sym(
        &mut main_o,
        b"_start",
        SymbolSection::Section(tm),
        om,
        code.len() as u64,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    let la = main_o.add_symbol(Symbol {
        name: b"lib_a".to_vec(),
        value: 0,
        size: 0,
        kind: SymbolKind::Text,
        scope: SymbolScope::Dynamic,
        weak: false,
        section: SymbolSection::Undefined,
        flags: SymbolFlags::Elf {
            st_info: (elf::STB_GLOBAL << 4) | elf::STT_FUNC,
            st_other: 0,
        },
    });
    main_o
        .add_relocation(
            tm,
            Relocation {
                offset: om + 1,
                symbol: la,
                addend: -4,
                flags: RelocationFlags::Elf {
                    r_type: elf::R_X86_64_PLT32,
                },
            },
        )
        .unwrap();
    let main_path = td.write("main.o", &main_o.write().unwrap());
    let out = td.out("ar_out");

    let opts = LinkOptions {
        format: Some(OutputFormat::Elf),
        machine: Some(Machine::X86_64),
        dynamic: DynamicMode::Static,
        ..Default::default()
    };
    link_typed(&[main_path, ar_path], &out, &opts).expect("archive link must succeed");

    let bytes = std::fs::read(&out).unwrap();
    let parsed = ReadFile::parse(&*bytes).unwrap();
    assert!(parsed.symbols().any(|s| s.name() == Ok("lib_a")));
    assert!(parsed.symbols().any(|s| s.name() == Ok("lib_b")));
    assert!(!parsed.symbols().any(|s| s.name() == Ok("unused_fn")));

    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        assert_eq!(exec_linux_binary(&out), 55);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 4. PE/COFF (x86_64 & aarch64) — Subsystems, .bss RVA, Relocs, IAT, Checksum
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn pe_subsystems_console_windows_native_and_dll() {
    let td = TempDir::new("pe_sub");
    let obj_main = td.write(
        "main.obj",
        &make_minimal_main_obj(BinaryFormat::Coff, Architecture::X86_64),
    );

    // Explicit Console (3), Windows GUI (2), Native (1)
    for (sub, expected_val, tag) in [
        (PeSubsystem::Console, 3u16, "console"),
        (PeSubsystem::Windows, 2u16, "gui"),
        (PeSubsystem::Native, 1u16, "native"),
    ] {
        let out = td.out(&format!("{tag}.exe"));
        let opts = LinkOptions {
            format: Some(OutputFormat::Pe),
            machine: Some(Machine::X86_64),
            subsystem: Some(sub),
            ..Default::default()
        };
        link_typed(&[obj_main.clone()], &out, &opts).expect("PE link");
        let b = std::fs::read(&out).unwrap();
        assert_eq!(&b[0..2], b"MZ");
        let nt = le_u32(&b, 0x3c) as usize;
        assert_eq!(&b[nt..nt + 4], b"PE\0\0");
        let opt = nt + 24;
        assert_eq!(le_u16(&b, opt), 0x20b, "PE32+ magic");
        assert_eq!(le_u16(&b, opt + 68), expected_val, "subsystem {tag}");
        // Non-zero PE checksum
        assert_ne!(le_u32(&b, opt + 64), 0, "PE checksum must be computed");
    }

    // Auto-detected GUI when WinMain is present
    let mut win_obj =
        WriteObject::new(BinaryFormat::Coff, Architecture::X86_64, Endianness::Little);
    let tw = win_obj.section_id(StandardSection::Text);
    let ow = win_obj.append_section_data(tw, &[0x31, 0xc0, 0xc3], 16);
    add_sym(
        &mut win_obj,
        b"WinMain",
        SymbolSection::Section(tw),
        ow,
        3,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    let win_p = td.write("win.obj", &win_obj.write().unwrap());
    let win_out = td.out("auto_gui.exe");
    let opts_auto = LinkOptions {
        format: Some(OutputFormat::Pe),
        machine: Some(Machine::X86_64),
        ..Default::default()
    };
    link_typed(&[win_p], &win_out, &opts_auto).expect("WinMain auto GUI");
    let wb = std::fs::read(&win_out).unwrap();
    let w_opt = le_u32(&wb, 0x3c) as usize + 24;
    assert_eq!(
        le_u16(&wb, w_opt + 68),
        2,
        "WinMain defaults to Windows GUI"
    );

    // Shared DLL sets IMAGE_FILE_DLL (0x2000)
    let dll_out = td.out("test.dll");
    let opts_dll = LinkOptions {
        format: Some(OutputFormat::Pe),
        machine: Some(Machine::X86_64),
        shared: true,
        ..Default::default()
    };
    link_typed(&[obj_main], &dll_out, &opts_dll).expect("PE DLL link");
    let db = std::fs::read(&dll_out).unwrap();
    let d_nt = le_u32(&db, 0x3c) as usize;
    let chars = le_u16(&db, d_nt + 22);
    assert_ne!(chars & 0x2000, 0, "IMAGE_FILE_DLL must be set");
}

#[test]
fn pe_bss_ordering_after_data_relocs_and_extra_imports() {
    let td = TempDir::new("pe_bss_reloc");

    // Object 1: defines .data `g_data` (16 bytes), .bss `g_bss` (256 bytes),
    // and calls custom DLL import `my_custom_dll_fn` + `MessageBoxA` (USER32.dll)
    let mut obj1 = WriteObject::new(BinaryFormat::Coff, Architecture::X86_64, Endianness::Little);
    let t1 = obj1.section_id(StandardSection::Text);
    let r1 = obj1.section_id(StandardSection::ReadOnlyData);
    let d1 = obj1.section_id(StandardSection::Data);
    let b1 = obj1.section_id(StandardSection::UninitializedData);

    let ro_off = obj1.append_section_data(r1, b"Hello GUI\0", 8);
    let ro_sym = add_sym(
        &mut obj1,
        b"banner",
        SymbolSection::Section(r1),
        ro_off,
        10,
        SymbolKind::Data,
        SymbolScope::Linkage,
        false,
    );
    let da_off = obj1.append_section_data(d1, &[0u8; 16], 16);
    add_sym(
        &mut obj1,
        b"g_data",
        SymbolSection::Section(d1),
        da_off,
        16,
        SymbolKind::Data,
        SymbolScope::Linkage,
        false,
    );
    // ADDR64 relocation in .data pointing to banner (triggers .reloc base relocations!)
    obj1.add_relocation(
        d1,
        Relocation {
            offset: da_off,
            symbol: ro_sym,
            addend: 0,
            flags: RelocationFlags::Generic {
                kind: RelocationKind::Absolute,
                encoding: RelocationEncoding::Generic,
                size: 64,
            },
        },
    )
    .unwrap();

    let bss_off = obj1.append_section_bss(b1, 256, 16);
    let bss_sym = add_sym(
        &mut obj1,
        b"g_bss",
        SymbolSection::Section(b1),
        bss_off,
        256,
        SymbolKind::Data,
        SymbolScope::Linkage,
        false,
    );

    // main: lea rax, [rip+g_bss]; call MessageBoxA; call my_custom_dll_fn; ret
    let code = [
        0x48, 0x8d, 0x05, 0, 0, 0, 0, // lea rax, [rip + disp32] (rel @ +3)
        0xe8, 0, 0, 0, 0, // call MessageBoxA (rel @ +8)
        0xe8, 0, 0, 0, 0, // call my_custom_dll_fn (rel @ +13)
        0xc3,
    ];
    let t_off = obj1.append_section_data(t1, &code, 16);
    add_sym(
        &mut obj1,
        b"main",
        SymbolSection::Section(t1),
        t_off,
        code.len() as u64,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    let msgbox = undef_sym(&mut obj1, b"MessageBoxA", SymbolKind::Text);
    let custom = undef_sym(&mut obj1, b"my_custom_dll_fn", SymbolKind::Text);

    for (off, sym) in [
        (t_off + 3, bss_sym),
        (t_off + 8, msgbox),
        (t_off + 13, custom),
    ] {
        obj1.add_relocation(
            t1,
            Relocation {
                offset: off,
                symbol: sym,
                addend: 0, // COFF reads addend from section bytes (0)
                flags: RelocationFlags::Generic {
                    kind: RelocationKind::Relative,
                    encoding: RelocationEncoding::X86Branch,
                    size: 32,
                },
            },
        )
        .unwrap();
    }

    let in_obj = td.write("pe_full.obj", &obj1.write().unwrap());
    let out = td.out("pe_full.exe");
    let mut extra_imports = HashMap::new();
    extra_imports.insert("my_custom_dll_fn".to_string(), "CUSTOM_MOD.dll".to_string());

    let opts = LinkOptions {
        format: Some(OutputFormat::Pe),
        machine: Some(Machine::X86_64),
        subsystem: Some(PeSubsystem::Windows),
        extra_imports,
        ..Default::default()
    };
    link_typed(&[in_obj], &out, &opts).expect("PE full link");

    let bytes = std::fs::read(&out).unwrap();
    let parsed = ReadFile::parse(&*bytes).unwrap();
    assert_eq!(parsed.format(), BinaryFormat::Pe);

    // Check section RVAs: .text < .rdata < .data < .idata < .reloc, and .data VirtualSize >= 16 + 256
    let sec_rva = |name: &str| -> (u64, u64) {
        let s = parsed.sections().find(|s| s.name() == Ok(name)).unwrap();
        (s.address() - 0x14000_0000, s.size())
    };
    let (text_rva, _) = sec_rva(".text");
    let (rdata_rva, _) = sec_rva(".rdata");
    let (data_rva, data_vsz) = sec_rva(".data");
    let (idata_rva, _) = sec_rva(".idata");
    let (reloc_rva, _) = sec_rva(".reloc");

    assert!(text_rva < rdata_rva);
    assert!(rdata_rva < data_rva);
    assert!(data_vsz >= 16 + 256, ".data VirtualSize must include .bss");
    assert!(
        idata_rva >= data_rva + data_vsz,
        ".idata RVA must be strictly after .data+.bss"
    );
    assert!(reloc_rva > idata_rva);

    // Verify lea rax, [rip+g_bss] targets RVA >= data_rva + 16 and < data_rva + data_vsz
    let text_sec = parsed.sections().find(|s| s.name() == Ok(".text")).unwrap();
    let text_bytes = text_sec.data().unwrap();
    let disp = le_i32(text_bytes, 3);
    let target_rva = (text_rva as i64 + 7 + disp as i64) as u64;
    assert!(
        target_rva >= data_rva + 16 && target_rva < data_rva + data_vsz,
        "g_bss target RVA 0x{target_rva:x} must sit in .bss tail of .data [0x{:x}..0x{:x})",
        data_rva + 16,
        data_rva + data_vsz
    );

    // Verify ADDR64 in .data points to .rdata banner
    let data_sec = parsed.sections().find(|s| s.name() == Ok(".data")).unwrap();
    let banner_va = le_u64(data_sec.data().unwrap(), 0);
    assert_eq!(banner_va, 0x14000_0000 + rdata_rva);

    // Verify DYNAMIC_BASE (0x0040) is set when .reloc is present
    let opt = le_u32(&bytes, 0x3c) as usize + 24;
    let dll_chars = le_u16(&bytes, opt + 70);
    assert_ne!(dll_chars & 0x0040, 0, "DYNAMIC_BASE set with .reloc");

    // Verify imports from both USER32.dll and CUSTOM_MOD.dll
    let imports = parsed.imports().unwrap();
    assert!(
        imports
            .iter()
            .any(|i| i.name() == b"MessageBoxA" && i.library().eq_ignore_ascii_case(b"USER32.dll"))
    );
    assert!(imports.iter().any(|i| {
        i.name() == b"my_custom_dll_fn" && i.library().eq_ignore_ascii_case(b"CUSTOM_MOD.dll")
    }));
}

#[test]
fn pe_aarch64_image_and_branch26_relocation() {
    let td = TempDir::new("pe_a64");
    let mut obj = WriteObject::new(
        BinaryFormat::Coff,
        Architecture::Aarch64,
        Endianness::Little,
    );
    let text = obj.section_id(StandardSection::Text);

    // main: bl ExitProcess ; ret
    let mut code = Vec::new();
    code.extend_from_slice(&0x9400_0000u32.to_le_bytes());
    code.extend_from_slice(&0xd65f_03c0u32.to_le_bytes());
    let off = obj.append_section_data(text, &code, 16);
    add_sym(
        &mut obj,
        b"main",
        SymbolSection::Section(text),
        off,
        8,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    let ep = undef_sym(&mut obj, b"ExitProcess", SymbolKind::Text);
    obj.add_relocation(
        text,
        Relocation {
            offset: off,
            symbol: ep,
            addend: 0,
            flags: RelocationFlags::Coff { typ: 0x0003 }, // IMAGE_REL_ARM64_BRANCH26
        },
    )
    .unwrap();

    let in_obj = td.write("a64.obj", &obj.write().unwrap());
    let out = td.out("a64.exe");
    let opts = LinkOptions {
        format: Some(OutputFormat::Pe),
        machine: Some(Machine::Aarch64),
        subsystem: Some(PeSubsystem::Console),
        ..Default::default()
    };
    link_typed(&[in_obj], &out, &opts).expect("PE aarch64 link");

    let bytes = std::fs::read(&out).unwrap();
    let parsed = ReadFile::parse(&*bytes).unwrap();
    assert_eq!(parsed.format(), BinaryFormat::Pe);
    assert_eq!(parsed.architecture(), Architecture::Aarch64);
    assert!(
        parsed
            .imports()
            .unwrap()
            .iter()
            .any(|i| i.name() == b"ExitProcess")
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 5. Mach-O (x86_64 & aarch64) — Magic, Segments, Codesig, Relocs, Dylib Policy
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn macho_x86_64_multi_object_relocs_codesig_and_dylib() {
    let td = TempDir::new("macho_x64");

    let mut obj_a = WriteObject::new(
        BinaryFormat::MachO,
        Architecture::X86_64,
        Endianness::Little,
    );
    let ta = obj_a.section_id(StandardSection::Text);
    let da = obj_a.section_id(StandardSection::Data);

    let g_off = obj_a.append_section_data(da, &1234u64.to_le_bytes(), 8);
    let g_sym = add_sym(
        &mut obj_a,
        b"g_val",
        SymbolSection::Section(da),
        g_off,
        8,
        SymbolKind::Data,
        SymbolScope::Linkage,
        false,
    );
    let h_off = obj_a.append_section_data(ta, &[0xb8, 5, 0, 0, 0, 0xc3], 16);
    add_sym(
        &mut obj_a,
        b"helper_fn",
        SymbolSection::Section(ta),
        h_off,
        6,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    let p_off = obj_a.append_section_data(da, &[0u8; 8], 8);
    add_sym(
        &mut obj_a,
        b"g_ptr",
        SymbolSection::Section(da),
        p_off,
        8,
        SymbolKind::Data,
        SymbolScope::Linkage,
        false,
    );
    obj_a
        .add_relocation(
            da,
            Relocation {
                offset: p_off,
                symbol: g_sym,
                addend: 0,
                flags: RelocationFlags::MachO {
                    r_type: 0, // X86_64_RELOC_UNSIGNED
                    r_pcrel: false,
                    r_length: 3,
                },
            },
        )
        .unwrap();

    let mut obj_b = WriteObject::new(
        BinaryFormat::MachO,
        Architecture::X86_64,
        Endianness::Little,
    );
    let tb = obj_b.section_id(StandardSection::Text);
    // _main: call _helper_fn; call _exit; ret
    let code = [0xe8, 0, 0, 0, 0, 0xe8, 0, 0, 0, 0, 0xc3];
    let mb_off = obj_b.append_section_data(tb, &code, 16);
    add_sym(
        &mut obj_b,
        b"main",
        SymbolSection::Section(tb),
        mb_off,
        code.len() as u64,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    let h_ext = undef_sym(&mut obj_b, b"helper_fn", SymbolKind::Text);
    let exit_ext = undef_sym(&mut obj_b, b"exit", SymbolKind::Text);
    obj_b
        .add_relocation(
            tb,
            Relocation {
                offset: mb_off + 1,
                symbol: h_ext,
                addend: 0,
                flags: RelocationFlags::MachO {
                    r_type: 2, // X86_64_RELOC_BRANCH
                    r_pcrel: true,
                    r_length: 2,
                },
            },
        )
        .unwrap();
    obj_b
        .add_relocation(
            tb,
            Relocation {
                offset: mb_off + 6,
                symbol: exit_ext,
                addend: 0,
                flags: RelocationFlags::MachO {
                    r_type: 2,
                    r_pcrel: true,
                    r_length: 2,
                },
            },
        )
        .unwrap();

    let pa = td.write("a.o", &obj_a.write().unwrap());
    let pb = td.write("b.o", &obj_b.write().unwrap());
    let out = td.out("macho_exec");
    let opts = LinkOptions {
        format: Some(OutputFormat::Macho),
        machine: Some(Machine::X86_64),
        ..Default::default()
    };
    link_typed(&[pa.clone(), pb], &out, &opts).expect("Mach-O x86_64 multi-object link");

    let bytes = std::fs::read(&out).unwrap();
    assert_eq!(le_u32(&bytes, 0), 0xFEED_FACF, "MH_MAGIC_64");
    assert_eq!(le_u32(&bytes, 4), 0x0100_0007, "CPU_TYPE_X86_64");
    assert_eq!(le_u32(&bytes, 12), 2, "MH_EXECUTE");

    // Verify ad-hoc embedded signature SuperBlob (0xFADE0CC0) + CodeDirectory (0xFADE0C02)
    let has_codesig = bytes
        .windows(4)
        .any(|w| u32::from_be_bytes(w.try_into().unwrap()) == 0xFADE_0CC0);
    let has_cd = bytes
        .windows(4)
        .any(|w| u32::from_be_bytes(w.try_into().unwrap()) == 0xFADE_0C02);
    assert!(has_codesig && has_cd, "Mach-O must carry ad-hoc codesig");

    let parsed = ReadFile::parse(&*bytes).unwrap();
    assert_eq!(parsed.format(), BinaryFormat::MachO);
    assert_eq!(parsed.architecture(), Architecture::X86_64);
    assert!(parsed.entry() > 0);
    assert!(parsed.sections().any(|s| s.name() == Ok("__text")));
    assert!(parsed.sections().any(|s| s.name() == Ok("__data")));

    // Verify dylib build (MH_DYLIB = 6)
    let dylib_out = td.out("libfoo.dylib");
    let dylib_opts = LinkOptions {
        format: Some(OutputFormat::Macho),
        machine: Some(Machine::X86_64),
        shared: true,
        soname: Some("@rpath/libfoo.dylib".into()),
        ..Default::default()
    };
    link_typed(&[pa], &dylib_out, &dylib_opts).expect("Mach-O dylib link");
    let dbytes = std::fs::read(&dylib_out).unwrap();
    assert_eq!(le_u32(&dbytes, 12), 6, "MH_DYLIB");
}

#[test]
fn macho_aarch64_with_system_library_and_policy_check() {
    let td = TempDir::new("macho_a64");
    let mut obj = WriteObject::new(
        BinaryFormat::MachO,
        Architecture::Aarch64,
        Endianness::Little,
    );
    let text = obj.section_id(StandardSection::Text);
    let data = obj.section_id(StandardSection::Data);

    let d_off = obj.append_section_data(data, &42u64.to_le_bytes(), 8);
    let d_sym = add_sym(
        &mut obj,
        b"answer",
        SymbolSection::Section(data),
        d_off,
        8,
        SymbolKind::Data,
        SymbolScope::Linkage,
        false,
    );

    // _main: adrp x0, _answer@PAGE; add x0, x0, _answer@PAGEOFF; bl _exit; ret
    let mut code = Vec::new();
    code.extend_from_slice(&0x9000_0000u32.to_le_bytes()); // +0: adrp x0, ...
    code.extend_from_slice(&0x9100_0000u32.to_le_bytes()); // +4: add x0, x0, ...
    code.extend_from_slice(&0x9400_0000u32.to_le_bytes()); // +8: bl _exit
    code.extend_from_slice(&0xd65f_03c0u32.to_le_bytes()); // +12: ret
    let t_off = obj.append_section_data(text, &code, 16);
    add_sym(
        &mut obj,
        b"main",
        SymbolSection::Section(text),
        t_off,
        code.len() as u64,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    let exit_sym = undef_sym(&mut obj, b"exit", SymbolKind::Text);

    obj.add_relocation(
        text,
        Relocation {
            offset: t_off,
            symbol: d_sym,
            addend: 0,
            flags: RelocationFlags::MachO {
                r_type: 3, // ARM64_RELOC_PAGE21
                r_pcrel: true,
                r_length: 2,
            },
        },
    )
    .unwrap();
    obj.add_relocation(
        text,
        Relocation {
            offset: t_off + 4,
            symbol: d_sym,
            addend: 0,
            flags: RelocationFlags::MachO {
                r_type: 4, // ARM64_RELOC_PAGEOFF12
                r_pcrel: false,
                r_length: 2,
            },
        },
    )
    .unwrap();
    obj.add_relocation(
        text,
        Relocation {
            offset: t_off + 8,
            symbol: exit_sym,
            addend: 0,
            flags: RelocationFlags::MachO {
                r_type: 2, // ARM64_RELOC_BRANCH26
                r_pcrel: true,
                r_length: 2,
            },
        },
    )
    .unwrap();

    let in_obj = td.write("a64.o", &obj.write().unwrap());
    let out = td.out("a64_macho");

    // Without `-l System` / needed, ARM64 Mach-O executable must fail with policy error
    let opts_no_sys = LinkOptions {
        format: Some(OutputFormat::Macho),
        machine: Some(Machine::Aarch64),
        ..Default::default()
    };
    let err = link_typed(&[in_obj.clone()], &out, &opts_no_sys)
        .expect_err("ARM64 Mach-O without libSystem must be rejected");
    assert!(err.message.contains("libSystem"));

    // With `-l System`, succeeds and emits valid ARM64 Mach-O
    let opts_sys = LinkOptions {
        format: Some(OutputFormat::Macho),
        machine: Some(Machine::Aarch64),
        libraries: vec!["System".into()],
        ..Default::default()
    };
    link_typed(&[in_obj], &out, &opts_sys).expect("ARM64 Mach-O with -l System succeeds");

    let bytes = std::fs::read(&out).unwrap();
    assert_eq!(le_u32(&bytes, 0), 0xFEED_FACF);
    assert_eq!(le_u32(&bytes, 4), 0x0100_000C, "CPU_TYPE_ARM64");
    let parsed = ReadFile::parse(&*bytes).unwrap();
    assert_eq!(parsed.architecture(), Architecture::Aarch64);
    assert!(
        bytes
            .windows(b"/usr/lib/libSystem.B.dylib".len())
            .any(|w| w == b"/usr/lib/libSystem.B.dylib"),
        "must emit LC_LOAD_DYLIB for libSystem"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 6. Negative Error Handling, Overflows & Utility APIs
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn undefined_symbols_report_exact_names_across_formats() {
    let td = TempDir::new("neg_undef");

    let make_undef = |fmt: BinaryFormat, arch: Architecture, sym_name: &[u8]| -> Vec<u8> {
        let mut obj = WriteObject::new(fmt, arch, Endianness::Little);
        let text = obj.section_id(StandardSection::Text);
        let code: Vec<u8> = match arch {
            Architecture::X86_64 => vec![0xe8, 0, 0, 0, 0, 0xc3],
            Architecture::Aarch64 => vec![0x00, 0x00, 0x00, 0x94, 0xc0, 0x03, 0x5f, 0xd6],
            _ => unreachable!(),
        };
        let off = obj.append_section_data(text, &code, 16);
        add_sym(
            &mut obj,
            b"main",
            SymbolSection::Section(text),
            off,
            code.len() as u64,
            SymbolKind::Text,
            SymbolScope::Linkage,
            false,
        );
        let target = undef_sym(&mut obj, sym_name, SymbolKind::Text);
        let flags = match fmt {
            BinaryFormat::Elf => RelocationFlags::Elf {
                r_type: elf::R_X86_64_PLT32,
            },
            BinaryFormat::Coff => RelocationFlags::Generic {
                kind: RelocationKind::Relative,
                encoding: RelocationEncoding::X86Branch,
                size: 32,
            },
            BinaryFormat::MachO => RelocationFlags::MachO {
                r_type: 2,
                r_pcrel: true,
                r_length: 2,
            },
            _ => unreachable!(),
        };
        obj.add_relocation(
            text,
            Relocation {
                offset: if arch == Architecture::X86_64 {
                    off + 1
                } else {
                    off
                },
                symbol: target,
                addend: if fmt == BinaryFormat::Elf { -4 } else { 0 },
                flags,
            },
        )
        .unwrap();
        obj.write().unwrap()
    };

    // ELF Static
    let elf_in = td.write(
        "u.o",
        &make_undef(BinaryFormat::Elf, Architecture::X86_64, b"missing_elf_sym"),
    );
    let err_elf = link_typed(
        &[elf_in],
        &td.out("e.bin"),
        &LinkOptions {
            format: Some(OutputFormat::Elf),
            machine: Some(Machine::X86_64),
            dynamic: DynamicMode::Static,
            ..Default::default()
        },
    )
    .expect_err("static ELF undefined must fail");
    assert_eq!(err_elf.kind, LinkErrorKind::Unresolved);
    assert!(err_elf.unresolved.contains(&"missing_elf_sym".to_string()));

    // PE Auto (unknown symbol not in Windows DLL table)
    let pe_in = td.write(
        "u.obj",
        &make_undef(
            BinaryFormat::Coff,
            Architecture::X86_64,
            b"totally_unknown_win_sym",
        ),
    );
    let err_pe = link_typed(
        &[pe_in],
        &td.out("p.exe"),
        &LinkOptions {
            format: Some(OutputFormat::Pe),
            machine: Some(Machine::X86_64),
            dynamic: DynamicMode::Auto,
            ..Default::default()
        },
    )
    .expect_err("unknown PE symbol must fail");
    assert_eq!(err_pe.kind, LinkErrorKind::Unresolved);
    assert!(
        err_pe
            .unresolved
            .contains(&"totally_unknown_win_sym".to_string())
    );

    // Mach-O Static
    let mac_in = td.write(
        "u_mac.o",
        &make_undef(
            BinaryFormat::MachO,
            Architecture::X86_64,
            b"missing_macho_sym",
        ),
    );
    let err_mac = link_typed(
        &[mac_in],
        &td.out("m.out"),
        &LinkOptions {
            format: Some(OutputFormat::Macho),
            machine: Some(Machine::X86_64),
            dynamic: DynamicMode::Static,
            ..Default::default()
        },
    )
    .expect_err("static Mach-O undefined must fail");
    assert_eq!(err_mac.kind, LinkErrorKind::Unresolved);
    assert!(
        err_mac
            .unresolved
            .contains(&"missing_macho_sym".to_string())
    );
}

#[test]
fn duplicate_strong_symbols_fail_unless_allowed() {
    let td = TempDir::new("dup_sym");
    let a = td.write(
        "a.o",
        &make_minimal_main_obj(BinaryFormat::Elf, Architecture::X86_64),
    );
    let b = td.write(
        "b.o",
        &make_minimal_main_obj(BinaryFormat::Elf, Architecture::X86_64),
    );

    let opts_strict = LinkOptions {
        format: Some(OutputFormat::Elf),
        machine: Some(Machine::X86_64),
        dynamic: DynamicMode::Static,
        ..Default::default()
    };
    let err = link_typed(&[a.clone(), b.clone()], &td.out("dup.bin"), &opts_strict)
        .expect_err("duplicate global symbol must fail");
    assert!(err.message.contains("duplicate definition"));
    assert!(err.message.contains("main"));

    let opts_allow = LinkOptions {
        format: Some(OutputFormat::Elf),
        machine: Some(Machine::X86_64),
        dynamic: DynamicMode::Static,
        allow_multiple_definition: true,
        ..Default::default()
    };
    link_typed(&[a, b], &td.out("dup_ok.bin"), &opts_allow)
        .expect("allow_multiple_definition must permit duplicate");
}

#[test]
fn malformed_objects_and_invalid_relocations_fail_cleanly() {
    let td = TempDir::new("malformed_neg");

    // 1. Truncated ELF header
    let trunc_elf = td.write("trunc.o", b"\x7fELF\x02\x01\x01\x00");
    let e1 = link_typed(&[trunc_elf], &td.out("o1"), &LinkOptions::default())
        .expect_err("truncated ELF must error");
    assert!(!e1.message.is_empty());

    // 2. Corrupt AR archive header
    let bad_ar = td.write(
        "bad.a",
        b"!<arch>\ncorrupt_member_header_without_end_marker",
    );
    let e2 = link_typed(&[bad_ar], &td.out("o2"), &LinkOptions::default())
        .expect_err("corrupt archive must error");
    assert!(!e2.message.is_empty());

    // 3. Unsupported ELF relocation raw type (e.g. 9999)
    let mut bad_rel_obj =
        WriteObject::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let text = bad_rel_obj.section_id(StandardSection::Text);
    let off = bad_rel_obj.append_section_data(text, &[0, 0, 0, 0, 0xc3], 16);
    let s = add_sym(
        &mut bad_rel_obj,
        b"_start",
        SymbolSection::Section(text),
        off,
        5,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    bad_rel_obj
        .add_relocation(
            text,
            Relocation {
                offset: off,
                symbol: s,
                addend: 0,
                flags: RelocationFlags::Elf { r_type: 9999 },
            },
        )
        .unwrap();
    let bad_rel_p = td.write("badrel.o", &bad_rel_obj.write().unwrap());
    let e3 = link_typed(
        &[bad_rel_p],
        &td.out("o3"),
        &LinkOptions {
            format: Some(OutputFormat::Elf),
            machine: Some(Machine::X86_64),
            dynamic: DynamicMode::Static,
            ..Default::default()
        },
    )
    .expect_err("unsupported reloc type must error");
    assert!(e3.message.contains("unsupported relocation"));

    // 4. Relocation overflow: R_X86_64_8 against VA >= 0x400000 (cannot fit in i8)
    let mut ovf_obj = WriteObject::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let to = ovf_obj.section_id(StandardSection::Text);
    let oo = ovf_obj.append_section_data(to, &[0u8; 8], 16);
    let so = add_sym(
        &mut ovf_obj,
        b"_start",
        SymbolSection::Section(to),
        oo,
        8,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    ovf_obj
        .add_relocation(
            to,
            Relocation {
                offset: oo,
                symbol: so,
                addend: 0,
                flags: RelocationFlags::Elf {
                    r_type: elf::R_X86_64_8,
                },
            },
        )
        .unwrap();
    let ovf_p = td.write("ovf.o", &ovf_obj.write().unwrap());
    let e4 = link_typed(
        &[ovf_p],
        &td.out("o4"),
        &LinkOptions {
            format: Some(OutputFormat::Elf),
            machine: Some(Machine::X86_64),
            dynamic: DynamicMode::Static,
            ..Default::default()
        },
    )
    .expect_err("R_X86_64_8 overflow must fail");
    assert_eq!(e4.kind, LinkErrorKind::RelocationOverflow);

    // 5. Relocation out-of-bounds offset inside section
    let mut oob_obj = WriteObject::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let toob = oob_obj.section_id(StandardSection::Text);
    let ooob = oob_obj.append_section_data(toob, &[0xc3], 16);
    let soob = add_sym(
        &mut oob_obj,
        b"_start",
        SymbolSection::Section(toob),
        ooob,
        1,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    oob_obj
        .add_relocation(
            toob,
            Relocation {
                offset: 64,
                symbol: soob,
                addend: 0,
                flags: RelocationFlags::Elf {
                    r_type: elf::R_X86_64_64,
                },
            },
        )
        .unwrap();
    let oob_p = td.write("oob.o", &oob_obj.write().unwrap());
    let e5 = link_typed(
        &[oob_p],
        &td.out("o5"),
        &LinkOptions {
            format: Some(OutputFormat::Elf),
            machine: Some(Machine::X86_64),
            dynamic: DynamicMode::Static,
            ..Default::default()
        },
    )
    .expect_err("OOB relocation offset must fail");
    assert!(!e5.message.is_empty());

    // 6. Relocation inside .bss (illegal)
    let mut bss_rel = WriteObject::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let tb = bss_rel.section_id(StandardSection::Text);
    let bb = bss_rel.section_id(StandardSection::UninitializedData);
    let ot = bss_rel.append_section_data(tb, &[0xc3], 16);
    let st = add_sym(
        &mut bss_rel,
        b"_start",
        SymbolSection::Section(tb),
        ot,
        1,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    let _ob = bss_rel.append_section_bss(bb, 16, 8);
    bss_rel
        .add_relocation(
            bb,
            Relocation {
                offset: 0,
                symbol: st,
                addend: 0,
                flags: RelocationFlags::Elf {
                    r_type: elf::R_X86_64_64,
                },
            },
        )
        .unwrap();
    let br_p = td.write("bssrel.o", &bss_rel.write().unwrap());
    let e6 = link_typed(
        &[br_p],
        &td.out("o6"),
        &LinkOptions {
            format: Some(OutputFormat::Elf),
            machine: Some(Machine::X86_64),
            dynamic: DynamicMode::Static,
            ..Default::default()
        },
    )
    .expect_err("relocation against BSS must fail");
    assert!(e6.message.contains("BSS"));

    // 7. Missing entry point (`foo` only, no `main` or `_start`)
    let mut no_entry =
        WriteObject::new(BinaryFormat::Elf, Architecture::X86_64, Endianness::Little);
    let tn = no_entry.section_id(StandardSection::Text);
    let on = no_entry.append_section_data(tn, &[0xc3], 16);
    add_sym(
        &mut no_entry,
        b"not_main",
        SymbolSection::Section(tn),
        on,
        1,
        SymbolKind::Text,
        SymbolScope::Linkage,
        false,
    );
    let ne_p = td.write("noentry.o", &no_entry.write().unwrap());
    let e7 = link_typed(
        &[ne_p],
        &td.out("o7"),
        &LinkOptions {
            format: Some(OutputFormat::Elf),
            machine: Some(Machine::X86_64),
            dynamic: DynamicMode::Static,
            ..Default::default()
        },
    )
    .expect_err("missing entry symbol must fail");
    assert!(e7.message.contains("not found") || e7.message.contains("entry"));
}

#[test]
fn format_sniffing_resolve_format_response_files_and_cli_wrappers() {
    let td = TempDir::new("util_api");
    let elf_p = td.write("e.o", &make_x64_exit_start_obj(0));
    let pe_p = td.write(
        "p.obj",
        &make_minimal_main_obj(BinaryFormat::Coff, Architecture::X86_64),
    );
    let mac_p = td.write(
        "m.o",
        &make_minimal_main_obj(BinaryFormat::MachO, Architecture::X86_64),
    );

    assert_eq!(sniff_format(&elf_p), "elf");
    assert_eq!(sniff_format(&pe_p), "pe");
    assert_eq!(sniff_format(&mac_p), "macho");

    assert_eq!(
        resolve_format(std::slice::from_ref(&elf_p), &LinkOptions::default()).unwrap(),
        ResolvedFormat::Elf
    );
    assert_eq!(
        resolve_format(std::slice::from_ref(&pe_p), &LinkOptions::default()).unwrap(),
        ResolvedFormat::Pe
    );
    assert_eq!(
        resolve_format(std::slice::from_ref(&mac_p), &LinkOptions::default()).unwrap(),
        ResolvedFormat::Macho
    );
    // Explicit option overrides sniff
    assert_eq!(
        resolve_format(
            std::slice::from_ref(&elf_p),
            &LinkOptions {
                format: Some(OutputFormat::Pe),
                ..Default::default()
            }
        )
        .unwrap(),
        ResolvedFormat::Pe
    );
    // Empty input list is Usage error
    let empty_err = resolve_format(&[], &LinkOptions::default()).unwrap_err();
    assert_eq!(empty_err.kind, LinkErrorKind::Usage);

    // inspect_object works across all three formats
    inspect_object(&elf_p).expect("inspect ELF");
    inspect_object(&pe_p).expect("inspect COFF");
    inspect_object(&mac_p).expect("inspect Mach-O");

    // Response files: comments, blank lines, double quotes with spaces, multiple tokens per line
    let rsp_path = td.write(
        "args.rsp",
        b"# header comment\n\n\"dir with space/a.o\" b.o\n  # indented comment\nc.o\n",
    );
    let expanded =
        expand_response_files(vec![format!("@{}", rsp_path.display()), "d.o".into()]).unwrap();
    assert_eq!(expanded, vec!["dir with space/a.o", "b.o", "c.o", "d.o"]);

    // Missing response file errors cleanly
    assert!(expand_response_files(vec![format!("@{}", td.out("nope.rsp").display())]).is_err());

    // link_direct and link_with_options wrappers
    let out_direct = td.out("direct_elf");
    link_direct(&[elf_p.clone()], &out_direct).expect("link_direct");
    assert!(out_direct.exists());

    let out_opts = td.out("opts_elf");
    link_with_options(
        &[elf_p.clone()],
        &out_opts,
        &LinkOptions {
            format: Some(OutputFormat::Elf),
            machine: Some(Machine::X86_64),
            dynamic: DynamicMode::Static,
            ..Default::default()
        },
    )
    .expect("link_with_options");
    assert!(out_opts.exists());

    // link_cli end-to-end invocation
    let cli_out = td.out("cli_elf");
    link_cli(&[
        elf_p.to_string_lossy().into_owned(),
        "-o".into(),
        cli_out.to_string_lossy().into_owned(),
        "--static".into(),
    ])
    .expect("link_cli");
    assert!(cli_out.exists());
}
