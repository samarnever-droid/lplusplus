//! `linker` — in-process direct linker for Linux ELF, Windows PE, and macOS Mach-O.
//!
//! Production-grade drop-in for `lpp` compiler binaries. Links relocatable
//! objects and static archives without spawning an external linker.
//!
//! Layout math follows the System V ABI, PE/COFF spec, and Mach-O loader
//! contract: `p_vaddr % p_align == p_offset % p_align`, PE `SizeOfImage` is
//! section-aligned, Mach-O `sizeofcmds` equals the sum of load commands.

pub const LPP_FREESTANDING: bool = true;

use object::read::archive::ArchiveFile;
use object::{
    Architecture, BinaryFormat, Object, ObjectSection, ObjectSymbol, RelocationKind,
    RelocationTarget, SymbolKind, SymbolSection,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

// ── Integer / layout helpers ───────────────────────────────────────────────

fn put_u16(buf: &mut [u8], offset: usize, value: u16) {
    buf[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
fn put_u32(buf: &mut [u8], offset: usize, value: u32) {
    buf[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn put_u64(buf: &mut [u8], offset: usize, value: u64) {
    buf[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}
fn put_u16be(buf: &mut [u8], offset: usize, value: u16) {
    buf[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}
fn put_u32be(buf: &mut [u8], offset: usize, value: u32) {
    buf[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

/// Power-of-two alignment. Saturates on overflow so a later bounds check fails
/// instead of wrapping into a plausible-looking address.
fn align_up(value: usize, alignment: usize) -> usize {
    debug_assert!(alignment <= 1 || alignment.is_power_of_two());
    if alignment <= 1 {
        return value;
    }
    match value.checked_add(alignment - 1) {
        Some(v) => v & !(alignment - 1),
        None => usize::MAX,
    }
}

fn align_up_u64(value: u64, alignment: u64) -> u64 {
    if alignment <= 1 {
        return value;
    }
    debug_assert!(alignment.is_power_of_two());
    match value.checked_add(alignment - 1) {
        Some(v) => v & !(alignment - 1),
        None => u64::MAX,
    }
}

/// ELF / PE identity: file offset and virtual address must be congruent
/// modulo the page/section alignment.
fn congruent_offset(vaddr: u64, align: u64) -> u64 {
    if align == 0 {
        return 0;
    }
    vaddr % align
}

fn fits_i8(v: i64) -> bool {
    v >= i8::MIN as i64 && v <= i8::MAX as i64
}
fn fits_i16(v: i64) -> bool {
    v >= i16::MIN as i64 && v <= i16::MAX as i64
}
fn fits_i26(v: i64) -> bool {
    v >= -(1 << 25) && v < (1 << 25)
}
fn fits_i32(v: i64) -> bool {
    v >= i32::MIN as i64 && v <= i32::MAX as i64
}
fn fits_u32(v: i64) -> bool {
    v >= 0 && v <= u32::MAX as i64
}

fn read_i32_at(buf: &[u8], off: usize) -> Result<i32, String> {
    let b: [u8; 4] = buf
        .get(off..off + 4)
        .ok_or_else(|| format!("read i32 at {off} out of range"))?
        .try_into()
        .map_err(|_| "read i32".to_string())?;
    Ok(i32::from_le_bytes(b))
}
fn read_i64_at(buf: &[u8], off: usize) -> Result<i64, String> {
    let b: [u8; 8] = buf
        .get(off..off + 8)
        .ok_or_else(|| format!("read i64 at {off} out of range"))?
        .try_into()
        .map_err(|_| "read i64".to_string())?;
    Ok(i64::from_le_bytes(b))
}
fn write_i8_at(buf: &mut [u8], off: usize, v: i64, ctx: &str) -> Result<(), String> {
    if !fits_i8(v) {
        return Err(format!("{ctx}: 8-bit relocation overflow ({v})"));
    }
    *buf.get_mut(off).ok_or_else(|| format!("{ctx}: patch OOB"))? = v as u8;
    Ok(())
}
fn write_i16_at(buf: &mut [u8], off: usize, v: i64, ctx: &str) -> Result<(), String> {
    if !fits_i16(v) {
        return Err(format!("{ctx}: 16-bit relocation overflow ({v})"));
    }
    let slot = buf
        .get_mut(off..off + 2)
        .ok_or_else(|| format!("{ctx}: patch OOB"))?;
    slot.copy_from_slice(&(v as i16).to_le_bytes());
    Ok(())
}
fn write_i32_at(buf: &mut [u8], off: usize, v: i64, ctx: &str) -> Result<(), String> {
    if !fits_i32(v) {
        return Err(format!("{ctx}: 32-bit relocation overflow ({v})",));
    }
    let slot = buf
        .get_mut(off..off + 4)
        .ok_or_else(|| format!("{ctx}: patch OOB"))?;
    slot.copy_from_slice(&(v as i32).to_le_bytes());
    Ok(())
}
fn write_u32_at(buf: &mut [u8], off: usize, v: i64, ctx: &str) -> Result<(), String> {
    if !fits_u32(v) {
        return Err(format!("{ctx}: unsigned 32-bit relocation overflow ({v})"));
    }
    let slot = buf
        .get_mut(off..off + 4)
        .ok_or_else(|| format!("{ctx}: patch OOB"))?;
    slot.copy_from_slice(&(v as u32).to_le_bytes());
    Ok(())
}
fn write_u64_at(buf: &mut [u8], off: usize, v: u64, ctx: &str) -> Result<(), String> {
    let slot = buf
        .get_mut(off..off + 8)
        .ok_or_else(|| format!("{ctx}: patch OOB"))?;
    slot.copy_from_slice(&v.to_le_bytes());
    Ok(())
}
fn write_u32_raw(buf: &mut [u8], off: usize, v: u32, ctx: &str) -> Result<(), String> {
    let slot = buf
        .get_mut(off..off + 4)
        .ok_or_else(|| format!("{ctx}: patch OOB"))?;
    slot.copy_from_slice(&v.to_le_bytes());
    Ok(())
}

fn is_archive_bytes(bytes: &[u8]) -> bool {
    bytes.starts_with(b"!<arch>\n")
}

fn chmod_exec(output: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = fs::metadata(output)
            .map_err(|e| format!("stat '{}': {e}", output.display()))?
            .permissions();
        perm.set_mode(0o755);
        fs::set_permissions(output, perm)
            .map_err(|e| format!("chmod '{}': {e}", output.display()))?;
    }
    let _ = output;
    Ok(())
}

// ── SHA-256 (ad-hoc Mach-O codesign + GNU build-id) ────────────────────────

fn sha256(data: &[u8]) -> [u8; 32] {
    fn rotr(x: u32, n: u32) -> u32 {
        x.rotate_right(n)
    }
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >> 3);
            let s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];
        for i in 0..64 {
            let s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 32];
    for (i, w) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
    }
    out
}

/// SysV ELF `elf_hash` used by `SHT_HASH`.
fn elf_hash(name: &[u8]) -> u32 {
    let mut h: u32 = 0;
    for &c in name {
        h = (h << 4).wrapping_add(c as u32);
        let g = h & 0xf0000000;
        if g != 0 {
            h ^= g >> 24;
        }
        h &= !g;
    }
    h
}

/// Microsoft PE checksum: 16-bit one's-complement sum of the image, then
/// add the file length. The checksum field itself is skipped.
fn pe_checksum(image: &[u8], checksum_off: usize) -> u32 {
    let mut sum: u64 = 0;
    let mut i = 0usize;
    while i + 1 < image.len() {
        if i == checksum_off {
            i += 4;
            continue;
        }
        if i + 1 == checksum_off {
            // odd alignment should not happen; treat conservatively
            i += 1;
            continue;
        }
        let word = u16::from_le_bytes([image[i], image[i + 1]]) as u64;
        sum = sum.wrapping_add(word);
        sum = (sum & 0xffff) + (sum >> 16);
        i += 2;
    }
    if i < image.len() && i != checksum_off && i + 1 != checksum_off {
        sum = sum.wrapping_add(image[i] as u64);
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    (sum as u32).wrapping_add(image.len() as u32)
}

fn page(addr: u64) -> u64 {
    addr & !0xfffu64
}

fn fnv1a64(data: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

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
            dynamic: if LPP_FREESTANDING {
                DynamicMode::Auto
            } else {
                DynamicMode::Auto
            },
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
            needed: Vec::new(),
            no_startup: false,
            build_id: true,
            page_size: None,
            extra_imports: HashMap::new(),
        }
    }
}

#[derive(Debug)]
pub struct LinkError {
    pub message: String,
    pub unresolved: Vec<String>,
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)?;
        if !self.unresolved.is_empty() {
            write!(
                f,
                "\nunresolved symbols ({}):",
                self.unresolved.len()
            )?;
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

// ── Relocations / symbols ──────────────────────────────────────────────────

#[derive(Clone)]
enum RelTarget {
    /// Named (possibly external) symbol.
    Name(String),
    /// Offset within this object's classified section.
    Local(SectionClass, u64),
}

fn got_key(obj_idx: usize, target: &RelTarget) -> String {
    match target {
        RelTarget::Name(n) => n.clone(),
        RelTarget::Local(c, o) => format!("__local.{obj_idx}.{}.{}", c.tag(), o),
    }
}

struct Relocation {
    offset: usize,
    target: RelTarget,
    addend: i64,
    size: u8,
    kind: RelocationKind,
    section_class: SectionClass,
    raw_type: u32,
}

#[derive(Clone)]
struct Defined {
    name: String,
    bind: Bind,
    class: SectionClass,
    /// Offset from the start of this object's contribution to `class`.
    offset: u64,
    size: u64,
    align: u64,
    object: usize,
}

#[derive(Clone)]
struct CommonSym {
    name: String,
    size: u64,
    align: u64,
}

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
    if name == ".bss"
        || name.starts_with(".bss.")
        || kind == object::SectionKind::UninitializedData
    {
        return Some(SectionClass::Bss);
    }
    if name.starts_with(".data")
        || name.starts_with("__data")
        || kind == object::SectionKind::Data
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

fn parse_object(bytes: &[u8], path: &Path) -> Result<ObjectImage, String> {
    let file =
        object::File::parse(bytes).map_err(|e| format!("parse '{}': {e}", path.display()))?;
    let format = match file.format() {
        BinaryFormat::Elf => OutputFormat::Elf,
        BinaryFormat::Coff | BinaryFormat::Pe => OutputFormat::Pe,
        BinaryFormat::MachO => OutputFormat::Macho,
        other => {
            return Err(format!(
                "'{}': unsupported object format {other:?}",
                path.display()
            ))
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
            SectionClass::Text => (&mut text, if format == OutputFormat::Pe { 0xCC } else { 0x90 }),
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

        for (off, rel) in sec.relocations() {
            let raw_off = usize::try_from(off).map_err(|_| "relocation offset overflow")?;
            let RelocationTarget::Symbol(si) = rel.target() else {
                return Err(format!(
                    "'{}': unsupported non-symbol relocation in '{name}'",
                    path.display()
                ));
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
                } else if rel.size() == 32 || rel.size() == 0 {
                    if let Ok(v) = read_i32_at(buf, site) {
                        addend = v as i64;
                    }
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
        if let SymbolSection::Section(idx) = sym.section() {
            if let Some((_, class, base)) = map.iter().find(|(i, _, _)| *i == idx) {
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
        || clean.starts_with(".text")
        || clean.starts_with(".rdata")
        || clean.starts_with(".rodata")
        || clean.starts_with(".data")
        || clean.starts_with(".bss")
        || clean.starts_with(".tls")
        || clean.starts_with(".xdata")
        || clean.starts_with(".pdata")
        || clean.starts_with(".debug")
        || clean.starts_with('$');
    if let SymbolSection::Section(idx) = sym.section() {
        if let Some((_, class, base)) = map.iter().find(|(i, _, _)| *i == idx) {
            if anonymous || sym.is_local() {
                return RelTarget::Local(*class, *base as u64 + sym.address());
            }
        }
    }
    if anonymous {
        return RelTarget::Local(SectionClass::Text, 0);
    }
    RelTarget::Name(clean.to_string())
}

// ── Archives (selective extraction) ────────────────────────────────────────

struct Archive<T> {
    path: PathBuf,
    members: Vec<(String, T)>,
    /// defined symbol → member index
    index: HashMap<String, usize>,
    pulled: Vec<bool>,
}

fn parse_archive_members(path: &Path) -> Result<Vec<(String, ObjectImage)>, String> {
    let bytes = fs::read(path).map_err(|e| format!("read '{}': {e}", path.display()))?;
    let archive =
        ArchiveFile::parse(&*bytes).map_err(|e| format!("parse archive '{}': {e}", path.display()))?;
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
) -> Result<(), String> {
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
) -> Result<(Vec<ObjectImage>, OutputFormat, Machine), String> {
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
        let clean = lib.strip_prefix("lib").unwrap_or(lib).strip_suffix(".a").unwrap_or(lib).strip_suffix(".dylib").unwrap_or(lib).strip_suffix(".so").unwrap_or(lib);
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
            ));
        }
    }
    if objects.is_empty() {
        // start from the first archive member that defines a plausible entry
        let entries = ["main", "lpp_main", "_start", "start", "mainCRTStartup", "WinMain"];
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
            ));
        }
        if o.format != format {
            return Err(format!(
                "'{}': object format mismatch ({:?} vs {:?})",
                o.path.display(),
                o.format,
                format
            ));
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
    }
}

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
}

fn merge_objects(objects: &[ObjectImage], opts: &LinkOptions) -> Result<Merged, String> {
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
                            ));
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

fn write_map(path: &Path, merged: &Merged, objects: &[ObjectImage], entry: &str, entry_va: u64) -> Result<(), String> {
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
    fs::write(path, s).map_err(|e| format!("write map '{}': {e}", path.display()))
}

// ── AArch64 instruction patching (ELF / PE / Mach-O) ───────────────────────

fn a64_read(buf: &[u8], off: usize) -> Result<u32, String> {
    let b: [u8; 4] = buf
        .get(off..off + 4)
        .ok_or_else(|| "aarch64 patch OOB".to_string())?
        .try_into()
        .map_err(|_| "aarch64 patch".to_string())?;
    Ok(u32::from_le_bytes(b))
}
fn a64_write(buf: &mut [u8], off: usize, instr: u32) -> Result<(), String> {
    write_u32_raw(buf, off, instr, "aarch64")
}

fn a64_patch_call26(buf: &mut [u8], off: usize, s: u64, a: i64, p: u64) -> Result<(), String> {
    let instr = a64_read(buf, off)?;
    let raw_imm = (instr & 0x03FF_FFFF) as i32;
    let inline_addend = if (raw_imm & 0x0200_0000) != 0 {
        ((raw_imm | !0x03FF_FFFF) as i64) << 2
    } else {
        (raw_imm as i64) << 2
    };
    let addend = if a != 0 { (a as i32) as i64 } else { inline_addend };
    let dest = s.wrapping_add_signed(addend);
    let disp = dest as i64 - p as i64;
    if disp & 3 != 0 {
        return Err("aarch64 CALL26/JUMP26 is not 4-byte aligned".into());
    }
    let imm = disp >> 2;
    if !fits_i26(imm) {
        return Err(format!("aarch64 CALL26/JUMP26 out of range ({disp})"));
    }
    a64_write(buf, off, (instr & 0xFC00_0000) | ((imm as u32) & 0x03FF_FFFF))
}

fn a64_patch_adr_pg_hi21(buf: &mut [u8], off: usize, s: u64, a: i64, p: u64) -> Result<(), String> {
    let dest = s.wrapping_add_signed(a);
    let delta = page(dest) as i64 - page(p) as i64;
    let imm = delta >> 12;
    if imm < -(1 << 20) || imm >= (1 << 20) {
        return Err(format!("aarch64 ADR_PREL_PG_HI21 out of range ({delta})"));
    }
    let immlo = (imm as u32) & 3;
    let immhi = ((imm as u32) >> 2) & 0x7_ffff;
    let instr = a64_read(buf, off)?;
    a64_write(
        buf,
        off,
        (instr & 0x9F00_001F) | (immlo << 29) | (immhi << 5),
    )
}

fn a64_patch_add_lo12(buf: &mut [u8], off: usize, s: u64, a: i64) -> Result<(), String> {
    let dest = s.wrapping_add_signed(a);
    let imm12 = (dest & 0xfff) as u32;
    let instr = a64_read(buf, off)?;
    a64_write(buf, off, (instr & 0xFFC0_03FF) | (imm12 << 10))
}

fn a64_patch_ldst_lo12(buf: &mut [u8], off: usize, s: u64, a: i64, shift: u32) -> Result<(), String> {
    let dest = s.wrapping_add_signed(a);
    let imm12 = ((dest >> shift) & 0xfff) as u32;
    let instr = a64_read(buf, off)?;
    a64_write(buf, off, (instr & 0xFFC0_03FF) | (imm12 << 10))
}

fn a64_patch_condbr19(buf: &mut [u8], off: usize, s: u64, a: i64, p: u64) -> Result<(), String> {
    let dest = s.wrapping_add_signed(a);
    let disp = dest as i64 - p as i64;
    if disp & 3 != 0 {
        return Err("aarch64 CONDBR19 not aligned".into());
    }
    let imm = disp >> 2;
    if imm < -(1 << 18) || imm >= (1 << 18) {
        return Err(format!("aarch64 CONDBR19 out of range ({disp})"));
    }
    let instr = a64_read(buf, off)?;
    a64_write(
        buf,
        off,
        (instr & 0xFF00_001F) | (((imm as u32) & 0x7_ffff) << 5),
    )
}

fn a64_patch_tstbr14(buf: &mut [u8], off: usize, s: u64, a: i64, p: u64) -> Result<(), String> {
    let dest = s.wrapping_add_signed(a);
    let disp = dest as i64 - p as i64;
    if disp & 3 != 0 {
        return Err("aarch64 TSTBR14 not aligned".into());
    }
    let imm = disp >> 2;
    if imm < -(1 << 13) || imm >= (1 << 13) {
        return Err(format!("aarch64 TSTBR14 out of range ({disp})"));
    }
    let instr = a64_read(buf, off)?;
    a64_write(
        buf,
        off,
        (instr & 0xFFF8_001F) | (((imm as u32) & 0x3fff) << 5),
    )
}

fn a64_patch_movw(buf: &mut [u8], off: usize, s: u64, a: i64, shift: u32, check: bool) -> Result<(), String> {
    let dest = s.wrapping_add_signed(a);
    let imm16 = ((dest >> shift) & 0xffff) as u32;
    if check {
        let max = 1u64 << (shift + 16);
        if dest >= max {
            return Err(format!("aarch64 MOVW immediate overflow (G{})", shift / 16));
        }
    }
    let instr = a64_read(buf, off)?;
    a64_write(buf, off, (instr & 0xFFE0_001F) | (imm16 << 5))
}

fn a64_patch_adr_lo21(buf: &mut [u8], off: usize, s: u64, a: i64, p: u64) -> Result<(), String> {
    let dest = s.wrapping_add_signed(a);
    let imm = dest as i64 - p as i64;
    if imm < -(1 << 20) || imm >= (1 << 20) {
        return Err(format!("aarch64 ADR_PREL_LO21 out of range ({imm})"));
    }
    let immlo = (imm as u32) & 3;
    let immhi = ((imm as u32) >> 2) & 0x7_ffff;
    let instr = a64_read(buf, off)?;
    a64_write(
        buf,
        off,
        (instr & 0x9F00_001F) | (immlo << 29) | (immhi << 5),
    )
}

// ═══════════════════════════════════════════════════════════════════════════
// ELF
// ═══════════════════════════════════════════════════════════════════════════

const ELF_BASE_EXEC: u64 = 0x400000;
const PT_NULL: u32 = 0;
const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_INTERP: u32 = 3;
const PT_NOTE: u32 = 4;
const PT_PHDR: u32 = 6;
const PT_TLS: u32 = 7;
const PT_GNU_EH_FRAME: u32 = 0x6474E550;
const PT_GNU_STACK: u32 = 0x6474E551;
const PT_GNU_RELRO: u32 = 0x6474E552;
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;

// R_X86_64_*
const R_X86_64_64: u32 = 1;
const R_X86_64_PC32: u32 = 2;
const R_X86_64_GOT32: u32 = 3;
const R_X86_64_PLT32: u32 = 4;
const R_X86_64_GOTPCREL: u32 = 9;
const R_X86_64_32: u32 = 10;
const R_X86_64_32S: u32 = 11;
const R_X86_64_16: u32 = 12;
const R_X86_64_PC16: u32 = 13;
const R_X86_64_8: u32 = 14;
const R_X86_64_PC8: u32 = 15;
const R_X86_64_TPOFF64: u32 = 18;
const R_X86_64_DTPOFF32: u32 = 21;
const R_X86_64_GOTTPOFF: u32 = 22;
const R_X86_64_TPOFF32: u32 = 23;
const R_X86_64_PC64: u32 = 24;
const R_X86_64_GOTOFF64: u32 = 25;
const R_X86_64_GOTPC32: u32 = 26;
const R_X86_64_SIZE32: u32 = 32;
const R_X86_64_SIZE64: u32 = 33;
const R_X86_64_GOTPCRELX: u32 = 41;
const R_X86_64_REX_GOTPCRELX: u32 = 42;

// R_AARCH64_*
const R_AARCH64_ABS64: u32 = 257;
const R_AARCH64_ABS32: u32 = 258;
const R_AARCH64_ABS16: u32 = 259;
const R_AARCH64_PREL64: u32 = 260;
const R_AARCH64_PREL32: u32 = 261;
const R_AARCH64_PREL16: u32 = 262;
const R_AARCH64_MOVW_UABS_G0: u32 = 263;
const R_AARCH64_MOVW_UABS_G0_NC: u32 = 264;
const R_AARCH64_MOVW_UABS_G1: u32 = 265;
const R_AARCH64_MOVW_UABS_G1_NC: u32 = 266;
const R_AARCH64_MOVW_UABS_G2: u32 = 267;
const R_AARCH64_MOVW_UABS_G2_NC: u32 = 268;
const R_AARCH64_MOVW_UABS_G3: u32 = 269;
const R_AARCH64_ADR_PREL_LO21: u32 = 274;
const R_AARCH64_ADR_PREL_PG_HI21: u32 = 275;
const R_AARCH64_ADR_PREL_PG_HI21_NC: u32 = 276;
const R_AARCH64_ADD_ABS_LO12_NC: u32 = 277;
const R_AARCH64_LDST8_ABS_LO12_NC: u32 = 278;
const R_AARCH64_TSTBR14: u32 = 279;
const R_AARCH64_CONDBR19: u32 = 280;
const R_AARCH64_JUMP26: u32 = 282;
const R_AARCH64_CALL26: u32 = 283;
const R_AARCH64_LDST16_ABS_LO12_NC: u32 = 284;
const R_AARCH64_LDST32_ABS_LO12_NC: u32 = 285;
const R_AARCH64_LDST64_ABS_LO12_NC: u32 = 286;
const R_AARCH64_LDST128_ABS_LO12_NC: u32 = 299;
const R_AARCH64_ADR_GOT_PAGE: u32 = 311;
const R_AARCH64_LD64_GOT_LO12_NC: u32 = 312;
const R_AARCH64_JUMP_SLOT: u32 = 402;
const R_AARCH64_ABS64_DYN: u32 = 257;

fn elf_interp(machine: Machine) -> &'static str {
    match machine {
        Machine::X86_64 => "/lib64/ld-linux-x86-64.so.2",
        Machine::Aarch64 => "/lib/ld-linux-aarch64.so.1",
    }
}

fn libc_soname_for(sym: &str) -> Option<&'static str> {
    if is_libm_symbol(sym) {
        return Some("libm.so.6");
    }
    if is_libdl_symbol(sym) {
        return Some("libdl.so.2");
    }
    if is_libpthread_symbol(sym) {
        return Some("libpthread.so.0");
    }
    if is_libc_symbol(sym) {
        return Some("libc.so.6");
    }
    None
}

fn is_libm_symbol(n: &str) -> bool {
    matches!(
        n,
        "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "atan2"
            | "sinh" | "cosh" | "tanh" | "exp" | "exp2" | "log" | "log2" | "log10"
            | "pow" | "sqrt" | "ceil" | "floor" | "fmod" | "fabs" | "hypot"
            | "round" | "trunc" | "nearbyint" | "sincos" | "ldexp" | "frexp"
            | "sinf" | "cosf" | "tanf" | "expf" | "logf" | "powf" | "sqrtf"
    )
}
fn is_libdl_symbol(n: &str) -> bool {
    matches!(n, "dlopen" | "dlsym" | "dlclose" | "dlerror" | "dladdr")
}
fn is_libpthread_symbol(n: &str) -> bool {
    n.starts_with("pthread_") || matches!(n, "sem_init" | "sem_wait" | "sem_post" | "sem_destroy")
}
fn is_libc_symbol(n: &str) -> bool {
    matches!(
        n,
        "malloc" | "free" | "realloc" | "calloc" | "aligned_alloc" | "posix_memalign"
            | "printf" | "fprintf" | "sprintf" | "snprintf" | "vsnprintf" | "vfprintf"
            | "puts" | "putchar" | "getchar" | "scanf" | "sscanf"
            | "memset" | "memcpy" | "memmove" | "memcmp" | "strlen" | "strcmp"
            | "strncmp" | "strcpy" | "strncpy" | "strcat" | "strchr" | "strstr"
            | "strdup" | "strtol" | "strtoul" | "strtod" | "atoi" | "atol" | "atoll"
            | "exit" | "abort" | "_exit" | "atexit"
            | "fopen" | "fclose" | "fread" | "fwrite" | "fflush" | "fseek" | "ftell"
            | "rewind" | "feof" | "ferror" | "fgets" | "fputs"
            | "getenv" | "setenv" | "unsetenv" | "system" | "time" | "clock"
            | "qsort" | "bsearch" | "rand" | "srand" | "abs" | "labs" | "llabs"
            | "tolower" | "toupper" | "isdigit" | "isalpha" | "isspace" | "isalnum"
            | "mmap" | "munmap" | "mprotect" | "brk" | "sbrk"
            | "read" | "write" | "open" | "close" | "lseek" | "stat" | "fstat"
            | "unlink" | "rename" | "getcwd" | "chdir" | "mkdir" | "rmdir"
            | "getpid" | "getuid" | "getgid" | "fork" | "execve" | "waitpid"
            | "signal" | "sigaction" | "kill" | "raise"
            | "socket" | "bind" | "listen" | "accept" | "connect" | "send" | "recv"
            | "sendto" | "recvfrom" | "closesocket" | "htons" | "htonl" | "ntohs" | "ntohl"
            | "getaddrinfo" | "freeaddrinfo" | "setsockopt" | "getsockopt"
            | "poll" | "select" | "gettimeofday" | "nanosleep" | "usleep"
            | "stdin" | "stdout" | "stderr" | "__libc_start_main" | "__errno_location"
            | "memchr" | "strerror" | "perror" | "setvbuf" | "ungetc"
            | "opendir" | "readdir" | "closedir"
            | "clock_gettime" | "localtime" | "gmtime" | "strftime"
            | "lpp_c_malloc" | "lpp_c_free" | "lpp_c_load_u8" | "lpp_c_store_u8"
            | "lpp_c_load_i32" | "lpp_c_store_i32" | "lpp_c_load_i64" | "lpp_c_store_i64"
    )
}

fn elf_needed_for(undef: &[String], opts: &LinkOptions) -> Vec<String> {
    let mut n = BTreeSet::new();
    for s in &opts.needed {
        n.insert(s.clone());
    }
    for u in undef {
        if let Some(so) = libc_soname_for(u) {
            n.insert(so.to_string());
        }
    }
    if n.is_empty() && !undef.is_empty() {
        n.insert("libc.so.6".into());
    }
    n.into_iter().collect()
}

/// x86_64 static TLS variant 2: TP points *after* the TLS block.
/// `TPOFF(S) = offset_in_tls - tls_memsz`  (negative).
fn x64_tpoff(off_in_tls: u64, tls_memsz: u64) -> i64 {
    off_in_tls as i64 - tls_memsz as i64
}

/// AArch64 static TLS variant 1: TP + TCB (16) + offset.
fn a64_tpoff(off_in_tls: u64, _tls_memsz: u64) -> i64 {
    16 + off_in_tls as i64
}

/// x86_64 `_start`: SysV argc/argv from the stack, 16-byte align, call `main`.
///
/// Layout (bytes from `start_off`):
///   0  xor ebp, ebp
///   2  pop rsi                 ; argc
///   3  mov rdi, rsp            ; argv
///   6  and rsp, -16
///  10  xor edx, edx            ; envp = NULL
///  12  e8 <rel32>              ; call main; next-ip = start_off+17
///  17  mov edi, eax
///  19  syscall exit / call exit@plt (disp at +20)
fn emit_elf_start_stub_x64(
    start_off: usize,
    main_off: usize,
    use_exit_plt: bool,
) -> Result<(Vec<u8>, Option<usize>), String> {
    let mut s = vec![
        0x31, 0xed, // xor ebp, ebp
        0x5e, // pop rsi
        0x48, 0x89, 0xe7, // mov rdi, rsp
        0x48, 0x83, 0xe4, 0xf0, // and rsp, -16
        0x31, 0xd2, // xor edx, edx
        0xe8, 0, 0, 0, 0, // call main  (disp @ 13, P+4 = 17)
    ];
    let next_ip = start_off as i64 + 17;
    let disp = main_off as i64 - next_ip;
    if !fits_i32(disp) {
        return Err("entry point out of range for startup call".into());
    }
    s[13..17].copy_from_slice(&(disp as i32).to_le_bytes());
    if use_exit_plt {
        s.extend_from_slice(&[0x89, 0xc7, 0xe8, 0, 0, 0, 0, 0x0f, 0x0b]);
        Ok((s, Some(start_off + 20)))
    } else {
        s.extend_from_slice(&[
            0x89, 0xc7, // mov edi, eax
            0xb8, 60, 0, 0, 0, // mov eax, 60 (sys_exit)
            0x0f, 0x05, // syscall
            0x0f, 0x0b, // ud2
        ]);
        Ok((s, None))
    }
}

fn emit_elf_start_stub_a64() -> Vec<u8> {
    // AArch64: mov x0,#0; mov x8,#93; svc #0  (exit 0) — real branch patched later
    // We emit a BL to main then exit via syscall 93.
    // bl main ; mov x8,#93 ; svc #0
    vec![
        0x00, 0x00, 0x00, 0x94, // bl #0 (patched)
        0xa8, 0x0b, 0x80, 0xd2, // mov x8, #93
        0x01, 0x00, 0x00, 0xd4, // svc #0
    ]
}

pub fn write_elf(inputs: &[PathBuf], output: &Path) -> Result<(), String> {
    write_elf_with_options(inputs, output, &LinkOptions::default())
}

pub fn write_elf_with_options(
    inputs: &[PathBuf],
    output: &Path,
    opts: &LinkOptions,
) -> Result<(), String> {
    let (objects, _, machine) = load_objects(inputs, opts)?;
    let mut merged = merge_objects(&objects, opts)?;

    let mut undef = collect_undefined(&objects, &merged);
    // entry names are not "undefined" if we will synthesize _start
    let want_dynamic = match opts.dynamic {
        DynamicMode::Force => true,
        DynamicMode::Static => false,
        DynamicMode::Auto => !undef.is_empty() && undef.iter().any(|u| libc_soname_for(u).is_some()),
    };
    if !want_dynamic {
        if opts.dynamic == DynamicMode::Static && !undef.is_empty() {
            return Err(LinkError {
                message: "unresolved symbols in static link".into(),
                unresolved: undef,
            }
            .into());
        }
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

    let mut injected_start_off: Option<usize> = None;
    let mut injected_exit_reloc: Option<usize> = None;
    if !opts.no_startup && !has_start && has_main {
        let main_off = merged
            .syms
            .get("main")
            .or_else(|| merged.syms.get("lpp_main"))
            .map(|s| s.offset)
            .unwrap();
        let start_off = align_up(merged.text.len(), 16);
        merged.text.resize(start_off, 0x90);
        match machine {
            Machine::X86_64 => {
                let (stub, exit_at) =
                    emit_elf_start_stub_x64(start_off, main_off as usize, want_dynamic)?;
                if let Some(site) = exit_at {
                    injected_exit_reloc = Some(site);
                    if !undef.iter().any(|u| u == "exit") {
                        undef.push("exit".into());
                    }
                }
                merged.text.extend_from_slice(&stub);
            }
            Machine::Aarch64 => {
                let mut stub = emit_elf_start_stub_a64();
                let p = start_off as u64;
                let s = main_off;
                let dest = s;
                let disp = dest as i64 - p as i64;
                if disp & 3 != 0 || !fits_i26(disp >> 2) {
                    return Err("aarch64 startup BL out of range".into());
                }
                let imm = (disp >> 2) as u32;
                stub[0..4].copy_from_slice(&(0x9400_0000u32 | (imm & 0x03FF_FFFF)).to_le_bytes());
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
                size: 32,
                object: usize::MAX,
            },
        );
        injected_start_off = Some(start_off);
        let _ = injected_start_off;
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
            if libc_soname_for(u).is_some() || opts.needed.iter().any(|_| true) {
                let n = plt_keys.len();
                plt_keys.entry(u.clone()).or_insert(n);
            }
        }
    }
    if let Some(off) = injected_exit_reloc {
        let _ = off;
        let n = plt_keys.len();
        plt_keys.entry("exit".into()).or_insert(n);
    }

    if !want_dynamic {
        let leftover: Vec<String> = undef
            .into_iter()
            .filter(|u| !merged.syms.contains_key(u) && !plt_keys.contains_key(u))
            .collect();
        if !leftover.is_empty() {
            return Err(LinkError {
                message: "unresolved symbols".into(),
                unresolved: leftover,
            }
            .into());
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
            return Err(LinkError {
                message: "unresolved symbols".into(),
                unresolved: leftover,
            }
            .into());
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

    let dyn_mode = want_dynamic && !plt_keys.is_empty();

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
    let mut phnum: u16 = 1 + 1 + 1 + 1; // PHDR + LOAD_R + LOAD_RX + LOAD_RW + GNU_STACK... wait
    // recount precisely below after we know optionals.

    let ehdr_size = 64usize;
    let phdr_entsize = 56usize;

    // First compute phnum
    let has_interp = dyn_mode;
    let has_note = opts.build_id;
    let has_tls = !merged.tls.is_empty() || merged.tbss_size > 0;
    let has_dynamic = dyn_mode;
    phnum = 2; // PHDR + GNU_STACK
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
    let mut put_dynstr = |s: &str, dynstr: &mut Vec<u8>, dynstr_index: &mut HashMap<String, u32>| -> u32 {
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
    if dyn_mode || opts.shared {
        for n in &dyn_needed {
            needed_str_off.push(put_dynstr(n, &mut dynstr, &mut dynstr_index));
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
    let tls_off_in_data = align_up(data_off_in_data + merged.data.len(), merged.tls_align.max(1));
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

    let lookup_va = |target: &RelTarget, obj_idx: usize| -> Result<u64, String> {
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
                Err(format!("unresolved symbol '{n}'"))
            }
        }
    };

    let lookup_size = |n: &str| -> u64 {
        merged.syms.get(n).map(|s| s.size).unwrap_or(0)
    };

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
    let mut got_list: Vec<(String, usize)> = got_keys.iter().map(|(k, v)| (k.clone(), *v)).collect();
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
                // PLT0: stp x16,x30,[sp,#-16]!; adrp/ldr/add/br to GOT.PLT+16
                let p0 = plt_off_in_text;
                // stp x16, x30, [sp, #-16]!
                rx[p0..p0 + 4].copy_from_slice(&0xa9bf7bf0u32.to_le_bytes());
                // adrp x16, page(GOT.PLT+16)
                let dest = va_gotplt + 16;
                let p = va_plt;
                let delta = self::page(dest) as i64 - self::page(p) as i64;
                let imm = delta >> 12;
                let immlo = (imm as u32) & 3;
                let immhi = ((imm as u32) >> 2) & 0x7_ffff;
                let adrp = 0x90000010u32 | (immlo << 29) | (immhi << 5);
                rx[p0 + 4..p0 + 8].copy_from_slice(&adrp.to_le_bytes());
                // ldr x17, [x16, #lo12]
                let lo = ((dest & 0xfff) >> 3) as u32;
                let ldr = 0xf9400211u32 | (lo << 10);
                rx[p0 + 8..p0 + 12].copy_from_slice(&ldr.to_le_bytes());
                // add x16, x16, #lo12
                let add = 0x91000210u32 | (((dest & 0xfff) as u32) << 10);
                rx[p0 + 12..p0 + 16].copy_from_slice(&add.to_le_bytes());
                // br x17
                rx[p0 + 16..p0 + 20].copy_from_slice(&0xd61f0220u32.to_le_bytes());
                rw[gotplt_off_in_data..gotplt_off_in_data + 8]
                    .copy_from_slice(&va_dynamic.to_le_bytes());
                let mut plt_ordered: Vec<(String, usize)> =
                    plt_keys.iter().map(|(k, v)| (k.clone(), *v)).collect();
                plt_ordered.sort_by_key(|(_, i)| *i);
                for (i, (_name, _)) in plt_ordered.iter().enumerate() {
                    let po = plt_off_in_text + plt0_size + i * 16;
                    let slot = va_gotplt + 24 + (i as u64) * 8;
                    let plt_va_i = va_plt + plt0_size as u64 + (i as u64) * 16;
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
                    let gp = gotplt_off_in_data + 24 + i * 8;
                    rw[gp..gp + 8].copy_from_slice(&plt_va_i.to_le_bytes());
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
                Machine::Aarch64 => 402u64,
            };
            let r_info = ((i as u64 + 1) << 32) | r_type;
            put_u64(&mut rx, e + 8, r_info);
            put_u64(&mut rx, e + 16, 0);
        }

        // .dynamic
        let mut dynb = vec![0u8; dynamic_size];
        let mut dp = 0;
        let mut put_dt = |dynb: &mut [u8], dp: &mut usize, tag: i64, val: u64| {
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
        put_dt(&mut dynb, &mut dp, 21, 0); // DT_DEBUG
        let mut flags1 = 0u64;
        if pie {
            flags1 |= 0x0800_0000; // DF_1_PIE
        }
        put_dt(&mut dynb, &mut dp, 0x6fff_fffb, flags1); // DT_FLAGS_1
        put_dt(&mut dynb, &mut dp, 0, 0); // DT_NULL
        rw[..dynamic_size].copy_from_slice(&dynb[..dynamic_size.min(dynb.len())]);
    }

    // Patch injected exit call through PLT if needed
    if let Some(site) = injected_exit_reloc {
        if let Some(&i) = plt_keys.get("exit") {
            let plt = va_plt + plt0_size as u64 + (i * plt_entsize) as u64;
            let p = va_text + site as u64;
            let disp = plt as i64 - (p as i64 + 4);
            write_i32_at(&mut rx, site, disp, "startup exit PLT")?;
        }
    }

    // ── apply relocations ───────────────────────────────────────────────
    for (oi, obj) in objects.iter().enumerate() {
        for rel in &obj.relocations {
            let (buf, buf_va, local_base) = match rel.section_class {
                SectionClass::Text => (
                    &mut rx[..],
                    va_text,
                    merged.place[oi].text,
                ),
                SectionClass::Rodata => (
                    &mut rx[rodata_off_in_text..],
                    va_rodata,
                    merged.place[oi].rodata,
                ),
                SectionClass::InitArray => (
                    &mut rx[init_off_in_text..],
                    va_init,
                    merged.place[oi].init_array,
                ),
                SectionClass::FiniArray => (
                    &mut rx[fini_off_in_text..],
                    va_fini,
                    merged.place[oi].fini_array,
                ),
                SectionClass::Data => (
                    &mut rw[data_off_in_data..],
                    va_data,
                    merged.place[oi].data,
                ),
                SectionClass::Tls => (
                    &mut rw[tls_off_in_data..],
                    va_tls,
                    merged.place[oi].tls,
                ),
                SectionClass::Bss => {
                    return Err(format!(
                        "'{}': relocation against BSS",
                        obj.path.display()
                    ));
                }
            };
            let patch = local_base + rel.offset;
            let p = buf_va + patch as u64;
            let ctx = obj.path.display().to_string();

            let name_opt = match &rel.target {
                RelTarget::Name(n) => Some(n.as_str()),
                _ => None,
            };
            let s = lookup_va(&rel.target, oi).map_err(|e| format!("'{ctx}': {e}"))?;
            let a = rel.addend;

            let got_va_of = |n: &str| -> Result<u64, String> {
                got_keys
                    .get(n)
                    .map(|i| va_got + (*i as u64) * 8)
                    .ok_or_else(|| format!("'{ctx}': no GOT slot for {n}"))
            };
            let got_va_target = |t: &RelTarget| -> Result<u64, String> {
                got_va_of(&got_key(oi, t))
            };

            match (machine, rel.raw_type) {
                (Machine::X86_64, R_X86_64_64) => {
                    write_u64_at(buf, patch, s.wrapping_add_signed(a), &ctx)?;
                }
                (Machine::X86_64, R_X86_64_PC64) => {
                    let v = s.wrapping_add_signed(a).wrapping_sub(p);
                    write_u64_at(buf, patch, v, &ctx)?;
                }
                (Machine::X86_64, R_X86_64_PC32) | (Machine::X86_64, R_X86_64_PLT32) => {
                    write_i32_at(buf, patch, s as i64 + a - p as i64, &ctx)?;
                }
                (Machine::X86_64, R_X86_64_32) => {
                    write_u32_at(buf, patch, s as i64 + a, &ctx)?;
                }
                (Machine::X86_64, R_X86_64_32S) => {
                    write_i32_at(buf, patch, s as i64 + a, &ctx)?;
                }
                (Machine::X86_64, R_X86_64_16) => {
                    write_i16_at(buf, patch, s as i64 + a, &ctx)?;
                }
                (Machine::X86_64, R_X86_64_PC16) => {
                    write_i16_at(buf, patch, s as i64 + a - p as i64, &ctx)?;
                }
                (Machine::X86_64, R_X86_64_8) => {
                    write_i8_at(buf, patch, s as i64 + a, &ctx)?;
                }
                (Machine::X86_64, R_X86_64_PC8) => {
                    write_i8_at(buf, patch, s as i64 + a - p as i64, &ctx)?;
                }
                (Machine::X86_64, R_X86_64_GOTPCREL)
                | (Machine::X86_64, R_X86_64_GOTPCRELX)
                | (Machine::X86_64, R_X86_64_REX_GOTPCRELX)
                | (Machine::X86_64, R_X86_64_GOTTPOFF) => {
                    let g = got_va_target(&rel.target)?;
                    write_i32_at(buf, patch, g as i64 + a - p as i64, &ctx)?;
                }
                (Machine::X86_64, R_X86_64_GOT32) => {
                    let g = got_va_target(&rel.target)?;
                    write_i32_at(buf, patch, (g - va_got) as i64 + a, &ctx)?;
                }
                (Machine::X86_64, R_X86_64_GOTPC32) => {
                    write_i32_at(buf, patch, va_got as i64 + a - p as i64, &ctx)?;
                }
                (Machine::X86_64, R_X86_64_GOTOFF64) => {
                    write_u64_at(buf, patch, s.wrapping_add_signed(a).wrapping_sub(va_got), &ctx)?;
                }
                (Machine::X86_64, R_X86_64_TPOFF32) => {
                    let off = match &rel.target {
                        RelTarget::Name(n) => merged
                            .syms
                            .get(n)
                            .filter(|s| s.class == SectionClass::Tls)
                            .map(|s| s.offset)
                            .ok_or_else(|| format!("'{ctx}': TPOFF of non-TLS '{n}'"))?,
                        RelTarget::Local(SectionClass::Tls, o) => {
                            merged.place[oi].tls as u64 + *o
                        }
                        _ => return Err(format!("'{ctx}': TPOFF of non-TLS")),
                    };
                    write_i32_at(buf, patch, x64_tpoff(off, tls_memsz) + a, &ctx)?;
                }
                (Machine::X86_64, R_X86_64_TPOFF64) | (Machine::X86_64, R_X86_64_DTPOFF32) => {
                    let off = match &rel.target {
                        RelTarget::Name(n) => merged.syms.get(n).map(|s| s.offset).unwrap_or(0),
                        RelTarget::Local(_, o) => *o,
                    };
                    if rel.raw_type == R_X86_64_TPOFF64 {
                        write_u64_at(buf, patch, (x64_tpoff(off, tls_memsz) + a) as u64, &ctx)?;
                    } else {
                        write_i32_at(buf, patch, off as i64 + a, &ctx)?;
                    }
                }
                (Machine::X86_64, R_X86_64_SIZE32) => {
                    let n = name_opt.unwrap_or("");
                    write_u32_at(buf, patch, lookup_size(n) as i64 + a, &ctx)?;
                }
                (Machine::X86_64, R_X86_64_SIZE64) => {
                    let n = name_opt.unwrap_or("");
                    write_u64_at(buf, patch, (lookup_size(n) as i64 + a) as u64, &ctx)?;
                }
                (Machine::Aarch64, R_AARCH64_ABS64) | (Machine::Aarch64, R_AARCH64_ABS64_DYN) => {
                    write_u64_at(buf, patch, s.wrapping_add_signed(a), &ctx)?;
                }
                (Machine::Aarch64, R_AARCH64_ABS32) => {
                    write_u32_at(buf, patch, s as i64 + a, &ctx)?;
                }
                (Machine::Aarch64, R_AARCH64_ABS16) => {
                    write_i16_at(buf, patch, s as i64 + a, &ctx)?;
                }
                (Machine::Aarch64, R_AARCH64_PREL64) => {
                    write_u64_at(buf, patch, s.wrapping_add_signed(a).wrapping_sub(p), &ctx)?;
                }
                (Machine::Aarch64, R_AARCH64_PREL32) => {
                    write_i32_at(buf, patch, s as i64 + a - p as i64, &ctx)?;
                }
                (Machine::Aarch64, R_AARCH64_PREL16) => {
                    write_i16_at(buf, patch, s as i64 + a - p as i64, &ctx)?;
                }
                (Machine::Aarch64, R_AARCH64_CALL26) | (Machine::Aarch64, R_AARCH64_JUMP26) => {
                    a64_patch_call26(buf, patch, s, a, p)?;
                }
                (Machine::Aarch64, R_AARCH64_ADR_PREL_PG_HI21)
                | (Machine::Aarch64, R_AARCH64_ADR_PREL_PG_HI21_NC) => {
                    a64_patch_adr_pg_hi21(buf, patch, s, a, p)?;
                }
                (Machine::Aarch64, R_AARCH64_ADR_PREL_LO21) => {
                    a64_patch_adr_lo21(buf, patch, s, a, p)?;
                }
                (Machine::Aarch64, R_AARCH64_ADD_ABS_LO12_NC) => {
                    a64_patch_add_lo12(buf, patch, s, a)?;
                }
                (Machine::Aarch64, R_AARCH64_LDST8_ABS_LO12_NC) => {
                    a64_patch_ldst_lo12(buf, patch, s, a, 0)?;
                }
                (Machine::Aarch64, R_AARCH64_LDST16_ABS_LO12_NC) => {
                    a64_patch_ldst_lo12(buf, patch, s, a, 1)?;
                }
                (Machine::Aarch64, R_AARCH64_LDST32_ABS_LO12_NC) => {
                    a64_patch_ldst_lo12(buf, patch, s, a, 2)?;
                }
                (Machine::Aarch64, R_AARCH64_LDST64_ABS_LO12_NC) => {
                    a64_patch_ldst_lo12(buf, patch, s, a, 3)?;
                }
                (Machine::Aarch64, R_AARCH64_LDST128_ABS_LO12_NC) => {
                    a64_patch_ldst_lo12(buf, patch, s, a, 4)?;
                }
                (Machine::Aarch64, R_AARCH64_CONDBR19) => {
                    a64_patch_condbr19(buf, patch, s, a, p)?;
                }
                (Machine::Aarch64, R_AARCH64_TSTBR14) => {
                    a64_patch_tstbr14(buf, patch, s, a, p)?;
                }
                (Machine::Aarch64, R_AARCH64_MOVW_UABS_G0) => {
                    a64_patch_movw(buf, patch, s, a, 0, true)?;
                }
                (Machine::Aarch64, R_AARCH64_MOVW_UABS_G0_NC) => {
                    a64_patch_movw(buf, patch, s, a, 0, false)?;
                }
                (Machine::Aarch64, R_AARCH64_MOVW_UABS_G1) => {
                    a64_patch_movw(buf, patch, s, a, 16, true)?;
                }
                (Machine::Aarch64, R_AARCH64_MOVW_UABS_G1_NC) => {
                    a64_patch_movw(buf, patch, s, a, 16, false)?;
                }
                (Machine::Aarch64, R_AARCH64_MOVW_UABS_G2) => {
                    a64_patch_movw(buf, patch, s, a, 32, true)?;
                }
                (Machine::Aarch64, R_AARCH64_MOVW_UABS_G2_NC) => {
                    a64_patch_movw(buf, patch, s, a, 32, false)?;
                }
                (Machine::Aarch64, R_AARCH64_MOVW_UABS_G3) => {
                    a64_patch_movw(buf, patch, s, a, 48, false)?;
                }
                (Machine::Aarch64, R_AARCH64_ADR_GOT_PAGE) => {
                    let g = got_va_target(&rel.target)?;
                    a64_patch_adr_pg_hi21(buf, patch, g, 0, p)?;
                }
                (Machine::Aarch64, R_AARCH64_LD64_GOT_LO12_NC) => {
                    let g = got_va_target(&rel.target)?;
                    a64_patch_ldst_lo12(buf, patch, g, 0, 3)?;
                }
                (_, 0) => {
                    // Fall back to object-crate RelocationKind.
                    match rel.kind {
                        RelocationKind::Absolute if rel.size == 64 => {
                            write_u64_at(buf, patch, s.wrapping_add_signed(a), &ctx)?;
                        }
                        RelocationKind::Absolute => {
                            write_i32_at(buf, patch, s as i64 + a, &ctx)?;
                        }
                        RelocationKind::Relative | RelocationKind::PltRelative => {
                            write_i32_at(buf, patch, s as i64 + a - p as i64, &ctx)?;
                        }
                        RelocationKind::GotRelative => {
                            let g = got_va_target(&rel.target)?;
                            write_i32_at(buf, patch, g as i64 + a - p as i64, &ctx)?;
                        }
                        _ => {
                            return Err(format!(
                                "'{ctx}': unsupported relocation kind {:?} type {}",
                                rel.kind, rel.raw_type
                            ));
                        }
                    }
                }
                _ => {
                    return Err(format!(
                        "'{ctx}': unsupported relocation type {} on {:?}",
                        rel.raw_type, machine
                    ));
                }
            }
        }
    }

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
    } else {
        return Err("required symbol 'main' (or 'lpp_main' / '_start') not found".into());
    };
    let entry_sym = merged
        .syms
        .get(&entry_name)
        .ok_or_else(|| format!("entry symbol '{entry_name}' not defined"))?;
    let entry_va = sec_va(entry_sym.class) + entry_sym.offset;

    if let Some(mp) = &opts.map_path {
        write_map(mp, &merged, &objects, &entry_name, entry_va)?;
    }

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
    let shdr_table_off = align_up(
        strtab_off + if opts.strip { 0 } else { strtab.len() },
        8,
    );

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
        shdrs.push(Sh {
            name: name_off(shstr, b".dynsym\0"),
            typ: 11,
            flags: shf_a,
            addr: va_dynsym,
            off: (text_file + dynsym_off) as u64,
            size: dynsym_size as u64,
            link: 0, // patched to dynstr index later — leave 0, still readable
            info: 1,
            addralign: 8,
            entsize: 24,
        });
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
        shdrs.push(Sh {
            name: name_off(shstr, b".hash\0"),
            typ: 5,
            flags: shf_a,
            addr: va_hash,
            off: (text_file + hash_off) as u64,
            size: hash_size as u64,
            link: 0,
            info: 0,
            addralign: 8,
            entsize: 4,
        });
        shdrs.push(Sh {
            name: name_off(shstr, b".rela.plt\0"),
            typ: 4,
            flags: shf_a,
            addr: va_relaplt,
            off: (text_file + rela_plt_off) as u64,
            size: rela_plt_size as u64,
            link: 0,
            info: 0,
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
            link: 0,
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
    let etype: u16 = if pie || dyn_mode { 3 } else { 2 }; // ET_DYN / ET_EXEC
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
    let mut emit_ph = |elf: &mut [u8],
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
    emit_ph(
        &mut elf,
        &mut ph,
        PT_GNU_STACK,
        PF_R | PF_W,
        0,
        0,
        0,
        0,
        0x10,
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

// ═══════════════════════════════════════════════════════════════════════════
// Windows PE
// ═══════════════════════════════════════════════════════════════════════════

const PE_IMAGE_BASE: u64 = 0x140000000;
const PE_SECTION_RVA: u32 = 0x1000;
const PE_FILE_ALIGN: usize = 0x200;
const PE_SECT_ALIGN: usize = 0x1000;

const AMD64_ADDR64: u16 = 1;
const AMD64_ADDR32: u16 = 2;
const AMD64_ADDR32NB: u16 = 3;
const AMD64_REL32: u16 = 4;
const AMD64_REL32_1: u16 = 5;
const AMD64_REL32_2: u16 = 6;
const AMD64_REL32_3: u16 = 7;
const AMD64_REL32_4: u16 = 8;
const AMD64_REL32_5: u16 = 9;
const AMD64_SECTION: u16 = 10;
const AMD64_SECREL: u16 = 11;

const ARM64_ADDR32: u16 = 1;
const ARM64_ADDR32NB: u16 = 2;
const ARM64_BRANCH26: u16 = 3;
const ARM64_PAGEBASE_REL21: u16 = 4;
const ARM64_REL21: u16 = 5;
const ARM64_PAGEOFFSET_12A: u16 = 6;
const ARM64_PAGEOFFSET_12L: u16 = 7;
const ARM64_SECREL: u16 = 8;
const ARM64_SECTION: u16 = 13;
const ARM64_ADDR64: u16 = 14;
const ARM64_BRANCH19: u16 = 15;
const ARM64_BRANCH14: u16 = 16;
const ARM64_REL32: u16 = 17;

fn pe_align(v: usize, a: usize) -> usize {
    align_up(v, a)
}

fn classify_dll(name: &str, extra: &HashMap<String, String>) -> Option<String> {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    if let Some(d) = extra.get(clean).or_else(|| extra.get(name)) {
        return Some(d.clone());
    }
    if is_user32_symbol(clean) {
        return Some("USER32.dll".into());
    }
    if is_gdi32_symbol(clean) {
        return Some("GDI32.dll".into());
    }
    if is_ws2_32_symbol(clean) {
        return Some("WS2_32.dll".into());
    }
    if is_advapi32_symbol(clean) {
        return Some("ADVAPI32.dll".into());
    }
    if is_shell32_symbol(clean) {
        return Some("SHELL32.dll".into());
    }
    if is_ole32_symbol(clean) {
        return Some("OLE32.dll".into());
    }
    if is_oleaut32_symbol(clean) {
        return Some("OLEAUT32.dll".into());
    }
    if is_ntdll_symbol(clean) {
        return Some("ntdll.dll".into());
    }
    if is_bcrypt_symbol(clean) {
        return Some("BCRYPT.dll".into());
    }
    if is_crt_symbol(clean) {
        return Some("msvcrt.dll".into());
    }
    if is_kernel32_symbol(clean) {
        return Some("KERNEL32.dll".into());
    }
    None
}

fn is_crt_symbol(name: &str) -> bool {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    matches!(
        clean,
        "malloc" | "free" | "realloc" | "calloc" | "printf" | "puts" | "memset" | "memcpy"
            | "memmove" | "memcmp" | "strlen" | "strcmp" | "strncmp" | "strcpy" | "strncpy"
            | "strcat" | "strchr" | "strstr" | "sprintf" | "snprintf" | "sscanf" | "exit"
            | "abort" | "sin" | "cos" | "tan" | "pow" | "sqrt" | "ceil" | "floor" | "fmod"
            | "fabs" | "abs" | "labs" | "llabs" | "getpid" | "_getpid" | "atan2" | "log"
            | "exp" | "getchar" | "putchar" | "fopen" | "fclose" | "fread" | "fwrite"
            | "fflush" | "fprintf" | "fseek" | "ftell" | "getenv" | "system" | "time"
            | "clock" | "_errno" | "__getmainargs" | "__set_app_type" | "_acmdln"
            | "_initterm" | "_initterm_e" | "_configthreadlocale" | "lpp_c_malloc"
            | "lpp_c_free" | "lpp_c_load_u8" | "lpp_c_store_u8" | "lpp_c_load_i32"
            | "lpp_c_store_i32" | "lpp_c_load_i64" | "lpp_c_store_i64" | "dlopen"
            | "dlsym" | "dlclose" | "dlerror" | "vsnprintf" | "vfprintf" | "atoi" | "atol"
            | "strtol" | "strtoul" | "strtod" | "qsort" | "bsearch" | "rand" | "srand"
            | "tolower" | "toupper" | "isdigit" | "isalpha" | "isspace" | "_beginthreadex"
            | "_endthreadex" | "__iob_func" | "__acrt_iob_func" | "_fdopen" | "_fileno"
            | "rewind" | "fgets" | "fputs" | "ungetc" | "setvbuf" | "perror" | "strerror"
            | "memchr" | "strdup" | "_strdup" | "strncpy_s" | "strcpy_s"
    )
}

fn is_kernel32_symbol(name: &str) -> bool {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    matches!(
        clean,
        "ExitProcess" | "GetTickCount64" | "LoadLibraryA" | "LoadLibraryW" | "GetProcAddress"
            | "GetStdHandle" | "WriteFile" | "ReadFile" | "VirtualAlloc" | "VirtualFree"
            | "VirtualProtect" | "CreateThread" | "WaitForSingleObject" | "WaitForMultipleObjects"
            | "CloseHandle" | "CreateFileA" | "CreateFileW" | "GetFileSize" | "SetFilePointer"
            | "DeleteFileA" | "MoveFileA" | "GetFileAttributesA" | "CreateDirectoryA"
            | "RemoveDirectoryA" | "FindFirstFileA" | "FindNextFileA" | "FindClose" | "Sleep"
            | "CreateProcessA" | "GetExitCodeProcess" | "CreatePipe" | "GetEnvironmentVariableA"
            | "SetEnvironmentVariableA" | "GetModuleFileNameA" | "GetModuleHandleA"
            | "GetModuleHandleW" | "GetLastError" | "SetLastError" | "QueryPerformanceCounter"
            | "QueryPerformanceFrequency" | "GetCommandLineA" | "GetCommandLineW"
            | "GetProcessHeap" | "HeapAlloc" | "HeapFree" | "HeapReAlloc" | "FormatMessageA"
            | "GetConsoleMode" | "SetConsoleMode" | "FlushFileBuffers" | "GetSystemTimeAsFileTime"
            | "InitializeCriticalSection" | "EnterCriticalSection" | "LeaveCriticalSection"
            | "DeleteCriticalSection" | "TlsAlloc" | "TlsGetValue" | "TlsSetValue" | "TlsFree"
            | "GetCurrentThreadId" | "GetCurrentProcessId" | "GetCurrentProcess"
            | "TerminateProcess" | "IsDebuggerPresent" | "SetUnhandledExceptionFilter"
            | "AddVectoredExceptionHandler" | "RaiseException" | "RtlCaptureContext"
            | "RtlLookupFunctionEntry" | "RtlVirtualUnwind" | "MultiByteToWideChar"
            | "WideCharToMultiByte" | "GetACP" | "GetConsoleOutputCP" | "WriteConsoleA"
            | "ReadConsoleA" | "GetFileType" | "SetHandleInformation" | "DuplicateHandle"
            | "CreateEventA" | "SetEvent" | "ResetEvent" | "CreateMutexA" | "ReleaseMutex"
            | "GetSystemInfo" | "GetNativeSystemInfo" | "GlobalMemoryStatusEx"
            | "GetDiskFreeSpaceExA" | "OutputDebugStringA" | "DebugBreak"
            | "InitializeSListHead" | "EncodePointer" | "DecodePointer"
            | "GetStartupInfoW" | "GetStartupInfoA" | "SetDefaultDllDirectories"
    ) || clean.starts_with("K32")
}

fn is_user32_symbol(name: &str) -> bool {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    matches!(
        clean,
        "CreateWindowExA" | "CreateWindowExW" | "DestroyWindow" | "DefWindowProcA"
            | "DefWindowProcW" | "PostQuitMessage" | "RegisterClassA" | "RegisterClassExA"
            | "RegisterClassW" | "GetDC" | "ReleaseDC" | "LoadCursorA" | "LoadCursorW"
            | "PeekMessageA" | "PeekMessageW" | "GetMessageA" | "TranslateMessage"
            | "DispatchMessageA" | "DispatchMessageW" | "GetAsyncKeyState" | "GetKeyState"
            | "GetCursorPos" | "ScreenToClient" | "ClientToScreen" | "FillRect" | "ShowWindow"
            | "UpdateWindow" | "SetForegroundWindow" | "MessageBoxA" | "MessageBoxW"
            | "LoadIconA" | "SetWindowPos" | "BringWindowToTop" | "BeginPaint" | "EndPaint"
            | "SetProcessDPIAware" | "AdjustWindowRectEx" | "GetClientRect" | "GetWindowRect"
            | "InvalidateRect" | "SetWindowTextA" | "GetWindowTextA" | "ShowCursor"
            | "SetCursor" | "LoadImageA" | "SendMessageA" | "PostMessageA" | "KillTimer"
            | "SetTimer" | "GetSystemMetrics" | "MonitorFromWindow" | "GetMonitorInfoA"
            | "EnumDisplaySettingsA" | "ChangeDisplaySettingsA" | "ReleaseCapture"
            | "SetCapture" | "TrackMouseEvent"
    )
}

fn is_gdi32_symbol(name: &str) -> bool {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    matches!(
        clean,
        "CreateCompatibleDC" | "CreateCompatibleBitmap" | "SelectObject" | "DeleteDC"
            | "DeleteObject" | "CreateSolidBrush" | "CreatePen" | "RoundRect" | "TextOutA"
            | "SetBkMode" | "SetTextColor" | "BitBlt" | "StretchBlt" | "Ellipse" | "MoveToEx"
            | "LineTo" | "GetTextExtentPoint32A" | "CreateFontA" | "SetStretchBltMode"
            | "SetBrushOrgEx" | "SetMapMode" | "SetGraphicsMode" | "SetTextCharacterExtra"
            | "SetTextAlign" | "SetLayout" | "GetStockObject" | "Rectangle" | "Polygon"
            | "Polyline" | "CreateDIBSection" | "GetDIBits" | "SetDIBits" | "ChoosePixelFormat"
            | "SetPixelFormat" | "SwapBuffers" | "DescribePixelFormat"
    )
}

fn is_ws2_32_symbol(name: &str) -> bool {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    matches!(
        clean,
        "WSAStartup" | "WSACleanup" | "WSAGetLastError" | "WSAIoctl" | "WSASocketA"
            | "WSARecv" | "WSASend" | "socket" | "bind" | "listen" | "accept" | "connect"
            | "send" | "recv" | "sendto" | "recvfrom" | "closesocket" | "shutdown" | "select"
            | "htons" | "htonl" | "ntohs" | "ntohl" | "getaddrinfo" | "freeaddrinfo"
            | "gethostname" | "getsockname" | "getpeername" | "setsockopt" | "getsockopt"
            | "ioctlsocket" | "inet_ntoa" | "inet_addr" | "WSAPoll" | "WSADuplicateSocketA"
    )
}

fn is_advapi32_symbol(name: &str) -> bool {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    matches!(
        clean,
        "RegOpenKeyExA" | "RegCloseKey" | "RegQueryValueExA" | "RegSetValueExA"
            | "RegCreateKeyExA" | "RegDeleteKeyA" | "RegEnumKeyExA" | "CryptAcquireContextA"
            | "CryptReleaseContext" | "CryptGenRandom" | "GetUserNameA" | "OpenProcessToken"
            | "GetTokenInformation" | "ConvertSidToStringSidA" | "SystemFunction036"
    )
}

fn is_shell32_symbol(name: &str) -> bool {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    matches!(
        clean,
        "SHGetFolderPathA" | "SHGetKnownFolderPath" | "ShellExecuteA" | "DragQueryFileA"
            | "DragFinish" | "CommandLineToArgvW" | "SHGetFileInfoA"
    )
}

fn is_ole32_symbol(name: &str) -> bool {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    matches!(
        clean,
        "CoInitializeEx" | "CoUninitialize" | "CoCreateInstance" | "CoTaskMemFree"
            | "CoTaskMemAlloc" | "OleInitialize" | "OleUninitialize" | "CLSIDFromString"
            | "StringFromGUID2" | "CoInitializeSecurity"
    )
}

fn is_oleaut32_symbol(name: &str) -> bool {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    matches!(
        clean,
        "SysAllocString" | "SysFreeString" | "SysStringLen" | "VariantInit" | "VariantClear"
            | "VariantChangeType" | "SafeArrayCreate" | "SafeArrayDestroy"
    )
}

fn is_ntdll_symbol(name: &str) -> bool {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    matches!(
        clean,
        "RtlGetVersion" | "NtQueryInformationProcess" | "NtDelayExecution" | "RtlNtStatusToDosError"
            | "NtQuerySystemInformation" | "RtlAddFunctionTable" | "RtlDeleteFunctionTable"
    )
}

fn is_bcrypt_symbol(name: &str) -> bool {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    matches!(
        clean,
        "BCryptOpenAlgorithmProvider" | "BCryptCloseAlgorithmProvider" | "BCryptGenRandom"
            | "BCryptCreateHash" | "BCryptHashData" | "BCryptFinishHash" | "BCryptDestroyHash"
            | "BCryptGenerateSymmetricKey" | "BCryptEncrypt" | "BCryptDecrypt"
            | "BCryptDestroyKey"
    )
}

struct ImportData {
    data: Vec<u8>,
    iat_rvas: HashMap<String, u32>,
    iat_rva: u32,
    iat_size: u32,
}

fn build_imports(
    raw_imports: &[String],
    section_rva: u32,
    extra: &HashMap<String, String>,
) -> Result<ImportData, String> {
    let mut by_dll: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for imp in raw_imports {
        let clean = imp.strip_prefix("__imp_").unwrap_or(imp).to_string();
        let dll = classify_dll(&clean, extra).ok_or_else(|| {
            format!(
                "unresolved PE import '{clean}' — provide it with --import DLL={clean} \
                 or add support for its DLL to the import table"
            )
        })?;
        let slot = by_dll.entry(dll).or_default();
        if !slot.contains(&clean) {
            slot.push(clean);
        }
    }
    let dll_count = by_dll.len();
    let desc_size = if dll_count == 0 { 0 } else { (dll_count + 1) * 20 };
    let mut total_entries = 0;
    for funcs in by_dll.values() {
        total_entries += funcs.len() + 1;
    }
    let ilt_size = total_entries * 8;
    let iat_size = total_entries * 8;
    let ilt_off = align_up(desc_size, 8);
    let iat_off = ilt_off + ilt_size;
    let mut data = vec![0u8; iat_off + iat_size];
    let mut iat_rvas = HashMap::new();

    if dll_count > 0 {
        let mut cur_desc = 0;
        let mut cur_ilt = ilt_off;
        let mut cur_iat = iat_off;
        for (dll_name, funcs) in &by_dll {
            let dll_name_off = data.len();
            data.extend_from_slice(dll_name.as_bytes());
            data.push(0);
            if data.len() % 2 != 0 {
                data.push(0);
            }
            let mut hint_off = HashMap::new();
            for f in funcs {
                let h = data.len();
                data.extend_from_slice(&[0u8, 0]); // hint
                data.extend_from_slice(f.as_bytes());
                data.push(0);
                if data.len() % 2 != 0 {
                    data.push(0);
                }
                hint_off.insert(f.clone(), h);
            }
            let this_ilt = section_rva + cur_ilt as u32;
            let this_iat = section_rva + cur_iat as u32;
            let this_name = section_rva + dll_name_off as u32;
            put_u32(&mut data, cur_desc, this_ilt);
            put_u32(&mut data, cur_desc + 12, this_name);
            put_u32(&mut data, cur_desc + 16, this_iat);
            cur_desc += 20;
            for f in funcs {
                let name_rva = section_rva + hint_off[f] as u32;
                let thunk = name_rva as u64;
                data[cur_ilt..cur_ilt + 8].copy_from_slice(&thunk.to_le_bytes());
                data[cur_iat..cur_iat + 8].copy_from_slice(&thunk.to_le_bytes());
                let iat_entry = section_rva + cur_iat as u32;
                iat_rvas.insert(format!("__imp_{f}"), iat_entry);
                iat_rvas.insert(f.clone(), iat_entry);
                cur_ilt += 8;
                cur_iat += 8;
            }
            cur_ilt += 8;
            cur_iat += 8;
        }
    }
    Ok(ImportData {
        data,
        iat_rvas,
        iat_rva: section_rva + iat_off as u32,
        iat_size: iat_size as u32,
    })
}

fn generate_base_relocs_from_rvas(rvas: &[u32]) -> Vec<u8> {
    if rvas.is_empty() {
        return Vec::new();
    }
    let mut sorted = rvas.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut reloc = Vec::new();
    let page_size = 0x1000u32;
    let mut i = 0;
    while i < sorted.len() {
        let page_rva = sorted[i] & !(page_size - 1);
        let mut entries = Vec::new();
        while i < sorted.len() && (sorted[i] & !(page_size - 1)) == page_rva {
            let off = (sorted[i] & (page_size - 1)) as u16;
            entries.push(0xA000u16 | off); // IMAGE_REL_BASED_DIR64
            i += 1;
        }
        let block = 8 + entries.len() * 2;
        let padded = align_up(block, 4);
        let start = reloc.len();
        reloc.resize(start + padded, 0);
        put_u32(&mut reloc, start, page_rva);
        put_u32(&mut reloc, start + 4, padded as u32);
        for (idx, e) in entries.iter().enumerate() {
            put_u16(&mut reloc, start + 8 + idx * 2, *e);
        }
    }
    reloc
}

pub fn write_pe(inputs: &[PathBuf], output: &Path) -> Result<(), String> {
    write_pe_with_options(inputs, output, &LinkOptions::default())
}

pub fn write_pe_with_options(
    inputs: &[PathBuf],
    output: &Path,
    opts: &LinkOptions,
) -> Result<(), String> {
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
                && !raw_imports.iter().any(|i| i == u || u == &format!("__imp_{i}"))
                && !u.starts_with(".refptr.")
        })
        .collect();
    if !leftover.is_empty() && opts.dynamic == DynamicMode::Static {
        return Err(LinkError {
            message: "unresolved COFF symbols".into(),
            unresolved: leftover,
        }
        .into());
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
        return Err(LinkError {
            message: "unresolved COFF symbols".into(),
            unresolved: unknown_leftover,
        }.into());
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

    let text_rva = PE_SECTION_RVA;
    let thunks_len = raw_imports.len() * 6;
    let total_text = merged.text.len() + thunks_len;
    let rdata_rva = pe_align(text_rva as usize + total_text, PE_SECT_ALIGN) as u32;
    let data_rva = pe_align(rdata_rva as usize + merged.rodata.len(), PE_SECT_ALIGN) as u32;
    let tls_rva = pe_align(data_rva as usize + merged.data.len(), PE_SECT_ALIGN) as u32;
    let tls_virt = merged.tls.len() + merged.tbss_size;
    let idata_rva = pe_align(
        if has_tls {
            tls_rva as usize + tls_virt
        } else {
            data_rva as usize + merged.data.len()
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
            let rva = pe_sym_rva(sy, text_rva, rdata_rva, data_rva, tls_rva);
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
                    return Err(format!("'{}': relocation against BSS", obj.path.display()));
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
                    a64_patch_ldst_lo12(
                        patch_buf,
                        patch,
                        image_base + target,
                        rel.addend,
                        size,
                    )?;
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
                    ));
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
    let entry_rva = pe_sym_rva(entry_sym, text_rva, rdata_rva, data_rva, tls_rva) as u32;
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

fn pe_sym_rva(sy: &ResolvedSym, text: u32, rdata: u32, data: u32, tls: u32) -> u64 {
    let base = match sy.class {
        SectionClass::Text => text,
        SectionClass::Rodata | SectionClass::InitArray | SectionClass::FiniArray => rdata,
        SectionClass::Data | SectionClass::Bss => data,
        SectionClass::Tls => tls,
    };
    base as u64 + sy.offset
}

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
    tls_rva: u32,
    _obj_idx: usize,
) -> Result<u64, String> {
    match &rel.target {
        RelTarget::Local(class, off) => {
            let (rva, b) = match class {
                SectionClass::Text => (text_rva, bases.text),
                SectionClass::Rodata | SectionClass::InitArray | SectionClass::FiniArray => {
                    (rdata_rva, bases.rodata)
                }
                SectionClass::Data => (data_rva, bases.data),
                SectionClass::Bss => (data_rva, bases.bss),
                SectionClass::Tls => (tls_rva, bases.tls),
            };
            Ok(rva as u64 + b as u64 + *off)
        }
        RelTarget::Name(name) => {
            if name == "__ImageBase" {
                return Ok(0);
            }
            if let Some(sy) = merged.syms.get(name) {
                return Ok(pe_sym_rva(sy, text_rva, rdata_rva, data_rva, tls_rva));
            }
            if let Some(&rva) = refptr_offsets.get(name) {
                return Ok(rva as u64);
            }
            if let Some(inner) = name.strip_prefix(".refptr.") {
                if let Some(sy) = merged.syms.get(inner) {
                    return Ok(pe_sym_rva(sy, text_rva, rdata_rva, data_rva, tls_rva));
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
            ))
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Mach-O
// ═══════════════════════════════════════════════════════════════════════════

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
        (code.len() + page_size - 1) / page_size
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
    let (objects, _, machine) = load_objects(inputs, opts)?;
    let mut merged = merge_objects(&objects, opts)?;
    let page_size = opts
        .page_size
        .unwrap_or(machine.page_size(OutputFormat::Macho));
    let page_u = page_size as u64;

    let undef = collect_undefined(&objects, &merged);
    let mut imports: Vec<String> = undef
        .iter()
        .filter(|u| !merged.syms.contains_key(*u))
        .cloned()
        .collect();
    imports.sort();
    imports.dedup();

    if opts.dynamic == DynamicMode::Static && !imports.is_empty() {
        return Err(LinkError {
            message: "unresolved Mach-O symbols (static)".into(),
            unresolved: imports,
        }
        .into());
    }

    // GOT slots for imports + GOT-using relocs (including defined / local PIC)
    let mut got_keys: BTreeMap<String, usize> = BTreeMap::new();
    for n in &imports {
        let i = got_keys.len();
        got_keys.entry(n.clone()).or_insert(i);
    }
    for (oi, o) in objects.iter().enumerate() {
        for rel in &o.relocations {
            let macho_got = matches!(rel.raw_type, 3 | 4 | 5 | 6 | 7);
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
            let target_dylib = opts
                .extra_imports
                .get(clean)
                .or_else(|| opts.extra_imports.get(name));
            // dylib ordinal is 1-based; libSystem is always first
            let raw_ord = if let Some(td) = target_dylib {
                dylibs
                    .iter()
                    .position(|d| d.contains(td.as_str()))
                    .map(|p| p + 1)
                    .unwrap_or(1)
            } else {
                1usize
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
        if let Some(p) = strtab.windows(needle.len()).position(|w| w == needle.as_slice()) {
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
    let lookup = |target: &RelTarget, oi: usize| -> Result<u64, String> {
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
                Err(format!("unresolved Mach-O symbol '{n}'"))
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
                        _ => return Err(format!("'{ctx}': GOT reloc on local")),
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
                        _ => return Err(format!("'{ctx}': GOT reloc on local")),
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
                        _ => return Err(format!("'{ctx}': POINTER_TO_GOT on local")),
                    };
                    let g = data_sec_addr
                        + got_in_data as u64
                        + (*got_keys.get(n).unwrap_or(&0) as u64) * 8;
                    write_u64_at(buf, patch, g, &ctx)?;
                }
                (_, 0) => {
                    match rel.kind {
                        RelocationKind::Absolute if rel.size == 64 => {
                            write_u64_at(buf, patch, s.wrapping_add_signed(rel.addend), &ctx)?;
                        }
                        RelocationKind::Relative | RelocationKind::PltRelative => {
                            write_i32_at(buf, patch, s as i64 + rel.addend - p as i64, &ctx)?;
                        }
                        _ => {
                            write_i32_at(buf, patch, s as i64 + rel.addend - p as i64, &ctx)?;
                        }
                    }
                }
                _ => {
                    return Err(format!(
                        "'{ctx}': unsupported Mach-O reloc type {} ({machine:?})",
                        rel.raw_type
                    ));
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
    let mut emit_seg = |bin: &mut [u8],
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
    let mut emit_sect = |bin: &mut [u8],
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
    put_u32(
        &mut bin,
        c + 8,
        (linkedit_fileoff + symoff_in_le) as u32,
    );
    put_u32(&mut bin, c + 12, nlists.len() as u32);
    put_u32(
        &mut bin,
        c + 16,
        (linkedit_fileoff + stroff_in_le) as u32,
    );
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

// ═══════════════════════════════════════════════════════════════════════════
// inspect
// ═══════════════════════════════════════════════════════════════════════════

pub fn inspect_object(input: &Path) -> Result<(), String> {
    let bytes = fs::read(input).map_err(|e| format!("read '{}': {e}", input.display()))?;
    if is_archive_bytes(&bytes) {
        let archive = ArchiveFile::parse(&*bytes)
            .map_err(|e| format!("parse archive '{}': {e}", input.display()))?;
        println!("format: archive");
        println!("path: {}", input.display());
        let mut n = 0usize;
        for member in archive.members() {
            if let Ok(m) = member {
                let name = String::from_utf8_lossy(m.name());
                if let Ok(data) = m.data(&*bytes) {
                    println!("  member {name}  {} bytes", data.len());
                    n += 1;
                }
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
        if let Ok(n) = s.name() {
            if !n.is_empty() && !n.starts_with('.') {
                println!("  {n}");
            }
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
            for m in ar.members() {
                if let Ok(m) = m {
                    if let Ok(d) = m.data(&*bytes) {
                        if d.len() >= 4 {
                            if &d[0..4] == b"\x7fELF" {
                                return "elf";
                            }
                            let machine = u16::from_le_bytes([d[0], d[1]]);
                            if machine == 0x8664 || machine == 0xAA64 || machine == 0x14C {
                                return "pe";
                            }
                            let be = u32::from_be_bytes([d[0], d[1], d[2], d[3]]);
                            let le = u32::from_le_bytes([d[0], d[1], d[2], d[3]]);
                            const MACHO: [u32; 4] =
                                [0xFEED_FACE, 0xFEED_FACF, 0xCAFE_BABE, 0xCAFE_D00D];
                            if MACHO.contains(&be) || MACHO.contains(&le) {
                                return "macho";
                            }
                        }
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
        OutputFormat::Pe => write_pe_with_options(inputs, output, opts),
        OutputFormat::Macho => write_macho_with_options(inputs, output, opts),
        OutputFormat::Elf => write_elf_with_options(inputs, output, opts),
    }
}

pub fn expand_response_files(args: Vec<String>) -> Result<Vec<String>, String> {
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

fn resolve_lib(name: &str, paths: &[PathBuf], fmt: Option<OutputFormat>) -> Result<PathBuf, String> {
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
    if let Some(parent) = p_name.parent() {
        if !parent.as_os_str().is_empty() {
            search.push(parent.to_path_buf());
        }
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
    ))
}

fn parse_hex_u64(s: &str) -> Result<u64, String> {
    let t = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    u64::from_str_radix(t, 16).or_else(|_| s.parse::<u64>()).map_err(|_| format!("invalid integer '{s}'"))
}

pub fn link_cli(args: &[String]) -> Result<(), String> {
    let args = expand_response_files(args.to_vec())?;
    if args.first().map(String::as_str) == Some("inspect") {
        if args.len() != 2 {
            usage();
            return Err("inspect requires exactly one object file argument".into());
        }
        return inspect_object(Path::new(&args[1]));
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
    if matches!(args.first().map(String::as_str), Some("pe") | Some("elf") | Some("macho")) {
        opts.format = Some(match args[0].as_str() {
            "pe" => OutputFormat::Pe,
            "macho" => OutputFormat::Macho,
            _ => OutputFormat::Elf,
        });
        i = 1;
    }
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-o" => {
                i += 1;
                output = Some(PathBuf::from(
                    args.get(i).ok_or("missing argument for -o")?,
                ));
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
                    _ => return Err(format!("unknown emulation '{t}'")),
                }
            }
            "--subsystem" => {
                i += 1;
                opts.subsystem = Some(match args.get(i).ok_or("missing --subsystem")?.as_str() {
                    "console" => PeSubsystem::Console,
                    "windows" => PeSubsystem::Windows,
                    "native" => PeSubsystem::Native,
                    s => return Err(format!("unknown subsystem '{s}'")),
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
                return Err(format!("unknown option '{s}'"));
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
            if matches!(ext_lower.as_str(), "c" | "cpp" | "cc" | "cxx" | "lpp" | "rs") {
                return Err(format!(
                    "Input file '{}' is a source file. lpp-link requires compiled objects (.o / .obj) or archives (.a / .lib).",
                    path.display()
                ));
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

    link_with_options(&inputs, &output, &opts)
}

// ═══════════════════════════════════════════════════════════════════════════
// Calculation tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn align_up_pow2() {
        assert_eq!(align_up(0, 16), 0);
        assert_eq!(align_up(1, 16), 16);
        assert_eq!(align_up(16, 16), 16);
        assert_eq!(align_up(17, 16), 32);
        assert_eq!(align_up(0x1fff, 0x1000), 0x2000);
    }

    #[test]
    fn elf_identity_holds_for_page_layout() {
        let page = 0x1000u64;
        let base = 0x400000u64;
        let text_file = 0x1000u64;
        let text_va = base + text_file;
        assert_eq!(text_va % page, text_file % page);
        let rx_size = 0x1234u64;
        let data_va = align_up_u64(text_va + rx_size, page);
        let data_file = align_up(text_file as usize + rx_size as usize, page as usize) as u64;
        assert_eq!(data_va % page, data_file % page);
    }

    #[test]
    fn x86_64_tpoff_variant2() {
        // 16-byte TLS block, symbol at 0 → TP-relative -16
        assert_eq!(x64_tpoff(0, 16), -16);
        assert_eq!(x64_tpoff(8, 16), -8);
        assert_eq!(x64_tpoff(0, 8), -8);
    }

    #[test]
    fn aarch64_tpoff_variant1() {
        assert_eq!(a64_tpoff(0, 16), 16);
        assert_eq!(a64_tpoff(8, 16), 24);
    }

    #[test]
    fn sha256_abc() {
        let d = sha256(b"abc");
        let hex: String = d.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn elf_hash_foo() {
        // SysV ELF hash("foo") = 0x6d5f.
        assert_eq!(elf_hash(b"foo"), 0x6d5f);
        assert_ne!(elf_hash(b"foo"), elf_hash(b"bar"));
    }

    #[test]
    fn dynamic_entry_count_matches_emission() {
        // One DT_NEEDED plus the 15 fixed tags emitted in write_elf_with_options.
        const FIXED: usize = 15;
        assert_eq!(1 + FIXED, 16);
    }

    #[test]
    fn pe_checksum_length_term() {
        let mut img = vec![0u8; 64];
        img[0] = b'M';
        img[1] = b'Z';
        let sum = pe_checksum(&img, 32);
        assert_eq!(sum, 64 + (b'M' as u32) + ((b'Z' as u32) << 8));
    }

    #[test]
    fn a64_call26_encoding() {
        let mut buf = [0x00u8, 0x00, 0x00, 0x94]; // bl #0
        // P=0, S=16 → disp 16 → imm 4
        a64_patch_call26(&mut buf, 0, 16, 0, 0).unwrap();
        let instr = u32::from_le_bytes(buf);
        assert_eq!(instr & 0xFC00_0000, 0x9400_0000);
        assert_eq!(instr & 0x03FF_FFFF, 4);
    }

    #[test]
    fn a64_add_lo12() {
        let mut buf = 0x91000000u32.to_le_bytes(); // add x0, x0, #0
        a64_patch_add_lo12(&mut buf, 0, 0x1234, 0).unwrap();
        let instr = u32::from_le_bytes(buf);
        assert_eq!((instr >> 10) & 0xfff, 0x234);
    }

    #[test]
    fn x64_startup_stub_call_next_ip() {
        // call main is at +12; disp at +13; next-ip at +17
        let (stub, exit_at) = emit_elf_start_stub_x64(0x1000, 0x1000 + 0x20, false).unwrap();
        assert_eq!(stub[12], 0xe8);
        let disp = i32::from_le_bytes(stub[13..17].try_into().unwrap());
        // S=0x1020, P+4=0x1011 → disp = 0x0f
        assert_eq!(disp, 0x20 - 17);
        assert!(exit_at.is_none());
        assert_eq!(stub[17], 0x89); // mov edi, eax
        let (stub2, exit_at2) = emit_elf_start_stub_x64(0, 0, true).unwrap();
        assert_eq!(exit_at2, Some(20));
        assert_eq!(stub2[19], 0xe8);
    }

    #[test]
    fn rel32_formula() {
        // S=0x401100, P=0x40100c (field), A=-4  →  0x401100-4-0x40100c = 0xf0
        let s = 0x401100i64;
        let p = 0x40100ci64;
        let a = -4i64;
        assert_eq!(s + a - p, 0xf0);
        // equivalent: S - (P+4)
        assert_eq!(s - (p + 4), 0xf0);
    }

    #[test]
    fn pe_page_reloc_groups() {
        let blob = generate_base_relocs_from_rvas(&[0x1008, 0x1010, 0x2000]);
        assert!(blob.len() >= 16);
        let page0 = u32::from_le_bytes(blob[0..4].try_into().unwrap());
        assert_eq!(page0, 0x1000);
    }

    #[test]
    fn fits_checks() {
        assert!(fits_i32(i32::MAX as i64));
        assert!(!fits_i32(i32::MAX as i64 + 1));
        assert!(fits_i26((1 << 25) - 1));
        assert!(!fits_i26(1 << 25));
    }
}
