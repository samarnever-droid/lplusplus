use super::*;

fn macho_nlist(strx: u32, n_type: u8, n_sect: u8, desc: u16, value: u64) -> [u8; 16] {
    let mut e = [0u8; 16];
    put_u32(&mut e, 0, strx);
    e[4] = n_type;
    e[5] = n_sect;
    put_u16(&mut e, 6, desc);
    put_u64(&mut e, 8, value);
    e
}

fn uleb(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let mut b = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            b |= 0x80;
        }
        out.push(b);
        if v == 0 {
            break;
        }
    }
}

fn macho_adhoc_codesig(code: &[u8], ident: &str, page_size: usize, exec_limit: u64) -> Vec<u8> {
    // SuperBlob { CD } — ad-hoc, no CMS blob. Required on Apple Silicon.
    let ident_bytes = ident.as_bytes();
    let n_pages = if code.is_empty() {
        0
    } else {
        code.len().div_ceil(page_size)
    };
    let version = 0x0002_0400u32;
    let cd_fixed = 88usize; // CodeDirectory up through execSegFlags
    let ident_off = cd_fixed;
    let hash_off = align_up(ident_off + ident_bytes.len() + 1, 4);
    let cd_len = hash_off + n_pages * 32;

    let mut cd = vec![0u8; cd_len];
    put_u32be(&mut cd, 0, 0xfade_0c02); // CSMAGIC_CODEDIRECTORY
    put_u32be(&mut cd, 4, cd_len as u32);
    put_u32be(&mut cd, 8, version);
    put_u32be(&mut cd, 12, 0x2); // CS_ADHOC
    put_u32be(&mut cd, 16, hash_off as u32);
    put_u32be(&mut cd, 20, ident_off as u32);
    put_u32be(&mut cd, 24, 0); // nSpecialSlots
    put_u32be(&mut cd, 28, n_pages as u32);
    put_u32be(&mut cd, 32, code.len() as u32); // codeLimit
    cd[36] = 32; // hashSize
    cd[37] = 2; // SHA256
    cd[38] = 0;
    cd[39] = page_size.trailing_zeros() as u8;
    // scatter/team/spare/codeLimit64 already 0
    put_u64(&mut cd, 64, 0); // execSegBase — but fields are big-endian from 0..44; 0x20400 extras:
    // execSegBase at 64, execSegLimit 72, execSegFlags 80 — Apple uses native? Actually BE throughout CD.
    // execSeg* are 64-bit BE.
    for (i, b) in 0u64.to_be_bytes().iter().enumerate() {
        cd[64 + i] = *b;
    }
    for (i, b) in exec_limit.to_be_bytes().iter().enumerate() {
        cd[72 + i] = *b;
    }
    let flags = 1u64; // CS_EXECSEG_MAIN_BINARY
    for (i, b) in flags.to_be_bytes().iter().enumerate() {
        cd[80 + i] = *b;
    }
    cd[ident_off..ident_off + ident_bytes.len()].copy_from_slice(ident_bytes);
    for i in 0..n_pages {
        let start = i * page_size;
        let end = (start + page_size).min(code.len());
        let digest = sha256(&code[start..end]);
        cd[hash_off + i * 32..hash_off + i * 32 + 32].copy_from_slice(&digest);
    }

    let count = 1u32;
    let super_hdr = 12u32;
    let index_size = 8u32;
    let cd_offset = super_hdr + index_size;
    let total = cd_offset as usize + cd.len();
    let mut blob = vec![0u8; total];
    put_u32be(&mut blob, 0, 0xfade_0cc0); // CSMAGIC_EMBEDDED_SIGNATURE
    put_u32be(&mut blob, 4, total as u32);
    put_u32be(&mut blob, 8, count);
    put_u32be(&mut blob, 12, 0); // CSSLOT_CODEDIRECTORY
    put_u32be(&mut blob, 16, cd_offset);
    blob[cd_offset as usize..].copy_from_slice(&cd);
    blob
}

pub fn write_macho(inputs: &[PathBuf], output: &Path) -> Result<(), String> {
    write_macho_with_options(inputs, output, &LinkOptions::default())
}

pub fn write_macho_with_options(
    inputs: &[PathBuf],
    output: &Path,
    opts: &LinkOptions,
) -> Result<(), String> {
    write_macho_with_options_t(inputs, output, opts).map_err(String::from)
}

