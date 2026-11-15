//! Comprehensive v0.3 production test suite for `lpp-codegen-llvm`.
//!
//! Covers:
//! - LLVM IR emission (`emit_llvm_ir`) + `clang` object compilation across
//!   `OptLevel::{None, O1, O2, O3, O4, Os, Oz}` and targets
//!   (`X86_64`, `Aarch64`, `X86_64None`, `Aarch64None`)
//! - Debug information (`debug_info = true`) producing valid ELF DWARF v4
//!   sections (`.debug_info`, `.debug_line`, `.debug_abbrev`, `.debug_str`) on Linux/ELF
//!   from LLVM `!llvm.dbg.cu`, `!DIFile`, `!DISubprogram`, `!DILocation` metadata,
//!   plus Windows COFF CodeView vs ELF DWARF target rules
//! - Freestanding kernel IR & custom ELF sections (`@export_name`, `@section`,
//!   `@interrupt`, `@naked`, `port_*`, `mmio_read*` -> `load volatile`,
//!   `mmio_write*` -> `store volatile`, `ptr_*`, `cpu_*`, `rdtsc`, CR/MSR)
//! - ARC lifecycle (`lpp_arc_alloc_with_destructor`, `lpp_arc_retain`, `lpp_arc_release`),
//!   cycle-collector trace tables (`lpp_trace_*`, `lpp_arc_mark_collectible`),
//!   closures, `async`/`await`, `spawn`, and materialized function values
//! - Tuples 2..=16, f-strings, slices, ADTs & exhaustive pattern matching,
//!   `Set` & `Deque`, `VectorI64x2` (`<2 x i64>`) SIMD, external C FFI, and E5xxx rejections

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use lpp_codegen_api::{
    Backend, CodegenError, CodegenErrorKind, CodegenOptions, CompiledModule, DebugSourceResolver,
    NameResolver, OptLevel, Target,
};
use lpp_codegen_llvm::{LlvmBackend, emit_llvm_ir};
use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, HirPackage, PackageSpec,
    ResolutionMode, StringInterner, Symbol, lower_package,
};
use lpp_mir::{InterpreterLimits, MirFunctionId, MirProgram, build_mir, execute_mir_arc};
use lpp_ownership::compute_ownership_plan;
use lpp_types::{ShadowInferenceOptions, TypeInterner, infer_hir_package};

static TEMP_SEQ: AtomicUsize = AtomicUsize::new(0);

// ── Test Harness Scaffolding ───────────────────────────────────────────────

#[derive(Debug)]
struct MemoryFileSystem {
    files: BTreeMap<PathBuf, String>,
}

impl MemoryFileSystem {
    fn single(source: &str, root: &str) -> Self {
        Self::multi(&[(&format!("{root}/main.lpp"), source)])
    }

    fn multi(entries: &[(&str, &str)]) -> Self {
        let mut files = BTreeMap::new();
        for (path, src) in entries {
            files.insert(PathBuf::from(path), (*src).to_owned());
        }
        Self { files }
    }
}

impl FileSystem for MemoryFileSystem {
    fn is_file(&self, path: &Path) -> Result<bool, FileSystemError> {
        Ok(self.files.contains_key(path))
    }

    fn canonicalize(&self, path: &Path) -> Result<PathBuf, FileSystemError> {
        if self.files.contains_key(path) || self.files.keys().any(|f| f.starts_with(path)) {
            Ok(path.to_owned())
        } else {
            Err(FileSystemError::new("canonicalize", path, "path not found"))
        }
    }

    fn read_to_string(&self, path: &Path) -> Result<String, FileSystemError> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| FileSystemError::new("read", path, "path not found"))
    }
}

struct Names<'a>(&'a StringInterner);

impl NameResolver for Names<'_> {
    fn resolve(&self, symbol_raw: u32) -> Option<&str> {
        self.0.resolve(Symbol::from_raw(symbol_raw))
    }
}

struct OriginResolver {
    origins: Vec<Option<(usize, u32, u32)>>,
    files: Vec<(String, String)>,
    entry_slot: Option<usize>,
}

impl DebugSourceResolver for OriginResolver {
    fn resolve_origin(&self, origin_raw: u32) -> Option<(&str, &str, u32, u32)> {
        let (slot, line, col) = (*self.origins.get(origin_raw as usize)?)?;
        let (dir, name) = self.files.get(slot)?;
        Some((dir.as_str(), name.as_str(), line, col))
    }

    fn entry_file(&self) -> Option<(&str, &str)> {
        self.entry_slot
            .and_then(|s| self.files.get(s))
            .or_else(|| self.files.first())
            .map(|(d, n)| (d.as_str(), n.as_str()))
    }
}

struct CompiledPipeline {
    program: MirProgram,
    types: TypeInterner,
    package: HirPackage,
    resolver: OriginResolver,
}

fn pipeline(source: &str) -> CompiledPipeline {
    let fs = MemoryFileSystem::single(source, "/v03llvm");
    pipeline_fs(&fs, "/v03llvm/main.lpp", "v03llvm", "/v03llvm")
}

fn pipeline_fs(
    fs: &MemoryFileSystem,
    entry_path: &str,
    pkg_name: &str,
    pkg_root: &str,
) -> CompiledPipeline {
    let graph = GraphBuilder::new(fs)
        .build(GraphRequest::new(
            entry_path,
            PackageSpec::new(pkg_name, pkg_root),
        ))
        .unwrap_or_else(|e| panic!("graph error: {e:?}"));
    let package = lower_package(&graph, ResolutionMode::Namespaced)
        .unwrap_or_else(|e| panic!("lower: {e:?}"));
    let mut inference = infer_hir_package(&package, ShadowInferenceOptions::default())
        .unwrap_or_else(|e| panic!("typecheck error: {e:?}"));
    let program = build_mir(
        &package,
        &graph.sources,
        &mut inference,
        lpp_mir::MirBuildOptions::default(),
    )
    .unwrap_or_else(|e| panic!("MIR build error: {e:?}"));

    let entry_file_name = Path::new(entry_path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned());
    let mut entry_slot = None;
    let mut files: Vec<(String, String)> = Vec::new();
    let mut file_slots: BTreeMap<u32, usize> = BTreeMap::new();
    let mut origins = Vec::new();
    for (_, origin) in package.origins.enumerate() {
        let span = origin.span;
        let mapped = graph.sources.get(span.file).map(|file| {
            let slot = *file_slots.entry(span.file.raw()).or_insert_with(|| {
                let path = Path::new(file.name());
                let dir = path
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|| ".".to_owned());
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| file.name().to_owned());
                let idx = files.len();
                if entry_file_name.as_deref() == Some(name.as_str()) {
                    entry_slot = Some(idx);
                }
                files.push((dir, name));
                idx
            });
            let (line, col) = file.line_column(span.start).unwrap_or((1, 1));
            (slot, line as u32, col as u32)
        });
        origins.push(mapped);
    }

    CompiledPipeline {
        program,
        types: inference.interner,
        package,
        resolver: OriginResolver {
            origins,
            files,
            entry_slot,
        },
    }
}

fn emit_ir(cp: &CompiledPipeline) -> Result<String, CodegenError> {
    let names = Names(&cp.package.names.symbols);
    emit_llvm_ir(&cp.program, &cp.types, &names)
}

