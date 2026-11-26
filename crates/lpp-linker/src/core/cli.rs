use super::*;

// ═══════════════════════════════════════════════════════════════════════════
// inspect
// ═══════════════════════════════════════════════════════════════════════════

pub fn inspect_object(input: &Path) -> Result<(), String> {
    inspect_object_t(input).map_err(String::from)
}

/// Typed variant of [`inspect_object`] returning the engine's [`LinkError`]
/// (6A.2 boundary); the public function is its `String` adapter.
pub fn inspect_object_t(input: &Path) -> Result<(), LinkError> {
    let bytes = fs::read(input).map_err(|e| format!("read '{}': {e}", input.display()))?;
    if is_archive_bytes(&bytes) {
        let archive = ArchiveFile::parse(&*bytes)
            .map_err(|e| format!("parse archive '{}': {e}", input.display()))?;
        println!("format: archive");
        println!("path: {}", input.display());
        let mut n = 0usize;
        for m in archive.members().flatten() {
            let name = String::from_utf8_lossy(m.name());
            if let Ok(data) = m.data(&*bytes) {
                println!("  member {name}  {} bytes", data.len());
                n += 1;
            }
        }
        println!("members: {n}");
        return Ok(());
    }
    let file =
        object::File::parse(&*bytes).map_err(|e| format!("parse '{}': {e}", input.display()))?;
    let mut reloc_count = 0usize;
    let mut reloc_kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut reloc_types: BTreeMap<u32, usize> = BTreeMap::new();
    println!("format: {:?}", file.format());
    println!("architecture: {:?}", file.architecture());
    println!("endian: {:?}", file.endianness());
    println!("is-64: {}", file.is_64());
    println!("entry: 0x{:x}", file.entry());
    println!("sections:");
    for sec in file.sections() {
        for (_, rel) in sec.relocations() {
            reloc_count += 1;
            *reloc_kinds.entry(format!("{:?}", rel.kind())).or_default() += 1;
            *reloc_types.entry(raw_reloc_type(&rel)).or_default() += 1;
        }
        println!(
            "  {:<20} size={:<8} align={:<4} kind={:?}",
            sec.name().unwrap_or("<unnamed>"),
            sec.size(),
            sec.align(),
            sec.kind()
        );
    }
    let defined = file.symbols().filter(|s| !s.is_undefined()).count();
    let undefined = file.symbols().filter(|s| s.is_undefined()).count();
    let weak = file.symbols().filter(|s| s.is_weak()).count();
    let common = file.symbols().filter(|s| s.is_common()).count();
    println!("symbols: defined={defined} undefined={undefined} weak={weak} common={common}");
    println!("undefined:");
    for s in file.symbols().filter(|s| s.is_undefined()) {
        if let Ok(n) = s.name()
            && !n.is_empty()
            && !n.starts_with('.')
        {
            println!("  {n}");
        }
    }
    println!("relocations: {reloc_count}");
    println!("relocation-kinds:");
    for (k, c) in reloc_kinds {
        println!("  {k}={c}");
    }
    println!("relocation-types:");
    for (k, c) in reloc_types {
        println!("  {k}={c}");
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// High-level entrypoints and CLI
// ═══════════════════════════════════════════════════════════════════════════

pub fn usage() {
    eprintln!(
        "lpp-link {} — L++ direct native linker",
        env!("CARGO_PKG_VERSION")
    );
    eprintln!();
    eprintln!("Usage: lpp-link [mode] [options] <objects...> -o <output>");
    eprintln!("       lpp-link inspect <object.o|archive.a>");
    eprintln!();
    eprintln!("Modes (optional; sniffed from the first input if omitted):");
    eprintln!("  elf | pe | macho");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  -o FILE                 output path (required)");
    eprintln!("  -e, --entry NAME        entry symbol");
    eprintln!("  -L DIR                  add library search path");
    eprintln!("  -l NAME                 add libNAME.a / NAME.lib");
    eprintln!("  -m TARGET               elf_x86_64 | elf_aarch64 | i386pep | arm64pe");
    eprintln!("                          macho_x86_64 | macho_arm64");
    eprintln!("  --subsystem console|windows|native");
    eprintln!("  --image-base HEX        image base (ELF ET_EXEC / PE)");
    eprintln!("  --pie / --no-pie        ELF ET_DYN / fixed exec");
    eprintln!("  --static / --dynamic    unresolved-symbol policy");
    eprintln!("  --dll, -shared          PE DLL / ELF ET_DYN");
    eprintln!("  --dynamic-linker PATH   ELF PT_INTERP");
    eprintln!("  --needed LIB            extra DT_NEEDED / implicit dylib");
    eprintln!("  --import DLL=sym[,sym]  extra PE import mapping");
    eprintln!("  --stack RES[,CMT]       PE stack reserve/commit");
    eprintln!("  --no-startup            do not inject _start");
    eprintln!("  --strip                 omit ELF .symtab");
    eprintln!("  --allow-multiple-definition");
    eprintln!("  --build-id / --no-build-id");
    eprintln!("  -Map FILE               write link map");
    eprintln!("  -v, --verbose");
    eprintln!("  -h, --help   -V, --version");
    eprintln!("  @file                   response file");
}

pub fn sniff_format(path: &Path) -> &'static str {
    let Ok(bytes) = fs::read(path) else {
        return "elf";
    };
    if is_archive_bytes(&bytes) {
        // Prefer the first real member.
        if let Ok(ar) = ArchiveFile::parse(&*bytes) {
            for m in ar.members().flatten() {
                if let Ok(d) = m.data(&*bytes)
                    && d.len() >= 4
                {
                    if &d[0..4] == b"\x7fELF" {
                        return "elf";
                    }
                    let machine = u16::from_le_bytes([d[0], d[1]]);
                    if machine == 0x8664 || machine == 0xAA64 || machine == 0x14C {
                        return "pe";
                    }
                    let be = u32::from_be_bytes([d[0], d[1], d[2], d[3]]);
                    let le = u32::from_le_bytes([d[0], d[1], d[2], d[3]]);
                    const MACHO: [u32; 4] = [0xFEED_FACE, 0xFEED_FACF, 0xCAFE_BABE, 0xCAFE_D00D];
                    if MACHO.contains(&be) || MACHO.contains(&le) {
                        return "macho";
                    }
                }
            }
        }
        return "elf";
    }
    if bytes.len() >= 4 {
        if &bytes[0..4] == b"\x7fELF" {
            return "elf";
        }
        let be = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let le = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        const MACHO_MAGICS: [u32; 4] = [0xFEED_FACE, 0xFEED_FACF, 0xCAFE_BABE, 0xCAFE_D00D];
        if MACHO_MAGICS.contains(&be) || MACHO_MAGICS.contains(&le) {
            return "macho";
        }
        let machine = u16::from_le_bytes([bytes[0], bytes[1]]);
        if machine == 0x8664 || machine == 0xAA64 || machine == 0x14C {
            return "pe";
        }
    }
    "elf"
}

/// Base-compatible legacy entry point.
///
/// Keep this signature stable so existing L++ compiler integration can call the
/// linker without changing its FFI/module wiring. New callers may use
/// `link_with_options` for extended control.
pub fn link_direct(inputs: &[PathBuf], output: &Path) -> Result<(), String> {
    link_with_options(inputs, output, &LinkOptions::default())
}

pub fn link_with_options(
    inputs: &[PathBuf],
    output: &Path,
    opts: &LinkOptions,
) -> Result<(), String> {
    link_with_options_t(inputs, output, opts).map_err(String::from)
}

/// Typed variant of [`link_with_options`] returning the engine's [`LinkError`]
/// (6A.2 boundary); the public function is its `String` adapter.
pub fn link_with_options_t(
    inputs: &[PathBuf],
    output: &Path,
    opts: &LinkOptions,
) -> Result<(), LinkError> {
    if inputs.is_empty() {
        return Err("at least one input object is required".into());
    }
    let fmt = opts.format.unwrap_or_else(|| {
        match inputs.first().map(|p| sniff_format(p)).unwrap_or("elf") {
            "pe" => OutputFormat::Pe,
            "macho" => OutputFormat::Macho,
            _ => OutputFormat::Elf,
        }
    });
    match fmt {
        OutputFormat::Pe => write_pe_with_options_t(inputs, output, opts),
        OutputFormat::Macho => write_macho_with_options_t(inputs, output, opts),
        OutputFormat::Elf => write_elf_with_options_t(inputs, output, opts),
    }
}

pub fn expand_response_files(args: Vec<String>) -> Result<Vec<String>, String> {
    expand_response_files_t(args).map_err(String::from)
}

/// Typed variant of [`expand_response_files`] returning the engine's [`LinkError`]
/// (6A.2 boundary); the public function is its `String` adapter.
pub fn expand_response_files_t(args: Vec<String>) -> Result<Vec<String>, LinkError> {
    let mut expanded = Vec::new();
    for arg in args {
        if let Some(rsp_path) = arg.strip_prefix('@') {
            let content = fs::read_to_string(rsp_path)
                .map_err(|e| format!("failed to read response file '@{rsp_path}': {e}"))?;
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                // Support simple quoted tokens.
                let mut cur = String::new();
                let mut in_q = false;
                for ch in trimmed.chars() {
                    match ch {
                        '"' => in_q = !in_q,
                        c if c.is_whitespace() && !in_q => {
                            if !cur.is_empty() {
                                expanded.push(std::mem::take(&mut cur));
                            }
                        }
                        _ => cur.push(ch),
                    }
                }
                if !cur.is_empty() {
                    expanded.push(cur);
                }
            }
        } else {
            expanded.push(arg);
        }
    }
    Ok(expanded)
}

