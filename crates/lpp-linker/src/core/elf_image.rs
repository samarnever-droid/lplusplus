use super::*;

pub(super) struct ElfImageContext<'a> {
    pub(super) opts: &'a LinkOptions,
    pub(super) output: &'a Path,
    pub(super) merged: &'a Merged,
    pub(super) machine: Machine,
    pub(super) dyn_mode: bool,
    pub(super) pie: bool,
    pub(super) base: u64,
    pub(super) entry_va: u64,
    pub(super) page_size: usize,
    pub(super) phnum: u16,
    pub(super) phoff: usize,
    pub(super) phdrs_bytes: usize,
    pub(super) has_interp: bool,
    pub(super) has_note: bool,
    pub(super) has_tls: bool,
    pub(super) has_dynamic: bool,
    pub(super) interp_str: String,
    pub(super) interp_off: usize,
    pub(super) note_off: usize,
    pub(super) note_size: usize,
    pub(super) hdr_filesz: usize,
    pub(super) text_file: usize,
    pub(super) text_va: u64,
    pub(super) text_len: usize,
    pub(super) plt_off_in_text: usize,
    pub(super) rodata_off_in_text: usize,
    pub(super) init_off_in_text: usize,
    pub(super) fini_off_in_text: usize,
    pub(super) plt_size: usize,
    pub(super) plt_entsize: usize,
    pub(super) dynsym_off: usize,
    pub(super) dynsym_size: usize,
    pub(super) dynstr_off: usize,
    pub(super) dynstr_size: usize,
    pub(super) hash_off: usize,
    pub(super) hash_size: usize,
    pub(super) rela_plt_off: usize,
    pub(super) rela_plt_size: usize,
    pub(super) rx_filesz: usize,
    pub(super) data_va: u64,
    pub(super) data_file: usize,
    pub(super) data_filesz: usize,
    pub(super) data_memsz: usize,
    pub(super) dynamic_size: usize,
    pub(super) gotplt_off_in_data: usize,
    pub(super) gotplt_size: usize,
    pub(super) got_off_in_data: usize,
    pub(super) got_size: usize,
    pub(super) data_off_in_data: usize,
    pub(super) tls_off_in_data: usize,
    pub(super) tls_memsz: u64,
    pub(super) tls_align: u64,
    pub(super) va_text: u64,
    pub(super) va_plt: u64,
    pub(super) va_rodata: u64,
    pub(super) va_init: u64,
    pub(super) va_fini: u64,
    pub(super) va_dynsym: u64,
    pub(super) va_dynstr: u64,
    pub(super) va_hash: u64,
    pub(super) va_relaplt: u64,
    pub(super) va_data: u64,
    pub(super) va_dynamic: u64,
    pub(super) va_gotplt: u64,
    pub(super) va_got: u64,
    pub(super) va_tls: u64,
    pub(super) va_bss: u64,
    pub(super) rx: Vec<u8>,
    pub(super) rw: Vec<u8>,
}