fn compile_target(cp: &CompiledPipeline, target: Target) -> Result<CompiledModule, CodegenError> {
    let names = Names(&cp.package.names.symbols);
    let options = CodegenOptions::new(target, &names);
    LlvmBackend.compile_module(&cp.program, &cp.types, &options)
}

fn compile_with_options(
    cp: &CompiledPipeline,
    target: Target,
    opt_level: OptLevel,
    debug_info: bool,
    debug_sources: Option<&dyn DebugSourceResolver>,
    pgo_fingerprint: Option<u64>,
) -> Result<CompiledModule, CodegenError> {
    let names = Names(&cp.package.names.symbols);
    let mut options = CodegenOptions::new(target, &names)
        .with_opt_level(opt_level)
        .with_debug_info(debug_info)
        .with_debug_sources(debug_sources);
    if let Some(fp) = pgo_fingerprint {
        options = options.with_pgo_fingerprint(fp);
    }
    LlvmBackend.compile_module(&cp.program, &cp.types, &options)
}

fn main_function(cp: &CompiledPipeline) -> MirFunctionId {
    cp.program
        .functions()
        .find(|(_, f)| f.name.and_then(|sym| cp.package.names.symbols.resolve(sym)) == Some("main"))
        .map(|(id, _)| id)
        .expect("program defines main")
}

fn run_arc_oracle(cp: &CompiledPipeline) -> String {
    let entry = main_function(cp);
    let plan = compute_ownership_plan(&cp.program, &cp.types).expect("ownership plan");
    let outcome = execute_mir_arc(
        &cp.program,
        &cp.types,
        entry,
        &[],
        InterpreterLimits::default(),
        &plan.pinned_types(),
    )
    .unwrap_or_else(|e| panic!("ARC oracle failed: {e:?}"));
    assert!(outcome.stats.steps > 0, "expected non-empty execution");
    outcome.output.concat()
}

fn link_and_run_with_libs(object: &[u8], link_libraries: &[String], tag: &str) -> (String, i32) {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "lpp_v03_llvm_{}_{}_{}",
        std::process::id(),
        tag,
        seq
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("module.o");
    let shim_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("lpp-codegen-cranelift")
        .join("tests")
        .join("c_shim.c");
    let bin_path = dir.join("prog");
    std::fs::write(&obj_path, object).unwrap();

    let mut link_cmd = Command::new("cc");
    link_cmd
        .arg(&obj_path)
        .arg(&shim_path)
        .arg("-o")
        .arg(&bin_path)
        .arg("-lpthread")
        .arg("-lm");
    for lib in link_libraries {
        link_cmd.arg(format!("-l{lib}"));
    }
    let link_out = link_cmd.output().expect("invoke cc linker");
    assert!(
        link_out.status.success(),
        "cc link failed for {tag}:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&link_out.stdout),
        String::from_utf8_lossy(&link_out.stderr)
    );

    let run_out = Command::new(&bin_path)
        .output()
        .expect("execute native binary");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8(run_out.stdout).expect("UTF-8 stdout");
    let code = run_out.status.code().unwrap_or(-1);
    (stdout, code)
}

fn assert_llvm_matches_arc_oracle(source: &str, opt_level: OptLevel, tag: &str) -> CompiledModule {
    let cp = pipeline(source);
    let expected = run_arc_oracle(&cp);
    let module = compile_with_options(&cp, Target::host(), opt_level, false, None, None)
        .unwrap_or_else(|e| panic!("LLVM compile failed for {tag} ({opt_level:?}): {e:?}"));
    let (actual, code) = link_and_run_with_libs(&module.object, &module.link_libraries, tag);
    assert_eq!(
        code, 0,
        "native binary returned non-zero code {code} for {tag}"
    );
    assert_eq!(
        actual, expected,
        "LLVM native vs ARC oracle mismatch for {tag}"
    );
    module
}

// ── Pure-Rust ELF64 & DWARF4 Parsers ───────────────────────────────────────

const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
const ET_REL: u16 = 1;
const EM_X86_64: u16 = 62;
const EM_AARCH64: u16 = 183;

const SHT_PROGBITS: u32 = 1;
const SHT_SYMTAB: u32 = 2;
#[allow(dead_code)]
const SHT_STRTAB: u32 = 3;

#[allow(dead_code)]
const SHF_WRITE: u64 = 0x1;
const SHF_ALLOC: u64 = 0x2;
const SHF_EXECINSTR: u64 = 0x4;

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct ElfSection {
    name: String,
    sh_type: u32,
    sh_flags: u64,
    sh_offset: u64,
    sh_size: u64,
    sh_link: u32,
    sh_info: u32,
    sh_addralign: u64,
    data: Vec<u8>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct ElfSymbol {
    name: String,
    bind: u8,
    sym_type: u8,
    shndx: u16,
    value: u64,
    size: u64,
}

struct Elf64View {
    e_type: u16,
    e_machine: u16,
    sections: Vec<ElfSection>,
    symbols: Vec<ElfSymbol>,
}

fn read_u16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(b[off..off + 2].try_into().unwrap())
}
fn read_u32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}
fn read_u64(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().unwrap())
}
fn read_cstr(b: &[u8], off: usize) -> String {
    let end = b[off..]
        .iter()
        .position(|&c| c == 0)
        .map_or(b.len(), |p| off + p);
    String::from_utf8_lossy(&b[off..end]).into_owned()
}

impl Elf64View {
    fn parse(bytes: &[u8]) -> Self {
        assert!(bytes.len() >= 64, "ELF object too short");
        assert_eq!(&bytes[0..4], &ELF_MAGIC, "invalid ELF magic");
        assert_eq!(bytes[4], 2, "expected ELF64");
        assert_eq!(bytes[5], 1, "expected little-endian ELF");

        let e_type = read_u16(bytes, 16);
        let e_machine = read_u16(bytes, 18);
        let e_shoff = read_u64(bytes, 40) as usize;
        let e_shentsize = read_u16(bytes, 58) as usize;
        let e_shnum = read_u16(bytes, 60) as usize;
        let e_shstrndx = read_u16(bytes, 62) as usize;

        let shstr_hdr = e_shoff + e_shstrndx * e_shentsize;
        let shstr_off = read_u64(bytes, shstr_hdr + 24) as usize;
        let shstr_size = read_u64(bytes, shstr_hdr + 32) as usize;
        let shstr = &bytes[shstr_off..shstr_off + shstr_size];

        let mut sections = Vec::with_capacity(e_shnum);
        for i in 0..e_shnum {
            let h = e_shoff + i * e_shentsize;
            let sh_name = read_u32(bytes, h) as usize;
            let sh_type = read_u32(bytes, h + 4);
            let sh_flags = read_u64(bytes, h + 8);
            let sh_offset = read_u64(bytes, h + 24);
            let sh_size = read_u64(bytes, h + 32);
            let sh_link = read_u32(bytes, h + 40);
            let sh_info = read_u32(bytes, h + 44);
            let sh_addralign = read_u64(bytes, h + 48);
            let data = if sh_type == 8 /* SHT_NOBITS */ || sh_size == 0 {
                Vec::new()
            } else {
                let start = sh_offset as usize;
                let end = start + sh_size as usize;
                bytes[start..end].to_vec()
            };
            sections.push(ElfSection {
                name: read_cstr(shstr, sh_name),
                sh_type,
                sh_flags,
                sh_offset,
                sh_size,
                sh_link,
                sh_info,
                sh_addralign,
                data,
            });
        }

        let mut symbols = Vec::new();
        if let Some(symtab) = sections.iter().find(|s| s.sh_type == SHT_SYMTAB) {
            let strtab = &sections[symtab.sh_link as usize].data;
            for entry in symtab.data.as_chunks::<24>().0 {
                let st_name = read_u32(entry, 0) as usize;
                let st_info = entry[4];
                let st_shndx = read_u16(entry, 6);
                let st_value = read_u64(entry, 8);
                let st_size = read_u64(entry, 16);
                symbols.push(ElfSymbol {
                    name: read_cstr(strtab, st_name),
                    bind: st_info >> 4,
                    sym_type: st_info & 0xF,
                    shndx: st_shndx,
                    value: st_value,
                    size: st_size,
                });
            }
        }

        Self {
            e_type,
            e_machine,
            sections,
            symbols,
        }
    }

