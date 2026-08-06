//! WebAssembly backend (wasm32 / WASI).
//!
//! This backend lowers L++ MIR directly to a binary WebAssembly module —
//! there is no external toolchain step (no wat, no wasm-ld, no object
//! files). Everything is emitted through the small hand-rolled binary
//! encoder in this file, so the compiler keeps its zero-extra-dependency
//! promise: `lpp hello.lpp --target wasm32-wasi` produces a ready-to-run
//! `.wasm` module.
//!
//! ## Output profile
//!
//! * Module imports one WASI function, `wasi_snapshot_preview1.fd_write`,
//!   used to implement the `print*` family. Any WASI-capable runtime
//!   (wasmtime, wasmer, wazero, browser shims) can execute the module.
//! * Exports `_start` (the WASI command entry point, wrapping the L++ `main`
//!   function) and the linear `memory`.
//! * Strings use a wasm-local representation: a pointer into static data
//!   where the 4 bytes before the pointer hold the byte length. All strings
//!   in the current subset are immortal literals baked into the data
//!   section, so there is no ARC traffic for them.
//!
//! ## Supported subset (v1)
//!
//! `Int` / `Float` / `Bool` / `Char` scalar arithmetic and comparisons,
//! `Str` *literals* and `Str` values passed around (printable via
//! `print_str`, measurable via `str_len`, comparable via `str_eq`),
//! functions + recursion, and structured control flow (`if` / `while` /
//! early `return`). Unsupported features — structs, enums, lists, maps,
//! tuples, closures, async tasks, threads, slices, SIMD, FFI — are rejected
//! up front with a precise "WebAssembly backend does not yet support …"
//! diagnostic rather than silently mis-emitting.
//!
//! ## Control-flow strategy
//!
//! MIR is a goto-based CFG; WebAssembly is structured. Instead of needing a
//! full Relooper, functions are laid out in reverse post-order inside one
//! `block…loop` dispatcher:
//!
//! ```wat
//! (block $bad
//!   (loop $dispatch
//!     (block $L_n-1 … (block $L_0
//!       (br_table $L_0 … $L_n-1 $bad (local.get $disp))) … end $L_0
//!       <body 0> ;; sits between end $L_0 and end $L_1
//!     … end $L_1
//!     <body 1>
//!     …
//!     <body n-1>
//!   ) ;; loop
//!   unreachable
//! ) ;; $bad
//! ```
//!
//! A terminator targeting a block that is *later* in the layout is a plain
//! `br`, so hot forward paths cost exactly one instruction. Back-edges
//! (loops) bounce through the dispatcher exactly once per iteration, which
//! engines compile to a jump table. Every MIR CFG — reducible or not — is
//! therefore handled without special cases.

use std::collections::HashMap;

use crate::ast::BinaryOperator;
use crate::mir::ir::*;
use crate::type_facts::AbiClass;
use crate::types::{TypeRef, TypeTable};

// ── Binary encoder primitives ────────────────────────────────────────────────

/// Unsigned LEB128.
fn uleb(out: &mut Vec<u8>, mut value: u64) {
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
fn sleb(out: &mut Vec<u8>, mut value: i64) {
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
fn enc_name(out: &mut Vec<u8>, name: &str) {
    uleb(out, name.len() as u64);
    out.extend_from_slice(name.as_bytes());
}

/// Sections are length-prefixed; accumulate into a scratch buffer and splice.
fn enc_section(out: &mut Vec<u8>, id: u8, payload: &[u8]) {
    out.push(id);
    uleb(out, payload.len() as u64);
    out.extend_from_slice(payload);
}

// ── Value types ──────────────────────────────────────────────────────────────

/// The three wasm value types this backend produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Val {
    I32,
    I64,
    F64,
}

impl Val {
    fn byte(self) -> u8 {
        match self {
            Val::I32 => 0x7f,
            Val::I64 => 0x7e,
            Val::F64 => 0x7c,
        }
    }
}

/// Mapping of a MIR-visible L++ type onto a wasm value type for locals and
/// signatures.
///
/// * `Bool` (L++ I8 ABI) widens to `i32` — wasm has no byte locals, and the
///   canonical value is still 0/1 exactly like the native backends.
/// * Pointers (`Str`, …) are `i32` offsets into linear memory (wasm32).
/// * `Void` locals map to `i64`, mirroring the Cranelift backend, which maps
///   the Void ABI class to I64 so that "unused result" placeholders have a
///   home. Void *signatures* simply have no results/params.
fn val_of_type(ty: &TypeRef) -> Val {
    match ty.abi_class() {
        AbiClass::I8 | AbiClass::Pointer => Val::I32,
        AbiClass::Void | AbiClass::I64 => Val::I64,
        AbiClass::F64 => Val::F64,
        // VectorI64x2 never reaches here: the up-front validation rejects it.
        AbiClass::VectorI64x2 => Val::I64,
    }
}

// ── Opcode mnemonics ─────────────────────────────────────────────────────────

mod op {
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
    pub const DROP: u8 = 0x1a;
    pub const LOCAL_GET: u8 = 0x20;
    pub const LOCAL_SET: u8 = 0x21;
    pub const LOCAL_TEE: u8 = 0x22;
    pub const I32_LOAD: u8 = 0x28;
    pub const I32_LOAD8_U: u8 = 0x2c;
    pub const I32_STORE: u8 = 0x36;
    pub const I32_STORE8: u8 = 0x3a;
    pub const I32_CONST: u8 = 0x41;
    pub const I64_CONST: u8 = 0x42;
    pub const F64_CONST: u8 = 0x44;
    pub const I32_EQZ: u8 = 0x45;
    pub const I32_GE_U: u8 = 0x4f;
    pub const I32_NE: u8 = 0x47;
    pub const I64_EQZ: u8 = 0x50;
    pub const I64_EQ: u8 = 0x51;
    pub const I64_NE: u8 = 0x52;
    pub const I64_LT_S: u8 = 0x53;
    pub const I64_GT_S: u8 = 0x55;
    pub const I64_LE_S: u8 = 0x57;
    pub const I64_GE_S: u8 = 0x59;
    pub const F64_EQ: u8 = 0x61;
    pub const F64_NE: u8 = 0x62;
    pub const F64_LT: u8 = 0x63;
    pub const F64_GT: u8 = 0x64;
    pub const F64_LE: u8 = 0x65;
    pub const F64_GE: u8 = 0x66;
    pub const I32_EQ: u8 = 0x46;
    pub const I32_LT_S: u8 = 0x48;
    pub const I32_GT_S: u8 = 0x4a;
    pub const I32_LE_S: u8 = 0x4c;
    pub const I32_GE_S: u8 = 0x4e;
    pub const I32_ADD: u8 = 0x6a;
    pub const I32_SUB: u8 = 0x6b;
    pub const I32_MUL: u8 = 0x6c;
    pub const I32_DIV_S: u8 = 0x6d;
    pub const I32_REM_S: u8 = 0x6f;
    pub const I32_AND: u8 = 0x71;
    pub const I32_OR: u8 = 0x72;
    pub const I32_XOR: u8 = 0x73;
    pub const I32_SHL: u8 = 0x74;
    pub const I32_SHR_S: u8 = 0x75;
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
    pub const F64_ABS: u8 = 0x99;
    pub const F64_TRUNC: u8 = 0x9d;
    pub const F64_ADD: u8 = 0xa0;
    pub const F64_SUB: u8 = 0xa1;
    pub const F64_MUL: u8 = 0xa2;
    pub const F64_DIV: u8 = 0xa3;
    pub const I32_WRAP_I64: u8 = 0xa7;
    pub const I64_EXTEND_I32_U: u8 = 0xad;
    pub const I64_TRUNC_F64_U: u8 = 0xb1;
    pub const BLOCK_VOID: u8 = 0x40; // empty block type
}

// ── Static memory layout ─────────────────────────────────────────────────────

/// iovec for `fd_write`: `[ptr i32][len i32]` at offsets 0 and 4.
const IOVEC_BUF: u32 = 0;
const IOVEC_LEN: u32 = 4;
/// `fd_write` writes the count of bytes written here.
const FD_WRITE_OUT: u32 = 8;
/// 64-byte scratch used by the number formatters.
const NUM_BUF: u32 = 16;
const NUM_BUF_SIZE: u32 = 64;
/// Start of the interned literal pool. Every entry is
/// `[len i32][bytes…]`, 4-aligned; code receives a pointer to the bytes.
const POOL_START: u32 = NUM_BUF + NUM_BUF_SIZE;

// ── Feature validation ───────────────────────────────────────────────────────

/// Builtins the wasm backend implements natively inside the module.
const SUPPORTED_BUILTINS: [&str; 7] = [
    "lpp_print_int",
    "lpp_print_bool",
    "lpp_print_float",
    "lpp_print_str",
    "lpp_str_len",
    "lpp_str_eq",
    "fmod",
];

fn wasm_type_error(ty: &TypeRef, where_: &str) -> String {
    let feature = match ty {
        TypeRef::Custom(_) => "custom structs".to_string(),
        TypeRef::Generic(name, _) => format!("{} containers/generics", name),
        TypeRef::Function => "closures and function values".to_string(),
        TypeRef::Tuple(_) => "tuples".to_string(),
        TypeRef::StrSlice | TypeRef::Slice(_) => "borrowed slices".to_string(),
        TypeRef::Task(_) => "async tasks".to_string(),
        TypeRef::VectorI64x2 => "SIMD vector types".to_string(),
        other => format!("type {:?}", other),
    };
    format!(
        "WebAssembly backend does not yet support {} (in {}). Native targets are unaffected; only the wasm32 backend is limited.",
        feature, where_
    )
}

fn validate_local_type(ty: &TypeRef, where_: &str) -> Result<(), String> {
    match ty {
        TypeRef::Int | TypeRef::Float | TypeRef::Bool | TypeRef::Char | TypeRef::Str
        | TypeRef::Void => Ok(()),
        other => Err(wasm_type_error(other, where_)),
    }
}

/// Validate the whole program for the supported wasm subset and prescan the
/// builtins used so codegen can plan function indices and imports.
fn validate_program(program: &MirProgram) -> Result<HashMap<String, u32>, String> {
    let mut builtin_uses: HashMap<String, u32> = HashMap::new();

    let mut functions: Vec<&MirFunction> = program.functions.values().collect();
    functions.sort_by_key(|f| f.id.0);

    for function in functions {
        if function.is_async {
            return Err(format!(
                "WebAssembly backend does not yet support async functions (in '{}')",
                function.name
            ));
        }
        validate_local_type(
            &function.return_type,
            &format!("return type of '{}'", function.name),
        )?;
        for local in &function.locals {
            validate_local_type(&local.ty, &format!("local in '{}'", function.name))?;
        }
        for block in &function.blocks {
            for instr in &block.instrs {
                match instr {
                    MirInstr::Assign(_, rvalue) => {
                        validate_rvalue(rvalue, &function.name, &mut builtin_uses)?
                    }
                    MirInstr::AssignField { field, .. } => {
                        return Err(format!(
                            "WebAssembly backend does not yet support custom structs (field store '{}' in '{}')",
                            field, function.name
                        ));
                    }
                    // ARC traffic on `Str` is a no-op under wasm: every string
                    // in the supported subset is an immortal static literal.
                    MirInstr::Retain(local) | MirInstr::Release(local) => {
                        let ty = &function.locals[local.0].ty;
                        if *ty != TypeRef::Str {
                            return Err(wasm_type_error(
                                ty,
                                &format!("ARC op in '{}'", function.name),
                            ));
                        }
                    }
                }
            }
        }
    }
    Ok(builtin_uses)
}

fn validate_rvalue(
    rvalue: &Rvalue,
    fn_name: &str,
    builtin_uses: &mut HashMap<String, u32>,
) -> Result<(), String> {
    let unsupported = |feature: &str| -> String {
        format!(
            "WebAssembly backend does not yet support {} (in '{}')",
            feature, fn_name
        )
    };
    match rvalue {
        Rvalue::Use(_) | Rvalue::Move(_) | Rvalue::BinaryOp(..) | Rvalue::CallDirect(..) => Ok(()),
        Rvalue::BuiltinCall(symbol, _) => {
            if SUPPORTED_BUILTINS.contains(&symbol.as_str()) {
                *builtin_uses.entry(symbol.clone()).or_insert(0) += 1;
                Ok(())
            } else {
                Err(format!(
                    "WebAssembly backend does not yet provide builtin '{}' (in '{}'). \
                     Available on wasm32 today: {}.",
                    symbol,
                    fn_name,
                    SUPPORTED_BUILTINS.join(", ")
                ))
            }
        }
        Rvalue::AllocateTuple(..) | Rvalue::TupleField(..) => Err(unsupported("tuples")),
        Rvalue::MakeSlice { .. }
        | Rvalue::SliceLen(_)
        | Rvalue::SliceGet(..)
        | Rvalue::SliceToStr(_) => Err(unsupported("borrowed slices")),
        Rvalue::MakeTask(..) | Rvalue::Await(_) => Err(unsupported("async tasks")),
        Rvalue::MakeClosure(..) | Rvalue::MakeStackClosure(..) | Rvalue::CallIndirect(..) => {
            Err(unsupported("closures"))
        }
        Rvalue::FieldAccess(..) => Err(unsupported("custom structs (field access)")),
        Rvalue::AllocateStruct(_)
        | Rvalue::AllocateArcStruct(_)
        | Rvalue::AllocateArenaStruct(..)
        | Rvalue::AllocateStackStruct(_) => Err(unsupported("custom structs")),
        Rvalue::AllocateList(_) => Err(unsupported("lists")),
        Rvalue::SpawnThread(_) => Err(unsupported("threads (spawn)")),
        Rvalue::FuncRef(_) => Err(unsupported("function pointers")),
    }
}

// ── Compiler ─────────────────────────────────────────────────────────────────

/// Which synthesized runtime functions a module needs. Declaration order is
/// also emission order, so derived `Ord` keeps indices deterministic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Helper {
    Write,
    WriteU64,
    PrintInt,
    PrintBool,
    PrintFloat,
    PrintStr,
    StrLen,
    StrEq,
}

