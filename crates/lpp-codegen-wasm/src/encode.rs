//! Binary WebAssembly primitives: LEB128 encoders, section wrappers, the
//! opcode table, the value-type tag, and the `FB` body-builder (a growing
//! byte buffer with typed emit helpers).
//!
//! This is a hand-written emitter — there is no `wat`/`wasm-ld` step, so the
//! crate keeps the zero-extra-dependency promise.

/// Unsigned LEB128.
pub fn uleb(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// Signed LEB128.
pub fn sleb(out: &mut Vec<u8>, mut value: i64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        let sign_clear = byte & 0x40 == 0;
        if (value == 0 && sign_clear) || (value == -1 && !sign_clear) {
            out.push(byte);
            break;
        }
        out.push(byte | 0x80);
    }
}

/// Append a wasm name (length-prefixed UTF-8).
pub fn enc_name(out: &mut Vec<u8>, name: &str) {
    uleb(out, name.len() as u64);
    out.extend_from_slice(name.as_bytes());
}

/// Sections are length-prefixed; accumulate into a scratch buffer and splice.
pub fn enc_section(out: &mut Vec<u8>, id: u8, payload: &[u8]) {
    out.push(id);
    uleb(out, payload.len() as u64);
    out.extend_from_slice(payload);
}

/// The three wasm value types this backend produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Val {
    I32,
    I64,
    F64,
}

impl Val {
    pub fn byte(self) -> u8 {
        match self {
            Val::I32 => 0x7f,
            Val::I64 => 0x7e,
            Val::F64 => 0x7c,
        }
    }
}