    fn section(&self, name: &str) -> Option<&ElfSection> {
        self.sections.iter().find(|s| s.name == name)
    }

    fn symbol(&self, name: &str) -> Option<&ElfSymbol> {
        self.symbols.iter().find(|s| s.name == name)
    }
}

// ── Cross-Format Object Inspection Helpers (ELF / Mach-O / COFF) ────────────

fn normalize_sym_name(format: object::BinaryFormat, name: &str) -> &str {
    if format == object::BinaryFormat::MachO {
        name.strip_prefix('_').unwrap_or(name)
    } else {
        name
    }
}

fn obj_section<'a>(obj: &'a object::File<'a>, elf_name: &str) -> Option<object::Section<'a, 'a>> {
    use object::{Object as _, ObjectSection as _};
    let macho_alt = elf_name.strip_prefix('.').map(|s| format!("__{s}"));
    obj.sections().find(|sec| {
        sec.name()
            .ok()
            .is_some_and(|n| n == elf_name || macho_alt.as_deref() == Some(n))
    })
}

fn obj_defined_global_funcs(bytes: &[u8]) -> BTreeSet<String> {
    use object::{Object as _, ObjectSymbol as _, SymbolKind};
    let obj = object::File::parse(bytes).expect("parse object");
    let fmt = obj.format();
    obj.symbols()
        .filter(|sym| sym.is_global() && sym.is_definition() && sym.kind() == SymbolKind::Text)
        .filter_map(|sym| {
            sym.name()
                .ok()
                .map(|n| normalize_sym_name(fmt, n).to_string())
        })
        .collect()
}

fn obj_section_relocs(bytes: &[u8], elf_sec_name: &str) -> Vec<String> {
    use object::{Object as _, ObjectSection as _, ObjectSymbol as _, RelocationTarget};
    let obj = object::File::parse(bytes).expect("parse object");
    let fmt = obj.format();
    let Some(sec) = obj_section(&obj, elf_sec_name) else {
        return Vec::new();
    };
    sec.relocations()
        .filter_map(|(_, reloc)| match reloc.target() {
            RelocationTarget::Symbol(sym_idx) => obj
                .symbol_by_index(sym_idx)
                .ok()
                .and_then(|s| s.name().ok())
                .map(|n| normalize_sym_name(fmt, n).to_string()),
            _ => None,
        })
        .collect()
}

fn read_uleb128(data: &[u8], pos: &mut usize) -> u64 {
    let mut result = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = data[*pos];
        *pos += 1;
        result |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 {
            return result;
        }
        shift += 7;
    }
}

#[derive(Debug)]
struct DwarfLineHeader {
    version: u16,
    dirs: Vec<String>,
    files: Vec<(String, u64)>,
}

fn parse_dwarf_line_header(line_bytes: &[u8]) -> DwarfLineHeader {
    let unit_len = read_u32(line_bytes, 0) as usize;
    assert!(unit_len > 0 && unit_len + 4 <= line_bytes.len());
    let version = read_u16(line_bytes, 4);
    let _header_length = read_u32(line_bytes, 6);
    let mut pos = 10;
    let _min_insn_len = line_bytes[pos];
    pos += 1;
    if version >= 4 {
        let _max_ops_per_insn = line_bytes[pos];
        pos += 1;
    }
    let _default_is_stmt = line_bytes[pos];
    let _line_base = line_bytes[pos + 1] as i8;
    let _line_range = line_bytes[pos + 2];
    let opcode_base = line_bytes[pos + 3];
    pos += 4;
    pos += (opcode_base.saturating_sub(1)) as usize;

    let mut dirs = Vec::new();
    while line_bytes[pos] != 0 {
        let s = read_cstr(line_bytes, pos);
        pos += s.len() + 1;
        dirs.push(s);
    }
    pos += 1;

    let mut files = Vec::new();
    while line_bytes[pos] != 0 {
        let name = read_cstr(line_bytes, pos);
        pos += name.len() + 1;
        let dir_idx = read_uleb128(line_bytes, &mut pos);
        let _mtime = read_uleb128(line_bytes, &mut pos);
        let _size = read_uleb128(line_bytes, &mut pos);
        files.push((name, dir_idx));
    }

    DwarfLineHeader {
        version,
        dirs,
        files,
    }
}

fn extract_cstr_table(debug_str: &[u8]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut pos = 0;
    while pos < debug_str.len() {
        let s = read_cstr(debug_str, pos);
        pos += s.len() + 1;
        if !s.is_empty() {
            out.insert(s);
        }
    }
    out
}

// ── 1. Optimization Levels, Multi-Target & Determinism ─────────────────────

#[test]
fn llvm_opt_levels_o0_through_oz_compile_and_execute_matching_oracle() {
    let source = r#"
struct MatrixCell:
    row: Int
    col: Int
    val: Int

def fibonacci_sum(n: Int) -> Int:
    mut a := 0
    mut b := 1
    mut acc := 0
    for i in 0..n:
        acc = acc + a + (i % 3)
        nxt := a + b
        a = b
        b = nxt
    return acc

def main() -> Int:
    mut cells := [MatrixCell(0, 0, fibonacci_sum(10)), MatrixCell(1, 1, fibonacci_sum(12))]
    mut total := 0
    for c in cells:
        total = total + c.row * 10 + c.col + c.val
    print_int(total)
    return 0
"#;
    let cp = pipeline(source);
    let expected = run_arc_oracle(&cp);

    let mut text_sizes = BTreeMap::new();
    for (idx, opt) in [
        OptLevel::None,
        OptLevel::O1,
        OptLevel::O2,
        OptLevel::O3,
        OptLevel::O4,
        OptLevel::Os,
        OptLevel::Oz,
    ]
    .into_iter()
    .enumerate()
    {
        let m1 = compile_with_options(&cp, Target::host(), opt, false, None, None)
            .unwrap_or_else(|e| panic!("LLVM compile failed for {opt:?}: {e:?}"));
        let m2 = compile_with_options(&cp, Target::host(), opt, false, None, None).unwrap();
        assert!(
            m1.same_object(&m2),
            "{opt:?} compilation must be deterministic"
        );

        use object::{Object as _, ObjectKind, ObjectSection as _, SectionKind};
        let obj = object::File::parse(&*m1.object).expect("parse object");
        assert_eq!(obj.kind(), ObjectKind::Relocatable);
        let text = obj
            .sections()
            .find(|s| s.kind() == SectionKind::Text)
            .expect("text section missing");
        text_sizes.insert(format!("{opt:?}"), text.data().expect("text data").len());

        let (stdout, code) =
            link_and_run_with_libs(&m1.object, &m1.link_libraries, &format!("opt_{idx}"));
        assert_eq!(code, 0);
        assert_eq!(stdout, expected, "opt level {opt:?} drifted from oracle");
    }

    assert!(
        text_sizes["None"] >= text_sizes["Oz"] || text_sizes["O3"] > 0,
        "expected non-zero .text sizes across opt levels: {text_sizes:?}"
    );
}

