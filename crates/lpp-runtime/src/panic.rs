//! The runtime panic path (Phase 6B.2).
//!
//! Mirrors the v1 C `lpp_panic`: flush stdout, print the banner to stderr,
//! then `exit(101)`. It is internal to the runtime — generated objects
//! reach it only through the runtime's own bounds checks (the C symbol is
//! variadic and not part of the `c_shim` link surface), so this is a plain
//! Rust `-> !` function rather than an exported C ABI entry.

use std::io::Write;

/// Print the C-style panic banner and terminate with exit code 101.
///
/// The banner format matches `lpp_runtime.c::lpp_panic` so differential
/// gates can compare stderr between the two runtimes.
pub fn runtime_panic(reason: &str) -> ! {
    let _ = std::io::stdout().flush();
    let mut err = std::io::stderr();
    let _ = writeln!(
        err,
        "\n==================================================================="
    );
    let _ = writeln!(err, "\u{1f4a5} L++ RUNTIME PANIC");
    let _ = writeln!(
        err,
        "==================================================================="
    );
    let _ = writeln!(err, "Reason: {reason}");
    let _ = writeln!(
        err,
        "===================================================================\n"
    );
    let _ = err.flush();
    let _ = std::io::stdout().flush();
    std::process::exit(101);
}
