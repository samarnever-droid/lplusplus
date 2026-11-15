use std::path::PathBuf;
use std::process::Command;

use lpp_driver::{
    BackendChoice, CompileError, CompileOptions, CompilerSession, DriverRequest, RewriteEngine,
    build_executable_configured, runtime_library_path,
};

fn workdir(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("lpp_unsafe_e2e_{}_{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

fn build_and_run(name: &str, source: &str) -> String {
    let dir = workdir(name);
    let entry = dir.join("main.lpp");
    let exe = dir.join(format!("main{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&entry, source).unwrap();

    let runtime = runtime_library_path().expect("test runtime library is built");
    build_executable_configured(
        &entry,
        "unsafe_e2e",
        BackendChoice::Cranelift,
        CompileOptions::for_backend(BackendChoice::Cranelift),
        &exe,
        runtime.parent().unwrap(),
        Some("cc"),
    )
    .expect("compilation and linking should succeed");

    let output = Command::new(&exe).output().unwrap();
    assert!(
        output.status.success(),
        "binary failed (exit {:?}): stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn compile_expect_err(name: &str, source: &str, options: CompileOptions) -> CompileError {
    let dir = workdir(name);
    let entry = dir.join("main.lpp");
    std::fs::write(&entry, source).unwrap();
    lpp_driver::compile_entry_with_options(&entry, "unsafe_neg", BackendChoice::Cranelift, options)
        .expect_err("compilation should be rejected")
}

#[test]
fn unsafe_ptr_mmio_intrinsics_and_kernel_attributes_execute_end_to_end() {
    let source = r#"
extern "C":
    def malloc(size: USize) -> RawPtr
    def free(ptr: RawPtr)

@repr(C)
@packed
struct PackedPair:
    tag: U8
    val: U32

@repr(C)
@align(16)
struct AlignedPageInfo:
    base: U64
    flags: U16

@export_name("lpp_custom_helper_sym")
def custom_exported(x: Int) -> Int:
    return x + 7

unsafe def write_quad(p: Ptr[U64], idx: Int, val: U64):
    slot := ptr_offset[U64](p, idx)
    ptr_write[U64](slot, val)

unsafe def read_quad(p: Ptr[U64], idx: Int) -> U64:
    slot := ptr_offset[U64](p, idx)
    return ptr_read[U64](slot)

def main():
    print(size_of[U64]())
    print(align_of[U64]())
    print(size_of[PackedPair]())
    print(align_of[PackedPair]())
    print(align_of[AlignedPageInfo]())

    unsafe:
        n: USize = 64 as USize
        raw := malloc(n)
        null_p := ptr_null[U64]()
        print_bool(ptr_is_null(null_p))
        p := raw as Ptr[U64]
        print_bool(ptr_is_null(p))

        write_quad(p, 0, 12345 as U64)
        write_quad(p, 1, 99999 as U64)
        v0 := read_quad(p, 0)
        v1 := read_quad(p, 1)
        print(v0 as Int)
        print(v1 as Int)

        # Byte-level memcpy/memset/memcmp/mmio tests using a second buffer
        raw2 := malloc(n)
        b1 := raw as Ptr[U8]
        b2 := raw2 as Ptr[U8]
        memset_raw(b1, 0 as U8, 32 as USize)
        memset_raw(b2, 0 as U8, 32 as USize)
        mmio_write32(raw as USize, 0x12345678 as U32)
        memory_barrier()
        got := mmio_read32(raw as USize)
        print(got as Int)

        memcpy_raw(b2, b1, 16 as USize)
        cmp := memcmp_raw(b1, b2, 16 as USize)
        print(cmp)

        free(raw)
        free(raw2)

    print(custom_exported(35))
"#;
    let out = build_and_run("ptr_mmio_all", source);
    assert_eq!(
        out,
        "8\n8\n5\n1\n16\n1\n0\n12345\n99999\n305419896\n0\n42\n"
    );
}

#[test]
fn safe_context_rejects_unsafe_ops_with_e3201_and_invalid_cast_with_e3202() {
    let err_ptr = compile_expect_err(
        "reject_ptr_in_safe",
        r#"
def main():
    p := ptr_null[U64]()
    x := ptr_read[U64](p)
    print(x as Int)
"#,
        CompileOptions::for_backend(BackendChoice::Cranelift),
    );
    let msg = format!("{err_ptr}");
    assert!(msg.contains("[E3201]"), "expected [E3201], got: {msg}");

    let err_unsafe_fn = compile_expect_err(
        "reject_unsafe_fn_call_in_safe",
        r#"
unsafe def danger() -> Int:
    return 1

def main():
    print(danger())
"#,
        CompileOptions::for_backend(BackendChoice::Cranelift),
    );
    assert!(format!("{err_unsafe_fn}").contains("[E3201]"));

    let err_cast = compile_expect_err(
        "reject_str_to_int_cast",
        r#"
def main():
    unsafe:
        x := "hello" as Int
        print(x)
"#,
        CompileOptions::for_backend(BackendChoice::Cranelift),
    );
    let cast_msg = format!("{err_cast}");
    assert!(
        cast_msg.contains("[E3202]"),
        "expected [E3202], got: {cast_msg}"
    );
}

#[test]
fn deny_unsafe_rejects_unsafe_code_with_e4501_and_allows_safe_code() {
    let mut deny_opts = CompileOptions::for_backend(BackendChoice::Cranelift);
    deny_opts.deny_unsafe = true;

    let err = compile_expect_err(
        "deny_unsafe_blocks_unsafe",
        r#"
def main():
    unsafe:
        x := 10 as U64
        print(x as Int)
"#,
        deny_opts.clone(),
    );
    let msg = format!("{err}");
    assert!(msg.contains("[E4501]"), "expected [E4501], got: {msg}");

    // Safe program passes with deny_unsafe: true
    let dir = workdir("deny_unsafe_allows_safe");
    let entry = dir.join("main.lpp");
    std::fs::write(&entry, "def main():\n    print(42)\n").unwrap();
    lpp_driver::compile_entry_with_options(&entry, "safe_ok", BackendChoice::Cranelift, deny_opts)
        .expect("safe program should pass with --deny-unsafe");

    // Verify CLI flag --deny-unsafe returns non-zero exit on unsafe code
    let cli_dir = workdir("deny_unsafe_cli");
    let cli_entry = cli_dir.join("main.lpp");
    std::fs::write(&cli_entry, "def main():\n    unsafe:\n        print(1)\n").unwrap();
    let request = DriverRequest::from_args(
        [
            "lpp".to_string(),
            cli_entry.to_string_lossy().into_owned(),
            "--check".to_string(),
            "--deny-unsafe".to_string(),
        ],
        &cli_dir,
    )
    .unwrap();
    let engine = RewriteEngine;
    let code = CompilerSession::new(engine).execute(&request).exit_code();
    assert_ne!(code, 0, "--deny-unsafe CLI must reject unsafe program");
}

#[test]
fn bare_metal_x86_64_and_aarch64_kernels_compile_and_link_static_elf() {
    use lpp_driver::build_freestanding_kernel;

    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let kernel_src = repo_root.join("examples/kernel_x86_64/src/main.lpp");
    assert!(
        kernel_src.is_file(),
        "example kernel must exist at {}",
        kernel_src.display()
    );

    let dir = workdir("bare_metal_kernel");
    let kernel_elf = dir.join("kernel_x86_64.elf");

    let mut x64_opts = CompileOptions::for_backend(BackendChoice::Cranelift);
    x64_opts.target = lpp_driver::Target::X86_64None;
    x64_opts.image_base = Some(0x0010_0000);
    x64_opts.multiboot2 = true;

    let module = build_freestanding_kernel(&kernel_src, "lpp_kernel", x64_opts, &kernel_elf)
        .expect("bare-metal x86_64 kernel must compile and statically link");
    assert_eq!(module.entry.as_deref(), Some("_start"));

    let elf_bytes = std::fs::read(&kernel_elf).unwrap();
    assert_eq!(&elf_bytes[0..4], b"\x7fELF");
    // Multiboot2 magic (0xE85250D6 little-endian: d6 50 52 e8) must sit within first 32 KiB
    let mb2_magic = 0xE852_50D6_u32.to_le_bytes();
    let search_limit = elf_bytes.len().min(32 * 1024);
    assert!(
        elf_bytes[..search_limit].windows(4).any(|w| w == mb2_magic),
        "Multiboot2 header magic must appear in first 32 KiB of kernel ELF"
    );

    // Verify with readelf & objdump if present on Linux CI/host
    if let Ok(readelf) = Command::new("readelf")
        .args(["-h", "-l"])
        .arg(&kernel_elf)
        .output()
        && readelf.status.success()
    {
        let text = String::from_utf8_lossy(&readelf.stdout);
        assert!(
            text.contains("EXEC (Executable file)"),
            "kernel must be static ET_EXEC ELF, got:\n{text}"
        );
        assert!(
            !text.contains("INTERP"),
            "freestanding kernel must have zero PT_INTERP dynamic loader:\n{text}"
        );
    }
    if let Ok(objdump) = Command::new("objdump")
        .args(["-d"])
        .arg(&kernel_elf)
        .output()
        && objdump.status.success()
    {
        let disasm = String::from_utf8_lossy(&objdump.stdout);
        for expected in ["cli", "hlt", "lgdt", "rdtsc"] {
            assert!(
                disasm.contains(expected),
                "disassembly missing `{expected}` instruction:\n{disasm}"
            );
        }
    }

    // Verify CLI `--target x86_64-unknown-none` works end-to-end
    let cli_elf = dir.join("cli_kernel.elf");
    let request = DriverRequest::from_args(
        [
            "lpp".to_string(),
            kernel_src.to_string_lossy().into_owned(),
            "--target".to_string(),
            "x86_64-unknown-none".to_string(),
            "--multiboot2".to_string(),
            "--image-base".to_string(),
            "0x100000".to_string(),
            "-o".to_string(),
            cli_elf.to_string_lossy().into_owned(),
        ],
        &dir,
    )
    .unwrap();
    let exit = CompilerSession::new(RewriteEngine)
        .execute(&request)
        .exit_code();
    assert_eq!(exit, 0, "CLI bare-metal build must succeed");
    assert!(cli_elf.is_file());

    // Also verify aarch64-unknown-none freestanding ELF generation
    let a64_src = dir.join("kernel_a64.lpp");
    std::fs::write(
        &a64_src,
        r#"
@export_name("_start")
unsafe def a64_kmain():
    cpu_cli()
    mmio_write32(0x09000000 as USize, 0x41 as U32)
    memory_barrier()
    while true:
        cpu_hlt()
"#,
    )
    .unwrap();
    let a64_elf = dir.join("kernel_a64.elf");
    let mut a64_opts = CompileOptions::for_backend(BackendChoice::Cranelift);
    a64_opts.target = lpp_driver::Target::Aarch64None;
    build_freestanding_kernel(&a64_src, "a64_kernel", a64_opts, &a64_elf)
        .expect("bare-metal aarch64 kernel must compile and statically link");
    let a64_bytes = std::fs::read(&a64_elf).unwrap();
    assert_eq!(&a64_bytes[0..4], b"\x7fELF");
    assert_eq!(u16::from_le_bytes([a64_bytes[18], a64_bytes[19]]), 183); // EM_AARCH64 = 183
}
