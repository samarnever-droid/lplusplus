// ── AArch64 instruction patching (ELF / PE / Mach-O) ───────────────────────

fn a64_read(buf: &[u8], off: usize) -> Result<u32, LinkError> {
    let b: [u8; 4] = buf
        .get(off..off + 4)
        .ok_or_else(|| "aarch64 patch OOB".to_string())?
        .try_into()
        .map_err(|_| "aarch64 patch".to_string())?;
    Ok(u32::from_le_bytes(b))
}
fn a64_write(buf: &mut [u8], off: usize, instr: u32) -> Result<(), LinkError> {
    write_u32_raw(buf, off, instr, "aarch64")
}

fn a64_patch_call26(buf: &mut [u8], off: usize, s: u64, a: i64, p: u64) -> Result<(), LinkError> {
    let instr = a64_read(buf, off)?;
    let raw_imm = (instr & 0x03FF_FFFF) as i32;
    let inline_addend = if (raw_imm & 0x0200_0000) != 0 {
        ((raw_imm | !0x03FF_FFFF) as i64) << 2
    } else {
        (raw_imm as i64) << 2
    };
    let addend = if a != 0 {
        (a as i32) as i64
    } else {
        inline_addend
    };
    let dest = s.wrapping_add_signed(addend);
    let disp = dest as i64 - p as i64;
    if disp & 3 != 0 {
        return Err("aarch64 CALL26/JUMP26 is not 4-byte aligned".into());
    }
    let imm = disp >> 2;
    if !fits_i26(imm) {
        return Err(format!("aarch64 CALL26/JUMP26 out of range ({disp})").into());
    }
    a64_write(
        buf,
        off,
        (instr & 0xFC00_0000) | ((imm as u32) & 0x03FF_FFFF),
    )
}

fn a64_patch_adr_pg_hi21(
    buf: &mut [u8],
    off: usize,
    s: u64,
    a: i64,
    p: u64,
) -> Result<(), LinkError> {
    let dest = s.wrapping_add_signed(a);
    let delta = page(dest) as i64 - page(p) as i64;
    let imm = delta >> 12;
    if imm < -(1 << 20) || imm >= (1 << 20) {
        return Err(format!("aarch64 ADR_PREL_PG_HI21 out of range ({delta})").into());
    }
    let immlo = (imm as u32) & 3;
    let immhi = ((imm as u32) >> 2) & 0x7_ffff;
    let instr = a64_read(buf, off)?;
    a64_write(
        buf,
        off,
        (instr & 0x9F00_001F) | (immlo << 29) | (immhi << 5),
    )
}

fn a64_patch_add_lo12(buf: &mut [u8], off: usize, s: u64, a: i64) -> Result<(), LinkError> {
    let dest = s.wrapping_add_signed(a);
    let imm12 = (dest & 0xfff) as u32;
    let instr = a64_read(buf, off)?;
    a64_write(buf, off, (instr & 0xFFC0_03FF) | (imm12 << 10))
}

fn a64_patch_ldst_lo12(
    buf: &mut [u8],
    off: usize,
    s: u64,
    a: i64,
    shift: u32,
) -> Result<(), LinkError> {
    let dest = s.wrapping_add_signed(a);
    let imm12 = ((dest >> shift) & 0xfff) as u32;
    let instr = a64_read(buf, off)?;
    a64_write(buf, off, (instr & 0xFFC0_03FF) | (imm12 << 10))
}

fn a64_patch_condbr19(buf: &mut [u8], off: usize, s: u64, a: i64, p: u64) -> Result<(), LinkError> {
    let dest = s.wrapping_add_signed(a);
    let disp = dest as i64 - p as i64;
    if disp & 3 != 0 {
        return Err("aarch64 CONDBR19 not aligned".into());
    }
    let imm = disp >> 2;
    if imm < -(1 << 18) || imm >= (1 << 18) {
        return Err(format!("aarch64 CONDBR19 out of range ({disp})").into());
    }
    let instr = a64_read(buf, off)?;
    a64_write(
        buf,
        off,
        (instr & 0xFF00_001F) | (((imm as u32) & 0x7_ffff) << 5),
    )
}

fn a64_patch_tstbr14(buf: &mut [u8], off: usize, s: u64, a: i64, p: u64) -> Result<(), LinkError> {
    let dest = s.wrapping_add_signed(a);
    let disp = dest as i64 - p as i64;
    if disp & 3 != 0 {
        return Err("aarch64 TSTBR14 not aligned".into());
    }
    let imm = disp >> 2;
    if imm < -(1 << 13) || imm >= (1 << 13) {
        return Err(format!("aarch64 TSTBR14 out of range ({disp})").into());
    }
    let instr = a64_read(buf, off)?;
    a64_write(
        buf,
        off,
        (instr & 0xFFF8_001F) | (((imm as u32) & 0x3fff) << 5),
    )
}

fn a64_patch_movw(
    buf: &mut [u8],
    off: usize,
    s: u64,
    a: i64,
    shift: u32,
    check: bool,
) -> Result<(), LinkError> {
    let dest = s.wrapping_add_signed(a);
    let imm16 = ((dest >> shift) & 0xffff) as u32;
    if check {
        let max = 1u64 << (shift + 16);
        if dest >= max {
            return Err(format!("aarch64 MOVW immediate overflow (G{})", shift / 16).into());
        }
    }
    let instr = a64_read(buf, off)?;
    a64_write(buf, off, (instr & 0xFFE0_001F) | (imm16 << 5))
}

fn a64_patch_adr_lo21(buf: &mut [u8], off: usize, s: u64, a: i64, p: u64) -> Result<(), LinkError> {
    let dest = s.wrapping_add_signed(a);
    let imm = dest as i64 - p as i64;
    if imm < -(1 << 20) || imm >= (1 << 20) {
        return Err(format!("aarch64 ADR_PREL_LO21 out of range ({imm})").into());
    }
    let immlo = (imm as u32) & 3;
    let immhi = ((imm as u32) >> 2) & 0x7_ffff;
    let instr = a64_read(buf, off)?;
    a64_write(
        buf,
        off,
        (instr & 0x9F00_001F) | (immlo << 29) | (immhi << 5),
    )
}

