use super::pe_support::*;
use super::*;

pub fn write_pe(inputs: &[PathBuf], output: &Path) -> Result<(), String> {
    write_pe_with_options(inputs, output, &LinkOptions::default())
}

pub fn write_pe_with_options(
    inputs: &[PathBuf],
    output: &Path,
    opts: &LinkOptions,
) -> Result<(), String> {
    write_pe_with_options_t(inputs, output, opts).map_err(String::from)
}

/// Typed variant of [`write_pe_with_options`] returning the engine's [`LinkError`]
/// (6A.2 boundary); the public function is its `String` adapter.
pub fn write_pe_with_options_t(
    inputs: &[PathBuf],
    output: &Path,
    opts: &LinkOptions,
) -> Result<(), LinkError> {
    let (objects, _, machine) = load_objects(inputs, opts)?;
    let mut merged = merge_objects(&objects, opts)?;
    let image_base = opts.image_base.unwrap_or(PE_IMAGE_BASE);

    let undef = collect_undefined(&objects, &merged);

    // Import set: explicit __imp_* plus known DLL symbols plus extras.
    let mut raw_imports: Vec<String> = Vec::new();
    let mut refptr_names: Vec<String> = Vec::new();
    let push_imp = |raw_imports: &mut Vec<String>, n: &str| {
        let c = n.strip_prefix("__imp_").unwrap_or(n).to_string();
        if !raw_imports.iter().any(|x| x == &c) {
            raw_imports.push(c);
        }
    };
    for o in &objects {
        for rel in &o.relocations {
            if let RelTarget::Name(t) = &rel.target {
                if let Some(name) = t.strip_prefix("__imp_") {
                    push_imp(&mut raw_imports, name);
                } else if let Some(name) = t.strip_prefix(".refptr.") {
                    if !refptr_names.iter().any(|x| x == name) {
                        refptr_names.push(name.to_string());
                    }
                } else if !merged.syms.contains_key(t)
                    && (classify_dll(t, &opts.extra_imports).is_some()
                        || opts.extra_imports.contains_key(t))
                {
                    push_imp(&mut raw_imports, t);
                }
            }
        }
    }
    for u in &undef {
        if classify_dll(u, &opts.extra_imports).is_some() || u.starts_with("__imp_") {
            push_imp(&mut raw_imports, u);
        }
    }

    let leftover: Vec<String> = undef
        .into_iter()
        .filter(|u| {
            !merged.syms.contains_key(u)
                && !raw_imports
                    .iter()
                    .any(|i| i == u || u == &format!("__imp_{i}"))
                && !u.starts_with(".refptr.")
                && u != "__ImageBase"
        })
        .collect();
    if !leftover.is_empty() && opts.dynamic == DynamicMode::Static {
        return Err(LinkError::unresolved("unresolved COFF symbols", leftover));
    }
    // Auto/Force may promote known DLL imports automatically, but must never
    // turn an arbitrary unresolved symbol into a fake KERNEL32 import.
    // Unknown symbols remain errors unless the caller supplied --import.
    let mut unknown_leftover = Vec::new();
    for u in leftover {
        if opts.dynamic == DynamicMode::Auto || opts.dynamic == DynamicMode::Force {
            if classify_dll(&u, &opts.extra_imports).is_some() {
                push_imp(&mut raw_imports, &u);
            } else {
                unknown_leftover.push(u);
            }
        } else {
            unknown_leftover.push(u);
        }
    }
    if !unknown_leftover.is_empty() {
        return Err(LinkError::unresolved(
            "unresolved COFF symbols",
            unknown_leftover,
        ));
    }

    let refptr_data_off = merged.data.len();
    merged
        .data
        .resize(refptr_data_off + refptr_names.len() * 8, 0);

    // TLS directory lives in .rdata — reserve *before* RVA assignment.
    let has_tls = !merged.tls.is_empty() || merged.tbss_size > 0;
    let tls_dir_off = if has_tls {
        let o = merged.rodata.len();
        merged.rodata.resize(o + 40, 0);
        Some(o)
    } else {
        None
    };
    let tls_index_off = if has_tls {
        let o = merged.data.len();
        merged.data.resize(o + 8, 0);
        Some(o)
    } else {
        None
    };

    if merged.bss_size > 0 {
        let aligned = align_up(merged.data.len(), 16);
        merged.data.resize(aligned, 0);
    }

    let text_rva = PE_SECTION_RVA;
    let thunks_len = raw_imports.len() * 6;
    let total_text = merged.text.len() + thunks_len;
    let rdata_rva = pe_align(text_rva as usize + total_text, PE_SECT_ALIGN) as u32;
    let data_rva = pe_align(rdata_rva as usize + merged.rodata.len(), PE_SECT_ALIGN) as u32;
    let bss_rva = data_rva + merged.data.len() as u32;
    let data_virt = merged.data.len() + merged.bss_size;
    let tls_rva = pe_align(data_rva as usize + data_virt, PE_SECT_ALIGN) as u32;
    let tls_virt = merged.tls.len() + merged.tbss_size;
    let idata_rva = pe_align(
        if has_tls {
            tls_rva as usize + tls_virt
        } else {
            data_rva as usize + data_virt
        },
        PE_SECT_ALIGN,
    ) as u32;

    let import = build_imports(&raw_imports, idata_rva, &opts.extra_imports)?;

    // 6-byte x86_64 thunks: ff 25 rel32 → IAT. ARM64 uses 12-byte veneer.
    let mut thunk_rvas: HashMap<String, u32> = HashMap::new();
    match machine {
        Machine::X86_64 => {
            for imp in &raw_imports {
                if let Some(&iat) = import.iat_rvas.get(imp) {
                    let thunk_rva = text_rva + merged.text.len() as u32;
                    let next_ip = thunk_rva + 6;
                    let disp = iat as i64 - next_ip as i64;
                    let mut t = [0xFFu8, 0x25, 0, 0, 0, 0];
                    t[2..6].copy_from_slice(&(disp as i32).to_le_bytes());
                    merged.text.extend_from_slice(&t);
                    thunk_rvas.insert(imp.clone(), thunk_rva);
                }
            }
        }
        Machine::Aarch64 => {
            for imp in &raw_imports {
                if let Some(&iat) = import.iat_rvas.get(imp) {
                    let thunk_rva = text_rva + merged.text.len() as u32;
                    let p = image_base + thunk_rva as u64;
                    let dest = image_base + iat as u64;
                    let delta = page(dest) as i64 - page(p) as i64;
                    let imm = delta >> 12;
                    let immlo = (imm as u32) & 3;
                    let immhi = ((imm as u32) >> 2) & 0x7_ffff;
                    let adrp = 0x90000010u32 | (immlo << 29) | (immhi << 5);
                    let lo = ((dest & 0xfff) >> 3) as u32;
                    let ldr = 0xf9400210u32 | (lo << 10);
                    let br = 0xd61f0200u32;
                    merged.text.extend_from_slice(&adrp.to_le_bytes());
                    merged.text.extend_from_slice(&ldr.to_le_bytes());
                    merged.text.extend_from_slice(&br.to_le_bytes());
                    thunk_rvas.insert(imp.clone(), thunk_rva);
                }
            }
        }
    }

    if let (Some(dir_off), Some(idx_off)) = (tls_dir_off, tls_index_off) {
        let start_va = image_base + tls_rva as u64;
        let end_va = start_va + merged.tls.len() as u64;
        let index_va = image_base + data_rva as u64 + idx_off as u64;
        put_u64(&mut merged.rodata, dir_off, start_va);
        put_u64(&mut merged.rodata, dir_off + 8, end_va);
        put_u64(&mut merged.rodata, dir_off + 16, index_va);
        put_u64(&mut merged.rodata, dir_off + 24, 0);
        put_u32(&mut merged.rodata, dir_off + 32, 0);
        put_u32(&mut merged.rodata, dir_off + 36, 0);
    }

    let mut refptr_rvas: HashMap<String, usize> = HashMap::new();
    for (i, name) in refptr_names.iter().enumerate() {
        let slot_rva = data_rva as usize + refptr_data_off + i * 8;
        refptr_rvas.insert(format!(".refptr.{name}"), slot_rva);
        if let Some(sy) = merged.syms.get(name) {
            let rva = pe_sym_rva(sy, text_rva, rdata_rva, data_rva, bss_rva, tls_rva);
            let addr = image_base + rva;
            merged.data[refptr_data_off + i * 8..refptr_data_off + i * 8 + 8]
                .copy_from_slice(&addr.to_le_bytes());
        }
    }

    let mut abs_rvas: Vec<u32> = refptr_rvas.values().map(|v| *v as u32).collect();
    if let Some(dir_off) = tls_dir_off {
        // IMAGE_TLS_DIRECTORY64 StartAddressOfRawData / End / AddressOfIndex are VAs
        abs_rvas.push(rdata_rva + dir_off as u32);
        abs_rvas.push(rdata_rva + dir_off as u32 + 8);
        abs_rvas.push(rdata_rva + dir_off as u32 + 16);
    }

    for (idx, obj) in objects.iter().enumerate() {
        let b = merged.place[idx];
        for rel in &obj.relocations {
            let target = resolve_pe_target(
                rel,
                &merged,
                &import.iat_rvas,
                &thunk_rvas,
                &refptr_rvas,
                &b,
                text_rva,
                rdata_rva,
                data_rva,
                bss_rva,
                tls_rva,
                idx,
            )?;
            let (patch_buf, patch_rva, section_base) = match rel.section_class {
                SectionClass::Text => (&mut merged.text, text_rva, b.text),
                SectionClass::Rodata | SectionClass::InitArray | SectionClass::FiniArray => {
                    (&mut merged.rodata, rdata_rva, b.rodata)
                }
                SectionClass::Data => (&mut merged.data, data_rva, b.data),
                SectionClass::Tls => (&mut merged.tls, tls_rva, b.tls),
                SectionClass::Bss => {
                    return Err(format!("'{}': relocation against BSS", obj.path.display()).into());
                }
            };
            let patch = section_base + rel.offset;
            let patch_rva_addr = patch_rva as i64 + patch as i64;
            let rnum = if rel.raw_type != 0 {
                rel.raw_type as u16
            } else {
                match rel.kind {
                    RelocationKind::Absolute if rel.size == 64 => AMD64_ADDR64,
                    RelocationKind::Absolute => AMD64_ADDR32,
                    RelocationKind::Relative => AMD64_REL32,
                    RelocationKind::SectionIndex => AMD64_SECTION,
                    RelocationKind::SectionOffset => AMD64_SECREL,
                    _ => {
                        if rel.size == 64 {
                            AMD64_ADDR64
                        } else {
                            AMD64_REL32
                        }
                    }
                }
            };
            let ctx = obj.path.display().to_string();
            match (machine, rnum) {
                (Machine::X86_64, AMD64_ADDR64) | (Machine::Aarch64, ARM64_ADDR64) => {
                    let abs = image_base + target;
                    write_u64_at(patch_buf, patch, abs.wrapping_add_signed(rel.addend), &ctx)?;
                    abs_rvas.push(patch_rva_addr as u32);
                }
                (Machine::X86_64, AMD64_ADDR32) | (Machine::Aarch64, ARM64_ADDR32) => {
                    write_u32_at(
                        patch_buf,
                        patch,
                        (image_base + target) as i64 + rel.addend,
                        &ctx,
                    )?;
                }
                (Machine::X86_64, AMD64_ADDR32NB) | (Machine::Aarch64, ARM64_ADDR32NB) => {
                    write_u32_at(patch_buf, patch, target as i64 + rel.addend, &ctx)?;
                }
                (Machine::X86_64, AMD64_REL32)
                | (Machine::X86_64, AMD64_REL32_1)
                | (Machine::X86_64, AMD64_REL32_2)
                | (Machine::X86_64, AMD64_REL32_3)
                | (Machine::X86_64, AMD64_REL32_4)
                | (Machine::X86_64, AMD64_REL32_5)
                | (Machine::Aarch64, ARM64_REL32) => {
                    let adj: i64 = match rnum {
                        AMD64_REL32_1 => 1,
                        AMD64_REL32_2 => 2,
                        AMD64_REL32_3 => 3,
                        AMD64_REL32_4 => 4,
                        AMD64_REL32_5 => 5,
                        _ => 0,
                    };
                    // S + A - (P + 4 + adj)   — PE/COFF 5.6 + implicit addend A
                    let disp = target as i64 + rel.addend - (patch_rva_addr + 4 + adj);
                    write_i32_at(patch_buf, patch, disp, &ctx)?;
                }
                (Machine::X86_64, AMD64_SECTION) | (Machine::Aarch64, ARM64_SECTION) => {}
                (Machine::X86_64, AMD64_SECREL) | (Machine::Aarch64, ARM64_SECREL) => {
                    let secrel = if target < rdata_rva as u64 {
                        target - text_rva as u64
                    } else if target < data_rva as u64 {
                        target - rdata_rva as u64
                    } else if target < idata_rva as u64 {
                        target - data_rva as u64
                    } else {
                        target
                    };
                    write_u32_at(patch_buf, patch, secrel as i64 + rel.addend, &ctx)?;
                }
                (Machine::Aarch64, ARM64_BRANCH26) => {
                    let p = image_base + patch_rva_addr as u64;
                    let s = image_base + target;
                    a64_patch_call26(patch_buf, patch, s, rel.addend, p)?;
                }
                (Machine::Aarch64, ARM64_PAGEBASE_REL21) => {
                    let p = image_base + patch_rva_addr as u64;
                    let s = image_base + target;
                    a64_patch_adr_pg_hi21(patch_buf, patch, s, rel.addend, p)?;
                }
                (Machine::Aarch64, ARM64_PAGEOFFSET_12A) => {
                    a64_patch_add_lo12(patch_buf, patch, image_base + target, rel.addend)?;
                }
                (Machine::Aarch64, ARM64_PAGEOFFSET_12L) => {
                    // shift inferred from instruction size field — default 3 (64-bit)
                    let instr = a64_read(patch_buf, patch)?;
                    let size = (instr >> 30) & 3;
                    a64_patch_ldst_lo12(patch_buf, patch, image_base + target, rel.addend, size)?;
                }
                (Machine::Aarch64, ARM64_BRANCH19) => {
                    let p = image_base + patch_rva_addr as u64;
                    a64_patch_condbr19(patch_buf, patch, image_base + target, rel.addend, p)?;
                }
                (Machine::Aarch64, ARM64_BRANCH14) => {
                    let p = image_base + patch_rva_addr as u64;
                    a64_patch_tstbr14(patch_buf, patch, image_base + target, rel.addend, p)?;
                }
                (Machine::Aarch64, ARM64_REL21) => {
                    let p = image_base + patch_rva_addr as u64;
                    a64_patch_adr_lo21(patch_buf, patch, image_base + target, rel.addend, p)?;
                }
                _ => {
                    return Err(format!(
                        "'{ctx}': unsupported COFF relocation type {rnum} ({machine:?})"
                    )
                    .into());
                }
            }
        }
    }

    let reloc_data = generate_base_relocs_from_rvas(&abs_rvas);
    let has_reloc = !reloc_data.is_empty();
    let reloc_rva = if has_reloc {
        pe_align(idata_rva as usize + import.data.len(), PE_SECT_ALIGN) as u32
    } else {
        0
    };

    let has_text = !merged.text.is_empty();
    let has_rdata = !merged.rodata.is_empty();
    let has_data = !merged.data.is_empty() || merged.bss_size > 0;
    let has_idata = !import.data.is_empty();

    let text_raw_size = pe_align(merged.text.len(), PE_FILE_ALIGN);
    let rdata_raw_size = pe_align(merged.rodata.len(), PE_FILE_ALIGN);
    let data_raw_size = pe_align(merged.data.len(), PE_FILE_ALIGN);
    let tls_raw_size = if has_tls {
        pe_align(merged.tls.len(), PE_FILE_ALIGN)
    } else {
        0
    };
    let idata_raw_size = if has_idata {
        pe_align(import.data.len(), PE_FILE_ALIGN)
    } else {
        0
    };
    let reloc_raw_size = if has_reloc {
        pe_align(reloc_data.len(), PE_FILE_ALIGN)
    } else {
        0
    };

    let mut section_count: u16 = 0;
    if has_text {
        section_count += 1;
    }
    if has_rdata {
        section_count += 1;
    }
    if has_data {
        section_count += 1;
    }
    if has_tls {
        section_count += 1;
    }
    if has_idata {
        section_count += 1;
    }
    if has_reloc {
        section_count += 1;
    }

    let nt = 0x80;
    let opt = nt + 24;
    let opt_size: u16 = 0xF0;
    let required = opt + opt_size as usize + section_count as usize * 40;
    let headers_size = pe_align(required, PE_FILE_ALIGN);

    let text_raw_off = headers_size;
    let rdata_raw_off = text_raw_off + text_raw_size;
    let data_raw_off = rdata_raw_off + rdata_raw_size;
    let tls_raw_off = data_raw_off + data_raw_size;
    let idata_raw_off = tls_raw_off + tls_raw_size;
    let reloc_raw_off = idata_raw_off + idata_raw_size;

    let image_end = if has_reloc {
        reloc_rva as usize + reloc_data.len()
    } else if has_idata {
        idata_rva as usize + import.data.len()
    } else if has_tls {
        tls_rva as usize + tls_virt
    } else if has_data {
        data_rva as usize + merged.data.len() + merged.bss_size
    } else if has_rdata {
        rdata_rva as usize + merged.rodata.len()
    } else {
        text_rva as usize + merged.text.len()
    };
    let image_size = pe_align(image_end, PE_SECT_ALIGN);
    let file_size = reloc_raw_off + reloc_raw_size;
    if file_size > 512 * 1024 * 1024 {
        return Err("PE image exceeds 512 MiB safety limit".into());
    }

    let mut pe = vec![0u8; file_size.max(headers_size)];
    pe[0..2].copy_from_slice(b"MZ");
    put_u32(&mut pe, 0x3c, 0x80);
    // Minimal DOS stub message
    let stub = b"This program cannot be run in DOS mode.\r\r\n$";
    if 0x40 + stub.len() < 0x80 {
        pe[0x40..0x40 + stub.len()].copy_from_slice(stub);
    }
    pe[nt..nt + 4].copy_from_slice(b"PE\0\0");
    put_u16(&mut pe, nt + 4, machine.pe_machine());
    put_u16(&mut pe, nt + 6, section_count);
    // TimeDateStamp: 0 for reproducible builds (honour SOURCE_DATE_EPOCH if set)
    if let Ok(v) = env::var("SOURCE_DATE_EPOCH") {
        if let Ok(ts) = v.parse::<u32>() {
            put_u32(&mut pe, nt + 8, ts);
        }
    }
    put_u16(&mut pe, nt + 20, opt_size);
    let mut chars: u16 = 0x0002 | 0x0020; // EXECUTABLE_IMAGE | LARGE_ADDRESS_AWARE
    if opts.shared {
        chars |= 0x2000; // IMAGE_FILE_DLL
    }
    if opts.strip {
        chars |= 0x0100;
    }
    put_u16(&mut pe, nt + 22, chars);

    put_u16(&mut pe, opt, 0x20b);
    pe[opt + 2] = 2; // MajorLinkerVersion
    pe[opt + 3] = 3; // MinorLinkerVersion
    put_u32(&mut pe, opt + 4, text_raw_size as u32);
    put_u32(
        &mut pe,
        opt + 8,
        (rdata_raw_size + data_raw_size + tls_raw_size + idata_raw_size) as u32,
    );
    put_u32(&mut pe, opt + 12, merged.bss_size as u32); // SizeOfUninitializedData

    let entry_candidates = [
        "mainCRTStartup",
        "WinMainCRTStartup",
        "DllMain",
        "main",
        "_main",
        "WinMain",
        "wWinMain",
        "lpp_main",
        "_start",
    ];
    let entry_sym = if let Some(e) = &opts.entry {
        merged
            .syms
            .get(e)
            .ok_or_else(|| format!("entry symbol '{e}' not defined"))?
    } else {
        entry_candidates
            .iter()
            .find_map(|&n| merged.syms.get(n))
            .ok_or_else(|| {
                "required entry symbol ('mainCRTStartup', 'main', 'WinMain', or 'lpp_main') not found"
                    .to_string()
            })?
    };
    let entry_rva = pe_sym_rva(entry_sym, text_rva, rdata_rva, data_rva, bss_rva, tls_rva) as u32;
    put_u32(&mut pe, opt + 16, entry_rva);
    put_u32(&mut pe, opt + 20, text_rva);
    put_u64(&mut pe, opt + 24, image_base);
    put_u32(&mut pe, opt + 32, PE_SECT_ALIGN as u32);
    put_u32(&mut pe, opt + 36, PE_FILE_ALIGN as u32);
    put_u16(&mut pe, opt + 40, 6);
    put_u16(&mut pe, opt + 42, 0);
    put_u16(&mut pe, opt + 44, 0);
    put_u16(&mut pe, opt + 46, 0);
    put_u16(&mut pe, opt + 48, 6);
    put_u16(&mut pe, opt + 50, 0);
    put_u32(&mut pe, opt + 56, image_size as u32);
    put_u32(&mut pe, opt + 60, headers_size as u32);
    // checksum filled later
    let subsystem = opts.subsystem.unwrap_or_else(|| {
        if merged.syms.contains_key("WinMain")
            || merged.syms.contains_key("wWinMain")
            || merged.syms.contains_key("WinMainCRTStartup")
        {
            PeSubsystem::Windows
        } else if matches!(opts.subsystem, None) && opts.shared {
            PeSubsystem::Windows
        } else {
            PeSubsystem::Console
        }
    });
    let sub_val = match subsystem {
        PeSubsystem::Native => 1u16,
        PeSubsystem::Windows => 2,
        PeSubsystem::Console => 3,
    };
    put_u16(&mut pe, opt + 68, sub_val);
    // HIGH_ENTROPY_VA | DYNAMIC_BASE | NX_COMPAT | TERMINAL_SERVER_AWARE
    let mut dll_chars: u16 = 0x8160;
    if !has_reloc {
        dll_chars &= !0x0040; // drop DYNAMIC_BASE if no fixups
        dll_chars &= !0x0020; // drop HIGH_ENTROPY_VA
    }
    put_u16(&mut pe, opt + 70, dll_chars);
    put_u64(&mut pe, opt + 72, opts.stack_reserve);
    put_u64(&mut pe, opt + 80, opts.stack_commit);
    put_u64(&mut pe, opt + 88, opts.heap_reserve);
    put_u64(&mut pe, opt + 96, opts.heap_commit);
    put_u32(&mut pe, opt + 108, 16);

    let dirs = opt + 112;
    if has_idata {
        put_u32(&mut pe, dirs + 8, idata_rva);
        put_u32(&mut pe, dirs + 12, import.data.len() as u32);
        put_u32(&mut pe, dirs + 12 * 8, import.iat_rva);
        put_u32(&mut pe, dirs + 12 * 8 + 4, import.iat_size);
    }
    if let Some((pdata_off, pdata_len)) = merged.pdata_info() {
        for &(start, len) in &merged.pdata_ranges {
            if len >= 12 && start + len <= merged.rodata.len() {
                let slice = &mut merged.rodata[start..start + len];
                let mut entries: Vec<[u8; 12]> = slice
                    .chunks_exact(12)
                    .map(|c| c.try_into().unwrap())
                    .collect();
                entries.sort_by_key(|e| u32::from_le_bytes(e[0..4].try_into().unwrap()));
                for (i, entry) in entries.iter().enumerate() {
                    slice[i * 12..(i + 1) * 12].copy_from_slice(entry);
                }
            }
        }
        put_u32(&mut pe, dirs + 3 * 8, rdata_rva + pdata_off as u32);
        put_u32(&mut pe, dirs + 3 * 8 + 4, pdata_len as u32);
    }
    if has_reloc {
        put_u32(&mut pe, dirs + 5 * 8, reloc_rva);
        put_u32(&mut pe, dirs + 5 * 8 + 4, reloc_data.len() as u32);
    }
    if let Some(dir_off) = tls_dir_off {
        put_u32(&mut pe, dirs + 9 * 8, rdata_rva + dir_off as u32);
        put_u32(&mut pe, dirs + 9 * 8 + 4, 40);
    }

    let mut sec = opt + opt_size as usize;
    let emit_section = |pe: &mut [u8],
                        sec: &mut usize,
                        name: &[u8; 8],
                        rva: u32,
                        raw_size: usize,
                        raw_off: usize,
                        virt_size: usize,
                        characteristics: u32| {
        pe[*sec..*sec + 8].copy_from_slice(name);
        put_u32(pe, *sec + 8, virt_size as u32);
        put_u32(pe, *sec + 12, rva);
        put_u32(pe, *sec + 16, raw_size as u32);
        put_u32(pe, *sec + 20, raw_off as u32);
        put_u32(pe, *sec + 36, characteristics);
        *sec += 40;
    };
    if has_text {
        let mut n = [0u8; 8];
        n[..5].copy_from_slice(b".text");
        emit_section(
            &mut pe,
            &mut sec,
            &n,
            text_rva,
            text_raw_size,
            text_raw_off,
            merged.text.len(),
            0x60000020,
        );
    }
    if has_rdata {
        let mut n = [0u8; 8];
        n[..6].copy_from_slice(b".rdata");
        emit_section(
            &mut pe,
            &mut sec,
            &n,
            rdata_rva,
            rdata_raw_size,
            rdata_raw_off,
            merged.rodata.len(),
            0x40000040,
        );
    }
    if has_data {
        let mut n = [0u8; 8];
        n[..5].copy_from_slice(b".data");
        emit_section(
            &mut pe,
            &mut sec,
            &n,
            data_rva,
            data_raw_size,
            data_raw_off,
            merged.data.len() + merged.bss_size,
            0xC0000040,
        );
    }
    if has_tls {
        let mut n = [0u8; 8];
        n[..4].copy_from_slice(b".tls");
        emit_section(
            &mut pe,
            &mut sec,
            &n,
            tls_rva,
            tls_raw_size,
            tls_raw_off,
            tls_virt,
            0xC0000040,
        );
    }
    if has_idata {
        let mut n = [0u8; 8];
        n[..6].copy_from_slice(b".idata");
        emit_section(
            &mut pe,
            &mut sec,
            &n,
            idata_rva,
            idata_raw_size,
            idata_raw_off,
            import.data.len(),
            0xC0000040,
        );
    }
    if has_reloc {
        let mut n = [0u8; 8];
        n[..6].copy_from_slice(b".reloc");
        emit_section(
            &mut pe,
            &mut sec,
            &n,
            reloc_rva,
            reloc_raw_size,
            reloc_raw_off,
            reloc_data.len(),
            0x42000040,
        );
    }

    if has_text {
        pe[text_raw_off..text_raw_off + merged.text.len()].copy_from_slice(&merged.text);
    }
    if has_rdata {
        pe[rdata_raw_off..rdata_raw_off + merged.rodata.len()].copy_from_slice(&merged.rodata);
    }
    if has_data && !merged.data.is_empty() {
        pe[data_raw_off..data_raw_off + merged.data.len()].copy_from_slice(&merged.data);
    }
    if has_tls && !merged.tls.is_empty() {
        pe[tls_raw_off..tls_raw_off + merged.tls.len()].copy_from_slice(&merged.tls);
    }
    if has_idata {
        pe[idata_raw_off..idata_raw_off + import.data.len()].copy_from_slice(&import.data);
    }
    if has_reloc {
        pe[reloc_raw_off..reloc_raw_off + reloc_data.len()].copy_from_slice(&reloc_data);
    }

    let sum = pe_checksum(&pe, opt + 64);
    put_u32(&mut pe, opt + 64, sum);

    if let Some(mp) = &opts.map_path {
        write_map(
            mp,
            &merged,
            &objects,
            &entry_sym.name,
            image_base + entry_rva as u64,
        )?;
    }

    fs::write(output, pe).map_err(|e| format!("write '{}': {e}", output.display()))?;
    vlog(
        opts,
        format!(
            "PE {:?} {}  entry_rva=0x{entry_rva:x}  checksum=0x{sum:08x}",
            machine,
            output.display()
        ),
    );
    Ok(())
}

