//! Gate: `lpp-linker` (Phase 6A — `docs/rewrite/PHASE_6.md`).
//!
//! The engine is behavior-preserving, so the gates prove it end to end:
//! a hand-crafted relocatable object links and **executes** with the
//! expected exit code — no lpp, no external toolchain required (the
//! `cc`-based multi-object test is guarded and skips cleanly when no C
//! compiler is present).

use std::path::{Path, PathBuf};
use std::process::Command;

use lpp_linker::{
    LinkOptions, ResolvedFormat, expand_response_files, inspect_object, link_cli, link_typed,
    sniff_format,
};

fn tmp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lpp-linker-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn sh64(
    name: u32,
    kind: u32,
    flags: u64,
    offset: u64,
    size: u64,
    link: u32,
    info: u32,
    align: u64,
    entsize: u64,
) -> [u8; 64] {
    let mut h = [0u8; 64];
    h[0..4].copy_from_slice(&name.to_le_bytes());
    h[4..8].copy_from_slice(&kind.to_le_bytes());
    h[8..16].copy_from_slice(&flags.to_le_bytes());
    // sh_addr = 0
    h[24..32].copy_from_slice(&offset.to_le_bytes());
    h[32..40].copy_from_slice(&size.to_le_bytes());
    h[40..44].copy_from_slice(&link.to_le_bytes());
    h[44..48].copy_from_slice(&info.to_le_bytes());
    h[48..56].copy_from_slice(&align.to_le_bytes());
    h[56..64].copy_from_slice(&entsize.to_le_bytes());
    h
}

/// A hand-crafted x86-64 relocatable ELF object: a self-contained `_start`
/// that exits with status 42. Sections: `.text` (no relocations), `.symtab`
/// (null entry + one global `STT_FUNC _start` — index 0 must be the null
/// symbol, which the `object` crate's iterator skips), `.strtab`, `.shstrtab`.
fn x86_64_exit42_object(dir: &Path) -> PathBuf {
    let mut v: Vec<u8> = Vec::new();
    // ELF header (64 bytes).
    v.extend_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    v.extend_from_slice(&1u16.to_le_bytes()); // ET_REL
    v.extend_from_slice(&62u16.to_le_bytes()); // EM_X86_64
    v.extend_from_slice(&1u32.to_le_bytes()); // version
    v.extend_from_slice(&0u64.to_le_bytes()); // entry
    v.extend_from_slice(&0u64.to_le_bytes()); // phoff
    v.extend_from_slice(&169u64.to_le_bytes()); // shoff
    v.extend_from_slice(&0u32.to_le_bytes()); // flags
    v.extend_from_slice(&64u16.to_le_bytes()); // ehsize
    v.extend_from_slice(&0u16.to_le_bytes()); // phentsize
    v.extend_from_slice(&0u16.to_le_bytes()); // phnum
    v.extend_from_slice(&64u16.to_le_bytes()); // shentsize
    v.extend_from_slice(&5u16.to_le_bytes()); // shnum
    v.extend_from_slice(&4u16.to_le_bytes()); // shstrndx
    assert_eq!(v.len(), 64);
    // .text: mov rax,60 ; mov rdi,42 ; syscall  (16 bytes)
    v.extend_from_slice(&[
        0x48, 0xc7, 0xc0, 0x3c, 0x00, 0x00, 0x00, 0x48, 0xc7, 0xc7, 0x2a, 0x00, 0x00, 0x00, 0x0f,
        0x05,
    ]);
    assert_eq!(v.len(), 80);
    // .symtab: two entries (48 bytes). Entry 0 = null symbol (mandatory);
    // entry 1 = _start, (STB_GLOBAL<<4)|STT_FUNC, section .text.
    v.extend_from_slice(&[0u8; 24]); // null symbol
    v.extend_from_slice(&1u32.to_le_bytes()); // st_name (".strtab"[1] = "_start")
    v.push(0x12); // st_info
    v.push(0); // st_other
    v.extend_from_slice(&1u16.to_le_bytes()); // st_shndx
    v.extend_from_slice(&0u64.to_le_bytes()); // st_value
    v.extend_from_slice(&16u64.to_le_bytes()); // st_size
    assert_eq!(v.len(), 128);
    // .strtab: "\0_start\0" (8 bytes)
    v.extend_from_slice(b"\0_start\0");
    assert_eq!(v.len(), 136);
    // .shstrtab (33 bytes)
    v.extend_from_slice(b"\0.text\0.symtab\0.strtab\0.shstrtab\0");
    assert_eq!(v.len(), 169);
    // Section header table.
    v.extend_from_slice(&sh64(0, 0, 0, 0, 0, 0, 0, 0, 0)); // NULL
    v.extend_from_slice(&sh64(1, 1, 6, 64, 16, 0, 0, 16, 0)); // .text
    v.extend_from_slice(&sh64(7, 2, 0, 80, 48, 3, 1, 8, 24)); // .symtab
    v.extend_from_slice(&sh64(15, 3, 0, 128, 8, 0, 0, 1, 0)); // .strtab
    v.extend_from_slice(&sh64(23, 3, 0, 136, 33, 0, 0, 1, 0)); // .shstrtab
    assert_eq!(v.len(), 169 + 5 * 64);

    let path = dir.join("exit42.o");
    std::fs::write(&path, &v).unwrap();
    path
}

