//! Gate: the IO builtins (Phase 6B.3c) — differential against the C
//! reference.
//!
//! A single test (so the stdout/stderr fd redirection never races another
//! test in this binary): it compiles + runs the v1 C IO scenario capturing
//! stdout and stderr, then runs the same calls through the Rust runtime with
//! the process fds redirected to temp files, and asserts the two byte
//! streams are identical. `lpp_eprint_str` is defined by the `c_shim`
//! reference (absent from `lpp_runtime.c`), so the C scenario carries the
//! shim's definition.

use std::path::PathBuf;
use std::process::Command;

const IO_SCENARIO: &str = r#"
#include <stdio.h>
#include <stdint.h>
extern void lpp_print_int(int64_t);
extern void lpp_print_float(double);
extern void lpp_print_bool(int8_t);
extern void lpp_print_str(const char*);
extern void lpp_write_str(const char*);
/* eprint_str is shim-only; provide the c_shim definition here. */
void lpp_eprint_str(const char *t){ if(!t) return; fputs(t, stderr); fputc('\n', stderr); }
int main(void){
    lpp_print_int(42);
    lpp_print_int(-7);
    lpp_print_int(9223372036854775807LL);
    lpp_print_str("hello");
    lpp_write_str("X");
    lpp_write_str("Y");
    lpp_print_bool(1);
    lpp_print_bool(0);
    lpp_print_float(3.5);
    lpp_print_float(-0.5);
    lpp_print_float(0.0);
    lpp_eprint_str("error-line");
    return 0;
}
"#;

/// Run the exact same IO calls through the Rust runtime, with fd 1 / fd 2
/// redirected to temp files, and return (stdout, stderr) as strings.
fn rust_io_capture(dir: &std::path::Path) -> (String, String) {
    use lpp_runtime::io::{
        lpp_eprint_str, lpp_print_bool, lpp_print_float, lpp_print_int, lpp_print_str,
        lpp_write_str,
    };

    let out_path = dir.join("rust_out.txt");
    let err_path = dir.join("rust_err.txt");
    let out_c = std::ffi::CString::new(out_path.to_str().unwrap()).unwrap();
    let err_c = std::ffi::CString::new(err_path.to_str().unwrap()).unwrap();

    unsafe {
        let saved_out = libc::dup(1);
        let saved_err = libc::dup(2);
        let out_fd = libc::open(
            out_c.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
            0o644 as libc::c_int,
        );
        let err_fd = libc::open(
            err_c.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
            0o644 as libc::c_int,
        );
        assert!(out_fd >= 0 && err_fd >= 0, "open temp files");
        libc::dup2(out_fd, 1);
        libc::dup2(err_fd, 2);

        lpp_print_int(42);
        lpp_print_int(-7);
        lpp_print_int(9223372036854775807);
        lpp_print_str(c"hello".as_ptr());
        lpp_write_str(c"X".as_ptr());
        lpp_write_str(c"Y".as_ptr());
        lpp_print_bool(1);
        lpp_print_bool(0);
        lpp_print_float(3.5);
        lpp_print_float(-0.5);
        lpp_print_float(0.0);
        lpp_eprint_str(c"error-line".as_ptr());

        libc::fflush(std::ptr::null_mut());
        libc::dup2(saved_out, 1);
        libc::dup2(saved_err, 2);
        libc::close(out_fd);
        libc::close(err_fd);
        libc::close(saved_out);
        libc::close(saved_err);
    }

    let out = std::fs::read_to_string(&out_path).unwrap();
    let err = std::fs::read_to_string(&err_path).unwrap();
    (out, err)
}

#[test]
fn io_matches_the_c_reference() {
    let cc = match Command::new("cc").arg("--version").output() {
        Ok(o) if o.status.success() => "cc",
        _ => {
            eprintln!("skipping: no C compiler available");
            return;
        }
    };
    let dir = std::env::temp_dir().join(format!("lpp-runtime-io-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Build + run the C reference scenario, capturing both streams.
    let scenario = dir.join("io_scenario.c");
    std::fs::write(&scenario, IO_SCENARIO).unwrap();
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bin = dir.join("io_c");
    let status = Command::new(cc)
        .args(["-O1", "-fno-stack-protector"])
        .arg(crate_root.join("lpp_runtime.c"))
        .arg(&scenario)
        .arg("-o")
        .arg(&bin)
        .arg("-lm")
        .status()
        .unwrap();
    if !status.success() {
        eprintln!("skipping: cc failed to build the C reference");
        return;
    }
    let c_out = Command::new(&bin).output().unwrap();
    let c_stdout = String::from_utf8_lossy(&c_out.stdout).to_string();
    let c_stderr = String::from_utf8_lossy(&c_out.stderr).to_string();

    // Run the same calls through the Rust runtime.
    let (r_stdout, r_stderr) = rust_io_capture(&dir);

    assert_eq!(
        r_stdout, c_stdout,
        "Rust IO stdout diverges from the C reference"
    );
    assert_eq!(
        r_stderr, c_stderr,
        "Rust IO stderr diverges from the C reference"
    );
    // Sanity: the streams are non-trivial (guards against both being empty).
    assert!(
        c_stdout.contains("9223372036854775807") && c_stdout.contains("3.500000"),
        "C stdout sanity"
    );
    assert_eq!(c_stderr, "error-line\n", "C stderr sanity");
}