pub(super) fn finish_elf_image(args: ElfImageContext<'_>) -> Result<(), LinkError> {
    let ElfImageContext {
        opts,
        output,
        merged,
        machine,
        dyn_mode,
        pie,
        base,
        entry_va,
        page_size,
        phnum,
        phoff,
        phdrs_bytes,
        has_interp,
        has_note,
        has_tls,
        has_dynamic,
        interp_str,
        interp_off,
        note_off,
        note_size,
        hdr_filesz,
        text_file,
        text_va,
        text_len,
        plt_off_in_text,
        rodata_off_in_text,
        init_off_in_text,
        fini_off_in_text,
        plt_size,
        plt_entsize,
        dynsym_off,
        dynsym_size,
        dynstr_off,
        dynstr_size,
        hash_off,
        hash_size,
        rela_plt_off,
        rela_plt_size,
        rx_filesz,
        data_va,
        data_file,
        data_filesz,
        data_memsz,
        dynamic_size,
        gotplt_off_in_data,
        gotplt_size,
        got_off_in_data,
        got_size,
        data_off_in_data,
        tls_off_in_data,
        tls_memsz,
        tls_align,
        va_text,
        va_plt,
        va_rodata,
        va_init,
        va_fini,
        va_dynsym,
        va_dynstr,
        va_hash,
        va_relaplt,
        va_data,
        va_dynamic,
        va_gotplt,
        va_got,
        va_tls,
        va_bss,
        rx,
        rw,
    } = args;

    let sec_va = |class: SectionClass| -> u64 {
        match class {
            SectionClass::Text => va_text,
            SectionClass::Rodata => va_rodata,
            SectionClass::Data => va_data,
            SectionClass::Bss => va_bss,
            SectionClass::Tls => va_tls,
            SectionClass::InitArray => va_init,
            SectionClass::FiniArray => va_fini,
        }
    };

    // ── assemble image ──────────────────────────────────────────────────
    // Optional section headers at end of file (after RW).
    let shstr = b"\0.shstrtab\0.text\0.rodata\0.data\0.bss\0.tdata\0.tbss\0.init_array\0.fini_array\0.interp\0.note.gnu.build-id\0.plt\0.dynsym\0.dynstr\0.hash\0.rela.plt\0.dynamic\0.got.plt\0.got\0.symtab\0.strtab\0";
    // We always emit a practical set of section headers for gdb/readelf.
    // Build .strtab / .symtab if not stripped.
    let mut strtab = vec![0u8];
    let mut symtab = Vec::new();
    // NULL symbol
    symtab.extend_from_slice(&[0u8; 24]);
    if !opts.strip {
        let mut names: Vec<_> = merged.syms.values().collect();
        names.sort_by_key(|s| s.name.as_str());
        for sy in names {
            let so = strtab.len() as u32;
            strtab.extend_from_slice(sy.name.as_bytes());
            strtab.push(0);
            let mut ent = [0u8; 24];
            put_u32(&mut ent, 0, so);
            ent[4] = match sy.bind {
                Bind::Local => 0x00,
                Bind::Weak => 0x22, // STB_WEAK STT_FUNC-ish; type refined below
                Bind::Global | Bind::Common => 0x12,
            };
            if sy.class != SectionClass::Text {
                ent[4] = (ent[4] & 0xf0) | 1; // STT_OBJECT
            }
            let shndx: u16 = match sy.class {
                SectionClass::Text => 1,
                SectionClass::Rodata => 2,
                SectionClass::Data => 3,
                SectionClass::Bss => 4,
                SectionClass::Tls => 5,
                SectionClass::InitArray => 7,
                SectionClass::FiniArray => 8,
            };
            put_u16(&mut ent, 6, shndx);
            put_u64(&mut ent, 8, sec_va(sy.class) + sy.offset);
            put_u64(&mut ent, 16, sy.size);
            symtab.extend_from_slice(&ent);
        }
    }

    let file_end_data = data_file + data_filesz;
    let shoff_unaligned = file_end_data;
    let shoff = align_up(shoff_unaligned, 8);

    // Section header count: NULL + the ones we emit
    // 1 .text 2 .rodata 3 .data 4 .bss 5 .tdata? 6 .tbss? 7 .init_array 8 .fini_array
    // + optional interp note plt dynsym dynstr hash rela.plt dynamic got.plt got
    // + .shstrtab .symtab .strtab
    // We'll construct a vec of shdrs.

    struct Sh {
        name: u32,
        typ: u32,
        flags: u64,
        addr: u64,
        off: u64,
        size: u64,
        link: u32,
        info: u32,
        addralign: u64,
        entsize: u64,
    }
    fn name_off(tab: &[u8], n: &[u8]) -> u32 {
        tab.windows(n.len())
            .position(|w| w == n)
            .map(|p| p as u32)
            .unwrap_or(0)
    }

    let mut shdrs: Vec<Sh> = Vec::new();
    shdrs.push(Sh {
        name: 0,
        typ: 0,
        flags: 0,
        addr: 0,
        off: 0,
        size: 0,
        link: 0,
        info: 0,
        addralign: 0,
        entsize: 0,
    });
    let shf_a = 2u64;
    let shf_ax = 2 | 4;
    let shf_wa = 1 | 2;
    let shf_tls = 0x400;

    shdrs.push(Sh {
        name: name_off(shstr, b".text\0"),
        typ: 1,
        flags: shf_ax,
        addr: va_text,
        off: text_file as u64,
        size: text_len as u64,
        link: 0,
        info: 0,
        addralign: 16,
        entsize: 0,
    });
    shdrs.push(Sh {
        name: name_off(shstr, b".rodata\0"),
        typ: 1,
        flags: shf_a,
        addr: va_rodata,
        off: (text_file + rodata_off_in_text) as u64,
        size: merged.rodata.len() as u64,
        link: 0,
        info: 0,
        addralign: 16,
        entsize: 0,
    });
    shdrs.push(Sh {
        name: name_off(shstr, b".data\0"),
        typ: 1,
        flags: shf_wa,
        addr: va_data,
        off: (data_file + data_off_in_data) as u64,
        size: merged.data.len() as u64,
        link: 0,
        info: 0,
        addralign: 16,
        entsize: 0,
    });
    shdrs.push(Sh {
        name: name_off(shstr, b".bss\0"),
        typ: 8,
        flags: shf_wa,
        addr: va_bss,
        off: 0,
        size: merged.bss_size as u64,
        link: 0,
        info: 0,
        addralign: 16,
        entsize: 0,
    });
    if has_tls {
        shdrs.push(Sh {
            name: name_off(shstr, b".tdata\0"),
            typ: 1,
            flags: shf_wa | shf_tls,
            addr: va_tls,
            off: (data_file + tls_off_in_data) as u64,
            size: merged.tls.len() as u64,
            link: 0,
            info: 0,
            addralign: tls_align,
            entsize: 0,
        });
        shdrs.push(Sh {
            name: name_off(shstr, b".tbss\0"),
            typ: 8,
            flags: shf_wa | shf_tls,
            addr: va_tls + merged.tls.len() as u64,
            off: 0,
            size: merged.tbss_size as u64,
            link: 0,
            info: 0,
            addralign: tls_align,
            entsize: 0,
        });
    }
    if !merged.init_array.is_empty() {
        shdrs.push(Sh {
            name: name_off(shstr, b".init_array\0"),
            typ: 14,
            flags: shf_wa,
            addr: va_init,
            off: (text_file + init_off_in_text) as u64,
            size: merged.init_array.len() as u64,
            link: 0,
            info: 0,
            addralign: 8,
            entsize: 8,
        });
    }
    if !merged.fini_array.is_empty() {
        shdrs.push(Sh {
            name: name_off(shstr, b".fini_array\0"),
            typ: 15,
            flags: shf_wa,
            addr: va_fini,
            off: (text_file + fini_off_in_text) as u64,
            size: merged.fini_array.len() as u64,
            link: 0,
            info: 0,
            addralign: 8,
            entsize: 8,
        });
    }

    let shstr_off = shoff;
    let symtab_off = align_up(shstr_off + shstr.len(), 8);
    let strtab_off = align_up(symtab_off + if opts.strip { 0 } else { symtab.len() }, 8);
    let shdr_table_off = align_up(strtab_off + if opts.strip { 0 } else { strtab.len() }, 8);

    // We need shstrndx and will push remaining headers then write.
    // Reserve slots for .shstrtab .symtab .strtab at the end after we know indices.

    if has_interp {
        shdrs.push(Sh {
            name: name_off(shstr, b".interp\0"),
            typ: 1,
            flags: shf_a,
            addr: base + interp_off as u64,
            off: interp_off as u64,
            size: (interp_str.len() + 1) as u64,
            link: 0,
            info: 0,
            addralign: 1,
            entsize: 0,
        });
    }
    if has_note {
        shdrs.push(Sh {
            name: name_off(shstr, b".note.gnu.build-id\0"),
            typ: 7,
            flags: shf_a,
            addr: base + note_off as u64,
            off: note_off as u64,
            size: note_size as u64,
            link: 0,
            info: 0,
            addralign: 4,
            entsize: 0,
        });
    }
    if dyn_mode {
        shdrs.push(Sh {
            name: name_off(shstr, b".plt\0"),
            typ: 1,
            flags: shf_ax,
            addr: va_plt,
            off: (text_file + plt_off_in_text) as u64,
            size: plt_size as u64,
            link: 0,
            info: 0,
            addralign: 16,
            entsize: plt_entsize as u64,
        });
        // Track section indices for sh_link cross-references
        let dynsym_shndx = shdrs.len() as u32;
        shdrs.push(Sh {
            name: name_off(shstr, b".dynsym\0"),
            typ: 11,
            flags: shf_a,
            addr: va_dynsym,
            off: (text_file + dynsym_off) as u64,
            size: dynsym_size as u64,
            link: dynsym_shndx + 1, // .dynstr is next
            info: 1,
            addralign: 8,
            entsize: 24,
        });
        let dynstr_shndx = shdrs.len() as u32;
        shdrs.push(Sh {
            name: name_off(shstr, b".dynstr\0"),
            typ: 3,
            flags: shf_a,
            addr: va_dynstr,
            off: (text_file + dynstr_off) as u64,
            size: dynstr_size as u64,
            link: 0,
            info: 0,
            addralign: 1,
            entsize: 0,
        });
        let _ = dynstr_shndx; // used via dynsym_shndx+1 above
        shdrs.push(Sh {
            name: name_off(shstr, b".hash\0"),
            typ: 5,
            flags: shf_a,
            addr: va_hash,
            off: (text_file + hash_off) as u64,
            size: hash_size as u64,
            link: dynsym_shndx, // points to .dynsym
            info: 0,
            addralign: 8,
            entsize: 4,
        });
        shdrs.push(Sh {
            name: name_off(shstr, b".rela.plt\0"),
            typ: 4,
            flags: shf_a | 0x40, // SHF_INFO_LINK
            addr: va_relaplt,
            off: (text_file + rela_plt_off) as u64,
            size: rela_plt_size as u64,
            link: dynsym_shndx, // associated symbol table = .dynsym
            info: 0,            // sh_info = index of .plt (patched by caller if needed)
            addralign: 8,
            entsize: 24,
        });
        shdrs.push(Sh {
            name: name_off(shstr, b".dynamic\0"),
            typ: 6,
            flags: shf_wa,
            addr: va_dynamic,
            off: data_file as u64,
            size: dynamic_size as u64,
            link: dynstr_shndx, // points to .dynstr section
            info: 0,
            addralign: 8,
            entsize: 16,
        });
        shdrs.push(Sh {
            name: name_off(shstr, b".got.plt\0"),
            typ: 1,
            flags: shf_wa,
            addr: va_gotplt,
            off: (data_file + gotplt_off_in_data) as u64,
            size: gotplt_size as u64,
            link: 0,
            info: 0,
            addralign: 8,
            entsize: 8,
        });
        if got_size > 0 {
            shdrs.push(Sh {
                name: name_off(shstr, b".got\0"),
                typ: 1,
                flags: shf_wa,
                addr: va_got,
                off: (data_file + got_off_in_data) as u64,
                size: got_size as u64,
                link: 0,
                info: 0,
                addralign: 8,
                entsize: 8,
            });
        }
    }

    let shstrndx = shdrs.len() as u16;
    shdrs.push(Sh {
        name: name_off(shstr, b".shstrtab\0"),
        typ: 3,
        flags: 0,
        addr: 0,
        off: shstr_off as u64,
        size: shstr.len() as u64,
        link: 0,
        info: 0,
        addralign: 1,
        entsize: 0,
    });
    if !opts.strip {
        let _symtab_idx = shdrs.len();
        shdrs.push(Sh {
            name: name_off(shstr, b".symtab\0"),
            typ: 2,
            flags: 0,
            addr: 0,
            off: symtab_off as u64,
            size: symtab.len() as u64,
            link: (shdrs.len() + 1) as u32, // .strtab next
            info: 1,
            addralign: 8,
            entsize: 24,
        });
        shdrs.push(Sh {
            name: name_off(shstr, b".strtab\0"),
            typ: 3,
            flags: 0,
            addr: 0,
            off: strtab_off as u64,
            size: strtab.len() as u64,
            link: 0,
            info: 0,
            addralign: 1,
            entsize: 0,
        });
    }

    let shnum = shdrs.len() as u16;
    let file_size = shdr_table_off + shnum as usize * 64;

    if file_size > 512 * 1024 * 1024 {
        return Err("ELF image exceeds 512 MiB safety limit".into());
    }

    let mut elf = vec![0u8; file_size];

    // Ehdr
    elf[0..4].copy_from_slice(b"\x7fELF");
    elf[4] = 2;
    elf[5] = 1;
    elf[6] = 1;
    elf[7] = 0; // ELFOSABI_SYSV
    let etype: u16 = if pie || opts.shared { 3 } else { 2 }; // ET_DYN / ET_EXEC
    put_u16(&mut elf, 16, etype);
    put_u16(&mut elf, 18, machine.elf_em());
    put_u32(&mut elf, 20, 1);
    put_u64(&mut elf, 24, entry_va);
    put_u64(&mut elf, 32, phoff as u64);
    put_u64(&mut elf, 40, shdr_table_off as u64);
    put_u32(&mut elf, 48, 0);
    put_u16(&mut elf, 52, 64);
    put_u16(&mut elf, 54, 56);
    put_u16(&mut elf, 56, phnum);
    put_u16(&mut elf, 58, 64);
    put_u16(&mut elf, 60, shnum);
    put_u16(&mut elf, 62, shstrndx);

    // Phdrs
    let mut ph = phoff;
    let emit_ph = |elf: &mut [u8],
                   ph: &mut usize,
                   typ: u32,
                   flags: u32,
                   off: u64,
                   vaddr: u64,
                   filesz: u64,
                   memsz: u64,
                   align: u64| {
        put_u32(elf, *ph, typ);
        put_u32(elf, *ph + 4, flags);
        put_u64(elf, *ph + 8, off);
        put_u64(elf, *ph + 16, vaddr);
        put_u64(elf, *ph + 24, vaddr);
        put_u64(elf, *ph + 32, filesz);
        put_u64(elf, *ph + 40, memsz);
        put_u64(elf, *ph + 48, align);
        debug_assert!(
            align <= 1 || (vaddr % align) == (off % align),
            "p_vaddr % p_align != p_offset % p_align"
        );
        *ph += 56;
    };

    emit_ph(
        &mut elf,
        &mut ph,
        PT_PHDR,
        PF_R,
        phoff as u64,
        base + phoff as u64,
        phdrs_bytes as u64,
        phdrs_bytes as u64,
        8,
    );
    if has_interp {
        emit_ph(
            &mut elf,
            &mut ph,
            PT_INTERP,
            PF_R,
            interp_off as u64,
            base + interp_off as u64,
            (interp_str.len() + 1) as u64,
            (interp_str.len() + 1) as u64,
            1,
        );
    }
    if has_note {
        emit_ph(
            &mut elf,
            &mut ph,
            PT_NOTE,
            PF_R,
            note_off as u64,
            base + note_off as u64,
            note_size as u64,
            note_size as u64,
            4,
        );
    }
    // LOAD R — headers
    emit_ph(
        &mut elf,
        &mut ph,
        PT_LOAD,
        PF_R,
        0,
        base,
        hdr_filesz as u64,
        hdr_filesz as u64,
        page_size as u64,
    );
    // LOAD RX
    emit_ph(
        &mut elf,
        &mut ph,
        PT_LOAD,
        PF_R | PF_X,
        text_file as u64,
        text_va,
        rx_filesz as u64,
        rx_filesz as u64,
        page_size as u64,
    );
    // LOAD RW
    emit_ph(
        &mut elf,
        &mut ph,
        PT_LOAD,
        PF_R | PF_W,
        data_file as u64,
        data_va,
        data_filesz as u64,
        data_memsz as u64,
        page_size as u64,
    );
    if has_tls {
        emit_ph(
            &mut elf,
            &mut ph,
            PT_TLS,
            PF_R,
            (data_file + tls_off_in_data) as u64,
            va_tls,
            merged.tls.len() as u64,
            tls_memsz,
            tls_align.max(1),
        );
    }
    if has_dynamic {
        emit_ph(
            &mut elf,
            &mut ph,
            PT_DYNAMIC,
            PF_R | PF_W,
            data_file as u64,
            va_dynamic,
            dynamic_size as u64,
            dynamic_size as u64,
            8,
        );
    }
    if let Some((eh_off, eh_len)) = merged.eh_frame_info() {
        emit_ph(
            &mut elf,
            &mut ph,
            PT_GNU_EH_FRAME,
            PF_R,
            (text_file + rodata_off_in_text + eh_off) as u64,
            va_rodata + eh_off as u64,
            eh_len as u64,
            eh_len as u64,
            8,
        );
    }
    emit_ph(
        &mut elf,
        &mut ph,
        PT_GNU_STACK, // PT_GNU_STACK
        PF_R | PF_W,
        0,
        0,
        0,
        0,
        16,
    );
    let _ = (PT_NULL, PT_GNU_EH_FRAME, PT_GNU_RELRO);

    // interp + note
    if has_interp {
        elf[interp_off..interp_off + interp_str.len()].copy_from_slice(interp_str.as_bytes());
    }
    if has_note {
        put_u32(&mut elf, note_off, 4);
        put_u32(&mut elf, note_off + 4, 16);
        put_u32(&mut elf, note_off + 8, 3); // NT_GNU_BUILD_ID
        elf[note_off + 12..note_off + 16].copy_from_slice(b"GNU\0");
        let digest = sha256(&rx);
        elf[note_off + 16..note_off + 32].copy_from_slice(&digest[..16]);
    }

    elf[text_file..text_file + rx.len()].copy_from_slice(&rx);
    elf[data_file..data_file + rw.len()].copy_from_slice(&rw);

    elf[shstr_off..shstr_off + shstr.len()].copy_from_slice(shstr);
    if !opts.strip {
        elf[symtab_off..symtab_off + symtab.len()].copy_from_slice(&symtab);
        elf[strtab_off..strtab_off + strtab.len()].copy_from_slice(&strtab);
    }
    for (i, sh) in shdrs.iter().enumerate() {
        let o = shdr_table_off + i * 64;
        put_u32(&mut elf, o, sh.name);
        put_u32(&mut elf, o + 4, sh.typ);
        put_u64(&mut elf, o + 8, sh.flags);
        put_u64(&mut elf, o + 16, sh.addr);
        put_u64(&mut elf, o + 24, sh.off);
        put_u64(&mut elf, o + 32, sh.size);
        put_u32(&mut elf, o + 40, sh.link);
        put_u32(&mut elf, o + 44, sh.info);
        put_u64(&mut elf, o + 48, sh.addralign);
        put_u64(&mut elf, o + 56, sh.entsize);
    }

    fs::write(output, elf).map_err(|e| format!("write '{}': {e}", output.display()))?;
    chmod_exec(output)?;
    vlog(
        opts,
        format!(
            "ELF {:?} {}  entry=0x{entry_va:x}  phnum={phnum}  dynamic={dyn_mode}",
            machine,
            output.display()
        ),
    );
    Ok(())
}
