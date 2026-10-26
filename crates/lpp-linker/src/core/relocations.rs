// ── Relocations / symbols ──────────────────────────────────────────────────

#[derive(Clone)]
enum RelTarget {
    /// Named (possibly external) symbol.
    Name(String),
    /// Offset within this object's classified section.
    Local(SectionClass, u64),
}

fn got_key(obj_idx: usize, target: &RelTarget) -> String {
    match target {
        RelTarget::Name(n) => n.clone(),
        RelTarget::Local(c, o) => format!("__local.{obj_idx}.{}.{}", c.tag(), o),
    }
}

struct Relocation {
    offset: usize,
    target: RelTarget,
    addend: i64,
    size: u8,
    kind: RelocationKind,
    section_class: SectionClass,
    raw_type: u32,
}

#[derive(Clone)]
#[allow(dead_code)]
struct Defined {
    name: String,
    bind: Bind,
    class: SectionClass,
    /// Offset from the start of this object's contribution to `class`.
    offset: u64,
    size: u64,
    align: u64,
    object: usize,
}

#[derive(Clone)]
struct CommonSym {
    name: String,
    size: u64,
    align: u64,
}