#[test]
fn llvm_multi_target_elf64_emission_hosted_and_freestanding() {
    let source = r#"
@export_name("boot_compute")
def boot_compute(a: Int, b: Int) -> Int:
    return (a << 3) + (b * 5) - 1

def main() -> Int:
    return boot_compute(4, 6)
"#;
    let cp = pipeline(source);
    let ir = emit_ir(&cp).expect("emit_llvm_ir succeeds");
    let expected_x86_64_triple = if cfg!(target_os = "windows") {
        "x86_64-pc-windows-msvc"
    } else if cfg!(target_os = "macos") {
        "x86_64-apple-darwin"
    } else {
        "x86_64-unknown-linux-gnu"
    };
    assert!(
        ir.contains(&format!("target triple = \"{expected_x86_64_triple}\"")),
        "expected triple {expected_x86_64_triple} in IR"
    );
    assert!(ir.contains("define i64 @boot_compute(i64"));

    let cases = [
        (Target::X86_64, EM_X86_64),
        (Target::X86_64None, EM_X86_64),
        (Target::Aarch64, EM_AARCH64),
        (Target::Aarch64None, EM_AARCH64),
    ];
    for (target, expected_machine) in cases {
        let module = compile_target(&cp, target)
            .unwrap_or_else(|e| panic!("LLVM compile failed for {target:?}: {e:?}"));
        assert_eq!(module.target, target);
        assert!(module.exported_symbols.contains("boot_compute"));

        use object::{Architecture, BinaryFormat, Object as _, ObjectKind};
        let obj = object::File::parse(&*module.object).expect("parse object");
        assert_eq!(obj.kind(), ObjectKind::Relocatable);
        let expected_arch = match expected_machine {
            EM_X86_64 => Architecture::X86_64,
            EM_AARCH64 => Architecture::Aarch64,
            _ => unreachable!(),
        };
        assert_eq!(obj.architecture(), expected_arch);
        if obj.format() == BinaryFormat::Elf {
            let elf = Elf64View::parse(&module.object);
            assert_eq!(elf.e_type, ET_REL);
            assert_eq!(elf.e_machine, expected_machine);
        }
        let globals = obj_defined_global_funcs(&module.object);
        assert!(
            globals.contains("boot_compute"),
            "{target:?} missing boot_compute: {globals:?}"
        );
        assert!(
            globals.contains("main"),
            "{target:?} missing C entry main: {globals:?}"
        );
    }

    let wasm_err = compile_target(&cp, Target::Wasm32Wasi)
        .expect_err("LLVM backend must reject Target::Wasm32Wasi");
    assert_eq!(
        wasm_err.kind,
        CodegenErrorKind::UnsupportedTarget(Target::Wasm32Wasi)
    );
    assert_eq!(wasm_err.code(), "E5002");
}

// ── 2. Debug Metadata (DWARF v4 vs Windows CodeView) ───────────────────────

#[test]
fn llvm_debug_info_true_emits_dwarf4_elf_sections_across_hosted_and_freestanding() {
    let helper_src = "\
pub def multiply_acc(x: Int, y: Int) -> Int:
    prod := x * y
    return prod + 1
";
    let main_src = "\
import helper

def double_it(v: Int) -> Int:
    return v + v

def main() -> Int:
    a := helper.multiply_acc(6, 7)
    b := double_it(a)
    print_int(b)
    return 0
";
    let fs =
        MemoryFileSystem::multi(&[("/dbg/main.lpp", main_src), ("/dbg/helper.lpp", helper_src)]);
    let cp = pipeline_fs(&fs, "/dbg/main.lpp", "dbg", "/dbg");

    let no_dbg = compile_with_options(&cp, Target::X86_64, OptLevel::None, false, None, None)
        .expect("non-debug compile");
    let no_dbg_obj = object::File::parse(&*no_dbg.object).expect("parse no_dbg object");
    assert!(
        obj_section(&no_dbg_obj, ".debug_info").is_none(),
        "debug_info=false must omit .debug_info"
    );

    // On ELF targets (including X86_64None / Aarch64None everywhere, and X86_64 / Aarch64 on non-Windows),
    // `debug_info = true` causes LLVM metadata (`!llvm.dbg.cu`, `!DIFile`, `!DISubprogram`, `!DILocation`)
    // to materialize valid `.debug_info`, `.debug_line`, `.debug_abbrev`, and `.debug_str` DWARF v4 sections.
    let mut dwarf_targets = vec![Target::X86_64None, Target::Aarch64None];
    if !cfg!(target_os = "windows") {
        dwarf_targets.push(Target::X86_64);
        dwarf_targets.push(Target::Aarch64);
    }

    for target in dwarf_targets {
        let dbg_mod =
            compile_with_options(&cp, target, OptLevel::None, true, Some(&cp.resolver), None)
                .unwrap_or_else(|e| panic!("debug compile failed for {target:?}: {e:?}"));

        use object::{BinaryFormat, ObjectSection as _};
        let obj = object::File::parse(&*dbg_mod.object).expect("parse DWARF object");
        let debug_info = obj_section(&obj, ".debug_info")
            .expect(".debug_info missing")
            .data()
            .expect("debug_info data")
            .to_vec();
        let debug_abbrev = obj_section(&obj, ".debug_abbrev")
            .expect(".debug_abbrev missing")
            .data()
            .expect("debug_abbrev data")
            .to_vec();
        let debug_str = obj_section(&obj, ".debug_str")
            .expect(".debug_str missing")
            .data()
            .expect("debug_str data")
            .to_vec();
        let debug_line = obj_section(&obj, ".debug_line")
            .expect(".debug_line missing")
            .data()
            .expect("debug_line data")
            .to_vec();

        if obj.format() == BinaryFormat::Elf {
            let elf = Elf64View::parse(&dbg_mod.object);
            for sec_name in [".debug_info", ".debug_abbrev", ".debug_str", ".debug_line"] {
                let sec = elf.section(sec_name).unwrap();
                assert_eq!(sec.sh_type, SHT_PROGBITS);
                assert_eq!(
                    sec.sh_flags & SHF_ALLOC,
                    0,
                    "{} must not set SHF_ALLOC",
                    sec.name
                );
                assert!(!sec.data.is_empty(), "{} must be non-empty", sec.name);
            }
        } else {
            for data in [&debug_info, &debug_abbrev, &debug_str, &debug_line] {
                assert!(!data.is_empty(), "DWARF section must be non-empty");
            }
        }

        // DWARF compile unit header in `.debug_info` has version >= 4 (u16 at offset 4)
        let cu_version = read_u16(&debug_info, 4);
        assert_eq!(cu_version, 4, "{target:?} expected DWARF v4 CU header");

        let strings = extract_cstr_table(&debug_str);
        assert!(
            strings.contains("lpp 0.3.0"),
            "{target:?} .debug_str missing producer: {strings:?}"
        );
        assert!(
            strings.contains("main.lpp"),
            "{target:?} .debug_str missing main.lpp: {strings:?}"
        );
        assert!(
            strings.contains("double_it") && strings.contains("lpp_main"),
            "{target:?} .debug_str missing subprogram names: {strings:?}"
        );
        assert!(
            strings.iter().any(|s| s.contains("multiply_acc")),
            "{target:?} .debug_str missing helper function name"
        );

        let line_hdr = parse_dwarf_line_header(&debug_line);
        assert!(
            line_hdr.version >= 4,
            "{target:?} expected DWARF >= 4 line table"
        );
        assert!(
            strings.contains("/dbg") || line_hdr.dirs.iter().any(|d| d == "/dbg"),
            "{target:?} DWARF metadata missing /dbg directory: strings={strings:?}, dirs={:?}",
            line_hdr.dirs
        );
        let line_files: BTreeSet<&str> = line_hdr.files.iter().map(|(f, _)| f.as_str()).collect();
        assert!(
            line_files.iter().any(|f| f.ends_with("main.lpp"))
                && line_files.iter().any(|f| f.ends_with("helper.lpp")),
            "{target:?} .debug_line missing multi-file entries: {line_files:?}"
        );
    }

    // Host execution of debug-compiled binary still runs cleanly
    let host_dbg = compile_with_options(
        &cp,
        Target::host(),
        OptLevel::None,
        true,
        Some(&cp.resolver),
        None,
    )
    .unwrap();
    let (stdout, code) = link_and_run_with_libs(&host_dbg.object, &[], "llvm_dbg_host");
    assert_eq!(code, 0);
    assert_eq!(stdout, "86\n");
}

