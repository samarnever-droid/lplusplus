use std::collections::HashMap;

use cranelift_codegen::ir::SourceLoc;
use cranelift_object::object::write::{Object, Relocation, SectionId, SymbolId};
use cranelift_object::object::{
    BinaryFormat, RelocationEncoding, RelocationFlags, RelocationKind, SectionKind,
};
use lpp_codegen_api::DebugSourceResolver;

pub(crate) struct CompiledDebugFunction {
    pub export_name: String,
    pub source_name: String,
    pub origin_raw: u32,
    pub code_len: u32,
    pub srclocs: Vec<(u32, SourceLoc)>,
}

fn push_uleb128(buf: &mut Vec<u8>, mut v: u64) {
    loop {
        let mut byte = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            byte |= 0x80;
        }
        buf.push(byte);
        if v == 0 {
            break;
        }
    }
}

fn push_sleb128(buf: &mut Vec<u8>, mut v: i64) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        let done = (v == 0 && (byte & 0x40) == 0) || (v == -1 && (byte & 0x40) != 0);
        if done {
            buf.push(byte);
            break;
        }
        buf.push(byte | 0x80);
    }
}

struct StrTable {
    bytes: Vec<u8>,
    offsets: HashMap<String, u32>,
}

impl StrTable {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            offsets: HashMap::new(),
        }
    }

    fn intern(&mut self, s: &str) -> u32 {
        if let Some(&off) = self.offsets.get(s) {
            return off;
        }
        let off = self.bytes.len() as u32;
        self.bytes.extend_from_slice(s.as_bytes());
        self.bytes.push(0);
        self.offsets.insert(s.to_string(), off);
        off
    }
}

fn add_debug_section(object: &mut Object<'_>, elf_name: &str, macho_name: &str) -> SectionId {
    let (seg, name) = if object.format() == BinaryFormat::MachO {
        (b"__DWARF".to_vec(), macho_name.as_bytes().to_vec())
    } else {
        (Vec::new(), elf_name.as_bytes().to_vec())
    };
    object.add_section(seg, name, SectionKind::Debug)
}

fn abs_addr_reloc(offset: u64, symbol: SymbolId) -> Relocation {
    Relocation {
        offset,
        symbol,
        addend: 0,
        flags: RelocationFlags::Generic {
            kind: RelocationKind::Absolute,
            encoding: RelocationEncoding::Generic,
            size: 64,
        },
    }
}

