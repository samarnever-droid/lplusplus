use super::*;

/// Escape a Rust string into an LLVM `c"..."` literal body (including the
/// terminating NUL).
pub(super) fn llvm_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 4);
    for byte in text.bytes() {
        match byte {
            b'\n' => out.push_str("\\0A"),
            b'\t' => out.push_str("\\09"),
            b'\r' => out.push_str("\\0D"),
            b'"' => out.push_str("\\22"),
            b'\\' => out.push_str("\\5C"),
            b'%' => out.push_str("\\25"),
            0x20..=0x7e => out.push(byte as char),
            other => out.push_str(&format!("\\{other:02X}")),
        }
    }
    out.push_str("\\00");
    out
}

pub(super) fn llvm_arc_string(text: &str) -> String {
    // LPP_ARC_IMMORTAL is 0x41524331 on both `magic` and `refcount`;
    // LLVM's c-string bytes follow the little-endian x86_64 target.
    let mut out = String::from("\\31\\43\\52\\41\\31\\43\\52\\41");
    out.push_str("\\00\\00\\00\\00\\00\\00\\00\\00");
    out.push_str(&llvm_string(text));
    out
}
