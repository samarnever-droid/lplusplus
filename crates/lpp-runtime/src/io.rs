//! The IO builtins (Phase 6B.3c).
//!
//! Rust re-implementation of the `lpp_runtime.c` console primitives,
//! byte-identical to the C reference: `print_int`/`print_float`/`print_bool`
//! use the same `printf` formats (`%lld\n`, `%f\n`, `%d\n`), `print_str` uses
//! `puts` (trailing newline), `write_str` uses `fputs`-equivalent output with
//! NO newline, and `eprint_str` (defined by the `c_shim` reference, absent
//! from `lpp_runtime.c`) writes to stderr with a trailing newline. Each
//! stdout write is followed by a flush so output ordering matches C's
//! `fflush(stdout)`.

use libc::{c_char, c_void, fflush, printf, puts, strlen, write};

use crate::string::arc_string;

/// The bytes of a NUL-terminated C string (empty for NULL), no terminator.
unsafe fn cstr_bytes<'a>(s: *const c_char) -> &'a [u8] {
    if s.is_null() {
        return &[];
    }
    unsafe { std::slice::from_raw_parts(s as *const u8, strlen(s)) }
}

/// Flush all open C streams (POSIX `fflush(NULL)`), matching the C runtime's
/// `fflush(stdout)` after each write without needing the `stdout` `FILE *`.
unsafe fn flush_all() {
    unsafe { fflush(std::ptr::null_mut()) };
}

/// `print_int(value)` — `printf("%lld\n", value)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_print_int(value: i64) {
    unsafe {
        printf(b"%lld\n\0".as_ptr().cast::<c_char>(), value);
        flush_all();
    }
}

/// `print_float(value)` — `printf("%f\n", value)` (6 decimal places, NOT %g).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_print_float(value: f64) {
    unsafe {
        printf(b"%f\n\0".as_ptr().cast::<c_char>(), value);
        flush_all();
    }
}

/// `print_bool(value)` — `printf("%d\n", value ? 1 : 0)`. The parameter is
/// `i8` (L++ `Bool` lowers to i8).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_print_bool(value: i8) {
    unsafe {
        printf(b"%d\n\0".as_ptr().cast::<c_char>(), i32::from(value != 0));
        flush_all();
    }
}

/// `print_str(ptr)` — `puts(ptr)` (writes the string plus a trailing
/// newline). NULL is ignored.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_print_str(ptr: *const c_char) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        puts(ptr);
        flush_all();
    }
}

/// `write_str(ptr)` — writes the raw string with NO trailing newline
/// (`fputs(ptr, stdout)`). NULL is ignored.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_write_str(ptr: *const c_char) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        printf(b"%s\0".as_ptr().cast::<c_char>(), ptr);
        flush_all();
    }
}

/// `eprint_str(ptr)` — writes the string plus a trailing newline to stderr.
/// Defined by the `c_shim` reference (absent from `lpp_runtime.c`); uses a
/// direct fd-2 write to avoid the `stderr` `FILE *` static. NULL is ignored.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_eprint_str(ptr: *const c_char) {
    if ptr.is_null() {
        return;
    }
    let bytes = unsafe { cstr_bytes(ptr) };
    unsafe {
        write(2, bytes.as_ptr().cast::<c_void>(), bytes.len());
        write(2, b"\n".as_ptr().cast::<c_void>(), 1);
    }
}

/// `input()` — read one line from stdin, returned as an ARC string with the
/// trailing newline stripped. At EOF (or with stdin closed, as under the test
/// harness) it returns the empty string, so an interactive loop reading past
/// the end of input terminates instead of blocking. A trailing CR is stripped
/// too, so CRLF input yields the same bytes as LF.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_input() -> *mut c_char {
    use std::io::Read;
    let stdin = std::io::stdin();
    let mut handle = stdin.lock();
    let mut buf: Vec<u8> = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match handle.read(&mut byte) {
            Ok(0) => break, // EOF
            Ok(_) => {
                if byte[0] == b'\n' {
                    break;
                }
                buf.push(byte[0]);
            }
            Err(_) => break, // a read error is treated as end of input
        }
    }
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    unsafe { arc_string(&buf) }
}

// ── Filesystem + subprocess builtins (E5003 IO file-ops slice) ──────────────
//
// The v1 C reference (`lpp_runtime.c`) implements `read_file`/`write_file`/
// `file_exists`/`file_size`/`command_output`; `dir_create`/`dir_remove`/
// `path_exists`/`file_copy`/`file_move`/`delete_file`/`append_file` never had a
// C body (that is why they hit E5003 in the rewrite). These Rust ports keep the
// v1 ABI: paths and data arrive as ARC/C strings (`i64` pointers at the ABI
// boundary), the `-> i64` results use `0 = success, -1 = error` for the mutating
// ops and `1/0` for the existence predicates, and the `-> Str` results
// (`read_file`, `command_output`) return a freshly ARC-allocated owning string
// (the empty string on error, matching every other string builtin's
// `if (!out) return lpp_empty_str();` contract rather than v1's raw NULL).

/// Build a `PathBuf` from raw path bytes without forcing UTF-8 on POSIX (paths
/// are arbitrary byte strings there); other platforms fall back to a lossy
/// decode, which is exact for the ASCII paths the corpus uses.
fn path_from(bytes: &[u8]) -> std::path::PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        std::path::PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
    }
    #[cfg(not(unix))]
    {
        std::path::PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
    }
}

