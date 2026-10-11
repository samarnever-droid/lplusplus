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
        write(2, bytes.as_ptr().cast::<c_void>(), bytes.len() as _);
        write(2, b"\n".as_ptr().cast::<c_void>(), 1 as _);
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
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)
    {
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_command_exec(cmd: *const c_char) -> i64 {
    let bytes = unsafe { cstr_bytes(cmd) };
    if bytes.is_empty() {
        return -1;
    }
    let command = String::from_utf8_lossy(bytes);
    #[cfg(windows)]
    let status = std::process::Command::new("cmd")
        .arg("/C")
        .arg(command.as_ref())
        .status();
    #[cfg(not(windows))]
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(command.as_ref())
        .status();
    match status {
        Ok(s) => i64::from(s.code().unwrap_or(-1)),
        Err(_) => -1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_env_get(name: *const c_char) -> *mut c_char {
    if name.is_null() {
        return unsafe { arc_string(&[]) };
    }
    let key = String::from_utf8_lossy(unsafe { cstr_bytes(name) });
    match std::env::var(key.as_ref()) {
        Ok(val) => unsafe { arc_string(val.as_bytes()) },
        Err(_) => unsafe { arc_string(&[]) },
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_env_set(name: *const c_char, value: *const c_char) -> i64 {
    if name.is_null() {
        return -1;
    }
    let key = String::from_utf8_lossy(unsafe { cstr_bytes(name) });
    let val = if value.is_null() {
        String::new()
    } else {
        String::from_utf8_lossy(unsafe { cstr_bytes(value) }).into_owned()
    };
    unsafe {
        std::env::set_var(key.as_ref(), val);
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_dir_list(path: *const c_char) -> *mut c_void {
    let list = unsafe { crate::list::lpp_list_new_arc() };
    if list.is_null() || path.is_null() {
        return list;
    }
    let p = path_from(unsafe { cstr_bytes(path) });
    if let Ok(entries) = std::fs::read_dir(p) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let s = name.to_string_lossy();
            let arc = unsafe { arc_string(s.as_bytes()) };
            unsafe { crate::list::lpp_list_push_arc(list, arc as *mut c_void) };
        }
    }
    list
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_path_join(base: *const c_char, child: *const c_char) -> *mut c_char {
    let b = unsafe { cstr_bytes(base) };
    let c = unsafe { cstr_bytes(child) };
    let sep = if cfg!(windows) { b'\\' } else { b'/' };
    let need_sep = !b.is_empty() && b[b.len() - 1] != b'/' && b[b.len() - 1] != b'\\';
    let mut out = Vec::with_capacity(b.len() + usize::from(need_sep) + c.len());
    out.extend_from_slice(b);
    if need_sep {
        out.push(sep);
    }
    out.extend_from_slice(c);
    unsafe { arc_string(&out) }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_sleep_ms(ms: i64) {
    if ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(ms as u64));
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_exit(code: i64) {
    unsafe { flush_all() };
    std::process::exit(code as i32);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_write_int(value: i64) {
    unsafe {
        printf(b"%lld\0".as_ptr().cast::<c_char>(), value);
        flush_all();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_write_uint(value: i64) {
    unsafe {
        printf(b"%llu\0".as_ptr().cast::<c_char>(), value as u64);
        flush_all();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_write_float(value: f64) {
    unsafe {
        printf(b"%g\0".as_ptr().cast::<c_char>(), value);
        flush_all();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_write_bool(value: i8) {
    let s = if value != 0 {
        b"true\0".as_ptr()
    } else {
        b"false\0".as_ptr()
    };
    unsafe {
        printf(b"%s\0".as_ptr().cast::<c_char>(), s.cast::<c_char>());
        flush_all();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_write_char(value: i64) {
    let b = (value & 0xFF) as u8;
    unsafe {
        write(1, (&b as *const u8).cast::<c_void>(), 1 as _);
        flush_all();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_write_nl() {
    unsafe {
        write(1, b"\n".as_ptr().cast::<c_void>(), 1 as _);
        flush_all();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_ewrite_str(ptr: *const c_char) {
    if ptr.is_null() {
        return;
    }
    let bytes = unsafe { cstr_bytes(ptr) };
    unsafe {
        write(2, bytes.as_ptr().cast::<c_void>(), bytes.len() as _);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_flush() {
    unsafe { flush_all() };
}

// ── CLI Arguments & Working Directory Builtins ──────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn lpp_args_count() -> i64 {
    std::env::args_os().count() as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_args_get(index: i64) -> *mut c_char {
    if index < 0 {
        return unsafe { arc_string(&[]) };
    }
    match std::env::args_os().nth(index as usize) {
        Some(arg) => {
            let s = arg.to_string_lossy();
            unsafe { arc_string(s.as_bytes()) }
        }
        None => unsafe { arc_string(&[]) },
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_args() -> *mut c_void {
    let list = unsafe { crate::list::lpp_list_new_arc() };
    if list.is_null() {
        return list;
    }
    for arg in std::env::args_os() {
        let s = arg.to_string_lossy();
        let arc = unsafe { arc_string(s.as_bytes()) };
        unsafe { crate::list::lpp_list_push_arc(list, arc as *mut c_void) };
    }
    list
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_cwd() -> *mut c_char {
    match std::env::current_dir() {
        Ok(dir) => {
            let s = dir.to_string_lossy();
            unsafe { arc_string(s.as_bytes()) }
        }
        Err(_) => unsafe { arc_string(&[]) },
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_chdir(path: *const c_char) -> i64 {
    if path.is_null() {
        return -1;
    }
    let p = path_from(unsafe { cstr_bytes(path) });
    match std::env::set_current_dir(p) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_is_dir(path: *const c_char) -> i64 {
    if path.is_null() {
        return 0;
    }
    i64::from(path_from(unsafe { cstr_bytes(path) }).is_dir())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_is_file(path: *const c_char) -> i64 {
    if path.is_null() {
        return 0;
    }
    i64::from(path_from(unsafe { cstr_bytes(path) }).is_file())
}

// ── Positional Durable File I/O ─────────────────────────────────────────────

use std::collections::BTreeMap;
use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicI64, Ordering},
};

static NEXT_FD: AtomicI64 = AtomicI64::new(1);

fn fd_table() -> &'static Mutex<BTreeMap<i64, std::fs::File>> {
    static TABLE: OnceLock<Mutex<BTreeMap<i64, std::fs::File>>> = OnceLock::new();
    TABLE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_file_open(path: *const c_char, _flags: i64) -> i64 {
    if path.is_null() {
        return -1;
    }
    let p = path_from(unsafe { cstr_bytes(path) });
    make_parent_dirs(&p);
    let Ok(file) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&p)
    else {
        return -1;
    };
    let fd = NEXT_FD.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut table) = fd_table().lock() {
        table.insert(fd, file);
        fd
    } else {
        -1
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_file_close(fd: i64) {
    if fd <= 0 {
        return;
    }
    if let Ok(mut table) = fd_table().lock() {
        table.remove(&fd);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_file_pread(
    fd: i64,
    buf: i64,
    buf_off: i64,
    file_off: i64,
    len: i64,
) -> i64 {
    use std::io::{Read, Seek, SeekFrom};
    if fd <= 0 || buf == 0 || len < 0 || file_off < 0 || buf_off < 0 {
        return -1;
    }
    let buf_ptr = buf as usize as *mut u8;
    let buf_size = unsafe { *(buf_ptr as *const i64) };
    if buf_off.saturating_add(len) > buf_size {
        return -1;
    }
    if len == 0 {
        return 0;
    }
    let Ok(mut table) = fd_table().lock() else {
        return -1;
    };
    let Some(file) = table.get_mut(&fd) else {
        return -1;
    };
    if file.seek(SeekFrom::Start(file_off as u64)).is_err() {
        return -1;
    }
    let dst =
        unsafe { std::slice::from_raw_parts_mut(buf_ptr.add(8 + buf_off as usize), len as usize) };
    let mut total = 0usize;
    while total < dst.len() {
        match file.read(&mut dst[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return -1,
        }
    }
    total as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_file_pwrite(
    fd: i64,
    buf: i64,
    buf_off: i64,
    file_off: i64,
    len: i64,
) -> i64 {
    use std::io::{Seek, SeekFrom, Write};
    if fd <= 0 || buf == 0 || len < 0 || file_off < 0 || buf_off < 0 {
        return -1;
    }
    let buf_ptr = buf as usize as *const u8;
    let buf_size = unsafe { *(buf_ptr as *const i64) };
    if buf_off.saturating_add(len) > buf_size {
        return -1;
    }
    if len == 0 {
        return 0;
    }
    let Ok(mut table) = fd_table().lock() else {
        return -1;
    };
    let Some(file) = table.get_mut(&fd) else {
        return -1;
    };
    if file.seek(SeekFrom::Start(file_off as u64)).is_err() {
        return -1;
    }
    let src =
        unsafe { std::slice::from_raw_parts(buf_ptr.add(8 + buf_off as usize), len as usize) };
    match file.write_all(src) {
        Ok(()) => len,
        Err(_) => -1,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_file_fsync(fd: i64) -> i64 {
    if fd <= 0 {
        return -1;
    }
    let Ok(table) = fd_table().lock() else {
        return -1;
    };
    let Some(file) = table.get(&fd) else {
        return -1;
    };
    match file.sync_all() {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_file_fdatasync(fd: i64) -> i64 {
    if fd <= 0 {
        return -1;
    }
    let Ok(table) = fd_table().lock() else {
        return -1;
    };
    let Some(file) = table.get(&fd) else {
        return -1;
    };
    match file.sync_data() {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_file_ftruncate(fd: i64, size: i64) -> i64 {
    if fd <= 0 || size < 0 {
        return -1;
    }
    let Ok(table) = fd_table().lock() else {
        return -1;
    };
    let Some(file) = table.get(&fd) else {
        return -1;
    };
    match file.set_len(size as u64) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn lpp_fd_size(fd: i64) -> i64 {
    if fd <= 0 {
        return -1;
    }
    let Ok(table) = fd_table().lock() else {
        return -1;
    };
    let Some(file) = table.get(&fd) else {
        return -1;
    };
    match file.metadata() {
        Ok(meta) => i64::try_from(meta.len()).unwrap_or(-1),
        Err(_) => -1,
    }
}
