// ── Archives (selective extraction) ────────────────────────────────────────

struct Archive<T> {
    path: PathBuf,
    members: Vec<(String, T)>,
    /// defined symbol → member index
    index: HashMap<String, usize>,
    pulled: Vec<bool>,
}

fn parse_archive_members(path: &Path) -> Result<Vec<(String, ObjectImage)>, LinkError> {
    let bytes = fs::read(path).map_err(|e| format!("read '{}': {e}", path.display()))?;
    let archive = ArchiveFile::parse(&*bytes)
        .map_err(|e| format!("parse archive '{}': {e}", path.display()))?;
    let mut out = Vec::new();
    for member in archive.members() {
        let member = member.map_err(|e| format!("archive member in '{}': {e}", path.display()))?;
        let data = member
            .data(&*bytes)
            .map_err(|e| format!("archive member data in '{}': {e}", path.display()))?;
        if data.len() < 4 {
            continue;
        }
        let name = String::from_utf8_lossy(member.name()).to_string();
        // skip archive symbol index / longname members
        if name == "/" || name == "//" || name == "__.SYMDEF" || name.starts_with("__.SYMDEF") {
            continue;
        }
        match parse_object(data, &path.join(&name)) {
            Ok(img) => out.push((name, img)),
            Err(_) => continue,
        }
    }
    Ok(out)
}

fn archive_from_members(path: &Path, members: Vec<(String, ObjectImage)>) -> Archive<ObjectImage> {
    let mut index = HashMap::new();
    for (i, (_, img)) in members.iter().enumerate() {
        for n in img.defined_names() {
            index.entry(n.to_string()).or_insert(i);
        }
    }
    let n = members.len();
    Archive {
        path: path.to_path_buf(),
        members,
        index,
        pulled: vec![false; n],
    }
}

fn pull_archives(
    objects: &mut Vec<ObjectImage>,
    archives: &mut [Archive<ObjectImage>],
    opts: &LinkOptions,
) -> Result<(), LinkError> {
    loop {
        let mut need: BTreeSet<String> = BTreeSet::new();
        let mut have: BTreeSet<String> = BTreeSet::new();
        for o in objects.iter() {
            for n in o.defined_names() {
                have.insert(n.to_string());
            }
            for u in &o.undefined {
                need.insert(u.clone());
            }
        }
        for u in have.iter() {
            need.remove(u);
        }
        let mut added = false;
        for name in need {
            for ar in archives.iter_mut() {
                if let Some(&idx) = ar.index.get(&name) {
                    if !ar.pulled[idx] {
                        ar.pulled[idx] = true;
                        let (mname, img) = &ar.members[idx];
                        vlog(
                            opts,
                            format!("pull {}({}) for `{name}`", ar.path.display(), mname),
                        );
                        objects.push(ObjectImage {
                            path: ar.path.join(mname),
                            machine: img.machine,
                            format: img.format,
                            text: img.text.clone(),
                            rodata: img.rodata.clone(),
                            data: img.data.clone(),
                            bss_size: img.bss_size,
                            bss_align: img.bss_align,
                            tls: img.tls.clone(),
                            tbss_size: img.tbss_size,
                            tls_align: img.tls_align,
                            init_array: img.init_array.clone(),
                            fini_array: img.fini_array.clone(),
                            symbols: img.symbols.clone(),
                            commons: img.commons.clone(),
                            relocations: img
                                .relocations
                                .iter()
                                .map(|r| Relocation {
                                    offset: r.offset,
                                    target: match &r.target {
                                        RelTarget::Name(s) => RelTarget::Name(s.clone()),
                                        RelTarget::Local(c, o) => RelTarget::Local(*c, *o),
                                    },
                                    addend: r.addend,
                                    size: r.size,
                                    kind: r.kind,
                                    section_class: r.section_class,
                                    raw_type: r.raw_type,
                                })
                                .collect(),
                            undefined: img.undefined.clone(),
                            pdata_ranges: img.pdata_ranges.clone(),
                            eh_frame_ranges: img.eh_frame_ranges.clone(),
                        });
                        added = true;
                    }
                    break;
                }
            }
        }
        if !added {
            break;
        }
    }
    Ok(())
}