#[test]
fn llvm_debug_info_handles_opt_levels_and_missing_resolver_gracefully() {
    let source = r#"
def step(n: Int) -> Int:
    return n * 3 + 7

def main() -> Int:
    print_int(step(10))
    return 0
"#;
    let cp = pipeline(source);
    for opt in [OptLevel::None, OptLevel::O2, OptLevel::Os] {
        let m = compile_with_options(&cp, Target::host(), opt, true, None, None)
            .unwrap_or_else(|e| panic!("debug+no_resolver failed at {opt:?}: {e:?}"));
        use object::ObjectSection as _;
        let obj = object::File::parse(&*m.object).expect("parse DWARF object");
        let debug_str = obj_section(&obj, ".debug_str").expect(".debug_str missing");
        let strings = extract_cstr_table(debug_str.data().expect("debug_str data"));
        assert!(
            strings.contains("main.lpp") && strings.contains("step"),
            "{opt:?} expected fallback metadata in .debug_str: {strings:?}"
        );
        let (out, code) = link_and_run_with_libs(&m.object, &[], "dbg_fallback");
        assert_eq!(code, 0);
        assert_eq!(out, "37\n");
    }
}

// ── 3. Freestanding Kernel IR, Volatile MMIO, Port I/O & Sections ──────────

#[test]
fn llvm_freestanding_kernel_ir_mmio_volatile_port_io_and_sections() {
    let source = r#"
@repr(C)
struct IdtPointer:
    limit: Int
    base: Int

@packed
struct PackedFrame:
    vec: Int
    err: Int

@align(4096)
struct PageFrame:
    entry: Int

@export_name("_start")
@section(".text.boot")
def kernel_entry() -> Int:
    unsafe:
        p := port_in8(0x3F8)
        q := port_in16(0x64)
        r := port_in32(0xCF8)
        port_out8(0x80, p)
        port_out16(0x80, q)
        port_out32(0xCF8, r)
        mmio_write8(0xB8000, 65)
        mmio_write16(0xB8000, 0x0F41)
        mmio_write32(0xFEE00020, 1)
        mmio_write64(0xFEE00030, 2)
        m8 := mmio_read8(0xB8000)
        m16 := mmio_read16(0xB8000)
        m32 := mmio_read32(0xFEE00020)
        m64 := mmio_read64(0xFEE00030)
        tsc := rdtsc()
        sz := size_of[PageFrame]()
        al := align_of[PageFrame]()
        cr0 := read_cr0()
        write_cr3(cr0)
        cpu_cli()
        return p + q + r + m8 + m16 + m32 + m64 + tsc + sz + al

@interrupt
@section(".text.isr")
def timer_isr(frame_ptr: Int):
    unsafe:
        port_out8(0x20, 0x20)
        cpu_hlt()

@naked
@section(".text.trampoline")
def ap_trampoline():
    unsafe:
        cpu_sti()

@section(".rodata.boot")
def boot_magic() -> Int:
    return 0x55AA

def main() -> Int:
    return kernel_entry()
"#;
    let cp = pipeline(source);
    let ir = emit_ir(&cp).expect("LLVM IR emission for freestanding kernel");

    // Volatile MMIO loads and stores with exact bit widths & alignments
    assert!(
        ir.contains("store volatile i8 "),
        "missing store volatile i8:\n{ir}"
    );
    assert!(
        ir.contains("store volatile i16 "),
        "missing store volatile i16:\n{ir}"
    );
    assert!(
        ir.contains("store volatile i32 "),
        "missing store volatile i32:\n{ir}"
    );
    assert!(
        ir.contains("store volatile i64 "),
        "missing store volatile i64:\n{ir}"
    );
    assert!(
        ir.contains("load volatile i8, ptr "),
        "missing load volatile i8:\n{ir}"
    );
    assert!(
        ir.contains("load volatile i16, ptr "),
        "missing load volatile i16:\n{ir}"
    );
    assert!(
        ir.contains("load volatile i32, ptr "),
        "missing load volatile i32:\n{ir}"
    );
    assert!(
        ir.contains("load volatile i64, ptr "),
        "missing load volatile i64:\n{ir}"
    );

    // Port I/O & arch intrinsic declarations
    for sym in [
        "lpp_arch_port_in8",
        "lpp_arch_port_in16",
        "lpp_arch_port_in32",
        "lpp_arch_port_out8",
        "lpp_arch_port_out16",
        "lpp_arch_port_out32",
        "lpp_arch_rdtsc",
        "lpp_arch_read_cr0",
        "lpp_arch_write_cr3",
        "lpp_arch_cpu_cli",
        "lpp_arch_cpu_hlt",
        "lpp_arch_cpu_sti",
    ] {
        assert!(ir.contains(&format!("@{sym}")), "LLVM IR missing {sym}");
    }

    // Custom section placement attributes on definitions
    assert!(
        ir.contains("@_start() section \".text.boot\""),
        "LLVM IR missing _start .text.boot section:\n{ir}"
    );
    assert!(
        ir.contains("@timer_isr(") && ir.contains("section \".text.isr\""),
        "LLVM IR missing timer_isr .text.isr section"
    );
    assert!(
        ir.contains("@ap_trampoline() section \".text.trampoline\""),
        "LLVM IR missing ap_trampoline .text.trampoline section"
    );
    assert!(
        ir.contains("@boot_magic() section \".rodata.boot\""),
        "LLVM IR missing boot_magic .rodata.boot section"
    );

    // Compile freestanding ELF64 objects for both x86_64-none and aarch64-none
    for (target, expected_machine) in [
        (Target::X86_64None, EM_X86_64),
        (Target::Aarch64None, EM_AARCH64),
    ] {
        let module = compile_target(&cp, target)
            .unwrap_or_else(|e| panic!("freestanding compile failed for {target:?}: {e:?}"));
        let elf = Elf64View::parse(&module.object);
        assert_eq!(elf.e_machine, expected_machine);

        for sec_name in [
            ".text.boot",
            ".text.isr",
            ".text.trampoline",
            ".rodata.boot",
        ] {
            let sec = elf
                .section(sec_name)
                .unwrap_or_else(|| panic!("{target:?} ELF missing custom section {sec_name}"));
            assert_eq!(sec.sh_type, SHT_PROGBITS);
            assert_ne!(sec.sh_flags & SHF_ALLOC, 0);
            assert_ne!(sec.sh_flags & SHF_EXECINSTR, 0);
            assert!(!sec.data.is_empty());
        }

        let start_sym = elf.symbol("_start").expect("_start missing in ELF symtab");
        assert_eq!(elf.sections[start_sym.shndx as usize].name, ".text.boot");
        let isr_sym = elf
            .symbol("timer_isr")
            .expect("timer_isr missing in ELF symtab");
        assert_eq!(elf.sections[isr_sym.shndx as usize].name, ".text.isr");
    }
}

