// ═══════════════════════════════════════════════════════════════════════════
// Input images
// ═══════════════════════════════════════════════════════════════════════════

struct ObjectImage {
    path: PathBuf,
    machine: Machine,
    format: OutputFormat,
    text: Vec<u8>,
    rodata: Vec<u8>,
    data: Vec<u8>,
    bss_size: usize,
    bss_align: usize,
    tls: Vec<u8>,
    tbss_size: usize,
    tls_align: usize,
    init_array: Vec<u8>,
    fini_array: Vec<u8>,
    symbols: Vec<Defined>,
    commons: Vec<CommonSym>,
    relocations: Vec<Relocation>,
    undefined: Vec<String>,
    pdata_ranges: Vec<(usize, usize)>,
    eh_frame_ranges: Vec<(usize, usize)>,
}

impl ObjectImage {
    fn defined_names(&self) -> impl Iterator<Item = &str> {
        self.symbols
            .iter()
            .filter(|s| s.bind != Bind::Local)
            .map(|s| s.name.as_str())
            .chain(self.commons.iter().map(|c| c.name.as_str()))
    }
}

fn classify_section(name: &str, kind: object::SectionKind) -> Option<SectionClass> {
    if name.starts_with(".debug")
        || name.starts_with(".drectve")
        || name.starts_with(".comment")
        || name.starts_with(".note")
        || name.starts_with(".llvm_")
        || name == ".llvmbc"
        || name.starts_with(".group")
    {
        return None;
    }
    if name == ".init_array" || name.starts_with(".init_array.") || name == ".ctors" {
        return Some(SectionClass::InitArray);
    }
    if name == ".fini_array" || name.starts_with(".fini_array.") || name == ".dtors" {
        return Some(SectionClass::FiniArray);
    }
    if name.starts_with(".text")
        || name.starts_with("__text")
        || name == ".multiboot2"
        || name == ".boot"
        || kind == object::SectionKind::Text
    {
        return Some(SectionClass::Text);
    }
    if name.starts_with(".rdata")
        || name.starts_with(".rodata")
        || name.starts_with(".xdata")
        || name.starts_with(".pdata")
        || name.starts_with("__const")
        || name.starts_with("__cstring")
        || name.starts_with("__literal")
        || kind == object::SectionKind::ReadOnlyData
        || kind == object::SectionKind::ReadOnlyString
        || kind == object::SectionKind::ReadOnlyDataWithRel
    {
        return Some(SectionClass::Rodata);
    }
    if name.starts_with(".tls")
        || name.starts_with(".tdata")
        || name.starts_with(".tbss")
        || kind == object::SectionKind::Tls
        || kind == object::SectionKind::UninitializedTls
    {
        return Some(SectionClass::Tls);
    }
    if name == ".bss" || name.starts_with(".bss.") || kind == object::SectionKind::UninitializedData
    {
        return Some(SectionClass::Bss);
    }
    if name.starts_with(".data") || name.starts_with("__data") || kind == object::SectionKind::Data
    {
        return Some(SectionClass::Data);
    }
    None
}

fn raw_reloc_type(rel: &object::Relocation) -> u32 {
    match rel.flags() {
        object::RelocationFlags::Elf { r_type } => r_type,
        object::RelocationFlags::Coff { typ } => typ as u32,
        object::RelocationFlags::MachO { r_type, .. } => r_type as u32,
        _ => 0,
    }
}

fn bind_of(sym: &object::Symbol<'_, '_>) -> Bind {
    if sym.is_common() || matches!(sym.section(), SymbolSection::Common) {
        Bind::Common
    } else if sym.is_weak() {
        Bind::Weak
    } else if sym.is_local() {
        Bind::Local
    } else {
        Bind::Global
    }
}

