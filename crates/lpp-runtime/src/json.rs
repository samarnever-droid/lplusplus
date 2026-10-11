//! JSON parsing, querying, and escaping builtins for `lpp-runtime`.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use libc::c_char;

use crate::arc::lpp_empty_str;
use crate::string::arc_string;

const MAX_DEPTH: usize = 256;

#[derive(Debug, Clone)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Vec<u8>),
    Array(Vec<JsonValue>),
    Object(Vec<(Vec<u8>, JsonValue)>),
}

fn root_registry() -> &'static Mutex<HashSet<usize>> {
    static ROOTS: OnceLock<Mutex<HashSet<usize>>> = OnceLock::new();
    ROOTS.get_or_init(|| Mutex::new(HashSet::new()))
}

unsafe fn cstr_bytes<'a>(s: *const c_char) -> &'a [u8] {
    if s.is_null() {
        return &[];
    }
    unsafe { std::slice::from_raw_parts(s as *const u8, libc::strlen(s)) }
}

fn skip_ws(input: &[u8], pos: &mut usize) {
    while *pos < input.len() && matches!(input[*pos], b' ' | b'\t' | b'\r' | b'\n') {
        *pos += 1;
    }
}

fn parse_string(input: &[u8], pos: &mut usize) -> Option<Vec<u8>> {
    skip_ws(input, pos);
    if *pos >= input.len() || input[*pos] != b'"' {
        return None;
    }
    *pos += 1;
    let mut out = Vec::new();
    while *pos < input.len() {
        let b = input[*pos];
        *pos += 1;
        if b == b'"' {
            return Some(out);
        }
        if b == b'\\' {
            if *pos >= input.len() {
                return None;
            }
            let esc = input[*pos];
            *pos += 1;
            match esc {
                b'"' => out.push(b'"'),
                b'\\' => out.push(b'\\'),
                b'/' => out.push(b'/'),
                b'b' => out.push(0x08),
                b'f' => out.push(0x0c),
                b'n' => out.push(b'\n'),
                b'r' => out.push(b'\r'),
                b't' => out.push(b'\t'),
                b'u' => {
                    if *pos + 4 > input.len() {
                        return None;
                    }
                    let hex = std::str::from_utf8(&input[*pos..*pos + 4]).ok()?;
                    let code = u32::from_str_radix(hex, 16).ok()?;
                    *pos += 4;
                    if let Some(ch) = char::from_u32(code) {
                        let mut buf = [0u8; 4];
                        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                    }
                }
                other => out.push(other),
            }
        } else {
            out.push(b);
        }
    }
    None
}

fn parse_number(input: &[u8], pos: &mut usize) -> Option<JsonValue> {
    let start = *pos;
    if *pos < input.len() && input[*pos] == b'-' {
        *pos += 1;
    }
    let mut is_float = false;
    while *pos < input.len() && input[*pos].is_ascii_digit() {
        *pos += 1;
    }
    if *pos < input.len() && input[*pos] == b'.' {
        is_float = true;
        *pos += 1;
        while *pos < input.len() && input[*pos].is_ascii_digit() {
            *pos += 1;
        }
    }
    if *pos < input.len() && (input[*pos] == b'e' || input[*pos] == b'E') {
        is_float = true;
        *pos += 1;
        if *pos < input.len() && (input[*pos] == b'+' || input[*pos] == b'-') {
            *pos += 1;
        }
        while *pos < input.len() && input[*pos].is_ascii_digit() {
            *pos += 1;
        }
    }
    let s = std::str::from_utf8(&input[start..*pos]).ok()?;
    if is_float {
        s.parse::<f64>().ok().map(JsonValue::Float)
    } else {
        s.parse::<i64>().ok().map(JsonValue::Int)
    }
}

