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
const R_AARCH64_JUMP_SLOT: u32 = 1026;
#[allow(dead_code)]
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
        "sin"
            | "cos"
            | "tan"
            | "asin"
            | "acos"
            | "atan"
            | "atan2"
            | "sinh"
            | "cosh"
            | "tanh"
            | "exp"
            | "exp2"
            | "log"
            | "log2"
            | "log10"
            | "pow"
            | "sqrt"
            | "ceil"
            | "floor"
            | "fmod"
            | "fabs"
            | "hypot"
            | "round"
            | "trunc"
            | "nearbyint"
            | "sincos"
            | "ldexp"
            | "frexp"
            | "sinf"
            | "cosf"
            | "tanf"
            | "expf"
            | "logf"
            | "powf"
            | "sqrtf"
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
        "malloc"
            | "free"
            | "realloc"
            | "calloc"
            | "aligned_alloc"
            | "posix_memalign"
            | "printf"
            | "fprintf"
            | "sprintf"
            | "snprintf"
            | "vsnprintf"
            | "vfprintf"
            | "puts"
            | "putchar"
            | "getchar"
            | "scanf"
            | "sscanf"
            | "memset"
            | "memcpy"
            | "memmove"
            | "memcmp"
            | "strlen"
            | "strcmp"
            | "strncmp"
            | "strcpy"
            | "strncpy"
            | "strcat"
            | "strchr"
            | "strstr"
            | "strdup"
            | "strtol"
            | "strtoul"
            | "strtod"
            | "atoi"
            | "atol"
            | "atoll"
            | "exit"
            | "abort"
            | "_exit"
            | "atexit"
            | "fopen"
            | "fclose"
            | "fread"
            | "fwrite"
            | "fflush"
            | "fseek"
            | "ftell"
            | "rewind"
            | "feof"
            | "ferror"
            | "fgets"
            | "fputs"
            | "getenv"
            | "setenv"
            | "unsetenv"
            | "system"
            | "time"
            | "clock"
            | "qsort"
            | "bsearch"
            | "rand"
            | "srand"
            | "abs"
            | "labs"
            | "llabs"
            | "tolower"
            | "toupper"
            | "isdigit"
            | "isalpha"
            | "isspace"
            | "isalnum"
            | "mmap"
            | "munmap"
            | "mprotect"
            | "brk"
            | "sbrk"
            | "read"
            | "write"
            | "open"
            | "close"
            | "lseek"
            | "stat"
            | "fstat"
            | "unlink"
            | "rename"
            | "getcwd"
            | "chdir"
            | "mkdir"
            | "rmdir"
            | "getpid"
            | "getuid"
            | "getgid"
            | "fork"
            | "execve"
            | "waitpid"
            | "signal"
            | "sigaction"
            | "kill"
            | "raise"
            | "socket"
            | "bind"
            | "listen"
            | "accept"
            | "connect"
            | "send"
            | "recv"
            | "sendto"
            | "recvfrom"
            | "closesocket"
            | "htons"
            | "htonl"
            | "ntohs"
            | "ntohl"
            | "getaddrinfo"
            | "freeaddrinfo"
            | "setsockopt"
            | "getsockopt"
            | "poll"
            | "select"
            | "gettimeofday"
            | "nanosleep"
            | "usleep"
            | "stdin"
            | "stdout"
            | "stderr"
            | "__libc_start_main"
            | "__errno_location"
            | "memchr"
            | "strerror"
            | "perror"
            | "setvbuf"
            | "ungetc"
            | "opendir"
            | "readdir"
            | "closedir"
            | "clock_gettime"
            | "localtime"
            | "gmtime"
            | "strftime"
            | "lpp_c_malloc"
            | "lpp_c_free"
            | "lpp_c_load_u8"
            | "lpp_c_store_u8"
            | "lpp_c_load_i32"
            | "lpp_c_store_i32"
            | "lpp_c_load_i64"
            | "lpp_c_store_i64"
    )
}

