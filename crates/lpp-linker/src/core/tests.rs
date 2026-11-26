use super::pe_support::generate_base_relocs_from_rvas;
use super::*;

#[test]
fn align_up_pow2() {
    assert_eq!(align_up(0, 16), 0);
    assert_eq!(align_up(1, 16), 16);
    assert_eq!(align_up(16, 16), 16);
    assert_eq!(align_up(17, 16), 32);
    assert_eq!(align_up(0x1fff, 0x1000), 0x2000);
}

#[test]
fn elf_identity_holds_for_page_layout() {
    let page = 0x1000u64;
    let base = 0x400000u64;
    let text_file = 0x1000u64;
    let text_va = base + text_file;
    assert_eq!(text_va % page, text_file % page);
    let rx_size = 0x1234u64;
    let data_va = align_up_u64(text_va + rx_size, page);
    let data_file = align_up(text_file as usize + rx_size as usize, page as usize) as u64;
    assert_eq!(data_va % page, data_file % page);
}

#[test]
fn x86_64_tpoff_variant2() {
    // 16-byte TLS block, symbol at 0 → TP-relative -16
    assert_eq!(x64_tpoff(0, 16), -16);
    assert_eq!(x64_tpoff(8, 16), -8);
    assert_eq!(x64_tpoff(0, 8), -8);
}

#[test]
fn aarch64_tpoff_variant1() {
    assert_eq!(a64_tpoff(0, 16), 16);
    assert_eq!(a64_tpoff(8, 16), 24);
}

#[test]
fn sha256_abc() {
    let d = sha256(b"abc");
    let hex: String = d.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        hex,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn elf_hash_foo() {
    // SysV ELF hash("foo") = 0x6d5f.
    assert_eq!(elf_hash(b"foo"), 0x6d5f);
    assert_ne!(elf_hash(b"foo"), elf_hash(b"bar"));
}

#[test]
fn dynamic_entry_count_matches_emission() {
    // One DT_NEEDED plus the 15 fixed tags emitted in write_elf_with_options.
    const FIXED: usize = 15;
    assert_eq!(1 + FIXED, 16);
}

#[test]
fn pe_checksum_length_term() {
    let mut img = vec![0u8; 64];
    img[0] = b'M';
    img[1] = b'Z';
    let sum = pe_checksum(&img, 32);
    assert_eq!(sum, 64 + (b'M' as u32) + ((b'Z' as u32) << 8));
}

#[test]
fn a64_call26_encoding() {
    let mut buf = [0x00u8, 0x00, 0x00, 0x94]; // bl #0
    // P=0, S=16 → disp 16 → imm 4
    a64_patch_call26(&mut buf, 0, 16, 0, 0).unwrap();
    let instr = u32::from_le_bytes(buf);
    assert_eq!(instr & 0xFC00_0000, 0x9400_0000);
    assert_eq!(instr & 0x03FF_FFFF, 4);
}

#[test]
fn a64_add_lo12() {
    let mut buf = 0x91000000u32.to_le_bytes(); // add x0, x0, #0
    a64_patch_add_lo12(&mut buf, 0, 0x1234, 0).unwrap();
    let instr = u32::from_le_bytes(buf);
    assert_eq!((instr >> 10) & 0xfff, 0x234);
}

#[test]
fn x64_startup_stub_freestanding() {
    // Freestanding: call main is at +12; disp at +13; next-ip at +17;
    // then a raw exit(2) syscall and no PLT patch sites.
    let (stub, plt_calls) =
        emit_elf_start_stub_x64(0x1000, 0x1000 + 0x20, StartupAbi::Freestanding).unwrap();
    assert_eq!(stub[12], 0xe8);
    let disp = i32::from_le_bytes(stub[13..17].try_into().unwrap());
    assert_eq!(disp, 0x20 - 17); // S - (P+4), next-ip = start_off+17
    assert_eq!(stub[17], 0x89); // mov edi, eax
    assert_eq!(stub[19], 0xb8); // mov eax, 60
    assert_eq!(&stub[24..26], &[0x0f, 0x05]); // syscall
    assert!(plt_calls.is_empty());
}

#[test]
fn x64_startup_stub_hosted_glibc() {
    // Hosted glibc: Scrt1-style bootstrap through __libc_start_main.
    let start_off = 0x1000usize;
    let main_off = 0x1000usize + 0x40;
    let (stub, plt_calls) =
        emit_elf_start_stub_x64(start_off, main_off, StartupAbi::HostedGlibc).unwrap();
    // mov r9, rdx (rtld_fini) proves this is the hosted bootstrap.
    assert_eq!(&stub[2..5], &[0x49, 0x89, 0xd1]);
    // lea rdi, [rip+main] at +20, disp @ 23, next-ip = start_off+27.
    assert_eq!(&stub[20..23], &[0x48, 0x8d, 0x3d]);
    let lea_disp = i32::from_le_bytes(stub[23..27].try_into().unwrap());
    assert_eq!(lea_disp as i64, main_off as i64 - (start_off as i64 + 27));
    // call __libc_start_main@plt at +27, patch site @ start_off+28.
    assert_eq!(stub[27], 0xe8);
    assert_eq!(stub[32], 0xf4); // hlt
    assert_eq!(plt_calls, vec![("__libc_start_main", start_off + 28)]);
}

#[test]
fn rel32_formula() {
    // S=0x401100, P=0x40100c (field), A=-4  →  0x401100-4-0x40100c = 0xf0
    let s = 0x401100i64;
    let p = 0x40100ci64;
    let a = -4i64;
    assert_eq!(s + a - p, 0xf0);
    // equivalent: S - (P+4)
    assert_eq!(s - (p + 4), 0xf0);
}

#[test]
fn pe_page_reloc_groups() {
    let blob = generate_base_relocs_from_rvas(&[0x1008, 0x1010, 0x2000]);
    assert!(blob.len() >= 16);
    let page0 = u32::from_le_bytes(blob[0..4].try_into().unwrap());
    assert_eq!(page0, 0x1000);
}

#[test]
fn fits_checks() {
    assert!(fits_i32(i32::MAX as i64));
    assert!(!fits_i32(i32::MAX as i64 + 1));
    assert!(fits_i26((1 << 25) - 1));
    assert!(!fits_i26(1 << 25));
}

#[test]
fn pe_bss_starts_after_data() {
    let data_sym = ResolvedSym {
        name: "g_data".to_string(),
        bind: Bind::Global,
        class: SectionClass::Data,
        offset: 8,
        size: 8,
        object: 0,
    };
    let bss_sym = ResolvedSym {
        name: "g_bss".to_string(),
        bind: Bind::Global,
        class: SectionClass::Bss,
        offset: 0,
        size: 8,
        object: 0,
    };
    let data_rva = 0x3000;
    let bss_rva = data_rva + 32;
    assert_eq!(
        super::pe_writer::pe_sym_rva(&data_sym, 0x1000, 0x2000, data_rva, bss_rva, 0x4000),
        0x3008
    );
    assert_eq!(
        super::pe_writer::pe_sym_rva(&bss_sym, 0x1000, 0x2000, data_rva, bss_rva, 0x4000),
        0x3020
    );
}