fn parse_value(input: &[u8], pos: &mut usize, depth: usize) -> Option<JsonValue> {
    if depth > MAX_DEPTH {
        return None;
    }
    skip_ws(input, pos);
    if *pos >= input.len() {
        return None;
    }
    match input[*pos] {
        b'{' => parse_object(input, pos, depth + 1),
        b'[' => parse_array(input, pos, depth + 1),
        b'"' => parse_string(input, pos).map(JsonValue::Str),
        b't' if input[*pos..].starts_with(b"true") => {
            *pos += 4;
            Some(JsonValue::Bool(true))
        }
        b'f' if input[*pos..].starts_with(b"false") => {
            *pos += 5;
            Some(JsonValue::Bool(false))
        }
        b'n' if input[*pos..].starts_with(b"null") => {
            *pos += 4;
            Some(JsonValue::Null)
        }
        b'-' | b'0'..=b'9' => parse_number(input, pos),
        _ => None,
    }
}

fn parse_array(input: &[u8], pos: &mut usize, depth: usize) -> Option<JsonValue> {
    skip_ws(input, pos);
    if *pos >= input.len() || input[*pos] != b'[' {
        return None;
    }
    *pos += 1;
    let mut items = Vec::new();
    loop {
        skip_ws(input, pos);
        if *pos >= input.len() {
            return None;
        }
        if input[*pos] == b']' {
            *pos += 1;
            return Some(JsonValue::Array(items));
        }
        let val = parse_value(input, pos, depth)?;
        items.push(val);
        skip_ws(input, pos);
        if *pos < input.len() && input[*pos] == b',' {
            *pos += 1;
        } else if *pos < input.len() && input[*pos] == b']' {
            *pos += 1;
            return Some(JsonValue::Array(items));
        } else {
            return None;
        }
    }
}