fn resolve_lib(
    name: &str,
    paths: &[PathBuf],
    fmt: Option<OutputFormat>,
) -> Result<PathBuf, LinkError> {
    let p_name = Path::new(name);
    if p_name.is_file() {
        return Ok(p_name.to_path_buf());
    }
    let mut candidates = match fmt {
        Some(OutputFormat::Pe) => vec![
            format!("{name}.lib"),
            format!("lib{name}.lib"),
            format!("lib{name}.a"),
            format!("{name}.obj"),
        ],
        Some(OutputFormat::Macho) => vec![
            format!("lib{name}.a"),
            format!("lib{name}.dylib"),
            format!("{name}.o"),
        ],
        _ => vec![
            format!("lib{name}.a"),
            format!("lib{name}.so"),
            format!("{name}.o"),
            format!("lib{name}.lib"),
        ],
    };
    candidates.insert(0, name.to_string());
    let mut search = paths.to_vec();
    if let Some(parent) = p_name.parent()
        && !parent.as_os_str().is_empty()
    {
        search.push(parent.to_path_buf());
    }
    search.push(PathBuf::from("."));
    search.push(PathBuf::from("/usr/lib"));
    search.push(PathBuf::from("/usr/local/lib"));
    search.push(PathBuf::from("/lib"));
    search.push(PathBuf::from("/lib64"));
    search.push(PathBuf::from("/usr/lib/x86_64-linux-gnu"));
    search.push(PathBuf::from("/usr/lib/aarch64-linux-gnu"));
    for dir in &search {
        for c in &candidates {
            let p = dir.join(c);
            if p.is_file() {
                return Ok(p);
            }
        }
    }
    Err(format!(
        "cannot find library '{name}' in {}",
        search
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )
    .into())
}

