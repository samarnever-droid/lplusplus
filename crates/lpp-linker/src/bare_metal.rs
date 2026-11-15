//! Pure-Rust freestanding bare-metal ELF support objects (`x86_64-unknown-none`,
//! `aarch64-unknown-none`).
//!
//! Produces deterministic relocatable ELF64 objects containing the hardware
//! instruction sequences for `lpp_arch_*` kernel intrinsics, `lpp_panic_div_zero`,
//! and an optional Multiboot2 header block — requiring zero external C compiler
//! or assembler on any host platform.

use object::write::{Object, SectionId, Symbol, SymbolSection};
use object::{
    Architecture, BinaryFormat, Endianness, SectionKind, SymbolFlags, SymbolKind, SymbolScope,
};

use crate::Machine;

fn add_code_symbol(obj: &mut Object<'_>, section: SectionId, name: &str, bytes: &[u8]) {
    let offset = obj.append_section_data(section, bytes, 16);
    obj.add_symbol(Symbol {
        name: name.as_bytes().to_vec(),
        value: offset,
        size: bytes.len() as u64,
        kind: SymbolKind::Text,
        scope: SymbolScope::Dynamic,
        weak: false,
        section: SymbolSection::Section(section),
        flags: SymbolFlags::None,
    });
}

fn push_a64(buf: &mut Vec<u8>, insns: &[u32]) {
    for insn in insns {
        buf.extend_from_slice(&insn.to_le_bytes());
    }
}

/// Emit a deterministic ELF64 relocatable object containing the bare-metal
/// `lpp_arch_*` hardware instruction wrappers and `lpp_panic_div_zero` halt loop
/// for `machine`.
#[must_use]
pub fn freestanding_arch_object(machine: Machine) -> Vec<u8> {
    let arch = match machine {
        Machine::X86_64 => Architecture::X86_64,
        Machine::Aarch64 => Architecture::Aarch64,
    };
    let mut obj = Object::new(BinaryFormat::Elf, arch, Endianness::Little);
    let text = obj.add_section(Vec::new(), b".text".to_vec(), SectionKind::Text);

    match machine {
        Machine::X86_64 => {
            // Port I/O (System V AMD64: port in rdi/di, val in rsi/esi)
            add_code_symbol(
                &mut obj,
                text,
                "lpp_arch_port_in8",
                &[0x66, 0x89, 0xfa, 0x31, 0xc0, 0xec, 0xc3],
            );
            add_code_symbol(
                &mut obj,
                text,
                "lpp_arch_port_in16",
                &[0x66, 0x89, 0xfa, 0x31, 0xc0, 0x66, 0xed, 0xc3],
            );
            add_code_symbol(
                &mut obj,
                text,
                "lpp_arch_port_in32",
                &[0x66, 0x89, 0xfa, 0xed, 0xc3],
            );
            add_code_symbol(
                &mut obj,
                text,
                "lpp_arch_port_out8",
                &[0x66, 0x89, 0xfa, 0x89, 0xf0, 0xee, 0xc3],
            );
            add_code_symbol(
                &mut obj,
                text,
                "lpp_arch_port_out16",
                &[0x66, 0x89, 0xfa, 0x89, 0xf0, 0x66, 0xef, 0xc3],
            );
            add_code_symbol(
                &mut obj,
                text,
                "lpp_arch_port_out32",
                &[0x66, 0x89, 0xfa, 0x89, 0xf0, 0xef, 0xc3],
            );

            // Interrupts & halt/pause
            add_code_symbol(&mut obj, text, "lpp_arch_cpu_cli", &[0xfa, 0xc3]);
            add_code_symbol(&mut obj, text, "lpp_arch_cpu_sti", &[0xfb, 0xc3]);
            add_code_symbol(&mut obj, text, "lpp_arch_cpu_hlt", &[0xf4, 0xc3]);
            add_code_symbol(&mut obj, text, "lpp_cpu_pause", &[0xf3, 0x90, 0xc3]);

            // TSC, Control Registers, MSRs, Descriptor Tables, TLB
            add_code_symbol(
                &mut obj,
                text,
                "lpp_arch_rdtsc",
                &[0x0f, 0x31, 0x48, 0xc1, 0xe2, 0x20, 0x48, 0x09, 0xd0, 0xc3],
            );
            add_code_symbol(
                &mut obj,
                text,
                "lpp_arch_read_cr0",
                &[0x0f, 0x20, 0xc0, 0xc3],
            );
            add_code_symbol(
                &mut obj,
                text,
                "lpp_arch_read_cr2",
                &[0x0f, 0x20, 0xd0, 0xc3],
            );
            add_code_symbol(
                &mut obj,
                text,
                "lpp_arch_read_cr3",
                &[0x0f, 0x20, 0xd8, 0xc3],
            );
            add_code_symbol(
                &mut obj,
                text,
                "lpp_arch_read_cr4",
                &[0x0f, 0x20, 0xe0, 0xc3],
            );
            add_code_symbol(
                &mut obj,
                text,
                "lpp_arch_write_cr3",
                &[0x0f, 0x22, 0xdf, 0xc3],
            );
            add_code_symbol(
                &mut obj,
                text,
                "lpp_arch_write_cr4",
                &[0x0f, 0x22, 0xe7, 0xc3],
            );
            add_code_symbol(
                &mut obj,
                text,
                "lpp_arch_rdmsr",
                &[
                    0x89, 0xf9, 0x0f, 0x32, 0x48, 0xc1, 0xe2, 0x20, 0x48, 0x09, 0xd0, 0xc3,
                ],
            );
            add_code_symbol(
                &mut obj,
                text,
                "lpp_arch_wrmsr",
                &[
                    0x89, 0xf9, 0x89, 0xf0, 0x48, 0x89, 0xf2, 0x48, 0xc1, 0xea, 0x20, 0x0f, 0x30,
                    0xc3,
                ],
            );
            add_code_symbol(&mut obj, text, "lpp_arch_lgdt", &[0x0f, 0x01, 0x17, 0xc3]);
            add_code_symbol(&mut obj, text, "lpp_arch_lidt", &[0x0f, 0x01, 0x1f, 0xc3]);
            add_code_symbol(&mut obj, text, "lpp_arch_invlpg", &[0x0f, 0x01, 0x3f, 0xc3]);

            // Freestanding trap on division by zero: cli; hlt; jmp -3 (halt forever)
            add_code_symbol(
                &mut obj,
                text,
                "lpp_panic_div_zero",
                &[0xfa, 0xf4, 0xeb, 0xfd],
            );
        }
        Machine::Aarch64 => {
            let ret = 0xd65f_03c0_u32;
            let zero_ret = {
                let mut v = Vec::new();
                push_a64(&mut v, &[0xd280_0000, ret]);
                v
            };
            let just_ret = {
                let mut v = Vec::new();
                push_a64(&mut v, &[ret]);
                v
            };
            for sym in [
                "lpp_arch_port_in8",
                "lpp_arch_port_in16",
                "lpp_arch_port_in32",
                "lpp_arch_read_cr0",
                "lpp_arch_read_cr2",
                "lpp_arch_read_cr3",
                "lpp_arch_read_cr4",
                "lpp_arch_rdmsr",
            ] {
                add_code_symbol(&mut obj, text, sym, &zero_ret);
            }
            for sym in [
                "lpp_arch_port_out8",
                "lpp_arch_port_out16",
                "lpp_arch_port_out32",
                "lpp_arch_write_cr3",
                "lpp_arch_write_cr4",
                "lpp_arch_wrmsr",
                "lpp_arch_lgdt",
                "lpp_arch_lidt",
            ] {
                add_code_symbol(&mut obj, text, sym, &just_ret);
            }

            let mut buf = Vec::new();
            push_a64(&mut buf, &[0xd503_42df, ret]); // msr daifset, #2; ret
            add_code_symbol(&mut obj, text, "lpp_arch_cpu_cli", &buf);

            buf.clear();
            push_a64(&mut buf, &[0xd503_42ff, ret]); // msr daifclr, #2; ret
            add_code_symbol(&mut obj, text, "lpp_arch_cpu_sti", &buf);

            buf.clear();
            push_a64(&mut buf, &[0xd503_207f, ret]); // wfi; ret
            add_code_symbol(&mut obj, text, "lpp_arch_cpu_hlt", &buf);

            buf.clear();
            push_a64(&mut buf, &[0xd503_203f, ret]); // yield; ret
            add_code_symbol(&mut obj, text, "lpp_cpu_pause", &buf);

            buf.clear();
            push_a64(&mut buf, &[0xd53b_e040, ret]); // mrs x0, cntvct_el0; ret
            add_code_symbol(&mut obj, text, "lpp_arch_rdtsc", &buf);

            buf.clear();
            push_a64(&mut buf, &[0xd508_871f, 0xd503_3f9f, 0xd503_3fdf, ret]); // tlbi vmalle1; dsb sy; isb; ret
            add_code_symbol(&mut obj, text, "lpp_arch_invlpg", &buf);

            buf.clear();
            push_a64(&mut buf, &[0xd503_207f, 0x17ff_ffff]); // wfi; b -4
            add_code_symbol(&mut obj, text, "lpp_panic_div_zero", &buf);
        }
    }

    obj.write()
        .expect("ELF arch object serialization must succeed")
}