#[allow(dead_code)]
pub mod op {
    pub const UNREACHABLE: u8 = 0x00;
    pub const BLOCK: u8 = 0x02;
    pub const LOOP: u8 = 0x03;
    pub const IF: u8 = 0x04;
    pub const ELSE: u8 = 0x05;
    pub const END: u8 = 0x0b;
    pub const BR: u8 = 0x0c;
    pub const BR_IF: u8 = 0x0d;
    pub const BR_TABLE: u8 = 0x0e;
    pub const RETURN: u8 = 0x0f;
    pub const CALL: u8 = 0x10;
    pub const CALL_INDIRECT: u8 = 0x11;
    pub const DROP: u8 = 0x1a;
    pub const SELECT: u8 = 0x1b;
    pub const LOCAL_GET: u8 = 0x20;
    pub const LOCAL_SET: u8 = 0x21;
    pub const LOCAL_TEE: u8 = 0x22;
    pub const GLOBAL_GET: u8 = 0x23;
    pub const GLOBAL_SET: u8 = 0x24;
    pub const I32_LOAD: u8 = 0x28;
    pub const I64_LOAD: u8 = 0x29;
    pub const F64_LOAD: u8 = 0x2b;
    pub const I32_LOAD8_U: u8 = 0x2c;
    pub const I32_LOAD16_U: u8 = 0x2e;
    pub const I32_STORE: u8 = 0x36;
    pub const I64_STORE: u8 = 0x37;
    pub const F64_STORE: u8 = 0x39;
    pub const I32_STORE8: u8 = 0x3a;
    pub const I32_STORE16: u8 = 0x3b;
    pub const MEMORY_SIZE: u8 = 0x3f;
    pub const MEMORY_GROW: u8 = 0x40;
    pub const I32_CONST: u8 = 0x41;
    pub const I64_CONST: u8 = 0x42;
    pub const F64_CONST: u8 = 0x44;
    pub const I32_EQZ: u8 = 0x45;
    pub const I32_EQ: u8 = 0x46;
    pub const I32_NE: u8 = 0x47;
    pub const I32_LT_S: u8 = 0x48;
    pub const I32_LT_U: u8 = 0x49;
    pub const I32_GT_S: u8 = 0x4a;
    pub const I32_GT_U: u8 = 0x4b;
    pub const I32_LE_S: u8 = 0x4c;
    pub const I32_LE_U: u8 = 0x4d;
    pub const I32_GE_S: u8 = 0x4e;
    pub const I32_GE_U: u8 = 0x4f;
    pub const I64_EQZ: u8 = 0x50;
    pub const I64_EQ: u8 = 0x51;
    pub const I64_NE: u8 = 0x52;
    pub const I64_LT_S: u8 = 0x53;
    pub const I64_LT_U: u8 = 0x54;
    pub const I64_GT_S: u8 = 0x55;
    pub const I64_GT_U: u8 = 0x56;
    pub const I64_LE_S: u8 = 0x57;
    pub const I64_LE_U: u8 = 0x58;
    pub const I64_GE_S: u8 = 0x59;
    pub const I64_GE_U: u8 = 0x5a;
    pub const F64_EQ: u8 = 0x61;
    pub const F64_NE: u8 = 0x62;
    pub const F64_LT: u8 = 0x63;
    pub const F64_GT: u8 = 0x64;
    pub const F64_LE: u8 = 0x65;
    pub const F64_GE: u8 = 0x66;
    pub const I32_ADD: u8 = 0x6a;
    pub const I32_SUB: u8 = 0x6b;
    pub const I32_MUL: u8 = 0x6c;
    pub const I32_DIV_S: u8 = 0x6d;
    pub const I32_DIV_U: u8 = 0x6e;
    pub const I32_REM_S: u8 = 0x6f;
    pub const I32_AND: u8 = 0x71;
    pub const I32_OR: u8 = 0x72;
    pub const I32_XOR: u8 = 0x73;
    pub const I32_SHL: u8 = 0x74;
    pub const I32_SHR_S: u8 = 0x75;
    pub const I32_SHR_U: u8 = 0x76;
    pub const I64_ADD: u8 = 0x7c;
    pub const I64_SUB: u8 = 0x7d;
    pub const I64_MUL: u8 = 0x7e;
    pub const I64_DIV_S: u8 = 0x7f;
    pub const I64_DIV_U: u8 = 0x80;
    pub const I64_REM_S: u8 = 0x81;
    pub const I64_REM_U: u8 = 0x82;
    pub const I64_AND: u8 = 0x83;
    pub const I64_OR: u8 = 0x84;
    pub const I64_XOR: u8 = 0x85;
    pub const I64_SHL: u8 = 0x86;
    pub const I64_SHR_S: u8 = 0x87;
    pub const I64_SHR_U: u8 = 0x88;
    pub const I32_REM_U: u8 = 0x70;
    pub const I32_ROTL: u8 = 0x77;
    pub const I32_ROTR: u8 = 0x78;
    pub const I64_CLZ: u8 = 0x79;
    pub const I64_CTZ: u8 = 0x7A;
    pub const I64_POPCNT: u8 = 0x7B;
    pub const I64_ROTL: u8 = 0x89;
    pub const I64_ROTR: u8 = 0x8A;
    pub const F64_ABS: u8 = 0x99;
    pub const F64_NEG: u8 = 0x9a;
    pub const F64_CEIL: u8 = 0x9b;
    pub const F64_FLOOR: u8 = 0x9c;
    pub const F64_TRUNC: u8 = 0x9d;
    pub const F64_SQRT: u8 = 0x9f;
    pub const F64_ADD: u8 = 0xa0;
    pub const F64_SUB: u8 = 0xa1;
    pub const F64_MUL: u8 = 0xa2;
    pub const F64_DIV: u8 = 0xa3;
    pub const I32_WRAP_I64: u8 = 0xa7;
    pub const I64_EXTEND_I32_S: u8 = 0xac;
    pub const I64_EXTEND_I32_U: u8 = 0xad;
    pub const I64_TRUNC_F64_S: u8 = 0xb0;
    pub const I64_TRUNC_F64_U: u8 = 0xb1;
    pub const F64_CONVERT_I32_S: u8 = 0xb7;
    pub const F64_CONVERT_I64_S: u8 = 0xb9;
    pub const I64_REINTERPRET_F64: u8 = 0xbd;
    pub const F64_REINTERPRET_I64: u8 = 0xbf;
    pub const BLOCK_VOID: u8 = 0x40;
    pub const BULK_PREFIX: u8 = 0xfc;
    pub const MEMORY_COPY_SUB: u64 = 10;
    pub const MEMORY_FILL_SUB: u64 = 11;
}