pub(super) fn pe_sym_rva(
    sy: &ResolvedSym,
    text: u32,
    rdata: u32,
    data: u32,
    bss: u32,
    tls: u32,
) -> u64 {
    let base = match sy.class {
        SectionClass::Text => text,
        SectionClass::Rodata | SectionClass::InitArray | SectionClass::FiniArray => rdata,
        SectionClass::Data => data,
        SectionClass::Bss => bss,
        SectionClass::Tls => tls,
    };
    base as u64 + sy.offset
}

#[allow(clippy::too_many_arguments)]
fn resolve_pe_target(
    rel: &Relocation,
    merged: &Merged,
    iat_rvas: &HashMap<String, u32>,
    thunk_rvas: &HashMap<String, u32>,
    refptr_offsets: &HashMap<String, usize>,
    bases: &Placement,
    text_rva: u32,
    rdata_rva: u32,
    data_rva: u32,
    bss_rva: u32,
    tls_rva: u32,
    _obj_idx: usize,
) -> Result<u64, LinkError> {
    match &rel.target {
        RelTarget::Local(class, off) => {
            let (rva, b) = match class {
                SectionClass::Text => (text_rva, bases.text),
                SectionClass::Rodata | SectionClass::InitArray | SectionClass::FiniArray => {
                    (rdata_rva, bases.rodata)
                }
                SectionClass::Data => (data_rva, bases.data),
                SectionClass::Bss => (bss_rva, bases.bss),
                SectionClass::Tls => (tls_rva, bases.tls),
            };
            Ok(rva as u64 + b as u64 + *off)
        }
        RelTarget::Name(name) => {
            if name == "__ImageBase" {
                return Ok(0);
            }
            if let Some(sy) = merged.syms.get(name) {
                return Ok(pe_sym_rva(
                    sy, text_rva, rdata_rva, data_rva, bss_rva, tls_rva,
                ));
            }
            if let Some(&rva) = refptr_offsets.get(name) {
                return Ok(rva as u64);
            }
            if let Some(inner) = name.strip_prefix(".refptr.") {
                if let Some(sy) = merged.syms.get(inner) {
                    return Ok(pe_sym_rva(
                        sy, text_rva, rdata_rva, data_rva, bss_rva, tls_rva,
                    ));
                }
            }
            if !name.starts_with("__imp_") {
                if let Some(&trva) = thunk_rvas.get(name) {
                    return Ok(trva as u64);
                }
                let stripped = name.strip_prefix("__imp_").unwrap_or(name);
                if let Some(&trva) = thunk_rvas.get(stripped) {
                    return Ok(trva as u64);
                }
            }
            if let Some(&rva) = iat_rvas.get(name) {
                return Ok(rva as u64);
            }
            let stripped = name.strip_prefix("__imp_").unwrap_or(name);
            if let Some(&rva) = iat_rvas.get(stripped) {
                return Ok(rva as u64);
            }
            Err(format!(
                "unresolved external COFF symbol '{name}' — not defined by any input object \
                 and not a known DLL import. Pass --import DLL=sym or use --dynamic."
            )
            .into())
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Mach-O
// ═══════════════════════════════════════════════════════════════════════════