fn parse_object(input: &[u8], pos: &mut usize, depth: usize) -> Option<JsonValue> {
    skip_ws(input, pos);
    if *pos >= input.len() || input[*pos] != b'{' {
        return None;
    }
    *pos += 1;
    let mut entries = Vec::new();
    loop {
        skip_ws(input, pos);
        if *pos >= input.len() {
            break;
        }
        if input[*pos] == b'}' {
            *pos += 1;
            break;
        }
        let Some(key) = parse_string(input, pos) else {
            break;
        };
        skip_ws(input, pos);
        if *pos >= input.len() || input[*pos] != b':' {
            break;
        }
        *pos += 1;
        if let Some(val) = parse_value(input, pos, depth) {
            entries.push((key, val));
        }
        skip_ws(input, pos);
        if *pos < input.len() && input[*pos] == b',' {
            *pos += 1;
        } else if *pos < input.len() && input[*pos] == b'}' {
            *pos += 1;
            break;
        } else {
            break;
        }
    }
    Some(JsonValue::Object(entries))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_json_parse(json: *const c_char) -> i64 {
    if json.is_null() {
        return 0;
    }
    let bytes = unsafe { cstr_bytes(json) };
    let mut pos = 0usize;
    let Some(val) = parse_value(bytes, &mut pos, 0) else {
        return 0;
    };
    let ptr = Box::into_raw(Box::new(val)) as usize;
    if let Ok(mut roots) = root_registry().lock() {
        roots.insert(ptr);
    }
    ptr as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_json_get_int(handle: i64, key: *const c_char) -> i64 {
    if handle == 0 || key.is_null() {
        return 0;
    }
    let key_bytes = unsafe { cstr_bytes(key) };
    let node = unsafe { &*(handle as usize as *const JsonValue) };
    if let JsonValue::Object(entries) = node {
        for (k, v) in entries {
            if k.as_slice() == key_bytes {
                return match v {
                    JsonValue::Int(n) => *n,
                    JsonValue::Float(f) => *f as i64,
                    JsonValue::Bool(b) => i64::from(*b),
                    _ => 0,
                };
            }
        }
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_json_get_str(handle: i64, key: *const c_char) -> *mut c_char {
    if handle == 0 || key.is_null() {
        return unsafe { lpp_empty_str() as *mut c_char };
    }
    let key_bytes = unsafe { cstr_bytes(key) };
    let node = unsafe { &*(handle as usize as *const JsonValue) };
    if let JsonValue::Object(entries) = node {
        for (k, v) in entries {
            if k.as_slice() == key_bytes {
                if let JsonValue::Str(s) = v {
                    return unsafe { arc_string(s) };
                }
                return unsafe { lpp_empty_str() as *mut c_char };
            }
        }
    }
    unsafe { lpp_empty_str() as *mut c_char }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_json_get_obj(handle: i64, key: *const c_char) -> i64 {
    if handle == 0 || key.is_null() {
        return 0;
    }
    let key_bytes = unsafe { cstr_bytes(key) };
    let node = unsafe { &*(handle as usize as *const JsonValue) };
    if let JsonValue::Object(entries) = node {
        for (k, v) in entries {
            if k.as_slice() == key_bytes {
                if matches!(v, JsonValue::Object(_) | JsonValue::Array(_)) {
                    return (v as *const JsonValue) as usize as i64;
                }
                return 0;
            }
        }
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_json_free(handle: i64) {
    if handle == 0 {
        return;
    }
    let addr = handle as usize;
    let is_root = root_registry()
        .lock()
        .map(|mut roots| roots.remove(&addr))
        .unwrap_or(false);
    if is_root {
        unsafe {
            drop(Box::from_raw(addr as *mut JsonValue));
        }
    }
}

/// `json_escape(s)` — escape a string for safe embedding inside a JSON string literal
/// (without surrounding quotes).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_json_escape(s: *const c_char) -> *mut c_char {
    let bytes = unsafe { cstr_bytes(s) };
    let mut out = Vec::with_capacity(bytes.len());
    for &b in bytes {
        match b {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            0x08 => out.extend_from_slice(b"\\b"),
            0x0c => out.extend_from_slice(b"\\f"),
            0x00..=0x1f => {
                let esc = format!("\\u{:04x}", b);
                out.extend_from_slice(esc.as_bytes());
            }
            other => out.push(other),
        }
    }
    unsafe { arc_string(&out) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arc::lpp_arc_release;
    use crate::clock_rng::{
        lpp_c_free, lpp_c_load_i64, lpp_c_load_u8, lpp_c_malloc, lpp_c_store_i64, lpp_c_store_u8,
        lpp_sys_mem_free, lpp_sys_mem_total, lpp_sys_uptime, lpp_time_ns,
    };
    use crate::io::{
        lpp_args, lpp_args_count, lpp_args_get, lpp_cwd, lpp_fd_size, lpp_file_close,
        lpp_file_ftruncate, lpp_file_open, lpp_file_pread, lpp_file_pwrite, lpp_is_dir,
        lpp_is_file,
    };
    use crate::list::{lpp_list_get_arc, lpp_list_len};
    use crate::string::{
        lpp_str_join, lpp_str_len, lpp_str_lines, lpp_str_pad_left, lpp_str_pad_right,
    };
    use std::ffi::{CStr, CString};

    #[test]
    fn json_parse_and_escape_roundtrip() {
        unsafe {
            let input =
                CString::new(r#"{"count": 42, "name": "lpp\nfast", "nested": {"ok": true}}"#)
                    .unwrap();
            let root = lpp_json_parse(input.as_ptr());
            assert_ne!(root, 0);

            let key_count = CString::new("count").unwrap();
            assert_eq!(lpp_json_get_int(root, key_count.as_ptr()), 42);

            let key_name = CString::new("name").unwrap();
            let name_ptr = lpp_json_get_str(root, key_name.as_ptr());
            assert_eq!(CStr::from_ptr(name_ptr).to_str().unwrap(), "lpp\nfast");
            lpp_arc_release(name_ptr.cast());

            let key_nested = CString::new("nested").unwrap();
            let nested = lpp_json_get_obj(root, key_nested.as_ptr());
            assert_ne!(nested, 0);
            let key_ok = CString::new("ok").unwrap();
            assert_eq!(lpp_json_get_int(nested, key_ok.as_ptr()), 1);

            lpp_json_free(nested); // non-root free is a safe no-op
            lpp_json_free(root);

            let raw = CString::new("a\"b\\c\nd").unwrap();
            let esc = lpp_json_escape(raw.as_ptr());
            assert_eq!(CStr::from_ptr(esc).to_str().unwrap(), r#"a\"b\\c\nd"#);
            lpp_arc_release(esc.cast());
        }
    }

    #[test]
    fn cli_fs_string_and_sys_builtins_work() {
        unsafe {
            assert!(lpp_args_count() >= 1);
            let arg0 = lpp_args_get(0);
            assert!(lpp_str_len(arg0) > 0);
            lpp_arc_release(arg0.cast());

            let all = lpp_args();
            assert!(lpp_list_len(all) >= 1);
            lpp_arc_release(all.cast());

            let lines_src = CString::new("alpha\r\nbeta\ngamma").unwrap();
            let lines_list = lpp_str_lines(lines_src.as_ptr());
            assert_eq!(lpp_list_len(lines_list), 3);
            let sep = CString::new("|").unwrap();
            let joined = lpp_str_join(lines_list, sep.as_ptr());
            assert_eq!(CStr::from_ptr(joined).to_str().unwrap(), "alpha|beta|gamma");
            lpp_arc_release(joined.cast());
            let first = lpp_list_get_arc(lines_list, 0) as *mut c_char;
            assert_eq!(CStr::from_ptr(first).to_str().unwrap(), "alpha");
            lpp_arc_release(lines_list.cast());

            let s = CString::new("7").unwrap();
            let pad = CString::new("0").unwrap();
            let pl = lpp_str_pad_left(s.as_ptr(), 4, pad.as_ptr());
            let pr = lpp_str_pad_right(s.as_ptr(), 4, pad.as_ptr());
            assert_eq!(CStr::from_ptr(pl).to_str().unwrap(), "0007");
            assert_eq!(CStr::from_ptr(pr).to_str().unwrap(), "7000");
            lpp_arc_release(pl.cast());
            lpp_arc_release(pr.cast());

            let cwd = lpp_cwd();
            assert_eq!(lpp_is_dir(cwd), 1);
            assert_eq!(lpp_is_file(cwd), 0);
            lpp_arc_release(cwd.cast());

            let tmp = std::env::temp_dir().join(format!("lpp_fd_test_{}.bin", std::process::id()));
            let tmp_c = CString::new(tmp.to_string_lossy().as_bytes()).unwrap();
            let fd = lpp_file_open(tmp_c.as_ptr(), 1);
            assert!(fd >= 0);
            let payload = b"hello_pread_pwrite";
            let wbuf = crate::buf::lpp_buf_alloc(payload.len() as i64);
            for (i, &b) in payload.iter().enumerate() {
                crate::buf::lpp_buf_set8(wbuf, i as i64, i64::from(b));
            }
            assert_eq!(
                lpp_file_pwrite(fd, wbuf, 0, 0, payload.len() as i64),
                payload.len() as i64
            );
            assert_eq!(lpp_fd_size(fd), payload.len() as i64);
            let rbuf = crate::buf::lpp_buf_alloc(18);
            assert_eq!(lpp_file_pread(fd, rbuf, 0, 0, 18), 18);
            for (i, &b) in payload.iter().enumerate() {
                assert_eq!(crate::buf::lpp_buf_get8(rbuf, i as i64), i64::from(b));
            }
            crate::buf::lpp_buf_free(wbuf);
            crate::buf::lpp_buf_free(rbuf);
            assert_eq!(lpp_file_ftruncate(fd, 5), 0);
            assert_eq!(lpp_fd_size(fd), 5);
            lpp_file_close(fd);
            let _ = std::fs::remove_file(&tmp);

            assert!(lpp_time_ns() > 0);
            assert!(lpp_sys_mem_total() > 0);
            assert!(lpp_sys_mem_free() > 0);
            assert!(lpp_sys_uptime() >= 0);

            let raw_mem = lpp_c_malloc(16);
            assert_ne!(raw_mem, 0);
            lpp_c_store_u8(raw_mem, 0, 0xAB);
            assert_eq!(lpp_c_load_u8(raw_mem, 0), 0xAB);
            lpp_c_store_i64(raw_mem, 8, 123456789);
            assert_eq!(lpp_c_load_i64(raw_mem, 8), 123456789);
            lpp_c_free(raw_mem);
        }
    }
}