fn elf_needed_for(undef: &[String], opts: &LinkOptions) -> Vec<String> {
    let mut n = BTreeSet::new();
    for s in &opts.needed {
        n.insert(s.clone());
    }
    for lib in &opts.libraries {
        let fname = Path::new(lib)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(lib);
        let soname = if fname.starts_with("lib") || fname.contains(".so") {
            fname.to_string()
        } else {
            format!("lib{fname}.so")
        };
        n.insert(soname);
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

/// Which C runtime the synthesized `_start` targets.
///
/// A freestanding image has no libc and exits through a raw syscall. A hosted
/// image links glibc (directly, or transitively through a Rust cdylib like
/// `liblpp_runtime.so`), so it MUST bootstrap through `__libc_start_main` — the
/// same contract the system `Scrt1.o` implements. Calling `main` directly (as
/// the freestanding stub does) leaves glibc half-initialized: TLS, `errno`,
/// stdio buffers, `__environ`, the stack guard, and the C++/Rust runtime's own
/// setup never run, and the process crashes as soon as any of that is touched.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StartupAbi {
    /// No libc: call `main`, then `exit(2)` via a raw syscall.
    Freestanding,
    /// glibc-hosted: bootstrap through `__libc_start_main`, exactly like
    /// `Scrt1.o`, so libc (and any runtime depending on it) is fully live
    /// before `main` runs.
    HostedGlibc,
}

/// x86_64 `_start`. Returns the stub bytes plus a list of
/// `(plt_symbol, call_site_offset_in_text)` pairs whose `call rel32` must be
/// patched to the symbol's PLT entry once the PLT layout is fixed.
///
/// Freestanding layout (bytes from `start_off`):
///   0  xor ebp, ebp
///   2  pop rsi                 ; argc
///   3  mov rdi, rsp            ; argv
///   6  and rsp, -16
///  10  xor edx, edx            ; envp = NULL
///  12  e8 <rel32>              ; call main; next-ip = start_off+17
///  17  mov edi, eax
///  19  mov eax, 60 ; syscall   ; exit(status)
///
/// Hosted-glibc layout (mirrors `Scrt1.o`):
///   0  xor ebp, ebp
///   2  mov r9, rdx             ; rtld_fini (passed by the loader in rdx)
///   5  pop rsi                 ; argc
///   6  mov rdx, rsp            ; argv
///   9  and rsp, -16
///  13  push rax                ; align / junk slot
///  14  push rsp                ; stack_end
///  15  xor r8d, r8d            ; fini = NULL   (glibc >= 2.34 runs init_array itself)
///  18  xor ecx, ecx            ; init = NULL
///  20  lea rdi, [rip+main]     ; main; disp @ 23, next-ip = start_off+27
///  27  e8 <rel32>              ; call __libc_start_main@plt; disp @ 28, next-ip = +32
///  32  hlt                     ; unreachable (__libc_start_main never returns)
type ElfStartStub = (Vec<u8>, Vec<(&'static str, usize)>);

fn emit_elf_start_stub_x64(
    start_off: usize,
    main_off: usize,
    abi: StartupAbi,
) -> Result<ElfStartStub, LinkError> {
    match abi {
        StartupAbi::Freestanding => {
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
            s.extend_from_slice(&[
                0x89, 0xc7, // mov edi, eax
                0xb8, 60, 0, 0, 0, // mov eax, 60 (sys_exit)
                0x0f, 0x05, // syscall
                0x0f, 0x0b, // ud2
            ]);
            Ok((s, Vec::new()))
        }
        StartupAbi::HostedGlibc => {
            let mut s = vec![
                0x31, 0xed, // 0  xor ebp, ebp
                0x49, 0x89, 0xd1, // 2  mov r9, rdx          ; rtld_fini
                0x5e, // 5  pop rsi                          ; argc
                0x48, 0x89, 0xe2, // 6  mov rdx, rsp         ; argv
                0x48, 0x83, 0xe4, 0xf0, // 9  and rsp, -16
                0x50, // 13 push rax
                0x54, // 14 push rsp                         ; stack_end
                0x45, 0x31, 0xc0, // 15 xor r8d, r8d         ; fini = NULL
                0x31, 0xc9, // 18 xor ecx, ecx               ; init = NULL
                0x48, 0x8d, 0x3d, 0, 0, 0,
                0, // 20 lea rdi,[rip+main]  (disp @ 23, next-ip = +27)
                0xe8, 0, 0, 0, 0,    // 27 call __libc_start_main@plt (disp @ 28, next-ip = +32)
                0xf4, // 32 hlt
            ];
            let lea_next_ip = start_off as i64 + 27;
            let lea_disp = main_off as i64 - lea_next_ip;
            if !fits_i32(lea_disp) {
                return Err("entry point out of range for startup lea".into());
            }
            s[23..27].copy_from_slice(&(lea_disp as i32).to_le_bytes());
            // The call rel32 is filled in post-layout, once __libc_start_main's
            // PLT slot address is known (site = start_off + 28).
            Ok((s, vec![("__libc_start_main", start_off + 28)]))
        }
    }
}

fn emit_elf_start_stub_a64() -> Vec<u8> {
    // bti c ; bl main ; mov x8,#93 ; svc #0
    // bti c (0xd503245f) is required for ARM64 BTI hardware compatibility.
    vec![
        0x5f, 0x24, 0x03, 0xd5, // bti c
        0x00, 0x00, 0x00, 0x94, // bl #0 (patched at start_off + 4)
        0xa8, 0x0b, 0x80, 0xd2, // mov x8, #93
        0x01, 0x00, 0x00, 0xd4, // svc #0
    ]
}