/// Instruction-emission helper for one function body. Declares extra
/// (non-parameter) locals so callers can allocate scratch slots by type.
pub struct FB {
    pub body: Vec<u8>,
    /// Declared extra locals (wasm local groups, in declaration order).
    pub extras: Vec<Val>,
    /// Number of parameters — extra local indices start after them.
    pub params: u32,
}

// `FB` mirrors the v1 emitter's full surface; slice 1 uses a subset, so the
// remaining helpers (64-bit loads/stores, f64 mem ops, globals, bulk memory,
// `call_indirect`) are retained for the struct/list/ARC slices.
#[allow(dead_code)]
impl FB {
    pub fn new(params: u32) -> Self {
        Self {
            body: Vec::with_capacity(128),
            extras: Vec::new(),
            params,
        }
    }

    /// Allocate a scratch local of the given type, returning its index.
    pub fn scratch(&mut self, val: Val) -> u32 {
        let index = self.params + self.extras.len() as u32;
        self.extras.push(val);
        index
    }

    pub fn op(&mut self, op: u8) -> &mut Self {
        self.body.push(op);
        self
    }

    pub fn i32c(&mut self, v: i64) -> &mut Self {
        self.body.push(op::I32_CONST);
        sleb(&mut self.body, v);
        self
    }

    pub fn i64c(&mut self, v: i64) -> &mut Self {
        self.body.push(op::I64_CONST);
        sleb(&mut self.body, v);
        self
    }

