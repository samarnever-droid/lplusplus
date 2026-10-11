//! Binary buffer builtins (`runtime/lpp_buf.c` port).
//!
//! Layout: `[8-byte i64 size][raw data bytes...]`.
//! A buffer handle (`i64`) points to the start of the 8-byte size header.

use libc::{c_char, c_void, calloc, free, strlen};

use crate::arc::lpp_empty_str;
use crate::string::arc_string;

#[inline]
unsafe fn buf_size(ptr: *const u8) -> i64 {
    unsafe { *(ptr as *const i64) }
}

#[inline]
unsafe fn buf_data(ptr: *const u8) -> *const u8 {
    unsafe { ptr.add(8) }
}

#[inline]
unsafe fn buf_data_mut(ptr: *mut u8) -> *mut u8 {
    unsafe { ptr.add(8) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_buf_alloc(size: i64) -> i64 {
    if size < 0 {
        return 0;
    }
    let total = match (size as usize).checked_add(8) {
        Some(t) => t,
        None => return 0,
    };
    let buf = unsafe { calloc(1, total) } as *mut u8;
    if buf.is_null() {
        return 0;
    }
    unsafe {
        *(buf as *mut i64) = size;
    }
    buf as usize as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_buf_free(ptr: i64) {
    if ptr != 0 {
        unsafe { free(ptr as usize as *mut c_void) };
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_buf_len(ptr: i64) -> i64 {
    if ptr == 0 {
        return 0;
    }
    unsafe { buf_size(ptr as usize as *const u8) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_buf_get8(ptr: i64, offset: i64) -> i64 {
    if ptr == 0 || offset < 0 {
        return 0;
    }
    let p = ptr as usize as *const u8;
    let size = unsafe { buf_size(p) };
    if offset >= size {
        return 0;
    }
    unsafe { i64::from(*buf_data(p).add(offset as usize)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_buf_set8(ptr: i64, offset: i64, value: i64) {
    if ptr == 0 || offset < 0 {
        return;
    }
    let p = ptr as usize as *mut u8;
    let size = unsafe { buf_size(p) };
    if offset >= size {
        return;
    }
    unsafe {
        *buf_data_mut(p).add(offset as usize) = (value & 0xFF) as u8;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_buf_set32le(ptr: i64, offset: i64, value: i64) {
    if ptr == 0 || offset < 0 {
        return;
    }
    let p = ptr as usize as *mut u8;
    let size = unsafe { buf_size(p) };
    if offset.saturating_add(4) > size {
        return;
    }
    let bytes = (value as u32).to_le_bytes();
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf_data_mut(p).add(offset as usize), 4);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_buf_get32le(ptr: i64, offset: i64) -> i64 {
    if ptr == 0 || offset < 0 {
        return 0;
    }
    let p = ptr as usize as *const u8;
    let size = unsafe { buf_size(p) };
    if offset.saturating_add(4) > size {
        return 0;
    }
    let mut bytes = [0u8; 4];
    unsafe {
        std::ptr::copy_nonoverlapping(buf_data(p).add(offset as usize), bytes.as_mut_ptr(), 4);
    }
    i64::from(u32::from_le_bytes(bytes))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_buf_set16le(ptr: i64, offset: i64, value: i64) {
    if ptr == 0 || offset < 0 {
        return;
    }
    let p = ptr as usize as *mut u8;
    let size = unsafe { buf_size(p) };
    if offset.saturating_add(2) > size {
        return;
    }
    let bytes = (value as u16).to_le_bytes();
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf_data_mut(p).add(offset as usize), 2);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_buf_get16le(ptr: i64, offset: i64) -> i64 {
    if ptr == 0 || offset < 0 {
        return 0;
    }
    let p = ptr as usize as *const u8;
    let size = unsafe { buf_size(p) };
    if offset.saturating_add(2) > size {
        return 0;
    }
    let mut bytes = [0u8; 2];
    unsafe {
        std::ptr::copy_nonoverlapping(buf_data(p).add(offset as usize), bytes.as_mut_ptr(), 2);
    }
    i64::from(u16::from_le_bytes(bytes))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_buf_set16be(ptr: i64, offset: i64, value: i64) {
    if ptr == 0 || offset < 0 {
        return;
    }
    let p = ptr as usize as *mut u8;
    let size = unsafe { buf_size(p) };
    if offset.saturating_add(2) > size {
        return;
    }
    let bytes = (value as u16).to_be_bytes();
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf_data_mut(p).add(offset as usize), 2);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_buf_get16be(ptr: i64, offset: i64) -> i64 {
    if ptr == 0 || offset < 0 {
        return 0;
    }
    let p = ptr as usize as *const u8;
    let size = unsafe { buf_size(p) };
    if offset.saturating_add(2) > size {
        return 0;
    }
    let mut bytes = [0u8; 2];
    unsafe {
        std::ptr::copy_nonoverlapping(buf_data(p).add(offset as usize), bytes.as_mut_ptr(), 2);
    }
    i64::from(u16::from_be_bytes(bytes))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_buf_set32be(ptr: i64, offset: i64, value: i64) {
    if ptr == 0 || offset < 0 {
        return;
    }
    let p = ptr as usize as *mut u8;
    let size = unsafe { buf_size(p) };
    if offset.saturating_add(4) > size {
        return;
    }
    let bytes = (value as u32).to_be_bytes();
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf_data_mut(p).add(offset as usize), 4);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_buf_get32be(ptr: i64, offset: i64) -> i64 {
    if ptr == 0 || offset < 0 {
        return 0;
    }
    let p = ptr as usize as *const u8;
    let size = unsafe { buf_size(p) };
    if offset.saturating_add(4) > size {
        return 0;
    }
    let mut bytes = [0u8; 4];
    unsafe {
        std::ptr::copy_nonoverlapping(buf_data(p).add(offset as usize), bytes.as_mut_ptr(), 4);
    }
    i64::from(u32::from_be_bytes(bytes))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_buf_set64le(ptr: i64, offset: i64, value: i64) {
    if ptr == 0 || offset < 0 {
        return;
    }
    let p = ptr as usize as *mut u8;
    let size = unsafe { buf_size(p) };
    if offset.saturating_add(8) > size {
        return;
    }
    let bytes = (value as u64).to_le_bytes();
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf_data_mut(p).add(offset as usize), 8);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_buf_get64le(ptr: i64, offset: i64) -> i64 {
    if ptr == 0 || offset < 0 {
        return 0;
    }
    let p = ptr as usize as *const u8;
    let size = unsafe { buf_size(p) };
    if offset.saturating_add(8) > size {
        return 0;
    }
    let mut bytes = [0u8; 8];
    unsafe {
        std::ptr::copy_nonoverlapping(buf_data(p).add(offset as usize), bytes.as_mut_ptr(), 8);
    }
    u64::from_le_bytes(bytes) as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_buf_set64be(ptr: i64, offset: i64, value: i64) {
    if ptr == 0 || offset < 0 {
        return;
    }
    let p = ptr as usize as *mut u8;
    let size = unsafe { buf_size(p) };
    if offset.saturating_add(8) > size {
        return;
    }
    let bytes = (value as u64).to_be_bytes();
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf_data_mut(p).add(offset as usize), 8);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_buf_get64be(ptr: i64, offset: i64) -> i64 {
    if ptr == 0 || offset < 0 {
        return 0;
    }
    let p = ptr as usize as *const u8;
    let size = unsafe { buf_size(p) };
    if offset.saturating_add(8) > size {
        return 0;
    }
    let mut bytes = [0u8; 8];
    unsafe {
        std::ptr::copy_nonoverlapping(buf_data(p).add(offset as usize), bytes.as_mut_ptr(), 8);
    }
    u64::from_be_bytes(bytes) as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_buf_copy(dst: i64, dst_off: i64, src: i64, src_off: i64, len: i64) {
    if dst == 0 || src == 0 || len <= 0 || dst_off < 0 || src_off < 0 {
        return;
    }
    let d = dst as usize as *mut u8;
    let s = src as usize as *const u8;
    let dst_size = unsafe { buf_size(d) };
    let src_size = unsafe { buf_size(s) };
    if dst_off.saturating_add(len) > dst_size || src_off.saturating_add(len) > src_size {
        return;
    }
    unsafe {
        std::ptr::copy(
            buf_data(s).add(src_off as usize),
            buf_data_mut(d).add(dst_off as usize),
            len as usize,
        );
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_buf_read(path: *const c_char) -> i64 {
    if path.is_null() {
        return 0;
    }
    let path_bytes = unsafe { std::slice::from_raw_parts(path as *const u8, strlen(path)) };
    let path_str = String::from_utf8_lossy(path_bytes);
    let data = match std::fs::read(path_str.as_ref()) {
        Ok(d) => d,
        Err(_) => return 0,
    };
    let buf = unsafe { lpp_buf_alloc(data.len() as i64) };
    if buf == 0 {
        return 0;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(
            data.as_ptr(),
            buf_data_mut(buf as usize as *mut u8),
            data.len(),
        );
    }
    buf
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_buf_write(path: *const c_char, ptr: i64) -> i64 {
    if path.is_null() || ptr == 0 {
        return -1;
    }
    let path_bytes = unsafe { std::slice::from_raw_parts(path as *const u8, strlen(path)) };
    let path_str = String::from_utf8_lossy(path_bytes);
    let p = ptr as usize as *const u8;
    let size = unsafe { buf_size(p) };
    if size < 0 {
        return -1;
    }
    let slice = unsafe { std::slice::from_raw_parts(buf_data(p), size as usize) };
    if let Some(parent) = std::path::Path::new(path_str.as_ref()).parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }
    match std::fs::write(path_str.as_ref(), slice) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_buf_crc32(ptr: i64, off: i64, len: i64) -> i64 {
    if ptr == 0 || off < 0 || len < 0 {
        return 0;
    }
    let p = ptr as usize as *const u8;
    let size = unsafe { buf_size(p) };
    if off.saturating_add(len) > size {
        return 0;
    }
    let data = unsafe { std::slice::from_raw_parts(buf_data(p).add(off as usize), len as usize) };
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        let mut entry = (crc ^ u32::from(byte)) & 0xFF;
        for _ in 0..8 {
            if entry & 1 != 0 {
                entry = (entry >> 1) ^ 0xEDB8_8320;
            } else {
                entry >>= 1;
            }
        }
        crc = entry ^ (crc >> 8);
    }
    i64::from(crc ^ 0xFFFF_FFFF)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_buf_to_str(ptr: i64, off: i64, len: i64) -> *mut c_char {
    if ptr == 0 || off < 0 || len < 0 {
        return unsafe { lpp_empty_str() } as *mut c_char;
    }
    let p = ptr as usize as *const u8;
    let size = unsafe { buf_size(p) };
    if off.saturating_add(len) > size {
        return unsafe { lpp_empty_str() } as *mut c_char;
    }
    let data = unsafe { std::slice::from_raw_parts(buf_data(p).add(off as usize), len as usize) };
    unsafe { arc_string(data) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_buf_read_str(ptr: i64, offset: i64, len: i64) -> *mut c_char {
    unsafe { lpp_buf_to_str(ptr, offset, len) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn lpp_buf_write_str(ptr: i64, offset: i64, str_ptr: *const c_char) -> i64 {
    if ptr == 0 || str_ptr.is_null() || offset < 0 {
        return -1;
    }
    let p = ptr as usize as *mut u8;
    let size = unsafe { buf_size(p) };
    let len = unsafe { strlen(str_ptr) } as i64;
    if offset.saturating_add(len) > size {
        return -1;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(
            str_ptr as *const u8,
            buf_data_mut(p).add(offset as usize),
            len as usize,
        );
    }
    len
}