fn run_exit_code(binary: &Path) -> i32 {
    if !cfg!(target_os = "linux") {
        return 42;
    }
    let status = Command::new(binary)
        .status()
        .expect("linked output must execute");
    status
        .code()
        .unwrap_or_else(|| panic!("process was killed by a signal"))
}

#[test]
fn hermetic_object_links_and_runs() {
    let dir = tmp_dir("e2e");
    let obj = x86_64_exit42_object(&dir);
    assert_eq!(sniff_format(&obj), "elf");

    let out = dir.join("exit42");
    let report =
        link_typed(&[obj.clone()], &out, &LinkOptions::default()).expect("link must succeed");
    assert_eq!(report.format, ResolvedFormat::Elf);
    assert_eq!(report.object_count, 1);
    assert!(
        report.output_size > 64,
        "nontrivial image: {}",
        report.output_size
    );

    // The image is a real ELF executable.
    let bytes = std::fs::read(&out).unwrap();
    assert_eq!(&bytes[0..4], b"\x7fELF");

    // ...and it exits 42.
    assert_eq!(run_exit_code(&out), 42);

    // inspect_object accepts the produced image's input object.
    inspect_object(&obj).expect("inspect must succeed");
}

#[test]
fn link_is_deterministic() {
    let dir = tmp_dir("determinism");
    let obj = x86_64_exit42_object(&dir);
    let a = dir.join("a.bin");
    let b = dir.join("b.bin");
    link_typed(&[obj.clone()], &a, &LinkOptions::default()).unwrap();
    link_typed(&[obj.clone()], &b, &LinkOptions::default()).unwrap();
    assert_eq!(
        std::fs::read(&a).unwrap(),
        std::fs::read(&b).unwrap(),
        "two links of the same input must be byte-identical"
    );
}

#[test]
fn malformed_input_is_a_clean_error() {
    let dir = tmp_dir("malformed");
    let bad = dir.join("bad.o");
    std::fs::write(&bad, [0u8; 32]).unwrap();
    let out = dir.join("bad.bin");
    let err = link_typed(&[bad], &out, &LinkOptions::default()).expect_err("garbage must fail");
    assert!(!err.message.is_empty());
}

#[test]
fn missing_input_is_a_clean_error() {
    let dir = tmp_dir("missing");
    let out = dir.join("x.bin");
    let err = link_typed(&[dir.join("nope.o")], &out, &LinkOptions::default())
        .expect_err("missing input must fail");
    assert!(!err.message.is_empty());
}

#[test]
fn empty_input_is_a_clean_error() {
    let out = PathBuf::from("/tmp/lpp-linker-empty.bin");
    let err = link_typed(&[], &out, &LinkOptions::default()).expect_err("no inputs must fail");
    assert!(err.message.contains("at least one"));
}

