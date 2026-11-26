use super::*;

pub fn write_elf(inputs: &[PathBuf], output: &Path) -> Result<(), String> {
    write_elf_with_options(inputs, output, &LinkOptions::default())
}

pub fn write_elf_with_options(
    inputs: &[PathBuf],
    output: &Path,
    opts: &LinkOptions,
) -> Result<(), String> {
    write_elf_with_options_t(inputs, output, opts).map_err(String::from)
}

/// Typed variant of [`write_elf_with_options`] returning the engine's [`LinkError`]
/// (6A.2 boundary); the public function is its `String` adapter.
pub fn write_elf_with_options_t(
    inputs: &[PathBuf],
    output: &Path,
    opts: &LinkOptions,
) -> Result<(), LinkError> {
    let (objects, _, machine) = load_objects(inputs, opts)?;
    let mut merged = merge_objects(&objects, opts)?;

    let mut undef = collect_undefined(&objects, &merged);
    // entry names are not "undefined" if we will synthesize _start
    let want_dynamic = if opts.shared {
        true
    } else {
        match opts.dynamic {
            DynamicMode::Force => true,
            DynamicMode::Static => false,
            DynamicMode::Auto => {
                !undef.is_empty() && undef.iter().any(|u| libc_soname_for(u).is_some())
            }
        }
    };
    if !want_dynamic && opts.dynamic == DynamicMode::Static && !undef.is_empty() {
        return Err(LinkError::unresolved(
            "unresolved symbols in static link",
            undef,
        ));
    }

    let page_size = opts
        .page_size
        .unwrap_or(machine.page_size(OutputFormat::Elf));
    let pie = opts.pie || opts.shared;
    let base = if pie {
        0u64
    } else {
        opts.image_base.unwrap_or(ELF_BASE_EXEC)
    };

    // ── decide entry / maybe inject _start ──────────────────────────────
    let entry_names_user = opts.entry.clone();
    let has_start = merged.syms.contains_key("_start") || merged.syms.contains_key("start");
    let has_main = merged.syms.contains_key("main") || merged.syms.contains_key("lpp_main");

    // Startup PLT calls needing a post-layout `call rel32` patch to a PLT slot
    // (e.g. hosted glibc's `__libc_start_main`). Empty for freestanding images.
    let mut startup_plt_calls: Vec<(&'static str, usize)> = Vec::new();
    if !opts.shared && !opts.no_startup && !has_start && has_main {
        let main_off = merged
            .syms
            .get("main")
            .or_else(|| merged.syms.get("lpp_main"))
            .map(|s| s.offset)
            .unwrap();
        let start_off = align_up(merged.text.len(), 16);
        merged.text.resize(start_off, 0x90);
        let stub_len: usize;
        match machine {
            Machine::X86_64 => {
                let abi = if want_dynamic {
                    StartupAbi::HostedGlibc
                } else {
                    StartupAbi::Freestanding
                };
                let (stub, plt_calls) = emit_elf_start_stub_x64(start_off, main_off as usize, abi)?;
                for (sym, site) in plt_calls {
                    startup_plt_calls.push((sym, site));
                    if !undef.iter().any(|u| u == sym) {
                        undef.push(sym.to_string());
                    }
                }
                stub_len = stub.len();
                merged.text.extend_from_slice(&stub);
            }
            Machine::Aarch64 => {
                // TODO(hosted-glibc aarch64): mirror the x86_64 __libc_start_main
                // bootstrap. The current stub calls `main` directly + raw exit
                // syscall, which is correct only for freestanding images.
                let mut stub = emit_elf_start_stub_a64();
                let p = (start_off + 4) as u64; // bl instruction is at start_off + 4 (after bti c)
                let s = main_off;
                let dest = s;
                let disp = dest as i64 - p as i64;
                if disp & 3 != 0 || !fits_i26(disp >> 2) {
                    return Err("aarch64 startup BL out of range".into());
                }
                let imm = (disp >> 2) as u32;
                stub[4..8].copy_from_slice(&(0x9400_0000u32 | (imm & 0x03FF_FFFF)).to_le_bytes());
                stub_len = stub.len();
                merged.text.extend_from_slice(&stub);
            }
        }
        merged.syms.insert(
            "_start".into(),
            ResolvedSym {
                name: "_start".into(),
                bind: Bind::Global,
                class: SectionClass::Text,
                offset: start_off as u64,
                size: stub_len as u64,
                object: usize::MAX,
            },
        );
    }

    // ── GOT / PLT for remaining undefined + GOT-relative relocs ─────────
    let mut got_keys: BTreeMap<String, usize> = BTreeMap::new();
    let mut plt_keys: BTreeMap<String, usize> = BTreeMap::new();
    let mut dyn_needed = if want_dynamic {
        elf_needed_for(&undef, opts)
    } else {
        Vec::new()
    };

    for (oi, o) in objects.iter().enumerate() {
        for rel in &o.relocations {
            let needs_got = matches!(
                rel.raw_type,
                R_X86_64_GOTPCREL
                    | R_X86_64_GOTPCRELX
                    | R_X86_64_REX_GOTPCRELX
                    | R_X86_64_GOT32
                    | R_X86_64_GOTPC32
                    | R_X86_64_GOTTPOFF
                    | R_AARCH64_ADR_GOT_PAGE
                    | R_AARCH64_LD64_GOT_LO12_NC
            ) || rel.kind == RelocationKind::GotRelative
                || rel.kind == RelocationKind::Got;
            if needs_got {
                let key = got_key(oi, &rel.target);
                let n = got_keys.len();
                got_keys.entry(key).or_insert(n);
            }
            if let RelTarget::Name(name) = &rel.target {
                let is_undef = !merged.syms.contains_key(name);
                let needs_plt = is_undef
                    && (rel.raw_type == R_X86_64_PLT32
                        || rel.raw_type == R_AARCH64_CALL26
                        || rel.raw_type == R_AARCH64_JUMP26
                        || rel.kind == RelocationKind::PltRelative);
                if needs_plt
                    || (is_undef
                        && want_dynamic
                        && (rel.raw_type == R_X86_64_PC32 || rel.kind == RelocationKind::Relative))
                {
                    let n = plt_keys.len();
                    plt_keys.entry(name.clone()).or_insert(n);
                }
            }
        }
    }
    if want_dynamic {
        for u in &undef {
            let n = plt_keys.len();
            plt_keys.entry(u.clone()).or_insert(n);
        }
    }
    for (sym, _site) in &startup_plt_calls {
        let n = plt_keys.len();
        plt_keys.entry((*sym).to_string()).or_insert(n);
    }

    if !want_dynamic {
        let leftover: Vec<String> = undef
            .into_iter()
            .filter(|u| !merged.syms.contains_key(u) && !plt_keys.contains_key(u))
            .collect();
        if !leftover.is_empty() {
            return Err(LinkError::unresolved("unresolved symbols", leftover));
        }
    } else {
        // anything not going through PLT/GOT and not defined is still an error
        let leftover: Vec<String> = undef
            .iter()
            .filter(|u| {
                !merged.syms.contains_key(*u)
                    && !plt_keys.contains_key(*u)
                    && !got_keys.contains_key(*u)
                    && libc_soname_for(u).is_none()
            })
            .cloned()
            .collect();
        if !leftover.is_empty() && opts.dynamic == DynamicMode::Static {
            return Err(LinkError::unresolved("unresolved symbols", leftover));
        }
        for u in &undef {
            if !merged.syms.contains_key(u) && !plt_keys.contains_key(u) {
                let n = plt_keys.len();
                plt_keys.entry(u.clone()).or_insert(n);
            }
        }
        if dyn_needed.is_empty() {
            dyn_needed = elf_needed_for(&undef, opts);
        }
    }

    let dyn_mode = want_dynamic && (opts.shared || !plt_keys.is_empty());

    // PLT / GOT.PLT bytes
    let (plt_entsize, plt0_size) = match machine {
        Machine::X86_64 => (16usize, 16usize),
        Machine::Aarch64 => (16usize, 32usize),
    };
    let plt_size = if dyn_mode {
        plt0_size + plt_keys.len() * plt_entsize
    } else {
        0
    };
    let gotplt_entries = if dyn_mode { 3 + plt_keys.len() } else { 0 };
    let got_entries = got_keys.len();

    // ── file / VA layout ────────────────────────────────────────────────
    // Program headers we will emit:
    //   PHDR, [INTERP], [NOTE], LOAD_R (headers), LOAD_RX, LOAD_RW, [TLS], [DYNAMIC], GNU_STACK
    let ehdr_size = 64usize;
    let phdr_entsize = 56usize;

    // First compute phnum
    let has_interp = dyn_mode && !opts.shared;
    let has_note = opts.build_id;
    let has_tls = !merged.tls.is_empty() || merged.tbss_size > 0;
    let has_dynamic = dyn_mode;
    let has_eh_frame = merged.eh_frame_info().is_some();
    let mut phnum = 2; // PHDR + GNU_STACK
    phnum += 3; // three LOADs (R, RX, RW)
    if has_interp {
        phnum += 1;
    }
    if has_note {
        phnum += 1;
    }
    if has_tls {
        phnum += 1;
    }
    if has_dynamic {
        phnum += 1;
    }
    if has_eh_frame {
        phnum += 1;
    }

    let phoff = ehdr_size;
    let phdrs_bytes = phnum as usize * phdr_entsize;
    let mut ro_cursor = phoff + phdrs_bytes;

    let interp_str = opts
        .dynamic_linker
        .clone()
        .unwrap_or_else(|| elf_interp(machine).to_string());
    let interp_off = if has_interp {
        let o = ro_cursor;
        ro_cursor += interp_str.len() + 1;
        o
    } else {
        0
    };

    // build-id note: namesz=4, descsz=16, type=3, "GNU\0" + 16 bytes
    let note_off = if has_note {
        ro_cursor = align_up(ro_cursor, 4);
        let o = ro_cursor;
        ro_cursor += 4 + 4 + 4 + 4 + 16;
        o
    } else {
        0
    };
    let note_size = if has_note { 32usize } else { 0 };

    // Dynamic tables live in the RX/R segment after text+rodata for RELRO-less
    // simplicity: we put dynsym/dynstr/hash/rela in the RX image after rodata,
    // and .dynamic + .got.plt in RW.

    // Text segment starts at next page.
    let text_file = align_up(ro_cursor, page_size);
    // VA of file offset 0 is `base` for the header LOAD.
    // Identity: vaddr = base + file_off  for the first pages, then RX at base+text_file.
    let hdr_filesz = ro_cursor;
    let text_va = base + text_file as u64;

    let text_len = merged.text.len();
    let plt_off_in_text = align_up(text_len, 16);
    let rodata_off_in_text = align_up(plt_off_in_text + plt_size, 16);
    let init_off_in_text = align_up(rodata_off_in_text + merged.rodata.len(), 8);
    let fini_off_in_text = align_up(init_off_in_text + merged.init_array.len(), 8);

    let exported_syms: Vec<ResolvedSym> = if opts.shared || dyn_mode {
        merged
            .syms
            .values()
            .filter(|s| s.bind == Bind::Global || s.bind == Bind::Weak)
            .cloned()
            .collect()
    } else {
        Vec::new()
    };

    // After arrays: dynsym / dynstr / hash / rela
    let mut dyn_cursor = align_up(fini_off_in_text + merged.fini_array.len(), 8);
    let dynsym_count = if dyn_mode || opts.shared {
        1 + plt_keys.len() + exported_syms.len()
    } else {
        0
    };
    let dynsym_off = dyn_cursor;
    let dynsym_size = dynsym_count * 24;
    dyn_cursor += dynsym_size;

    let mut dynstr = vec![0u8]; // index 0 = empty
    let mut dynstr_index: HashMap<String, u32> = HashMap::new();
    let put_dynstr =
        |s: &str, dynstr: &mut Vec<u8>, dynstr_index: &mut HashMap<String, u32>| -> u32 {
            if let Some(&i) = dynstr_index.get(s) {
                return i;
            }
            let i = dynstr.len() as u32;
            dynstr.extend_from_slice(s.as_bytes());
            dynstr.push(0);
            dynstr_index.insert(s.to_string(), i);
            i
        };
    let mut needed_str_off = Vec::new();
    let mut soname_str_off: Option<u32> = None;
    let mut runpath_str_off: Option<u32> = None;
    if dyn_mode || opts.shared {
        for n in &dyn_needed {
            needed_str_off.push(put_dynstr(n, &mut dynstr, &mut dynstr_index));
        }
        if !opts.rpath.is_empty() {
            let joined = opts.rpath.join(":");
            runpath_str_off = Some(put_dynstr(&joined, &mut dynstr, &mut dynstr_index));
        }
        for name in plt_keys.keys() {
            put_dynstr(name, &mut dynstr, &mut dynstr_index);
        }
        for sy in &exported_syms {
            put_dynstr(&sy.name, &mut dynstr, &mut dynstr_index);
        }
        if let Some(sn) = &opts.soname {
            soname_str_off = Some(put_dynstr(sn, &mut dynstr, &mut dynstr_index));
        } else if opts.shared {
            let def_soname = output
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("libout.so");
            soname_str_off = Some(put_dynstr(def_soname, &mut dynstr, &mut dynstr_index));
        }
    }
    let dynstr_off = dyn_cursor;
    let dynstr_size = dynstr.len();
    dyn_cursor = align_up(dyn_cursor + dynstr_size, 8);

    let hash_off = dyn_cursor;
    let hash_nbucket = dynsym_count.max(1);
    let hash_size = if dyn_mode || opts.shared {
        (2 + hash_nbucket + dynsym_count) * 4
    } else {
        0
    };
    dyn_cursor = align_up(dyn_cursor + hash_size, 8);

    let rela_plt_count = if dyn_mode { plt_keys.len() } else { 0 };
    let rela_plt_off = dyn_cursor;
    let rela_plt_size = rela_plt_count * 24;
    dyn_cursor += rela_plt_size;

    let rx_size = dyn_cursor;
    let rx_filesz = rx_size;

    // Data segment: next page in VA, congruent file offset.
    let data_va = align_up_u64(text_va + rx_size as u64, page_size as u64);
    // Choose data_file such that data_file % page == data_va % page.
    let want_mod = congruent_offset(data_va, page_size as u64) as usize;
    let mut data_file = align_up(text_file + rx_filesz, page_size);
    // bump until congruence holds (usually already true if both page-aligned)
    while data_file % page_size != want_mod {
        data_file += 1;
        if data_file > text_file + rx_filesz + page_size {
            data_file = align_up(text_file + rx_filesz, page_size) + want_mod;
            break;
        }
    }

    // Fixed dynamic tags emitted below are: HASH, STRTAB, SYMTAB, STRSZ,
    // SYMENT, PLTGOT, PLTRELSZ, PLTREL, JMPREL, RELA, RELASZ, RELAENT,
    // DEBUG, FLAGS_1, NULL = 15 entries, plus one DT_NEEDED per library.
    const ELF_DYNAMIC_FIXED_ENTRIES: usize = 15;
    let dynamic_ents = if dyn_mode || opts.shared {
        dyn_needed.len()
            + ELF_DYNAMIC_FIXED_ENTRIES
            + if soname_str_off.is_some() { 1 } else { 0 }
            + if runpath_str_off.is_some() { 1 } else { 0 }
    } else {
        0
    };
    let dynamic_off_in_data = 0usize;
    let dynamic_size = dynamic_ents * 16;
    let gotplt_off_in_data = align_up(dynamic_size, 8);
    let gotplt_size = gotplt_entries * 8;
    let got_off_in_data = align_up(gotplt_off_in_data + gotplt_size, 8);
    let got_size = got_entries * 8;
    let data_off_in_data = align_up(got_off_in_data + got_size, 16);
    let tls_off_in_data = align_up(
        data_off_in_data + merged.data.len(),
        merged.tls_align.max(1),
    );
    let data_filesz = tls_off_in_data + merged.tls.len();
    let data_memsz = data_filesz + merged.tbss_size + merged.bss_size;

    let tls_memsz = (merged.tls.len() + merged.tbss_size) as u64;
    let tls_align = merged.tls_align.max(1) as u64;

    // Absolute VAs
    let va_text = text_va;
    let va_plt = text_va + plt_off_in_text as u64;
    let va_rodata = text_va + rodata_off_in_text as u64;
    let va_init = text_va + init_off_in_text as u64;
    let va_fini = text_va + fini_off_in_text as u64;
    let va_dynsym = text_va + dynsym_off as u64;
    let va_dynstr = text_va + dynstr_off as u64;
    let va_hash = text_va + hash_off as u64;
    let va_relaplt = text_va + rela_plt_off as u64;
    let va_data = data_va + data_off_in_data as u64;
    let va_dynamic = data_va + dynamic_off_in_data as u64;
    let va_gotplt = data_va + gotplt_off_in_data as u64;
    let va_got = data_va + got_off_in_data as u64;
    let va_tls = data_va + tls_off_in_data as u64;
    let va_bss = data_va + data_filesz as u64;

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

    let lookup_va = |target: &RelTarget, obj_idx: usize| -> Result<u64, LinkError> {
        match target {
            RelTarget::Local(class, off) => {
                let base_off = match class {
                    SectionClass::Text => merged.place[obj_idx].text as u64,
                    SectionClass::Rodata => merged.place[obj_idx].rodata as u64,
                    SectionClass::Data => merged.place[obj_idx].data as u64,
                    SectionClass::Bss => merged.place[obj_idx].bss as u64,
                    SectionClass::Tls => merged.place[obj_idx].tls as u64,
                    SectionClass::InitArray => merged.place[obj_idx].init_array as u64,
                    SectionClass::FiniArray => merged.place[obj_idx].fini_array as u64,
                };
                Ok(sec_va(*class) + base_off + *off)
            }
            RelTarget::Name(n) => {
                if let Some(sy) = merged.syms.get(n) {
                    return Ok(sec_va(sy.class) + sy.offset);
                }
                if let Some(&i) = plt_keys.get(n) {
                    return Ok(va_plt + plt0_size as u64 + (i * plt_entsize) as u64);
                }
                if n == "__ImageBase" {
                    return Ok(base);
                }
                Err(format!("unresolved symbol '{n}'").into())
            }
        }
    };

    let lookup_size = |n: &str| -> u64 { merged.syms.get(n).map(|s| s.size).unwrap_or(0) };

    // ── materialise RX / RW buffers ─────────────────────────────────────
    let mut rx = vec![0u8; rx_size];
    rx[..merged.text.len()].copy_from_slice(&merged.text);
    rx[rodata_off_in_text..rodata_off_in_text + merged.rodata.len()]
        .copy_from_slice(&merged.rodata);
    rx[init_off_in_text..init_off_in_text + merged.init_array.len()]
        .copy_from_slice(&merged.init_array);
    rx[fini_off_in_text..fini_off_in_text + merged.fini_array.len()]
        .copy_from_slice(&merged.fini_array);

    let mut rw = vec![0u8; data_filesz];
    rw[data_off_in_data..data_off_in_data + merged.data.len()].copy_from_slice(&merged.data);
    rw[tls_off_in_data..tls_off_in_data + merged.tls.len()].copy_from_slice(&merged.tls);

    // Fill GOT with symbol VAs (or TLS offsets).
    let mut got_list: Vec<(String, usize)> =
        got_keys.iter().map(|(k, v)| (k.clone(), *v)).collect();
    got_list.sort_by_key(|(_, i)| *i);
    for (name, idx) in &got_list {
        let val = if let Some(rest) = name.strip_prefix("__local.") {
            // __local.{obj}.{class}.{off}
            let mut it = rest.splitn(3, '.');
            let oi: usize = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let tag = it.next().unwrap_or("text");
            let off: u64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let class = match tag {
                "rdata" => SectionClass::Rodata,
                "data" => SectionClass::Data,
                "bss" => SectionClass::Bss,
                "tls" => SectionClass::Tls,
                "init_array" => SectionClass::InitArray,
                "fini_array" => SectionClass::FiniArray,
                _ => SectionClass::Text,
            };
            let base = match class {
                SectionClass::Text => merged.place[oi].text as u64,
                SectionClass::Rodata => merged.place[oi].rodata as u64,
                SectionClass::Data => merged.place[oi].data as u64,
                SectionClass::Bss => merged.place[oi].bss as u64,
                SectionClass::Tls => merged.place[oi].tls as u64,
                SectionClass::InitArray => merged.place[oi].init_array as u64,
                SectionClass::FiniArray => merged.place[oi].fini_array as u64,
            };
            if class == SectionClass::Tls {
                match machine {
                    Machine::X86_64 => x64_tpoff(base + off, tls_memsz) as u64,
                    Machine::Aarch64 => a64_tpoff(base + off, tls_memsz) as u64,
                }
            } else {
                sec_va(class) + base + off
            }
        } else if let Some(sy) = merged.syms.get(name) {
            if sy.class == SectionClass::Tls {
                match machine {
                    Machine::X86_64 => x64_tpoff(sy.offset, tls_memsz) as u64,
                    Machine::Aarch64 => a64_tpoff(sy.offset, tls_memsz) as u64,
                }
            } else {
                sec_va(sy.class) + sy.offset
            }
        } else if let Some(&i) = plt_keys.get(name) {
            va_plt + plt0_size as u64 + (i * plt_entsize) as u64
        } else {
            0
        };
        let pos = got_off_in_data + idx * 8;
        rw[pos..pos + 8].copy_from_slice(&val.to_le_bytes());
    }

    // Build PLT / GOT.PLT
    if dyn_mode {
        match machine {
            Machine::X86_64 => {
                // PLT0: push [rip+gotplt+8]; jmp [rip+gotplt+16]
                let p0 = plt_off_in_text;
                rx[p0] = 0xff;
                rx[p0 + 1] = 0x35;
                let next = va_plt + 6;
                let d1 = va_gotplt as i64 + 8 - next as i64;
                rx[p0 + 2..p0 + 6].copy_from_slice(&(d1 as i32).to_le_bytes());
                rx[p0 + 6] = 0xff;
                rx[p0 + 7] = 0x25;
                let next2 = va_plt + 12;
                let d2 = va_gotplt as i64 + 16 - next2 as i64;
                rx[p0 + 8..p0 + 12].copy_from_slice(&(d2 as i32).to_le_bytes());
                // GOT.PLT[0] = _DYNAMIC
                rw[gotplt_off_in_data..gotplt_off_in_data + 8]
                    .copy_from_slice(&va_dynamic.to_le_bytes());
                let mut plt_ordered: Vec<(String, usize)> =
                    plt_keys.iter().map(|(k, v)| (k.clone(), *v)).collect();
                plt_ordered.sort_by_key(|(_, i)| *i);
                for (i, (_name, _)) in plt_ordered.iter().enumerate() {
                    let po = plt_off_in_text + plt0_size + i * 16;
                    let slot_va = va_gotplt + 24 + (i as u64) * 8;
                    let plt_va = va_plt + plt0_size as u64 + (i as u64) * 16;
                    // jmp [rip+got]
                    rx[po] = 0xff;
                    rx[po + 1] = 0x25;
                    let disp = slot_va as i64 - (plt_va as i64 + 6);
                    rx[po + 2..po + 6].copy_from_slice(&(disp as i32).to_le_bytes());
                    // push reloc index
                    rx[po + 6] = 0x68;
                    rx[po + 7..po + 11].copy_from_slice(&(i as u32).to_le_bytes());
                    // jmp plt0
                    rx[po + 11] = 0xe9;
                    let back = va_plt as i64 - (plt_va as i64 + 16);
                    rx[po + 12..po + 16].copy_from_slice(&(back as i32).to_le_bytes());
                    // initial GOT.PLT[3+i] = PLT[i]+6 (push)
                    let lazy = plt_va + 6;
                    let gp = gotplt_off_in_data + 24 + i * 8;
                    rw[gp..gp + 8].copy_from_slice(&lazy.to_le_bytes());
                }
            }
            Machine::Aarch64 => {
                // PLT0: bti c; stp x16,x30,[sp,#-16]!; adrp/ldr/add/br to GOT.PLT+16
                let p0 = plt_off_in_text;
                // bti c (0xd503245f) — ARM64 BTI compatibility for indirect branch br x17 from PLT[i]
                rx[p0..p0 + 4].copy_from_slice(&0xd503245fu32.to_le_bytes());
                // stp x16, x30, [sp, #-16]!
                rx[p0 + 4..p0 + 8].copy_from_slice(&0xa9bf7bf0u32.to_le_bytes());
                // adrp x16, page(GOT.PLT+16) — load resolver from GOT.PLT[2]
                let dest = va_gotplt + 16;
                let p = va_plt + 8; // adrp is at offset 8 (p0 + 8)
                let delta = self::page(dest) as i64 - self::page(p) as i64;
                let imm = delta >> 12;
                let immlo = (imm as u32) & 3;
                let immhi = ((imm as u32) >> 2) & 0x7_ffff;
                let adrp = 0x90000010u32 | (immlo << 29) | (immhi << 5);
                rx[p0 + 8..p0 + 12].copy_from_slice(&adrp.to_le_bytes());
                // ldr x17, [x16, #lo12(GOT.PLT[2])]  — x17 = resolver fn
                let lo = ((dest & 0xfff) >> 3) as u32;
                let ldr = 0xf9400211u32 | (lo << 10);
                rx[p0 + 12..p0 + 16].copy_from_slice(&ldr.to_le_bytes());
                // add x16, x16, #lo12(GOT.PLT[2]) — x16 = &GOT.PLT[2]
                let add = 0x91000210u32 | (((dest & 0xfff) as u32) << 10);
                rx[p0 + 16..p0 + 20].copy_from_slice(&add.to_le_bytes());
                // br x17
                rx[p0 + 20..p0 + 24].copy_from_slice(&0xd61f0220u32.to_le_bytes());
                // GOT.PLT[0..2] = 0; ld.so fills [0]=link_map, [1]=module_id, [2]=resolver at startup.
                let mut plt_ordered: Vec<(String, usize)> =
                    plt_keys.iter().map(|(k, v)| (k.clone(), *v)).collect();
                plt_ordered.sort_by_key(|(_, i)| *i);
                for (i, (_name, _)) in plt_ordered.iter().enumerate() {
                    let po = plt_off_in_text + plt0_size + i * 16;
                    let slot = va_gotplt + 24 + (i as u64) * 8;
                    let plt_va_i = va_plt + plt0_size as u64 + (i as u64) * 16;
                    // adrp x16, page(slot)
                    let delta = self::page(slot) as i64 - self::page(plt_va_i) as i64;
                    let imm = delta >> 12;
                    let immlo = (imm as u32) & 3;
                    let immhi = ((imm as u32) >> 2) & 0x7_ffff;
                    let adrp = 0x90000010u32 | (immlo << 29) | (immhi << 5);
                    rx[po..po + 4].copy_from_slice(&adrp.to_le_bytes());
                    let lo = ((slot & 0xfff) >> 3) as u32;
                    let ldr = 0xf9400211u32 | (lo << 10);
                    rx[po + 4..po + 8].copy_from_slice(&ldr.to_le_bytes());
                    let add = 0x91000210u32 | (((slot & 0xfff) as u32) << 10);
                    rx[po + 8..po + 12].copy_from_slice(&add.to_le_bytes());
                    rx[po + 12..po + 16].copy_from_slice(&0xd61f0220u32.to_le_bytes());
                    // GOT.PLT[i] initially points to PLT0 (va_plt).
                    let gp = gotplt_off_in_data + 24 + i * 8;
                    rw[gp..gp + 8].copy_from_slice(&va_plt.to_le_bytes());
                }
            }
        }
    }

    // dynsym / dynstr / hash / rela.plt
    if dyn_mode || opts.shared {
        // dynsym[0] = NULL already zeroed
        let mut plt_ordered: Vec<(String, usize)> =
            plt_keys.iter().map(|(k, v)| (k.clone(), *v)).collect();
        plt_ordered.sort_by_key(|(_, i)| *i);
        for (i, (name, _)) in plt_ordered.iter().enumerate() {
            let so = *dynstr_index.get(name).unwrap_or(&0);
            let e = dynsym_off + 24 * (i + 1);
            put_u32(&mut rx, e, so); // st_name
            rx[e + 4] = 0x12; // STB_GLOBAL STT_FUNC
            rx[e + 5] = 0; // st_other
            put_u16(&mut rx, e + 6, 0); // shndx UNDEF
            put_u64(&mut rx, e + 8, 0);
            put_u64(&mut rx, e + 16, 0);
        }

        let base_idx = 1 + plt_keys.len();
        for (i, sy) in exported_syms.iter().enumerate() {
            let so = *dynstr_index.get(&sy.name).unwrap_or(&0);
            let e = dynsym_off + 24 * (base_idx + i);
            let va = sec_va(sy.class) + sy.offset;
            let shndx: u16 = match sy.class {
                SectionClass::Text => 1,
                SectionClass::Rodata => 2,
                SectionClass::Data => 3,
                SectionClass::Bss => 4,
                SectionClass::Tls => 5,
                SectionClass::InitArray => 6,
                SectionClass::FiniArray => 7,
            };
            let st_type = match sy.class {
                SectionClass::Text => 0x12, // STB_GLOBAL STT_FUNC
                _ => 0x11,                  // STB_GLOBAL STT_OBJECT
            };
            put_u32(&mut rx, e, so); // st_name
            rx[e + 4] = st_type;
            rx[e + 5] = 0; // st_other
            put_u16(&mut rx, e + 6, shndx);
            put_u64(&mut rx, e + 8, va);
            put_u64(&mut rx, e + 16, sy.size);
        }
        rx[dynstr_off..dynstr_off + dynstr.len()].copy_from_slice(&dynstr);

        // SysV hash
        let mut hash = vec![0u8; hash_size];
        put_u32(&mut hash, 0, hash_nbucket as u32);
        put_u32(&mut hash, 4, dynsym_count as u32);
        let mut buckets = vec![0u32; hash_nbucket];
        let mut chains = vec![0u32; dynsym_count];
        for (i, (name, _)) in plt_ordered.iter().enumerate() {
            let si = i + 1;
            let h = elf_hash(name.as_bytes()) as usize % hash_nbucket;
            chains[si] = buckets[h];
            buckets[h] = si as u32;
        }
        for (i, sy) in exported_syms.iter().enumerate() {
            let si = base_idx + i;
            let h = elf_hash(sy.name.as_bytes()) as usize % hash_nbucket;
            chains[si] = buckets[h];
            buckets[h] = si as u32;
        }
        let mut hp = 8;
        for b in buckets {
            put_u32(&mut hash, hp, b);
            hp += 4;
        }
        for c in chains {
            put_u32(&mut hash, hp, c);
            hp += 4;
        }
        rx[hash_off..hash_off + hash_size].copy_from_slice(&hash);

        // rela.plt
        for (i, _) in plt_ordered.iter().enumerate() {
            let e = rela_plt_off + i * 24;
            let off = va_gotplt + 24 + (i as u64) * 8;
            put_u64(&mut rx, e, off);
            let r_type = match machine {
                Machine::X86_64 => 7u64, // R_X86_64_JUMP_SLOT
                Machine::Aarch64 => R_AARCH64_JUMP_SLOT as u64,
            };
            let r_info = ((i as u64 + 1) << 32) | r_type;
            put_u64(&mut rx, e + 8, r_info);
            put_u64(&mut rx, e + 16, 0);
        }

        // .dynamic
        let mut dynb = vec![0u8; dynamic_size];
        let mut dp = 0;
        let put_dt = |dynb: &mut [u8], dp: &mut usize, tag: i64, val: u64| {
            put_u64(dynb, *dp, tag as u64);
            put_u64(dynb, *dp + 8, val);
            *dp += 16;
        };
        for off in &needed_str_off {
            put_dt(&mut dynb, &mut dp, 1, *off as u64); // DT_NEEDED
        }
        put_dt(&mut dynb, &mut dp, 4, va_hash); // DT_HASH
        put_dt(&mut dynb, &mut dp, 5, va_dynstr); // DT_STRTAB
        put_dt(&mut dynb, &mut dp, 6, va_dynsym); // DT_SYMTAB
        put_dt(&mut dynb, &mut dp, 10, dynstr_size as u64); // DT_STRSZ
        put_dt(&mut dynb, &mut dp, 11, 24); // DT_SYMENT
        put_dt(&mut dynb, &mut dp, 3, va_gotplt); // DT_PLTGOT
        put_dt(&mut dynb, &mut dp, 2, rela_plt_size as u64); // DT_PLTRELSZ
        put_dt(&mut dynb, &mut dp, 20, 7); // DT_PLTREL = DT_RELA
        put_dt(&mut dynb, &mut dp, 23, va_relaplt); // DT_JMPREL
        put_dt(&mut dynb, &mut dp, 7, va_relaplt); // DT_RELA
        put_dt(&mut dynb, &mut dp, 8, rela_plt_size as u64); // DT_RELASZ
        put_dt(&mut dynb, &mut dp, 9, 24); // DT_RELAENT
        if let Some(so_off) = soname_str_off {
            put_dt(&mut dynb, &mut dp, 14, so_off as u64); // DT_SONAME
        }
        if let Some(rp_off) = runpath_str_off {
            put_dt(&mut dynb, &mut dp, 29, rp_off as u64); // DT_RUNPATH
        }
        put_dt(&mut dynb, &mut dp, 21, 0); // DT_DEBUG
        let mut flags1 = 0u64;
        if opts.pie && !opts.shared {
            flags1 |= 0x0800_0000; // DF_1_PIE (executables only, NOT shared libraries)
        }
        put_dt(&mut dynb, &mut dp, 0x6fff_fffb, flags1); // DT_FLAGS_1
        put_dt(&mut dynb, &mut dp, 0, 0); // DT_NULL
        rw[..dynamic_size].copy_from_slice(&dynb[..dynamic_size.min(dynb.len())]);
    }

    // Patch synthesized-startup calls (e.g. hosted glibc's __libc_start_main,
    // or the freestanding exit shim) through their PLT slots.
    for (sym, site) in &startup_plt_calls {
        if let Some(&i) = plt_keys.get(*sym) {
            let plt = va_plt + plt0_size as u64 + (i * plt_entsize) as u64;
            let p = va_text + *site as u64;
            let disp = plt as i64 - (p as i64 + 4);
            write_i32_at(&mut rx, *site, disp, "startup PLT call")?;
        }
    }

    apply_elf_relocations(
        ElfRelocationContext {
            objects: &objects,
            merged: &merged,
            machine,
            rx: &mut rx,
            rw: &mut rw,
            rodata_off_in_text,
            init_off_in_text,
            fini_off_in_text,
            data_off_in_data,
            tls_off_in_data,
            va_text,
            va_rodata,
            va_init,
            va_fini,
            va_data,
            va_tls,
            va_got,
            got_keys: &got_keys,
            tls_memsz,
        },
        lookup_va,
        lookup_size,
    )?;

    // ── entry ───────────────────────────────────────────────────────────
    let entry_name = if let Some(e) = &entry_names_user {
        e.clone()
    } else if merged.syms.contains_key("_start") {
        "_start".into()
    } else if merged.syms.contains_key("start") {
        "start".into()
    } else if merged.syms.contains_key("main") {
        "main".into()
    } else if merged.syms.contains_key("lpp_main") {
        "lpp_main".into()
    } else if opts.shared {
        "".to_string()
    } else {
        return Err("required symbol 'main' (or 'lpp_main' / '_start') not found".into());
    };
    let entry_va = if entry_name.is_empty() {
        0u64
    } else {
        let entry_sym = merged
            .syms
            .get(&entry_name)
            .ok_or_else(|| format!("entry symbol '{entry_name}' not defined"))?;
        sec_va(entry_sym.class) + entry_sym.offset
    };

    if let Some(mp) = &opts.map_path {
        write_map(mp, &merged, &objects, &entry_name, entry_va)?;
    }

    finish_elf_image(ElfImageContext {
        opts,
        output,
        merged: &merged,
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
    })
}