/// Create every missing ancestor directory of `path` (v1 `make_parent_dirs`).
/// A missing/empty parent is a no-op; errors are swallowed, exactly like the C
/// reference, because the subsequent open reports the real failure.
fn make_parent_dirs(path: &std::path::Path) {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }
}

/// `read_file(path) -> Str` — whole-file contents as an owning ARC string; the
/// empty string on any error (missing/unreadable path).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_read_file(path: *const c_char) -> *mut c_char {
    let bytes = unsafe { cstr_bytes(path) };
    match std::fs::read(path_from(bytes)) {
        Ok(data) => unsafe { arc_string(&data) },
        Err(_) => unsafe { arc_string(&[]) },
    }
}

/// `write_file(path, data) -> Int` — truncating write; creates parent dirs.
/// Returns 0 on success, -1 on error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_write_file(path: *const c_char, data: *const c_char) -> i64 {
    if path.is_null() || data.is_null() {
        return -1;
    }
    let p = path_from(unsafe { cstr_bytes(path) });
    let data = unsafe { cstr_bytes(data) };
    make_parent_dirs(&p);
    match std::fs::write(&p, data) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

/// `append_file(path, data) -> Int` — create-or-append; creates parent dirs.
/// Returns 0 on success, -1 on error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_append_file(path: *const c_char, data: *const c_char) -> i64 {
    use std::io::Write;
    if path.is_null() || data.is_null() {
        return -1;
    }
    let p = path_from(unsafe { cstr_bytes(path) });
    let data = unsafe { cstr_bytes(data) };
    make_parent_dirs(&p);
    match std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        Ok(mut f) => {
            if f.write_all(data).is_ok() {
                0
            } else {
                -1
            }
        }
        Err(_) => -1,
    }
}

/// `delete_file(path) -> Int` — remove a regular file. 0 on success, -1 on error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_delete_file(path: *const c_char) -> i64 {
    match std::fs::remove_file(path_from(unsafe { cstr_bytes(path) })) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

/// `file_exists(path) -> Int` — 1 if `path` is an existing regular file, else 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_file_exists(path: *const c_char) -> i64 {
    i64::from(path_from(unsafe { cstr_bytes(path) }).is_file())
}

/// `file_size(path) -> Int` — size in bytes of a regular file, or -1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_file_size(path: *const c_char) -> i64 {
    match std::fs::metadata(path_from(unsafe { cstr_bytes(path) })) {
        Ok(meta) if meta.is_file() => i64::try_from(meta.len()).unwrap_or(-1),
        _ => -1,
    }
}

/// `file_copy(src, dst) -> Int` — copy contents, creating `dst`'s parents.
/// 0 on success, -1 on error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_file_copy(src: *const c_char, dst: *const c_char) -> i64 {
    if src.is_null() || dst.is_null() {
        return -1;
    }
    let s = path_from(unsafe { cstr_bytes(src) });
    let d = path_from(unsafe { cstr_bytes(dst) });
    make_parent_dirs(&d);
    match std::fs::copy(&s, &d) {
        Ok(_) => 0,
        Err(_) => -1,
    }
}

/// `file_move(src, dst) -> Int` — rename `src` to `dst` (parents created for
/// `dst`), with a copy+unlink fallback across filesystems. 0 ok, -1 error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_file_move(src: *const c_char, dst: *const c_char) -> i64 {
    if src.is_null() || dst.is_null() {
        return -1;
    }
    let s = path_from(unsafe { cstr_bytes(src) });
    let d = path_from(unsafe { cstr_bytes(dst) });
    make_parent_dirs(&d);
    if std::fs::rename(&s, &d).is_ok() {
        return 0;
    }
    // Cross-device rename fails with EXDEV; fall back to copy then unlink.
    match std::fs::copy(&s, &d) {
        Ok(_) => {
            let _ = std::fs::remove_file(&s);
            0
        }
        Err(_) => -1,
    }
}

/// `dir_create(path) -> Int` — create the directory and any missing ancestors
/// (idempotent). 0 on success, -1 on error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_dir_create(path: *const c_char) -> i64 {
    match std::fs::create_dir_all(path_from(unsafe { cstr_bytes(path) })) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

/// `dir_remove(path) -> Int` — remove a directory and its contents. 0 ok, -1 err.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_dir_remove(path: *const c_char) -> i64 {
    match std::fs::remove_dir_all(path_from(unsafe { cstr_bytes(path) })) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

/// `path_exists(path) -> Int` — 1 if anything (file or directory) exists at
/// `path`, else 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_path_exists(path: *const c_char) -> i64 {
    i64::from(path_from(unsafe { cstr_bytes(path) }).exists())
}

/// `command_output(cmd) -> Str` — run `cmd` through the platform shell and
/// return its captured stdout as an owning ARC string (the empty string on
/// spawn failure or a NULL command), mirroring v1's `popen`/`_popen`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_command_output(cmd: *const c_char) -> *mut c_char {
    let bytes = unsafe { cstr_bytes(cmd) };
    if bytes.is_empty() {
        return unsafe { arc_string(&[]) };
    }
    let command = String::from_utf8_lossy(bytes);
    #[cfg(windows)]
    let spawned = std::process::Command::new("cmd")
        .arg("/C")
        .arg(command.as_ref())
        .output();
    #[cfg(not(windows))]
    let spawned = std::process::Command::new("sh")
        .arg("-c")
        .arg(command.as_ref())
        .output();
    match spawned {
        Ok(output) => unsafe { arc_string(&output.stdout) },
        Err(_) => unsafe { arc_string(&[]) },
    }
}
