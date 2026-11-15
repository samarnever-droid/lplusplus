use std::collections::HashMap;
use std::fmt::Write as _;

use lpp_codegen_api::{DebugSourceResolver, OptLevel, Target};

/// Collects and serializes LLVM DWARF/CodeView metadata nodes (`!DIFile`,
/// `!DICompileUnit`, `!DISubprogram`, `!DILocation`) when `-g` / `--debug`
/// compilation is requested.
pub(crate) struct DebugMetadata<'a> {
    enabled: bool,
    resolver: Option<&'a dyn DebugSourceResolver>,
    next_id: u32,
    cu_id: u32,
    empty_tuple_id: u32,
    subroutine_ty_id: u32,
    files: HashMap<(String, String), u32>,
    default_file_id: u32,
    locs: HashMap<(u32, u32, u32), u32>,
    defs: Vec<String>,
    is_windows_coff: bool,
    is_optimized: bool,
}

fn escape_md(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out
}

impl<'a> DebugMetadata<'a> {
    pub fn new(
        enabled: bool,
        resolver: Option<&'a dyn DebugSourceResolver>,
        target: Target,
        opt_level: OptLevel,
    ) -> Self {
        let mut this = Self {
            enabled,
            resolver,
            next_id: 0,
            cu_id: 0,
            empty_tuple_id: 0,
            subroutine_ty_id: 0,
            files: HashMap::new(),
            default_file_id: 0,
            locs: HashMap::new(),
            defs: Vec::new(),
            is_windows_coff: cfg!(target_os = "windows") && !target.is_freestanding(),
            is_optimized: !opt_level.is_none(),
        };
        if enabled {
            let (entry_dir, entry_file) = resolver
                .and_then(|r| r.entry_file())
                .unwrap_or((".", "main.lpp"));
            let cu_id = this.alloc_id();
            let dwarf_or_cv_flag = this.alloc_id();
            let dbg_info_ver_flag = this.alloc_id();
            let empty_tuple_id = this.alloc_id();
            let void_ty_list = this.alloc_id();
            let subroutine_ty_id = this.alloc_id();

            this.cu_id = cu_id;
            this.empty_tuple_id = empty_tuple_id;
            this.subroutine_ty_id = subroutine_ty_id;

            let file_id = this.intern_file(entry_dir, entry_file);
            this.default_file_id = file_id;

            let _ = dwarf_or_cv_flag;
            let _ = dbg_info_ver_flag;
            this.defs.push(format!(
                "!{cu_id} = distinct !DICompileUnit(language: DW_LANG_C99, file: !{file_id}, producer: \"lpp {}\", isOptimized: {}, runtimeVersion: 0, emissionKind: FullDebug, enums: !{empty_tuple_id})",
                env!("CARGO_PKG_VERSION"),
                if this.is_optimized { "true" } else { "false" }
            ));
            if this.is_windows_coff {
                this.defs.push(format!(
                    "!{dwarf_or_cv_flag} = !{{i32 2, !\"CodeView\", i32 1}}"
                ));
            } else {
                this.defs.push(format!(
                    "!{dwarf_or_cv_flag} = !{{i32 7, !\"Dwarf Version\", i32 4}}"
                ));
            }
            this.defs.push(format!(
                "!{dbg_info_ver_flag} = !{{i32 2, !\"Debug Info Version\", i32 3}}"
            ));
            this.defs.push(format!("!{empty_tuple_id} = !{{}}"));
            this.defs.push(format!("!{void_ty_list} = !{{null}}"));
            this.defs.push(format!(
                "!{subroutine_ty_id} = !DISubroutineType(types: !{void_ty_list})"
            ));
        }
        this
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn alloc_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn intern_file(&mut self, dir: &str, filename: &str) -> u32 {
        let key = (dir.to_string(), filename.to_string());
        if let Some(&id) = self.files.get(&key) {
            return id;
        }
        let id = self.alloc_id();
        self.defs.push(format!(
            "!{id} = !DIFile(filename: \"{}\", directory: \"{}\")",
            escape_md(filename),
            escape_md(dir)
        ));
        self.files.insert(key, id);
        id
    }

    pub fn register_subprogram(
        &mut self,
        source_name: &str,
        linkage_name: &str,
        origin_raw: u32,
    ) -> Option<(u32, u32)> {
        if !self.enabled {
            return None;
        }
        let (file_id, line, col) = if let Some((dir, file, l, c)) =
            self.resolver.and_then(|r| r.resolve_origin(origin_raw))
        {
            (self.intern_file(dir, file), l.max(1), c.max(1))
        } else {
            (self.default_file_id, 1, 1)
        };
        let sp_id = self.alloc_id();
        let sp_flags = if self.is_optimized {
            "DISPFlagDefinition | DISPFlagOptimized"
        } else {
            "DISPFlagDefinition"
        };
        self.defs.push(format!(
            "!{sp_id} = distinct !DISubprogram(name: \"{}\", linkageName: \"{}\", scope: !{file_id}, file: !{file_id}, line: {line}, type: !{}, scopeLine: {line}, flags: DIFlagPrototyped, spFlags: {sp_flags}, unit: !{}, retainedNodes: !{})",
            escape_md(source_name),
            escape_md(linkage_name),
            self.subroutine_ty_id,
            self.cu_id,
            self.empty_tuple_id,
        ));
        let default_loc = self.location_for_coords(sp_id, line, col);
        Some((sp_id, default_loc))
    }

    pub fn location_for_origin(&mut self, sp_id: u32, origin_raw: u32) -> Option<u32> {
        if !self.enabled {
            return None;
        }
        let (line, col) = self
            .resolver
            .and_then(|r| r.resolve_origin(origin_raw))
            .map(|(_, _, l, c)| (l.max(1), c.max(1)))
            .unwrap_or((1, 1));
        Some(self.location_for_coords(sp_id, line, col))
    }

    fn location_for_coords(&mut self, sp_id: u32, line: u32, col: u32) -> u32 {
        let key = (sp_id, line.max(1), col.max(1));
        if let Some(&id) = self.locs.get(&key) {
            return id;
        }
        let id = self.alloc_id();
        self.defs.push(format!(
            "!{id} = !DILocation(line: {}, column: {}, scope: !{sp_id})",
            key.1, key.2
        ));
        self.locs.insert(key, id);
        id
    }

    pub fn emit_module_trailer(&self, out: &mut String) {
        if !self.enabled {
            return;
        }
        let _ = writeln!(out);
        let _ = writeln!(out, "!llvm.dbg.cu = !{{!{}}}", self.cu_id);
        let _ = writeln!(out, "!llvm.module.flags = !{{!1, !2}}");
        for def in &self.defs {
            let _ = writeln!(out, "{def}");
        }
    }
}