    pub fn f64c(&mut self, v: f64) -> &mut Self {
        self.body.push(op::F64_CONST);
        self.body.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn g(&mut self, local: u32) -> &mut Self {
        self.body.push(op::LOCAL_GET);
        uleb(&mut self.body, local as u64);
        self
    }

    pub fn s(&mut self, local: u32) -> &mut Self {
        self.body.push(op::LOCAL_SET);
        uleb(&mut self.body, local as u64);
        self
    }

    pub fn t(&mut self, local: u32) -> &mut Self {
        self.body.push(op::LOCAL_TEE);
        uleb(&mut self.body, local as u64);
        self
    }

    pub fn gget(&mut self, global: u32) -> &mut Self {
        self.body.push(op::GLOBAL_GET);
        uleb(&mut self.body, global as u64);
        self
    }

    pub fn gset(&mut self, global: u32) -> &mut Self {
        self.body.push(op::GLOBAL_SET);
        uleb(&mut self.body, global as u64);
        self
    }

    pub fn call(&mut self, index: u32) -> &mut Self {
        self.body.push(op::CALL);
        uleb(&mut self.body, index as u64);
        self
    }

    pub fn call_indirect(&mut self, type_index: u32, table_index: u8) -> &mut Self {
        self.body.push(op::CALL_INDIRECT);
        uleb(&mut self.body, type_index as u64);
        self.body.push(table_index);
        self
    }

    pub fn block(&mut self) -> &mut Self {
        self.body.push(op::BLOCK);
        self.body.push(op::BLOCK_VOID);
        self
    }

    pub fn loop_(&mut self) -> &mut Self {
        self.body.push(op::LOOP);
        self.body.push(op::BLOCK_VOID);
        self
    }

    pub fn if_(&mut self) -> &mut Self {
        self.body.push(op::IF);
        self.body.push(op::BLOCK_VOID);
        self
    }

    /// An `if` with an i64 result (block type 0x7E): both branches must
    /// each produce exactly one i64.
    pub fn if_i64(&mut self) -> &mut Self {
        self.body.push(op::IF);
        self.body.push(0x7E);
        self
    }

    pub fn else_(&mut self) -> &mut Self {
        self.body.push(op::ELSE);
        self
    }

    pub fn end(&mut self) -> &mut Self {
        self.body.push(op::END);
        self
    }

    pub fn br(&mut self, depth: u32) -> &mut Self {
        self.body.push(op::BR);
        uleb(&mut self.body, depth as u64);
        self
    }

    pub fn br_if(&mut self, depth: u32) -> &mut Self {
        self.body.push(op::BR_IF);
        uleb(&mut self.body, depth as u64);
        self
    }

    pub fn load8(&mut self, offset: u32) -> &mut Self {
        self.body.push(op::I32_LOAD8_U);
        self.body.push(0);
        uleb(&mut self.body, offset as u64);
        self
    }

    pub fn load16(&mut self, offset: u32) -> &mut Self {
        self.body.push(op::I32_LOAD16_U);
        self.body.push(1);
        uleb(&mut self.body, offset as u64);
        self
    }

    pub fn load32(&mut self, offset: u32) -> &mut Self {
        self.body.push(op::I32_LOAD);
        self.body.push(0);
        uleb(&mut self.body, offset as u64);
        self
    }

    pub fn load64(&mut self, offset: u32) -> &mut Self {
        self.body.push(op::I64_LOAD);
        self.body.push(0);
        uleb(&mut self.body, offset as u64);
        self
    }

    pub fn loadf64(&mut self, offset: u32) -> &mut Self {
        self.body.push(op::F64_LOAD);
        self.body.push(0);
        uleb(&mut self.body, offset as u64);
        self
    }

    pub fn store8(&mut self, offset: u32) -> &mut Self {
        self.body.push(op::I32_STORE8);
        self.body.push(0);
        uleb(&mut self.body, offset as u64);
        self
    }

    pub fn store16(&mut self, offset: u32) -> &mut Self {
        self.body.push(op::I32_STORE16);
        self.body.push(1);
        uleb(&mut self.body, offset as u64);
        self
    }

    pub fn store32(&mut self, offset: u32) -> &mut Self {
        self.body.push(op::I32_STORE);
        self.body.push(0);
        uleb(&mut self.body, offset as u64);
        self
    }

    pub fn store64(&mut self, offset: u32) -> &mut Self {
        self.body.push(op::I64_STORE);
        self.body.push(0);
        uleb(&mut self.body, offset as u64);
        self
    }

    pub fn storef64(&mut self, offset: u32) -> &mut Self {
        self.body.push(op::F64_STORE);
        self.body.push(0);
        uleb(&mut self.body, offset as u64);
        self
    }

    /// `memory.size` (memory 0) — pushes the page count as i32.
    pub fn memory_size(&mut self) -> &mut Self {
        self.body.push(op::MEMORY_SIZE);
        self.body.push(0);
        self
    }

    /// `memory.grow` (memory 0) — pops the page count, pushes the
    /// previous page count (-1 on failure).
    pub fn memory_grow(&mut self) -> &mut Self {
        self.body.push(op::MEMORY_GROW);
        self.body.push(0);
        self
    }

    pub fn br_table(&mut self, targets: &[u32], default: u32) -> &mut Self {
        self.body.push(op::BR_TABLE);
        uleb(&mut self.body, targets.len() as u64);
        for target in targets {
            uleb(&mut self.body, *target as u64);
        }
        uleb(&mut self.body, default as u64);
        self
    }

    pub fn memory_copy(&mut self) -> &mut Self {
        self.body.push(op::BULK_PREFIX);
        uleb(&mut self.body, op::MEMORY_COPY_SUB);
        self.body.extend_from_slice(&[0, 0]);
        self
    }

    pub fn memory_fill(&mut self) -> &mut Self {
        self.body.push(op::BULK_PREFIX);
        uleb(&mut self.body, op::MEMORY_FILL_SUB);
        self.body.push(0);
        self
    }
}

/// Encode the code-section locals list: runs of equal value types.
pub fn enc_locals(out: &mut Vec<u8>, locals: &[Val]) {
    let mut runs: Vec<(u32, u8)> = Vec::new();
    for local in locals {
        if let Some(last) = runs.last_mut()
            && last.1 == local.byte()
        {
            last.0 += 1;
            continue;
        }
        runs.push((1, local.byte()));
    }
    uleb(out, runs.len() as u64);
    for (count, byte) in runs {
        uleb(out, count as u64);
        out.push(byte);
    }
}