const ALL_HELPERS: [Helper; 8] = [
    Helper::Write,
    Helper::WriteU64,
    Helper::PrintInt,
    Helper::PrintBool,
    Helper::PrintFloat,
    Helper::PrintStr,
    Helper::StrLen,
    Helper::StrEq,
];

fn helper_signature(helper: Helper) -> (Vec<Val>, Vec<Val>) {
    match helper {
        Helper::Write => (vec![Val::I32, Val::I32], vec![]),
        Helper::WriteU64 => (vec![Val::I64], vec![]),
        Helper::PrintInt => (vec![Val::I64], vec![]),
        Helper::PrintBool => (vec![Val::I32], vec![]),
        Helper::PrintFloat => (vec![Val::F64], vec![]),
        Helper::PrintStr => (vec![Val::I32], vec![]),
        Helper::StrLen => (vec![Val::I32], vec![Val::I64]),
        Helper::StrEq => (vec![Val::I32, Val::I32], vec![Val::I32]),
    }
}

struct WasmCompiler<'a> {
    program: &'a MirProgram,
    /// MIR function → wasm function index.
    fn_index: HashMap<FuncId, u32>,
    /// Helper → wasm function index.
    helper_index: HashMap<Helper, u32>,
    /// wasm index of the imported `fd_write`.
    fd_write_index: u32,
    /// wasm index of `_start`.
    start_index: u32,
    /// Interned literal → payload address (pointer handed to code).
    literals: HashMap<String, u32>,
    /// Absolute-address scratch buffer; valid bytes start at POOL_START.
    pool: Vec<u8>,
    /// Registered deduped signatures: (params, results) → type index.
    types: Vec<(Vec<Val>, Vec<Val>)>,
    type_map: HashMap<(Vec<Val>, Vec<Val>), u32>,
    /// Per wasm-function-index function names (name section).
    names: Vec<(u32, String)>,
}

impl<'a> WasmCompiler<'a> {
    fn new(program: &'a MirProgram) -> Self {
        Self {
            program,
            fn_index: HashMap::new(),
            helper_index: HashMap::new(),
            fd_write_index: 0,
            start_index: 0,
            literals: HashMap::new(),
            pool: vec![0u8; POOL_START as usize],
            types: Vec::new(),
            type_map: HashMap::new(),
            names: Vec::new(),
        }
    }

    fn register_type(&mut self, params: Vec<Val>, results: Vec<Val>) -> u32 {
        let key = (params, results);
        if let Some(&idx) = self.type_map.get(&key) {
            return idx;
        }
        let idx = self.types.len() as u32;
        self.types.push(key.clone());
        self.type_map.insert(key, idx);
        idx
    }

    /// Intern bytes into the static pool, returning the payload pointer.
    fn intern(&mut self, bytes: &[u8]) -> u32 {
        let key = String::from_utf8_lossy(bytes).into_owned();
        self.intern_key(&key)
    }

    fn intern_key(&mut self, key: &str) -> u32 {
        if let Some(&addr) = self.literals.get(key) {
            return addr;
        }
        // Align each entry to 4 bytes so the length header reads aligned.
        while self.pool.len() % 4 != 0 {
            self.pool.push(0);
        }
        let base = self.pool.len() as u32;
        self.pool.extend_from_slice(&(key.len() as u32).to_le_bytes());
        self.pool.extend_from_slice(key.as_bytes());
        let payload = base + 4;
        self.literals.insert(key.to_string(), payload);
        payload
    }

    fn lookup_literal(&self, value: &str) -> u32 {
        *self
            .literals
            .get(value)
            .expect("string literals are interned during the pre-codegen walk")
    }