/// Emit DWARF v4 `.debug_abbrev`, `.debug_str`, `.debug_info`, and `.debug_line`
/// sections directly into `object` for all compiled Cranelift functions.
pub(crate) fn emit_dwarf_sections(
    object: &mut Object<'_>,
    functions: &[CompiledDebugFunction],
    resolver: Option<&dyn DebugSourceResolver>,
) {
    if functions.is_empty() {
        return;
    }

    let mut strs = StrTable::new();
    let producer_off = strs.intern("lpp 0.3.0");
    let (comp_dir, entry_filename) = resolver
        .and_then(|r| r.entry_file())
        .unwrap_or((".", "main.lpp"));
    let entry_dir_off = strs.intern(comp_dir);
    let entry_file_off = strs.intern(entry_filename);

    // Intern directories and `(dir, file)` pairs -> 1-based DWARF file numbers.
    let mut dirs: Vec<String> = vec![comp_dir.to_string()];
    let mut dir_map: HashMap<String, u64> = HashMap::new();
    dir_map.insert(comp_dir.to_string(), 1);

    let mut files: Vec<(String, u64)> = vec![(entry_filename.to_string(), 1)];
    let mut file_map: HashMap<(String, String), u32> = HashMap::new();
    file_map.insert((comp_dir.to_string(), entry_filename.to_string()), 1);

    let mut intern_file = |dir: &str, name: &str| -> u32 {
        let key = (dir.to_string(), name.to_string());
        if let Some(&id) = file_map.get(&key) {
            return id;
        }
        let dir_idx = if let Some(&di) = dir_map.get(dir) {
            di
        } else {
            dirs.push(dir.to_string());
            let di = dirs.len() as u64;
            dir_map.insert(dir.to_string(), di);
            di
        };
        files.push((name.to_string(), dir_idx));
        let fi = files.len() as u32;
        file_map.insert(key, fi);
        fi
    };

    // Resolve symbol IDs and pre-intern all referenced files before serializing tables.
    struct ResolvedFn<'a> {
        func: &'a CompiledDebugFunction,
        sym: SymbolId,
        file_id: u32,
        decl_line: u32,
        name_off: u32,
        linkage_off: u32,
    }

    let mut resolved = Vec::new();
    for func in functions {
        let Some(sym) = object.symbol_id(func.export_name.as_bytes()) else {
            continue;
        };
        let (file_id, decl_line) = resolver
            .and_then(|r| r.resolve_origin(func.origin_raw))
            .map(|(dir, file, line, _)| (intern_file(dir, file), line.max(1)))
            .unwrap_or((1, 1));
        for &(_, loc) in &func.srclocs {
            if !loc.is_default()
                && let Some((dir, file, _, _)) = resolver.and_then(|r| r.resolve_origin(loc.bits()))
            {
                let _ = intern_file(dir, file);
            }
        }
        resolved.push(ResolvedFn {
            func,
            sym,
            file_id,
            decl_line,
            name_off: strs.intern(&func.source_name),
            linkage_off: strs.intern(&func.export_name),
        });
    }
    if resolved.is_empty() {
        return;
    }

    // 1. `.debug_abbrev`
    let mut abbrev = Vec::new();
    // Abbrev 1: DW_TAG_compile_unit (0x11), DW_CHILDREN_yes (1)
    push_uleb128(&mut abbrev, 1);
    push_uleb128(&mut abbrev, 0x11);
    abbrev.push(1);
    for (attr, form) in [
        (0x25u64, 0x0eu64), // DW_AT_producer : DW_FORM_strp
        (0x13, 0x05),       // DW_AT_language : DW_FORM_data2
        (0x03, 0x0e),       // DW_AT_name     : DW_FORM_strp
        (0x1b, 0x0e),       // DW_AT_comp_dir : DW_FORM_strp
        (0x10, 0x17),       // DW_AT_stmt_list: DW_FORM_sec_offset
    ] {
        push_uleb128(&mut abbrev, attr);
        push_uleb128(&mut abbrev, form);
    }
    abbrev.extend_from_slice(&[0, 0]);

    // Abbrev 2: DW_TAG_subprogram (0x2e), DW_CHILDREN_no (0)
    push_uleb128(&mut abbrev, 2);
    push_uleb128(&mut abbrev, 0x2e);
    abbrev.push(0);
    for (attr, form) in [
        (0x03u64, 0x0eu64), // DW_AT_name         : DW_FORM_strp
        (0x6e, 0x0e),       // DW_AT_linkage_name : DW_FORM_strp
        (0x3a, 0x06),       // DW_AT_decl_file    : DW_FORM_data4
        (0x3b, 0x06),       // DW_AT_decl_line    : DW_FORM_data4
        (0x11, 0x01),       // DW_AT_low_pc       : DW_FORM_addr
        (0x12, 0x06),       // DW_AT_high_pc      : DW_FORM_data4 (offset from low_pc)
    ] {
        push_uleb128(&mut abbrev, attr);
        push_uleb128(&mut abbrev, form);
    }
    abbrev.extend_from_slice(&[0, 0, 0]);

    // 2. `.debug_info`
    let mut info = Vec::new();
    let mut info_relocs: Vec<Relocation> = Vec::new();
    info.extend_from_slice(&0u32.to_le_bytes()); // unit_length placeholder
    info.extend_from_slice(&4u16.to_le_bytes()); // DWARF version 4
    info.extend_from_slice(&0u32.to_le_bytes()); // debug_abbrev_offset = 0
    info.push(8); // address_size = 8

    // DIE: Compile unit (abbrev 1)
    push_uleb128(&mut info, 1);
    info.extend_from_slice(&producer_off.to_le_bytes());
    info.extend_from_slice(&0x000cu16.to_le_bytes()); // DW_LANG_C99
    info.extend_from_slice(&entry_file_off.to_le_bytes());
    info.extend_from_slice(&entry_dir_off.to_le_bytes());
    info.extend_from_slice(&0u32.to_le_bytes()); // stmt_list = .debug_line + 0

    for rf in &resolved {
        push_uleb128(&mut info, 2);
        info.extend_from_slice(&rf.name_off.to_le_bytes());
        info.extend_from_slice(&rf.linkage_off.to_le_bytes());
        info.extend_from_slice(&rf.file_id.to_le_bytes());
        info.extend_from_slice(&rf.decl_line.to_le_bytes());
        let addr_off = info.len() as u64;
        info.extend_from_slice(&0u64.to_le_bytes());
        info_relocs.push(abs_addr_reloc(addr_off, rf.sym));
        info.extend_from_slice(&rf.func.code_len.max(1).to_le_bytes());
    }
    info.push(0); // terminate CU children
    let info_len = (info.len() - 4) as u32;
    info[0..4].copy_from_slice(&info_len.to_le_bytes());

    // 3. `.debug_line` (DWARF 4 Line Number Program)
    let mut line_sec = Vec::new();
    let mut line_relocs: Vec<Relocation> = Vec::new();
    line_sec.extend_from_slice(&0u32.to_le_bytes()); // unit_length placeholder
    line_sec.extend_from_slice(&4u16.to_le_bytes()); // version 4
    let hdr_len_pos = line_sec.len();
    line_sec.extend_from_slice(&0u32.to_le_bytes()); // header_length placeholder
    let hdr_start = line_sec.len();
    line_sec.push(1); // minimum_instruction_length
    line_sec.push(1); // maximum_operations_per_instruction
    line_sec.push(1); // default_is_stmt
    line_sec.push((-5i8) as u8); // line_base
    line_sec.push(14); // line_range
    line_sec.push(13); // opcode_base
    line_sec.extend_from_slice(&[0, 1, 1, 1, 1, 0, 0, 0, 1, 0, 0, 1]);

    // include_directories (NUL-terminated strings, terminated by empty string)
    for d in &dirs {
        line_sec.extend_from_slice(d.as_bytes());
        line_sec.push(0);
    }
    line_sec.push(0);

    // file_names
    for (fname, dir_idx) in &files {
        line_sec.extend_from_slice(fname.as_bytes());
        line_sec.push(0);
        push_uleb128(&mut line_sec, *dir_idx);
        push_uleb128(&mut line_sec, 0);
        push_uleb128(&mut line_sec, 0);
    }
    line_sec.push(0);

    let hdr_len = (line_sec.len() - hdr_start) as u32;
    line_sec[hdr_len_pos..hdr_len_pos + 4].copy_from_slice(&hdr_len.to_le_bytes());

    for rf in &resolved {
        // DW_LNS_extended_op(0), len=9, DW_LNE_set_address(2), 8-byte addr
        line_sec.extend_from_slice(&[0, 9, 2]);
        let rel_off = line_sec.len() as u64;
        line_sec.extend_from_slice(&0u64.to_le_bytes());
        line_relocs.push(abs_addr_reloc(rel_off, rf.sym));

        let mut cur_addr = 0u32;
        let mut cur_line = 1i64;
        let mut cur_file = 1u32;

        // Emit entry decl location at offset 0 first, then instruction srclocs.
        let mut pts: Vec<(u32, u32, u32, u32)> = vec![(0, rf.file_id, rf.decl_line, 1)];
        for &(off, loc) in &rf.func.srclocs {
            if !loc.is_default()
                && let Some((dir, file, l, c)) = resolver.and_then(|r| r.resolve_origin(loc.bits()))
            {
                let fid = file_map
                    .get(&(dir.to_string(), file.to_string()))
                    .copied()
                    .unwrap_or(1);
                pts.push((off, fid, l.max(1), c.max(1)));
            }
        }
        for (off, fid, l, c) in pts {
            if off < cur_addr {
                continue;
            }
            if fid != cur_file {
                line_sec.push(4); // DW_LNS_set_file
                push_uleb128(&mut line_sec, u64::from(fid));
                cur_file = fid;
            }
            if c > 0 {
                line_sec.push(5); // DW_LNS_set_column
                push_uleb128(&mut line_sec, u64::from(c));
            }
            if off > cur_addr {
                line_sec.push(2); // DW_LNS_advance_pc
                push_uleb128(&mut line_sec, u64::from(off - cur_addr));
                cur_addr = off;
            }
            let dline = i64::from(l) - cur_line;
            if dline != 0 {
                line_sec.push(3); // DW_LNS_advance_line
                push_sleb128(&mut line_sec, dline);
                cur_line = i64::from(l);
            }
            line_sec.push(1); // DW_LNS_copy
        }
        let end_off = rf.func.code_len.max(cur_addr + 1);
        if end_off > cur_addr {
            line_sec.push(2); // DW_LNS_advance_pc
            push_uleb128(&mut line_sec, u64::from(end_off - cur_addr));
        }
        line_sec.extend_from_slice(&[0, 1, 1]); // DW_LNE_end_sequence
    }
    let line_len = (line_sec.len() - 4) as u32;
    line_sec[0..4].copy_from_slice(&line_len.to_le_bytes());

    let abbrev_sec_id = add_debug_section(object, ".debug_abbrev", "__debug_abbrev");
    object.append_section_data(abbrev_sec_id, &abbrev, 1);

    let str_sec_id = add_debug_section(object, ".debug_str", "__debug_str");
    object.append_section_data(str_sec_id, &strs.bytes, 1);

    let info_sec_id = add_debug_section(object, ".debug_info", "__debug_info");
    object.append_section_data(info_sec_id, &info, 1);
    for reloc in info_relocs {
        let _ = object.add_relocation(info_sec_id, reloc);
    }

    let line_sec_id = add_debug_section(object, ".debug_line", "__debug_line");
    object.append_section_data(line_sec_id, &line_sec, 1);
    for reloc in line_relocs {
        let _ = object.add_relocation(line_sec_id, reloc);
    }
}
