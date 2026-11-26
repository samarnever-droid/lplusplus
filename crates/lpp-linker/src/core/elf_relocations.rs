use super::*;

pub(super) struct ElfRelocationContext<'a> {
    pub(super) objects: &'a [ObjectImage],
    pub(super) merged: &'a Merged,
    pub(super) machine: Machine,
    pub(super) rx: &'a mut [u8],
    pub(super) rw: &'a mut [u8],
    pub(super) rodata_off_in_text: usize,
    pub(super) init_off_in_text: usize,
    pub(super) fini_off_in_text: usize,
    pub(super) data_off_in_data: usize,
    pub(super) tls_off_in_data: usize,
    pub(super) va_text: u64,
    pub(super) va_rodata: u64,
    pub(super) va_init: u64,
    pub(super) va_fini: u64,
    pub(super) va_data: u64,
    pub(super) va_tls: u64,
    pub(super) va_got: u64,
    pub(super) got_keys: &'a BTreeMap<String, usize>,
    pub(super) tls_memsz: u64,
}

pub(super) fn apply_elf_relocations<F, G>(
    ctx: ElfRelocationContext<'_>,
    lookup_va: F,
    lookup_size: G,
) -> Result<(), LinkError>
where
    F: Fn(&RelTarget, usize) -> Result<u64, LinkError>,
    G: Fn(&str) -> u64,
{
    let ElfRelocationContext {
        objects,
        merged,
        machine,
        rx,
        rw,
        rodata_off_in_text,
        init_off_in_text,
        fini_off_in_text,
        data_off_in_data,
        tls_off_in_data,
        va_text,
        va_rodata,
        va_init,
        va_fini,
        va_data,
        va_tls,
        va_got,
        got_keys,
        tls_memsz,
    } = ctx;
    for (oi, obj) in objects.iter().enumerate() {
        for rel in &obj.relocations {
            let (buf, buf_va, local_base) = match rel.section_class {
                SectionClass::Text => (&mut rx[..], va_text, merged.place[oi].text),
                SectionClass::Rodata => (
                    &mut rx[rodata_off_in_text..],
                    va_rodata,
                    merged.place[oi].rodata,
                ),
                SectionClass::InitArray => (
                    &mut rx[init_off_in_text..],
                    va_init,
                    merged.place[oi].init_array,
                ),
                SectionClass::FiniArray => (
                    &mut rx[fini_off_in_text..],
                    va_fini,
                    merged.place[oi].fini_array,
                ),
                SectionClass::Data => (&mut rw[data_off_in_data..], va_data, merged.place[oi].data),
                SectionClass::Tls => (&mut rw[tls_off_in_data..], va_tls, merged.place[oi].tls),
                SectionClass::Bss => {
                    return Err(format!("'{}': relocation against BSS", obj.path.display()).into());
                }
            };
            let patch = local_base + rel.offset;
            let p = buf_va + patch as u64;
            let ctx = obj.path.display().to_string();

            let name_opt = match &rel.target {
                RelTarget::Name(n) => Some(n.as_str()),
                _ => None,
            };
            let s = lookup_va(&rel.target, oi).map_err(|e| format!("'{ctx}': {e}"))?;
            let a = rel.addend;

            let got_va_of = |n: &str| -> Result<u64, LinkError> {
                got_keys
                    .get(n)
                    .map(|i| va_got + (*i as u64) * 8)
                    .ok_or_else(|| {
                        LinkError::new(
                            LinkErrorKind::Internal,
                            format!("'{ctx}': no GOT slot for {n}"),
                        )
                    })
            };
            let got_va_target =
                |t: &RelTarget| -> Result<u64, LinkError> { got_va_of(&got_key(oi, t)) };

            match (machine, rel.raw_type) {
                (Machine::X86_64, R_X86_64_64) => {
                    write_u64_at(buf, patch, s.wrapping_add_signed(a), &ctx)?;
                }
                (Machine::X86_64, R_X86_64_PC64) => {
                    let v = s.wrapping_add_signed(a).wrapping_sub(p);
                    write_u64_at(buf, patch, v, &ctx)?;
                }
                (Machine::X86_64, R_X86_64_PC32) | (Machine::X86_64, R_X86_64_PLT32) => {
                    write_i32_at(buf, patch, s as i64 + a - p as i64, &ctx)?;
                }
                (Machine::X86_64, R_X86_64_32) => {
                    write_u32_at(buf, patch, s as i64 + a, &ctx)?;
                }
                (Machine::X86_64, R_X86_64_32S) => {
                    write_i32_at(buf, patch, s as i64 + a, &ctx)?;
                }
                (Machine::X86_64, R_X86_64_16) => {
                    write_i16_at(buf, patch, s as i64 + a, &ctx)?;
                }
                (Machine::X86_64, R_X86_64_PC16) => {
                    write_i16_at(buf, patch, s as i64 + a - p as i64, &ctx)?;
                }
                (Machine::X86_64, R_X86_64_8) => {
                    write_i8_at(buf, patch, s as i64 + a, &ctx)?;
                }
                (Machine::X86_64, R_X86_64_PC8) => {
                    write_i8_at(buf, patch, s as i64 + a - p as i64, &ctx)?;
                }
                (Machine::X86_64, R_X86_64_GOTPCREL)
                | (Machine::X86_64, R_X86_64_GOTPCRELX)
                | (Machine::X86_64, R_X86_64_REX_GOTPCRELX)
                | (Machine::X86_64, R_X86_64_GOTTPOFF) => {
                    let g = got_va_target(&rel.target)?;
                    write_i32_at(buf, patch, g as i64 + a - p as i64, &ctx)?;
                }
                (Machine::X86_64, R_X86_64_GOT32) => {
                    let g = got_va_target(&rel.target)?;
                    write_i32_at(buf, patch, (g - va_got) as i64 + a, &ctx)?;
                }
                (Machine::X86_64, R_X86_64_GOTPC32) => {
                    write_i32_at(buf, patch, va_got as i64 + a - p as i64, &ctx)?;
                }
                (Machine::X86_64, R_X86_64_GOTOFF64) => {
                    write_u64_at(
                        buf,
                        patch,
                        s.wrapping_add_signed(a).wrapping_sub(va_got),
                        &ctx,
                    )?;
                }
                (Machine::X86_64, R_X86_64_TPOFF32) => {
                    let off = match &rel.target {
                        RelTarget::Name(n) => merged
                            .syms
                            .get(n)
                            .filter(|s| s.class == SectionClass::Tls)
                            .map(|s| s.offset)
                            .ok_or_else(|| format!("'{ctx}': TPOFF of non-TLS '{n}'"))?,
                        RelTarget::Local(SectionClass::Tls, o) => merged.place[oi].tls as u64 + *o,
                        _ => return Err(format!("'{ctx}': TPOFF of non-TLS").into()),
                    };
                    write_i32_at(buf, patch, x64_tpoff(off, tls_memsz) + a, &ctx)?;
                }
                (Machine::X86_64, R_X86_64_TPOFF64) | (Machine::X86_64, R_X86_64_DTPOFF32) => {
                    let off = match &rel.target {
                        RelTarget::Name(n) => merged.syms.get(n).map(|s| s.offset).unwrap_or(0),
                        RelTarget::Local(_, o) => *o,
                    };
                    if rel.raw_type == R_X86_64_TPOFF64 {
                        write_u64_at(buf, patch, (x64_tpoff(off, tls_memsz) + a) as u64, &ctx)?;
                    } else {
                        write_i32_at(buf, patch, off as i64 + a, &ctx)?;
                    }
                }
                (Machine::X86_64, R_X86_64_SIZE32) => {
                    let n = name_opt.unwrap_or("");
                    write_u32_at(buf, patch, lookup_size(n) as i64 + a, &ctx)?;
                }
                (Machine::X86_64, R_X86_64_SIZE64) => {
                    let n = name_opt.unwrap_or("");
                    write_u64_at(buf, patch, (lookup_size(n) as i64 + a) as u64, &ctx)?;
                }
                (Machine::Aarch64, R_AARCH64_ABS64) => {
                    write_u64_at(buf, patch, s.wrapping_add_signed(a), &ctx)?;
                }
                (Machine::Aarch64, R_AARCH64_ABS32) => {
                    write_u32_at(buf, patch, s as i64 + a, &ctx)?;
                }
                (Machine::Aarch64, R_AARCH64_ABS16) => {
                    write_i16_at(buf, patch, s as i64 + a, &ctx)?;
                }
                (Machine::Aarch64, R_AARCH64_PREL64) => {
                    write_u64_at(buf, patch, s.wrapping_add_signed(a).wrapping_sub(p), &ctx)?;
                }
                (Machine::Aarch64, R_AARCH64_PREL32) => {
                    write_i32_at(buf, patch, s as i64 + a - p as i64, &ctx)?;
                }
                (Machine::Aarch64, R_AARCH64_PREL16) => {
                    write_i16_at(buf, patch, s as i64 + a - p as i64, &ctx)?;
                }
                (Machine::Aarch64, R_AARCH64_CALL26) | (Machine::Aarch64, R_AARCH64_JUMP26) => {
                    a64_patch_call26(buf, patch, s, a, p)?;
                }
                (Machine::Aarch64, R_AARCH64_ADR_PREL_PG_HI21)
                | (Machine::Aarch64, R_AARCH64_ADR_PREL_PG_HI21_NC) => {
                    a64_patch_adr_pg_hi21(buf, patch, s, a, p)?;
                }
                (Machine::Aarch64, R_AARCH64_ADR_PREL_LO21) => {
                    a64_patch_adr_lo21(buf, patch, s, a, p)?;
                }
                (Machine::Aarch64, R_AARCH64_ADD_ABS_LO12_NC) => {
                    a64_patch_add_lo12(buf, patch, s, a)?;
                }
                (Machine::Aarch64, R_AARCH64_LDST8_ABS_LO12_NC) => {
                    a64_patch_ldst_lo12(buf, patch, s, a, 0)?;
                }
                (Machine::Aarch64, R_AARCH64_LDST16_ABS_LO12_NC) => {
                    a64_patch_ldst_lo12(buf, patch, s, a, 1)?;
                }
                (Machine::Aarch64, R_AARCH64_LDST32_ABS_LO12_NC) => {
                    a64_patch_ldst_lo12(buf, patch, s, a, 2)?;
                }
                (Machine::Aarch64, R_AARCH64_LDST64_ABS_LO12_NC) => {
                    a64_patch_ldst_lo12(buf, patch, s, a, 3)?;
                }
                (Machine::Aarch64, R_AARCH64_LDST128_ABS_LO12_NC) => {
                    a64_patch_ldst_lo12(buf, patch, s, a, 4)?;
                }
                (Machine::Aarch64, R_AARCH64_CONDBR19) => {
                    a64_patch_condbr19(buf, patch, s, a, p)?;
                }
                (Machine::Aarch64, R_AARCH64_TSTBR14) => {
                    a64_patch_tstbr14(buf, patch, s, a, p)?;
                }
                (Machine::Aarch64, R_AARCH64_MOVW_UABS_G0) => {
                    a64_patch_movw(buf, patch, s, a, 0, true)?;
                }
                (Machine::Aarch64, R_AARCH64_MOVW_UABS_G0_NC) => {
                    a64_patch_movw(buf, patch, s, a, 0, false)?;
                }
                (Machine::Aarch64, R_AARCH64_MOVW_UABS_G1) => {
                    a64_patch_movw(buf, patch, s, a, 16, true)?;
                }
                (Machine::Aarch64, R_AARCH64_MOVW_UABS_G1_NC) => {
                    a64_patch_movw(buf, patch, s, a, 16, false)?;
                }
                (Machine::Aarch64, R_AARCH64_MOVW_UABS_G2) => {
                    a64_patch_movw(buf, patch, s, a, 32, true)?;
                }
                (Machine::Aarch64, R_AARCH64_MOVW_UABS_G2_NC) => {
                    a64_patch_movw(buf, patch, s, a, 32, false)?;
                }
                (Machine::Aarch64, R_AARCH64_MOVW_UABS_G3) => {
                    a64_patch_movw(buf, patch, s, a, 48, false)?;
                }
                (Machine::Aarch64, R_AARCH64_ADR_GOT_PAGE) => {
                    let g = got_va_target(&rel.target)?;
                    a64_patch_adr_pg_hi21(buf, patch, g, 0, p)?;
                }
                (Machine::Aarch64, R_AARCH64_LD64_GOT_LO12_NC) => {
                    let g = got_va_target(&rel.target)?;
                    a64_patch_ldst_lo12(buf, patch, g, 0, 3)?;
                }
                (_, 0) => {
                    // Fall back to object-crate RelocationKind.
                    match rel.kind {
                        RelocationKind::Absolute if rel.size == 64 => {
                            write_u64_at(buf, patch, s.wrapping_add_signed(a), &ctx)?;
                        }
                        RelocationKind::Absolute => {
                            write_i32_at(buf, patch, s as i64 + a, &ctx)?;
                        }
                        RelocationKind::Relative | RelocationKind::PltRelative => {
                            write_i32_at(buf, patch, s as i64 + a - p as i64, &ctx)?;
                        }
                        RelocationKind::GotRelative => {
                            let g = got_va_target(&rel.target)?;
                            write_i32_at(buf, patch, g as i64 + a - p as i64, &ctx)?;
                        }
                        _ => {
                            return Err(format!(
                                "'{ctx}': unsupported relocation kind {:?} type {}",
                                rel.kind, rel.raw_type
                            )
                            .into());
                        }
                    }
                }
                _ => {
                    return Err(format!(
                        "'{ctx}': unsupported relocation type {} on {:?}",
                        rel.raw_type, machine
                    )
                    .into());
                }
            }
        }
    }

    Ok(())
}