fn parse_object(bytes: &[u8], path: &Path) -> Result<ObjectImage, LinkError> {
    let file =
        object::File::parse(bytes).map_err(|e| format!("parse '{}': {e}", path.display()))?;
    let format = match file.format() {
        BinaryFormat::Elf => OutputFormat::Elf,
        BinaryFormat::Coff | BinaryFormat::Pe => OutputFormat::Pe,
        BinaryFormat::MachO => OutputFormat::Macho,
        other => {
            return Err(
                format!("'{}': unsupported object format {other:?}", path.display()).into(),
            );
        }
    };
    let machine = Machine::from_object(file.architecture()).ok_or_else(|| {
        format!(
            "'{}': unsupported architecture {:?}",
            path.display(),
            file.architecture()
        )
    })?;

    if file.kind() == object::ObjectKind::Dynamic {
        return Ok(ObjectImage {
            path: path.to_path_buf(),
            machine,
            format,
            text: Vec::new(),
            rodata: Vec::new(),
            data: Vec::new(),
            bss_size: 0,
            bss_align: 1,
            tls: Vec::new(),
            tbss_size: 0,
            tls_align: 1,
            init_array: Vec::new(),
            fini_array: Vec::new(),
            symbols: Vec::new(),
            commons: Vec::new(),
            relocations: Vec::new(),
            undefined: Vec::new(),
            pdata_ranges: Vec::new(),
            eh_frame_ranges: Vec::new(),
        });
    }

    let mut text = Vec::new();
    let mut rodata = Vec::new();
    let mut data = Vec::new();
    let mut bss_size = 0usize;
    let mut bss_align = 1usize;
    let mut tls = Vec::new();
    let mut tbss_size = 0usize;
    let mut tls_align = 1usize;
    let mut init_array = Vec::new();
    let mut fini_array = Vec::new();
    let mut map: Vec<(object::SectionIndex, SectionClass, usize)> = Vec::new();
    let mut relocs = Vec::new();
    let mut pdata_ranges = Vec::new();
    let mut eh_frame_ranges = Vec::new();

    for sec in file.sections() {
        let name = sec.name().unwrap_or("");
        if let object::SectionFlags::Coff { characteristics } = sec.flags() {
            // IMAGE_SCN_LNK_INFO | IMAGE_SCN_LNK_REMOVE
            if (characteristics & 0x00000800) != 0 || (characteristics & 0x00000200) != 0 {
                continue;
            }
        }
        let Some(class) = classify_section(name, sec.kind()) else {
            continue;
        };
        let sec_align = usize::try_from(sec.align()).unwrap_or(1).max(1);
        let is_bss = class == SectionClass::Bss
            || matches!(
                sec.kind(),
                object::SectionKind::UninitializedData | object::SectionKind::UninitializedTls
            )
            || name == ".tbss"
            || name.starts_with(".tbss.");

        let (buf, fill): (&mut Vec<u8>, u8) = match class {
            SectionClass::Text => (
                &mut text,
                if format == OutputFormat::Pe {
                    0xCC
                } else {
                    0x90
                },
            ),
            SectionClass::Rodata => (&mut rodata, 0),
            SectionClass::Data => (&mut data, 0),
            SectionClass::InitArray => (&mut init_array, 0),
            SectionClass::FiniArray => (&mut fini_array, 0),
            SectionClass::Tls => {
                if is_bss {
                    tls_align = tls_align.max(sec_align);
                    let base = align_up(tls.len() + tbss_size, sec_align);
                    tbss_size = base + usize::try_from(sec.size()).unwrap_or(0) - tls.len();
                    map.push((sec.index(), class, base));
                    continue;
                }
                tls_align = tls_align.max(sec_align);
                (&mut tls, 0)
            }
            SectionClass::Bss => {
                bss_align = bss_align.max(sec_align);
                let base = align_up(bss_size, sec_align);
                let n = usize::try_from(sec.size()).unwrap_or(0);
                bss_size = base + n;
                map.push((sec.index(), class, base));
                continue;
            }
        };

        let base = align_up(buf.len(), sec_align.max(1));
        buf.resize(base, fill);
        if is_bss && class == SectionClass::Tls {
            let n = usize::try_from(sec.size()).unwrap_or(0);
            buf.resize(base + n, 0);
            map.push((sec.index(), class, base));
            continue;
        }
        let bytes = if is_bss {
            vec![0u8; usize::try_from(sec.size()).unwrap_or(0)]
        } else {
            sec.uncompressed_data()
                .map_err(|e| format!("read section '{name}' from '{}': {e}", path.display()))?
                .into_owned()
        };
        buf.extend_from_slice(&bytes);
        map.push((sec.index(), class, base));

        if class == SectionClass::Rodata {
            if name == ".pdata" || name.starts_with(".pdata") {
                pdata_ranges.push((base, bytes.len()));
            }
            if name == ".eh_frame"
                || name.starts_with(".eh_frame")
                || name == ".eh_frame_hdr"
                || name.starts_with(".eh_frame_hdr")
            {
                eh_frame_ranges.push((base, bytes.len()));
            }
        }
    }

    for sec in file.sections() {
        let name = sec.name().unwrap_or("");
        if let object::SectionFlags::Coff { characteristics } = sec.flags()
            && ((characteristics & 0x00000800) != 0 || (characteristics & 0x00000200) != 0)
        {
            continue;
        }
        let Some(class) = classify_section(name, sec.kind()) else {
            continue;
        };
        let Some((_, _, base)) = map.iter().find(|(i, _, _)| *i == sec.index()) else {
            continue;
        };
        let base = *base;

        let buf: &[u8] = match class {
            SectionClass::Text => &text,
            SectionClass::Rodata => &rodata,
            SectionClass::Data => &data,
            SectionClass::InitArray => &init_array,
            SectionClass::FiniArray => &fini_array,
            SectionClass::Tls => &tls,
            SectionClass::Bss => &[],
        };

        for (off, rel) in sec.relocations() {
            let raw_off = usize::try_from(off).map_err(|_| "relocation offset overflow")?;
            let RelocationTarget::Symbol(si) = rel.target() else {
                return Err(format!(
                    "'{}': unsupported non-symbol relocation in '{name}'",
                    path.display()
                )
                .into());
            };
            let sym = file
                .symbol_by_index(si)
                .map_err(|e| format!("read relocation symbol: {e}"))?;
            let raw_name = sym
                .name()
                .map_err(|e| format!("read relocation symbol name: {e}"))?;
            let mut addend = rel.addend();
            // COFF stores the addend in the section bytes (PE/COFF spec 5.6).
            if format == OutputFormat::Pe {
                let site = base + raw_off;
                if rel.size() == 64 {
                    if let Ok(v) = read_i64_at(buf, site) {
                        addend = v;
                    }
                } else if (rel.size() == 32 || rel.size() == 0)
                    && let Ok(v) = read_i32_at(buf, site)
                {
                    addend = v as i64;
                }
            }
            if format == OutputFormat::Macho {
                addend = (addend as i32) as i64;
            }
            let target = resolve_local_target(raw_name, &sym, &map, format);
            relocs.push(Relocation {
                offset: base + raw_off,
                target,
                addend,
                size: rel.size(),
                kind: rel.kind(),
                section_class: class,
                raw_type: raw_reloc_type(&rel),
            });
        }
    }

    let mut symbols = Vec::new();
    let mut commons = Vec::new();
    let mut undefined = Vec::new();
    let mut seen_undef = BTreeSet::new();

    for sym in file.symbols() {
        let Ok(name) = sym.name() else { continue };
        if name.is_empty() || name.starts_with('.') || name.starts_with('$') {
            continue;
        }
        if matches!(
            sym.kind(),
            SymbolKind::File | SymbolKind::Section | SymbolKind::Unknown
        ) {
            continue;
        }
        let clean = if format == OutputFormat::Macho {
            name.strip_prefix('_').unwrap_or(name)
        } else {
            name
        };
        if clean.is_empty() {
            continue;
        }
        if sym.is_undefined() && !sym.is_common() {
            if seen_undef.insert(clean.to_string()) {
                undefined.push(clean.to_string());
            }
            continue;
        }
        if sym.is_common() || matches!(sym.section(), SymbolSection::Common) {
            commons.push(CommonSym {
                name: clean.to_string(),
                size: sym.size().max(1),
                align: sym.address().max(1).next_power_of_two(),
            });
            continue;
        }
        if let SymbolSection::Section(idx) = sym.section()
            && let Some((_, class, base)) = map.iter().find(|(i, _, _)| *i == idx)
        {
            symbols.push(Defined {
                name: clean.to_string(),
                bind: bind_of(&sym),
                class: *class,
                offset: *base as u64 + sym.address(),
                size: sym.size(),
                align: 1,
                object: 0,
            });
        }
    }

    Ok(ObjectImage {
        path: path.to_path_buf(),
        machine,
        format,
        text,
        rodata,
        data,
        bss_size,
        bss_align,
        tls,
        tbss_size,
        tls_align,
        init_array,
        fini_array,
        symbols,
        commons,
        relocations: relocs,
        undefined,
        pdata_ranges,
        eh_frame_ranges,
    })
}

fn resolve_local_target(
    raw_name: &str,
    sym: &object::Symbol<'_, '_>,
    map: &[(object::SectionIndex, SectionClass, usize)],
    format: OutputFormat,
) -> RelTarget {
    let clean = if format == OutputFormat::Macho {
        raw_name.strip_prefix('_').unwrap_or(raw_name)
    } else {
        raw_name
    };
    let anonymous = clean.is_empty()
        || sym.kind() == SymbolKind::Section
        || clean.starts_with('.')
        || clean.starts_with('$');
    if let SymbolSection::Section(idx) = sym.section()
        && let Some((_, class, base)) = map.iter().find(|(i, _, _)| *i == idx)
        && (anonymous || sym.is_local() || clean.starts_with('.'))
    {
        return RelTarget::Local(*class, *base as u64 + sym.address());
    }
    if anonymous {
        return RelTarget::Local(SectionClass::Text, 0);
    }
    RelTarget::Name(clean.to_string())
}