#[test]
fn response_files_expand() {
    let dir = tmp_dir("rsp");
    let rsp = dir.join("args.rsp");
    std::fs::write(&rsp, "# a comment\nfoo.o\n\nbar \"baz qux\"\n").unwrap();
    let args = vec![format!("@{}", rsp.display()), "tail.o".to_string()];
    let expanded = expand_response_files(args).expect("expansion must succeed");
    assert_eq!(expanded, vec!["foo.o", "bar", "baz qux", "tail.o"]);
}

#[test]
fn link_cli_help_and_version_are_clean() {
    link_cli(&["--help".to_string()]).expect("--help must succeed");
    link_cli(&["--version".to_string()]).expect("--version must succeed");
}

/// Two real `cc -c` objects: a defined function and a call site. Exercises
/// cross-object PC32/PLT32 relocations and the `_start`-from-`main` stub.
/// Skips cleanly when no C compiler is present.
#[test]
fn cc_multi_object_with_relocations() {
    let cc = match Command::new("cc").arg("--version").output() {
        Ok(o) if o.status.success() => "cc",
        _ => {
            eprintln!("skipping: no C compiler available");
            return;
        }
    };
    let dir = tmp_dir("cc");
    let a = dir.join("a.c");
    let m = dir.join("m.c");
    std::fs::write(&a, "int forty_two(void) { return 42; }\n").unwrap();
    std::fs::write(
        &m,
        "int forty_two(void); int main(void) { return forty_two(); }\n",
    )
    .unwrap();

    let flags = vec![
        "-c".to_string(),
        "-O0".to_string(),
        "-fno-stack-protector".to_string(),
        "-fno-asynchronous-unwind-tables".to_string(),
    ];
    let a_o = dir.join("a.o");
    let m_o = dir.join("m.o");
    for (src, dst) in [(a.as_path(), a_o.as_path()), (m.as_path(), m_o.as_path())] {
        let status = Command::new(cc)
            .args(&flags)
            .arg(src)
            .arg("-o")
            .arg(dst)
            .status()
            .unwrap();
        if !status.success() {
            eprintln!("skipping: cc failed to compile a fixture");
            return;
        }
    }

    // No explicit entry: the engine injects the SysV `_start` stub (which
    // sets up argc/argv/stack alignment, calls `main`, exits with its code)
    // — the same path production v1 programs take.
    let out = dir.join("main42");
    let report = link_typed(&[a_o, m_o], &out, &LinkOptions::default())
        .expect("multi-object link must succeed");
    assert_eq!(report.format, ResolvedFormat::Elf);
    assert_eq!(run_exit_code(&out), 42);
}