fn load_objects(
    inputs: &[PathBuf],
    opts: &LinkOptions,
) -> Result<(Vec<ObjectImage>, OutputFormat, Machine), LinkError> {
    if inputs.is_empty() {
        return Err("at least one input object is required".into());
    }
    let mut objects = Vec::new();
    let mut archives = Vec::new();
    let mut search_dirs = opts.search_paths.clone();
    search_dirs.push(PathBuf::from("."));
    search_dirs.push(PathBuf::from("/usr/lib"));
    search_dirs.push(PathBuf::from("/usr/local/lib"));
    search_dirs.push(PathBuf::from("/lib"));
    for lib in &opts.libraries {
        let p = PathBuf::from(lib);
        if let Some(parent) = p.parent() {
            if !parent.as_os_str().is_empty() && !search_dirs.contains(&parent.to_path_buf()) {
                search_dirs.push(parent.to_path_buf());
            }
        }
    }

    for p in inputs {
        let bytes = fs::read(p).map_err(|e| format!("read '{}': {e}", p.display()))?;
        if is_archive_bytes(&bytes) {
            let members = parse_archive_members(p)?;
            archives.push(archive_from_members(p, members));
            continue;
        }
        objects.push(parse_object(&bytes, p)?);
    }
    for lib in &opts.libraries {
        let path = PathBuf::from(lib);
        if path.exists() {
            if let Ok(bytes) = fs::read(&path) {
                if is_archive_bytes(&bytes) {
                    if let Ok(members) = parse_archive_members(&path) {
                        archives.push(archive_from_members(&path, members));
                    }
                }
            }
            continue;
        }
        let clean = lib
            .strip_prefix("lib")
            .unwrap_or(lib)
            .strip_suffix(".a")
            .unwrap_or(lib)
            .strip_suffix(".dylib")
            .unwrap_or(lib)
            .strip_suffix(".so")
            .unwrap_or(lib);
        let candidates = [
            lib.clone(),
            format!("lib{clean}.a"),
            format!("lib{clean}.dylib"),
            format!("lib{clean}.so"),
        ];
        let mut found = false;
        for dir in &search_dirs {
            for cand in &candidates {
                let cp = dir.join(cand);
                if cp.exists() {
                    if let Ok(bytes) = fs::read(&cp) {
                        if is_archive_bytes(&bytes) {
                            if let Ok(members) = parse_archive_members(&cp) {
                                archives.push(archive_from_members(&cp, members));
                            }
                        }
                    }
                    found = true;
                    break;
                }
            }
            if found {
                break;
            }
        }
        if !found && opts.dynamic == DynamicMode::Static {
            return Err(format!(
                "cannot find library '{lib}' in {}",
                search_dirs
                    .iter()
                    .map(|d| d.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .into());
        }
    }
    if objects.is_empty() {
        // start from the first archive member that defines a plausible entry
        let entries = [
            "main",
            "lpp_main",
            "_start",
            "start",
            "mainCRTStartup",
            "WinMain",
        ];
        let mut seeded = false;
        for ar in archives.iter_mut() {
            for e in entries {
                if let Some(&idx) = ar.index.get(e) {
                    if !ar.pulled[idx] {
                        ar.pulled[idx] = true;
                        let (mname, img) = &ar.members[idx];
                        objects.push(parse_object_clone(img, &ar.path.join(mname)));
                        seeded = true;
                        break;
                    }
                }
            }
            if seeded {
                break;
            }
        }
        if objects.is_empty() {
            return Err("no relocatable objects found in inputs".into());
        }
    }
    pull_archives(&mut objects, &mut archives, opts)?;

    let format = opts.format.unwrap_or(objects[0].format);
    let machine = opts.machine.unwrap_or(objects[0].machine);
    for o in &objects {
        if o.machine != machine {
            return Err(format!(
                "'{}': architecture mismatch ({:?} vs {:?})",
                o.path.display(),
                o.machine,
                machine
            )
            .into());
        }
        if o.format != format {
            return Err(format!(
                "'{}': object format mismatch ({:?} vs {:?})",
                o.path.display(),
                o.format,
                format
            )
            .into());
        }
    }
    Ok((objects, format, machine))
}

fn parse_object_clone(img: &ObjectImage, path: &Path) -> ObjectImage {
    ObjectImage {
        path: path.to_path_buf(),
        machine: img.machine,
        format: img.format,
        text: img.text.clone(),
        rodata: img.rodata.clone(),
        data: img.data.clone(),
        bss_size: img.bss_size,
        bss_align: img.bss_align,
        tls: img.tls.clone(),
        tbss_size: img.tbss_size,
        tls_align: img.tls_align,
        init_array: img.init_array.clone(),
        fini_array: img.fini_array.clone(),
        symbols: img.symbols.clone(),
        commons: img.commons.clone(),
        relocations: img
            .relocations
            .iter()
            .map(|r| Relocation {
                offset: r.offset,
                target: match &r.target {
                    RelTarget::Name(s) => RelTarget::Name(s.clone()),
                    RelTarget::Local(c, o) => RelTarget::Local(*c, *o),
                },
                addend: r.addend,
                size: r.size,
                kind: r.kind,
                section_class: r.section_class,
                raw_type: r.raw_type,
            })
            .collect(),
        undefined: img.undefined.clone(),
        pdata_ranges: img.pdata_ranges.clone(),
        eh_frame_ranges: img.eh_frame_ranges.clone(),
    }
}