    /// Decide which helpers to emit, assign wasm indices, and register all
    /// signatures. The import section occupies index 0 (`fd_write`).
    fn plan_indices(&mut self, builtin_uses: &HashMap<String, u32>) -> Result<(), String> {
        let mut used: Vec<Helper> = Vec::new();
        let mut need = |h: Helper, used: &mut Vec<Helper>| {
            if !used.contains(&h) {
                used.push(h);
            }
        };

        if builtin_uses.contains_key("lpp_print_int")
            || builtin_uses.contains_key("lpp_print_bool")
            || builtin_uses.contains_key("lpp_print_float")
            || builtin_uses.contains_key("lpp_print_str")
        {
            need(Helper::Write, &mut used);
        }
        if builtin_uses.contains_key("lpp_print_int") || builtin_uses.contains_key("lpp_print_float")
        {
            need(Helper::WriteU64, &mut used);
        }
        if builtin_uses.contains_key("lpp_print_int") {
            need(Helper::PrintInt, &mut used);
        }
        if builtin_uses.contains_key("lpp_print_bool") {
            need(Helper::PrintBool, &mut used);
        }
        if builtin_uses.contains_key("lpp_print_float") {
            need(Helper::PrintFloat, &mut used);
        }
        if builtin_uses.contains_key("lpp_print_str") {
            need(Helper::PrintStr, &mut used);
        }
        if builtin_uses.contains_key("lpp_str_len") {
            need(Helper::StrLen, &mut used);
        }
        if builtin_uses.contains_key("lpp_str_eq") {
            need(Helper::StrEq, &mut used);
        }
        used.sort();

        // fd_write import is function index 0; its type is registered first.
        self.fd_write_index = 0;
        self.register_type(
            vec![Val::I32, Val::I32, Val::I32, Val::I32],
            vec![Val::I32],
        );

        // User functions follow imports, in deterministic MIR id order.
        let mut functions: Vec<&MirFunction> = self.program.functions.values().collect();
        functions.sort_by_key(|f| f.id.0);
        let mut next = 1u32;
        for function in &functions {
            self.register_type(
                user_params(function),
                user_results(function),
            );
            self.fn_index.insert(function.id, next);
            self.names.push((next, function.name.clone()));
            next += 1;
        }

        for helper in ALL_HELPERS {
            if !used.contains(&helper) {
                continue;
            }
            let sig = helper_signature(helper);
            self.register_type(sig.0, sig.1);
            self.helper_index.insert(helper, next);
            self.names
                .push((next, format!("__lpp_wasm_{:?}", helper).to_lowercase()));
            next += 1;
        }

        // `_start` is always emitted (the WASI command entry point).
        self.register_type(vec![], vec![]);
        self.start_index = next;
        self.names.push((next, "_start".to_string()));
        Ok(())
    }

    // ── MIR function lowering ────────────────────────────────────────────

