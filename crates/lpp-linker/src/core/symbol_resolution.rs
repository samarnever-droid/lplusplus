// ── Global symbol resolution ───────────────────────────────────────────────

#[derive(Clone)]
struct ResolvedSym {
    name: String,
    bind: Bind,
    class: SectionClass,
    /// Offset in the merged section of `class`.
    offset: u64,
    size: u64,
    object: usize,
}

#[derive(Clone, Copy)]
struct Placement {
    text: usize,
    rodata: usize,
    data: usize,
    bss: usize,
    tls: usize,
    init_array: usize,
    fini_array: usize,
}

struct Merged {
    text: Vec<u8>,
    rodata: Vec<u8>,
    data: Vec<u8>,
    bss_size: usize,
    tls: Vec<u8>,
    tbss_size: usize,
    tls_align: usize,
    init_array: Vec<u8>,
    fini_array: Vec<u8>,
    place: Vec<Placement>,
    syms: HashMap<String, ResolvedSym>,
    commons_off: usize,
    pdata_ranges: Vec<(usize, usize)>,
    eh_frame_ranges: Vec<(usize, usize)>,
}

impl Merged {
    fn pdata_info(&self) -> Option<(usize, usize)> {
        if self.pdata_ranges.is_empty() {
            return None;
        }
        let min_start = self.pdata_ranges.iter().map(|(s, _)| *s).min()?;
        let max_end = self.pdata_ranges.iter().map(|(s, l)| *s + *l).max()?;
        Some((min_start, max_end - min_start))
    }
    fn eh_frame_info(&self) -> Option<(usize, usize)> {
        if self.eh_frame_ranges.is_empty() {
            return None;
        }
        let min_start = self.eh_frame_ranges.iter().map(|(s, _)| *s).min()?;
        let max_end = self.eh_frame_ranges.iter().map(|(s, l)| *s + *l).max()?;
        Some((min_start, max_end - min_start))
    }
}