/// Typed variant of [`write_macho_with_options`] returning the engine's [`LinkError`]
/// (6A.2 boundary); the public function is its `String` adapter.
pub fn write_macho_with_options_t(
    inputs: &[PathBuf],
    output: &Path,
    opts: &LinkOptions,
) -> Result<(), LinkError> {
    let (objects, _, machine) = load_objects(inputs, opts)?;
    let mut merged = merge_objects(&objects, opts)?;
    let page_size = opts
        .page_size
        .unwrap_or(machine.page_size(OutputFormat::Macho));
    let _page_u = page_size as u64;

    let undef = collect_undefined(&objects, &merged);
    let mut imports: Vec<String> = undef
        .iter()
        .filter(|u| !merged.syms.contains_key(*u))
        .cloned()
        .collect();
    imports.sort();
    imports.dedup();

    // macOS rejects static ARM64 executables outright — the kernel requires at least
    // libSystem to be loaded. Emit a clear policy error instead of a broken binary.
    if !opts.shared
        && machine == Machine::Aarch64
        && opts.libraries.is_empty()
        && opts.needed.is_empty()
    {
        return Err(
            "lpp-link: dynamic libSystem imports are required for ARM64 Mach-O \
 executables; pass -l System or link dynamically (macOS policy)"
                .into(),
        );
    }

    if opts.dynamic == DynamicMode::Static && !imports.is_empty() {
        return Err(LinkError::unresolved(
            "unresolved Mach-O symbols (static)",
            imports,
        ));
    }

    // GOT slots for imports + GOT-using relocs (including defined / local PIC)
    let mut got_keys: BTreeMap<String, usize> = BTreeMap::new();
    for n in &imports {
        let i = got_keys.len();
        got_keys.entry(n.clone()).or_insert(i);
    }
    for (oi, o) in objects.iter().enumerate() {
        for rel in &o.relocations {
            let macho_got = matches!(rel.raw_type, 3..=7);
            if macho_got {
                let key = got_key(oi, &rel.target);
                let i = got_keys.len();
                got_keys.entry(key).or_insert(i);
            }
        }
    }

    // Stubs: x86_64 jmp [rip+got] (6), aarch64 adrp+ldr+br (12)
    let stub_size = match machine {
        Machine::X86_64 => 6usize,
        Machine::Aarch64 => 12usize,
    };
    let mut stub_index: BTreeMap<String, usize> = BTreeMap::new();
    for n in got_keys.keys() {
        if !merged.syms.contains_key(n) {
            let i = stub_index.len();
            stub_index.entry(n.clone()).or_insert(i);
        }
    }
    let stubs_len = stub_index.len() * stub_size;
    let stubs_off = align_up(merged.text.len(), 16);
    merged.text.resize(stubs_off + stubs_len, 0x90);

    let got_bytes = got_keys.len() * 8;
    let data_off0 = merged.data.len();
    let got_in_data = align_up(data_off0, 8);
    if got_in_data + got_bytes > merged.data.len() {
        merged.data.resize(got_in_data + got_bytes, 0);
    }

    let has_start = merged.syms.contains_key("_start") || merged.syms.contains_key("start");
    let has_main = merged.syms.contains_key("main") || merged.syms.contains_key("lpp_main");
    if !opts.shared && !opts.no_startup && !has_start && !has_main && opts.entry.is_none() {
        return Err("required symbol 'main' or '_start' not found".into());
    }

    // Header + load commands size is computed, then we page-align __text.
    // We first estimate ncmds / sizeofcmds, then lock layout.

    let ident = output
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("a.out")
        .to_string();

    // Build symbol table (underscore prefix).
    let mut strtab = vec![0u8];
    let mut nlists: Vec<[u8; 16]> = Vec::new();
    nlists.push(macho_nlist(0, 0, 0, 0, 0));
    let mut names: Vec<ResolvedSym> = merged.syms.values().cloned().collect();
    names.sort_by(|a, b| a.name.cmp(&b.name));
    for sy in &names {
        let so = strtab.len() as u32;
        strtab.push(b'_');
        strtab.extend_from_slice(sy.name.as_bytes());
        strtab.push(0);
        // n_type = N_SECT | N_EXT
        let n_sect = match sy.class {
            SectionClass::Text => 1u8,
            _ => 2u8,
        };
        // value filled after we know VAs — placeholder
        nlists.push(macho_nlist(so, 0x0f, n_sect, 0, sy.offset));
    }
    // undefined imports
    let mut import_ord: Vec<String> = stub_index.keys().cloned().collect();
    import_ord.sort();
    for n in &import_ord {
        let so = strtab.len() as u32;
        strtab.push(b'_');
        strtab.extend_from_slice(n.as_bytes());
        strtab.push(0);
        // N_UNDF | N_EXT
        nlists.push(macho_nlist(so, 0x01, 0, 0x0100, 0)); // n_desc = REFERENCE_FLAG_UNDEFINED_LAZY
    }
    while strtab.len() % 8 != 0 {
        strtab.push(0);
    }

    let mut dylibs: Vec<String> = vec!["/usr/lib/libSystem.B.dylib".to_string()];
    for lib in &opts.libraries {
        let path = if lib.starts_with('/') || lib.contains('/') {
            lib.clone()
        } else if lib.ends_with(".dylib") {
            format!("/usr/lib/lib{lib}")
        } else if lib.starts_with("framework ") || lib.starts_with("-framework ") {
            let fw = lib.split_whitespace().last().unwrap_or(lib);
            format!("/System/Library/Frameworks/{fw}.framework/{fw}")
        } else {
            format!("/usr/lib/lib{lib}.dylib")
        };
        if !dylibs.contains(&path) {
            dylibs.push(path);
        }
    }
    for lib in &opts.needed {
        if !dylibs.contains(lib) {
            dylibs.push(lib.clone());
        }
    }
    for lib in opts.extra_imports.values() {
        let path = if lib.starts_with('/') || lib.contains('/') {
            lib.clone()
        } else if lib.ends_with(".dylib") {
            format!("/usr/lib/lib{lib}")
        } else {
            format!("/usr/lib/lib{lib}.dylib")
        };
        if !dylibs.contains(&path) {
            dylibs.push(path);
        }
    }

    // Build symbol → dylib ordinal map by scanning each resolved dylib that actually exists.
    // dylibs[0] = libSystem (ordinal 1), dylibs[1] = first user lib (ordinal 2), etc.
    // For each symbol we need to bind, we look up which dylib exports it so the bind opcodes
    // point to the right library.
    let mut sym_to_dylib_ord: HashMap<String, usize> = HashMap::new();
    for (dylib_idx, dylib_path) in dylibs.iter().enumerate() {
        let ord = dylib_idx + 1; // dyld ordinals are 1-based
        if let Ok(bytes) = fs::read(dylib_path)
            && let Ok(file) = object::File::parse(&*bytes)
        {
            for sym in file.exports().unwrap_or_default() {
                let raw = String::from_utf8_lossy(sym.name()).into_owned();
                // strip leading underscore (Mach-O convention) for internal lookup
                let clean = raw.strip_prefix('_').unwrap_or(&raw).to_string();
                sym_to_dylib_ord.entry(clean).or_insert(ord);
                sym_to_dylib_ord.entry(raw).or_insert(ord);
            }
        }
    }

    // dyld bind info for GOT slots
    // Segment ordinals in bind opcodes:
    //   executable (has __PAGEZERO): 0=__PAGEZERO 1=__TEXT 2=__DATA 3=__LINKEDIT  → data_seg_ord=2
    //   dylib      (no __PAGEZERO):  0=__TEXT      1=__DATA 2=__LINKEDIT           → data_seg_ord=1
    let is_dylib = opts.shared;
    let data_seg_ord: u8 = if is_dylib { 1 } else { 2 };

    let mut bind = Vec::new();
    if !got_keys.is_empty() {
        let mut ordered: Vec<(String, usize)> =
            got_keys.iter().map(|(k, v)| (k.clone(), *v)).collect();
        ordered.sort_by_key(|(_, i)| *i);
        for (name, idx) in &ordered {
            if merged.syms.contains_key(name) {
                continue;
            }
            let clean = name.strip_prefix('_').unwrap_or(name);
            // Resolution order:
            // 1. Scanned dylib exports (sym_to_dylib_ord built above from actual .dylib files)
            // 2. Explicit --import DLL=sym mapping (extra_imports)
            // 3. Fall back to ordinal 1 (libSystem) — shouldn't happen in practice
            let raw_ord = if let Some(&ord) = sym_to_dylib_ord
                .get(name)
                .or_else(|| sym_to_dylib_ord.get(clean))
            {
                ord
            } else {
                let target_dylib = opts
                    .extra_imports
                    .get(clean)
                    .or_else(|| opts.extra_imports.get(name));
                if let Some(td) = target_dylib {
                    dylibs
                        .iter()
                        .position(|d| d.contains(td.as_str()))
                        .map(|p| p + 1)
                        .unwrap_or(1)
                } else {
                    1usize // libSystem fallback
                }
            };
            // Encode ordinal: use IMM if ≤ 15, else ULEB form
            if raw_ord <= 15 {
                bind.push(0x10 | (raw_ord as u8 & 0x0f)); // SET_DYLIB_ORDINAL_IMM
            } else {
                bind.push(0x20); // SET_DYLIB_ORDINAL_ULEB
                uleb(&mut bind, raw_ord as u64);
            }
            bind.push(0x50 | 1); // SET_TYPE_IMM POINTER
            // segment + offset: offset from start of __DATA segment for this GOT slot
            let got_slot_off = got_in_data as u64 + (*idx as u64) * 8;
            bind.push(0x70 | data_seg_ord); // SET_SEGMENT_AND_OFFSET_ULEB
            uleb(&mut bind, got_slot_off);
            bind.push(0x40); // SET_SYMBOL_TRAILING_FLAGS_IMM 0
            // symbol name: stripped of leading underscore internally, needs _ prefix in bind
            let bind_name = if name.starts_with('_') {
                name.as_str()
            } else {
                // We'll prepend _ below
                name.as_str()
            };
            bind.push(b'_');
            bind.extend_from_slice(bind_name.trim_start_matches('_').as_bytes());
            bind.push(0);
            bind.push(0x90); // DO_BIND
        }
        bind.push(0x00); // DONE
    }
    let rebase = vec![0u8]; // DONE only — no interior pointers

    // Layout constants
    let dylinker = "/usr/lib/dyld";
    let dylinker_cmdsize = align_up(12 + dylinker.len() + 1, 8) as u32;
    let mut total_dylib_cmdsize = 0u32;
    for d in &dylibs {
        total_dylib_cmdsize += align_up(24 + d.len() + 1, 8) as u32;
    }

    // ncmds: count each LC that is actually emitted
    // Common to both: TEXT, DATA, LINKEDIT, SYMTAB, DYSYMTAB, UUID, BUILD_VERSION, SOURCE_VERSION, DYLD_INFO_ONLY, CODE_SIGNATURE = 10
    // + N × LC_LOAD_DYLIB
    // executable adds: PAGEZERO + LOAD_DYLINKER + MAIN = +3  → total 13 + N
    // dylib adds: ID_DYLIB                                = +1  → total 11 + N
    let ncmds: u32 = if is_dylib {
        11 + dylibs.len() as u32
    } else {
        13 + dylibs.len() as u32
    };
    let sizeofcmds: u32 =
        // Segments (always)
        (72 + 80)  // __TEXT + __text section
        + (72 + 80) // __DATA + __data section
        + 72        // __LINKEDIT
        // Segment-only for executables
        + (if is_dylib { 0 } else { 72 }) // __PAGEZERO
        // Symbol/bind tables (always)
        + 24        // SYMTAB
        + 80        // DYSYMTAB
        + 48        // DYLD_INFO_ONLY
        + 16        // CODE_SIGNATURE
        // Identity (mutually exclusive)
        + (if is_dylib {
            align_up(24 + ident.len() + 1, 8) as u32 // LC_ID_DYLIB
        } else {
            dylinker_cmdsize   // LOAD_DYLINKER
            + 24               // LC_MAIN
        })
        // Common metadata (always)
        + 24        // UUID
        + 32        // BUILD_VERSION
        + 16        // SOURCE_VERSION
        // Loaded dylibs (always)
        + total_dylib_cmdsize;

    let header_and_cmds = 32 + sizeofcmds as usize;
    let text_fileoff = align_up(header_and_cmds, page_size);
    let text_size = merged.text.len();
    let text_seg_filesz = align_up(text_fileoff + text_size, page_size);
    let text_seg_vmsize = text_seg_filesz as u64; // fileoff 0 .. text_seg_filesz

    let vm_base = 0x1_0000_0000u64;
    let text_vmaddr = vm_base;
    let text_sec_addr = vm_base + text_fileoff as u64;

    let data_fileoff = text_seg_filesz;
    let data_size = merged.data.len();
    let data_seg_filesz = align_up(data_size.max(1), page_size);
    let data_vmaddr = vm_base + text_seg_vmsize;
    let data_sec_addr = data_vmaddr;

    // LINKEDIT: rebase, bind, symtab, strtab, codesig
    let linkedit_fileoff = data_fileoff + data_seg_filesz;
    let mut le = Vec::new();
    let rebase_off = 0usize;
    le.extend_from_slice(&rebase);
    while le.len() % 8 != 0 {
        le.push(0);
    }
    let bind_off = le.len();
    le.extend_from_slice(&bind);
    while le.len() % 8 != 0 {
        le.push(0);
    }
    let symoff_in_le = le.len();
    for n in &nlists {
        le.extend_from_slice(n);
    }
    let stroff_in_le = le.len();
    le.extend_from_slice(&strtab);
    while le.len() % 16 != 0 {
        le.push(0);
    }

    // Fix nlist values now that VAs are known — rewrite the nlist region.
    // Defined symbols: value = section VA + offset
    // We stored offset in n_value. Rebuild.
    let mut rebuilt = Vec::new();
    rebuilt.extend_from_slice(&macho_nlist(0, 0, 0, 0, 0));
    for sy in &names {
        let so = {
            // find existing strx from previous nlist — easier to recompute
            0u32
        };
        let _ = so;
        let mut strx = 1u32;
        // scan strtab for _name
        let needle = {
            let mut n = Vec::new();
            n.push(b'_');
            n.extend_from_slice(sy.name.as_bytes());
            n.push(0);
            n
        };
        if let Some(p) = strtab
            .windows(needle.len())
            .position(|w| w == needle.as_slice())
        {
            strx = p as u32;
        }
        let va = match sy.class {
            SectionClass::Text => text_sec_addr + sy.offset,
            _ => data_sec_addr + sy.offset,
        };
        let n_sect = match sy.class {
            SectionClass::Text => 1u8,
            _ => 2u8,
        };
        rebuilt.extend_from_slice(&macho_nlist(strx, 0x0f, n_sect, 0, va));
    }
    for n in &import_ord {
        let needle = {
            let mut s = Vec::new();
            s.push(b'_');
            s.extend_from_slice(n.as_bytes());
            s.push(0);
            s
        };
        let strx = strtab
            .windows(needle.len())
            .position(|w| w == needle.as_slice())
            .unwrap_or(0) as u32;
        rebuilt.extend_from_slice(&macho_nlist(strx, 0x01, 0, 0x0100, 0));
    }
    let nlist_bytes: Vec<u8> = rebuilt;
    le[symoff_in_le..symoff_in_le + nlist_bytes.len()].copy_from_slice(&nlist_bytes);

    let codesig_in_le = le.len();
    // codesig computed after the rest of the file is built — placeholder, then patch.

    // Snapshot placement / symbols so we can mutate merged.text/data while resolving.
    let macho_place = merged.place.clone();
    let macho_sym_va: HashMap<String, u64> = merged
        .syms
        .iter()
        .map(|(n, sy)| {
            let va = match sy.class {
                SectionClass::Text => text_sec_addr + sy.offset,
                _ => data_sec_addr + sy.offset,
            };
            (n.clone(), va)
        })
        .collect();
    let lookup = |target: &RelTarget, oi: usize| -> Result<u64, LinkError> {
        match target {
            RelTarget::Local(class, off) => {
                let va = match class {
                    SectionClass::Text => text_sec_addr + macho_place[oi].text as u64 + *off,
                    _ => data_sec_addr + macho_place[oi].data as u64 + *off,
                };
                Ok(va)
            }
            RelTarget::Name(n) => {
                if let Some(&va) = macho_sym_va.get(n) {
                    return Ok(va);
                }
                if let Some(&i) = stub_index.get(n) {
                    return Ok(text_sec_addr + stubs_off as u64 + (i * stub_size) as u64);
                }
                Err(format!("unresolved Mach-O symbol '{n}'").into())
            }
        }
    };

    // Emit stubs
    for (name, &i) in &stub_index {
        let po = stubs_off + i * stub_size;
        let stub_va = text_sec_addr + po as u64;
        let got_va = data_sec_addr + got_in_data as u64 + (got_keys[name] * 8) as u64;
        match machine {
            Machine::X86_64 => {
                merged.text[po] = 0xff;
                merged.text[po + 1] = 0x25;
                let disp = got_va as i64 - (stub_va as i64 + 6);
                merged.text[po + 2..po + 6].copy_from_slice(&(disp as i32).to_le_bytes());
            }
            Machine::Aarch64 => {
                let delta = self::page(got_va) as i64 - self::page(stub_va) as i64;
                let imm = delta >> 12;
                let immlo = (imm as u32) & 3;
                let immhi = ((imm as u32) >> 2) & 0x7_ffff;
                let adrp = 0x90000010u32 | (immlo << 29) | (immhi << 5);
                let lo = ((got_va & 0xfff) >> 3) as u32;
                let ldr = 0xf9400210u32 | (lo << 10);
                let br = 0xd61f0200u32;
                merged.text[po..po + 4].copy_from_slice(&adrp.to_le_bytes());
                merged.text[po + 4..po + 8].copy_from_slice(&ldr.to_le_bytes());
                merged.text[po + 8..po + 12].copy_from_slice(&br.to_le_bytes());
            }
        }
    }

    for (oi, obj) in objects.iter().enumerate() {
        for rel in &obj.relocations {
            let (buf, sec_addr, local) = match rel.section_class {
                SectionClass::Text => (&mut merged.text, text_sec_addr, macho_place[oi].text),
                _ => (&mut merged.data, data_sec_addr, macho_place[oi].data),
            };
            let patch = local + rel.offset;
            let p = sec_addr + patch as u64;
            let s = lookup(&rel.target, oi)?;
            let ctx = obj.path.display().to_string();
            match (machine, rel.raw_type) {
                (Machine::X86_64, 0) => {
                    // UNSIGNED
                    write_u64_at(buf, patch, s.wrapping_add_signed(rel.addend), &ctx)?;
                }
                (Machine::X86_64, 1) | (Machine::X86_64, 2) => {
                    // SIGNED / BRANCH
                    write_i32_at(buf, patch, s as i64 + rel.addend - p as i64, &ctx)?;
                }
                (Machine::X86_64, 3) | (Machine::X86_64, 4) => {
                    // GOT_LOAD / GOT
                    let n = match &rel.target {
                        RelTarget::Name(n) => n,
                        _ => return Err(format!("'{ctx}': GOT reloc on local").into()),
                    };
                    let g = data_sec_addr
                        + got_in_data as u64
                        + (*got_keys.get(n).unwrap_or(&0) as u64) * 8;
                    write_i32_at(buf, patch, g as i64 + rel.addend - p as i64, &ctx)?;
                }
                (Machine::X86_64, 6) => {
                    write_i32_at(buf, patch, s as i64 + rel.addend - (p as i64 + 1), &ctx)?;
                }
                (Machine::X86_64, 7) => {
                    write_i32_at(buf, patch, s as i64 + rel.addend - (p as i64 + 2), &ctx)?;
                }
                (Machine::X86_64, 8) => {
                    write_i32_at(buf, patch, s as i64 + rel.addend - (p as i64 + 4), &ctx)?;
                }
                (Machine::Aarch64, 0) => {
                    write_u64_at(buf, patch, s.wrapping_add_signed(rel.addend), &ctx)?;
                }
                (Machine::Aarch64, 2) => {
                    a64_patch_call26(buf, patch, s, 0, p)?;
                }
                (Machine::Aarch64, 3) => {
                    a64_patch_adr_pg_hi21(buf, patch, s, rel.addend, p)?;
                }
                (Machine::Aarch64, 4) => {
                    // PAGEOFF12 — add or ldr; inspect instr
                    let instr = a64_read(buf, patch)?;
                    if (instr >> 23) & 0x3f == 0x22 || (instr & 0xFFC0_0000) == 0x9100_0000 {
                        a64_patch_add_lo12(buf, patch, s, rel.addend)?;
                    } else {
                        let shift = (instr >> 30) & 3;
                        a64_patch_ldst_lo12(buf, patch, s, rel.addend, shift)?;
                    }
                }
                (Machine::Aarch64, 5) | (Machine::Aarch64, 6) => {
                    let n = match &rel.target {
                        RelTarget::Name(n) => n,
                        _ => return Err(format!("'{ctx}': GOT reloc on local").into()),
                    };
                    let g = data_sec_addr
                        + got_in_data as u64
                        + (*got_keys.get(n).unwrap_or(&0) as u64) * 8;
                    if rel.raw_type == 5 {
                        a64_patch_adr_pg_hi21(buf, patch, g, 0, p)?;
                    } else {
                        a64_patch_ldst_lo12(buf, patch, g, 0, 3)?;
                    }
                }
                (Machine::Aarch64, 7) => {
                    let n = match &rel.target {
                        RelTarget::Name(n) => n,
                        _ => return Err(format!("'{ctx}': POINTER_TO_GOT on local").into()),
                    };
                    let g = data_sec_addr
                        + got_in_data as u64
                        + (*got_keys.get(n).unwrap_or(&0) as u64) * 8;
                    write_u64_at(buf, patch, g, &ctx)?;
                }
                _ => {
                    return Err(format!(
                        "'{ctx}': unsupported Mach-O reloc type {} ({machine:?})",
                        rel.raw_type
                    )
                    .into());
                }
            }
        }
    }

    let entryoff = if !is_dylib {
        let entry_name = if let Some(e) = &opts.entry {
            e.clone()
        } else if merged.syms.contains_key("main") {
            "main".into()
        } else if merged.syms.contains_key("lpp_main") {
            "lpp_main".into()
        } else if merged.syms.contains_key("_start") {
            "_start".into()
        } else if merged.syms.contains_key("start") {
            "start".into()
        } else {
            return Err("required symbol 'main' or '_start' not found".into());
        };
        let entry_sym = merged
            .syms
            .get(&entry_name)
            .ok_or_else(|| format!("entry '{entry_name}' not defined"))?;
        let entry_off_in_text = match entry_sym.class {
            SectionClass::Text => entry_sym.offset as usize,
            _ => return Err("Mach-O entry must be in __text".into()),
        };

        if let Some(mp) = &opts.map_path {
            write_map(
                mp,
                &merged,
                &objects,
                &entry_name,
                text_sec_addr + entry_off_in_text as u64,
            )?;
        }
        (text_fileoff + entry_off_in_text) as u64
    } else {
        0u64
    };

    // Build file without codesig first, then append codesig and patch LC.
    let linkedit_pre_sig = le.len();
    let mut bin = vec![0u8; linkedit_fileoff + linkedit_pre_sig];

    // mach_header_64
    put_u32(&mut bin, 0, 0xfeed_facf);
    put_u32(&mut bin, 4, machine.macho_cputype());
    put_u32(&mut bin, 8, machine.macho_cpusubtype());
    put_u32(&mut bin, 12, if is_dylib { 6 } else { 2 }); // MH_DYLIB vs MH_EXECUTE
    put_u32(&mut bin, 16, ncmds);
    put_u32(&mut bin, 20, sizeofcmds);
    // MH_NOUNDEFS|MH_DYLDLINK|MH_TWOLEVEL|MH_PIE
    put_u32(&mut bin, 24, 0x0020_0085);
    put_u32(&mut bin, 28, 0);

    let mut c = 32usize;
    let emit_seg = |bin: &mut [u8],
                    c: &mut usize,
                    name: &str,
                    vmaddr: u64,
                    vmsize: u64,
                    fileoff: u64,
                    filesize: u64,
                    maxprot: u32,
                    initprot: u32,
                    nsects: u32| {
        put_u32(bin, *c, 0x19); // LC_SEGMENT_64
        put_u32(bin, *c + 4, 72 + nsects * 80);
        let mut nm = [0u8; 16];
        let b = name.as_bytes();
        nm[..b.len().min(16)].copy_from_slice(&b[..b.len().min(16)]);
        bin[*c + 8..*c + 24].copy_from_slice(&nm);
        put_u64(bin, *c + 24, vmaddr);
        put_u64(bin, *c + 32, vmsize);
        put_u64(bin, *c + 40, fileoff);
        put_u64(bin, *c + 48, filesize);
        put_u32(bin, *c + 56, maxprot);
        put_u32(bin, *c + 60, initprot);
        put_u32(bin, *c + 64, nsects);
        put_u32(bin, *c + 68, 0);
        *c += 72;
    };
    let emit_sect = |bin: &mut [u8],
                     c: &mut usize,
                     sect: &str,
                     seg: &str,
                     addr: u64,
                     size: u64,
                     off: u32,
                     align_p2: u32,
                     flags: u32| {
        let mut sn = [0u8; 16];
        let mut gn = [0u8; 16];
        let sb = sect.as_bytes();
        let gb = seg.as_bytes();
        sn[..sb.len().min(16)].copy_from_slice(&sb[..sb.len().min(16)]);
        gn[..gb.len().min(16)].copy_from_slice(&gb[..gb.len().min(16)]);
        bin[*c..*c + 16].copy_from_slice(&sn);
        bin[*c + 16..*c + 32].copy_from_slice(&gn);
        put_u64(bin, *c + 32, addr);
        put_u64(bin, *c + 40, size);
        put_u32(bin, *c + 48, off);
        put_u32(bin, *c + 52, align_p2);
        put_u32(bin, *c + 56, 0);
        put_u32(bin, *c + 60, 0);
        put_u32(bin, *c + 64, flags);
        put_u32(bin, *c + 68, 0);
        put_u32(bin, *c + 72, 0);
        put_u32(bin, *c + 76, 0);
        *c += 80;
    };

    // PAGEZERO — only in executables; dylibs start at address 0 with no zero page
    if !is_dylib {
        emit_seg(&mut bin, &mut c, "__PAGEZERO", 0, vm_base, 0, 0, 0, 0, 0);
    }
    // TEXT
    emit_seg(
        &mut bin,
        &mut c,
        "__TEXT",
        text_vmaddr,
        text_seg_vmsize,
        0,
        text_seg_filesz as u64,
        7,
        5,
        1,
    );
    emit_sect(
        &mut bin,
        &mut c,
        "__text",
        "__TEXT",
        text_sec_addr,
        text_size as u64,
        text_fileoff as u32,
        4,
        0x8000_0400, // S_ATTR_PURE_INSTRUCTIONS | S_ATTR_SOME_INSTRUCTIONS
    );
    // DATA
    emit_seg(
        &mut bin,
        &mut c,
        "__DATA",
        data_vmaddr,
        data_seg_filesz as u64,
        data_fileoff as u64,
        data_seg_filesz as u64,
        7,
        3,
        1,
    );
    emit_sect(
        &mut bin,
        &mut c,
        "__data",
        "__DATA",
        data_sec_addr,
        data_size as u64,
        data_fileoff as u32,
        3,
        0,
    );
    // LINKEDIT — filesize includes codesig which we append next
    // We'll patch filesize after we know codesig length.
    let linkedit_cmd = c;
    emit_seg(
        &mut bin,
        &mut c,
        "__LINKEDIT",
        data_vmaddr + data_seg_filesz as u64,
        0, // patch
        linkedit_fileoff as u64,
        0, // patch
        7,
        1,
        0,
    );

    // SYMTAB
    put_u32(&mut bin, c, 0x2);
    put_u32(&mut bin, c + 4, 24);
    put_u32(&mut bin, c + 8, (linkedit_fileoff + symoff_in_le) as u32);
    put_u32(&mut bin, c + 12, nlists.len() as u32);
    put_u32(&mut bin, c + 16, (linkedit_fileoff + stroff_in_le) as u32);
    put_u32(&mut bin, c + 20, strtab.len() as u32);
    c += 24;

    // DYSYMTAB
    put_u32(&mut bin, c, 0xb);
    put_u32(&mut bin, c + 4, 80);
    let nlocal = 1u32;
    let nextdef = names.len() as u32;
    let nundef = import_ord.len() as u32;
    put_u32(&mut bin, c + 8, 0); // ilocalsym
    put_u32(&mut bin, c + 12, nlocal);
    put_u32(&mut bin, c + 16, nlocal); // iextdefsym
    put_u32(&mut bin, c + 20, nextdef);
    put_u32(&mut bin, c + 24, nlocal + nextdef); // iundefsym
    put_u32(&mut bin, c + 28, nundef);
    c += 80;

    if is_dylib {
        // LC_ID_DYLIB
        let id_sz = align_up(24 + ident.len() + 1, 8) as u32;
        put_u32(&mut bin, c, 0xd);
        put_u32(&mut bin, c + 4, id_sz);
        put_u32(&mut bin, c + 8, 24);
        put_u32(&mut bin, c + 12, 1);
        put_u32(&mut bin, c + 16, 0x0001_0000);
        put_u32(&mut bin, c + 20, 0x0001_0000);
        bin[c + 24..c + 24 + ident.len()].copy_from_slice(ident.as_bytes());
        c += id_sz as usize;
    } else {
        // LOAD_DYLINKER
        put_u32(&mut bin, c, 0xe);
        put_u32(&mut bin, c + 4, dylinker_cmdsize);
        put_u32(&mut bin, c + 8, 12);
        bin[c + 12..c + 12 + dylinker.len()].copy_from_slice(dylinker.as_bytes());
        c += dylinker_cmdsize as usize;
    }

    // UUID — deterministic from content hash of text+data
    let mut uuid_src = Vec::new();
    uuid_src.extend_from_slice(&merged.text);
    uuid_src.extend_from_slice(&merged.data);
    let d = sha256(&uuid_src);
    put_u32(&mut bin, c, 0x1b);
    put_u32(&mut bin, c + 4, 24);
    bin[c + 8..c + 24].copy_from_slice(&d[..16]);
    // RFC 4122 version 4 / variant bits so it looks like a real UUID
    bin[c + 8 + 6] = (bin[c + 8 + 6] & 0x0f) | 0x40;
    bin[c + 8 + 8] = (bin[c + 8 + 8] & 0x3f) | 0x80;
    c += 24;

    // BUILD_VERSION macos 12.0, sdk 14.0, tool ld 1.0
    put_u32(&mut bin, c, 0x32);
    put_u32(&mut bin, c + 4, 32);
    put_u32(&mut bin, c + 8, 1); // PLATFORM_MACOS
    put_u32(&mut bin, c + 12, 0x000C_0000);
    put_u32(&mut bin, c + 16, 0x000E_0000);
    put_u32(&mut bin, c + 20, 1);
    put_u32(&mut bin, c + 24, 3); // TOOL_LD
    put_u32(&mut bin, c + 28, 0x0002_0300);
    c += 32;

    // SOURCE_VERSION
    put_u32(&mut bin, c, 0x2A);
    put_u32(&mut bin, c + 4, 16);
    put_u64(&mut bin, c + 8, 0);
    c += 16;

    // MAIN (if not dylib)
    if !is_dylib {
        put_u32(&mut bin, c, 0x8000_0028);
        put_u32(&mut bin, c + 4, 24);
        put_u64(&mut bin, c + 8, entryoff);
        put_u64(&mut bin, c + 16, 0);
        c += 24;
    }

    // LOAD_DYLIB entries
    for dylib_path in &dylibs {
        let sz = align_up(24 + dylib_path.len() + 1, 8) as u32;
        put_u32(&mut bin, c, 0xc);
        put_u32(&mut bin, c + 4, sz);
        put_u32(&mut bin, c + 8, 24);
        put_u32(&mut bin, c + 12, 2);
        put_u32(&mut bin, c + 16, 0x0001_0000);
        put_u32(&mut bin, c + 20, 0x0001_0000);
        bin[c + 24..c + 24 + dylib_path.len()].copy_from_slice(dylib_path.as_bytes());
        c += sz as usize;
    }

    // DYLD_INFO_ONLY
    put_u32(&mut bin, c, 0x8000_0022);
    put_u32(&mut bin, c + 4, 48);
    put_u32(&mut bin, c + 8, (linkedit_fileoff + rebase_off) as u32);
    put_u32(&mut bin, c + 12, rebase.len() as u32);
    put_u32(&mut bin, c + 16, (linkedit_fileoff + bind_off) as u32);
    put_u32(&mut bin, c + 20, bind.len() as u32);
    put_u32(&mut bin, c + 24, 0);
    put_u32(&mut bin, c + 28, 0);
    put_u32(&mut bin, c + 32, 0);
    put_u32(&mut bin, c + 36, 0);
    put_u32(&mut bin, c + 40, 0);
    put_u32(&mut bin, c + 44, 0);
    c += 48;

    // CODE_SIGNATURE — dataoff/datasize patched after blob is built
    let codesig_cmd = c;
    put_u32(&mut bin, c, 0x1d);
    put_u32(&mut bin, c + 4, 16);
    put_u32(&mut bin, c + 8, 0);
    put_u32(&mut bin, c + 12, 0);
    c += 16;

    debug_assert_eq!(c, 32 + sizeofcmds as usize, "sizeofcmds mismatch");

    // copy text / data / linkedit-pre-sig
    bin[text_fileoff..text_fileoff + text_size].copy_from_slice(&merged.text);
    if data_size > 0 {
        bin[data_fileoff..data_fileoff + data_size].copy_from_slice(&merged.data);
    }
    bin[linkedit_fileoff..linkedit_fileoff + linkedit_pre_sig].copy_from_slice(&le);

    // Ad-hoc codesign over everything before the blob.
    let code_limit = linkedit_fileoff + codesig_in_le;
    let sig = macho_adhoc_codesig(&bin[..code_limit], &ident, page_size, text_seg_vmsize);
    put_u32(&mut bin, codesig_cmd + 8, code_limit as u32);
    put_u32(&mut bin, codesig_cmd + 12, sig.len() as u32);
    let le_total = codesig_in_le + sig.len();
    let le_vmsize = align_up(le_total, page_size) as u64;
    put_u64(&mut bin, linkedit_cmd + 32, le_vmsize);
    put_u64(&mut bin, linkedit_cmd + 48, le_total as u64);

    bin.resize(code_limit + sig.len(), 0);
    bin[code_limit..].copy_from_slice(&sig);

    fs::write(output, bin)
        .map_err(|e| format!("write Mach-O binary '{}': {e}", output.display()))?;
    chmod_exec(output)?;
    vlog(
        opts,
        format!(
            "Mach-O {:?} {}  entryoff=0x{entryoff:x}  imports={}",
            machine,
            output.display(),
            import_ord.len()
        ),
    );
    let _ = (fnv1a64, put_u16be);
    Ok(())
}
