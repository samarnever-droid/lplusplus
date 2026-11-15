use std::path::PathBuf;
use std::process::Command;

use lpp_common::OptimizationLevel;
use lpp_driver::{
    BackendChoice, CompileOptions, CompilerSession, DriverRequest, RewriteEngine, Target,
    build_executable_configured, build_freestanding_kernel_with_backend, runtime_library_path,
};

fn workdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lpp_llvm_e2e_{}_{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&directory_path(&dir)).unwrap();
    dir
}

fn directory_path(p: &PathBuf) -> PathBuf {
    p.clone()
}

fn run_on_backend(
    name: &str,
    source: &str,
    backend: BackendChoice,
    opt: OptimizationLevel,
) -> String {
    let tag = match (backend, opt) {
        (BackendChoice::Cranelift, _) => "cranelift",
        (BackendChoice::Llvm, OptimizationLevel::O0) => "llvm_o0",
        (BackendChoice::Llvm, OptimizationLevel::O3) => "llvm_o3",
        (BackendChoice::Llvm, _) => "llvm_opt",
        (BackendChoice::Wasm, _) => "wasm",
    };
    let dir = workdir(&format!("{name}_{tag}"));
    let entry = dir.join("main.lpp");
    let exe = dir.join(format!("main{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&entry, source).unwrap();

    let runtime = runtime_library_path().expect("test runtime library is built");
    let mut opts = CompileOptions::for_backend(backend);
    opts.optimization = opt;
    build_executable_configured(
        &entry,
        "llvm_parity_e2e",
        backend,
        opts,
        &exe,
        runtime.parent().unwrap(),
        Some("cc"),
    )
    .unwrap_or_else(|e| panic!("build failed ({tag}) for {name}: {e:?}"));

    let output = Command::new(&exe).output().unwrap();
    assert!(
        output.status.success(),
        "binary failed ({tag}) for {name} (exit {:?}): stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn assert_llvm_parity(name: &str, source: &str, expected: &str) {
    let cranelift_out = run_on_backend(
        name,
        source,
        BackendChoice::Cranelift,
        OptimizationLevel::O0,
    );
    let llvm_o0_out = run_on_backend(name, source, BackendChoice::Llvm, OptimizationLevel::O0);
    let llvm_o3_out = run_on_backend(name, source, BackendChoice::Llvm, OptimizationLevel::O3);
    assert_eq!(cranelift_out, expected, "Cranelift mismatch for {name}");
    assert_eq!(llvm_o0_out, cranelift_out, "LLVM -O0 mismatch for {name}");
    assert_eq!(llvm_o3_out, cranelift_out, "LLVM -O3 mismatch for {name}");
}

#[test]
fn llvm_parity_structs_enums_tuples_collections_closures() {
    let source = r#"
struct Vec2:
    x: Int
    y: Int

impl Vec2:
    def dot(self, other: Vec2) -> Int:
        return self.x * other.x + self.y * other.y

enum Op:
    Zero
    Add(a: Int, b: Int)
    Msg(text: Str)

def main():
    v1 := Vec2(3, 4)
    v2 := Vec2(5, 6)
    print(v1.dot(v2))

    pair := (10, "ok")
    print(pair.0)
    print_str(pair.1)

    mut nums := [1, 2, 3]
    list_push(nums, 4)
    print(len(nums))
    print(nums[3])

    mut fvals := [1.5, 2.5]
    print_float(fvals[1])

    mut bvals := [true, false]
    print_bool(bvals[0])

    mut svals := ["alpha", "beta"]
    print_str(svals[1])

    mut m := map_new()
    map_put(m, "k1", 99)
    print_bool(map_has(m, "k1"))
    print(map_get(m, "k1"))

    op1 := Op.Add(12, 30)
    op2 := Op.Msg("done")
    match op1:
        Zero:
            print(0)
        Add(a, b):
            print(a + b)
        Msg(s):
            print_str(s)
    match op2:
        Zero:
            print(0)
        Add(a, b):
            print(a + b)
        Msg(s):
            print_str(s)

    mut acc := 10
    bump := fn(delta: Int) -> Int:
        acc = acc + delta
        return acc
    print(bump(5))
    print(bump(7))
    print(acc)
"#;
    assert_llvm_parity(
        "managed_all",
        source,
        "39\n10\nok\n4\n4\n2.500000\n1\nbeta\n1\n99\n42\ndone\n15\n22\n22\n",
    );
}

#[test]
fn llvm_parity_async_spawn_await_cycles_and_ffi() {
    let source = r#"
extern "C":
    def abs(x: Int) -> Int

struct Node:
    value: Int
    next: Node

async def compute(x: Int) -> Int:
    return x * 2 + 1

def main():
    seed := 7
    spawn fn() -> Void:
        y := seed * 2
    res := compute(20).await
    print(res)

    tail := Node(2, 0)
    mut head := Node(1, tail)
    # Create a true self-cycle and reclaim on scope exit / collection.
    head.next = head
    print(head.value)
    print(abs(-77))
"#;
    assert_llvm_parity("async_cycles_ffi", source, "41\n1\n77\n");
}

#[test]
fn llvm_parity_unsafe_ptr_mmio_simd_and_str_slice() {
    let source = r#"
extern "C":
    def malloc(size: USize) -> RawPtr
    def free(ptr: RawPtr)

@repr(C)
@align(16)
struct Frame:
    magic: U64
    len: USize

def main():
    f := Frame(0x1234 as U64, 64 as USize)
    print(f.magic as Int)
    print(f.len as Int)
    print(size_of[Frame]())
    print(align_of[Frame]())

    unsafe:
        buf := malloc(32 as USize)
        b1 := buf as Ptr[U8]
        memset_raw(b1, 0x41 as U8, 8 as USize)
        p := buf as Ptr[U64]
        ptr_write[U64](p, 0xCAFEBABE as U64)
        got := ptr_read[U64](p)
        print(got as Int)

        buf2 := malloc(32 as USize)
        b2 := buf2 as Ptr[U8]
        memcpy_raw(b2, b1, 8 as USize)
        cmp := memcmp_raw(b1, b2, 8 as USize)
        print(cmp)
        free(buf)
        free(buf2)

    a := vec_i64x2(10, 20)
    b := vec_i64x2_splat(5)
    c := vec_i64x2_add(a, b)
    print(vec_i64x2_extract(c, 0))
    print(vec_i64x2_extract(c, 1))

    full := "hello-lpp"
    sub := str_slice(full, 6, 3)
    print_str(str_slice_to_str(sub))
"#;
    assert_llvm_parity(
        "unsafe_simd_slice",
        source,
        "4660\n64\n16\n16\n3405691582\n0\n15\n25\nlpp\n",
    );
}

#[test]
fn llvm_freestanding_kernel_x86_64_and_aarch64_elf() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().unwrap().parent().unwrap();
    let kernel_src = repo_root.join("examples/kernel_x86_64/src/main.lpp");
    assert!(kernel_src.is_file(), "missing {}", kernel_src.display());

    let dir = workdir("llvm_kernel");
    let kernel_elf = dir.join("kernel_llvm_x86_64.elf");

    let mut opts = CompileOptions::for_backend(BackendChoice::Llvm);
    opts.target = Target::X86_64None;
    opts.optimization = OptimizationLevel::O3;
    opts.multiboot2 = true;
    opts.image_base = Some(0x0010_0000);

    build_freestanding_kernel_with_backend(
        &kernel_src,
        "kernel_x86_64_llvm",
        BackendChoice::Llvm,
        opts,
        &kernel_elf,
    )
    .expect("LLVM freestanding x86_64 kernel must compile and statically link");

    let elf_bytes = std::fs::read(&kernel_elf).unwrap();
    assert_eq!(&elf_bytes[0..4], b"\x7fELF");
    assert_eq!(u16::from_le_bytes([elf_bytes[16], elf_bytes[17]]), 2); // ET_EXEC
    assert_eq!(u16::from_le_bytes([elf_bytes[18], elf_bytes[19]]), 62); // EM_X86_64
    let mb2_magic = 0xE852_50D6_u32.to_le_bytes();
    assert!(
        elf_bytes[..elf_bytes.len().min(32 * 1024)]
            .windows(4)
            .any(|w| w == mb2_magic),
        "Multiboot2 magic must appear in first 32 KiB"
    );

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
                "LLVM kernel disassembly missing `{expected}`:\n{disasm}"
            );
        }
    }

    // Verify CLI `--backend llvm --target x86_64-unknown-none -O3 --multiboot2` works end-to-end.
    let cli_elf = dir.join("cli_llvm_kernel.elf");
    let req = DriverRequest::from_args(
        [
            "lpp".to_string(),
            kernel_src.to_string_lossy().into_owned(),
            "--backend".to_string(),
            "llvm".to_string(),
            "--target".to_string(),
            "x86_64-unknown-none".to_string(),
            "-O3".to_string(),
            "--multiboot2".to_string(),
            "-o".to_string(),
            cli_elf.to_string_lossy().into_owned(),
        ],
        &dir,
    )
    .unwrap();
    let exit = CompilerSession::new(RewriteEngine)
        .execute(&req)
        .exit_code();
    assert_eq!(exit, 0, "CLI LLVM bare-metal build must succeed");
    assert!(cli_elf.is_file());

    // Also verify LLVM aarch64-unknown-none freestanding ELF generation.
    let a64_src = dir.join("kernel_a64_llvm.lpp");
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
    let a64_elf = dir.join("kernel_a64_llvm.elf");
    let mut a64_opts = CompileOptions::for_backend(BackendChoice::Llvm);
    a64_opts.target = Target::Aarch64None;
    a64_opts.optimization = OptimizationLevel::O2;
    build_freestanding_kernel_with_backend(
        &a64_src,
        "a64_kernel_llvm",
        BackendChoice::Llvm,
        a64_opts,
        &a64_elf,
    )
    .expect("LLVM freestanding aarch64 kernel must compile and statically link");
    let a64_bytes = std::fs::read(&a64_elf).unwrap();
    assert_eq!(&a64_bytes[0..4], b"\x7fELF");
    assert_eq!(u16::from_le_bytes([a64_bytes[18], a64_bytes[19]]), 183); // EM_AARCH64
}
