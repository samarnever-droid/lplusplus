// ═══════════════════════════════════════════════════════════════════════════
// Shared types
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SectionClass {
    Text,
    Rodata,
    Data,
    Bss,
    Tls,
    InitArray,
    FiniArray,
}

impl SectionClass {
    fn tag(self) -> &'static str {
        match self {
            SectionClass::Text => "text",
            SectionClass::Rodata => "rdata",
            SectionClass::Data => "data",
            SectionClass::Bss => "bss",
            SectionClass::Tls => "tls",
            SectionClass::InitArray => "init_array",
            SectionClass::FiniArray => "fini_array",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bind {
    Local,
    Weak,
    Global,
    Common,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Elf,
    Pe,
    Macho,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Machine {
    X86_64,
    Aarch64,
}

impl Machine {
    fn from_object(a: Architecture) -> Option<Self> {
        match a {
            Architecture::X86_64 => Some(Machine::X86_64),
            Architecture::Aarch64 => Some(Machine::Aarch64),
            _ => None,
        }
    }
    fn elf_em(self) -> u16 {
        match self {
            Machine::X86_64 => 62,
            Machine::Aarch64 => 183,
        }
    }
    fn pe_machine(self) -> u16 {
        match self {
            Machine::X86_64 => 0x8664,
            Machine::Aarch64 => 0xAA64,
        }
    }
    fn macho_cputype(self) -> u32 {
        match self {
            Machine::X86_64 => 0x0100_0007,
            Machine::Aarch64 => 0x0100_000c,
        }
    }
    fn macho_cpusubtype(self) -> u32 {
        match self {
            Machine::X86_64 => 3,
            Machine::Aarch64 => 0,
        }
    }
    fn page_size(self, format: OutputFormat) -> usize {
        match (self, format) {
            (Machine::Aarch64, OutputFormat::Macho) => 0x4000,
            (Machine::Aarch64, OutputFormat::Elf) => 0x10000,
            _ => 0x1000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeSubsystem {
    Console,
    Windows,
    Native,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DynamicMode {
    /// Fail on unresolved symbols (freestanding / fully static).
    Static,
    /// If symbols remain unresolved after archives, emit PLT/IAT/dyld binds.
    Auto,
    /// Always emit a dynamically linked image.
    Force,
}

#[derive(Debug, Clone)]
pub struct LinkOptions {
    pub format: Option<OutputFormat>,
    pub machine: Option<Machine>,
    pub entry: Option<String>,
    pub image_base: Option<u64>,
    pub pie: bool,
    pub dynamic: DynamicMode,
    pub subsystem: Option<PeSubsystem>,
    pub stack_reserve: u64,
    pub stack_commit: u64,
    pub heap_reserve: u64,
    pub heap_commit: u64,
    pub strip: bool,
    pub map_path: Option<PathBuf>,
    pub allow_multiple_definition: bool,
    pub shared: bool,
    pub verbose: bool,
    pub search_paths: Vec<PathBuf>,
    pub libraries: Vec<String>,
    pub dynamic_linker: Option<String>,
    pub soname: Option<String>,
    /// Runtime library search paths embedded as `DT_RUNPATH` (ELF). Lets a
    /// produced executable locate its shared dependencies (e.g. the L++
    /// runtime cdylib) without `LD_LIBRARY_PATH`, mirroring `cc -Wl,-rpath`.
    pub rpath: Vec<String>,
    pub needed: Vec<String>,
    pub no_startup: bool,
    pub build_id: bool,
    pub page_size: Option<usize>,
    pub extra_imports: HashMap<String, String>, // symbol -> DLL / dylib
}

impl Default for LinkOptions {
    fn default() -> Self {
        Self {
            format: None,
            machine: None,
            entry: None,
            image_base: None,
            pie: false,
            dynamic: DynamicMode::Auto,
            subsystem: None,
            stack_reserve: 0x100000,
            stack_commit: 0x1000,
            heap_reserve: 0x100000,
            heap_commit: 0x1000,
            strip: false,
            map_path: None,
            allow_multiple_definition: false,
            shared: false,
            verbose: false,
            search_paths: Vec::new(),
            libraries: Vec::new(),
            dynamic_linker: None,
            soname: None,
            rpath: Vec::new(),
            needed: Vec::new(),
            no_startup: false,
            build_id: true,
            page_size: None,
            extra_imports: HashMap::new(),
        }
    }
}

/// Category of a link failure (6A.2 typed boundary).
///
/// Classified at the error site when the site knows its own category
/// (the unresolved-symbol paths are `Unresolved`, the width-check
/// relocations are `RelocationOverflow`, ...); flat messages converted
/// via `From<String>` are classified by content as a fallback so the
/// list is always populated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkErrorKind {
    /// An input or output file could not be read or written.
    Io,
    /// An input file is not a recognizable relocatable object or archive.
    Malformed,
    /// The input format or target is not one the engine can link.
    UnsupportedFormat,
    /// Symbols were not resolved by any input (see `LinkError::unresolved`).
    Unresolved,
    /// A relocation value does not fit its target field.
    RelocationOverflow,
    /// A caller or CLI misuse (bad arguments, empty input list).
    Usage,
    /// Anything not classified above.
    Internal,
}

#[derive(Debug)]
pub struct LinkError {
    /// Category of the failure (see [`LinkErrorKind`]).
    pub kind: LinkErrorKind,
    pub message: String,
    pub unresolved: Vec<String>,
}

impl LinkError {
    /// Construct an error with an explicit category.
    pub fn new(kind: LinkErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            unresolved: Vec::new(),
        }
    }

    /// An unresolved-symbol failure carrying the full symbol list.
    pub fn unresolved(message: impl Into<String>, unresolved: Vec<String>) -> Self {
        Self {
            kind: LinkErrorKind::Unresolved,
            message: message.into(),
            unresolved,
        }
    }

    /// Classify a flat message (used by `From<String>` and any site that
    /// historically reported a bare string).
    fn kind_from_message(message: &str) -> LinkErrorKind {
        let lower = message.to_ascii_lowercase();
        if lower.contains("relocation overflow")
            || lower.contains("relocation offset overflow")
            || lower.contains("immediate overflow")
        {
            LinkErrorKind::RelocationOverflow
        } else if lower.contains("unresolved") || lower.contains("not found") {
            LinkErrorKind::Unresolved
        } else if lower.contains("cannot read")
            || lower.starts_with("read '")
            || lower.contains("failed to read")
            || lower.contains("no such file")
            || lower.contains("is a directory")
            || lower.contains("permission denied")
            || lower.contains("failed to write")
        {
            LinkErrorKind::Io
        } else if lower.contains("unsupported") || lower.contains("not supported") {
            LinkErrorKind::UnsupportedFormat
        } else if lower.contains("at least one")
            || lower.contains("unknown option")
            || lower.contains("expected ")
        {
            LinkErrorKind::Usage
        } else if lower.contains("not an elf")
            || lower.contains("file magic")
            || lower.contains("bad magic")
            || lower.contains("malformed")
            || lower.contains("truncated")
            || lower.contains("not a relocatable")
            || lower.contains("invalid")
        {
            LinkErrorKind::Malformed
        } else {
            LinkErrorKind::Internal
        }
    }
}

impl From<&str> for LinkError {
    fn from(message: &str) -> Self {
        message.to_string().into()
    }
}

impl From<String> for LinkError {
    fn from(message: String) -> Self {
        let kind = Self::kind_from_message(&message);
        Self {
            kind,
            message,
            unresolved: Vec::new(),
        }
    }
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)?;
        if !self.unresolved.is_empty() {
            write!(f, "\nunresolved symbols ({}):", self.unresolved.len())?;
            for s in &self.unresolved {
                write!(f, "\n  {s}")?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for LinkError {}

impl From<LinkError> for String {
    fn from(e: LinkError) -> String {
        e.to_string()
    }
}

fn vlog(opts: &LinkOptions, msg: impl fmt::Display) {
    if opts.verbose {
        eprintln!("lpp-link: {msg}");
    }
}