#[test]
fn llvm_pgo_symbol_sanitization_and_external_c_ffi_work_end_to_end() {
    let source = r#"
extern "C" link "m":
    def cos(x: Float) -> Float
    def sin(x: Float) -> Float

extern "C":
    def lpp_test_external_string_length(s: Str) -> Int

def free(x: Int) -> Int:
    return x + 5

@export_name("llvm_ffi_hook")
def hook(x: Int) -> Int:
    return x + 10

def main() -> Int:
    print_float(cos(0.0) + sin(0.0))
    print_int(lpp_test_external_string_length("llvm-ffi"))
    print_int(free(7) + hook(3))
    return 0
"#;
    let cp = pipeline(source);
    let ir = emit_ir(&cp).unwrap();
    assert!(ir.contains("@lpp_fn_free("));
    assert!(ir.contains("@llvm_ffi_hook("));
    assert!(ir.contains("declare double @cos(double)"));

    let module = compile_with_options(
        &cp,
        Target::host(),
        OptLevel::O2,
        false,
        None,
        Some(0xA1B2_C3D4_E5F6_0708),
    )
    .expect("FFI+PGO LLVM compile");
    assert_eq!(module.link_libraries, vec!["m".to_string()]);
    assert!(module.exported_symbols.contains("lpp_fn_free"));
    assert!(module.exported_symbols.contains("llvm_ffi_hook"));
    assert!(!module.exported_symbols.contains("free"));

    let (stdout, code) =
        link_and_run_with_libs(&module.object, &module.link_libraries, "llvm_ffi_pgo");
    assert_eq!(code, 0);
    assert_eq!(stdout, "1.000000\n8\n25\n");
}

// ── 4. ARC Lifecycle, Cycle Collector, Closures & Async/Spawn ──────────────

#[test]
fn llvm_arc_calls_cycle_trace_tables_and_heavy_cycle_collection_match_oracle() {
    let source = r#"
enum MaybeNode:
    None
    Some(node: Node)

struct Node:
    id: Int
    next: MaybeNode

struct Edge:
    t: (TupleLink, Str)

struct TupleLink:
    id: Int
    edge: Edge

def build_struct_cycle():
    keep := [Node(1, MaybeNode.None)]
    mut b := Node(2, MaybeNode.Some(list_get(keep, 0)))
    mut a := list_get(keep, 0)
    a.next = MaybeNode.Some(b)
    print_int(a.id)

def build_closure_cycle():
    mut fns := list_new()
    mut hits := 0
    cb := fn(v: Int) -> Int:
        hits = hits + list_len(fns) + v
        return hits
    list_push(fns, cb)
    f := list_get(fns, 0)
    print_int(f(5))

def build_tuple_cycle():
    mut t1 := TupleLink(1, 0)
    mut t2 := TupleLink(2, 0)
    t1.edge = Edge((t2, "ring"))
    e := t1.edge
    pair := e.t
    mut second := pair.0
    second.edge = Edge((t1, "loop"))
    print_str(pair.1)

def main() -> Int:
    build_struct_cycle()
    build_closure_cycle()
    build_tuple_cycle()
    return 0
"#;
    let cp = pipeline(source);
    let ir = emit_ir(&cp).unwrap();
    for arc_sym in [
        "lpp_arc_alloc_with_destructor",
        "lpp_arc_retain",
        "lpp_arc_release",
        "lpp_arc_mark_collectible",
        "lpp_trace_visit",
        "lpp_arc_collect_cycles",
    ] {
        assert!(
            ir.contains(&format!("@{arc_sym}")),
            "LLVM IR missing {arc_sym}"
        );
    }

    let module = assert_llvm_matches_arc_oracle(source, OptLevel::O2, "llvm_cycles");
    assert!(module.imported_symbols.contains("lpp_arc_mark_collectible"));
    assert!(module.imported_symbols.contains("lpp_trace_visit"));
    assert!(module.imported_symbols.contains("lpp_arc_collect_cycles"));
    assert!(
        module
            .exported_symbols
            .iter()
            .any(|s| s.starts_with("lpp_trace_s"))
    );
    assert!(
        module
            .exported_symbols
            .iter()
            .any(|s| s.starts_with("lpp_trace_e"))
    );
    assert!(
        module
            .exported_symbols
            .iter()
            .any(|s| s.starts_with("lpp_trace_c"))
    );
    assert!(
        module
            .exported_symbols
            .iter()
            .any(|s| s.starts_with("lpp_trace_t"))
    );
}

#[test]
fn llvm_closures_async_await_spawn_and_higher_order_match_oracle() {
    let source = r#"
struct Counter:
    base: Int
    tag: Str

async def compute_async(a: Int, b: Int) -> Str:
    return f"async={a * 10 + b}"

def mul_three(x: Int) -> Int:
    return x * 3

def main() -> Int:
    mut acc := 2
    c := Counter(10, "cnt")
    adder := fn(d: Int) -> Int:
        acc = acc + d
        return acc + c.base

    print_int(adder(5))
    print_int(adder(8))
    print_int(acc)

    ops := [fn(x: Int) -> Int: x + 1, fn(x: Int) -> Int: x * 4]
    f0 := list_get(ops, 0)
    f1 := list_get(ops, 1)
    print_int(f0(20) + f1(7))

    fv := mul_three
    print_int(fv(14))

    t1 := compute_async(3, 7)
    t2 := compute_async(9, 1)
    print_str(t1.await)
    print_str(t2.await)

    out := list_new()
    task_fn := fn():
        list_push(out, 88)
    spawn task_fn
    print_int(list_len(out))
    print_int(list_get(out, 0))
    return 0
"#;
    let module = assert_llvm_matches_arc_oracle(source, OptLevel::O1, "llvm_closures_async");
    assert!(
        module
            .exported_symbols
            .iter()
            .any(|s| s.starts_with("lpp_drop_c"))
    );
    assert!(
        module
            .exported_symbols
            .iter()
            .any(|s| s.starts_with("__lpp_task_thunk"))
    );
    assert!(
        module
            .exported_symbols
            .iter()
            .any(|s| s.starts_with("__lpp_closure_thunk"))
    );
    assert!(module.imported_symbols.contains("lpp_task_new"));
    assert!(module.imported_symbols.contains("lpp_task_await"));
    assert!(module.imported_symbols.contains("lpp_closure_destroy"));
}

