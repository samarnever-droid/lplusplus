// ── Integer / layout helpers ───────────────────────────────────────────────

fn put_u16(buf: &mut [u8], offset: usize, value: u16) {
    buf[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
fn put_u32(buf: &mut [u8], offset: usize, value: u32) {
    buf[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn put_u64(buf: &mut [u8], offset: usize, value: u64) {
    buf[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}
fn put_u16be(buf: &mut [u8], offset: usize, value: u16) {
    buf[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}
fn put_u32be(buf: &mut [u8], offset: usize, value: u32) {
    buf[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

/// Power-of-two alignment. Saturates on overflow so a later bounds check fails
/// instead of wrapping into a plausible-looking address.
fn align_up(value: usize, alignment: usize) -> usize {
    debug_assert!(alignment <= 1 || alignment.is_power_of_two());
    if alignment <= 1 {
        return value;
    }
    match value.checked_add(alignment - 1) {
        Some(v) => v & !(alignment - 1),
        None => usize::MAX,
    }
}

fn align_up_u64(value: u64, alignment: u64) -> u64 {
    if alignment <= 1 {
        return value;
    }
    debug_assert!(alignment.is_power_of_two());
    match value.checked_add(alignment - 1) {
        Some(v) => v & !(alignment - 1),
        None => u64::MAX,
    }
}

/// ELF / PE identity: file offset and virtual address must be congruent
/// modulo the page/section alignment.
fn congruent_offset(vaddr: u64, align: u64) -> u64 {
    if align == 0 {
        return 0;
    }
    vaddr % align
}

fn fits_i8(v: i64) -> bool {
    v >= i8::MIN as i64 && v <= i8::MAX as i64
}
fn fits_i16(v: i64) -> bool {
    v >= i16::MIN as i64 && v <= i16::MAX as i64
}
fn fits_i26(v: i64) -> bool {
    v >= -(1 << 25) && v < (1 << 25)
}
fn fits_i32(v: i64) -> bool {
    v >= i32::MIN as i64 && v <= i32::MAX as i64
}
fn fits_u32(v: i64) -> bool {
    v >= 0 && v <= u32::MAX as i64
}

fn read_i32_at(buf: &[u8], off: usize) -> Result<i32, LinkError> {
    let b: [u8; 4] = buf
        .get(off..off + 4)
        .ok_or_else(|| format!("read i32 at {off} out of range"))?
        .try_into()
        .map_err(|_| "read i32".to_string())?;
    Ok(i32::from_le_bytes(b))
}
fn read_i64_at(buf: &[u8], off: usize) -> Result<i64, LinkError> {
    let b: [u8; 8] = buf
        .get(off..off + 8)
        .ok_or_else(|| format!("read i64 at {off} out of range"))?
        .try_into()
        .map_err(|_| "read i64".to_string())?;
    Ok(i64::from_le_bytes(b))
}
fn write_i8_at(buf: &mut [u8], off: usize, v: i64, ctx: &str) -> Result<(), LinkError> {
    if !fits_i8(v) {
        return Err(format!("{ctx}: 8-bit relocation overflow ({v})").into());
    }
    *buf.get_mut(off)
        .ok_or_else(|| format!("{ctx}: patch OOB"))? = v as u8;
    Ok(())
}
fn write_i16_at(buf: &mut [u8], off: usize, v: i64, ctx: &str) -> Result<(), LinkError> {
    if !fits_i16(v) {
        return Err(format!("{ctx}: 16-bit relocation overflow ({v})").into());
    }
    let slot = buf
        .get_mut(off..off + 2)
        .ok_or_else(|| format!("{ctx}: patch OOB"))?;
    slot.copy_from_slice(&(v as i16).to_le_bytes());
    Ok(())
}
fn write_i32_at(buf: &mut [u8], off: usize, v: i64, ctx: &str) -> Result<(), LinkError> {
    if !fits_i32(v) {
        return Err(format!("{ctx}: 32-bit relocation overflow ({v})",).into());
    }
    let slot = buf
        .get_mut(off..off + 4)
        .ok_or_else(|| format!("{ctx}: patch OOB"))?;
    slot.copy_from_slice(&(v as i32).to_le_bytes());
    Ok(())
}
fn write_u32_at(buf: &mut [u8], off: usize, v: i64, ctx: &str) -> Result<(), LinkError> {
    if !fits_u32(v) {
        return Err(format!("{ctx}: unsigned 32-bit relocation overflow ({v})").into());
    }
    let slot = buf
        .get_mut(off..off + 4)
        .ok_or_else(|| format!("{ctx}: patch OOB"))?;
    slot.copy_from_slice(&(v as u32).to_le_bytes());
    Ok(())
}
fn write_u64_at(buf: &mut [u8], off: usize, v: u64, ctx: &str) -> Result<(), LinkError> {
    let slot = buf
        .get_mut(off..off + 8)
        .ok_or_else(|| format!("{ctx}: patch OOB"))?;
    slot.copy_from_slice(&v.to_le_bytes());
    Ok(())
}
fn write_u32_raw(buf: &mut [u8], off: usize, v: u32, ctx: &str) -> Result<(), LinkError> {
    let slot = buf
        .get_mut(off..off + 4)
        .ok_or_else(|| format!("{ctx}: patch OOB"))?;
    slot.copy_from_slice(&v.to_le_bytes());
    Ok(())
}

fn is_archive_bytes(bytes: &[u8]) -> bool {
    bytes.starts_with(b"!<arch>\n")
}

fn chmod_exec(output: &Path) -> Result<(), LinkError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = fs::metadata(output)
            .map_err(|e| format!("stat '{}': {e}", output.display()))?
            .permissions();
        perm.set_mode(0o755);
        fs::set_permissions(output, perm)
            .map_err(|e| format!("chmod '{}': {e}", output.display()))?;
    }
    let _ = output;
    Ok(())
}

// ── SHA-256 (ad-hoc Mach-O codesign + GNU build-id) ────────────────────────

fn sha256(data: &[u8]) -> [u8; 32] {
    fn rotr(x: u32, n: u32) -> u32 {
        x.rotate_right(n)
    }
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >> 3);
            let s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];
        for i in 0..64 {
            let s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 32];
    for (i, w) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
    }
    out
}

/// SysV ELF `elf_hash` used by `SHT_HASH`.
fn elf_hash(name: &[u8]) -> u32 {
    let mut h: u32 = 0;
    for &c in name {
        h = (h << 4).wrapping_add(c as u32);
        let g = h & 0xf0000000;
        if g != 0 {
            h ^= g >> 24;
        }
        h &= !g;
    }
    h
}

/// Microsoft PE checksum: 16-bit one's-complement sum of the image, then
/// add the file length. The checksum field itself is skipped.
fn pe_checksum(image: &[u8], checksum_off: usize) -> u32 {
    let mut sum: u64 = 0;
    let mut i = 0usize;
    while i + 1 < image.len() {
        if i == checksum_off {
            i += 4;
            continue;
        }
        if i + 1 == checksum_off {
            // odd alignment should not happen; treat conservatively
            i += 1;
            continue;
        }
        let word = u16::from_le_bytes([image[i], image[i + 1]]) as u64;
        sum = sum.wrapping_add(word);
        sum = (sum & 0xffff) + (sum >> 16);
        i += 2;
    }
    if i < image.len() && i != checksum_off && i + 1 != checksum_off {
        sum = sum.wrapping_add(image[i] as u64);
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    (sum as u32).wrapping_add(image.len() as u32)
}

fn page(addr: u64) -> u64 {
    addr & !0xfffu64
}

fn fnv1a64(data: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