fn parse_hex_u64(s: &str) -> Result<u64, LinkError> {
    let t = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    u64::from_str_radix(t, 16)
        .or_else(|_| s.parse::<u64>())
        .map_err(|_| LinkError::from(format!("invalid integer '{s}'")))
}

pub fn link_cli(args: &[String]) -> Result<(), String> {
    link_cli_t(args).map_err(String::from)
}

/// Typed variant of [`link_cli`] returning the engine's [`LinkError`]
/// (6A.2 boundary); the public function is its `String` adapter.
pub fn link_cli_t(args: &[String]) -> Result<(), LinkError> {
    let args = expand_response_files(args.to_vec())?;
    if args.first().map(String::as_str) == Some("inspect") {
        if args.len() != 2 {
            usage();
            return Err("inspect requires exactly one object file argument".into());
        }
        return inspect_object_t(Path::new(&args[1]));
    }
    match args.first().map(String::as_str) {
        Some("-h") | Some("--help") | Some("help") => {
            usage();
            return Ok(());
        }
        Some("-V") | Some("--version") | Some("version") => {
            println!("lpp-link {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        _ => {}
    }

    let mut opts = LinkOptions::default();
    let mut inputs: Vec<PathBuf> = Vec::new();
    let mut output: Option<PathBuf> = None;
    let mut i = 0usize;
    if matches!(
        args.first().map(String::as_str),
        Some("pe")
            | Some("elf")
            | Some("macho")
            | Some("macho-arm64")
            | Some("macho-x86_64")
            | Some("elf-arm64")
            | Some("elf-x86_64")
            | Some("pe-arm64")
            | Some("pe-x86_64")
    ) {
        opts.format = Some(match args[0].as_str() {
            "pe" | "pe-arm64" | "pe-x86_64" => OutputFormat::Pe,
            "macho" | "macho-arm64" | "macho-x86_64" => OutputFormat::Macho,
            _ => OutputFormat::Elf,
        });
        // Subcommand aliases that pin the machine
        match args[0].as_str() {
            "macho-arm64" | "elf-arm64" | "pe-arm64" => opts.machine = Some(Machine::Aarch64),
            "macho-x86_64" | "elf-x86_64" | "pe-x86_64" => opts.machine = Some(Machine::X86_64),
            _ => {}
        }
        i = 1;
    }
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-o" => {
                i += 1;
                output = Some(PathBuf::from(args.get(i).ok_or("missing argument for -o")?));
            }
            "-e" | "--entry" => {
                i += 1;
                opts.entry = Some(args.get(i).ok_or("missing argument for --entry")?.clone());
            }
            "-L" => {
                i += 1;
                opts.search_paths
                    .push(PathBuf::from(args.get(i).ok_or("missing argument for -L")?));
            }
            "-l" => {
                i += 1;
                opts.libraries
                    .push(args.get(i).ok_or("missing argument for -l")?.clone());
            }
            "-m" => {
                i += 1;
                let t = args.get(i).ok_or("missing argument for -m")?.as_str();
                match t {
                    "elf_x86_64" | "elf_amd64" => {
                        opts.format = Some(OutputFormat::Elf);
                        opts.machine = Some(Machine::X86_64);
                    }
                    "elf_aarch64" | "aarch64linux" | "aarch64elf" => {
                        opts.format = Some(OutputFormat::Elf);
                        opts.machine = Some(Machine::Aarch64);
                    }
                    "i386pep" | "x86_64pe" | "pe" => {
                        opts.format = Some(OutputFormat::Pe);
                        opts.machine = Some(Machine::X86_64);
                    }
                    "arm64pe" | "aarch64pe" => {
                        opts.format = Some(OutputFormat::Pe);
                        opts.machine = Some(Machine::Aarch64);
                    }
                    "macho_x86_64" => {
                        opts.format = Some(OutputFormat::Macho);
                        opts.machine = Some(Machine::X86_64);
                    }
                    "macho_arm64" | "arm64macos" => {
                        opts.format = Some(OutputFormat::Macho);
                        opts.machine = Some(Machine::Aarch64);
                    }
                    _ => return Err(format!("unknown emulation '{t}'").into()),
                }
            }
            "--subsystem" => {
                i += 1;
                opts.subsystem = Some(match args.get(i).ok_or("missing --subsystem")?.as_str() {
                    "console" => PeSubsystem::Console,
                    "windows" => PeSubsystem::Windows,
                    "native" => PeSubsystem::Native,
                    s => return Err(format!("unknown subsystem '{s}'").into()),
                });
            }
            "--image-base" => {
                i += 1;
                opts.image_base = Some(parse_hex_u64(args.get(i).ok_or("missing --image-base")?)?);
            }
            "--pie" => opts.pie = true,
            "--no-pie" => opts.pie = false,
            "--static" => opts.dynamic = DynamicMode::Static,
            "--dynamic" => opts.dynamic = DynamicMode::Force,
            "--dll" | "-shared" | "--shared" => opts.shared = true,
            "--dynamic-linker" => {
                i += 1;
                opts.dynamic_linker = Some(args.get(i).ok_or("missing --dynamic-linker")?.clone());
            }
            "--needed" => {
                i += 1;
                opts.needed
                    .push(args.get(i).ok_or("missing --needed")?.clone());
            }
            "-rpath" | "--rpath" => {
                i += 1;
                // Accept colon-separated groups, matching `ld -rpath a:b`.
                let spec = args.get(i).ok_or("missing -rpath PATH")?;
                for p in spec.split(':').filter(|s| !s.is_empty()) {
                    opts.rpath.push(p.to_string());
                }
            }
            "--import" => {
                i += 1;
                let spec = args.get(i).ok_or("missing --import DLL=sym[,sym]")?;
                let (dll, syms) = spec
                    .split_once('=')
                    .ok_or("--import expects DLL=sym[,sym]")?;
                for s in syms.split(',') {
                    let s = s.trim();
                    if !s.is_empty() {
                        opts.extra_imports.insert(s.to_string(), dll.to_string());
                    }
                }
            }
            "--stack" => {
                i += 1;
                let spec = args.get(i).ok_or("missing --stack")?;
                let mut parts = spec.split(',');
                opts.stack_reserve = parse_hex_u64(parts.next().unwrap_or("0x100000"))?;
                if let Some(c) = parts.next() {
                    opts.stack_commit = parse_hex_u64(c)?;
                }
            }
            "--no-startup" => opts.no_startup = true,
            "--strip" | "--strip-all" => opts.strip = true,
            "--allow-multiple-definition" => opts.allow_multiple_definition = true,
            "--build-id" => opts.build_id = true,
            "--no-build-id" => opts.build_id = false,
            "-Map" | "--map" => {
                i += 1;
                opts.map_path = Some(PathBuf::from(args.get(i).ok_or("missing -Map")?));
            }
            "-v" | "--verbose" => opts.verbose = true,
            "-h" | "--help" => {
                usage();
                return Ok(());
            }
            "-V" | "--version" => {
                println!("lpp-link {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            s if s.starts_with("-l") && s.len() > 2 => {
                opts.libraries.push(s[2..].to_string());
            }
            s if s.starts_with("-L") && s.len() > 2 => {
                opts.search_paths.push(PathBuf::from(&s[2..]));
            }
            s if s.starts_with('-') => {
                return Err(format!("unknown option '{s}'").into());
            }
            _ => inputs.push(PathBuf::from(a)),
        }
        i += 1;
    }

    let output = output.ok_or_else(|| {
        usage();
        "missing '-o <output>' argument".to_string()
    })?;

    for path in &inputs {
        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            let ext_lower = ext.to_lowercase();
            if matches!(
                ext_lower.as_str(),
                "c" | "cpp" | "cc" | "cxx" | "lpp" | "rs"
            ) {
                return Err(format!(
                    "Input file '{}' is a source file. lpp-link requires compiled objects (.o / .obj) or archives (.a / .lib).",
                    path.display()
                ).into());
            }
        }
    }

    for lib in &opts.libraries.clone() {
        let p = resolve_lib(lib, &opts.search_paths, opts.format)?;
        vlog(&opts, format!("-l{lib} -> {}", p.display()));
        inputs.push(p);
    }

    if inputs.is_empty() {
        usage();
        return Err("no input objects".into());
    }

    link_with_options_t(&inputs, &output, &opts)
}

// ═══════════════════════════════════════════════════════════════════════════
// Calculation tests
// ═══════════════════════════════════════════════════════════════════════════