// ── 5. Tuples 2..=16, f-Strings, ADTs, Set/Deque, SIMD ────────────────────

#[test]
fn llvm_tuples_2_through_16_and_nested_containers_match_oracle() {
    let source = r#"
def make_nested(n: Int, s: Str) -> (Int, Str, (Int, Str)):
    return (n, s, (n + 100, s + ":ok"))

def main() -> Int:
    t2 := (1, "two")
    t3 := (1, 2, 3)
    t4 := (1, 2, 3, 4)
    t5 := (1, 2, 3, 4, 5)
    t6 := (1, 2, 3, 4, 5, 6)
    t7 := (1, 2, 3, 4, 5, 6, 7)
    t8 := (1, 2, 3, 4, 5, 6, 7, 8)
    t9 := (1, 2, 3, 4, 5, 6, 7, 8, 9)
    t10 := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10)
    t11 := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11)
    t12 := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12)
    t13 := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13)
    t14 := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14)
    t15 := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15)
    t16 := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16)

    print_str(t2.1)
    print_int(t2.0 + t3.2 + t4.3 + t5.4 + t6.5 + t7.6 + t8.7)
    print_int(t9.8 + t10.9 + t11.10 + t12.11 + t13.12 + t14.13 + t15.14 + t16.15)

    mut nested := make_nested(15, "llvm")
    print_int(nested.0)
    print_str(nested.1)
    inner := nested.2
    print_int(inner.0)
    print_str(inner.1)
    nested = make_nested(20, "rebound")
    rebound := nested.2
    print_str(rebound.1)
    return 0
"#;
    let module = assert_llvm_matches_arc_oracle(source, OptLevel::O2, "llvm_tuples_16");
    let tuple_dtors: Vec<&String> = module
        .exported_symbols
        .iter()
        .filter(|s| s.starts_with("lpp_drop_t"))
        .collect();
    assert!(
        tuple_dtors.len() >= 15,
        "expected tuple destructors for arities 2..=16, got {:?}",
        tuple_dtors
    );
}

#[test]
fn llvm_fstrings_slices_and_maps_match_oracle() {
    let source = r#"
struct Product:
    title: Str
    qty: Int

def main() -> Int:
    p := Product("sensor", 4)
    gain := 2.25
    active := true
    msg := f"title={p.title},qty={p.qty},gain={gain},on={active}"
    print_str(msg)

    full := "alpha:beta:gamma"
    sl := str_slice(full, 6, 4)
    print_int(slice_len(sl))
    print_str(str_slice_to_str(sl))
    xs := [10, 20, 30, 40]
    sub := slice(xs, 1, 2)
    print_int(slice_len(sub))
    print_int(slice_get(sub, 1))
    return 0
"#;
    let cp = pipeline(source);
    let ir = emit_ir(&cp).unwrap();
    assert!(ir.contains("@lpp_str_concat"));
    assert!(ir.contains("@lpp_int_to_str"));
    assert!(ir.contains("@lpp_float_to_str"));
    assert!(ir.contains("@lpp_bool_to_str"));
    assert!(ir.contains("@lpp_str_slice"));
    assert_llvm_matches_arc_oracle(source, OptLevel::O2, "llvm_fstrings_slices");

    let map_source = r#"
def main() -> Int:
    mut cfg := map_new()
    map_put(cfg, "k1", 11)
    map_put(cfg, "k2", 29)
    if map_has(cfg, "k1"):
        return map_get(cfg, "k1") + map_get(cfg, "k2")
    return 0
"#;
    let map_cp = pipeline(map_source);
    let map_ir = emit_ir(&map_cp).unwrap();
    assert!(map_ir.contains("@lpp_map_new"));
    assert!(map_ir.contains("@lpp_map_put_str"));
    assert!(map_ir.contains("@lpp_map_has_str"));
    assert!(map_ir.contains("@lpp_map_get_str"));
    let map_mod = compile_target(&map_cp, Target::host()).unwrap();
    assert!(map_mod.imported_symbols.contains("lpp_map_put_str"));
}

#[test]
fn llvm_adts_and_exhaustive_pattern_matching_match_oracle() {
    let source = r#"
enum Packet:
    Empty
    Num(v: Int)
    Pair(a: Int, b: Int)
    Text(s: Str, weight: Int)

def eval_packet(p: Packet) -> Int:
    match p:
        Packet.Empty:
            return 0
        Packet.Num(v) if v < 0:
            return -v * 2
        Packet.Num(v):
            return v + 10
        Packet.Pair(a, b) if a == b:
            return a * 100
        Packet.Pair(a, b):
            return a + b
        Packet.Text(s, w):
            return str_len(s) + w

def bucket(n: Int) -> Str:
    match n:
        -5 | -1:
            return "neg"
        0..=9:
            return "unit"
        10..20:
            return "teen"
        _:
            return "other"

def main() -> Int:
    print_int(eval_packet(Packet.Empty))
    print_int(eval_packet(Packet.Num(-4)))
    print_int(eval_packet(Packet.Num(7)))
    print_int(eval_packet(Packet.Pair(6, 6)))
    print_int(eval_packet(Packet.Pair(6, 9)))
    print_int(eval_packet(Packet.Text("llvm", 5)))
    print_str(bucket(-5))
    print_str(bucket(9))
    print_str(bucket(10))
    print_str(bucket(42))
    return 0
"#;
    let module = assert_llvm_matches_arc_oracle(source, OptLevel::O3, "llvm_adts");
    assert!(
        module
            .exported_symbols
            .iter()
            .any(|s| s.starts_with("lpp_drop_e"))
    );
}

#[test]
fn llvm_set_and_deque_lowering_in_ir_and_cross_target_elf() {
    let source = r#"
def main() -> Int:
    s1 := set_new()
    _a1 := set_add(s1, 10)
    _a2 := set_add_str(s1, "alpha")
    ok_i := set_has(s1, 10)
    ok_s := set_has_str(s1, "alpha")
    _r1 := set_remove(s1, 10)
    _r2 := set_remove_str(s1, "alpha")
    s2 := set_new()
    _b := set_add(s2, 99)
    su := set_union(s1, s2)
    si := set_intersect(s1, s2)
    n := set_len(su) + set_len(si)
    set_clear(s1)
    set_free(s1)
    set_free(s2)
    set_free(su)
    set_free(si)

    dq := deque_new()
    deque_push_back(dq, 11)
    deque_push_front(dq, 22)
    pf := deque_peek_front(dq)
    pb := deque_peek_back(dq)
    g := deque_get(dq, 1)
    p1 := deque_pop_front(dq)
    p2 := deque_pop_back(dq)
    dl := deque_len(dq)
    deque_clear(dq)
    deque_free(dq)

    if ok_i && ok_s:
        return n + pf + pb + g + p1 + p2 + dl
    return 0
"#;
    let cp = pipeline(source);
    let ir = emit_ir(&cp).unwrap();
    for sym in [
        "lpp_set_new",
        "lpp_set_add",
        "lpp_set_has",
        "lpp_set_remove",
        "lpp_set_add_str",
        "lpp_set_has_str",
        "lpp_set_remove_str",
        "lpp_set_len",
        "lpp_set_clear",
        "lpp_set_free",
        "lpp_set_union",
        "lpp_set_intersect",
        "lpp_deque_new",
        "lpp_deque_push_back",
        "lpp_deque_push_front",
        "lpp_deque_pop_back",
        "lpp_deque_pop_front",
        "lpp_deque_peek_front",
        "lpp_deque_peek_back",
        "lpp_deque_get",
        "lpp_deque_len",
        "lpp_deque_clear",
        "lpp_deque_free",
    ] {
        assert!(ir.contains(&format!("@{sym}")), "LLVM IR missing {sym}");
    }

    for target in [Target::X86_64, Target::Aarch64] {
        let m = compile_target(&cp, target).unwrap();
        assert!(m.imported_symbols.contains("lpp_set_new"));
        assert!(m.imported_symbols.contains("lpp_deque_new"));
        let relocs = obj_section_relocs(&m.object, ".text");
        assert!(relocs.iter().any(|r| r == "lpp_set_new"));
        assert!(relocs.iter().any(|r| r == "lpp_deque_push_back"));
    }
}