fn merge_objects(objects: &[ObjectImage], opts: &LinkOptions) -> Result<Merged, LinkError> {
    let mut text = Vec::new();
    let mut rodata = Vec::new();
    let mut data = Vec::new();
    let mut bss_size = 0usize;
    let mut tls = Vec::new();
    let mut tbss_size = 0usize;
    let mut tls_align = 1usize;
    let mut init_array = Vec::new();
    let mut fini_array = Vec::new();
    let mut place = Vec::new();
    let mut syms: HashMap<String, ResolvedSym> = HashMap::new();
    let mut commons: HashMap<String, CommonSym> = HashMap::new();
    let mut pdata_ranges = Vec::new();
    let mut eh_frame_ranges = Vec::new();

    for (oi, obj) in objects.iter().enumerate() {
        let t = align_up(text.len(), 16);
        text.resize(t, 0x90);
        let r = align_up(rodata.len(), 16);
        rodata.resize(r, 0);
        let d = align_up(data.len(), 16);
        data.resize(d, 0);
        let b = align_up(bss_size, obj.bss_align.max(1));
        let tl = align_up(tls.len(), obj.tls_align.max(1));
        tls.resize(tl, 0);
        let ia = align_up(init_array.len(), 8);
        init_array.resize(ia, 0);
        let fa = align_up(fini_array.len(), 8);
        fini_array.resize(fa, 0);

        place.push(Placement {
            text: t,
            rodata: r,
            data: d,
            bss: b,
            tls: tl,
            init_array: ia,
            fini_array: fa,
        });

        tls_align = tls_align.max(obj.tls_align.max(1));

        for &(start, len) in &obj.pdata_ranges {
            pdata_ranges.push((r + start, len));
        }
        for &(start, len) in &obj.eh_frame_ranges {
            eh_frame_ranges.push((r + start, len));
        }

        for s in &obj.symbols {
            if s.bind == Bind::Local {
                continue;
            }
            let abs = match s.class {
                SectionClass::Text => t as u64 + s.offset,
                SectionClass::Rodata => r as u64 + s.offset,
                SectionClass::Data => d as u64 + s.offset,
                SectionClass::Bss => b as u64 + s.offset,
                SectionClass::Tls => tl as u64 + s.offset,
                SectionClass::InitArray => ia as u64 + s.offset,
                SectionClass::FiniArray => fa as u64 + s.offset,
            };
            let cand = ResolvedSym {
                name: s.name.clone(),
                bind: s.bind,
                class: s.class,
                offset: abs,
                size: s.size,
                object: oi,
            };
            match syms.get(&s.name) {
                None => {
                    commons.remove(&s.name);
                    syms.insert(s.name.clone(), cand);
                }
                Some(prev) => {
                    // Strong definition wins over weak / common.
                    let prev_rank = match prev.bind {
                        Bind::Global => 3,
                        Bind::Common => 2,
                        Bind::Weak => 1,
                        Bind::Local => 0,
                    };
                    let new_rank = match s.bind {
                        Bind::Global => 3,
                        Bind::Common => 2,
                        Bind::Weak => 1,
                        Bind::Local => 0,
                    };
                    if new_rank > prev_rank {
                        syms.insert(s.name.clone(), cand);
                    } else if new_rank == prev_rank && prev.bind == Bind::Global {
                        if !opts.allow_multiple_definition {
                            return Err(format!(
                                "duplicate definition of symbol '{}' ({} and {})",
                                s.name,
                                objects[prev.object].path.display(),
                                obj.path.display()
                            )
                            .into());
                        }
                    }
                }
            }
        }
        for c in &obj.commons {
            if syms.contains_key(&c.name) {
                continue;
            }
            commons
                .entry(c.name.clone())
                .and_modify(|e| {
                    e.size = e.size.max(c.size);
                    e.align = e.align.max(c.align);
                })
                .or_insert_with(|| c.clone());
        }

        text.extend_from_slice(&obj.text);
        rodata.extend_from_slice(&obj.rodata);
        data.extend_from_slice(&obj.data);
        bss_size = b + obj.bss_size;
        tls.extend_from_slice(&obj.tls);
        tbss_size += obj.tbss_size;
        init_array.extend_from_slice(&obj.init_array);
        fini_array.extend_from_slice(&obj.fini_array);
    }

    // Allocate COMMON in BSS (SysV: tentative definitions).
    let commons_off = {
        let mut off = bss_size;
        for c in commons.values() {
            let al = c.align.max(1).next_power_of_two() as usize;
            off = align_up(off, al);
            syms.insert(
                c.name.clone(),
                ResolvedSym {
                    name: c.name.clone(),
                    bind: Bind::Common,
                    class: SectionClass::Bss,
                    offset: off as u64,
                    size: c.size,
                    object: usize::MAX,
                },
            );
            off += c.size as usize;
        }
        bss_size = off;
        off
    };

    Ok(Merged {
        text,
        rodata,
        data,
        bss_size,
        tls,
        tbss_size,
        tls_align,
        init_array,
        fini_array,
        place,
        syms,
        commons_off,
        pdata_ranges,
        eh_frame_ranges,
    })
}

fn collect_undefined(objects: &[ObjectImage], merged: &Merged) -> Vec<String> {
    let mut u = BTreeSet::new();
    for o in objects {
        for rel in &o.relocations {
            if let RelTarget::Name(n) = &rel.target {
                if !merged.syms.contains_key(n) && n != "__ImageBase" && !n.starts_with("__self_") {
                    u.insert(n.clone());
                }
            }
        }
        for n in &o.undefined {
            if !merged.syms.contains_key(n) {
                u.insert(n.clone());
            }
        }
    }
    u.into_iter().collect()
}

fn write_map(
    path: &Path,
    merged: &Merged,
    objects: &[ObjectImage],
    entry: &str,
    entry_va: u64,
) -> Result<(), LinkError> {
    let mut s = String::new();
    s.push_str("# lpp-link map\n");
    s.push_str(&format!("# entry {entry} = 0x{entry_va:x}\n"));
    s.push_str(&format!("# objects {}\n", objects.len()));
    s.push_str(&format!(
        "# .text {}  .rodata {}  .data {}  .bss {} (common @ +{})  .tls {}+{}\n",
        merged.text.len(),
        merged.rodata.len(),
        merged.data.len(),
        merged.bss_size,
        merged.commons_off,
        merged.tls.len(),
        merged.tbss_size
    ));
    s.push_str("# symbols\n");
    let mut names: Vec<_> = merged.syms.values().collect();
    names.sort_by_key(|sy| (sy.class as u8, sy.offset, sy.name.as_str()));
    for sy in names {
        s.push_str(&format!(
            "  {:8} +0x{:08x}  {:>8}  {}\n",
            sy.class.tag(),
            sy.offset,
            sy.size,
            sy.name
        ));
    }
    fs::write(path, s).map_err(|e| {
        LinkError::new(
            LinkErrorKind::Io,
            format!("write map '{}': {e}", path.display()),
        )
    })
}