    /// Compute the emission order of blocks: reverse post-order from the
    /// entry, with unreachable blocks appended so every referenced body
    /// still encodes (nothing can branch to them, but the binary must be
    /// well formed).
    fn block_layout(mir_fn: &MirFunction) -> Vec<BlockId> {
        let by_id: HashMap<BlockId, &MirBlock> =
            mir_fn.blocks.iter().map(|b| (b.id, b)).collect();
        let mut order: Vec<BlockId> = Vec::with_capacity(mir_fn.blocks.len());
        let mut visited: std::collections::HashSet<BlockId> =
            std::collections::HashSet::with_capacity(mir_fn.blocks.len());
        if let Some(first) = mir_fn.blocks.first() {
            // Iterative post-order (blocks may number in the thousands).
            let mut stack: Vec<(BlockId, bool)> = vec![(first.id, false)];
            while let Some((id, expanded)) = stack.pop() {
                let Some(block) = by_id.get(&id) else { continue };
                if expanded {
                    order.push(id);
                    continue;
                }
                if !visited.insert(id) {
                    continue;
                }
                stack.push((id, true));
                match &block.terminator {
                    Terminator::Goto(target) => {
                        if !visited.contains(target) {
                            stack.push((*target, false));
                        }
                    }
                    Terminator::If {
                        then_block,
                        else_block,
                        ..
                    }
                    | Terminator::IfCmp {
                        then_block,
                        else_block,
                        ..
                    } => {
                        for target in [*then_block, *else_block] {
                            if !visited.contains(&target) {
                                stack.push((target, false));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        order.reverse();
        // Append blocks the DFS could not reach (dead code), keeping the
        // function's own block order for determinism.
        for block in &mir_fn.blocks {
            if !visited.contains(&block.id) {
                order.push(block.id);
            }
        }
        order
    }

    /// The mapping `LocalId → wasm local index`. Parameters come first.
    fn local_indices(mir_fn: &MirFunction) -> (Vec<u32>, Vec<Val>) {
        let mut index_of = vec![u32::MAX; mir_fn.locals.len()];
        let mut extra_types: Vec<Val> = Vec::new();
        let mut next = 0u32;
        for param_id in &mir_fn.params {
            index_of[param_id.0] = next;
            next += 1;
        }
        for local in &mir_fn.locals {
            if index_of[local.id.0] != u32::MAX {
                continue;
            }
            index_of[local.id.0] = next;
            extra_types.push(val_of_type(&local.ty));
            next += 1;
        }
        (index_of, extra_types)
    }

    fn operand_val(
        &self,
        out: &mut Vec<u8>,
        operand: &Operand,
        local_index: &[u32],
    ) {
        match operand {
            Operand::Local(id) | Operand::Borrowed(id) => {
                out.push(op::LOCAL_GET);
                uleb(out, local_index[id.0] as u64);
            }
            Operand::Int(value) => {
                out.push(op::I64_CONST);
                sleb(out, *value);
            }
            Operand::Float(value) => {
                out.push(op::F64_CONST);
                out.extend_from_slice(&value.to_le_bytes());
            }
            Operand::Bool(value) => {
                out.push(op::I32_CONST);
                sleb(out, if *value { 1 } else { 0 });
            }
            Operand::String(value) => {
                let addr = self.lookup_literal(value);
                out.push(op::I32_CONST);
                sleb(out, addr as i64);
            }
        }
    }

    /// The wasm value class an operand carries, for picking instruction
    /// variants (f64 vs i64 vs i32-family).
    fn operand_class(operand: &Operand, locals: &[LocalDecl]) -> Val {
        match operand {
            Operand::Local(id) | Operand::Borrowed(id) => val_of_type(&locals[id.0].ty),
            Operand::Int(_) => Val::I64,
            Operand::Float(_) => Val::F64,
            Operand::Bool(_) => Val::I32,
            Operand::String(_) => Val::I32,
        }
    }

    /// Emit a comparison producing an i32 0/1.
    fn emit_compare(
        &mut self,
        out: &mut Vec<u8>,
        operator: &BinaryOperator,
        left: &Operand,
        right: &Operand,
        local_index: &[u32],
        locals: &[LocalDecl],
    ) -> Result<(), String> {
        use BinaryOperator::*;
        if !matches!(operator, Eq | NotEq | Less | Greater | LessEq | GreaterEq) {
            return Err(format!(
                "WebAssembly backend: non-comparison operator {:?} reached a fused branch",
                operator
            ));
        }
        let class = Self::operand_class(left, locals);
        self.operand_val(out, left, local_index);
        self.operand_val(out, right, local_index);
        let opcode = match (class, operator) {
            (Val::F64, BinaryOperator::Eq) => op::F64_EQ,
            (Val::F64, BinaryOperator::NotEq) => op::F64_NE,
            (Val::F64, BinaryOperator::Less) => op::F64_LT,
            (Val::F64, BinaryOperator::Greater) => op::F64_GT,
            (Val::F64, BinaryOperator::LessEq) => op::F64_LE,
            (Val::F64, BinaryOperator::GreaterEq) => op::F64_GE,
            (Val::I64, BinaryOperator::Eq) => op::I64_EQ,
            (Val::I64, BinaryOperator::NotEq) => op::I64_NE,
            (Val::I64, BinaryOperator::Less) => op::I64_LT_S,
            (Val::I64, BinaryOperator::Greater) => op::I64_GT_S,
            (Val::I64, BinaryOperator::LessEq) => op::I64_LE_S,
            (Val::I64, BinaryOperator::GreaterEq) => op::I64_GE_S,
            (Val::I32, BinaryOperator::Eq) => op::I32_EQ,
            (Val::I32, BinaryOperator::NotEq) => op::I32_NE,
            (Val::I32, BinaryOperator::Less) => op::I32_LT_S,
            (Val::I32, BinaryOperator::Greater) => op::I32_GT_S,
            (Val::I32, BinaryOperator::LessEq) => op::I32_LE_S,
            (Val::I32, BinaryOperator::GreaterEq) => op::I32_GE_S,
            _ => {
                return Err(format!(
                    "WebAssembly backend: operator {:?} is not a comparison",
                    operator
                ));
            }
        };
        out.push(opcode);
        Ok(())
    }

    /// Emit a binary operation. Leaves exactly one value on the stack.
    fn emit_binary(
        &mut self,
        out: &mut Vec<u8>,
        operator: &BinaryOperator,
        left: &Operand,
        right: &Operand,
        local_index: &[u32],
        locals: &[LocalDecl],
    ) -> Result<(), String> {
        use BinaryOperator::*;
        match operator {
            Eq | NotEq | Less | Greater | LessEq | GreaterEq => {
                self.emit_compare(out, operator, left, right, local_index, locals)?;
            }
            Add | Subtract | Multiply | Divide | Modulo => {
                let class = Self::operand_class(left, locals);
                if class == Val::F64 {
                    match operator {
                        Modulo => {
                            // libm fmod(x, y) == x - trunc(x / y) * y, without
                            // the libm call. Operands are re-emitted; they are
                            // pure (locals/constants) so duplication is safe.
                            self.operand_val(out, left, local_index);
                            self.operand_val(out, left, local_index);
                            self.operand_val(out, right, local_index);
                            out.push(op::F64_DIV);
                            out.push(op::F64_TRUNC);
                            self.operand_val(out, right, local_index);
                            out.push(op::F64_MUL);
                            out.push(op::F64_SUB);
                        }
                        _ => {
                            self.operand_val(out, left, local_index);
                            self.operand_val(out, right, local_index);
                            out.push(match operator {
                                Add => op::F64_ADD,
                                Subtract => op::F64_SUB,
                                Multiply => op::F64_MUL,
                                Divide => op::F64_DIV,
                                _ => unreachable!(),
                            });
                        }
                    }
                } else {
                    self.operand_val(out, left, local_index);
                    self.operand_val(out, right, local_index);
                    out.push(match (class, operator) {
                        (Val::I64, Add) => op::I64_ADD,
                        (Val::I64, Subtract) => op::I64_SUB,
                        (Val::I64, Multiply) => op::I64_MUL,
                        (Val::I64, Divide) => op::I64_DIV_S,
                        (Val::I64, Modulo) => op::I64_REM_S,
                        (Val::I32, Add) => op::I32_ADD,
                        (Val::I32, Subtract) => op::I32_SUB,
                        (Val::I32, Multiply) => op::I32_MUL,
                        (Val::I32, Divide) => op::I32_DIV_S,
                        (Val::I32, Modulo) => op::I32_REM_S,
                        _ => unreachable!(),
                    });
                }
            }
            And | Or | BitAnd | BitOr | BitXor | Shl | Shr => {
                let class = Self::operand_class(left, locals);
                if class == Val::F64 {
                    return Err(format!(
                        "WebAssembly backend: bitwise/shift operator {:?} on Float is not supported",
                        operator
                    ));
                }
                self.operand_val(out, left, local_index);
                self.operand_val(out, right, local_index);
                out.push(match (class, operator) {
                    (Val::I64, And) | (Val::I64, BitAnd) => op::I64_AND,
                    (Val::I64, Or) | (Val::I64, BitOr) => op::I64_OR,
                    (Val::I64, BitXor) => op::I64_XOR,
                    (Val::I64, Shl) => op::I64_SHL,
                    (Val::I64, Shr) => op::I64_SHR_S,
                    (Val::I32, And) | (Val::I32, BitAnd) => op::I32_AND,
                    (Val::I32, Or) | (Val::I32, BitOr) => op::I32_OR,
                    (Val::I32, BitXor) => op::I32_XOR,
                    (Val::I32, Shl) => op::I32_SHL,
                    (Val::I32, Shr) => op::I32_SHR_S,
                    _ => unreachable!(),
                });
            }
        }
        Ok(())
    }

    /// Emit one rvalue, leaving exactly one value on the stack. When a call
    /// has no result, a placeholder i64 zero is produced (matching the other
    /// backends, which assign `iconst 0` into Void temporaries).
    fn emit_rvalue(
        &mut self,
        out: &mut Vec<u8>,
        rvalue: &Rvalue,
        local_index: &[u32],
        mir_fn: &MirFunction,
    ) -> Result<(), String> {
        let locals = &mir_fn.locals;
        match rvalue {
            Rvalue::Use(operand) => self.operand_val(out, operand, local_index),
            Rvalue::Move(local) => {
                self.operand_val(out, &Operand::Local(*local), local_index)
            }
            Rvalue::BinaryOp(operator, left, right) => {
                self.emit_binary(out, operator, left, right, local_index, locals)?
            }
            Rvalue::CallDirect(target, args) => {
                let target_fn = &self.program.functions[target];
                for arg in args {
                    self.operand_val(out, arg, local_index);
                }
                out.push(op::CALL);
                uleb(out, self.fn_index[target] as u64);
                if target_fn.return_type == TypeRef::Void {
                    out.push(op::I64_CONST);
                    sleb(out, 0);
                }
            }
            Rvalue::BuiltinCall(symbol, args) => match symbol.as_str() {
                "lpp_print_int" | "lpp_print_bool" | "lpp_print_float" | "lpp_print_str" => {
                    let helper = match symbol.as_str() {
                        "lpp_print_int" => Helper::PrintInt,
                        "lpp_print_bool" => Helper::PrintBool,
                        "lpp_print_float" => Helper::PrintFloat,
                        _ => Helper::PrintStr,
                    };
                    for arg in args {
                        self.operand_val(out, arg, local_index);
                    }
                    out.push(op::CALL);
                    uleb(out, self.helper_index[&helper] as u64);
                    out.push(op::I64_CONST);
                    sleb(out, 0);
                }
                "lpp_str_len" => {
                    for arg in args {
                        self.operand_val(out, arg, local_index);
                    }
                    out.push(op::CALL);
                    uleb(out, self.helper_index[&Helper::StrLen] as u64);
                }
                "lpp_str_eq" => {
                    for arg in args {
                        self.operand_val(out, arg, local_index);
                    }
                    out.push(op::CALL);
                    uleb(out, self.helper_index[&Helper::StrEq] as u64);
                }
                "fmod" => {
                    // Same expansion as BinaryOperator::Modulo on Float.
                    if args.len() != 2 {
                        return Err("fmod expects exactly two arguments".to_string());
                    }
                    self.operand_val(out, &args[0], local_index);
                    self.operand_val(out, &args[0], local_index);
                    self.operand_val(out, &args[1], local_index);
                    out.push(op::F64_DIV);
                    out.push(op::F64_TRUNC);
                    self.operand_val(out, &args[1], local_index);
                    out.push(op::F64_MUL);
                    out.push(op::F64_SUB);
                }
                other => {
                    return Err(format!(
                        "WebAssembly backend does not yet provide builtin '{}'",
                        other
                    ));
                }
            },
            other => {
                return Err(format!(
                    "WebAssembly backend internal error: unvalidated rvalue {:?} reached codegen",
                    other
                ));
            }
        }
        Ok(())
    }

    /// Emit a branch from body position `from` to layout position `target`.
    ///
    /// `nest` counts extra structured constructs the branch sits inside
    /// (e.g. 1 when emitted inside an `if` arm) — each one shifts every
    /// label depth by one, because the enclosing `if` is itself a label.
    fn emit_branch(
        out: &mut Vec<u8>,
        from: usize,
        target: usize,
        total: usize,
        disp: u32,
        nest: usize,
    ) {
        if target > from {
            // Direct branch: `L_target` is an enclosing block whose end is
            // exactly the start of the target body. Depth skips the blocks
            // between them.
            out.push(op::BR);
            uleb(out, (target - from - 1 + nest) as u64);
        } else {
            // Back-edge or self-edge: bounce through the dispatcher.
            out.push(op::I32_CONST);
            sleb(out, target as i64);
            out.push(op::LOCAL_SET);
            uleb(out, disp as u64);
            out.push(op::BR);
            uleb(out, (total - 1 - from + nest) as u64);
        }
    }

    fn preintern_operand(&mut self, operand: &Operand) {
        if let Operand::String(value) = operand {
            let value = value.clone();
            self.intern_key(&value);
        }
    }

    fn preintern_rvalue(&mut self, rvalue: &Rvalue) {
        match rvalue {
            Rvalue::Use(operand)
            | Rvalue::TupleField(operand, _)
            | Rvalue::SliceLen(operand)
            | Rvalue::SliceToStr(operand)
            | Rvalue::Await(operand)
            | Rvalue::SpawnThread(operand)
            | Rvalue::FieldAccess(operand, _)
            | Rvalue::AllocateArenaStruct(_, operand) => vec![operand],
            Rvalue::BinaryOp(_, left, right) | Rvalue::SliceGet(left, right) => {
                vec![left, right]
            }
            Rvalue::AllocateTuple(_, args)
            | Rvalue::MakeTask(_, _, args, _)
            | Rvalue::CallDirect(_, args)
            | Rvalue::BuiltinCall(_, args)
            | Rvalue::MakeClosure(_, args)
            | Rvalue::MakeStackClosure(_, args) => args.iter().collect::<Vec<&Operand>>(),
            Rvalue::CallIndirect(callee, args) => {
                let mut all = vec![callee];
                all.extend(args.iter());
                all
            }
            Rvalue::MakeSlice {
                base,
                start,
                length,
                ..
            } => vec![base, start, length],
            Rvalue::Move(_)
            | Rvalue::AllocateStruct(_)
            | Rvalue::AllocateArcStruct(_)
            | Rvalue::AllocateStackStruct(_)
            | Rvalue::AllocateList(_)
            | Rvalue::FuncRef(_) => Vec::new(),
        }
        .into_iter()
        .for_each(|operand| self.preintern_operand(operand));
    }

    /// Pre-intern every string literal a function can reference, so codegen
    /// can look up addresses without re-walking.
    fn preintern_function(&mut self, mir_fn: &MirFunction) {
        for block in &mir_fn.blocks {
            for instr in &block.instrs {
                if let MirInstr::Assign(_, rvalue) = instr {
                    self.preintern_rvalue(rvalue);
                }
            }
            match &block.terminator {
                Terminator::If { cond, .. } => self.preintern_operand(cond),
                Terminator::IfCmp { left, right, .. } => {
                    self.preintern_operand(left);
                    self.preintern_operand(right);
                }
                Terminator::Return(Some(operand)) | Terminator::ReturnOwned(operand) => {
                    self.preintern_operand(operand);
                }
                _ => {}
            }
        }
    }

    fn lower_function(&mut self, mir_fn: &MirFunction) -> Result<(Vec<Val>, Vec<u8>), String> {
        if mir_fn.blocks.is_empty() {
            return Err(format!("MIR function '{}' has no blocks", mir_fn.name));
        }
        let (local_index, extra_locals) = Self::local_indices(mir_fn);
        let disp_local = mir_fn.locals.len() as u32;

        let layout = Self::block_layout(mir_fn);
        let mut position: HashMap<BlockId, usize> = HashMap::with_capacity(layout.len());
        for (pos, block_id) in layout.iter().enumerate() {
            position.insert(*block_id, pos);
        }
        let total = layout.len();
        let entry_pos = position[&mir_fn.blocks.first().unwrap().id];

        self.preintern_function(mir_fn);

        let mut out: Vec<u8> = Vec::with_capacity(256);

        // Dispatch state := entry block position.
        out.push(op::I32_CONST);
        sleb(&mut out, entry_pos as i64);
        out.push(op::LOCAL_SET);
        uleb(&mut out, disp_local as u64);

        // (block $bad (loop $dispatch (block $L_{n-1} … (block $L_0 …
        out.push(op::BLOCK);
        out.push(op::BLOCK_VOID);
        out.push(op::LOOP);
        out.push(op::BLOCK_VOID);
        for _ in 0..total {
            out.push(op::BLOCK);
            out.push(op::BLOCK_VOID);
        }
        // br_table $L_0 … $L_{n-1}, default $bad (depth total + 1).
        out.push(op::LOCAL_GET);
        uleb(&mut out, disp_local as u64);
        out.push(op::BR_TABLE);
        uleb(&mut out, total as u64);
        for label in 0..total {
            uleb(&mut out, label as u64);
        }
        uleb(&mut out, (total + 1) as u64);

        for (pos, block_id) in layout.iter().enumerate() {
            // Each `end` closes the block whose tail starts this body; for
            // pos == 0 that is the innermost dispatch block.
            out.push(op::END);
            let block = &mir_fn.blocks[block_id.0];
            for instr in &block.instrs {
                self.lower_instr(&mut out, instr, &local_index, mir_fn)?;
            }
            self.lower_terminator(
                &mut out,
                &block.terminator,
                pos,
                &position,
                total,
                disp_local,
                &local_index,
                mir_fn,
            )?;
        }
        // Close the loop; the $bad landing pad traps, then close $bad.
        out.push(op::END);
        out.push(op::UNREACHABLE);
        out.push(op::END);
        // Function-level final end.
        out.push(op::END);

        // The dispatcher state is an extra i32 local.
        let mut all_extras = extra_locals;
        all_extras.push(Val::I32);
        Ok((all_extras, out))
    }

    fn lower_instr(
        &mut self,
        out: &mut Vec<u8>,
        instr: &MirInstr,
        local_index: &[u32],
        mir_fn: &MirFunction,
    ) -> Result<(), String> {
        match instr {
            MirInstr::Assign(dest, rvalue) => {
                self.emit_rvalue(out, rvalue, local_index, mir_fn)?;
                out.push(op::LOCAL_SET);
                uleb(out, local_index[dest.0] as u64);
            }
            // Validation guarantees reachable Retain/Release only involve
            // `Str`, whose values are immortal literals in this backend.
            MirInstr::Retain(_) | MirInstr::Release(_) => {}
            MirInstr::AssignField { .. } => {
                return Err(
                    "WebAssembly backend internal error: struct store reached codegen".to_string(),
                );
            }
        }
        Ok(())
    }

    fn lower_terminator(
        &mut self,
        out: &mut Vec<u8>,
        terminator: &Terminator,
        from: usize,
        position: &HashMap<BlockId, usize>,
        total: usize,
        disp: u32,
        local_index: &[u32],
        mir_fn: &MirFunction,
    ) -> Result<(), String> {
        match terminator {
            Terminator::Goto(target) => {
                Self::emit_branch(out, from, position[target], total, disp, 0);
            }
            Terminator::If {
                cond,
                then_block,
                else_block,
            } => {
                let class = Self::operand_class(cond, &mir_fn.locals);
                self.operand_val(out, cond, local_index);
                self.coerce_to_i32(out, class);
                // Inside the `if`, every enclosing label is one further away.
                out.push(op::IF);
                out.push(op::BLOCK_VOID);
                Self::emit_branch(out, from, position[then_block], total, disp, 1);
                out.push(op::ELSE);
                Self::emit_branch(out, from, position[else_block], total, disp, 1);
                out.push(op::END);
            }
            Terminator::IfCmp {
                op: operator,
                left,
                right,
                then_block,
                else_block,
            } => {
                self.emit_compare(out, operator, left, right, local_index, &mir_fn.locals)?;
                out.push(op::IF);
                out.push(op::BLOCK_VOID);
                Self::emit_branch(out, from, position[then_block], total, disp, 1);
                out.push(op::ELSE);
                Self::emit_branch(out, from, position[else_block], total, disp, 1);
                out.push(op::END);
            }
            Terminator::Return(Some(operand)) | Terminator::ReturnOwned(operand) => {
                self.operand_val(out, operand, local_index);
                out.push(op::RETURN);
            }
            Terminator::Return(None) => {
                if mir_fn.return_type != TypeRef::Void {
                    // Keep ABI parity with the native backends: implicit
                    // returns produce a zero of the correct value type.
                    match val_of_type(&mir_fn.return_type) {
                        Val::F64 => {
                            out.push(op::F64_CONST);
                            out.extend_from_slice(&0.0f64.to_le_bytes());
                        }
                        Val::I32 => {
                            out.push(op::I32_CONST);
                            sleb(out, 0);
                        }
                        Val::I64 => {
                            out.push(op::I64_CONST);
                            sleb(out, 0);
                        }
                    }
                }
                out.push(op::RETURN);
            }
            Terminator::Unreachable => out.push(op::UNREACHABLE),
        }
        Ok(())
    }

    /// wasm `if` consumes i32; Bool already is i32-canonical 0/1, Int needs a
    /// nonzero test, and Float is rejected by typecheck but mapped defensively.
    fn coerce_to_i32(&self, out: &mut Vec<u8>, class: Val) {
        match class {
            Val::I32 => {}
            Val::I64 => {
                // (x != 0) == !(x == 0)
                out.push(op::I64_EQZ);
                out.push(op::I32_EQZ);
            }
            Val::F64 => {
                out.push(op::F64_CONST);
                out.extend_from_slice(&0.0f64.to_le_bytes());
                out.push(op::F64_NE);
            }
        }
    }

    // ── Helper function bodies ───────────────────────────────────────────

    fn helper_body(&mut self, helper: Helper) -> (Vec<Val>, Vec<u8>) {
        match helper {
            Helper::Write => self.body_write(),
            Helper::WriteU64 => self.body_write_u64(),
            Helper::PrintInt => self.body_print_int(),
            Helper::PrintBool => self.body_print_bool(),
            Helper::PrintFloat => self.body_print_float(),
            Helper::PrintStr => self.body_print_str(),
            Helper::StrLen => self.body_str_len(),
            Helper::StrEq => self.body_str_eq(),
        }
    }

    /// `__lpp_wasm_write(ptr: i32, len: i32)`: fd_write(1, {ptr,len}).
    fn body_write(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut b = Vec::new();
        // iovec.buf = ptr
        b.push(op::I32_CONST);
        sleb(&mut b, IOVEC_BUF as i64);
        b.push(op::LOCAL_GET);
        uleb(&mut b, 0);
        b.push(op::I32_STORE);
        b.extend_from_slice(&[0, 0]);
        // iovec.len = len
        b.push(op::I32_CONST);
        sleb(&mut b, IOVEC_LEN as i64);
        b.push(op::LOCAL_GET);
        uleb(&mut b, 1);
        b.push(op::I32_STORE);
        b.extend_from_slice(&[0, 0]);
        // fd_write(fd=1, iovs=0, iovs_len=1, nwritten=FD_WRITE_OUT)
        for value in [1u32, 0, 1, FD_WRITE_OUT] {
            b.push(op::I32_CONST);
            sleb(&mut b, value as i64);
        }
        b.push(op::CALL);
        uleb(&mut b, self.fd_write_index as u64);
        b.push(op::DROP);
        b.push(op::END);
        (vec![], b)
    }

    /// `__lpp_wasm_write_u64(v: i64)`: print the *unsigned* decimal digits of
    /// `v` (no newline). Unsigned div/rem means the i64 sign bit is just a
    /// magnitude bit, so callers pass `0 - x` for negatives and INT64_MIN
    /// still prints correctly.
    fn body_write_u64(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut b = Vec::new();
        // locals: 0 = v (param), 1 = pos i32
        b.push(op::I32_CONST);
        sleb(&mut b, NUM_BUF_SIZE as i64);
        b.push(op::LOCAL_SET);
        uleb(&mut b, 1);
        // block $done
        b.push(op::BLOCK);
        b.push(op::BLOCK_VOID);
        // loop $digits
        b.push(op::LOOP);
        b.push(op::BLOCK_VOID);
        // pos = pos - 1; addr = NUM_BUF + pos
        b.push(op::LOCAL_GET);
        uleb(&mut b, 1);
        b.push(op::I32_CONST);
        sleb(&mut b, 1);
        b.push(op::I32_SUB);
        b.push(op::LOCAL_TEE);
        uleb(&mut b, 1);
        b.push(op::I32_CONST);
        sleb(&mut b, NUM_BUF as i64);
        b.push(op::I32_ADD);
        // digit = 48 + u32(v % 10)
        b.push(op::LOCAL_GET);
        uleb(&mut b, 0);
        b.push(op::I64_CONST);
        sleb(&mut b, 10);
        b.push(op::I64_REM_U);
        b.push(op::I32_WRAP_I64);
        b.push(op::I32_CONST);
        sleb(&mut b, 48);
        b.push(op::I32_ADD);
        b.push(op::I32_STORE8);
        b.extend_from_slice(&[0, 0]);
        // v /= 10
        b.push(op::LOCAL_GET);
        uleb(&mut b, 0);
        b.push(op::I64_CONST);
        sleb(&mut b, 10);
        b.push(op::I64_DIV_U);
        b.push(op::LOCAL_SET);
        uleb(&mut b, 0);
        // if v == 0 → done else loop
        b.push(op::LOCAL_GET);
        uleb(&mut b, 0);
        b.push(op::I64_EQZ);
        b.push(op::BR_IF);
        uleb(&mut b, 1); // → $done
        b.push(op::BR);
        uleb(&mut b, 0); // → $digits
        b.push(op::END); // loop
        b.push(op::END); // block
        // write(NUM_BUF + pos, NUM_BUF_SIZE - pos)
        b.push(op::I32_CONST);
        sleb(&mut b, NUM_BUF as i64);
        b.push(op::LOCAL_GET);
        uleb(&mut b, 1);
        b.push(op::I32_ADD);
        b.push(op::I32_CONST);
        sleb(&mut b, NUM_BUF_SIZE as i64);
        b.push(op::LOCAL_GET);
        uleb(&mut b, 1);
        b.push(op::I32_SUB);
        b.push(op::CALL);
        uleb(&mut b, self.helper_index[&Helper::Write] as u64);
        b.push(op::END);
        (vec![Val::I32], b)
    }

    /// `__lpp_print_int(x: i64)`: matches the native "%lld\n".
    fn body_print_int(&mut self) -> (Vec<Val>, Vec<u8>) {
        let dash = self.intern(b"-");
        let nl = self.intern(b"\n");
        let write = self.helper_index[&Helper::Write];
        let write_u64 = self.helper_index[&Helper::WriteU64];
        let mut b = Vec::new();
        // locals: 0 = x (param), 1 = mag i64
        b.push(op::LOCAL_GET);
        uleb(&mut b, 0);
        b.push(op::I64_CONST);
        sleb(&mut b, 0);
        b.push(op::I64_LT_S);
        b.push(op::IF);
        b.push(op::BLOCK_VOID);
        // write("-", 1); mag = 0 - x (wrap-around keeps INT64_MIN correct)
        b.push(op::I32_CONST);
        sleb(&mut b, dash as i64);
        b.push(op::I32_CONST);
        sleb(&mut b, 1);
        b.push(op::CALL);
        uleb(&mut b, write as u64);
        b.push(op::I64_CONST);
        sleb(&mut b, 0);
        b.push(op::LOCAL_GET);
        uleb(&mut b, 0);
        b.push(op::I64_SUB);
        b.push(op::LOCAL_SET);
        uleb(&mut b, 1);
        b.push(op::ELSE);
        b.push(op::LOCAL_GET);
        uleb(&mut b, 0);
        b.push(op::LOCAL_SET);
        uleb(&mut b, 1);
        b.push(op::END);
        // write_u64(mag); write("\n", 1)
        b.push(op::LOCAL_GET);
        uleb(&mut b, 1);
        b.push(op::CALL);
        uleb(&mut b, write_u64 as u64);
        b.push(op::I32_CONST);
        sleb(&mut b, nl as i64);
        b.push(op::I32_CONST);
        sleb(&mut b, 1);
        b.push(op::CALL);
        uleb(&mut b, write as u64);
        b.push(op::END);
        (vec![Val::I64], b)
    }

    /// `__lpp_print_bool(b: i32)`: native prints the integer value ("1"/"0")
    /// followed by a newline.
    fn body_print_bool(&mut self) -> (Vec<Val>, Vec<u8>) {
        let one = self.intern(b"1\n");
        let zero = self.intern(b"0\n");
        let write = self.helper_index[&Helper::Write];
        let mut b = Vec::new();
        b.push(op::LOCAL_GET);
        uleb(&mut b, 0);
        b.push(op::IF);
        b.push(op::BLOCK_VOID);
        b.push(op::I32_CONST);
        sleb(&mut b, one as i64);
        b.push(op::I32_CONST);
        sleb(&mut b, 2);
        b.push(op::CALL);
        uleb(&mut b, write as u64);
        b.push(op::ELSE);
        b.push(op::I32_CONST);
        sleb(&mut b, zero as i64);
        b.push(op::I32_CONST);
        sleb(&mut b, 2);
        b.push(op::CALL);
        uleb(&mut b, write as u64);
        b.push(op::END);
        b.push(op::END);
        (vec![], b)
    }

    /// `__lpp_print_str(s: i32)`: bytes + "\n" (matches `puts`).
    fn body_print_str(&mut self) -> (Vec<Val>, Vec<u8>) {
        let nl = self.intern(b"\n");
        let write = self.helper_index[&Helper::Write];
        let mut b = Vec::new();
        // write(s, *(s - 4))
        b.push(op::LOCAL_GET);
        uleb(&mut b, 0);
        b.push(op::LOCAL_GET);
        uleb(&mut b, 0);
        b.push(op::I32_CONST);
        sleb(&mut b, 4);
        b.push(op::I32_SUB);
        b.push(op::I32_LOAD);
        b.extend_from_slice(&[0, 0]);
        b.push(op::CALL);
        uleb(&mut b, write as u64);
        b.push(op::I32_CONST);
        sleb(&mut b, nl as i64);
        b.push(op::I32_CONST);
        sleb(&mut b, 1);
        b.push(op::CALL);
        uleb(&mut b, write as u64);
        b.push(op::END);
        (vec![], b)
    }

    /// `__lpp_str_len(s: i32) -> i64`.
    fn body_str_len(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut b = Vec::new();
        b.push(op::LOCAL_GET);
        uleb(&mut b, 0);
        b.push(op::I32_CONST);
        sleb(&mut b, 4);
        b.push(op::I32_SUB);
        b.push(op::I32_LOAD);
        b.extend_from_slice(&[0, 0]);
        b.push(op::I64_EXTEND_I32_U);
        b.push(op::END);
        (vec![], b)
    }

    /// `__lpp_str_eq(a: i32, b: i32) -> i32`: byte equality, returning 1/0
    /// to slot into the Bool ABI exactly like the native runtime.
    fn body_str_eq(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut b = Vec::new();
        // params: 0 = a, 1 = b; locals: 2 = la, 3 = lb, 4 = i
        for (param, dest) in [(0u32, 2u32), (1u32, 3u32)] {
            b.push(op::LOCAL_GET);
            uleb(&mut b, param as u64);
            b.push(op::I32_CONST);
            sleb(&mut b, 4);
            b.push(op::I32_SUB);
            b.push(op::I32_LOAD);
            b.extend_from_slice(&[0, 0]);
            b.push(op::LOCAL_SET);
            uleb(&mut b, dest as u64);
        }
        // if la != lb → 0
        b.push(op::LOCAL_GET);
        uleb(&mut b, 2);
        b.push(op::LOCAL_GET);
        uleb(&mut b, 3);
        b.push(op::I32_NE);
        b.push(op::IF);
        b.push(op::BLOCK_VOID);
        b.push(op::I32_CONST);
        sleb(&mut b, 0);
        b.push(op::RETURN);
        b.push(op::END);
        // i = 0
        b.push(op::I32_CONST);
        sleb(&mut b, 0);
        b.push(op::LOCAL_SET);
        uleb(&mut b, 4);
        // loop $cmp
        b.push(op::LOOP);
        b.push(op::BLOCK_VOID);
        // if i >= la → equal
        b.push(op::LOCAL_GET);
        uleb(&mut b, 4);
        b.push(op::LOCAL_GET);
        uleb(&mut b, 2);
        b.push(op::I32_GE_U);
        b.push(op::IF);
        b.push(op::BLOCK_VOID);
        b.push(op::I32_CONST);
        sleb(&mut b, 1);
        b.push(op::RETURN);
        b.push(op::END);
        // if a[i] != b[i] → 0
        b.push(op::LOCAL_GET);
        uleb(&mut b, 0);
        b.push(op::LOCAL_GET);
        uleb(&mut b, 4);
        b.push(op::I32_ADD);
        b.push(op::I32_LOAD8_U);
        b.extend_from_slice(&[0, 0]);
        b.push(op::LOCAL_GET);
        uleb(&mut b, 1);
        b.push(op::LOCAL_GET);
        uleb(&mut b, 4);
        b.push(op::I32_ADD);
        b.push(op::I32_LOAD8_U);
        b.extend_from_slice(&[0, 0]);
        b.push(op::I32_NE);
        b.push(op::IF);
        b.push(op::BLOCK_VOID);
        b.push(op::I32_CONST);
        sleb(&mut b, 0);
        b.push(op::RETURN);
        b.push(op::END);
        // i += 1; continue
        b.push(op::LOCAL_GET);
        uleb(&mut b, 4);
        b.push(op::I32_CONST);
        sleb(&mut b, 1);
        b.push(op::I32_ADD);
        b.push(op::LOCAL_SET);
        uleb(&mut b, 4);
        b.push(op::BR);
        uleb(&mut b, 0);
        b.push(op::END); // loop
        b.push(op::UNREACHABLE);
        b.push(op::END); // func
        (vec![Val::I32, Val::I32, Val::I32], b)
    }

    /// `__lpp_print_float(x: f64)`: matches the native `%f\n` formatting —
    /// six fixed fractional digits, rounded half away from zero, for finite
    /// values with |x| < 9e12 (beyond that the integral part prints with a
    /// zero fraction — a documented limit). NaN and ±inf print their
    /// C spellings.
    fn body_print_float(&mut self) -> (Vec<Val>, Vec<u8>) {
        let dash = self.intern(b"-");
        let dot = self.intern(b".");
        let nl = self.intern(b"\n");
        let nan = self.intern(b"nan\n");
        let inf = self.intern(b"inf\n");
        let neginf = self.intern(b"-inf\n");
        let zeros = self.intern(b".000000\n");
        let write = self.helper_index[&Helper::Write];
        let write_u64 = self.helper_index[&Helper::WriteU64];
        let emit_write = |b: &mut Vec<u8>, addr: u32, len: u32| {
            b.push(op::I32_CONST);
            sleb(b, addr as i64);
            b.push(op::I32_CONST);
            sleb(b, len as i64);
            b.push(op::CALL);
            uleb(b, write as u64);
        };
        let mut b = Vec::new();
        // params: 0 = x f64; locals: 1 = neg i32, 2 = n i64, 3 = pos i32
        // NaN?
        b.push(op::LOCAL_GET);
        uleb(&mut b, 0);
        b.push(op::LOCAL_GET);
        uleb(&mut b, 0);
        b.push(op::F64_NE);
        b.push(op::IF);
        b.push(op::BLOCK_VOID);
        emit_write(&mut b, nan, 4);
        b.push(op::RETURN);
        b.push(op::END);
        // ±inf? (finite * 0.0 == 0.0; NaN was filtered above)
        b.push(op::LOCAL_GET);
        uleb(&mut b, 0);
        b.push(op::F64_CONST);
        b.extend_from_slice(&0.0f64.to_le_bytes());
        b.push(op::F64_MUL);
        b.push(op::F64_CONST);
        b.extend_from_slice(&0.0f64.to_le_bytes());
        b.push(op::F64_NE);
        b.push(op::IF);
        b.push(op::BLOCK_VOID);
        b.push(op::LOCAL_GET);
        uleb(&mut b, 0);
        b.push(op::F64_CONST);
        b.extend_from_slice(&0.0f64.to_le_bytes());
        b.push(op::F64_LT);
        b.push(op::IF);
        b.push(op::BLOCK_VOID);
        emit_write(&mut b, neginf, 5);
        b.push(op::ELSE);
        emit_write(&mut b, inf, 4);
        b.push(op::END);
        b.push(op::RETURN);
        b.push(op::END);
        // neg = x < 0.0 ; x = |x|
        b.push(op::LOCAL_GET);
        uleb(&mut b, 0);
        b.push(op::F64_CONST);
        b.extend_from_slice(&0.0f64.to_le_bytes());
        b.push(op::F64_LT);
        b.push(op::LOCAL_SET);
        uleb(&mut b, 1);
        b.push(op::LOCAL_GET);
        uleb(&mut b, 0);
        b.push(op::F64_ABS);
        b.push(op::LOCAL_SET);
        uleb(&mut b, 0);
        // if neg → "-"
        b.push(op::LOCAL_GET);
        uleb(&mut b, 1);
        b.push(op::IF);
        b.push(op::BLOCK_VOID);
        emit_write(&mut b, dash, 1);
        b.push(op::END);
        // if x < 9e12 → fixed six-digit fraction
        b.push(op::LOCAL_GET);
        uleb(&mut b, 0);
        b.push(op::F64_CONST);
        b.extend_from_slice(&9.0e12f64.to_le_bytes());
        b.push(op::F64_LT);
        b.push(op::IF);
        b.push(op::BLOCK_VOID);
        // n = u64(x * 1e6 + 0.5)
        b.push(op::LOCAL_GET);
        uleb(&mut b, 0);
        b.push(op::F64_CONST);
        b.extend_from_slice(&1_000_000.0f64.to_le_bytes());
        b.push(op::F64_MUL);
        b.push(op::F64_CONST);
        b.extend_from_slice(&0.5f64.to_le_bytes());
        b.push(op::F64_ADD);
        b.push(op::I64_TRUNC_F64_U);
        b.push(op::LOCAL_SET);
        uleb(&mut b, 2);
        // write_u64(n / 1e6)
        b.push(op::LOCAL_GET);
        uleb(&mut b, 2);
        b.push(op::I64_CONST);
        sleb(&mut b, 1_000_000);
        b.push(op::I64_DIV_U);
        b.push(op::CALL);
        uleb(&mut b, write_u64 as u64);
        // "."
        emit_write(&mut b, dot, 1);
        // n = n % 1e6; six digits into NUM_BUF[0..6]
        b.push(op::LOCAL_GET);
        uleb(&mut b, 2);
        b.push(op::I64_CONST);
        sleb(&mut b, 1_000_000);
        b.push(op::I64_REM_U);
        b.push(op::LOCAL_SET);
        uleb(&mut b, 2);
        b.push(op::I32_CONST);
        sleb(&mut b, 6);
        b.push(op::LOCAL_SET);
        uleb(&mut b, 3);
        // loop $frac
        b.push(op::LOOP);
        b.push(op::BLOCK_VOID);
        b.push(op::LOCAL_GET);
        uleb(&mut b, 3);
        b.push(op::I32_CONST);
        sleb(&mut b, 1);
        b.push(op::I32_SUB);
        b.push(op::LOCAL_TEE);
        uleb(&mut b, 3);
        b.push(op::I32_CONST);
        sleb(&mut b, NUM_BUF as i64);
        b.push(op::I32_ADD);
        b.push(op::LOCAL_GET);
        uleb(&mut b, 2);
        b.push(op::I64_CONST);
        sleb(&mut b, 10);
        b.push(op::I64_REM_U);
        b.push(op::I32_WRAP_I64);
        b.push(op::I32_CONST);
        sleb(&mut b, 48);
        b.push(op::I32_ADD);
        b.push(op::I32_STORE8);
        b.extend_from_slice(&[0, 0]);
        b.push(op::LOCAL_GET);
        uleb(&mut b, 2);
        b.push(op::I64_CONST);
        sleb(&mut b, 10);
        b.push(op::I64_DIV_U);
        b.push(op::LOCAL_SET);
        uleb(&mut b, 2);
        b.push(op::LOCAL_GET);
        uleb(&mut b, 3);
        b.push(op::BR_IF);
        uleb(&mut b, 0);
        b.push(op::END); // loop
        // write(NUM_BUF, 6); "\n"
        emit_write(&mut b, NUM_BUF, 6);
        emit_write(&mut b, nl, 1);
        b.push(op::ELSE);
        // Huge finite values: integral part plus a fixed ".000000".
        b.push(op::LOCAL_GET);
        uleb(&mut b, 0);
        b.push(op::I64_TRUNC_F64_U);
        b.push(op::CALL);
        uleb(&mut b, write_u64 as u64);
        emit_write(&mut b, zeros, 8);
        b.push(op::END);
        b.push(op::END);
        (vec![Val::I32, Val::I64, Val::I32], b)
    }

    /// `_start`: the WASI entry point; calls the user `main` and discards
    /// any result so the exported function keeps its `() -> ()` signature
    /// (mirroring the native C-ABI `main` wrapper, which returns 0).
    fn body_start(&self, main_id: FuncId) -> Vec<u8> {
        let mut b = Vec::new();
        b.push(op::CALL);
        uleb(&mut b, self.fn_index[&main_id] as u64);
        if self.program.functions[&main_id].return_type != TypeRef::Void {
            b.push(op::DROP);
        }
        b.push(op::END);
        b
    }

    // ── Module assembly ──────────────────────────────────────────────────

    fn compile(mut self) -> Result<Vec<u8>, String> {
        // `_start` needs an entry point to call.
        let main_id = self
            .program
            .functions
            .iter()
            .find(|(_, f)| f.name == "main")
            .map(|(id, _)| *id)
            .ok_or_else(|| {
                "WebAssembly backend: program has no 'main' function to export as _start"
                    .to_string()
            })?;

        let builtin_uses = validate_program(self.program)?;
        self.plan_indices(&builtin_uses)?;

        // Lower all user functions (deterministic MIR id order).
        let mut functions: Vec<&MirFunction> = self.program.functions.values().collect();
        functions.sort_by_key(|f| f.id.0);
        let mut user_codes: Vec<(Vec<Val>, Vec<u8>)> = Vec::with_capacity(functions.len());
        for function in &functions {
            user_codes.push(self.lower_function(function)?);
        }

        // Helper bodies (index order == sorted helper order).
        let mut helpers: Vec<Helper> = self.helper_index.keys().copied().collect();
        helpers.sort();
        let mut helper_codes: Vec<(Vec<Val>, Vec<u8>)> = Vec::with_capacity(helpers.len());
        for helper in &helpers {
            helper_codes.push(self.helper_body(*helper));
        }

        let start_body = self.body_start(main_id);

        // ── Assemble sections ──
        let mut module: Vec<u8> = Vec::with_capacity(4096);
        module.extend_from_slice(&[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]);

        // Type section.
        {
            let mut payload = Vec::new();
            uleb(&mut payload, self.types.len() as u64);
            for (params, results) in &self.types {
                payload.push(0x60);
                uleb(&mut payload, params.len() as u64);
                for param in params {
                    payload.push(param.byte());
                }
                uleb(&mut payload, results.len() as u64);
                for result in results {
                    payload.push(result.byte());
                }
            }
            enc_section(&mut module, 1, &payload);
        }

        // Import section: fd_write only (function index 0, type index 0).
        {
            let mut payload = Vec::new();
            uleb(&mut payload, 1);
            enc_name(&mut payload, "wasi_snapshot_preview1");
            enc_name(&mut payload, "fd_write");
            payload.push(0x00); // func import
            uleb(&mut payload, 0); // type index 0 (registered first)
            enc_section(&mut module, 2, &payload);
        }

        // Function section: user functions, then helpers, then _start.
        let total_defined = user_codes.len() + helper_codes.len() + 1;
        {
            let mut payload = Vec::new();
            uleb(&mut payload, total_defined as u64);
            for function in &functions {
                let sig = (user_params(function), user_results(function));
                uleb(&mut payload, self.type_map[&sig] as u64);
            }
            for helper in &helpers {
                let sig = helper_signature(*helper);
                uleb(&mut payload, self.type_map[&sig] as u64);
            }
            uleb(
                &mut payload,
                self.type_map[&(Vec::<Val>::new(), Vec::<Val>::new())] as u64,
            );
            enc_section(&mut module, 3, &payload);
        }

        // Memory section: sized to fit the static pool.
        {
            let pages = (self.pool.len() as u64 + 65535) / 65536;
            let mut payload = Vec::new();
            uleb(&mut payload, 1);
            payload.push(0x00); // limits: min only
            uleb(&mut payload, pages.max(1));
            enc_section(&mut module, 5, &payload);
        }

        // Export section: memory + _start.
        {
            let mut payload = Vec::new();
            uleb(&mut payload, 2);
            enc_name(&mut payload, "memory");
            payload.push(0x02); // memory export
            uleb(&mut payload, 0);
            enc_name(&mut payload, "_start");
            payload.push(0x00); // func export
            uleb(&mut payload, self.start_index as u64);
            enc_section(&mut module, 7, &payload);
        }

        // Code section.
        {
            let mut payload = Vec::new();
            uleb(&mut payload, total_defined as u64);
            fn encode_code(payload: &mut Vec<u8>, extras: &[Val], body: &[u8]) {
                let mut entry = Vec::new();
                // Group consecutive equal local types.
                let mut groups: Vec<(u32, Val)> = Vec::new();
                for val in extras {
                    match groups.last_mut() {
                        Some((count, last)) if *last == *val => *count += 1,
                        _ => groups.push((1, *val)),
                    }
                }
                uleb(&mut entry, groups.len() as u64);
                for (count, val) in groups {
                    uleb(&mut entry, count as u64);
                    entry.push(val.byte());
                }
                entry.extend_from_slice(body);
                uleb(payload, entry.len() as u64);
                payload.extend_from_slice(&entry);
            }
            for (extras, body) in &user_codes {
                encode_code(&mut payload, extras, body);
            }
            for (extras, body) in &helper_codes {
                encode_code(&mut payload, extras, body);
            }
            encode_code(&mut payload, &[], &start_body);
            enc_section(&mut module, 10, &payload);
        }

        // Data section: one active segment covering the whole static pool.
        if self.pool.len() > POOL_START as usize {
            let mut payload = Vec::new();
            uleb(&mut payload, 1);
            payload.push(0x00); // active, memory 0
            payload.push(op::I32_CONST);
            sleb(&mut payload, POOL_START as i64);
            payload.push(op::END);
            let data = &self.pool[POOL_START as usize..];
            uleb(&mut payload, data.len() as u64);
            payload.extend_from_slice(data);
            enc_section(&mut module, 11, &payload);
        }

        // Name section (custom) — makes wasmtime stack traces readable.
        {
            let mut payload = Vec::new();
            enc_name(&mut payload, "name");
            // subsection 1: function names
            let mut sub = Vec::new();
            uleb(&mut sub, self.names.len() as u64);
            for (idx, name) in &self.names {
                uleb(&mut sub, *idx as u64);
                enc_name(&mut sub, name);
            }
            payload.push(1);
            uleb(&mut payload, sub.len() as u64);
            payload.extend_from_slice(&sub);
            enc_section(&mut module, 0, &payload);
        }

        Ok(module)
    }
}

fn user_params(function: &MirFunction) -> Vec<Val> {
    function
        .params
        .iter()
        .map(|pid| val_of_type(&function.locals[pid.0].ty))
        .collect()
}

fn user_results(function: &MirFunction) -> Vec<Val> {
    if function.return_type == TypeRef::Void {
        Vec::new()
    } else {
        vec![val_of_type(&function.return_type)]
    }
}

// ── Public entry point ───────────────────────────────────────────────────────

/// Compile a whole MIR program to a WebAssembly module (wasm32 / WASI).
pub fn compile(program: &MirProgram, _type_table: &TypeTable) -> Result<Vec<u8>, String> {
    WasmCompiler::new(program).compile()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uleb_encodes_known_values() {
        let mut out = Vec::new();
        uleb(&mut out, 624485);
        assert_eq!(out, vec![0xE5, 0x8E, 0x26]);
        let mut out = Vec::new();
        uleb(&mut out, 0);
        assert_eq!(out, vec![0x00]);
        let mut out = Vec::new();
        uleb(&mut out, 127);
        assert_eq!(out, vec![0x7F]);
        let mut out = Vec::new();
        uleb(&mut out, 128);
        assert_eq!(out, vec![0x80, 0x01]);
    }

    #[test]
    fn sleb_encodes_known_values() {
        let mut out = Vec::new();
        sleb(&mut out, -123456);
        assert_eq!(out, vec![0xC0, 0xBB, 0x78]);
        let mut out = Vec::new();
        sleb(&mut out, 0);
        assert_eq!(out, vec![0x00]);
        let mut out = Vec::new();
        sleb(&mut out, -1);
        assert_eq!(out, vec![0x7F]);
        let mut out = Vec::new();
        sleb(&mut out, 63);
        assert_eq!(out, vec![0x3F]);
        let mut out = Vec::new();
        sleb(&mut out, 64);
        assert_eq!(out, vec![0xC0, 0x00]);
        let mut out = Vec::new();
        sleb(&mut out, -64);
        assert_eq!(out, vec![0x40]);
    }

    #[test]
    fn locals_map_params_first() {
        let function = MirFunction {
            id: FuncId(0),
            name: "f".to_string(),
            params: vec![LocalId(0)],
            locals: vec![
                LocalDecl {
                    id: LocalId(0),
                    ty: TypeRef::Int,
                    is_mut: false,
                    debug_name: None,
                    binding_id: None,
                    ownership: Ownership::Copy,
                },
                LocalDecl {
                    id: LocalId(1),
                    ty: TypeRef::Bool,
                    is_mut: true,
                    debug_name: None,
                    binding_id: None,
                    ownership: Ownership::Copy,
                },
            ],
            blocks: vec![],
            start_block: BlockId(0),
            return_type: TypeRef::Void,
            is_async: false,
        };
        let (index_of, extras) = WasmCompiler::local_indices(&function);
        assert_eq!(index_of, vec![0, 1]);
        assert_eq!(extras, vec![Val::I32]);
    }

    #[test]
    fn rejects_unsupported_features_with_clear_errors() {
        let mut program = MirProgram {
            functions: HashMap::new(),
        };
        program.functions.insert(
            FuncId(0),
            MirFunction {
                id: FuncId(0),
                name: "main".to_string(),
                params: vec![],
                locals: vec![LocalDecl {
                    id: LocalId(0),
                    ty: TypeRef::Int,
                    is_mut: false,
                    debug_name: None,
                    binding_id: None,
                    ownership: Ownership::Copy,
                }],
                blocks: vec![MirBlock {
                    id: BlockId(0),
                    instrs: vec![MirInstr::Assign(
                        LocalId(0),
                        Rvalue::AllocateList(TypeRef::Int),
                    )],
                    terminator: Terminator::Return(None),
                }],
                start_block: BlockId(0),
                return_type: TypeRef::Void,
                is_async: false,
            },
        );
        let error = compile(&program, &TypeTable::new()).unwrap_err();
        assert!(error.contains("WebAssembly"), "unexpected error: {}", error);
        assert!(error.contains("lists"), "unexpected error: {}", error);
    }

    #[test]
    fn minimal_valid_module_emits() {
        // def main(): return
        let mut program = MirProgram {
            functions: HashMap::new(),
        };
        program.functions.insert(
            FuncId(0),
            MirFunction {
                id: FuncId(0),
                name: "main".to_string(),
                params: vec![],
                locals: vec![],
                blocks: vec![MirBlock {
                    id: BlockId(0),
                    instrs: vec![],
                    terminator: Terminator::Return(None),
                }],
                start_block: BlockId(0),
                return_type: TypeRef::Void,
                is_async: false,
            },
        );
        let module = compile(&program, &TypeTable::new()).expect("compiles");
        assert_eq!(&module[..8], &[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]);
        assert!(module.windows(6).any(|w| w == b"_start"));
    }
}