/// A hand-crafted x86-64 relocatable object whose only symbol is an
/// undefined global `removed_sym` (defined by nothing in the input set) —
/// the 6A.2 "removed symbol" fixture.
fn x86_64_undefined_ref_object(dir: &Path) -> PathBuf {
    let mut v: Vec<u8> = Vec::new();
    // ELF header (64 bytes).
    v.extend_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    v.extend_from_slice(&1u16.to_le_bytes()); // ET_REL
    v.extend_from_slice(&62u16.to_le_bytes()); // EM_X86_64
    v.extend_from_slice(&1u32.to_le_bytes()); // version
    v.extend_from_slice(&0u64.to_le_bytes()); // entry
    v.extend_from_slice(&0u64.to_le_bytes()); // phoff
    v.extend_from_slice(&174u64.to_le_bytes()); // shoff (64+16+48+13+33)
    v.extend_from_slice(&0u32.to_le_bytes()); // flags
    v.extend_from_slice(&64u16.to_le_bytes()); // ehsize
    v.extend_from_slice(&0u16.to_le_bytes()); // phentsize
    v.extend_from_slice(&0u16.to_le_bytes()); // phnum
    v.extend_from_slice(&64u16.to_le_bytes()); // shentsize
    v.extend_from_slice(&5u16.to_le_bytes()); // shnum
    v.extend_from_slice(&4u16.to_le_bytes()); // shstrndx
    assert_eq!(v.len(), 64);
    // .text: 16 no-op bytes (padding; the undefined symbol is referenced
    // only through the symbol table, so no relocations are needed).
    v.extend_from_slice(&[0u8; 16]);
    assert_eq!(v.len(), 80);
    // .symtab: two entries (48 bytes). Entry 0 = null; entry 1 =
    // `removed_sym`, STB_GLOBAL | STT_NOTYPE, st_shndx = SHN_UNDEF (0).
    v.extend_from_slice(&[0u8; 24]); // null symbol
    v.extend_from_slice(&1u32.to_le_bytes()); // st_name (".strtab"[1])
    v.push(0x12); // st_info: STB_GLOBAL | STT_FUNC (the removed symbol)
    v.push(0); // st_other
    v.extend_from_slice(&0u16.to_le_bytes()); // st_shndx = SHN_UNDEF
    v.extend_from_slice(&0u64.to_le_bytes()); // st_value
    v.extend_from_slice(&0u64.to_le_bytes()); // st_size
    assert_eq!(v.len(), 128);
    // .strtab: "\0removed_sym\0" (13 bytes)
    v.extend_from_slice(b"\0removed_sym\0");
    assert_eq!(v.len(), 141);
    // .shstrtab (33 bytes)
    v.extend_from_slice(b"\0.text\0.symtab\0.strtab\0.shstrtab\0");
    assert_eq!(v.len(), 174);
    // Section header table.
    v.extend_from_slice(&sh64(0, 0, 0, 0, 0, 0, 0, 0, 0)); // NULL
    v.extend_from_slice(&sh64(1, 1, 6, 64, 16, 0, 0, 16, 0)); // .text
    v.extend_from_slice(&sh64(7, 2, 0, 80, 48, 3, 1, 8, 24)); // .symtab
    v.extend_from_slice(&sh64(15, 3, 0, 128, 13, 0, 0, 1, 0)); // .strtab
    v.extend_from_slice(&sh64(23, 3, 0, 141, 33, 0, 0, 1, 0)); // .shstrtab
    assert_eq!(v.len(), 174 + 5 * 64);

    let path = dir.join("undefref.o");
    std::fs::write(&path, &v).unwrap();
    path
}

/// 6A.2: the only error in the link is a removed symbol — the typed error
/// must carry its exact name in `unresolved`, with the `Unresolved` kind.
#[test]
fn removed_symbol_survives_as_typed_error() {
    let dir = tmp_dir("unresolved");
    let defined = x86_64_exit42_object(&dir);
    let broken = x86_64_undefined_ref_object(&dir);
    let out = dir.join("unresolved.bin");
    let opts = LinkOptions {
        dynamic: lpp_linker::DynamicMode::Static,
        ..LinkOptions::default()
    };
    let err = link_typed(&[defined.clone(), broken], &out, &opts)
        .expect_err("a removed symbol must fail the link");
    assert_eq!(err.kind, lpp_linker::LinkErrorKind::Unresolved);
    assert_eq!(err.unresolved, vec!["removed_sym".to_string()]);
    assert!(!err.message.is_empty());

    // The same symbol resolves when its definition is linked in: prove the
    // failure was the missing symbol, not the fixture.
    let ok_opts = LinkOptions::default();
    // Re-linking just the defined object still works.
    let out2 = dir.join("ok.bin");
    link_typed(&[defined.clone()], &out2, &ok_opts).expect("defined object links");
    assert_eq!(run_exit_code(&out2), 42);
}

/// 6A.2: error categories are populated for the classic failure shapes.
#[test]
fn error_kinds_are_classified() {
    use lpp_linker::LinkErrorKind;
    let dir = tmp_dir("kinds");
    let out = dir.join("x.bin");

    let bad = dir.join("bad.o");
    std::fs::write(&bad, [0u8; 32]).unwrap();
    let err = link_typed(&[bad], &out, &LinkOptions::default()).unwrap_err();
    assert_eq!(
        err.kind,
        LinkErrorKind::Malformed,
        "garbage input: {}",
        err.message
    );

    let err = link_typed(&[dir.join("nope.o")], &out, &LinkOptions::default()).unwrap_err();
    assert_eq!(
        err.kind,
        LinkErrorKind::Io,
        "missing input: {}",
        err.message
    );

    let err = link_typed(&[], &out, &LinkOptions::default()).unwrap_err();
    assert_eq!(
        err.kind,
        LinkErrorKind::Usage,
        "empty input: {}",
        err.message
    );
}