#[test]
fn llvm_vector_i64x2_simd_ir_and_host_execution() {
    let source = r#"
def simd_kernel(a: Int, b: Int) -> Int:
    v := vec_i64x2(a, b)
    sp := vec_i64x2_splat(4)
    summed := vec_i64x2_add(v, sp)
    diffed := vec_i64x2_sub(summed, vec_i64x2(1, 2))
    prod := vec_i64x2_mul(diffed, vec_i64x2(3, 5))
    ored := vec_i64x2_or(prod, vec_i64x2(1, 2))
    xored := vec_i64x2_xor(ored, vec_i64x2(0, 1))
    anded := vec_i64x2_and(xored, vec_i64x2(0xFFFF, 0xFFFF))
    shr1 := vec_i64x2_shr(anded, 1)
    shr2 := vec_i64x2_shr_var(shr1, vec_i64x2(0, 1))
    l0 := vec_i64x2_extract(shr2, 0)
    l1 := vec_i64x2_extract(shr2, 1)
    return l0 + l1 + vec_i64x2_sum(shr2)

def main() -> Int:
    chk := vec_i64_checksum(16)
    print_int(simd_kernel(20, 30) + chk)
    return 0
"#;
    let cp = pipeline(source);
    let ir = emit_ir(&cp).unwrap();
    for ir_op in [
        "<2 x i64>",
        "add <2 x i64>",
        "sub <2 x i64>",
        "mul <2 x i64>",
        "and <2 x i64>",
        "or <2 x i64>",
        "xor <2 x i64>",
        "ashr <2 x i64>",
        "extractelement <2 x i64>",
        "@lpp_vec_i64_checksum",
    ] {
        assert!(
            ir.contains(ir_op),
            "LLVM IR missing SIMD construct `{ir_op}`"
        );
    }

    for target in [Target::X86_64, Target::Aarch64] {
        let _ = compile_target(&cp, target)
            .unwrap_or_else(|e| panic!("SIMD LLVM compile failed on {target:?}: {e:?}"));
    }

    let module =
        compile_with_options(&cp, Target::host(), OptLevel::O3, false, None, None).unwrap();
    let (stdout, code) = link_and_run_with_libs(&module.object, &[], "llvm_simd");
    assert_eq!(code, 0);
    let val: i64 = stdout.trim().parse().expect("numeric SIMD stdout");
    assert_eq!(val, 508, "unexpected SIMD result");
}

#[test]
fn llvm_negative_rejections_report_e5xxx_codes() {
    let non_const_shift = r#"
def shift_dyn(a: Int, n: Int) -> Int:
    v := vec_i64x2(a, a + 1)
    r := vec_i64x2_shr(v, n)
    return vec_i64x2_extract(r, 0)

def main() -> Int:
    return shift_dyn(16, 1)
"#;
    let cp_s = pipeline(non_const_shift);
    let err_s =
        compile_target(&cp_s, Target::host()).expect_err("non-const vector shift must fail");
    assert_eq!(err_s.code(), "E5001");
    assert!(err_s.function.is_some());

    let mixed_fn = r#"
def plain(x: Int) -> Int:
    return x + 1

def main() -> Int:
    k := 3
    c := fn(x: Int) -> Int:
        return x + k
    xs := [plain, c]
    f := list_get(xs, 0)
    print_int(f(1))
    return 0
"#;
    let cp_m = pipeline(mixed_fn);
    let err_m =
        compile_target(&cp_m, Target::host()).expect_err("mixed function origins must fail");
    assert_eq!(err_m.code(), "E5001");

    let cp_t = pipeline("def main() -> Int:\n    return 0\n");
    let err_t = compile_target(&cp_t, Target::Wasm32Wasi).expect_err("WASM target unsupported");
    assert_eq!(err_t.code(), "E5002");
}

#[test]
fn llvm_frame_pod_uses_alloca_zero_arc_and_owned_pod_passes_null_dtor() {
    let frame_only_src = r#"
struct Point:
    x: Int
    y: Int

def main() -> Int:
    mut total := 0
    for i in 0..100:
        mut p := Point(i, i + 1)
        p.x = p.x + 2
        pair := (p.x, p.y)
        total = total + pair.0 + pair.1
    print_int(total)
    return 0
"#;
    let cp_frame = pipeline(frame_only_src);
    let names_frame = Names(&cp_frame.package.names.symbols);
    let ir_frame = lpp_codegen_llvm::emit_llvm_ir(&cp_frame.program, &cp_frame.types, &names_frame)
        .expect("emit LLVM IR for Frame POD");
    assert!(
        ir_frame.contains("alloca [16 x i8], align 8"),
        "expected stack alloca [16 x i8] for Frame POD struct/tuple in LLVM IR:\n{ir_frame}"
    );
    assert!(
        !ir_frame.contains("call ptr @lpp_arc_alloc_with_destructor")
            && !ir_frame.contains("call void @lpp_arc_release"),
        "Frame-only POD program must emit zero ARC alloc/release calls:\n{ir_frame}"
    );
    assert_llvm_matches_arc_oracle(frame_only_src, OptLevel::O3, "llvm_frame_pod");

    let escaping_pod_src = r#"
struct Vec2:
    x: Int
    y: Int

def make_vec(x: Int, y: Int) -> Vec2:
    return Vec2(x * 2, y * 3)

def make_pair(a: Int, b: Int) -> (Int, Int):
    return (a + 1, b + 2)

def main() -> Int:
    v := make_vec(5, 7)
    p := make_pair(v.x, v.y)
    print_int(p.0 + p.1)
    return 0
"#;
    let cp_esc = pipeline(escaping_pod_src);
    let names_esc = Names(&cp_esc.package.names.symbols);
    let ir_esc = lpp_codegen_llvm::emit_llvm_ir(&cp_esc.program, &cp_esc.types, &names_esc)
        .expect("emit LLVM IR for Owned POD");
    assert!(
        ir_esc.contains("call ptr @lpp_arc_alloc_with_destructor(i64 16, ptr null)"),
        "Owned POD struct/tuple must pass null destructor to skip 8B dtor prefix:\n{ir_esc}"
    );
    assert_llvm_matches_arc_oracle(escaping_pod_src, OptLevel::O3, "llvm_owned_pod");
}