/// Emit a deterministic relocatable ELF64 object containing a 24-byte Multiboot2
/// header in `.multiboot2` so GRUB2 / QEMU can boot the linked kernel ELF.
#[must_use]
pub fn multiboot2_header_object(machine: Machine) -> Vec<u8> {
    let arch = match machine {
        Machine::X86_64 => Architecture::X86_64,
        Machine::Aarch64 => Architecture::Aarch64,
    };
    let mut obj = Object::new(BinaryFormat::Elf, arch, Endianness::Little);
    let sec = obj.add_section(Vec::new(), b".multiboot2".to_vec(), SectionKind::Text);
    const MAGIC: u32 = 0xE852_50D6;
    const ARCH_I386: u32 = 0;
    const HEADER_LEN: u32 = 24;
    let checksum: u32 = 0u32.wrapping_sub(MAGIC.wrapping_add(ARCH_I386).wrapping_add(HEADER_LEN));
    let mut hdr = Vec::with_capacity(24);
    hdr.extend_from_slice(&MAGIC.to_le_bytes());
    hdr.extend_from_slice(&ARCH_I386.to_le_bytes());
    hdr.extend_from_slice(&HEADER_LEN.to_le_bytes());
    hdr.extend_from_slice(&checksum.to_le_bytes());
    // End tag: type=0 (u16), flags=0 (u16), size=8 (u32)
    hdr.extend_from_slice(&0u16.to_le_bytes());
    hdr.extend_from_slice(&0u16.to_le_bytes());
    hdr.extend_from_slice(&8u32.to_le_bytes());
    let offset = obj.append_section_data(sec, &hdr, 8);
    obj.add_symbol(Symbol {
        name: b"__lpp_multiboot2_header".to_vec(),
        value: offset,
        size: hdr.len() as u64,
        kind: SymbolKind::Text,
        scope: SymbolScope::Dynamic,
        weak: false,
        section: SymbolSection::Section(sec),
        flags: SymbolFlags::None,
    });
    obj.write()
        .expect("Multiboot2 object serialization must succeed")
}
