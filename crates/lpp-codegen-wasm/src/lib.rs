//! Phase 5D: hand-written binary WebAssembly backend (`wasm32-wasi`).
//!
//! Slice 1 (data surface): scalars (`Int`/`Float`/`Bool`/`Char`), strings,
//! control flow (`if`/`else`, loops), direct calls (including recursion),
//! and the `print`/`write_str`/`str_len` builtins — all over WASI.
//!
//! Slice 2 (5D2a, managed data surface): structs, enums, lists, and the
//! ARC heap. Managed values are 32-bit *payload* offsets into a bump
//! allocator that lives behind the static string pool (a `heap` global);
//! each node carries the unified 24-byte header
//! `[rc i64][drop-index i64][magic "ARC1"]` (payload at +24) shared with
//! the pool strings and the native 5C ABI. The ARC model is
//! retain-on-transfer, mirroring the 5C/5C2 Cranelift lowering and the
//! `execute_mir_arc` interpreter: a managed value is retained when a new
//! owner is created (a `Use(Copy)` transfer, a projected field/element
//! read, a `Copy` source of a store/construction/argument, a returned
//! value); it is released when an owning slot is overwritten and, at
//! every `Return`, by a release pass over all managed locals in
//! declaration order. Destructors are generated per nominal
//! (`lpp_drop_s{raw}`/`lpp_drop_e{raw}`, in `MirAggregateId` order) plus a
//! list destroyer (`lpp_drop_list`); they are tabled in a funcref table
//! (slot 0 = a no-op "no destructor") and dispatched by `Release`
//! through `call_indirect`.
//!
//! The current parity surface also includes tuples, dynamic string `+`,
//! closures/function values, async tasks, restricted eager `spawn`, borrowed
//! list/string slices, deterministic maps, WASI line input, and the complete
//! positive legacy WASM corpus. Typed rejections remain for SIMD, arbitrary
//! C FFI, and host capabilities unavailable in WASI preview 1.
//!
//! CFG strategy: each function is one dispatch loop over its basic blocks
//! (in reverse post-order). A `current` local holds the block position:
//! ```text
//! current = entry
//! loop:
//!   if (current == 0) { <bb0> }
//!   if (current == 1) { <bb1> }
//!   ...
//! ```
//! Each block's terminator reassigns `current` (`goto`/`branch`) or
//! `return`s / traps. No forward `br`, no label-distance arithmetic.
#![allow(clippy::all, warnings)]

mod encode;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use lpp_codegen_api::{
    Backend, CodegenError, CodegenErrorKind, CodegenOptions, CompiledModule, NameResolver, Target,
    layout::{AggregateIndex, AggregateLayout, aggregate_layout},
};
use lpp_mir::{
    BasicBlockId, BinaryOperator, Constant, InstructionKind, MirAggregateId, MirFunctionId,
    MirLocalId, MirProgram, Operand, PlaceProjection, Rvalue, Terminator, UnaryOperator,
};
use lpp_types::{BuiltinId, PrimitiveType, TypeId, TypeInterner, TypeKind};

use encode::{FB, Val, enc_locals, enc_name, enc_section, op, sleb, uleb};

// ---------------------------------------------------------------------------
// Memory layout. Everything lives in the low memory page:
//
//   0..8    iovec buffer for `fd_write`  [ptr i32][len i32]
//   8       `fd_write` result (i32)
//   16..80  numeric / char formatter scratch (digits written downward from 80)
//   80..    static string-literal pool
//   (pool end, 8-aligned)..  the ARC bump heap (grows with `memory.grow`)
//
// A string literal is `[rc i64][drop i64][magic i64][len i32][bytes……]`; the
// string value is a pointer to the `len` field (base + 24), content at +4.
// A managed (struct/enum/list) value is a pointer to the payload at
// `base + 24` of a heap node `[rc i64][drop-index i64][magic "ARC1"]`.
// ---------------------------------------------------------------------------
const IOVEC_BUF: u32 = 0;
const FD_IO_OUT: u32 = 8;
const NUM_BUF: u32 = 16;
const NUM_BUF_END: u32 = 80;
const POOL_START: u32 = 80;
const STR_HEADER: u32 = 24;
const ARC_HEADER: i64 = 24;
const IMMORTAL_RC: i64 = i64::MAX;
const ARC_MAGIC: i64 = 0x41_52_43_31; // "ARC1"
/// The wasm heap-pointer global index (emitted only when the heap exists).
const GLOBAL_HEAP: u32 = 0;

// List payload (32 bytes of i64 slots, mirroring the v1 wasm list):
//   [data i64 @0][len i64 @8][cap i64 @16][is_arc i64 @24]
const LIST_DATA: i64 = 0;
const LIST_LEN: i64 = 8;
const LIST_CAP: i64 = 16;
const LIST_IS_ARC: i64 = 24;
const LIST_NEW_CAP: i64 = 8;
const LIST_CAP_MAX: i64 = 0x1000_0000;

// Borrowed slice view (24 raw bump-heap bytes; deliberately not ARC-owned):
//   [base i32 @0][padding @4][start i64 @8][len i64 @16]
// The source borrow checker guarantees that `base` outlives the view.
const SLICE_BASE: i64 = 0;
const SLICE_START: i64 = 8;
const SLICE_LEN: i64 = 16;
const SLICE_SIZE: i64 = 24;

// Map payload and linear-entry layout. Maps use deterministic linear search;
// growth leaves old bump-heap storage behind, matching the backend's other
// growable containers. Each entry is `[key i64][value i64]`.
const MAP_DATA: i64 = 0;
const MAP_LEN: i64 = 8;
const MAP_CAP: i64 = 16;
const MAP_STR_KEYS: i64 = 24;
const MAP_SIZE: i64 = 32;
const MAP_ENTRY_SIZE: i64 = 16;
const MAP_INITIAL_CAP: i64 = 8;

/// The enum tag lives at payload offset 0 (an i32; bytes 4..8 are the
/// padding of the native 8-byte tag — the shared layout is identical).
const ENUM_TAG: i64 = 0;

/// A runtime helper the lowering may require.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum H {
    FdWrite,
    FdRead,
    Input,
    PrintInt,
    PrintBool,
    PrintStr,
    PrintFloat,
    WriteStr,
    StrEq,
    StrLen,
    // ── 5D2b slice 4, batch 3: string builtins ──
    StrConcat,
    StrContains,
    StrStartsWith,
    StrEndsWith,
    StrFind,
    StrReplace,
    StrSubstr,
    StrRepeat,
    StrSplit,
    CharAt,
    Ord,
    Chr,
    StrTrim,
    StrLower,
    StrUpper,
    IntToStr,
    StrToInt,
    FloatToStr,
    BoolToStr,
    U64ToStr,
    U64ToHex,
    StrToU64,
    // ── freestanding numeric helpers ──
    Log2,
    Exp2,
    Pow,
    Sin,
    Cos,
    // ── 5D2a ARC heap ──
    /// `(size i32) -> node base i32` — 8-aligned bump with `memory.grow`.
    Alloc,
    /// `(size i32, drop-index i32) -> payload i32` — header + payload.
    ArcAlloc,
    /// `(ptr i32) -> ()` — no-op for null / immortal.
    Retain,
    /// `(ptr i32) -> ()` — decrement; last reference runs the tabled
    /// destructor; a double-release traps.
    Release,
    // ── 5D2a lists (payloads of i64 slots) ──
    /// `(is-arc i32) -> list i32`.
    ListNew,
    /// `(list i32, v i64) -> ()` — Int/Bool/Char elements (widened).
    ListPushI64,
    /// `(list i32, v f64) -> ()`.
    ListPushF64,
    /// `(list i32, v i32) -> ()` — managed elements (the caller retains
    /// for the list; the store is plain).
    ListPushPtr,
    /// `(list i32) -> len i64` — 0 for a null list.
    ListLen,
    /// `(list i32, index i64) -> i64` — Int/Bool/Char elements.
    ListGetI64,
    /// `(list i32, index i64) -> f64`.
    ListGetF64,
    /// `(list i32, index i64) -> i32` — managed elements (no retain; the
    /// caller retains for the new owner).
    ListGetPtr,
    /// `(list i32, index i64, v i64) -> ()` — Int/Bool/Char elements.
    ListSetI64,
    /// `(list i32, index i64, v f64) -> ()`.
    ListSetF64,
    /// `(list i32, index i64, v i32) -> ()` — managed elements: releases
    /// the replaced element and retains the new one (runtime-internal,
    /// mirroring `lpp_list_set_arc`).
    ListSetPtr,
    // ── borrowed list/string slices ──
    /// `(list i32, start i64, len i64) -> view i32`.
    SliceNewList,
    /// `(string i32, start i64, len i64) -> view i32`.
    SliceNewStr,
    /// `(view i32) -> len i64`.
    SliceLen,
    /// `(view i32, index i64) -> i64`.
    SliceGetI64,
    /// `(view i32, index i64) -> f64`.
    SliceGetF64,
    /// `(view i32, index i64) -> i32` (borrowed managed element).
    SliceGetPtr,
    /// `(view i32, index i64) -> string i32`.
    StrSliceGet,
    /// `(view i32) -> string i32`.
    StrSliceToStr,
    // ── maps with Int values and Int/String keys ──
    MapNew,
    MapEnsure,
    MapFindI64,
    MapFindStr,
    MapPutI64,
    MapPutStr,
    MapGetI64,
    MapGetStr,
    MapHasI64,
    MapHasStr,
    MapRemoveI64,
    MapRemoveStr,
    MapLen,
    // ── 5D2b slice 3 tasks ──
    /// `(code i32, env i32, managed i32) -> task i32` — a 32-byte ARC
    /// node `{code i64@0, env i64@8, result i64@16, state i32@24,
    /// result_managed i32@28}`; NULL code/env exit 101 (shim parity).
    TaskNew,
    /// `(task i32) -> result i64` — runs the task code exactly once
    /// (state 0→1→2; polling a non-fresh task exits 101).
    TaskPoll,
    /// `(task i32) -> result i64` — polls when fresh and retains a
    /// managed result (the task keeps its share; repeated await).
    TaskAwait,
    /// `(task i32) -> ()` — releases the task node (its destructor
    /// frees the env tuple and, once resolved, the managed result).
    TaskDestroy,
}

/// The wasm value class of an operand (drives op selection).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    I64,  // Int
    F64,  // Float
    Bool, // Bool (i32 0/1)
    Char, // Char (i32 code point)
    Str,  // String (i32 payload pointer; pooled strings use an immortal ARC count)
    Ptr,  // Managed struct/enum/list/tuple (i32 payload offset, ARC-tracked)
}

/// Whether `ty` is ARC-managed on the wasm ABI.
fn is_managed(types: &TypeInterner, ty: TypeId) -> bool {
    matches!(
        types.kind(ty),
        TypeKind::Primitive(PrimitiveType::String)
            | TypeKind::Nominal { .. }
            | TypeKind::List(_)
            | TypeKind::Map { .. }
            | TypeKind::Tuple(_)
            | TypeKind::Function { .. }
            | TypeKind::Task(_)
    )
}

/// The list-element storage class (drives helper selection).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ElementClass {
    I64, // Int / Bool / Char (zero-extended i64 slots)
    F64, // Float
    Ptr, // Managed
}

fn element_class(types: &TypeInterner, element: TypeId) -> ElementClass {
    match types.kind(element) {
        TypeKind::Primitive(PrimitiveType::String)
        | TypeKind::List(_)
        | TypeKind::Tuple(_)
        | TypeKind::Nominal { .. }
        | TypeKind::Function { .. }
        | TypeKind::Task(_) => ElementClass::Ptr,
        TypeKind::Primitive(PrimitiveType::Float) => ElementClass::F64,
        _ => ElementClass::I64,
    }
}

/// One function to emit: signature, extra locals, and the body opcodes.
struct Func {
    name: Option<String>,
    params: Vec<Val>,
    results: Vec<Val>,
    extra: Vec<Val>,
    body: Vec<u8>,
}

/// Everything the emitters need, resolved once.
struct Env<'a> {
    program: &'a MirProgram,
    types: &'a TypeInterner,
    names: &'a dyn NameResolver,
    user_index: HashMap<MirFunctionId, u32>,
    helper_index: HashMap<H, u32>,
    string_ptr: HashMap<String, u32>,
    /// Nominal → drop-table slot (1-based; 0 = no destructor).
    drop_slot: BTreeMap<MirAggregateId, u32>,
    /// The list destroyer's drop-table slot (when lists exist).
    list_drop_slot: Option<u32>,
    /// The type-table index of the `(i32) -> ()` destructor signature.
    drop_call_type: u32,
    /// Nominal field offsets, laid out once in `MirAggregateId` order.
    layouts: BTreeMap<MirAggregateId, AggregateLayout>,
    /// Nominal type id → aggregate instance.
    aggregates: AggregateIndex,
    /// 5D2b: closure/function id → dispatch-table index (the capsule's
    /// `code` word; funcref table 1, functions in `MirFunctionId` order).
    dispatch: HashMap<MirFunctionId, u32>,
    /// 5D2b: closure id → drop-table slot of its env destructor
    /// (`lpp_drop_c{n}`; absent for zero-capture closures).
    env_drop_slot: HashMap<MirFunctionId, u32>,
    /// 5D2b: drop-table slot of `lpp_closure_destroy` (when closures
    /// exist).
    closure_destroy_slot: Option<u32>,
    /// 5D2b: closure function TYPE id → type-table index of its call
    /// signature `(env, user params...) -> result` (shared by every
    /// closure of that type).
    closure_call_type: HashMap<TypeId, u32>,
    /// 5D2b slice 3: async function id → dispatch-table index of its
    /// task thunk (`__lpp_task_thunk{n}`).
    /// Table position (in `user_ids` order) of each user function —
    /// the element index a call_indirect into the dispatch table
    /// must use (the module index differs by the import count).
    user_dispatch: HashMap<MirFunctionId, u32>,
    task_thunk_dispatch: HashMap<MirFunctionId, u32>,
    /// 5D2b slice 3: zero-parameter closure id → dispatch-table index
    /// of its spawn thunk (`__lpp_closure_thunk{n}`).
    closure_thunk_dispatch: HashMap<MirFunctionId, u32>,
    /// 5D2b slice 3: drop-table slot of `lpp_drop_task`.
    task_drop_slot: Option<u32>,
    /// 5D2b slice 3: drop-table slot of `lpp_drop_tuple`.
    tuple_drop_slot: Option<u32>,
    /// 5D2b slice 3: type-table index of the task-code signature
    /// `(i32 env) -> i64 result` (0 when there are no tasks).
    task_code_call_type: u32,
    /// The funcref table index of the dispatch table (table 1 when a
    /// drop table exists, table 0 otherwise).
    dispatch_table: u8,
    /// 5D2b slice 3: function-value provenance (from the pre-scan).
    value_origins: BTreeMap<(MirFunctionId, MirLocalId), MirFunctionId>,
}

/// Per-function lowering state (local-index map + dispatch-local slot).
struct FnLower<'a> {
    env: &'a Env<'a>,
    local_index: HashMap<MirLocalId, u32>,
    cur_local: u32,
    fn_id: MirFunctionId,
    /// 5D2b: capture local → env slot index (value and cell captures).
    /// A capture local is a VIEW of its env slot — assigning to it
    /// writes the new value back to the env.
    captures: HashMap<MirLocalId, u32>,
}

pub struct WasmBackend;

impl Backend for WasmBackend {
    fn name(&self) -> &'static str {
        "wasm32-wasi"
    }

    fn targets(&self) -> &'static [Target] {
        &[Target::Wasm32Wasi]
    }

    fn compile_module(
        &self,
        program: &MirProgram,
        types: &TypeInterner,
        options: &CodegenOptions<'_>,
    ) -> Result<CompiledModule, CodegenError> {
        if options.target != Target::Wasm32Wasi {
            return Err(CodegenError::new(
                None,
                CodegenErrorKind::UnsupportedTarget(options.target),
            ));
        }
        let mut module = compile_module(program, types, options.names)?;
        // Optional one-time production pass: post-process the object with
        // wasm-opt. The raw level (OptLevel::None) leaves the object untouched
        // and byte-identical; a non-None level is deterministic for a fixed
        // wasm-opt version (a system tool, like clang for the native
        // backends).
        if let Some(flag) = options.opt_level.wasm_flag() {
            module.object = run_wasm_opt(&module.object, flag)?;
        }
        wasmparser::Validator::new()
            .validate_all(&module.object)
            .map_err(|error| {
                CodegenError::new(
                    None,
                    CodegenErrorKind::ObjectEmissionFailed(format!(
                        "generated WebAssembly failed validation: {error}"
                    )),
                )
            })?;
        Ok(module)
    }
}

/// Post-process a wasm object with `wasm-opt` (a system tool, like `clang`
/// for the native backends — not a Rust dependency). Deterministic for a
/// fixed wasm-opt version. Fails with a clear message if Binaryen is absent.
fn run_wasm_opt(object: &[u8], flag: &str) -> Result<Vec<u8>, CodegenError> {
    use std::process::Command;

    let fail = |msg: String| CodegenError::new(None, CodegenErrorKind::ObjectEmissionFailed(msg));

    let dir = std::env::temp_dir().join(format!("lpp_wasmopt_{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| fail(format!("wasm-opt temp dir: {e}")))?;
    let in_path = dir.join("in.wasm");
    let out_path = dir.join("out.wasm");
    std::fs::write(&in_path, object).map_err(|e| fail(format!("wasm-opt write: {e}")))?;

    let output = Command::new("wasm-opt")
        .arg(flag)
        .arg(&in_path)
        .arg("-o")
        .arg(&out_path)
        .output()
        .map_err(|e| {
            fail(format!(
                "wasm-opt not found or failed to launch ({e}); install Binaryen to use {flag}"
            ))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(fail(format!("wasm-opt {flag} failed: {stderr}")));
    }
    let optimized = std::fs::read(&out_path).map_err(|e| fail(format!("wasm-opt read: {e}")))?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(optimized)
}

fn compile_module(
    program: &MirProgram,
    types: &TypeInterner,
    names: &dyn NameResolver,
) -> Result<CompiledModule, CodegenError> {
    let _zzz_out = std::env::var("ZZZ_OUT");
    // Aggregates in stable MirAggregateId order — the order of the
    // generated destructors and their drop-table slots (slot 0 is the
    // "no destructor" no-op).
    let aggregates: Vec<MirAggregateId> = program.aggregates().map(|(id, _)| id).collect();
    let mut layouts: BTreeMap<MirAggregateId, AggregateLayout> = BTreeMap::new();
    let mut drop_slot: BTreeMap<MirAggregateId, u32> = BTreeMap::new();
    for (pos, &id) in aggregates.iter().enumerate() {
        layouts.insert(id, aggregate_layout(program, types, id));
        drop_slot.insert(id, (pos + 1) as u32);
    }
    let aggregate_index = AggregateIndex::build(program);
    let plan = pre_scan(program, types, names, &layouts, &aggregate_index)?;
    // The dispatch TABLE positions are deterministic from the plan:
    // the element segment is [users (user_ids order), task thunks
    // (plan order), closure thunks (plan order)], so thunk i sits at
    // user_count + its plan index. Fill the maps now — before `_start`
    // (and every user body) reads them; the emission loop below only
    // records the MODULE indices.
    // Closures, function values, and tasks ride the ARC heap (capsule /
    // env / tuple / task nodes), so such a program gets a heap and a
    // drop table even without lists or structs.
    let has_heap = !aggregates.is_empty()
        || plan.has_list
        || plan.has_tuple
        || !plan.closures.is_empty()
        || plan.has_function_value
        || plan.has_tasks
        // String-producing builtins (5D2b slice 4) allocate on the ARC
        // bump heap without any aggregate type in the program.
        || plan.needs.contains(&H::Alloc);
    let list_drop_slot = plan.has_list.then(|| (aggregates.len() + 1) as u32);

    // Imports in deterministic order: output, input, then process exit.
    let mut import_fields: Vec<&str> = Vec::new();
    if plan.needs.contains(&H::FdWrite) {
        import_fields.push("fd_write");
    }
    if plan.needs.contains(&H::FdRead) {
        import_fields.push("fd_read");
    }
    import_fields.push("proc_exit");
    let fd_write_idx = import_fields
        .iter()
        .position(|f| *f == "fd_write")
        .unwrap_or(0) as u32;
    let fd_read_idx = import_fields
        .iter()
        .position(|f| *f == "fd_read")
        .unwrap_or(0) as u32;
    let proc_exit_idx = import_fields
        .iter()
        .position(|f| *f == "proc_exit")
        .unwrap() as u32;

    // Build the string pool (sorted by content → deterministic).
    let (string_ptr, pool_bytes) = build_pool(&plan.strings);
    let pool_end = POOL_START + pool_bytes.len() as u32;
    // The bump heap starts 8-aligned, just past the pool.
    let heap_start = (pool_end + 7) & !7;

    // 5D2b: the dispatch table holds every user function (sync, async,
    // closure) in `MirFunctionId` order, followed by the 5D2b slice 3
    // task/closure thunks; a capsule's `code` word is its index there
    // (an async function value points at its task thunk, not the
    // function itself).
    let all_fn_ids: Vec<(MirFunctionId, lpp_mir::MirFunctionKind)> =
        program.functions().map(|(id, f)| (id, f.kind)).collect();
    let user_ids: Vec<MirFunctionId> = all_fn_ids.iter().map(|(id, _)| *id).collect();
    let user_count = user_ids.len() as u32;
    let dispatch: HashMap<MirFunctionId, u32> = user_ids
        .iter()
        .enumerate()
        .map(|(pos, &id)| (id, pos as u32))
        .collect();
    // The thunk dispatch maps are filled while the thunk bodies are
    // emitted (their module indices only exist at that point — the
    // helpers and destructors land between the users and the
    // thunks). The user functions are lowered after the thunks
    // because a task call embeds its thunk's module index.
    let user_dispatch: HashMap<MirFunctionId, u32> = HashMap::new();
    let task_thunk_dispatch: HashMap<MirFunctionId, u32> = HashMap::new();
    let closure_thunk_dispatch: HashMap<MirFunctionId, u32> = HashMap::new();
    let mut thunk_func_indices: Vec<u32> = Vec::new();

    // Closure env destructors (`lpp_drop_c{n}`) take the drop-table slots
    // after the aggregates (and the list destroyer, when present), in
    // `MirFunctionId` order over closures with at least one capture;
    // `lpp_closure_destroy` takes the final slot.
    let mut env_drop_slot: HashMap<MirFunctionId, u32> = HashMap::new();
    let mut next_drop_slot = match list_drop_slot {
        Some(slot) => slot + 1,
        None => (aggregates.len() + 1) as u32,
    };
    for &closure_id in &plan.closures {
        let captures = program.function_captures(program.function(closure_id).unwrap());
        if !captures.is_empty() {
            env_drop_slot.insert(closure_id, next_drop_slot);
            next_drop_slot += 1;
        }
    }
    // Function-value capsules (5D2b slice 2) reuse the same destroyer.
    let closure_destroy_slot = (!plan.closures.is_empty() || plan.has_function_value).then(|| {
        let slot = next_drop_slot;
        next_drop_slot += 1;
        slot
    });
    // Task runtime destructors (5D2b slice 3), after the closure
    // destroyer: `lpp_drop_task`, then `lpp_drop_tuple`.
    let task_drop_slot = plan.has_tasks.then(|| next_drop_slot);
    if plan.has_tasks {
        next_drop_slot += 1;
    }
    let tuple_drop_slot = (plan.has_tasks || plan.has_tuple).then(|| next_drop_slot);

    let mut env = Env {
        program,
        types,
        names,
        user_index: HashMap::new(),
        helper_index: HashMap::new(),
        string_ptr,
        drop_slot: drop_slot.clone(),
        list_drop_slot,
        drop_call_type: 0,
        layouts,
        aggregates: aggregate_index,
        dispatch,
        env_drop_slot,
        closure_destroy_slot,
        closure_call_type: HashMap::new(),
        user_dispatch,
        task_thunk_dispatch,
        closure_thunk_dispatch,
        task_drop_slot,
        tuple_drop_slot,
        task_code_call_type: 0,
        dispatch_table: if has_heap { 1 } else { 0 },
        value_origins: plan.value_origins,
    };

    // Lowered functions, in stable MirFunctionId order (async
    // functions lower too; their thunks call them directly).
    let base = import_fields.len() as u32;
    for (pos, fn_id) in user_ids.iter().enumerate() {
        env.user_index.insert(*fn_id, base + pos as u32);
        env.user_dispatch.insert(*fn_id, pos as u32);
    }
    // Task thunks: plan order (the drained async main is included).
    for (i, &fn_id) in plan.task_thunks.iter().enumerate() {
        env.task_thunk_dispatch.insert(fn_id, user_count + i as u32);
    }
    // Closure thunks: after the task thunks.
    for (i, &fn_id) in plan.closure_thunks.iter().enumerate() {
        env.closure_thunk_dispatch
            .insert(fn_id, user_count + plan.task_thunks.len() as u32 + i as u32);
    }
    // Needed helpers, fixed order.
    let helpers: Vec<H> = [
        H::FdWrite,
        H::FdRead,
        H::Input,
        H::PrintInt,
        H::PrintBool,
        H::PrintStr,
        H::PrintFloat,
        H::WriteStr,
        H::StrEq,
        H::StrLen,
        H::Alloc,
        H::ArcAlloc,
        H::Retain,
        H::Release,
        H::ListNew,
        H::ListPushI64,
        H::ListPushF64,
        H::ListPushPtr,
        H::ListLen,
        H::ListGetI64,
        H::ListGetF64,
        H::ListGetPtr,
        H::ListSetI64,
        H::ListSetF64,
        H::ListSetPtr,
        H::SliceNewList,
        H::SliceNewStr,
        H::SliceLen,
        H::SliceGetI64,
        H::SliceGetF64,
        H::SliceGetPtr,
        H::StrSliceGet,
        H::StrSliceToStr,
        H::MapNew,
        H::MapEnsure,
        H::MapFindI64,
        H::MapFindStr,
        H::MapPutI64,
        H::MapPutStr,
        H::MapGetI64,
        H::MapGetStr,
        H::MapHasI64,
        H::MapHasStr,
        H::MapRemoveI64,
        H::MapRemoveStr,
        H::MapLen,
        H::TaskNew,
        H::TaskPoll,
        H::TaskAwait,
        H::TaskDestroy,
        // 5D2b slice 4, batch 3: string builtins
        H::StrConcat,
        H::StrContains,
        H::StrStartsWith,
        H::StrEndsWith,
        H::StrFind,
        H::StrReplace,
        H::StrSubstr,
        H::StrRepeat,
        H::StrSplit,
        H::CharAt,
        H::Ord,
        H::Chr,
        H::StrTrim,
        H::StrLower,
        H::StrUpper,
        H::IntToStr,
        H::StrToInt,
        H::FloatToStr,
        H::BoolToStr,
        H::U64ToStr,
        H::U64ToHex,
        H::StrToU64,
        H::Log2,
        H::Exp2,
        H::Pow,
        H::Sin,
        H::Cos,
    ]
    .into_iter()
    .filter(|h| plan.needs.contains(h))
    .collect();
    for (pos, h) in helpers.iter().enumerate() {
        env.helper_index.insert(*h, base + user_count + pos as u32);
    }
    let start_index = base + user_count + helpers.len() as u32;

    // Memory size (at least the pool; the heap grows at runtime).
    let min_pages = pool_end.div_ceil(65536).max(1);

    // Type table: imports first (deterministic), then the destructor
    // signature `(i32) -> ()` (needed by `Release`'s `call_indirect`
    // before the bodies exist), then functions in first-use order,
    // deduped by signature.
    let mut type_list: Vec<(Vec<Val>, Vec<Val>)> = Vec::new();
    let mut type_map: HashMap<(Vec<Val>, Vec<Val>), u32> = HashMap::new();
    let import_tys: Vec<u32> = import_fields
        .iter()
        .map(|f| {
            let (p, r) = if matches!(*f, "fd_write" | "fd_read") {
                (vec![Val::I32; 4], vec![Val::I32])
            } else {
                (vec![Val::I32], vec![])
            };
            reg_type(&mut type_map, &mut type_list, &p, &r)
        })
        .collect();
    if has_heap {
        env.drop_call_type = reg_type(&mut type_map, &mut type_list, &[Val::I32], &[]);
    }
    // 5D2b slice 3: the task-code signature `(env) -> result word`
    // — every task thunk and closure thunk has exactly this shape
    // (the thunk takes the task env tuple; the callee's dispatch
    // index is the call_indirect element index), and
    // `lpp_task_poll` dispatches through it.
    if plan.has_tasks {
        env.task_code_call_type = reg_type(&mut type_map, &mut type_list, &[Val::I32], &[Val::I64]);
    }
    // 5D2b: every sync function's call signature (env, user params...)
    // -> result. Closures and bare function values dispatch through the
    // same table and the same call types (a function value's env word
    // is NULL). The frame's capture slots are `Capture`-kind parameters
    // (not user parameters) — they come from the env, not the stack.
    for &(fn_id, kind) in &all_fn_ids {
        if matches!(kind, lpp_mir::MirFunctionKind::Async) {
            continue;
        }
        let function = program.function(fn_id).unwrap();
        let mut params = vec![Val::I32]; // env
        for &param in program.function_parameters(function) {
            if matches!(
                program.local(param).unwrap().kind,
                lpp_mir::MirLocalKind::Capture
            ) {
                continue;
            }
            let ty = program.local(param).unwrap().ty;
            params.push(local_val(types, ty)?);
        }
        let results = if is_void(types, function.return_type) {
            vec![]
        } else {
            vec![local_val(types, function.return_type)?]
        };
        env.closure_call_type.insert(
            function.ty,
            reg_type(&mut type_map, &mut type_list, &params, &results),
        );
    }

    let mut funcs: Vec<Func> = Vec::new();
    for _ in &user_ids {
        funcs.push(Func {
            name: None,
            params: Vec::new(),
            results: Vec::new(),
            extra: Vec::new(),
            body: Vec::new(),
        });
    }
    // The printing helpers call the 2-arg `lpp_wasm_fd_write` helper, which in
    // turn drives the 4-arg WASI `fd_write` import.
    let fd_helper_idx = env
        .helper_index
        .get(&H::FdWrite)
        .copied()
        .unwrap_or(fd_write_idx);
    for &h in &helpers {
        let (params, results, extra, body) = lower_helper(
            &env,
            h,
            fd_helper_idx,
            fd_write_idx,
            fd_read_idx,
            proc_exit_idx,
        )?;
        funcs.push(Func {
            name: None,
            params,
            results,
            extra,
            body,
        });
    }
    // `_start`
    {
        let mut fb = FB::new(0);
        let main_index = env.user_index.get(&plan.main).unwrap();
        if plan.main_async {
            // The async entry is drained here — the execution boundary
            // holds no references: empty env tuple, task, await,
            // destroy (5D2b slice 3). The awaited result word is the
            // exit code.
            let main_fn = program.function(plan.main).unwrap();
            let tuple_slot = env.tuple_drop_slot.expect("task slots planned");
            let main_thunk = *env
                .task_thunk_dispatch
                .get(&plan.main)
                .expect("async main thunk planned");
            let h = |h: H| *env.helper_index.get(&h).expect("task helpers planned");
            fb.i32c(16);
            fb.i32c(tuple_slot as i64);
            fb.call(h(H::ArcAlloc));
            let tuple = fb.scratch(Val::I32);
            fb.t(tuple);
            fb.g(tuple).i64c(0).store64(0);
            fb.g(tuple).i64c(0).store64(8);
            // The tee'd tuple pointer: the stores re-read the
            // scratch, and TaskNew's env argument does the same —
            // drop the original stack copy before pushing the args.
            fb.op(op::DROP);
            let managed = if is_managed(types, main_fn.return_type) {
                1
            } else {
                0
            };
            fb.i32c(main_thunk as i64);
            fb.g(tuple);
            fb.i32c(managed as i64);
            fb.call(h(H::TaskNew));
            // TaskNew returns the node pointer as an i64 word; narrow
            // for the wasm ABI. Plain local SET (no tee): the drain
            // ends with `proc_exit`, so the stack must be empty at
            // every point before the final exit code.
            let task = fb.scratch(Val::I32);
            fb.op(op::I32_WRAP_I64);
            fb.s(task);
            // Await the drained result word into the scratch, then
            // destroy the node (void).
            fb.g(task);
            fb.call(h(H::TaskAwait));
            let word = fb.scratch(Val::I64);
            fb.s(word);
            fb.g(task);
            fb.call(h(H::TaskDestroy));
            fb.g(word);
            if plan.main_void {
                fb.op(op::DROP);
                fb.i32c(0);
            } else if is_managed(types, main_fn.return_type) {
                // Release the awaiter's share at the boundary (void
                // helper: the wrapped pointer is the argument), then
                // exit 0. The re-read word is the Release argument.
                fb.op(op::I32_WRAP_I64);
                fb.call(h(H::Release));
                fb.i32c(0);
            } else {
                fb.op(op::I32_WRAP_I64);
            }
        } else {
            fb.i32c(0); // env = NULL (dispatch ABI)
            fb.call(*main_index);
            if plan.main_void {
                fb.i32c(0);
            } else {
                fb.op(op::I32_WRAP_I64);
            }
        }
        fb.call(proc_exit_idx);
        // The async drain allocates scratch locals (tuple/task/word);
        // they are function locals (not parameters), so encode them.
        funcs.push(Func {
            name: None,
            params: vec![],
            results: vec![],
            extra: fb.extras.clone(),
            body: fb.body,
        });
    }
    // The 5D2a destructors, in MirAggregateId order (matching their
    // drop-table slots), then the list destroyer, then the no-op "no
    // destructor" that occupies table slot 0.
    let mut drop_func_indices: Vec<u32> = Vec::new();
    for &id in &aggregates {
        let (name, params, results, extra, body) = lower_destructor(&env, id)?;
        drop_func_indices.push(base + funcs.len() as u32);
        funcs.push(Func {
            name: Some(name),
            params,
            results,
            extra,
            body,
        });
    }
    let mut list_drop_index: Option<u32> = None;
    if has_heap && plan.has_list {
        let (name, params, results, extra, body) = lower_list_destructor(&env)?;
        list_drop_index = Some(base + funcs.len() as u32);
        funcs.push(Func {
            name: Some(name),
            params,
            results,
            extra,
            body,
        });
    }
    let mut noop_drop_index: Option<u32> = None;
    if has_heap {
        noop_drop_index = Some(base + funcs.len() as u32);
        let fb = FB::new(1);
        funcs.push(Func {
            name: Some("lpp_drop_none".to_string()),
            params: vec![Val::I32],
            results: vec![],
            extra: fb.extras.clone(),
            body: fb.body,
        });
    }
    // 5D2b: the closure env destructors (`lpp_drop_c{n}`, in
    // `MirFunctionId` order over capturing closures — matching the
    // drop-table slots) and `lpp_closure_destroy`, which releases the
    // capsule's env word (a NULL env is a no-op).
    // Present iff any closure exists (closure creation sets
    // `has_managed`, which pulls the whole ARC set into `needs`).
    let release_idx: Option<u32> = env.helper_index.get(&H::Release).copied();
    let mut closure_dtor_indices: Vec<u32> = Vec::new();
    for &closure_id in &plan.closures {
        if !env.env_drop_slot.contains_key(&closure_id) {
            continue;
        }
        let release_idx = release_idx.expect("Release registered");
        let function = program.function(closure_id).unwrap();
        // The env slots are the closure frame's capture slots
        // (`Capture`-kind parameters, in slot order).
        let capture_frame: Vec<MirLocalId> = program
            .function_parameters(function)
            .iter()
            .copied()
            .filter(|&p| {
                matches!(
                    program.local(p).unwrap().kind,
                    lpp_mir::MirLocalKind::Capture
                )
            })
            .collect();
        let mut fb = FB::new(1);
        for (i, &slot) in capture_frame.iter().enumerate() {
            let ty = program.local(slot).unwrap().ty;
            if is_managed(types, ty) {
                fb.g(0);
                fb.load32(8 * i as u32);
                fb.call(release_idx);
            }
        }
        closure_dtor_indices.push(base + funcs.len() as u32);
        funcs.push(Func {
            name: Some(format!("lpp_drop_c{}", closure_id.raw())),
            params: vec![Val::I32],
            results: vec![],
            extra: fb.extras.clone(),
            body: fb.body,
        });
    }
    let mut closure_destroy_index: Option<u32> = None;
    if let Some(_slot) = closure_destroy_slot {
        let release_idx = release_idx.expect("Release registered");
        closure_destroy_index = Some(base + funcs.len() as u32);
        let mut fb = FB::new(1);
        fb.g(0).load64(8).op(op::I32_WRAP_I64).call(release_idx);
        funcs.push(Func {
            name: Some("lpp_closure_destroy".to_string()),
            params: vec![Val::I32],
            results: vec![],
            extra: fb.extras.clone(),
            body: fb.body,
        });
    }
    // 5D2b slice 3: `lpp_drop_task` releases the task's env tuple and,
    // once resolved, the managed result; `lpp_drop_tuple` releases the
    // managed argument slots (bit `i` of the mask → slot at `16+8i`).
    let mut task_dtor_index: Option<u32> = None;
    let mut tuple_dtor_index: Option<u32> = None;
    if plan.has_tasks {
        let release_idx = release_idx.expect("Release registered");
        // ── lpp_drop_task ──
        task_dtor_index = Some(base + funcs.len() as u32);
        {
            let mut fb = FB::new(1);
            // Release is void (its pass-through is a caller
            // convenience) — the destructor drops nothing.
            fb.g(0).load64(8).op(op::I32_WRAP_I64).call(release_idx);
            fb.g(0).load32(24).i32c(2).op(op::I32_EQ);
            fb.g(0).load32(28).i32c(1).op(op::I32_EQ);
            fb.op(op::I32_AND);
            fb.if_();
            fb.g(0).load64(16).op(op::I32_WRAP_I64).call(release_idx);
            fb.end();
            funcs.push(Func {
                name: Some("lpp_drop_task".to_string()),
                params: vec![Val::I32],
                results: vec![],
                extra: fb.extras.clone(),
                body: fb.body,
            });
        }
    }
    if plan.has_tasks || plan.has_tuple {
        let release_idx = release_idx.expect("Release registered");
        // ── lpp_drop_tuple ──
        tuple_dtor_index = Some(base + funcs.len() as u32);
        {
            let mut fb = FB::new(1);
            let mask = fb.scratch(Val::I64);
            fb.g(0).load64(0).t(mask);
            // The tee's stack copy is never re-read (every use goes
            // through the local) — drop it.
            fb.op(op::DROP);
            for i in 0..64 {
                fb.g(mask)
                    .i64c(i as i64)
                    .op(op::I64_SHR_U)
                    .i64c(1)
                    .op(op::I64_AND)
                    .i64c(0)
                    .op(op::I64_NE);
                fb.if_();
                fb.g(0)
                    .load64(16 + 8 * i as u32)
                    .op(op::I32_WRAP_I64)
                    .call(release_idx);
                fb.end();
            }
            funcs.push(Func {
                name: Some("lpp_drop_tuple".to_string()),
                params: vec![Val::I32],
                results: vec![],
                extra: fb.extras.clone(),
                body: fb.body,
            });
        }
    }

    // 5D2b slice 3: the thunks. `__lpp_task_thunk{n}` adapts the task
    // env tuple to a call of the (async or zero-parameter sync)
    // function; `__lpp_closure_thunk{n}` adapts it to a call of the
    // zero-parameter closure. Both are the task-code shape
    // `(env) -> i64 result word` and take dispatch slots after the
    // user functions. The dispatch maps are filled here: this is the
    // first point at which the thunk module indices exist.
    //
    // The user functions lower AFTER the thunks (a task call embeds
    // its thunk's module index), but their `funcs` slots come first:
    // reserve the user positions, fill the slots after the thunk
    // bodies are known.
    // Emission mirrors the plan order the maps above were built
    // from (task thunks, then closure thunks): the i-th emitted
    // thunk lands in table slot user_count + i.
    for &thunk_id in &plan.task_thunks {
        let (params, results, extra, body) = lower_task_thunk(&env, thunk_id)?;
        let index = base + funcs.len() as u32;
        thunk_func_indices.push(index);
        funcs.push(Func {
            name: Some(format!("__lpp_task_thunk{}", thunk_id.raw())),
            params,
            results,
            extra,
            body,
        });
    }
    for &thunk_id in &plan.closure_thunks {
        let (params, results, extra, body) = lower_closure_thunk(&env, thunk_id)?;
        let index = base + funcs.len() as u32;
        thunk_func_indices.push(index);
        funcs.push(Func {
            name: Some(format!("__lpp_closure_thunk{}", thunk_id.raw())),
            params,
            results,
            extra,
            body,
        });
    }
    // Now that every dispatch index exists, lower the user functions
    // into their reserved slots.
    for (slot, fn_id) in funcs.iter_mut().zip(user_ids.iter()) {
        let (name, params, results, extra, body) = lower_user_function(&env, *fn_id)?;
        *slot = Func {
            name,
            params,
            results,
            extra,
            body,
        };
    }

    // The funcref table: [no-op, drops in MirAggregateId order, list,
    // closure env dtors, closure destroyer, task dtor, tuple dtor].
    let mut table_fns: Vec<u32> = Vec::new();
    if has_heap {
        if let Some(noop) = noop_drop_index {
            table_fns.push(noop);
        }
        table_fns.extend(drop_func_indices.iter().copied());
        if let Some(list_drop) = list_drop_index {
            table_fns.push(list_drop);
        }
        table_fns.extend(closure_dtor_indices.iter().copied());
        if let Some(destroy) = closure_destroy_index {
            table_fns.push(destroy);
        }
        if let Some(dt) = task_dtor_index {
            table_fns.push(dt);
        }
        if let Some(tt) = tuple_dtor_index {
            table_fns.push(tt);
        }
    }

    let func_tys: Vec<u32> = funcs
        .iter()
        .map(|f| reg_type(&mut type_map, &mut type_list, &f.params, &f.results))
        .collect();

    // ---- Type section (1) ----
    let mut tsec = Vec::new();
    uleb(&mut tsec, type_list.len() as u64);
    for (params, results) in &type_list {
        tsec.push(0x60);
        uleb(&mut tsec, params.len() as u64);
        for p in params {
            tsec.push(p.byte());
        }
        uleb(&mut tsec, results.len() as u64);
        for r in results {
            tsec.push(r.byte());
        }
    }

    // ---- Import section (2) ----
    let mut isec = Vec::new();
    uleb(&mut isec, import_fields.len() as u64);
    for (i, field) in import_fields.iter().enumerate() {
        enc_name(&mut isec, "wasi_snapshot_preview1");
        enc_name(&mut isec, field);
        isec.push(0x00); // func import
        uleb(&mut isec, import_tys[i] as u64);
    }

    // ---- Function section (3) ----
    let mut fsec = Vec::new();
    uleb(&mut fsec, funcs.len() as u64);
    for idx in &func_tys {
        uleb(&mut fsec, *idx as u64);
    }

    // ---- Table section (4) ---- (5D2a destructor table; 5D2b adds the
    // dispatch table as table 1). Bare function values (slice 2) and
    // task thunks (slice 3) also need the dispatch table.
    let has_closures = !plan.closures.is_empty() || plan.has_function_value || plan.has_tasks;
    let dispatch_len = user_ids.len() + thunk_func_indices.len();
    let mut table_sec: Vec<u8> = Vec::new();
    if has_heap && has_closures {
        uleb(&mut table_sec, 2);
        table_sec.push(0x70); // funcref
        table_sec.push(0x00); // limits: min only
        uleb(&mut table_sec, table_fns.len() as u64);
        table_sec.push(0x70);
        table_sec.push(0x00);
        uleb(&mut table_sec, dispatch_len as u64);
    } else if has_heap {
        uleb(&mut table_sec, 1);
        table_sec.push(0x70); // funcref
        table_sec.push(0x00); // limits: min only
        uleb(&mut table_sec, table_fns.len() as u64);
    } else if has_closures {
        uleb(&mut table_sec, 1);
        table_sec.push(0x70); // funcref (dispatch only)
        table_sec.push(0x00);
        uleb(&mut table_sec, dispatch_len as u64);
    } else {
        uleb(&mut table_sec, 0);
    }

    // ---- Memory section (5) ----
    let mut msec = Vec::new();
    uleb(&mut msec, 1);
    msec.push(0x00); // limits: min only
    uleb(&mut msec, min_pages as u64);

    // ---- Global section (6) ---- (the heap bump pointer)
    let mut gsec = Vec::new();
    if has_heap {
        uleb(&mut gsec, 1);
        gsec.push(0x7f); // i32
        gsec.push(0x01); // mutable
        gsec.push(0x41); // i32.const
        sleb(&mut gsec, heap_start as i64);
        gsec.push(0x0b); // end
    } else {
        uleb(&mut gsec, 0);
    }

    // ---- Export section (7) ----
    let mut export_vec: Vec<(String, u32, u8)> = Vec::new(); // (name, index, kind)
    export_vec.push(("_start".to_string(), start_index, 0x00));
    export_vec.push(("memory".to_string(), 0, 0x02));
    for (pos, f) in funcs.iter().enumerate().take(user_count as usize) {
        if let Some(name) = &f.name {
            export_vec.push((name.clone(), base + pos as u32, 0x00));
        }
    }
    let mut esec = Vec::new();
    uleb(&mut esec, export_vec.len() as u64);
    for (name, index, kind) in &export_vec {
        enc_name(&mut esec, name);
        esec.push(*kind);
        uleb(&mut esec, *index as u64);
    }

    // ---- Code section (10) ----
    let mut bodies: Vec<Vec<u8>> = Vec::new();
    for f in &funcs {
        let mut body = Vec::new();
        enc_locals(&mut body, &f.extra);
        body.extend_from_slice(&f.body);
        body.push(op::END);
        bodies.push(body);
    }
    let mut csec = Vec::new();
    uleb(&mut csec, bodies.len() as u64);
    for body in &bodies {
        uleb(&mut csec, body.len() as u64);
        csec.extend_from_slice(body);
    }

    // ---- Element section (9) ---- (destructor table; 5D2b dispatch
    // table in `MirFunctionId` order)
    let mut e9sec = Vec::new();
    if has_heap || has_closures {
        let entries = (has_heap as u64) + (has_closures as u64);
        uleb(&mut e9sec, entries);
        if has_heap {
            e9sec.push(0x00); // active, table 0
            e9sec.push(0x41); // i32.const
            sleb(&mut e9sec, 0);
            e9sec.push(0x0b); // end
            uleb(&mut e9sec, table_fns.len() as u64);
            for idx in &table_fns {
                uleb(&mut e9sec, *idx as u64);
            }
        }
        if has_closures {
            // The dispatch table: table 1 (table 0 when there is no
            // drop table). Element flags: 0x00 = active in table 0,
            // 0x02 = active with an explicit table index.
            if has_heap {
                e9sec.push(0x02); // active, explicit table index
                e9sec.push(0x01); // table 1
            } else {
                e9sec.push(0x00); // active, table 0
            }
            e9sec.push(0x41); // i32.const
            sleb(&mut e9sec, 0);
            e9sec.push(0x0b); // end
            if has_heap {
                // The 0x02 encoding carries an element-kind byte after
                // the offset expression (0x00 = funcref).
                e9sec.push(0x00);
            }
            // User functions first (their MODULE indices occupy the
            // first user_count table positions), then the thunks in
            // emission order (thunk i sits at table position
            // user_count + i — exactly what the dispatch maps store).
            uleb(&mut e9sec, dispatch_len as u64);
            for &fn_id in &user_ids {
                uleb(&mut e9sec, env.user_index[&fn_id] as u64);
            }
            for &idx in &thunk_func_indices {
                uleb(&mut e9sec, idx as u64);
            }
        }
    } else {
        uleb(&mut e9sec, 0);
    }

    // ---- Data section (11) ----
    let mut dsec = Vec::new();
    uleb(&mut dsec, 1);
    dsec.push(0x00); // active, memory 0
    dsec.push(0x41); // i32.const
    sleb(&mut dsec, POOL_START as i64);
    dsec.push(0x0b); // end
    uleb(&mut dsec, pool_bytes.len() as u64);
    dsec.extend_from_slice(&pool_bytes);

    // ---- Name section (custom, 0) ---- (function names for the census;
    // user functions plus the generated destructors)
    let mut named: Vec<(String, u32)> = Vec::new();
    for (pos, f) in funcs.iter().enumerate() {
        if let Some(name) = &f.name {
            named.push((name.clone(), base + pos as u32));
        }
    }
    let mut name_sec: Vec<u8> = Vec::new();
    if !named.is_empty() {
        enc_name(&mut name_sec, "name");
        let mut sub = Vec::new();
        uleb(&mut sub, named.len() as u64);
        for (name, index) in &named {
            enc_name(&mut sub, name);
            uleb(&mut sub, *index as u64);
        }
        name_sec.push(1); // subsection: function names
        uleb(&mut name_sec, sub.len() as u64);
        name_sec.extend_from_slice(&sub);
    }

    // ---- Assemble the module ----
    let mut module = Vec::new();
    module.extend_from_slice(&b"\0asm"[..]);
    uleb(&mut module, 1); // version
    uleb(&mut module, 0); // (reserved low byte)
    uleb(&mut module, 0);
    uleb(&mut module, 0);
    enc_section(&mut module, 1, &tsec);
    enc_section(&mut module, 2, &isec);
    enc_section(&mut module, 3, &fsec);
    enc_section(&mut module, 4, &table_sec);
    enc_section(&mut module, 5, &msec);
    enc_section(&mut module, 6, &gsec);
    enc_section(&mut module, 7, &esec);
    enc_section(&mut module, 9, &e9sec);
    enc_section(&mut module, 10, &csec);
    enc_section(&mut module, 11, &dsec);
    if !name_sec.is_empty() {
        enc_section(&mut module, 0, &name_sec);
    }

    // Symbol sets for the census.
    let mut exported_symbols = BTreeSet::new();
    exported_symbols.insert("_start".to_string());
    exported_symbols.insert("memory".to_string());
    for f in funcs.iter().take(user_count as usize) {
        if let Some(name) = &f.name {
            exported_symbols.insert(name.clone());
        }
    }
    let mut imported_symbols = BTreeSet::new();
    for field in &import_fields {
        imported_symbols.insert(format!("wasi_snapshot_preview1.{field}"));
    }

    if let Ok(path) = _zzz_out {
        std::fs::write(path, &module).unwrap();
    }

    Ok(CompiledModule {
        target: Target::Wasm32Wasi,
        object: module,
        exported_symbols,
        imported_symbols,
        entry: Some("main".to_string()),
    })
}

// ---------------------------------------------------------------------------
// Pre-scan: typed rejections + helper/string collection.
// ---------------------------------------------------------------------------

struct Plan {
    needs: BTreeSet<H>,
    strings: BTreeSet<String>,
    main: MirFunctionId,
    main_void: bool,
    /// Whether any list place/list literal appears (drives the list
    /// helpers, the list destroyer, and the heap).
    has_list: bool,
    /// Whether a source-level tuple value or projection appears. Tuples use
    /// the generic masked ARC tuple node also used for task environments.
    has_tuple: bool,
    /// Closure functions in `MirFunctionId` order (5D2b): drives the
    /// dispatch table, the per-closure env destructors, and
    /// `lpp_closure_destroy`.
    closures: Vec<MirFunctionId>,
    /// Whether any bare function value is used (5D2b slice 2): drives
    /// `lpp_closure_destroy` (function-value capsules reuse it).
    has_function_value: bool,
    /// 5D2b slice 3: async functions whose task thunk is used, in
    /// `MirFunctionId` order (`__lpp_task_thunk{n}`; deduped).
    task_thunks: Vec<MirFunctionId>,
    /// 5D2b slice 3: zero-parameter closures spawned via `spawn`, in
    /// `MirFunctionId` order (`__lpp_closure_thunk{n}`; deduped).
    closure_thunks: Vec<MirFunctionId>,
    /// 5D2b slice 3: a task is constructed somewhere (async call,
    /// async value, spawn, or async main): drives the task runtime
    /// helpers, the tuple/task destructors, and the thunk dispatch
    /// slots.
    has_tasks: bool,
    /// 5D2b slice 3: source-level `main` is async (`_start` drains it
    /// through its task thunk).
    main_async: bool,
    /// 5D2b slice 3: function-value provenance — `(fn, value local) ->
    /// originating function id` (bare values and `MakeClosure`).
    /// A `spawn` target must have a known origin.
    value_origins: BTreeMap<(MirFunctionId, MirLocalId), MirFunctionId>,
}

fn pre_scan(
    program: &MirProgram,
    types: &TypeInterner,
    names: &dyn NameResolver,
    layouts: &BTreeMap<MirAggregateId, AggregateLayout>,
    aggregates: &AggregateIndex,
) -> Result<Plan, CodegenError> {
    let mut needs = BTreeSet::new();
    let mut strings = BTreeSet::new();
    let mut main: Option<MirFunctionId> = None;
    let mut main_void = false;
    let mut has_list = false;
    let mut has_tuple = false;
    let mut has_managed = false;
    let mut has_function_value = false;
    let mut has_tasks = false;
    let mut closures: Vec<MirFunctionId> = Vec::new();
    let mut task_thunks: Vec<MirFunctionId> = Vec::new();
    let mut closure_thunks: Vec<MirFunctionId> = Vec::new();
    let mut value_origins: BTreeMap<(MirFunctionId, MirLocalId), MirFunctionId> = BTreeMap::new();
    let mut main_async = false;

    for (fn_id, function) in program.functions() {
        if matches!(function.kind, lpp_mir::MirFunctionKind::Closure) {
            closures.push(fn_id);
        }
        let is_main = function.name.and_then(|s| names.resolve(s.raw())) == Some("main");
        if is_main {
            main = Some(fn_id);
            main_void = is_void(types, function.return_type);
            // An async entry is drained by `_start` through its task
            // thunk (the wrapper never calls it directly), so the
            // thunk must exist even though no instruction names it.
            main_async = matches!(function.kind, lpp_mir::MirFunctionKind::Async);
            if main_async {
                has_tasks = true;
                task_thunks.push(fn_id);
            }
        }

        let mut lower = ScanLower {
            program,
            types,
            fn_id,
            needs: &mut needs,
            strings: &mut strings,
            has_list: &mut has_list,
            has_tuple: &mut has_tuple,
            has_managed: &mut has_managed,
            has_function_value: &mut has_function_value,
            has_tasks: &mut has_tasks,
            task_thunks: &mut task_thunks,
            closure_thunks: &mut closure_thunks,
            value_origins: &mut value_origins,
            layouts,
            aggregates,
        };
        for &block_id in program.function_blocks(function) {
            let block = program.block(block_id).unwrap();
            for &instr in program.block_instructions(block) {
                let instruction = program.instruction(instr).unwrap();
                check_instruction(&mut lower, &instruction.kind)?;
            }
            check_terminator(&mut lower, &block.terminator)?;
        }
    }

    let main = main.ok_or_else(|| {
        CodegenError::new(
            None,
            CodegenErrorKind::ObjectEmissionFailed(
                "program defines no source-level `main`".to_string(),
            ),
        )
    })?;

    // `fd_write` is pulled in by any printing helper.
    if needs.contains(&H::PrintInt)
        || needs.contains(&H::PrintBool)
        || needs.contains(&H::PrintStr)
        || needs.contains(&H::PrintFloat)
        || needs.contains(&H::WriteStr)
    {
        needs.insert(H::FdWrite);
    }
    // The task runtime (5D2b slice 3) rides the ARC heap: task nodes
    // and env tuples are ARC nodes with tabled destructors.
    if has_tasks {
        has_managed = true;
        needs.insert(H::TaskNew);
        needs.insert(H::TaskPoll);
        needs.insert(H::TaskAwait);
        needs.insert(H::TaskDestroy);
    }
    // The ARC machinery is whole-program: one heap, one table, and the
    // destructor set are shared by every managed value.
    if has_managed || has_list {
        for h in [H::Alloc, H::ArcAlloc, H::Retain, H::Release] {
            needs.insert(h);
        }
    }
    // The float formatter's literals live in the pool (deduped by
    // content, deterministic).
    if needs.contains(&H::PrintFloat) {
        for literal in ["NaN\n", "-inf\n", "inf\n", "-", ".", ".000000\n"] {
            strings.insert(literal.to_string());
        }
    }
    // The string-builtin helpers compose their output from pooled
    // fragments (deduped by content, deterministic).
    if needs.contains(&H::FloatToStr) || needs.contains(&H::BoolToStr) {
        for literal in [
            "true", "false", "nan", "inf", "-inf", "-0", "0", "-", ".", "e", "+",
        ] {
            strings.insert(literal.to_string());
        }
    }

    task_thunks.sort_unstable();
    task_thunks.dedup();
    closure_thunks.sort_unstable();
    closure_thunks.dedup();

    Ok(Plan {
        needs,
        strings,
        main,
        main_void,
        has_list,
        has_tuple,
        closures,
        has_function_value,
        task_thunks,
        closure_thunks,
        has_tasks,
        main_async,
        value_origins,
    })
}

struct ScanLower<'a> {
    program: &'a MirProgram,
    types: &'a TypeInterner,
    fn_id: MirFunctionId,
    needs: &'a mut BTreeSet<H>,
    strings: &'a mut BTreeSet<String>,
    has_list: &'a mut bool,
    has_tuple: &'a mut bool,
    has_managed: &'a mut bool,
    has_function_value: &'a mut bool,
    /// 5D2b slice 3: a task is constructed somewhere in the program.
    has_tasks: &'a mut bool,
    /// 5D2b slice 3: async functions whose task thunk is used.
    task_thunks: &'a mut Vec<MirFunctionId>,
    /// 5D2b slice 3: zero-parameter closures spawned via `spawn`.
    closure_thunks: &'a mut Vec<MirFunctionId>,
    /// 5D2b slice 3: function-value provenance.
    value_origins: &'a mut BTreeMap<(MirFunctionId, MirLocalId), MirFunctionId>,
    layouts: &'a BTreeMap<MirAggregateId, AggregateLayout>,
    aggregates: &'a AggregateIndex,
}

/// Pre-scan operand class with typed, function-attributed handling:
/// a bare function value is a 16-byte capsule on the ARC heap (5D2b
/// slice 2; slice 3 extends it to async functions, whose capsule
/// points at the task thunk with an empty task-env tuple).
fn scan_operand_class(lower: &mut ScanLower, operand: &Operand) -> Result<Class, CodegenError> {
    if matches!(operand, Operand::Function(_)) {
        // A bare function value materializes a 16-byte capsule
        // `[code, env]` on the ARC heap.
        *lower.has_managed = true;
        *lower.has_function_value = true;
        return operand_class(lower.program, lower.types, operand);
    }
    // A task handle is a 32-byte ARC node (5D2b slice 3).
    if matches!(operand, Operand::Copy(local)
    if matches!(
        lower.types.kind(lower.program.local(*local).unwrap().ty),
        TypeKind::Task(_)
    )) {
        *lower.has_managed = true;
        return Ok(Class::Ptr);
    }
    let class = operand_class(lower.program, lower.types, operand)?;
    if class == Class::Str {
        // Pooled strings use an immortal count, while dynamically produced
        // strings use ordinary ARC. Both safely share Retain/Release.
        *lower.has_managed = true;
    }
    Ok(class)
}

fn collect_strings(program: &MirProgram, rvalue: &Rvalue, strings: &mut BTreeSet<String>) {
    for operand in rvalue_operands(program, rvalue) {
        if let Operand::Constant(Constant::String { string, .. }) = operand
            && let Some(content) = program.string(string)
        {
            strings.insert(content.clone());
        }
    }
}

fn rvalue_operands(program: &MirProgram, rvalue: &Rvalue) -> Vec<Operand> {
    let mut out = Vec::new();
    match rvalue {
        Rvalue::Use(o) => out.push(*o),
        Rvalue::Unary { operand, .. } => out.push(*operand),
        Rvalue::Binary { left, right, .. } => {
            out.push(*left);
            out.push(*right);
        }
        Rvalue::Call { callee, arguments } => {
            out.push(*callee);
            out.extend_from_slice(program.operands(*arguments));
        }
        Rvalue::Builtin { arguments, .. } => out.extend_from_slice(program.operands(*arguments)),
        Rvalue::List(items) => out.extend_from_slice(program.operands(*items)),
        Rvalue::Tuple(items) => out.extend_from_slice(program.operands(*items)),
        Rvalue::ConstructStruct { fields, .. } => out.extend_from_slice(program.operands(*fields)),
        Rvalue::ConstructVariant { fields, .. } => out.extend_from_slice(program.operands(*fields)),
        Rvalue::Load(_) | Rvalue::ListLen(_) => {}
        Rvalue::MakeClosure { captures, .. } => out.extend_from_slice(program.operands(*captures)),
        Rvalue::Await(o) | Rvalue::Spawn(o) => out.push(*o),
    }
    out
}

// ---------------------------------------------------------------------------
// Place resolution (5D2a): the shared field-offset / list-index walk used
// by the pre-scan and the lowering, mirroring the 5C Cranelift resolver.
// ---------------------------------------------------------------------------

/// One step of a place projection.
#[derive(Debug, Clone, Copy)]
enum PlaceStep {
    /// An enum downcast: a no-op at the machine level (the dominating
    /// `SwitchEnum` already dispatched on the tag).
    Downcast,
    /// A struct field or a downcast enum's variant field.
    Field {
        offset: u32,
        ty: TypeId,
        managed: bool,
    },
    /// A list element (the index operand resolved at the site).
    ListIndex { element: TypeId, index: Operand },
    /// A boxed tuple slot. Tuple payload words begin after the mask/count
    /// header and each element occupies one 8-byte word.
    TupleField {
        offset: u32,
        ty: TypeId,
        managed: bool,
    },
}

/// The resolved form of a place: the root local plus its steps.
struct PlaceResolution {
    root: MirLocalId,
    steps: Vec<PlaceStep>,
    ty: TypeId,
}

fn resolve_place(
    program: &MirProgram,
    types: &TypeInterner,
    place: lpp_mir::MirPlaceId,
    layouts: &BTreeMap<MirAggregateId, AggregateLayout>,
    aggregates: &AggregateIndex,
    for_store: bool,
    fn_id: MirFunctionId,
) -> Result<PlaceResolution, CodegenError> {
    let descriptor = program.place(place).unwrap();
    let projections: Vec<PlaceProjection> = program.place_projections(descriptor).to_vec();
    if for_store && matches!(projections.last(), Some(PlaceProjection::Downcast(_))) {
        return Err(unsupported("store through downcast", Some(fn_id)));
    }

    let mut current_ty = program.local(descriptor.root).unwrap().ty;
    let mut steps = Vec::new();
    // The enum variant made active by the most recent downcast
    // (aggregate, variant ordinal).
    let mut active_enum: Option<(MirAggregateId, u32)> = None;

    for (position, projection) in projections.iter().enumerate() {
        let is_last = position + 1 == projections.len();
        match projection {
            PlaceProjection::Downcast(variant) => {
                let TypeKind::Nominal { .. } = types.kind(current_ty) else {
                    return Err(unsupported("downcast of non-nominal", Some(fn_id)));
                };
                let aggregate = aggregates
                    .aggregate_for(current_ty)
                    .ok_or_else(|| unsupported("unmapped nominal type", Some(fn_id)))?;
                let agg = program.aggregate(aggregate).unwrap();
                if agg.kind != lpp_mir::MirAggregateKind::Enum {
                    return Err(unsupported("downcast of struct", Some(fn_id)));
                }
                let variant = program.variant(*variant).unwrap();
                if variant.aggregate != aggregate {
                    return Err(unsupported("downcast aggregate mismatch", Some(fn_id)));
                }
                active_enum = Some((aggregate, variant.ordinal));
                steps.push(PlaceStep::Downcast);
            }
            PlaceProjection::Field(field) => {
                let TypeKind::Nominal { .. } = types.kind(current_ty) else {
                    return Err(unsupported("field of non-nominal", Some(fn_id)));
                };
                let aggregate = aggregates
                    .aggregate_for(current_ty)
                    .ok_or_else(|| unsupported("unmapped nominal type", Some(fn_id)))?;
                let layout = layouts.get(&aggregate).unwrap();
                let slots = match layout.kind {
                    lpp_mir::MirAggregateKind::Struct => &layout.struct_fields,
                    lpp_mir::MirAggregateKind::Enum => {
                        let (enum_aggregate, ordinal) = active_enum.take().ok_or_else(|| {
                            unsupported("enum field without downcast", Some(fn_id))
                        })?;
                        if enum_aggregate != aggregate {
                            return Err(unsupported("downcast aggregate mismatch", Some(fn_id)));
                        }
                        &layout.variant_fields[ordinal as usize]
                    }
                };
                let slot = slots
                    .iter()
                    .find(|slot| slot.field == *field)
                    .ok_or_else(|| unsupported("unknown field offset", Some(fn_id)))?;
                let managed = is_managed(types, slot.ty);
                if !is_last && !managed {
                    return Err(unsupported("projection through scalar field", Some(fn_id)));
                }
                steps.push(PlaceStep::Field {
                    offset: slot.offset,
                    ty: slot.ty,
                    managed,
                });
                current_ty = slot.ty;
            }
            PlaceProjection::ListIndex(index) => {
                let TypeKind::List(element) = types.kind(current_ty) else {
                    return Err(unsupported("index of non-list", Some(fn_id)));
                };
                match index {
                    Operand::Copy(local) => {
                        let ty = program.local(*local).unwrap().ty;
                        if types.kind(ty) != TypeKind::Primitive(PrimitiveType::Int) {
                            return Err(unsupported("non-int list index", Some(fn_id)));
                        }
                    }
                    Operand::Constant(Constant::Integer(_)) => {}
                    _ => return Err(unsupported("non-int list index", Some(fn_id))),
                }
                if !is_last && !is_managed(types, element) {
                    return Err(unsupported(
                        "projection through scalar list element",
                        Some(fn_id),
                    ));
                }
                steps.push(PlaceStep::ListIndex {
                    element,
                    index: *index,
                });
                current_ty = element;
            }
            PlaceProjection::TupleField(index) => {
                let TypeKind::Tuple(elements) = types.kind(current_ty) else {
                    return Err(unsupported("tuple field of non-tuple", Some(fn_id)));
                };
                let element_types = types.list(elements);
                let ty = *element_types
                    .get(*index as usize)
                    .ok_or_else(|| unsupported("tuple field index", Some(fn_id)))?;
                let managed = is_managed(types, ty);
                if !is_last && !managed {
                    return Err(unsupported(
                        "projection through scalar tuple element",
                        Some(fn_id),
                    ));
                }
                steps.push(PlaceStep::TupleField {
                    offset: 16 + 8 * *index,
                    ty,
                    managed,
                });
                current_ty = ty;
            }
        }
    }

    if current_ty != descriptor.ty {
        return Err(unsupported("place type mismatch", Some(fn_id)));
    }
    Ok(PlaceResolution {
        root: descriptor.root,
        steps,
        ty: current_ty,
    })
}

fn check_instruction(
    lower: &mut ScanLower<'_>,
    kind: &InstructionKind,
) -> Result<(), CodegenError> {
    match kind {
        InstructionKind::Assign { target, value } => {
            collect_strings(lower.program, value, lower.strings);
            check_rvalue(lower, value)?;
            // Function-value provenance (5D2b slice 3): a value
            // established here is the only one a `spawn` in this
            // function may target.
            match value {
                Rvalue::Use(Operand::Function(id)) => {
                    lower.value_origins.insert((lower.fn_id, *target), *id);
                }
                Rvalue::MakeClosure { function, .. } => {
                    lower
                        .value_origins
                        .insert((lower.fn_id, *target), *function);
                }
                Rvalue::Use(Operand::Copy(src)) => {
                    if let Some(origin) = lower.value_origins.get(&(lower.fn_id, *src)) {
                        lower.value_origins.insert((lower.fn_id, *target), *origin);
                    }
                }
                _ => {}
            }
            // A list literal's element class comes from the target.
            if let Rvalue::List(_) = value {
                let TypeKind::List(element) =
                    lower.types.kind(lower.program.local(*target).unwrap().ty)
                else {
                    return Err(unsupported(
                        "list literal to non-list target",
                        Some(lower.fn_id),
                    ));
                };
                check_list_literal(lower, element)?;
            }
            if let Rvalue::Tuple(items) = value {
                let target_ty = lower.program.local(*target).unwrap().ty;
                let TypeKind::Tuple(elements) = lower.types.kind(target_ty) else {
                    return Err(unsupported(
                        "tuple literal to non-tuple target",
                        Some(lower.fn_id),
                    ));
                };
                let element_types = lower.types.list(elements);
                let values = lower.program.operands(*items);
                if values.len() != element_types.len() || values.len() > 64 {
                    return Err(unsupported("tuple arity", Some(lower.fn_id)));
                }
                *lower.has_tuple = true;
                *lower.has_managed = true;
                for value in values {
                    scan_operand_class(lower, value)?;
                }
            }
            Ok(())
        }
        InstructionKind::Store { place, value } => {
            let resolution = resolve_place(
                lower.program,
                lower.types,
                *place,
                lower.layouts,
                lower.aggregates,
                true,
                lower.fn_id,
            )?;
            register_list_load_needs(lower, &resolution);
            if let Some(PlaceStep::ListIndex { element, .. }) = resolution.steps.last() {
                match element_class(lower.types, *element) {
                    ElementClass::I64 => lower.needs.insert(H::ListSetI64),
                    ElementClass::F64 => lower.needs.insert(H::ListSetF64),
                    ElementClass::Ptr => lower.needs.insert(H::ListSetPtr),
                };
            }
            if let Operand::Constant(Constant::String { string, .. }) = value
                && let Some(content) = lower.program.string(*string)
            {
                lower.strings.insert(content.clone());
            }
            scan_operand_class(lower, value).map(|_| ())
        }
    }
}

fn check_rvalue(lower: &mut ScanLower<'_>, rvalue: &Rvalue) -> Result<(), CodegenError> {
    match rvalue {
        Rvalue::Use(operand) => {
            // A bare function value materializes a capsule (5D2b
            // slice 2); tasks stay rejected by `scan_operand_class`.
            scan_operand_class(lower, operand).map(|_| ())
        }
        Rvalue::Unary { operator, operand } => {
            let class = scan_operand_class(lower, operand)?;
            if class == Class::Ptr {
                return Err(unsupported(
                    "unary operator on managed type",
                    Some(lower.fn_id),
                ));
            }
            match (operator, class) {
                (UnaryOperator::Not, Class::Bool | Class::I64) => {}
                (UnaryOperator::Negate, Class::I64 | Class::F64) => {}
                _ => {
                    return Err(unsupported(
                        "unary operator on this operand type",
                        Some(lower.fn_id),
                    ));
                }
            }
            Ok(())
        }
        Rvalue::Binary {
            left,
            operator,
            right,
        } => {
            let lc = scan_operand_class(lower, left)?;
            let rc = scan_operand_class(lower, right)?;
            if lc != rc {
                return Err(unsupported(
                    "binary operand type mismatch",
                    Some(lower.fn_id),
                ));
            }
            if lc == Class::Ptr {
                return Err(unsupported(
                    "binary operator on managed type",
                    Some(lower.fn_id),
                ));
            }
            match (operator, lc) {
                (BinaryOperator::Equal, Class::Str) | (BinaryOperator::NotEqual, Class::Str) => {
                    lower.needs.insert(H::StrEq);
                }
                (BinaryOperator::Add, Class::Str) => {
                    *lower.has_managed = true;
                    lower.needs.insert(H::Alloc);
                    lower.needs.insert(H::StrConcat);
                }
                (BinaryOperator::Subtract, Class::Str)
                | (BinaryOperator::Multiply, Class::Str)
                | (BinaryOperator::Divide, Class::Str)
                | (BinaryOperator::Modulo, Class::Str) => {
                    return Err(unsupported("string arithmetic", Some(lower.fn_id)));
                }
                _ => {}
            }
            // The remaining operator/type combinations are lowered to native
            // wasm ops (verified in the lowering).
            let _ = rc;
            Ok(())
        }
        Rvalue::Call { callee, arguments } => {
            match callee {
                Operand::Function(f) => {
                    // A direct async call constructs a task through
                    // the function's thunk (5D2b slice 3).
                    if matches!(
                        lower.program.function(*f).unwrap().kind,
                        lpp_mir::MirFunctionKind::Async
                    ) {
                        *lower.has_tasks = true;
                        lower.task_thunks.push(*f);
                        lower.needs.insert(H::TaskNew);
                    }
                }
                Operand::Copy(local) => {
                    // A capsule call: the callee local holds the
                    // 16-byte [code, env] node. A value whose result
                    // is a Task constructs a task (5D2b slice 3).
                    let ty = lower.program.local(*local).unwrap().ty;
                    let TypeKind::Function { result, .. } = lower.types.kind(ty) else {
                        return Err(unsupported("indirect call", Some(lower.fn_id)));
                    };
                    if matches!(lower.types.kind(result), TypeKind::Task(_)) {
                        *lower.has_tasks = true;
                        lower.needs.insert(H::TaskNew);
                    }
                }
                _ => {
                    return Err(unsupported("indirect call", Some(lower.fn_id)));
                }
            }
            for operand in lower.program.operands(*arguments) {
                scan_operand_class(lower, operand)?;
            }
            Ok(())
        }
        Rvalue::Builtin { builtin, arguments } => {
            let args = lower.program.operands(*arguments);
            check_builtin(lower, *builtin, args)
        }
        Rvalue::Load(place) => {
            let resolution = resolve_place(
                lower.program,
                lower.types,
                *place,
                lower.layouts,
                lower.aggregates,
                false,
                lower.fn_id,
            )?;
            register_list_load_needs(lower, &resolution);
            scan_operand_class(lower, &Operand::Copy(resolution.root)).map(|_| ())
        }
        // List literals are checked from their assignment context (the
        // target/place type supplies the element class of `[]`).
        Rvalue::List(_) => Ok(()),
        // Tuple literals are validated from their assignment context because
        // the target local supplies the element types.
        Rvalue::Tuple(_) => Ok(()),
        Rvalue::ConstructStruct { fields, .. } => {
            *lower.has_managed = true;
            for operand in lower.program.operands(*fields) {
                scan_operand_class(lower, operand)?;
            }
            Ok(())
        }
        Rvalue::ConstructVariant { fields, .. } => {
            *lower.has_managed = true;
            for operand in lower.program.operands(*fields) {
                scan_operand_class(lower, operand)?;
            }
            Ok(())
        }
        Rvalue::ListLen(operand) => {
            if scan_operand_class(lower, operand)? != Class::Ptr {
                return Err(unsupported("list length of non-list", Some(lower.fn_id)));
            }
            *lower.has_list = true;
            *lower.has_managed = true;
            lower.needs.insert(H::ListLen);
            Ok(())
        }
        Rvalue::MakeClosure { captures, .. } => {
            for operand in lower.program.operands(*captures) {
                if !matches!(operand, Operand::Copy(_)) {
                    return Err(unsupported("non-local capture", Some(lower.fn_id)));
                }
            }
            // The capsule and env are ARC nodes.
            *lower.has_managed = true;
            Ok(())
        }
        Rvalue::Await(operand) => {
            // 5D2b slice 3: `x.await` on a task local.
            let Operand::Copy(local) = operand else {
                return Err(unsupported("await of non-local task", Some(lower.fn_id)));
            };
            let ty = lower.program.local(*local).unwrap().ty;
            if !matches!(lower.types.kind(ty), TypeKind::Task(_)) {
                return Err(unsupported("await of non-task", Some(lower.fn_id)));
            }
            *lower.has_tasks = true;
            lower.needs.insert(H::TaskAwait);
            Ok(())
        }
        Rvalue::Spawn(operand) => {
            // 5D2b slice 3: eager spawn of a zero-parameter,
            // void-result value (closure or bare function).
            let Operand::Copy(local) = operand else {
                return Err(unsupported("spawn of non-local value", Some(lower.fn_id)));
            };
            let ty = lower.program.local(*local).unwrap().ty;
            let TypeKind::Function { parameters, result } = lower.types.kind(ty) else {
                return Err(unsupported("spawn of non-function", Some(lower.fn_id)));
            };
            let void = lower.types.primitive(PrimitiveType::Void);
            if !lower.types.list(parameters).is_empty() || result != void {
                return Err(unsupported(
                    "spawn of a non-void or parameterized value",
                    Some(lower.fn_id),
                ));
            }
            let origin = lower
                .value_origins
                .get(&(lower.fn_id, *local))
                .copied()
                .ok_or_else(|| {
                    unsupported(
                        "spawn of a function value of unknown origin",
                        Some(lower.fn_id),
                    )
                })?;
            let origin_fn = lower.program.function(origin).unwrap();
            let user_params = lower
                .program
                .function_parameters(origin_fn)
                .iter()
                .filter(|&p| {
                    !matches!(
                        lower.program.local(*p).unwrap().kind,
                        lpp_mir::MirLocalKind::Capture
                    )
                })
                .count();
            if user_params != 0 {
                return Err(unsupported(
                    "spawn of a parameterized value",
                    Some(lower.fn_id),
                ));
            }
            match origin_fn.kind {
                lpp_mir::MirFunctionKind::Closure => lower.closure_thunks.push(origin),
                lpp_mir::MirFunctionKind::Function => lower.task_thunks.push(origin),
                lpp_mir::MirFunctionKind::Async => {
                    return Err(unsupported("spawn of an async value", Some(lower.fn_id)));
                }
            }
            *lower.has_tasks = true;
            lower.needs.insert(H::TaskNew);
            lower.needs.insert(H::TaskPoll);
            lower.needs.insert(H::TaskDestroy);
            Ok(())
        }
    }
}

/// Register the `ListGet` needs for every list-index step of a place
/// (intermediate steps borrow through the same plain element read).
fn register_list_load_needs(lower: &mut ScanLower<'_>, resolution: &PlaceResolution) {
    for step in &resolution.steps {
        match step {
            PlaceStep::ListIndex { element, .. } => {
                *lower.has_list = true;
                *lower.has_managed = true;
                match element_class(lower.types, *element) {
                    ElementClass::I64 => lower.needs.insert(H::ListGetI64),
                    ElementClass::F64 => lower.needs.insert(H::ListGetF64),
                    ElementClass::Ptr => lower.needs.insert(H::ListGetPtr),
                };
            }
            PlaceStep::TupleField { .. } => {
                *lower.has_tuple = true;
                *lower.has_managed = true;
            }
            PlaceStep::Downcast | PlaceStep::Field { .. } => {}
        }
    }
}

/// Register the list-helper needs for a list literal of element type
/// `element` (the literal only needs `ListNew` + the element's `Push`;
/// reads/writes/lengths register their own helpers at the use sites).
fn check_list_literal(lower: &mut ScanLower<'_>, element: TypeId) -> Result<(), CodegenError> {
    *lower.has_list = true;
    *lower.has_managed = true;
    lower.needs.insert(H::ListNew);
    match element_class(lower.types, element) {
        ElementClass::I64 => lower.needs.insert(H::ListPushI64),
        ElementClass::F64 => lower.needs.insert(H::ListPushF64),
        ElementClass::Ptr => lower.needs.insert(H::ListPushPtr),
    };
    Ok(())
}

fn check_builtin(
    lower: &mut ScanLower<'_>,
    builtin: BuiltinId,
    args: &[Operand],
) -> Result<(), CodegenError> {
    let name = builtin.descriptor().name;
    let short = name.strip_prefix("lpp_").unwrap_or(name);

    match short {
        // Argument-free.
        "list_new" => {
            *lower.has_list = true;
            *lower.has_managed = true;
            lower.needs.insert(H::ListNew);
            Ok(())
        }
        "map_new" | "map_new_arc" => {
            lower.needs.insert(H::Alloc);
            lower.needs.insert(H::MapNew);
            Ok(())
        }
        "input" => {
            *lower.has_managed = true;
            lower.needs.insert(H::FdRead);
            lower.needs.insert(H::Input);
            Ok(())
        }
        _ => {
            let arg0 = args
                .first()
                .ok_or_else(|| unsupported("builtin with no argument", Some(lower.fn_id)))?;
            match short {
                "print" => {
                    let class = scan_operand_class(lower, arg0)?;
                    match class {
                        Class::I64 | Class::Char => lower.needs.insert(H::PrintInt),
                        Class::Bool => lower.needs.insert(H::PrintBool),
                        Class::Str => lower.needs.insert(H::PrintStr),
                        Class::F64 => lower.needs.insert(H::PrintFloat),
                        Class::Ptr => {
                            return Err(unsupported("print of managed type", Some(lower.fn_id)));
                        }
                    };
                    Ok(())
                }
                "print_str" | "eprint_str" => {
                    require_str(lower, arg0)?;
                    lower.needs.insert(H::PrintStr);
                    Ok(())
                }
                "print_int" => {
                    require_i64(lower, arg0)?;
                    lower.needs.insert(H::PrintInt);
                    Ok(())
                }
                "print_bool" => {
                    require_bool(lower, arg0)?;
                    lower.needs.insert(H::PrintBool);
                    Ok(())
                }
                "print_float" => {
                    require_f64(lower, arg0)?;
                    lower.needs.insert(H::PrintFloat);
                    Ok(())
                }
                "write_str" => {
                    require_str(lower, arg0)?;
                    lower.needs.insert(H::WriteStr);
                    Ok(())
                }
                "str_len" => {
                    require_str(lower, arg0)?;
                    lower.needs.insert(H::StrLen);
                    Ok(())
                }
                "list_push" | "list_set" => {
                    let element = list_element_of(lower, arg0)?;
                    *lower.has_list = true;
                    *lower.has_managed = true;
                    // list_push: (list, value); list_set: (list, index, value).
                    for arg in args.iter().skip(1) {
                        scan_operand_class(lower, arg)?;
                    }
                    let push = short == "list_push";
                    match element_class(lower.types, element) {
                        ElementClass::I64 => {
                            lower
                                .needs
                                .insert(if push { H::ListPushI64 } else { H::ListSetI64 })
                        }
                        ElementClass::F64 => {
                            lower
                                .needs
                                .insert(if push { H::ListPushF64 } else { H::ListSetF64 })
                        }
                        ElementClass::Ptr => {
                            lower
                                .needs
                                .insert(if push { H::ListPushPtr } else { H::ListSetPtr })
                        }
                    };
                    Ok(())
                }
                "list_get" => {
                    let element = list_element_of(lower, arg0)?;
                    *lower.has_list = true;
                    *lower.has_managed = true;
                    require_i64(
                        lower,
                        args.get(1).ok_or_else(|| {
                            unsupported("list index argument missing", Some(lower.fn_id))
                        })?,
                    )?;
                    match element_class(lower.types, element) {
                        ElementClass::I64 => lower.needs.insert(H::ListGetI64),
                        ElementClass::F64 => lower.needs.insert(H::ListGetF64),
                        ElementClass::Ptr => lower.needs.insert(H::ListGetPtr),
                    };
                    Ok(())
                }
                "list_len" => {
                    list_element_of(lower, arg0)?;
                    *lower.has_list = true;
                    *lower.has_managed = true;
                    lower.needs.insert(H::ListLen);
                    Ok(())
                }
                "slice" | "str_slice" => {
                    let start = args.get(1).ok_or_else(|| {
                        unsupported("slice start argument missing", Some(lower.fn_id))
                    })?;
                    let length = args.get(2).ok_or_else(|| {
                        unsupported("slice length argument missing", Some(lower.fn_id))
                    })?;
                    require_i64(lower, start)?;
                    require_i64(lower, length)?;
                    lower.needs.insert(H::Alloc);
                    if short == "slice" {
                        list_element_of(lower, arg0)?;
                        *lower.has_list = true;
                        *lower.has_managed = true;
                        lower.needs.insert(H::ListLen);
                        lower.needs.insert(H::SliceNewList);
                    } else {
                        require_str(lower, arg0)?;
                        lower.needs.insert(H::StrLen);
                        lower.needs.insert(H::SliceNewStr);
                    }
                    Ok(())
                }
                "slice_len" => {
                    slice_kind_of(lower.program, lower.types, arg0, lower.fn_id)?;
                    lower.needs.insert(H::SliceLen);
                    Ok(())
                }
                "slice_get" => {
                    require_i64(
                        lower,
                        args.get(1).ok_or_else(|| {
                            unsupported("slice index argument missing", Some(lower.fn_id))
                        })?,
                    )?;
                    match slice_kind_of(lower.program, lower.types, arg0, lower.fn_id)? {
                        SliceKind::String => {
                            *lower.has_managed = true;
                            lower.needs.insert(H::Alloc);
                            lower.needs.insert(H::StrSliceGet);
                        }
                        SliceKind::List(element) => {
                            *lower.has_list = true;
                            *lower.has_managed = true;
                            match element_class(lower.types, element) {
                                ElementClass::I64 => lower.needs.insert(H::SliceGetI64),
                                ElementClass::F64 => lower.needs.insert(H::SliceGetF64),
                                ElementClass::Ptr => lower.needs.insert(H::SliceGetPtr),
                            };
                            match element_class(lower.types, element) {
                                ElementClass::I64 => lower.needs.insert(H::ListGetI64),
                                ElementClass::F64 => lower.needs.insert(H::ListGetF64),
                                ElementClass::Ptr => lower.needs.insert(H::ListGetPtr),
                            };
                        }
                    }
                    Ok(())
                }
                "slice_get_bool" => {
                    let index = args.get(1).ok_or_else(|| {
                        unsupported("slice index argument missing", Some(lower.fn_id))
                    })?;
                    require_i64(lower, index)?;
                    let SliceKind::List(element) =
                        slice_kind_of(lower.program, lower.types, arg0, lower.fn_id)?
                    else {
                        return Err(unsupported(
                            "slice_get_bool requires a list slice",
                            Some(lower.fn_id),
                        ));
                    };
                    if !matches!(
                        lower.types.kind(element),
                        TypeKind::Primitive(PrimitiveType::Bool)
                    ) {
                        return Err(unsupported(
                            "slice_get_bool requires boolean elements",
                            Some(lower.fn_id),
                        ));
                    }
                    *lower.has_list = true;
                    *lower.has_managed = true;
                    lower.needs.insert(H::ListGetI64);
                    lower.needs.insert(H::SliceGetI64);
                    Ok(())
                }
                "slice_to_str" | "str_slice_to_str" => {
                    if slice_kind_of(lower.program, lower.types, arg0, lower.fn_id)?
                        != SliceKind::String
                    {
                        return Err(unsupported(
                            "slice_to_str requires a string slice",
                            Some(lower.fn_id),
                        ));
                    }
                    *lower.has_managed = true;
                    lower.needs.insert(H::Alloc);
                    lower.needs.insert(H::StrSliceToStr);
                    Ok(())
                }
                "map_put" | "map_put_str" | "map_put_float" | "map_put_str_float" | "map_get"
                | "map_get_str" | "map_get_float" | "map_get_str_float" | "map_has"
                | "map_has_str" | "map_remove" | "map_remove_str" => {
                    require_i64(lower, arg0)?;
                    let string_keys = short.contains("_str");
                    let float_value = short.ends_with("_float");
                    let is_put = short.starts_with("map_put");
                    let is_get = short.starts_with("map_get");
                    let is_has = short.starts_with("map_has");
                    let key = args.get(1).ok_or_else(|| {
                        unsupported("map key argument missing", Some(lower.fn_id))
                    })?;
                    let expected_key_class = if string_keys { Class::Str } else { Class::I64 };
                    if scan_operand_class(lower, key)? != expected_key_class {
                        return Err(unsupported("map key type mismatch", Some(lower.fn_id)));
                    }
                    if is_put {
                        let value = args.get(2).ok_or_else(|| {
                            unsupported("map value argument missing", Some(lower.fn_id))
                        })?;
                        if float_value {
                            require_f64(lower, value)?;
                        } else {
                            require_i64(lower, value)?;
                        }
                    }
                    let find = if string_keys {
                        lower.needs.insert(H::StrEq);
                        H::MapFindStr
                    } else {
                        H::MapFindI64
                    };
                    lower.needs.insert(find);
                    lower
                        .needs
                        .insert(match (is_put, is_get, is_has, string_keys) {
                            (true, _, _, false) => H::MapPutI64,
                            (true, _, _, true) => H::MapPutStr,
                            (_, true, _, false) => H::MapGetI64,
                            (_, true, _, true) => H::MapGetStr,
                            (_, _, true, false) => H::MapHasI64,
                            (_, _, true, true) => H::MapHasStr,
                            (_, _, _, false) => H::MapRemoveI64,
                            (_, _, _, true) => H::MapRemoveStr,
                        });
                    if is_put {
                        lower.needs.insert(H::MapEnsure);
                        lower.needs.insert(H::Alloc);
                    }
                    Ok(())
                }
                "map_len" => {
                    require_i64(lower, arg0)?;
                    lower.needs.insert(H::MapLen);
                    Ok(())
                }
                // ── 5D2b slice 4, batch 1: integer + float builtins ──
                "abs" | "clz64" | "ctz64" | "popcount64" | "bswap16" | "bswap32" | "bswap64"
                | "trunc_u8" | "trunc_u16" | "trunc_u32" | "trunc_i8" | "trunc_i16"
                | "trunc_i32" => {
                    require_i64(lower, arg0)?;
                    Ok(())
                }
                "min" | "max" | "min_u" | "max_u" | "lt_u" | "le_u" | "gt_u" | "ge_u" | "div_u"
                | "rem_u" | "shr_u" | "shl_u" | "rotl64" | "rotr64" | "rotl32" | "rotr32"
                | "add_wrap" | "sub_wrap" | "mul_wrap" | "add_checked" | "sub_checked"
                | "mul_checked" => {
                    let arg1 = args.get(1).ok_or_else(|| {
                        unsupported("builtin argument missing", Some(lower.fn_id))
                    })?;
                    require_i64(lower, arg0)?;
                    require_i64(lower, arg1)?;
                    Ok(())
                }
                "int_pow" => {
                    let arg1 = args.get(1).ok_or_else(|| {
                        unsupported("builtin argument missing", Some(lower.fn_id))
                    })?;
                    require_i64(lower, arg0)?;
                    require_i64(lower, arg1)?;
                    Ok(())
                }
                "ceil" | "floor" | "sqrt" | "sin" | "cos" => {
                    require_f64(lower, arg0)?;
                    if short == "sin" {
                        lower.needs.insert(H::Sin);
                    } else if short == "cos" {
                        lower.needs.insert(H::Cos);
                    }
                    Ok(())
                }
                "pow" => {
                    let arg1 = args.get(1).ok_or_else(|| {
                        unsupported("builtin argument missing", Some(lower.fn_id))
                    })?;
                    require_f64(lower, arg0)?;
                    require_f64(lower, arg1)?;
                    lower.needs.insert(H::Log2);
                    lower.needs.insert(H::Exp2);
                    lower.needs.insert(H::Pow);
                    Ok(())
                }
                "fmod" => {
                    let arg1 = args.get(1).ok_or_else(|| {
                        unsupported("builtin argument missing", Some(lower.fn_id))
                    })?;
                    require_f64(lower, arg0)?;
                    require_f64(lower, arg1)?;
                    Ok(())
                }
                // ── string construction, indexing, and splitting ──
                "str_substr" => {
                    require_str(lower, arg0)?;
                    require_i64(
                        lower,
                        args.get(1).ok_or_else(|| {
                            unsupported("builtin argument missing", Some(lower.fn_id))
                        })?,
                    )?;
                    require_i64(
                        lower,
                        args.get(2).ok_or_else(|| {
                            unsupported("builtin argument missing", Some(lower.fn_id))
                        })?,
                    )?;
                    *lower.has_managed = true;
                    lower.needs.insert(H::StrSubstr);
                    Ok(())
                }
                "str_repeat" | "char_at" => {
                    require_str(lower, arg0)?;
                    require_i64(
                        lower,
                        args.get(1).ok_or_else(|| {
                            unsupported("builtin argument missing", Some(lower.fn_id))
                        })?,
                    )?;
                    *lower.has_managed = true;
                    lower.needs.insert(if short == "str_repeat" {
                        H::StrRepeat
                    } else {
                        H::CharAt
                    });
                    Ok(())
                }
                "str_split" => {
                    require_str(lower, arg0)?;
                    require_i64(
                        lower,
                        args.get(1).ok_or_else(|| {
                            unsupported("builtin argument missing", Some(lower.fn_id))
                        })?,
                    )?;
                    *lower.has_list = true;
                    *lower.has_managed = true;
                    lower.needs.insert(H::StrSplit);
                    lower.needs.insert(H::ListNew);
                    lower.needs.insert(H::ListPushPtr);
                    Ok(())
                }
                "ord" => {
                    require_str(lower, arg0)?;
                    lower.needs.insert(H::Ord);
                    Ok(())
                }
                "chr" => {
                    require_i64(lower, arg0)?;
                    *lower.has_managed = true;
                    lower.needs.insert(H::Chr);
                    Ok(())
                }
                "str_eq" => {
                    require_str(lower, arg0)?;
                    require_str(
                        lower,
                        args.get(1).ok_or_else(|| {
                            unsupported("builtin argument missing", Some(lower.fn_id))
                        })?,
                    )?;
                    lower.needs.insert(H::StrEq);
                    Ok(())
                }
                // ── 5D2b slice 4, batch 3: string builtins ──
                "str_concat" | "str_replace" | "str_trim" | "str_to_lower" | "str_lower"
                | "str_to_upper" | "str_upper" | "int_to_str" | "str_to_int" | "float_to_str"
                | "bool_to_str" | "u64_to_str" | "u64_to_hex" | "str_to_u64" | "str_contains"
                | "str_starts_with" | "str_ends_with" | "str_find" => {
                    match short {
                        "str_concat" => {
                            let arg1 = args.get(1).ok_or_else(|| {
                                unsupported("builtin argument missing", Some(lower.fn_id))
                            })?;
                            require_str(lower, arg0)?;
                            require_str(lower, arg1)?;
                        }
                        "str_replace" => {
                            let arg1 = args.get(1).ok_or_else(|| {
                                unsupported("builtin argument missing", Some(lower.fn_id))
                            })?;
                            let a2 = args.get(2).ok_or_else(|| {
                                unsupported("builtin argument missing", Some(lower.fn_id))
                            })?;
                            require_str(lower, arg0)?;
                            require_str(lower, arg1)?;
                            require_str(lower, a2)?;
                        }
                        "str_contains" | "str_starts_with" | "str_ends_with" | "str_find" => {
                            let arg1 = args.get(1).ok_or_else(|| {
                                unsupported("builtin argument missing", Some(lower.fn_id))
                            })?;
                            require_str(lower, arg0)?;
                            require_str(lower, arg1)?;
                        }
                        "str_to_int" | "str_to_u64" | "str_trim" | "str_to_lower" | "str_lower"
                        | "str_to_upper" | "str_upper" | "int_to_str" | "float_to_str"
                        | "bool_to_str" | "u64_to_str" | "u64_to_hex" => match short {
                            "float_to_str" => require_f64(lower, arg0)?,
                            "bool_to_str" => require_bool(lower, arg0)?,
                            "int_to_str" | "u64_to_str" | "u64_to_hex" => require_i64(lower, arg0)?,
                            _ => require_str(lower, arg0)?,
                        },
                        _ => unreachable!("string builtin guarded by the outer match"),
                    }
                    // Every string-producing builtin allocates on the ARC
                    // bump heap, which needs the managed machinery.
                    if !matches!(
                        short,
                        "str_contains"
                            | "str_starts_with"
                            | "str_ends_with"
                            | "str_find"
                            | "str_to_int"
                            | "str_to_u64"
                    ) {
                        *lower.has_managed = true;
                        lower.needs.insert(H::Alloc);
                    }
                    match short {
                        "str_concat" => {
                            lower.needs.insert(H::StrConcat);
                        }
                        "str_replace" => {
                            lower.needs.insert(H::StrReplace);
                        }
                        "str_trim" => {
                            lower.needs.insert(H::StrTrim);
                        }
                        "str_to_lower" | "str_lower" => {
                            lower.needs.insert(H::StrLower);
                        }
                        "str_to_upper" | "str_upper" => {
                            lower.needs.insert(H::StrUpper);
                        }
                        "int_to_str" => {
                            lower.needs.insert(H::IntToStr);
                        }
                        "str_to_int" => {
                            lower.needs.insert(H::StrToInt);
                        }
                        "float_to_str" => {
                            lower.needs.insert(H::FloatToStr);
                        }
                        "bool_to_str" => {
                            lower.needs.insert(H::BoolToStr);
                        }
                        "u64_to_str" => {
                            lower.needs.insert(H::U64ToStr);
                        }
                        "u64_to_hex" => {
                            lower.needs.insert(H::U64ToHex);
                        }
                        "str_to_u64" => {
                            lower.needs.insert(H::StrToU64);
                        }
                        "str_contains" => {
                            lower.needs.insert(H::StrContains);
                        }
                        "str_starts_with" => {
                            lower.needs.insert(H::StrStartsWith);
                        }
                        "str_ends_with" => {
                            lower.needs.insert(H::StrEndsWith);
                        }
                        "str_find" => {
                            lower.needs.insert(H::StrFind);
                        }
                        _ => {}
                    };
                    Ok(())
                }
                _ => Err(unrepresentable(
                    builtin,
                    "not representable on wasm32-wasip1",
                    lower.fn_id,
                )),
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SliceKind {
    List(TypeId),
    String,
}

fn slice_kind_of(
    program: &MirProgram,
    types: &TypeInterner,
    operand: &Operand,
    function: MirFunctionId,
) -> Result<SliceKind, CodegenError> {
    let Operand::Copy(local) = operand else {
        return Err(unsupported(
            "slice argument must be a local",
            Some(function),
        ));
    };
    let ty = program
        .local(*local)
        .ok_or_else(|| unsupported("unknown slice local", Some(function)))?
        .ty;
    match types.kind(ty) {
        TypeKind::Slice(element) => Ok(SliceKind::List(element)),
        TypeKind::Primitive(PrimitiveType::StrSlice) => Ok(SliceKind::String),
        _ => Err(unsupported("slice argument expected", Some(function))),
    }
}

/// The element type of a list operand: a `Copy` of a local of list type.
fn list_element_of(lower: &ScanLower<'_>, operand: &Operand) -> Result<TypeId, CodegenError> {
    let Operand::Copy(local) = operand else {
        return Err(unsupported(
            "list argument must be a local",
            Some(lower.fn_id),
        ));
    };
    let ty = lower.program.local(*local).unwrap().ty;
    match lower.types.kind(ty) {
        TypeKind::List(element) => Ok(element),
        _ => Err(unsupported("list argument expected", Some(lower.fn_id))),
    }
}

fn check_terminator(
    lower: &mut ScanLower<'_>,
    terminator: &Terminator,
) -> Result<(), CodegenError> {
    match terminator {
        Terminator::Goto(_) | Terminator::Branch { .. } | Terminator::Unreachable => Ok(()),
        Terminator::Return(operand) => {
            // A returned function value materializes a capsule (the
            // retain/release bookkeeping is generic over managed
            // types); a returned string literal joins the pool.
            if let Some(op) = operand {
                if let Operand::Constant(Constant::String { string, .. }) = op
                    && let Some(content) = lower.program.string(*string)
                {
                    lower.strings.insert(content.clone());
                }
                scan_operand_class(lower, op)?;
            }
            Ok(())
        }
        Terminator::SwitchEnum { subject, .. } => {
            if scan_operand_class(lower, subject)? != Class::Ptr {
                return Err(unsupported("switch on non-managed type", Some(lower.fn_id)));
            }
            *lower.has_managed = true;
            Ok(())
        }
    }
}

fn require_str(lower: &mut ScanLower<'_>, operand: &Operand) -> Result<(), CodegenError> {
    if scan_operand_class(lower, operand)? == Class::Str {
        Ok(())
    } else {
        Err(unsupported("string argument expected", Some(lower.fn_id)))
    }
}

fn require_i64(lower: &mut ScanLower<'_>, operand: &Operand) -> Result<(), CodegenError> {
    if scan_operand_class(lower, operand)? == Class::I64 {
        Ok(())
    } else {
        Err(unsupported("integer argument expected", Some(lower.fn_id)))
    }
}

fn require_bool(lower: &mut ScanLower<'_>, operand: &Operand) -> Result<(), CodegenError> {
    if scan_operand_class(lower, operand)? == Class::Bool {
        Ok(())
    } else {
        Err(unsupported("boolean argument expected", Some(lower.fn_id)))
    }
}

fn require_f64(lower: &mut ScanLower<'_>, operand: &Operand) -> Result<(), CodegenError> {
    if scan_operand_class(lower, operand)? == Class::F64 {
        Ok(())
    } else {
        Err(unsupported("float argument expected", Some(lower.fn_id)))
    }
}

// ---------------------------------------------------------------------------
// Type / class mapping.
// ---------------------------------------------------------------------------

fn is_void(types: &TypeInterner, ty: TypeId) -> bool {
    matches!(types.kind(ty), TypeKind::Primitive(PrimitiveType::Void))
}

fn local_val(types: &TypeInterner, ty: TypeId) -> Result<Val, CodegenError> {
    Ok(match types.kind(ty) {
        TypeKind::Primitive(PrimitiveType::Void) => Val::I64,
        TypeKind::Primitive(PrimitiveType::Int) => Val::I64,
        TypeKind::Primitive(PrimitiveType::Float) => Val::F64,
        TypeKind::Primitive(PrimitiveType::Bool) => Val::I32,
        TypeKind::Primitive(PrimitiveType::Char) => Val::I32,
        TypeKind::Primitive(PrimitiveType::String) => Val::I32,
        TypeKind::Primitive(PrimitiveType::StrSlice) => Val::I32,
        TypeKind::Primitive(PrimitiveType::VectorI64x2) => {
            return Err(unsupported(
                "SIMD scalars are deferred on wasm32 (5D2b)",
                None,
            ));
        }
        // The remaining primitives are the six integer types; the wasm ABI
        // carries every integer as i64.
        TypeKind::Primitive(_) => Val::I64,
        // 5D2b: a closure's capsule pointer (16-byte ARC node).
        TypeKind::Function { .. } => Val::I32,
        // 5D2b slice 3: a task handle (32-byte ARC node).
        TypeKind::Task(_) => Val::I32,
        TypeKind::List(_)
        | TypeKind::Slice(_)
        | TypeKind::Map { .. }
        | TypeKind::Tuple(_)
        | TypeKind::Nominal { .. } => Val::I32,
        other => return Err(unsupported(type_desc(&other), None)),
    })
}

fn operand_class(
    program: &MirProgram,
    types: &TypeInterner,
    operand: &Operand,
) -> Result<Class, CodegenError> {
    match operand {
        Operand::Copy(local) => {
            let ty = program
                .local(*local)
                .ok_or_else(|| {
                    CodegenError::new(
                        None,
                        CodegenErrorKind::ObjectEmissionFailed("unknown local".to_string()),
                    )
                })?
                .ty;
            class_of_type(types, ty)
        }
        Operand::Constant(c) => Ok(match c {
            Constant::Integer(_) => Class::I64,
            Constant::FloatBits(_) => Class::F64,
            Constant::Bool(_) => Class::Bool,
            Constant::Character { .. } => Class::Char,
            Constant::String { .. } => Class::Str,
        }),
        // 5D2b slice 2: a bare function value is a 16-byte capsule
        // ARC node (a heap pointer).
        Operand::Function(_) => Ok(Class::Ptr),
    }
}

fn class_of_type(types: &TypeInterner, ty: TypeId) -> Result<Class, CodegenError> {
    Ok(match types.kind(ty) {
        TypeKind::Primitive(PrimitiveType::Int) => Class::I64,
        TypeKind::Primitive(PrimitiveType::Float) => Class::F64,
        TypeKind::Primitive(PrimitiveType::Bool) => Class::Bool,
        TypeKind::Primitive(PrimitiveType::Char) => Class::Char,
        TypeKind::Primitive(PrimitiveType::String) => Class::Str,
        TypeKind::Primitive(PrimitiveType::Void) => return Err(unsupported("void operand", None)),
        TypeKind::Primitive(PrimitiveType::StrSlice) => Class::Ptr,
        TypeKind::Primitive(PrimitiveType::VectorI64x2) => {
            return Err(unsupported(
                "SIMD scalars are deferred on wasm32 (5D2b)",
                None,
            ));
        }
        // The remaining primitives are the six integer types; the wasm ABI
        // carries every integer as i64.
        TypeKind::Primitive(_) => Class::I64,
        // Function values are 16-byte capsule ARC nodes (5D2b); task
        // handles are 32-byte ARC nodes (slice 3).
        TypeKind::List(_)
        | TypeKind::Slice(_)
        | TypeKind::Map { .. }
        | TypeKind::Tuple(_)
        | TypeKind::Nominal { .. }
        | TypeKind::Function { .. }
        | TypeKind::Task(_) => Class::Ptr,
        other => return Err(unsupported(type_desc(&other), None)),
    })
}

/// Load a field/element slot of `ty` at `offset` (wasm width per
/// class: i64 for Int, f64 for Float, i32 for the rest).
fn load_by_type(fb: &mut FB, types: &TypeInterner, offset: u32, ty: TypeId) {
    match class_of_type(types, ty).unwrap() {
        Class::I64 => fb.load64(offset),
        Class::F64 => fb.loadf64(offset),
        _ => fb.load32(offset),
    };
}

/// Store a value of `ty` at `offset` (the value is already on the
/// stack in its natural class width).
fn store_by_type(fb: &mut FB, types: &TypeInterner, offset: u32, ty: TypeId) {
    match class_of_type(types, ty).unwrap() {
        Class::I64 => fb.store64(offset),
        Class::F64 => fb.storef64(offset),
        _ => fb.store32(offset),
    };
}

/// Zero-extend an i32-class value (Bool/Char) to the i64 list slot
/// width (Int values are already i64).
fn widen_to_i64(fb: &mut FB, types: &TypeInterner, element: TypeId) {
    if class_of_type(types, element).unwrap() != Class::I64 {
        fb.op(op::I64_EXTEND_I32_U);
    }
}

fn type_desc(kind: &TypeKind) -> &'static str {
    match kind {
        TypeKind::List(_) => "list",
        TypeKind::Nominal { .. } => "struct",
        TypeKind::Slice(_) => "slice",
        TypeKind::Tuple(_) => "tuple",
        TypeKind::Function { .. } => "function value",
        TypeKind::Task(_) => "task",
        TypeKind::Map { .. } => "map",
        TypeKind::Primitive(PrimitiveType::StrSlice) => "str slice",
        TypeKind::Primitive(PrimitiveType::VectorI64x2) => "simd vector",
        TypeKind::Primitive(PrimitiveType::Void) => "void",
        _ => "type",
    }
}

fn unsupported(construct: &'static str, fn_id: Option<MirFunctionId>) -> CodegenError {
    CodegenError::new(fn_id, CodegenErrorKind::UnsupportedConstruct { construct })
}

fn unrepresentable(builtin: BuiltinId, reason: &'static str, fn_id: MirFunctionId) -> CodegenError {
    CodegenError::new(
        Some(fn_id),
        CodegenErrorKind::UnrepresentableBuiltin { builtin, reason },
    )
}

// ---------------------------------------------------------------------------
// String pool.
// ---------------------------------------------------------------------------

/// Register a signature in the type table, returning its index.
fn reg_type(
    map: &mut HashMap<(Vec<Val>, Vec<Val>), u32>,
    list: &mut Vec<(Vec<Val>, Vec<Val>)>,
    params: &[Val],
    results: &[Val],
) -> u32 {
    let key = (params.to_vec(), results.to_vec());
    if let Some(&idx) = map.get(&key) {
        return idx;
    }
    let idx = list.len() as u32;
    map.insert(key.clone(), idx);
    list.push(key);
    idx
}

fn build_pool(strings: &BTreeSet<String>) -> (HashMap<String, u32>, Vec<u8>) {
    let mut string_ptr = HashMap::new();
    let mut pool = Vec::new();
    let mut ptr = POOL_START;
    for content in strings {
        let base = ptr;
        string_ptr.insert(content.clone(), base + STR_HEADER);
        pool.extend_from_slice(&IMMORTAL_RC.to_le_bytes());
        pool.extend_from_slice(&0i64.to_le_bytes());
        pool.extend_from_slice(&ARC_MAGIC.to_le_bytes());
        pool.extend_from_slice(&(content.len() as i32).to_le_bytes());
        pool.extend_from_slice(content.as_bytes());
        let entry = STR_HEADER as usize + 4 + content.len();
        let padded = entry.div_ceil(8) * 8;
        pool.resize(pool.len() + (padded - entry), 0);
        ptr = base + padded as u32;
    }
    (string_ptr, pool)
}

// ---------------------------------------------------------------------------
// User-function lowering (dispatch loop).
// ---------------------------------------------------------------------------

/// The lowered artifacts of one user function: the export name, the
/// parameter value classes, the result value classes, the function's extra
/// value locals, and the function body bytes.
type FnLowering = (Option<String>, Vec<Val>, Vec<Val>, Vec<Val>, Vec<u8>);

fn lower_user_function(env: &Env<'_>, fn_id: MirFunctionId) -> Result<FnLowering, CodegenError> {
    let function = env.program.function(fn_id).unwrap();
    let name = function
        .name
        .and_then(|s| env.names.resolve(s.raw()))
        .map(str::to_string);
    let all_locals = env.program.function_locals(function);
    let params = env.program.function_parameters(function);
    let is_closure = matches!(function.kind, lpp_mir::MirFunctionKind::Closure);
    // The closure frame's capture slots: the `function_parameters`
    // entries of `Capture` kind (they are also value locals — the
    // body reads and writes them). The remaining parameters are the
    // user parameters.
    let capture_frame: Vec<MirLocalId> = params
        .iter()
        .copied()
        .filter(|&p| {
            matches!(
                env.program.local(p).unwrap().kind,
                lpp_mir::MirLocalKind::Capture
            )
        })
        .collect();
    let user_params: Vec<MirLocalId> = params
        .iter()
        .copied()
        .filter(|p| !capture_frame.contains(p))
        .collect();

    let mut local_index: HashMap<MirLocalId, u32> = HashMap::new();
    // Every wasm function carries the env pointer as local 0 (the
    // 5D2b dispatch ABI): closures read their capture slots from it;
    // plain functions receive NULL and ignore it. This keeps the
    // dispatch table and the call_indirect signatures uniform (a
    // function value may dispatch to any function of its type).
    let mut param_vals: Vec<Val> = Vec::new();
    param_vals.push(Val::I32); // env
    for &p in &user_params {
        param_vals.push(local_val(env.types, env.program.local(p).unwrap().ty)?);
    }
    let env_offset: u32 = 1;
    for (i, &p) in user_params.iter().enumerate() {
        local_index.insert(p, env_offset + i as u32);
    }
    let p = param_vals.len() as u32;
    let mut fb = FB::new(p);
    let mut np = 0u32;
    for &l in all_locals {
        if let std::collections::hash_map::Entry::Vacant(e) = local_index.entry(l) {
            e.insert(p + np);
            let ty = env.program.local(l).unwrap().ty;
            fb.extras.push(local_val(env.types, ty)?);
            np += 1;
        }
    }
    let cur_local = p + np;
    fb.extras.push(Val::I32); // `current`

    if is_closure {
        // Entry: load the capture slots (views, no retain) from the env.
        for (i, &slot) in capture_frame.iter().enumerate() {
            let index = local_index[&slot];
            let ty = env.program.local(slot).unwrap().ty;
            fb.g(0);
            match local_val(env.types, ty)? {
                Val::I32 => fb.load32(8 * i as u32),
                Val::I64 => fb.load64(8 * i as u32),
                Val::F64 => fb.loadf64(8 * i as u32),
            };
            fb.s(index);
        }
    }

    let (order, positions) = block_layout(env.program, function);
    let entry_pos = positions[&function.entry];

    let mut capture_slot = HashMap::new();
    for (i, &c) in capture_frame.iter().enumerate() {
        capture_slot.insert(c, i as u32);
    }
    let lower = FnLower {
        env,
        local_index,
        cur_local,
        fn_id,
        captures: capture_slot,
    };

    // current = entry
    fb.i32c(entry_pos as i64);
    fb.s(cur_local);
    fb.loop_();
    for (pos, block_id) in order {
        let block = env.program.block(block_id).unwrap();
        fb.g(cur_local);
        fb.i32c(pos as i64);
        fb.op(op::I32_EQ);
        fb.if_();
        for &instr in env.program.block_instructions(block) {
            let instruction = env.program.instruction(instr).unwrap();
            match &instruction.kind {
                InstructionKind::Assign { target, value } => {
                    lower.emit_assign(&mut fb, *target, value)?;
                }
                InstructionKind::Store { place, value } => {
                    lower.emit_store(&mut fb, *place, value)?;
                }
            }
        }
        lower.emit_terminator(&mut fb, &block.terminator, &positions)?;
        fb.end();
    }
    // A wasm `loop` does not auto-repeat: its `end` falls through. Re-dispatch
    // explicitly by branching back to the loop start (depth 0) after the last
    // guard.
    fb.br(0);
    fb.end(); // end loop
    // The dispatch loop only exits through `return`/`unreachable` in a block,
    // so this point is unreachable; state it explicitly so the function's
    // result requirement is satisfied for value-returning functions.
    fb.op(op::UNREACHABLE);

    let results = if is_void(env.types, function.return_type) {
        vec![]
    } else {
        vec![local_val(env.types, function.return_type)?]
    };
    Ok((name, param_vals, results, fb.extras, fb.body))
}

impl<'a> FnLower<'a> {
    fn emit_operand(&self, fb: &mut FB, operand: &Operand) -> Result<Class, CodegenError> {
        let class = operand_class(self.env.program, self.env.types, operand)?;
        match operand {
            Operand::Copy(local) => {
                let index = *self
                    .local_index
                    .get(local)
                    .ok_or_else(|| unsupported("unknown local", Some(self.fn_id)))?;
                fb.g(index);
            }
            Operand::Constant(c) => {
                match c {
                    Constant::Integer(v) => fb.i64c(*v),
                    Constant::FloatBits(bits) => fb.f64c(f64::from_bits(*bits)),
                    Constant::Bool(b) => fb.i32c(if *b { 1 } else { 0 }),
                    Constant::Character { character, .. } => fb.i32c(*character as i64),
                    Constant::String { string, .. } => {
                        let content = self
                            .env
                            .program
                            .string(*string)
                            .ok_or_else(|| {
                                CodegenError::new(
                                    Some(self.fn_id),
                                    CodegenErrorKind::ObjectEmissionFailed(
                                        "unknown string constant".to_string(),
                                    ),
                                )
                            })?
                            .clone();
                        let ptr = *self.env.string_ptr.get(&content).ok_or_else(|| {
                            CodegenError::new(
                                Some(self.fn_id),
                                CodegenErrorKind::ObjectEmissionFailed(
                                    "string not pooled".to_string(),
                                ),
                            )
                        })?;
                        fb.i32c(ptr as i64)
                    }
                };
            }
            Operand::Function(function_id) => {
                // A bare function value is a 16-byte capsule
                // `[code, env]` on the ARC heap (5D2b slice 2); the
                // capsule starts with rc = 1 — ownership transfers to
                // the use site (no retain). A sync value points at the
                // function with a NULL env; an async value (slice 3)
                // points at its task thunk with an empty task-env
                // tuple (spawnable values need a non-NULL env).
                let callee = self.env.program.function(*function_id).unwrap();
                let is_async = matches!(callee.kind, lpp_mir::MirFunctionKind::Async);
                let dispatch_index = if is_async {
                    *self
                        .env
                        .task_thunk_dispatch
                        .get(function_id)
                        .ok_or_else(|| unsupported("undispatched task thunk", Some(self.fn_id)))?
                } else {
                    *self.env.dispatch.get(function_id).ok_or_else(|| {
                        unsupported("undispatched function value", Some(self.fn_id))
                    })?
                };
                let destroy_slot = self
                    .env
                    .closure_destroy_slot
                    .ok_or_else(|| unsupported("missing closure destroyer", Some(self.fn_id)))?;
                let capsule = fb.scratch(Val::I32);
                fb.i32c(16);
                fb.i32c(destroy_slot as i64);
                self.call_helper(fb, H::ArcAlloc)?;
                // Keep the capsule on the stack (the caller owns it);
                // the scratch only addresses the fill-in stores.
                fb.t(capsule);
                fb.g(capsule).i64c(dispatch_index as i64).store64(0);
                if is_async {
                    let tuple_slot = self
                        .env
                        .tuple_drop_slot
                        .ok_or_else(|| unsupported("missing tuple destroyer", Some(self.fn_id)))?;
                    fb.i32c(16);
                    fb.i32c(tuple_slot as i64);
                    self.call_helper(fb, H::ArcAlloc)?;
                    let tuple = fb.scratch(Val::I32);
                    fb.t(tuple);
                    fb.g(tuple).i64c(0).store64(0); // mask
                    fb.g(tuple).i64c(0).store64(8); // packed offsets
                    // The empty tuple is a fresh node (rc = 1); its
                    // reference transfers to the capsule. The tee's
                    // stack copy is consumed by the store.
                    fb.g(capsule).op(op::I64_EXTEND_I32_S).store64(8);
                } else {
                    fb.g(capsule).i64c(0).store64(8);
                }
            }
        }
        Ok(class)
    }

    /// An operand that creates a new owner: a plain read, plus a retain
    /// when it is a `Copy` of a managed local (retain-on-transfer).
    /// `Retain` consumes the pointer, so the local is re-pushed after —
    /// the caller keeps its value and gains one reference count.
    fn emit_owned(&self, fb: &mut FB, operand: &Operand) -> Result<Class, CodegenError> {
        let class = self.emit_operand(fb, operand)?;
        if let Operand::Copy(local) = operand {
            let ty = self.env.program.local(*local).unwrap().ty;
            if is_managed(self.env.types, ty) {
                // Retain passes the value through; the caller keeps the
                // copy on the stack. Strings are managed too even though
                // their lowering class is `Str` rather than the generic
                // pointer class.
                self.call_helper(fb, H::Retain)?;
            }
        }
        Ok(class)
    }

    /// `target = rvalue`: produce the value (transfers retain inside),
    /// release the slot's old reference (if any), then store the new
    /// value — the interpreter's order (a self-assignment retains before
    /// it releases).
    fn emit_assign(
        &self,
        fb: &mut FB,
        target: MirLocalId,
        rvalue: &Rvalue,
    ) -> Result<(), CodegenError> {
        let target_ty = self.env.program.local(target).unwrap().ty;
        let _class = if let Rvalue::List(items) = rvalue {
            self.emit_list_literal(fb, target_ty, items)?
        } else if let Rvalue::Tuple(items) = rvalue {
            self.emit_tuple_literal(fb, target_ty, items)?
        } else if let Rvalue::Use(Operand::Constant(Constant::Integer(value))) = rvalue
            && matches!(self.env.types.kind(target_ty), TypeKind::Nominal { .. })
        {
            // Legacy aggregate null is spelled integer `0` in source. MIR
            // materializes it as a nominally typed local, while wasm pointers
            // are i32, so emit the context-typed zero directly instead of the
            // canonical i64 integer constant.
            fb.i32c(*value);
            Class::Ptr
        } else if let Rvalue::Use(operand) = rvalue {
            // `Use(Copy(managed))` transfers a new reference to the
            // target: retain on copy (a plain `emit_operand` would leave
            // the target a dangling alias, double-freed at exit).
            self.emit_owned(fb, operand)?
        } else if let Rvalue::Builtin { builtin, .. } = rvalue
            && builtin
                .descriptor()
                .name
                .strip_prefix("lpp_")
                .unwrap_or(builtin.descriptor().name)
                == "list_new"
        {
            // The element class (hence the is-ARC word) comes from the
            // target's list type.
            self.emit_list_new(fb, target_ty)?
        } else if let Rvalue::Builtin { builtin, .. } = rvalue
            && matches!(
                builtin
                    .descriptor()
                    .name
                    .strip_prefix("lpp_")
                    .unwrap_or(builtin.descriptor().name),
                "map_new" | "map_new_arc"
            )
        {
            self.emit_map_new(fb, target_ty)?
        } else {
            self.emit_rvalue(fb, rvalue)?
        };
        let index = *self
            .local_index
            .get(&target)
            .ok_or_else(|| unsupported("unknown local", Some(self.fn_id)))?;
        match self.captures.get(&target) {
            // 5D2b: a capture local is a view of its env slot (local 0
            // holds the env). The env slot owns the managed reference,
            // so the old value is released through the slot — never
            // through the local — then the new value lands in both.
            Some(&slot) => {
                if is_managed(self.env.types, target_ty) {
                    fb.g(0);
                    match local_val(self.env.types, target_ty)? {
                        Val::I32 => fb.load32(8 * slot),
                        Val::I64 => fb.load64(8 * slot),
                        Val::F64 => fb.loadf64(8 * slot),
                    };
                    self.call_helper(fb, H::Release)?;
                }
                fb.s(index);
                fb.g(0);
                fb.g(index);
                match local_val(self.env.types, target_ty)? {
                    Val::I32 => fb.store32(8 * slot),
                    Val::I64 => fb.store64(8 * slot),
                    Val::F64 => fb.storef64(8 * slot),
                };
                Ok(())
            }
            None => {
                if is_managed(self.env.types, target_ty) {
                    fb.g(index);
                    self.call_helper(fb, H::Release)?;
                }
                fb.s(index);
                Ok(())
            }
        }
    }

    /// Emit an operand as a value of the destination language type. The only
    /// contextual representation exception is legacy nominal null (`0`): an
    /// integer constant is normally i64, while a wasm aggregate pointer is i32.
    fn emit_owned_for_type(
        &self,
        fb: &mut FB,
        value: &Operand,
        destination: TypeId,
    ) -> Result<Class, CodegenError> {
        if matches!(value, Operand::Constant(Constant::Integer(0)))
            && matches!(self.env.types.kind(destination), TypeKind::Nominal { .. })
        {
            fb.i32c(0);
            Ok(Class::Ptr)
        } else {
            self.emit_owned(fb, value)
        }
    }

    /// `place = operand`: a bare local is an assign; a projected store
    /// walks the chain, releases the old field (managed fields only),
    /// and writes the element/field in place.
    fn emit_store(
        &self,
        fb: &mut FB,
        place: lpp_mir::MirPlaceId,
        value: &Operand,
    ) -> Result<(), CodegenError> {
        let resolution = resolve_place(
            self.env.program,
            self.env.types,
            place,
            &self.env.layouts,
            &self.env.aggregates,
            true,
            self.fn_id,
        )?;
        if resolution.steps.is_empty() {
            let _class = self.emit_owned_for_type(fb, value, resolution.ty)?;
            let index = *self
                .local_index
                .get(&resolution.root)
                .ok_or_else(|| unsupported("unknown local", Some(self.fn_id)))?;
            if is_managed(self.env.types, resolution.ty) {
                fb.g(index);
                self.call_helper(fb, H::Release)?;
            }
            fb.s(index);
            return Ok(());
        }
        let last = resolution.steps.last().unwrap();
        // The chain leaves exactly one base pointer on the stack. `tee`
        // parks it in a scratch local without duplicating it, so any
        // re-addressing must `get` the scratch (that `get` is the only
        // extra stack copy, and the final store/consumer pops it).
        let base_scratch = fb.scratch(Val::I32);
        self.emit_place_chain(fb, &resolution)?;
        fb.t(base_scratch);
        match last {
            PlaceStep::Field {
                offset,
                ty,
                managed,
            } => {
                // Stack: [base]. The i* store pops (value, addr): the
                // value goes on top of the single base copy.
                if *managed {
                    // Release the old field: re-fetch the base, load the
                    // field, release. The re-fetch is consumed by the
                    // load, leaving the original base on the stack.
                    fb.g(base_scratch);
                    fb.load32(*offset);
                    self.call_helper(fb, H::Release)?;
                }
                let _class = self.emit_owned_for_type(fb, value, *ty)?;
                store_by_type(fb, self.env.types, *offset, *ty);
                // Stack: [] — the store consumed the base and the value.
            }
            PlaceStep::TupleField {
                offset,
                ty,
                managed,
            } => {
                if *managed {
                    fb.g(base_scratch);
                    fb.load32(*offset);
                    self.call_helper(fb, H::Release)?;
                }
                self.emit_owned_for_type(fb, value, *ty)?;
                store_by_type(fb, self.env.types, *offset, *ty);
            }
            PlaceStep::ListIndex { element, index } => {
                // [base] -> get scratch -> [base, base] -> idx -> value
                // -> the ListSet* helper pops (value, idx, ptr), leaving
                // the chain's base copy to drop.
                fb.g(base_scratch);
                self.emit_operand(fb, index)?;
                let h = match element_class(self.env.types, *element) {
                    ElementClass::I64 => {
                        let _class = self.emit_operand(fb, value)?;
                        widen_to_i64(fb, self.env.types, *element);
                        H::ListSetI64
                    }
                    ElementClass::F64 => {
                        self.emit_operand(fb, value)?;
                        H::ListSetF64
                    }
                    ElementClass::Ptr => {
                        // The helper releases the replaced element and
                        // retains the new one (runtime-internal). Null must be
                        // emitted as the list element's i32 pointer type.
                        if matches!(value, Operand::Constant(Constant::Integer(0)))
                            && matches!(self.env.types.kind(*element), TypeKind::Nominal { .. })
                        {
                            fb.i32c(0);
                        } else {
                            self.emit_operand(fb, value)?;
                        }
                        H::ListSetPtr
                    }
                };
                self.call_helper(fb, h)?;
                // Stack: [base] — drop the chain's copy (the root local
                // still owns its reference).
                fb.op(op::DROP);
            }
            PlaceStep::Downcast => {
                return Err(unsupported("store through downcast", Some(self.fn_id)));
            }
        }
        Ok(())
    }

    /// Evaluate the projection chain up to (excluding) the final step;
    /// leaves the final base pointer on the stack. Intermediate steps
    /// are plain borrows (no retain): a field load or an ARC-element
    /// read the caller does not release.
    fn emit_place_chain(
        &self,
        fb: &mut FB,
        resolution: &PlaceResolution,
    ) -> Result<(), CodegenError> {
        let root_index = *self
            .local_index
            .get(&resolution.root)
            .ok_or_else(|| unsupported("unknown local", Some(self.fn_id)))?;
        fb.g(root_index);
        for step in resolution.steps.iter().take(resolution.steps.len() - 1) {
            match step {
                PlaceStep::Downcast => {}
                PlaceStep::Field { offset, .. } | PlaceStep::TupleField { offset, .. } => {
                    // Intermediate fields are managed (verified), i32 slots.
                    fb.load32(*offset);
                }
                PlaceStep::ListIndex { element, index } => {
                    // Stack: [base] → emit the index → [base, idx] →
                    // the plain element read → [element].
                    self.emit_operand(fb, index)?;
                    match element_class(self.env.types, *element) {
                        // Intermediate elements are managed (verified).
                        ElementClass::Ptr => self.call_helper(fb, H::ListGetPtr)?,
                        ElementClass::I64 | ElementClass::F64 => {
                            return Err(unsupported(
                                "projection through scalar list element",
                                Some(self.fn_id),
                            ));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// `Load(place)`: a bare local is a plain read (no retain); a
    /// projected read loads the field/element and retains when managed
    /// (the container keeps its own reference).
    fn emit_load_place(
        &self,
        fb: &mut FB,
        place: lpp_mir::MirPlaceId,
    ) -> Result<Class, CodegenError> {
        let resolution = resolve_place(
            self.env.program,
            self.env.types,
            place,
            &self.env.layouts,
            &self.env.aggregates,
            false,
            self.fn_id,
        )?;
        if resolution.steps.is_empty() {
            return self.emit_operand(fb, &Operand::Copy(resolution.root));
        }
        let last = resolution.steps.last().unwrap();
        // The chain leaves exactly one base pointer on the stack; the final
        // step consumes it (field load pops it, list getter pops it with the
        // index), so nothing leaks.
        self.emit_place_chain(fb, &resolution)?;
        match last {
            PlaceStep::Field {
                offset,
                ty,
                managed,
            } => {
                load_by_type(fb, self.env.types, *offset, *ty);
                if *managed {
                    // The loaded pointer is a fresh owner: Retain passes
                    // the value through, so the caller keeps it and the
                    // reference count is bumped. (No tee: scratch is not
                    // part of the exit-release set — parking a copy there
                    // would leak the reference.)
                    self.call_helper(fb, H::Retain)?;
                }
                class_of_type(self.env.types, *ty)
            }
            PlaceStep::TupleField {
                offset,
                ty,
                managed,
            } => {
                load_by_type(fb, self.env.types, *offset, *ty);
                if *managed {
                    self.call_helper(fb, H::Retain)?;
                }
                class_of_type(self.env.types, *ty)
            }
            PlaceStep::ListIndex { element, index } => {
                self.emit_operand(fb, index)?;
                match element_class(self.env.types, *element) {
                    ElementClass::I64 => {
                        self.call_helper(fb, H::ListGetI64)?;
                        if !matches!(
                            self.env.types.kind(*element),
                            TypeKind::Primitive(PrimitiveType::Int)
                        ) {
                            fb.op(op::I32_WRAP_I64);
                        }
                        class_of_type(self.env.types, *element)
                    }
                    ElementClass::F64 => {
                        self.call_helper(fb, H::ListGetF64)?;
                        Ok(Class::F64)
                    }
                    ElementClass::Ptr => {
                        // The getter returns the element's pointer without
                        // retaining; the load creates a fresh owner, so
                        // Retain (pass-through) bumps it. No tee — scratch
                        // is not released by the exit pass.
                        self.call_helper(fb, H::ListGetPtr)?;
                        self.call_helper(fb, H::Retain)?;
                        Ok(Class::Ptr)
                    }
                }
            }
            PlaceStep::Downcast => Err(unsupported("load through downcast", Some(self.fn_id))),
        }
    }

    /// A list literal `[e1, e2, …]` of element type `element`: `ListNew`
    /// then one push per element (a `Copy` element retains for the
    /// list; the source keeps its own reference).
    fn emit_list_literal(
        &self,
        fb: &mut FB,
        target_ty: TypeId,
        items: &lpp_mir::ListRange<Operand>,
    ) -> Result<Class, CodegenError> {
        let TypeKind::List(element) = self.env.types.kind(target_ty) else {
            return Err(unsupported("list literal of non-list", Some(self.fn_id)));
        };
        let ec = element_class(self.env.types, element);
        let list = fb.scratch(Val::I32);
        fb.i32c(if ec == ElementClass::Ptr { 1 } else { 0 });
        self.call_helper(fb, H::ListNew)?;
        fb.s(list);
        for operand in self.env.program.operands(*items) {
            fb.g(list);
            let _class = self.emit_owned(fb, operand)?;
            match ec {
                ElementClass::I64 => {
                    widen_to_i64(fb, self.env.types, element);
                    self.call_helper(fb, H::ListPushI64)?;
                }
                ElementClass::F64 => {
                    self.call_helper(fb, H::ListPushF64)?;
                }
                ElementClass::Ptr => {
                    self.call_helper(fb, H::ListPushPtr)?;
                }
            }
        }
        fb.g(list);
        Ok(Class::Ptr)
    }

    fn emit_tuple_literal(
        &self,
        fb: &mut FB,
        target_ty: TypeId,
        items: &lpp_mir::ListRange<Operand>,
    ) -> Result<Class, CodegenError> {
        let TypeKind::Tuple(elements) = self.env.types.kind(target_ty) else {
            return Err(unsupported("tuple literal of non-tuple", Some(self.fn_id)));
        };
        let element_types = self.env.types.list(elements);
        let values = self.env.program.operands(*items);
        if values.len() != element_types.len() || values.len() > 64 {
            return Err(unsupported("tuple arity", Some(self.fn_id)));
        }
        let drop_slot = self
            .env
            .tuple_drop_slot
            .ok_or_else(|| unsupported("missing tuple destroyer", Some(self.fn_id)))?;
        let tuple = fb.scratch(Val::I32);
        fb.i32c((16 + values.len() * 8) as i64);
        fb.i32c(drop_slot as i64);
        self.call_helper(fb, H::ArcAlloc)?;
        fb.s(tuple);
        let mut managed_mask = 0u64;
        for (index, (&ty, value)) in element_types.iter().zip(values.iter()).enumerate() {
            if is_managed(self.env.types, ty) {
                managed_mask |= 1u64 << index;
            }
            fb.g(tuple);
            self.emit_owned(fb, value)?;
            store_by_type(fb, self.env.types, 16 + 8 * index as u32, ty);
        }
        fb.g(tuple).i64c(managed_mask as i64).store64(0);
        fb.g(tuple).i64c(values.len() as i64).store64(8);
        fb.g(tuple);
        Ok(Class::Ptr)
    }

    fn emit_rvalue(&self, fb: &mut FB, rvalue: &Rvalue) -> Result<Class, CodegenError> {
        match rvalue {
            Rvalue::Use(operand) => self.emit_operand(fb, operand),
            Rvalue::Unary { operator, operand } => self.emit_unary(fb, *operator, operand),
            Rvalue::Binary {
                left,
                operator,
                right,
            } => self.emit_binary(fb, left, *operator, right),
            Rvalue::Call { callee, arguments } => self.emit_call(fb, *callee, arguments),
            Rvalue::Builtin { builtin, arguments } => self.emit_builtin(fb, *builtin, arguments),
            Rvalue::Load(place) => self.emit_load_place(fb, *place),
            // List literals are produced by `emit_assign` (the target
            // type supplies the element class of `[]`).
            Rvalue::List(_) => Err(unsupported("list literal outside assign", Some(self.fn_id))),
            Rvalue::Tuple(_) => Err(unsupported("tuple", Some(self.fn_id))),
            Rvalue::ConstructStruct { aggregate, fields } => {
                self.emit_construct(fb, *aggregate, None, fields)
            }
            Rvalue::ConstructVariant {
                aggregate,
                variant,
                fields,
            } => {
                let ordinal = self.env.program.variant(*variant).unwrap().ordinal;
                self.emit_construct(fb, *aggregate, Some(ordinal), fields)
            }
            Rvalue::ListLen(operand) => {
                self.emit_operand(fb, operand)?;
                self.call_helper(fb, H::ListLen)?;
                Ok(Class::I64)
            }
            Rvalue::MakeClosure { function, captures } => {
                self.emit_make_closure(fb, *function, captures)
            }
            Rvalue::Await(operand) => self.emit_await(fb, operand),
            Rvalue::Spawn(operand) => self.emit_spawn(fb, operand),
        }
    }

    /// `x.await` (5D2b slice 3): `TaskAwait(x)` — polls when fresh and
    /// retains a managed result — then unboxes the result word to the
    /// type behind the `Task`.
    fn emit_await(&self, fb: &mut FB, operand: &Operand) -> Result<Class, CodegenError> {
        let Operand::Copy(local) = operand else {
            return Err(unsupported("await of non-local task", Some(self.fn_id)));
        };
        let ty = self.env.program.local(*local).unwrap().ty;
        let inner = match self.env.types.kind(ty) {
            TypeKind::Task(inner) => inner,
            _ => return Err(unsupported("await of non-task", Some(self.fn_id))),
        };
        self.emit_operand(fb, operand)?;
        self.call_helper(fb, H::TaskAwait)?;
        if is_void(self.env.types, inner) {
            fb.op(op::DROP);
            fb.i64c(0);
            return Ok(Class::I64);
        }
        match local_val(self.env.types, inner)? {
            Val::I64 => Ok(Class::I64),
            Val::F64 => {
                fb.op(op::F64_REINTERPRET_I64);
                Ok(Class::F64)
            }
            Val::I32 => {
                fb.op(op::I32_WRAP_I64);
                if matches!(
                    self.env.types.kind(inner),
                    TypeKind::List(_) | TypeKind::Nominal { .. } | TypeKind::Task(_)
                ) {
                    Ok(Class::Ptr)
                } else {
                    Ok(Class::Bool)
                }
            }
        }
    }

    /// `spawn(x)` (5D2b slice 3): eager — build the task (code from
    /// the capsule, env a wrapper tuple holding the retained closure
    /// env or an empty tuple for a bare function), poll it to
    /// completion, and destroy the node. The statement yields nothing.
    fn emit_spawn(&self, fb: &mut FB, operand: &Operand) -> Result<Class, CodegenError> {
        let Operand::Copy(local) = operand else {
            return Err(unsupported("spawn of non-local value", Some(self.fn_id)));
        };
        let ty = self.env.program.local(*local).unwrap().ty;
        let TypeKind::Function { .. } = self.env.types.kind(ty) else {
            return Err(unsupported("spawn of non-function", Some(self.fn_id)));
        };
        let origin = self
            .env
            .value_origins
            .get(&(self.fn_id, *local))
            .copied()
            .ok_or_else(|| {
                unsupported(
                    "spawn of a function value of unknown origin",
                    Some(self.fn_id),
                )
            })?;
        let origin_fn = self.env.program.function(origin).unwrap();
        let index = *self
            .local_index
            .get(local)
            .ok_or_else(|| unsupported("unknown local", Some(self.fn_id)))?;
        // The task env: a 1-slot wrapper tuple whose slot 16 holds the
        // capsule env (NULL for a zero-capture closure or a bare
        // function — the slot is then simply 0).
        let tuple_slot = self
            .env
            .tuple_drop_slot
            .ok_or_else(|| unsupported("missing tuple destroyer", Some(self.fn_id)))?;
        let is_closure = matches!(origin_fn.kind, lpp_mir::MirFunctionKind::Closure);
        let is_sync_bare = matches!(origin_fn.kind, lpp_mir::MirFunctionKind::Function);
        if !is_closure && !is_sync_bare {
            return Err(unsupported("spawn of an async value", Some(self.fn_id)));
        }
        let code = if is_closure {
            *self
                .env
                .closure_thunk_dispatch
                .get(&origin)
                .ok_or_else(|| unsupported("undispatched closure thunk", Some(self.fn_id)))?
        } else {
            *self
                .env
                .task_thunk_dispatch
                .get(&origin)
                .ok_or_else(|| unsupported("undispatched task thunk", Some(self.fn_id)))?
        };
        fb.i32c(24);
        fb.i32c(tuple_slot as i64);
        self.call_helper(fb, H::ArcAlloc)?;
        let tuple = fb.scratch(Val::I32);
        fb.t(tuple);
        fb.g(tuple).i64c(if is_closure { 1 } else { 0 }).store64(0);
        fb.g(tuple).i64c(if is_closure { 16 } else { 0 }).store64(8);
        // The capsule pointer, retained for the wrapper tuple: the
        // closure thunk reads the closure env through it (a
        // bare-function wrapper stores 0 instead). The local already
        // HOLDS the capsule pointer (an i32 heap offset) — retain it
        // directly; no dereference.
        fb.g(index);
        self.call_helper(fb, H::Retain)?;
        let cap = fb.scratch(Val::I32);
        fb.t(cap);
        fb.g(tuple).g(cap).op(op::I64_EXTEND_I32_S).store64(16);
        fb.g(cap).op(op::DROP);
        // TaskNew(code, env, 0) — a spawned closure/function is void.
        fb.i32c(code as i64);
        fb.g(tuple);
        fb.i32c(0);
        self.call_helper(fb, H::TaskNew)?;
        // TaskNew returns the node pointer as an i64 word; narrow
        // for the wasm ABI before the scratch tee.
        let task = fb.scratch(Val::I32);
        fb.op(op::I32_WRAP_I64);
        fb.t(task);
        // Eager: poll to completion, then release the node.
        fb.g(task)
            .call(self.env.helper_index[&H::TaskPoll])
            .op(op::DROP);
        fb.g(task).call(self.env.helper_index[&H::TaskDestroy]);
        fb.i64c(0);
        Ok(Class::I64)
    }

    /// `ArcAlloc(total, drop-slot)` then one field store per slot: a
    /// `Copy` field retains for the new owner (the source keeps its own
    /// reference); a constructed field lands with its initial reference.
    fn emit_construct(
        &self,
        fb: &mut FB,
        aggregate: MirAggregateId,
        tag: Option<u32>,
        fields: &lpp_mir::ListRange<Operand>,
    ) -> Result<Class, CodegenError> {
        let layout = self
            .env
            .layouts
            .get(&aggregate)
            .ok_or_else(|| unsupported("unlaid-out nominal", Some(self.fn_id)))?
            .clone();
        let drop_slot = *self
            .env
            .drop_slot
            .get(&aggregate)
            .ok_or_else(|| unsupported("unregistered drop slot", Some(self.fn_id)))?;
        let ptr = fb.scratch(Val::I32);
        fb.i32c(layout.total_size as i64);
        fb.i32c(drop_slot as i64);
        self.call_helper(fb, H::ArcAlloc)?;
        fb.s(ptr);
        if let Some(ordinal) = tag {
            // The i32 tag at offset 0 (bytes 4..8 stay the padding of
            // the native 8-byte tag — the shared layout is identical).
            fb.g(ptr);
            fb.i32c(ordinal as i64);
            fb.store32(ENUM_TAG as u32);
        }
        let slots = match tag {
            None => &layout.struct_fields,
            Some(ordinal) => &layout.variant_fields[ordinal as usize],
        };
        let values = self.env.program.operands(*fields);
        if values.len() != slots.len() {
            return Err(unsupported("aggregate field count", Some(self.fn_id)));
        }
        for (operand, slot) in values.iter().zip(slots.iter()) {
            // wasm stores pop (value, addr): address first, value on top.
            fb.g(ptr);
            if matches!(operand, Operand::Constant(Constant::Integer(0)))
                && matches!(self.env.types.kind(slot.ty), TypeKind::Nominal { .. })
            {
                // Context-typed legacy null: nominal slots are i32 pointers,
                // while a standalone integer constant would otherwise emit i64.
                fb.i32c(0);
            } else {
                let _class = self.emit_owned(fb, operand)?;
            }
            store_by_type(fb, self.env.types, slot.offset, slot.ty);
        }
        fb.g(ptr);
        Ok(Class::Ptr)
    }

    fn emit_unary(
        &self,
        fb: &mut FB,
        operator: UnaryOperator,
        operand: &Operand,
    ) -> Result<Class, CodegenError> {
        let class = operand_class(self.env.program, self.env.types, operand)?;
        match (operator, class) {
            (UnaryOperator::Not, Class::Bool) => {
                self.emit_operand(fb, operand)?;
                fb.op(op::I32_EQZ);
                Ok(Class::Bool)
            }
            (UnaryOperator::Not, Class::I64) => {
                self.emit_operand(fb, operand)?;
                fb.op(op::I64_EQZ);
                Ok(Class::Bool)
            }
            (UnaryOperator::Negate, Class::I64) => {
                fb.i64c(0);
                self.emit_operand(fb, operand)?;
                fb.op(op::I64_SUB); // 0 - value
                Ok(Class::I64)
            }
            (UnaryOperator::Negate, Class::F64) => {
                self.emit_operand(fb, operand)?;
                fb.op(op::F64_NEG);
                Ok(Class::F64)
            }
            _ => Err(unsupported(
                "unary operator on this operand type",
                Some(self.fn_id),
            )),
        }
    }

    fn emit_binary(
        &self,
        fb: &mut FB,
        left: &Operand,
        operator: BinaryOperator,
        right: &Operand,
    ) -> Result<Class, CodegenError> {
        let left_static = operand_class(self.env.program, self.env.types, left)?;
        let right_static = operand_class(self.env.program, self.env.types, right)?;
        let left_is_null = matches!(left, Operand::Constant(Constant::Integer(0)));
        let right_is_null = matches!(right, Operand::Constant(Constant::Integer(0)));
        let lc = if left_is_null && right_static == Class::Ptr {
            fb.i32c(0);
            Class::Ptr
        } else {
            self.emit_operand(fb, left)?
        };
        let rc = if right_is_null && left_static == Class::Ptr {
            fb.i32c(0);
            Class::Ptr
        } else {
            self.emit_operand(fb, right)?
        }; // stack: [l, r]
        if lc != rc {
            return Err(unsupported(
                "binary operand type mismatch",
                Some(self.fn_id),
            ));
        }
        match (operator, lc) {
            (BinaryOperator::Equal, Class::Ptr) => {
                fb.op(op::I32_EQ);
                Ok(Class::Bool)
            }
            (BinaryOperator::NotEqual, Class::Ptr) => {
                fb.op(op::I32_NE);
                Ok(Class::Bool)
            }
            (_, Class::Ptr) => Err(unsupported(
                "binary operator on managed type",
                Some(self.fn_id),
            )),
            // --- String operations ---
            (BinaryOperator::Add, Class::Str) => {
                self.call_helper(fb, H::StrConcat)?;
                Ok(Class::Str)
            }
            (BinaryOperator::Equal, Class::Str) => {
                self.call_helper(fb, H::StrEq)?;
                Ok(Class::Bool)
            }
            (BinaryOperator::NotEqual, Class::Str) => {
                self.call_helper(fb, H::StrEq)?;
                fb.op(op::I32_EQZ);
                Ok(Class::Bool)
            }
            (_, Class::Str) => Err(unsupported("string operator", Some(self.fn_id))),
            // --- Int (i64) ---
            (op_, Class::I64) => {
                self.i64_op(fb, op_)?;
                Ok(match op_ {
                    BinaryOperator::Equal
                    | BinaryOperator::NotEqual
                    | BinaryOperator::Less
                    | BinaryOperator::Greater
                    | BinaryOperator::LessEqual
                    | BinaryOperator::GreaterEqual
                    | BinaryOperator::LogicalAnd
                    | BinaryOperator::LogicalOr => Class::Bool,
                    _ => Class::I64,
                })
            }
            // --- Float (f64) ---
            (op_, Class::F64) => {
                self.f64_op(fb, op_)?;
                Ok(match op_ {
                    BinaryOperator::Equal
                    | BinaryOperator::NotEqual
                    | BinaryOperator::Less
                    | BinaryOperator::Greater
                    | BinaryOperator::LessEqual
                    | BinaryOperator::GreaterEqual => Class::Bool,
                    _ => Class::F64,
                })
            }
            // --- Bool / Char (i32) ---
            (op_, Class::Bool | Class::Char) => {
                self.i32_op(fb, op_)?;
                Ok(match op_ {
                    BinaryOperator::Equal
                    | BinaryOperator::NotEqual
                    | BinaryOperator::Less
                    | BinaryOperator::Greater
                    | BinaryOperator::LessEqual
                    | BinaryOperator::GreaterEqual
                    | BinaryOperator::LogicalAnd
                    | BinaryOperator::LogicalOr => Class::Bool,
                    _ => Class::I64,
                })
            }
        }
    }

    fn i64_op(&self, fb: &mut FB, op_: BinaryOperator) -> Result<(), CodegenError> {
        let code = match op_ {
            BinaryOperator::Add => op::I64_ADD,
            BinaryOperator::Subtract => op::I64_SUB,
            BinaryOperator::Multiply => op::I64_MUL,
            BinaryOperator::Divide => op::I64_DIV_S,
            BinaryOperator::Modulo => op::I64_REM_S,
            BinaryOperator::BitAnd => op::I64_AND,
            BinaryOperator::BitOr => op::I64_OR,
            BinaryOperator::BitXor => op::I64_XOR,
            BinaryOperator::ShiftLeft => op::I64_SHL,
            BinaryOperator::ShiftRight => op::I64_SHR_S,
            BinaryOperator::Equal => op::I64_EQ,
            BinaryOperator::NotEqual => op::I64_NE,
            BinaryOperator::Less => op::I64_LT_S,
            BinaryOperator::Greater => op::I64_GT_S,
            BinaryOperator::LessEqual => op::I64_LE_S,
            BinaryOperator::GreaterEqual => op::I64_GE_S,
            BinaryOperator::LogicalAnd | BinaryOperator::LogicalOr => {
                return Err(unsupported("logical op on integer", Some(self.fn_id)));
            }
        };
        fb.op(code);
        Ok(())
    }

    fn f64_op(&self, fb: &mut FB, op_: BinaryOperator) -> Result<(), CodegenError> {
        if op_ == BinaryOperator::Modulo {
            // fmod: l - trunc(l / r) * r
            let l = fb.scratch(Val::F64);
            let r = fb.scratch(Val::F64);
            let t = fb.scratch(Val::F64);
            // move [l, r] into scratches (r is on top)
            fb.s(r);
            fb.s(l);
            fb.g(l);
            fb.g(r);
            fb.op(op::F64_DIV);
            fb.op(op::F64_TRUNC);
            fb.g(r);
            fb.op(op::F64_MUL);
            fb.s(t);
            fb.g(l);
            fb.g(t);
            fb.op(op::F64_SUB);
            return Ok(());
        }
        let code = match op_ {
            BinaryOperator::Add => op::F64_ADD,
            BinaryOperator::Subtract => op::F64_SUB,
            BinaryOperator::Multiply => op::F64_MUL,
            BinaryOperator::Divide => op::F64_DIV,
            BinaryOperator::Equal => op::F64_EQ,
            BinaryOperator::NotEqual => op::F64_NE,
            BinaryOperator::Less => op::F64_LT,
            BinaryOperator::Greater => op::F64_GT,
            BinaryOperator::LessEqual => op::F64_LE,
            BinaryOperator::GreaterEqual => op::F64_GE,
            _ => return Err(unsupported("float operator", Some(self.fn_id))),
        };
        fb.op(code);
        Ok(())
    }

    fn i32_op(&self, fb: &mut FB, op_: BinaryOperator) -> Result<(), CodegenError> {
        let code = match op_ {
            BinaryOperator::Equal => op::I32_EQ,
            BinaryOperator::NotEqual => op::I32_NE,
            BinaryOperator::Less => op::I32_LT_S,
            BinaryOperator::Greater => op::I32_GT_S,
            BinaryOperator::LessEqual => op::I32_LE_S,
            BinaryOperator::GreaterEqual => op::I32_GE_S,
            BinaryOperator::LogicalAnd => op::I32_AND,
            BinaryOperator::LogicalOr => op::I32_OR,
            _ => return Err(unsupported("boolean/char operator", Some(self.fn_id))),
        };
        fb.op(code);
        Ok(())
    }

    fn emit_call(
        &self,
        fb: &mut FB,
        callee: Operand,
        arguments: &lpp_mir::ListRange<Operand>,
    ) -> Result<Class, CodegenError> {
        match callee {
            Operand::Function(id) => {
                let callee_fn = self.env.program.function(id).unwrap();
                if matches!(callee_fn.kind, lpp_mir::MirFunctionKind::Async) {
                    // A direct async call constructs a task through the
                    // function's thunk (5D2b slice 3).
                    self.emit_task_construct(fb, None, arguments, callee_fn.return_type, Some(id))
                } else {
                    self.emit_direct_call(fb, id, arguments)
                }
            }
            Operand::Copy(local) => {
                let callee_ty = self.env.program.local(local).unwrap().ty;
                let result = match self.env.types.kind(callee_ty) {
                    TypeKind::Function { result, .. } => result,
                    _ => {
                        return Err(unsupported(
                            "call through a non-function value",
                            Some(self.fn_id),
                        ));
                    }
                };
                if matches!(self.env.types.kind(result), TypeKind::Task(_)) {
                    // A call through an async function value constructs
                    // a task; the capsule's code word is the task
                    // thunk. The result is a Task handle (a managed
                    // pointer) — no unboxing.
                    self.emit_task_construct(fb, Some(local), arguments, result, None)
                } else {
                    self.emit_closure_call(fb, local, arguments)
                }
            }
            _ => Err(unsupported("call through value", Some(self.fn_id))),
        }
    }

    /// Build the task env tuple for `arguments` and the task node
    /// (5D2b slice 3): `tuple(16+8n)` with the argument slots at
    /// `16+8i` (managed arguments retained for the tuple), then
    /// `TaskNew(code, env, managed_flag)`. `capsule` names the local
    /// whose capsule `code` word is the task thunk (a call through an
    /// async function value); otherwise the thunk for `fn_id` is used.
    fn emit_task_construct(
        &self,
        fb: &mut FB,
        capsule: Option<MirLocalId>,
        arguments: &lpp_mir::ListRange<Operand>,
        result_ty: TypeId,
        fn_id: Option<MirFunctionId>,
    ) -> Result<Class, CodegenError> {
        let args = self.env.program.operands(*arguments).to_vec();
        let tuple_slot = self
            .env
            .tuple_drop_slot
            .ok_or_else(|| unsupported("missing tuple destroyer", Some(self.fn_id)))?;
        let mut mask = 0i64;
        let mut packed = 0i64;
        for (i, operand) in args.iter().enumerate() {
            if let Operand::Copy(local) = operand {
                let ty = self.env.program.local(*local).unwrap().ty;
                if is_managed(self.env.types, ty) {
                    mask |= 1i64 << i;
                }
            }
            packed |= (16 + 8 * i as i64) << (16 * i as i64);
        }
        // The tuple: the ARC node the arguments' references land in.
        fb.i32c(16 + 8 * args.len() as i64);
        fb.i32c(tuple_slot as i64);
        self.call_helper(fb, H::ArcAlloc)?;
        let tuple = fb.scratch(Val::I32);
        fb.t(tuple);
        fb.g(tuple).i64c(mask).store64(0);
        fb.g(tuple).i64c(packed).store64(8);
        for (i, operand) in args.iter().enumerate() {
            let offset = 16 + 8 * i as u32;
            let Operand::Copy(local) = operand else {
                return Err(unsupported(
                    "async call with non-local argument",
                    Some(self.fn_id),
                ));
            };
            let ty = self.env.program.local(*local).unwrap().ty;
            if is_managed(self.env.types, ty) {
                // Retain for the tuple (it owns the reference); the
                // tee addresses the store and the value is dropped.
                self.emit_owned(fb, operand)?;
                let slot = fb.scratch(Val::I32);
                fb.t(slot);
                fb.g(tuple).g(slot).op(op::I64_EXTEND_I32_S).store64(offset);
                fb.g(slot).op(op::DROP);
            } else {
                match class_of_type(self.env.types, ty)? {
                    Class::I64 => {
                        self.emit_operand(fb, operand)?;
                        fb.g(tuple).store64(offset);
                    }
                    Class::F64 => {
                        self.emit_operand(fb, operand)?;
                        fb.op(op::I64_REINTERPRET_F64);
                        fb.g(tuple).store64(offset);
                    }
                    Class::Bool | Class::Char => {
                        self.emit_operand(fb, operand)?;
                        fb.op(op::I64_EXTEND_I32_U);
                        fb.g(tuple).store64(offset);
                    }
                    Class::Str => {
                        self.emit_operand(fb, operand)?;
                        fb.op(op::I64_EXTEND_I32_S);
                        fb.g(tuple).store64(offset);
                    }
                    Class::Ptr => unreachable!("checked above"),
                }
            }
        }
        // TaskNew(code, env, managed): the stack takes (code, env,
        // managed) — code first. The tee'd tuple pointer is no longer
        // needed (every use above re-reads the scratch local); drop it.
        fb.op(op::DROP);
        match (capsule, fn_id) {
            (None, Some(fn_id)) => {
                let code = *self
                    .env
                    .task_thunk_dispatch
                    .get(&fn_id)
                    .ok_or_else(|| unsupported("undispatched task thunk", Some(self.fn_id)))?;
                fb.i32c(code as i64);
            }
            (Some(capsule), None) => {
                let index = *self
                    .local_index
                    .get(&capsule)
                    .ok_or_else(|| unsupported("unknown local", Some(self.fn_id)))?;
                fb.g(index).load64(0).op(op::I32_WRAP_I64);
            }
            _ => unreachable!("exactly one code source"),
        }
        fb.g(tuple);
        let managed = if is_managed(self.env.types, result_ty) {
            1
        } else {
            0
        };
        fb.i32c(managed as i64);
        self.call_helper(fb, H::TaskNew)?;
        // TaskNew returns the node pointer as an i64 word; narrow
        // for the wasm ABI.
        fb.op(op::I32_WRAP_I64);
        Ok(Class::Ptr)
    }

    fn emit_direct_call(
        &self,
        fb: &mut FB,
        fn_id: MirFunctionId,
        arguments: &lpp_mir::ListRange<Operand>,
    ) -> Result<Class, CodegenError> {
        let callee_fn = self.env.program.function(fn_id).ok_or_else(|| {
            CodegenError::new(
                Some(self.fn_id),
                CodegenErrorKind::ObjectEmissionFailed("unknown callee".to_string()),
            )
        })?;
        // The dispatch ABI: every function takes the env pointer first
        // (NULL for a direct call — plain functions ignore it).
        fb.i32c(0);
        for arg in self.env.program.operands(*arguments) {
            // A managed argument retains for the callee (released by the
            // callee's exit release pass); the source keeps its own.
            self.emit_owned(fb, arg)?;
        }
        let index = *self
            .env
            .user_index
            .get(&fn_id)
            .ok_or_else(|| unsupported("unknown function", Some(self.fn_id)))?;
        fb.call(index);
        let rt = callee_fn.return_type;
        if is_void(self.env.types, rt) {
            fb.i64c(0);
            Ok(Class::I64)
        } else {
            class_of_type(self.env.types, rt)
        }
    }

    /// A call through a capsule: `Call(Copy(local), args)` where the
    /// local holds a closure capsule. Stack: the capsule, the user
    /// arguments (captures come from the env), then the dispatch index
    /// loaded from the capsule's `code` word; `call_indirect` on the
    /// dispatch table (table 1) pops them in reverse.
    fn emit_closure_call(
        &self,
        fb: &mut FB,
        capsule: MirLocalId,
        arguments: &lpp_mir::ListRange<Operand>,
    ) -> Result<Class, CodegenError> {
        let capsule_ty = self.env.program.local(capsule).unwrap().ty;
        let (parameters, result) = match self.env.types.kind(capsule_ty) {
            TypeKind::Function { parameters, result } => (parameters, result),
            _ => {
                return Err(unsupported(
                    "call through a non-function value",
                    Some(self.fn_id),
                ));
            }
        };
        let index = *self
            .local_index
            .get(&capsule)
            .ok_or_else(|| unsupported("unknown local", Some(self.fn_id)))?;
        // Stack bottom->top: env, user args..., dispatch index.
        // `call_indirect` pops the index, then the parameters in
        // reverse; the env (first parameter) is the capsule's env word.
        fb.g(index).load64(8).op(op::I32_WRAP_I64);
        // The argument operands are the user parameters of the closure
        // type; the count is checked against the type's parameter list.
        let args = self.env.program.operands(*arguments);
        if args.len() != self.env.types.list(parameters).len() {
            return Err(unsupported(
                "closure call argument count mismatch",
                Some(self.fn_id),
            ));
        }
        for arg in args {
            self.emit_owned(fb, arg)?;
        }
        let call_type = *self
            .env
            .closure_call_type
            .get(&capsule_ty)
            .ok_or_else(|| unsupported("unregistered closure call type", Some(self.fn_id)))?;
        fb.g(index) // dispatch index = the capsule's code word, on top
            .load64(0)
            .op(op::I32_WRAP_I64);
        fb.call_indirect(call_type, self.env.dispatch_table);
        if is_void(self.env.types, result) {
            fb.i64c(0);
            Ok(Class::I64)
        } else {
            class_of_type(self.env.types, result)
        }
    }

    /// Builds a closure capsule: an optional env node (one fixed
    /// 8-byte slot per capture) plus the 16-byte capsule node whose
    /// `code` word is the dispatch-table index and whose `env` word is
    /// the env pointer (0 for a zero-capture closure).
    fn emit_make_closure(
        &self,
        fb: &mut FB,
        function: MirFunctionId,
        captures: &lpp_mir::ListRange<Operand>,
    ) -> Result<Class, CodegenError> {
        let closure = self.env.program.function(function).ok_or_else(|| {
            CodegenError::new(
                Some(self.fn_id),
                CodegenErrorKind::ObjectEmissionFailed("unknown closure".to_string()),
            )
        })?;
        // The env slots are the closure frame's capture slots
        // (`Capture`-kind parameters); `captures` operand `i` feeds
        // slot `i`.
        let capture_locals: Vec<MirLocalId> = self
            .env
            .program
            .function_parameters(closure)
            .iter()
            .copied()
            .filter(|&p| {
                matches!(
                    self.env.program.local(p).unwrap().kind,
                    lpp_mir::MirLocalKind::Capture
                )
            })
            .collect();
        let count = capture_locals.len() as u32;

        // The env node owns one reference per managed capture.
        let env_node: u32 = if count == 0 {
            0
        } else {
            let env_dtor =
                *self.env.env_drop_slot.get(&function).ok_or_else(|| {
                    unsupported("unregistered closure env slot", Some(self.fn_id))
                })?;
            let ptr = fb.scratch(Val::I32);
            fb.i32c((8 * count) as i64);
            fb.i32c(env_dtor as i64);
            self.call_helper(fb, H::ArcAlloc)?;
            fb.s(ptr);
            let values = self.env.program.operands(*captures);
            if values.len() != capture_locals.len() {
                return Err(unsupported("closure capture count", Some(self.fn_id)));
            }
            for (i, operand) in values.iter().enumerate() {
                fb.g(ptr);
                let _class = self.emit_owned(fb, operand)?;
                let ty = self.env.program.local(capture_locals[i]).unwrap().ty;
                store_by_type(fb, self.env.types, 8 * i as u32, ty);
            }
            ptr
        };

        // The capsule node.
        let capsule = fb.scratch(Val::I32);
        let destroy_slot = self
            .env
            .closure_destroy_slot
            .ok_or_else(|| unsupported("missing closure destroyer", Some(self.fn_id)))?;
        fb.i32c(16);
        fb.i32c(destroy_slot as i64);
        self.call_helper(fb, H::ArcAlloc)?;
        fb.s(capsule);
        let code = *self
            .env
            .dispatch
            .get(&function)
            .ok_or_else(|| unsupported("undispatched closure", Some(self.fn_id)))?;
        fb.g(capsule).i64c(code as i64).store64(0);
        // The env word is the env node's pointer (a local value, not an
        // index); zero-capture closures have a NULL env.
        if count == 0 {
            fb.g(capsule).i64c(0).store64(8);
        } else {
            fb.g(capsule)
                .g(env_node)
                .op(op::I64_EXTEND_I32_S)
                .store64(8);
        }
        fb.g(capsule);
        Ok(Class::Ptr)
    }

    /// The second argument of a binary builtin (5D2b slice 4).
    fn arg1<'b>(&self, args: &'b [Operand]) -> Result<&'b Operand, CodegenError> {
        args.get(1)
            .ok_or_else(|| unsupported("builtin argument missing", Some(self.fn_id)))
    }

    fn emit_builtin(
        &self,
        fb: &mut FB,
        builtin: BuiltinId,
        arguments: &lpp_mir::ListRange<Operand>,
    ) -> Result<Class, CodegenError> {
        let args = self.env.program.operands(*arguments);
        let name = builtin.descriptor().name;
        let short = name.strip_prefix("lpp_").unwrap_or(name);
        if short == "input" {
            self.call_helper(fb, H::Input)?;
            return Ok(Class::Str);
        }
        let arg0 = args
            .first()
            .ok_or_else(|| unsupported("builtin with no argument", Some(self.fn_id)))?;
        match short {
            "print" => {
                let class = operand_class(self.env.program, self.env.types, arg0)?;
                match class {
                    Class::I64 => {
                        self.emit_operand(fb, arg0)?;
                    }
                    Class::Char => {
                        self.emit_operand(fb, arg0)?;
                        fb.op(op::I64_EXTEND_I32_U);
                    }
                    Class::Bool => {
                        self.emit_operand(fb, arg0)?;
                    }
                    Class::Str => {
                        self.emit_operand(fb, arg0)?;
                    }
                    Class::F64 => {
                        self.emit_operand(fb, arg0)?;
                    }
                    Class::Ptr => {
                        return Err(unsupported("print of managed type", Some(self.fn_id)));
                    }
                }
                let h = match class {
                    Class::I64 | Class::Char => H::PrintInt,
                    Class::Bool => H::PrintBool,
                    Class::Str => H::PrintStr,
                    Class::F64 => H::PrintFloat,
                    Class::Ptr => unreachable!(),
                };
                self.call_helper(fb, h)?;
                fb.i64c(0);
                Ok(Class::I64)
            }
            "print_str" | "eprint_str" => {
                self.emit_operand(fb, arg0)?;
                self.call_helper(fb, H::PrintStr)?;
                fb.i64c(0);
                Ok(Class::I64)
            }
            "print_int" => {
                self.emit_operand(fb, arg0)?;
                self.call_helper(fb, H::PrintInt)?;
                fb.i64c(0);
                Ok(Class::I64)
            }
            "print_bool" => {
                self.emit_operand(fb, arg0)?;
                self.call_helper(fb, H::PrintBool)?;
                fb.i64c(0);
                Ok(Class::I64)
            }
            "print_float" => {
                self.emit_operand(fb, arg0)?;
                self.call_helper(fb, H::PrintFloat)?;
                fb.i64c(0);
                Ok(Class::I64)
            }
            "write_str" => {
                self.emit_operand(fb, arg0)?;
                self.call_helper(fb, H::WriteStr)?;
                fb.i64c(0);
                Ok(Class::I64)
            }
            "str_len" => {
                self.emit_operand(fb, arg0)?;
                self.call_helper(fb, H::StrLen)?;
                Ok(Class::I64)
            }
            "list_new" => {
                // The element class comes from the assign target
                // (`emit_assign` routes here); the helper's is-ARC word
                // tells the list destroyer whether to release elements.
                Err(unsupported("list_new outside an assign", Some(self.fn_id)))
            }
            "list_push" | "list_set" => {
                let value = args.get(1).ok_or_else(|| {
                    unsupported("list element argument missing", Some(self.fn_id))
                })?;
                let element = self.list_element_of(arg0)?;
                let is_push = short == "list_push";
                // The list itself is a plain read (the helper never takes
                // ownership of the container).
                self.emit_operand(fb, arg0)?;
                if !is_push {
                    let index = args.get(1).ok_or_else(|| {
                        unsupported("list index argument missing", Some(self.fn_id))
                    })?;
                    let value = args.get(2).ok_or_else(|| {
                        unsupported("list element argument missing", Some(self.fn_id))
                    })?;
                    self.emit_operand(fb, index)?;
                    match element_class(self.env.types, element) {
                        ElementClass::I64 => {
                            self.emit_operand(fb, value)?;
                            widen_to_i64(fb, self.env.types, element);
                            self.call_helper(fb, H::ListSetI64)?;
                        }
                        ElementClass::F64 => {
                            self.emit_operand(fb, value)?;
                            self.call_helper(fb, H::ListSetF64)?;
                        }
                        ElementClass::Ptr => {
                            // The helper releases the replaced element
                            // and retains the new one (runtime-internal).
                            self.emit_operand(fb, value)?;
                            self.call_helper(fb, H::ListSetPtr)?;
                        }
                    }
                } else {
                    match element_class(self.env.types, element) {
                        ElementClass::I64 => {
                            // The element transfers into the list (the
                            // list owns the new reference).
                            self.emit_owned(fb, value)?;
                            widen_to_i64(fb, self.env.types, element);
                            self.call_helper(fb, H::ListPushI64)?;
                        }
                        ElementClass::F64 => {
                            self.emit_owned(fb, value)?;
                            self.call_helper(fb, H::ListPushF64)?;
                        }
                        ElementClass::Ptr => {
                            self.emit_owned(fb, value)?;
                            self.call_helper(fb, H::ListPushPtr)?;
                        }
                    }
                }
                fb.i64c(0);
                Ok(Class::I64)
            }
            "list_get" => {
                let element = self.list_element_of(arg0)?;
                self.emit_operand(fb, arg0)?;
                self.emit_operand(
                    fb,
                    args.get(1).ok_or_else(|| {
                        unsupported("list index argument missing", Some(self.fn_id))
                    })?,
                )?;
                match element_class(self.env.types, element) {
                    ElementClass::I64 => {
                        self.call_helper(fb, H::ListGetI64)?;
                        Ok(Class::I64)
                    }
                    ElementClass::F64 => {
                        self.call_helper(fb, H::ListGetF64)?;
                        Ok(Class::F64)
                    }
                    ElementClass::Ptr => {
                        self.call_helper(fb, H::ListGetPtr)?;
                        // An element read gains a reference (the
                        // container keeps its own).
                        self.call_helper(fb, H::Retain)?;
                        Ok(Class::Ptr)
                    }
                }
            }
            "list_len" => {
                self.list_element_of(arg0)?;
                self.emit_operand(fb, arg0)?;
                self.call_helper(fb, H::ListLen)?;
                Ok(Class::I64)
            }
            "slice" | "str_slice" => {
                self.emit_operand(fb, arg0)?;
                self.emit_operand(
                    fb,
                    args.get(1).ok_or_else(|| {
                        unsupported("slice start argument missing", Some(self.fn_id))
                    })?,
                )?;
                self.emit_operand(
                    fb,
                    args.get(2).ok_or_else(|| {
                        unsupported("slice length argument missing", Some(self.fn_id))
                    })?,
                )?;
                self.call_helper(
                    fb,
                    if short == "slice" {
                        H::SliceNewList
                    } else {
                        H::SliceNewStr
                    },
                )?;
                Ok(Class::Ptr)
            }
            "slice_len" => {
                slice_kind_of(self.env.program, self.env.types, arg0, self.fn_id)?;
                self.emit_operand(fb, arg0)?;
                self.call_helper(fb, H::SliceLen)?;
                Ok(Class::I64)
            }
            "slice_get" => {
                let kind = slice_kind_of(self.env.program, self.env.types, arg0, self.fn_id)?;
                self.emit_operand(fb, arg0)?;
                self.emit_operand(
                    fb,
                    args.get(1).ok_or_else(|| {
                        unsupported("slice index argument missing", Some(self.fn_id))
                    })?,
                )?;
                match kind {
                    SliceKind::String => {
                        self.call_helper(fb, H::StrSliceGet)?;
                        Ok(Class::Str)
                    }
                    SliceKind::List(element) => match class_of_type(self.env.types, element)? {
                        Class::I64 => {
                            self.call_helper(fb, H::SliceGetI64)?;
                            Ok(Class::I64)
                        }
                        Class::Bool => {
                            self.call_helper(fb, H::SliceGetI64)?;
                            fb.op(op::I32_WRAP_I64);
                            Ok(Class::Bool)
                        }
                        Class::Char => {
                            self.call_helper(fb, H::SliceGetI64)?;
                            fb.op(op::I32_WRAP_I64);
                            Ok(Class::Char)
                        }
                        Class::F64 => {
                            self.call_helper(fb, H::SliceGetF64)?;
                            Ok(Class::F64)
                        }
                        Class::Str => {
                            self.call_helper(fb, H::SliceGetPtr)?;
                            self.call_helper(fb, H::Retain)?;
                            Ok(Class::Str)
                        }
                        Class::Ptr => {
                            self.call_helper(fb, H::SliceGetPtr)?;
                            self.call_helper(fb, H::Retain)?;
                            Ok(Class::Ptr)
                        }
                    },
                }
            }
            "slice_get_bool" => {
                let SliceKind::List(element) =
                    slice_kind_of(self.env.program, self.env.types, arg0, self.fn_id)?
                else {
                    return Err(unsupported(
                        "slice_get_bool requires a list slice",
                        Some(self.fn_id),
                    ));
                };
                if !matches!(
                    self.env.types.kind(element),
                    TypeKind::Primitive(PrimitiveType::Bool)
                ) {
                    return Err(unsupported(
                        "slice_get_bool requires boolean elements",
                        Some(self.fn_id),
                    ));
                }
                self.emit_operand(fb, arg0)?;
                self.emit_operand(
                    fb,
                    args.get(1).ok_or_else(|| {
                        unsupported("slice index argument missing", Some(self.fn_id))
                    })?,
                )?;
                self.call_helper(fb, H::SliceGetI64)?;
                fb.op(op::I32_WRAP_I64);
                Ok(Class::Bool)
            }
            "slice_to_str" | "str_slice_to_str" => {
                if slice_kind_of(self.env.program, self.env.types, arg0, self.fn_id)?
                    != SliceKind::String
                {
                    return Err(unsupported(
                        "slice_to_str requires a string slice",
                        Some(self.fn_id),
                    ));
                }
                self.emit_operand(fb, arg0)?;
                self.call_helper(fb, H::StrSliceToStr)?;
                Ok(Class::Str)
            }
            "map_put" | "map_put_str" | "map_put_float" | "map_put_str_float" | "map_get"
            | "map_get_str" | "map_get_float" | "map_get_str_float" | "map_has" | "map_has_str"
            | "map_remove" | "map_remove_str" => {
                let string_keys = short.contains("_str");
                let float_value = short.ends_with("_float");
                let is_put = short.starts_with("map_put");
                let is_get = short.starts_with("map_get");
                let is_has = short.starts_with("map_has");
                self.emit_operand(fb, arg0)?;
                fb.op(op::I32_WRAP_I64);
                self.emit_operand(
                    fb,
                    args.get(1)
                        .ok_or_else(|| unsupported("map key argument missing", Some(self.fn_id)))?,
                )?;
                let helper = match (is_put, is_get, is_has, string_keys) {
                    (true, _, _, false) => H::MapPutI64,
                    (true, _, _, true) => H::MapPutStr,
                    (_, true, _, false) => H::MapGetI64,
                    (_, true, _, true) => H::MapGetStr,
                    (_, _, true, false) => H::MapHasI64,
                    (_, _, true, true) => H::MapHasStr,
                    (_, _, _, false) => H::MapRemoveI64,
                    (_, _, _, true) => H::MapRemoveStr,
                };
                if is_put {
                    self.emit_operand(
                        fb,
                        args.get(2).ok_or_else(|| {
                            unsupported("map value argument missing", Some(self.fn_id))
                        })?,
                    )?;
                    if float_value {
                        fb.op(op::I64_REINTERPRET_F64);
                    }
                }
                self.call_helper(fb, helper)?;
                if is_has {
                    fb.op(op::I32_WRAP_I64);
                    Ok(Class::Bool)
                } else if is_get && float_value {
                    fb.op(op::F64_REINTERPRET_I64);
                    Ok(Class::F64)
                } else {
                    if is_put || short.starts_with("map_remove") {
                        fb.i64c(0);
                    }
                    Ok(Class::I64)
                }
            }
            "map_len" => {
                self.emit_operand(fb, arg0)?;
                fb.op(op::I32_WRAP_I64);
                self.call_helper(fb, H::MapLen)?;
                Ok(Class::I64)
            }
            // ── 5D2b slice 4, batch 1: integer + float builtins ──────
            "abs" => {
                // No single-op i64.abs in the core spec: the mask
                // trick — m = x >> 63 (all 0s or all 1s), and
                // abs(x) = (x ^ m) - m.
                let m = fb.scratch(Val::I64);
                self.emit_operand(fb, arg0)?;
                fb.t(m);
                fb.g(m).i64c(63).op(op::I64_SHR_S).s(m);
                fb.g(m).op(op::I64_XOR).g(m).op(op::I64_SUB);
                Ok(Class::I64)
            }
            "clz64" | "ctz64" | "popcount64" => {
                self.emit_operand(fb, arg0)?;
                fb.op(match short {
                    "clz64" => op::I64_CLZ,
                    "ctz64" => op::I64_CTZ,
                    _ => op::I64_POPCNT,
                });
                Ok(Class::I64)
            }
            "bswap64" => {
                // No i64.bswap opcode: reverse the 8 byte lanes and
                // reassemble.
                let x = fb.scratch(Val::I64);
                self.emit_operand(fb, arg0)?;
                fb.s(x);
                fb.g(x)
                    .i64c(0x00000000000000FF)
                    .op(op::I64_AND)
                    .i64c(56)
                    .op(op::I64_SHL);
                fb.g(x)
                    .i64c(0x000000000000FF00)
                    .op(op::I64_AND)
                    .i64c(40)
                    .op(op::I64_SHL)
                    .op(op::I64_OR);
                fb.g(x)
                    .i64c(0x0000000000FF0000)
                    .op(op::I64_AND)
                    .i64c(24)
                    .op(op::I64_SHL)
                    .op(op::I64_OR);
                fb.g(x)
                    .i64c(0x00000000FF000000)
                    .op(op::I64_AND)
                    .i64c(8)
                    .op(op::I64_SHL)
                    .op(op::I64_OR);
                fb.g(x)
                    .i64c(0x000000FF00000000)
                    .op(op::I64_AND)
                    .i64c(8)
                    .op(op::I64_SHR_U)
                    .op(op::I64_OR);
                fb.g(x)
                    .i64c(0x0000FF0000000000)
                    .op(op::I64_AND)
                    .i64c(24)
                    .op(op::I64_SHR_U)
                    .op(op::I64_OR);
                fb.g(x)
                    .i64c(0x00FF000000000000)
                    .op(op::I64_AND)
                    .i64c(40)
                    .op(op::I64_SHR_U)
                    .op(op::I64_OR);
                fb.g(x)
                    .i64c(0xFF00000000000000u64 as i64)
                    .op(op::I64_AND)
                    .i64c(56)
                    .op(op::I64_SHR_U)
                    .op(op::I64_OR);
                Ok(Class::I64)
            }
            "bswap32" => {
                // Zero-extended 32-bit result (the oracle: u32 as i64).
                let x = fb.scratch(Val::I64);
                self.emit_operand(fb, arg0)?;
                fb.s(x);
                fb.g(x).i64c(0xFF).op(op::I64_AND).i64c(24).op(op::I64_SHL);
                fb.g(x)
                    .i64c(0xFF00)
                    .op(op::I64_AND)
                    .i64c(8)
                    .op(op::I64_SHL)
                    .op(op::I64_OR);
                fb.g(x)
                    .i64c(0xFF0000)
                    .op(op::I64_AND)
                    .i64c(8)
                    .op(op::I64_SHR_U)
                    .op(op::I64_OR);
                fb.g(x)
                    .i64c(0xFF000000)
                    .op(op::I64_AND)
                    .i64c(24)
                    .op(op::I64_SHR_U)
                    .op(op::I64_OR);
                Ok(Class::I64)
            }
            "bswap16" => {
                let x = fb.scratch(Val::I64);
                self.emit_operand(fb, arg0)?;
                fb.s(x);
                fb.g(x).i64c(0xFF).op(op::I64_AND).i64c(8).op(op::I64_SHL);
                fb.g(x)
                    .i64c(0xFF00)
                    .op(op::I64_AND)
                    .i64c(8)
                    .op(op::I64_SHR_U)
                    .op(op::I64_OR);
                Ok(Class::I64)
            }
            "trunc_u8" | "trunc_u16" | "trunc_u32" => {
                self.emit_operand(fb, arg0)?;
                fb.i64c(match short {
                    "trunc_u8" => 0xFF,
                    "trunc_u16" => 0xFFFF,
                    _ => 0xFFFFFFFF,
                })
                .op(op::I64_AND);
                Ok(Class::I64)
            }
            "trunc_i8" | "trunc_i16" | "trunc_i32" => {
                let shift = match short {
                    "trunc_i8" => 56,
                    "trunc_i16" => 48,
                    _ => 32,
                };
                self.emit_operand(fb, arg0)?;
                fb.i64c(shift).op(op::I64_SHL).i64c(shift).op(op::I64_SHR_S);
                Ok(Class::I64)
            }
            "min" | "max" | "min_u" | "max_u" => {
                let a = fb.scratch(Val::I64);
                let b = fb.scratch(Val::I64);
                let arg1 = self.arg1(args)?;
                self.emit_operand(fb, arg0)?;
                fb.t(a);
                self.emit_operand(fb, arg1)?;
                fb.t(b);
                fb.g(a).g(b).op(match short {
                    "min" => op::I64_LT_S,
                    "max" => op::I64_GT_S,
                    "min_u" => op::I64_LT_U,
                    _ => op::I64_GT_U,
                });
                fb.op(op::SELECT);
                Ok(Class::I64)
            }
            "lt_u" | "le_u" | "gt_u" | "ge_u" => {
                let arg1 = self.arg1(args)?;
                self.emit_operand(fb, arg0)?;
                self.emit_operand(fb, arg1)?;
                fb.op(match short {
                    "lt_u" => op::I64_LT_U,
                    "le_u" => op::I64_LE_U,
                    "gt_u" => op::I64_GT_U,
                    _ => op::I64_GE_U,
                });
                // The frozen ABI spells these predicates as i64 0/1
                // (the interpreter follows that contract). Materialize the
                // wasm comparison's i32 result into the ABI word so stores,
                // equality checks, and returns keep the same type.
                fb.op(op::I64_EXTEND_I32_U);
                Ok(Class::I64)
            }
            "div_u" | "rem_u" => {
                let arg1 = self.arg1(args)?;
                self.emit_operand(fb, arg0)?;
                self.emit_operand(fb, arg1)?;
                fb.op(if short == "div_u" {
                    op::I64_DIV_U
                } else {
                    op::I64_REM_U
                });
                Ok(Class::I64)
            }
            "shr_u" | "shl_u" => {
                // The oracle returns 0 for shifts outside [0, 64);
                // wasm masks the amount to 6 bits, so guard it.
                let amt = fb.scratch(Val::I64);
                let out = fb.scratch(Val::I64);
                let arg1 = self.arg1(args)?;
                self.emit_operand(fb, arg0)?;
                self.emit_operand(fb, arg1)?;
                fb.s(amt);
                fb.g(amt)
                    .i64c(63)
                    .op(op::I64_AND)
                    .op(if short == "shr_u" {
                        op::I64_SHR_U
                    } else {
                        op::I64_SHL
                    })
                    .s(out);
                fb.i64c(0);
                fb.g(out);
                fb.g(amt).i64c(0).op(op::I64_LT_S);
                fb.g(amt).i64c(64).op(op::I64_GE_S);
                fb.op(op::I32_OR);
                fb.op(op::SELECT);
                Ok(Class::I64)
            }
            "rotl64" | "rotr64" => {
                // The value goes on the bottom, the amount on top
                // (masked to 6 bits by the instruction — the oracle
                // masks with 63, same effect).
                let arg1 = self.arg1(args)?;
                self.emit_operand(fb, arg0)?;
                self.emit_operand(fb, arg1)?;
                fb.op(if short == "rotl64" {
                    op::I64_ROTL
                } else {
                    op::I64_ROTR
                });
                Ok(Class::I64)
            }
            "rotl32" | "rotr32" => {
                // Zero-extended 32-bit result (the oracle: u32 as i64);
                // value on the bottom, amount on top.
                let arg1 = self.arg1(args)?;
                self.emit_operand(fb, arg0)?;
                fb.op(op::I32_WRAP_I64);
                self.emit_operand(fb, arg1)?;
                fb.op(op::I32_WRAP_I64);
                fb.op(if short == "rotl32" {
                    op::I32_ROTL
                } else {
                    op::I32_ROTR
                });
                fb.op(op::I64_EXTEND_I32_U);
                Ok(Class::I64)
            }
            "add_wrap" | "sub_wrap" | "mul_wrap" => {
                let arg1 = self.arg1(args)?;
                self.emit_operand(fb, arg0)?;
                self.emit_operand(fb, arg1)?;
                fb.op(match short {
                    "add_wrap" => op::I64_ADD,
                    "sub_wrap" => op::I64_SUB,
                    _ => op::I64_MUL,
                });
                Ok(Class::I64)
            }
            "add_checked" => {
                let a = fb.scratch(Val::I64);
                let b = fb.scratch(Val::I64);
                let r = fb.scratch(Val::I64);
                let arg1 = self.arg1(args)?;
                self.emit_operand(fb, arg0)?;
                fb.s(a);
                self.emit_operand(fb, arg1)?;
                fb.s(b);
                fb.g(a).g(b).op(op::I64_ADD).s(r);
                // Overflow iff sign(a) == sign(b) != sign(r):
                // ((a & b) & ~r) < 0.
                fb.g(a)
                    .g(b)
                    .op(op::I64_AND)
                    .g(r)
                    .i64c(-1)
                    .op(op::I64_XOR)
                    .op(op::I64_AND)
                    .i64c(0)
                    .op(op::I64_LT_S);
                fb.if_();
                fb.op(op::UNREACHABLE);
                fb.end();
                fb.g(r);
                Ok(Class::I64)
            }
            "sub_checked" => {
                let a = fb.scratch(Val::I64);
                let b = fb.scratch(Val::I64);
                let r = fb.scratch(Val::I64);
                let arg1 = self.arg1(args)?;
                self.emit_operand(fb, arg0)?;
                fb.s(a);
                self.emit_operand(fb, arg1)?;
                fb.s(b);
                fb.g(a).g(b).op(op::I64_SUB).s(r);
                // Overflow iff sign(a) != sign(b) and
                // sign(r) != sign(a).
                fb.g(a)
                    .g(b)
                    .op(op::I64_XOR)
                    .i64c(0)
                    .op(op::I64_LT_S)
                    .g(a)
                    .g(r)
                    .op(op::I64_XOR)
                    .i64c(0)
                    .op(op::I64_LT_S);
                fb.op(op::I32_AND);
                fb.if_();
                fb.op(op::UNREACHABLE);
                fb.end();
                fb.g(r);
                Ok(Class::I64)
            }
            "mul_checked" => {
                let a = fb.scratch(Val::I64);
                let b = fb.scratch(Val::I64);
                let r = fb.scratch(Val::I64);
                let arg1 = self.arg1(args)?;
                self.emit_operand(fb, arg0)?;
                fb.s(a);
                self.emit_operand(fb, arg1)?;
                fb.s(b);
                fb.g(a).g(b).op(op::I64_MUL).s(r);
                // If neither operand is zero, the product is valid iff
                // r / a == b (the division only runs when a != 0).
                fb.g(a).i64c(0).op(op::I64_NE);
                fb.g(b).i64c(0).op(op::I64_NE);
                fb.op(op::I32_AND);
                fb.if_();
                fb.g(r).g(a).op(op::I64_DIV_S).g(b).op(op::I64_NE);
                fb.if_();
                fb.op(op::UNREACHABLE);
                fb.end();
                fb.end();
                fb.g(r);
                Ok(Class::I64)
            }
            "int_pow" => {
                let base = fb.scratch(Val::I64);
                let exp = fb.scratch(Val::I64);
                let result = fb.scratch(Val::I64);
                self.emit_operand(fb, arg0)?;
                fb.s(base);
                self.emit_operand(fb, self.arg1(args)?)?;
                fb.s(exp);
                // Negative integer exponents are unrepresentable and yield 0.
                fb.g(exp).i64c(0).op(op::I64_LT_S).if_();
                fb.i64c(0).s(result);
                fb.else_();
                fb.i64c(1).s(result);
                fb.block();
                fb.loop_();
                fb.g(exp).op(op::I64_EQZ).br_if(1);
                fb.g(exp).i64c(1).op(op::I64_AND).op(op::I32_WRAP_I64).if_();
                fb.g(result).g(base).op(op::I64_MUL).s(result);
                fb.end();
                fb.g(base).g(base).op(op::I64_MUL).s(base);
                fb.g(exp).i64c(1).op(op::I64_SHR_U).s(exp);
                fb.br(0);
                fb.end();
                fb.end();
                fb.end();
                fb.g(result);
                Ok(Class::I64)
            }
            "ceil" | "floor" | "sqrt" => {
                self.emit_operand(fb, arg0)?;
                fb.op(match short {
                    "ceil" => op::F64_CEIL,
                    "floor" => op::F64_FLOOR,
                    _ => op::F64_SQRT,
                });
                Ok(Class::F64)
            }
            "pow" => {
                self.emit_operand(fb, arg0)?;
                self.emit_operand(fb, self.arg1(args)?)?;
                self.call_helper(fb, H::Pow)?;
                Ok(Class::F64)
            }
            "sin" | "cos" => {
                self.emit_operand(fb, arg0)?;
                self.call_helper(fb, if short == "sin" { H::Sin } else { H::Cos })?;
                Ok(Class::F64)
            }
            "fmod" => {
                // No f64.fmod opcode: the oracle's exact formula,
                // a - floor(a / b) * b.
                let a = fb.scratch(Val::F64);
                let b = fb.scratch(Val::F64);
                let q = fb.scratch(Val::F64);
                let arg1 = self.arg1(args)?;
                self.emit_operand(fb, arg0)?;
                fb.s(a);
                self.emit_operand(fb, arg1)?;
                fb.s(b);
                fb.g(a)
                    .g(b)
                    .op(op::F64_DIV)
                    .op(op::F64_FLOOR)
                    .g(b)
                    .op(op::F64_MUL)
                    .s(q);
                fb.g(a).g(q).op(op::F64_SUB);
                Ok(Class::F64)
            }
            // ── string construction, indexing, and splitting ──
            "str_substr" => {
                self.emit_operand(fb, arg0)?;
                self.emit_operand(fb, self.arg1(args)?)?;
                self.emit_operand(
                    fb,
                    args.get(2)
                        .ok_or_else(|| unsupported("builtin argument missing", Some(self.fn_id)))?,
                )?;
                self.call_helper(fb, H::StrSubstr)?;
                Ok(Class::Str)
            }
            "str_repeat" | "char_at" => {
                self.emit_operand(fb, arg0)?;
                self.emit_operand(fb, self.arg1(args)?)?;
                self.call_helper(
                    fb,
                    if short == "str_repeat" {
                        H::StrRepeat
                    } else {
                        H::CharAt
                    },
                )?;
                Ok(Class::Str)
            }
            "str_split" => {
                self.emit_operand(fb, arg0)?;
                self.emit_operand(fb, self.arg1(args)?)?;
                self.call_helper(fb, H::StrSplit)?;
                Ok(Class::Ptr)
            }
            "ord" => {
                self.emit_operand(fb, arg0)?;
                self.call_helper(fb, H::Ord)?;
                Ok(Class::I64)
            }
            "chr" => {
                self.emit_operand(fb, arg0)?;
                self.call_helper(fb, H::Chr)?;
                Ok(Class::Str)
            }
            "str_eq" => {
                self.emit_operand(fb, arg0)?;
                self.emit_operand(fb, self.arg1(args)?)?;
                self.call_helper(fb, H::StrEq)?;
                fb.op(op::I64_EXTEND_I32_U);
                Ok(Class::I64)
            }
            // ── 5D2b slice 4, batch 3: string builtins ──
            "str_concat" | "str_replace" | "str_contains" | "str_starts_with" | "str_ends_with"
            | "str_find" | "str_trim" | "str_to_lower" | "str_lower" | "str_to_upper"
            | "str_upper" | "int_to_str" | "str_to_int" | "float_to_str" | "bool_to_str"
            | "u64_to_str" | "u64_to_hex" | "str_to_u64" => match short {
                "str_concat" => {
                    let arg1 = self.arg1(args)?;
                    self.emit_operand(fb, arg0)?;
                    self.emit_operand(fb, arg1)?;
                    self.call_helper(fb, H::StrConcat)?;
                    Ok(Class::Ptr)
                }
                "str_replace" => {
                    let arg1 = self.arg1(args)?;
                    let arg2 = args
                        .get(2)
                        .ok_or_else(|| unsupported("builtin argument missing", Some(self.fn_id)))?;
                    self.emit_operand(fb, arg0)?;
                    self.emit_operand(fb, arg1)?;
                    self.emit_operand(fb, arg2)?;
                    self.call_helper(fb, H::StrReplace)?;
                    Ok(Class::Ptr)
                }
                "str_contains" | "str_starts_with" | "str_ends_with" | "str_find" => {
                    let arg1 = self.arg1(args)?;
                    self.emit_operand(fb, arg0)?;
                    self.emit_operand(fb, arg1)?;
                    match short {
                        "str_contains" => {
                            self.call_helper(fb, H::StrContains)?;
                            Ok(Class::Bool)
                        }
                        "str_starts_with" => {
                            self.call_helper(fb, H::StrStartsWith)?;
                            Ok(Class::Bool)
                        }
                        "str_ends_with" => {
                            self.call_helper(fb, H::StrEndsWith)?;
                            Ok(Class::Bool)
                        }
                        _ => {
                            self.call_helper(fb, H::StrFind)?;
                            Ok(Class::I64)
                        }
                    }
                }
                "str_trim" | "str_to_lower" | "str_lower" | "str_to_upper" | "str_upper"
                | "int_to_str" | "str_to_int" | "float_to_str" | "bool_to_str" | "u64_to_str"
                | "u64_to_hex" | "str_to_u64" => {
                    self.emit_operand(fb, arg0)?;
                    match short {
                        "str_trim" => {
                            self.call_helper(fb, H::StrTrim)?;
                            Ok(Class::Ptr)
                        }
                        "str_to_lower" | "str_lower" => {
                            self.call_helper(fb, H::StrLower)?;
                            Ok(Class::Ptr)
                        }
                        "str_to_upper" | "str_upper" => {
                            self.call_helper(fb, H::StrUpper)?;
                            Ok(Class::Ptr)
                        }
                        "int_to_str" => {
                            self.call_helper(fb, H::IntToStr)?;
                            Ok(Class::Ptr)
                        }
                        "str_to_int" => {
                            self.call_helper(fb, H::StrToInt)?;
                            Ok(Class::I64)
                        }
                        "float_to_str" => {
                            self.call_helper(fb, H::FloatToStr)?;
                            Ok(Class::Ptr)
                        }
                        "bool_to_str" => {
                            self.call_helper(fb, H::BoolToStr)?;
                            Ok(Class::Ptr)
                        }
                        "u64_to_str" => {
                            self.call_helper(fb, H::U64ToStr)?;
                            Ok(Class::Ptr)
                        }
                        "u64_to_hex" => {
                            self.call_helper(fb, H::U64ToHex)?;
                            Ok(Class::Ptr)
                        }
                        _ => {
                            self.call_helper(fb, H::StrToU64)?;
                            Ok(Class::I64)
                        }
                    }
                }
                _ => {
                    unreachable!("string builtin guarded by the outer match")
                }
            },
            _ => Err(unrepresentable(
                builtin,
                "not representable on wasm32-wasip1",
                self.fn_id,
            )),
        }
    }

    /// The element type of a list operand (a `Copy` of a list local).
    fn list_element_of(&self, operand: &Operand) -> Result<TypeId, CodegenError> {
        let Operand::Copy(local) = operand else {
            return Err(unsupported(
                "list argument must be a local",
                Some(self.fn_id),
            ));
        };
        let ty = self.env.program.local(*local).unwrap().ty;
        match self.env.types.kind(ty) {
            TypeKind::List(element) => Ok(element),
            _ => Err(unsupported("list argument expected", Some(self.fn_id))),
        }
    }

    /// `list_new` with the target's element class (the is-ARC word).
    fn emit_list_new(&self, fb: &mut FB, target_ty: TypeId) -> Result<Class, CodegenError> {
        let TypeKind::List(element) = self.env.types.kind(target_ty) else {
            return Err(unsupported(
                "list_new outside a list target",
                Some(self.fn_id),
            ));
        };
        fb.i32c(
            if element_class(self.env.types, element) == ElementClass::Ptr {
                1
            } else {
                0
            },
        );
        self.call_helper(fb, H::ListNew)?;
        Ok(Class::Ptr)
    }

    fn emit_map_new(&self, fb: &mut FB, target_ty: TypeId) -> Result<Class, CodegenError> {
        if !matches!(
            self.env.types.kind(target_ty),
            TypeKind::Primitive(PrimitiveType::Int)
        ) {
            return Err(unsupported(
                "map_new outside its opaque Int handle target",
                Some(self.fn_id),
            ));
        }
        fb.i32c(0);
        self.call_helper(fb, H::MapNew)?;
        fb.op(op::I64_EXTEND_I32_U);
        Ok(Class::I64)
    }

    fn emit_terminator(
        &self,
        fb: &mut FB,
        terminator: &Terminator,
        positions: &HashMap<BasicBlockId, u32>,
    ) -> Result<(), CodegenError> {
        match terminator {
            Terminator::Goto(target) => {
                fb.i32c(positions[target] as i64);
                fb.s(self.cur_local);
            }
            Terminator::Branch {
                condition,
                then_block,
                else_block,
            } => {
                match self.emit_operand(fb, condition)? {
                    Class::Bool => {}
                    Class::I64 => {
                        // L++ permits integer truthiness. WebAssembly `if`
                        // needs an i32 condition, so lower `x` to `x != 0`.
                        fb.op(op::I64_EQZ);
                        fb.op(op::I32_EQZ);
                    }
                    _ => {
                        return Err(unsupported("branch condition type", Some(self.fn_id)));
                    }
                }
                fb.if_();
                fb.i32c(positions[then_block] as i64);
                fb.s(self.cur_local);
                fb.else_();
                fb.i32c(positions[else_block] as i64);
                fb.s(self.cur_local);
                fb.end();
            }
            Terminator::Return(operand) => {
                let function = self.env.program.function(self.fn_id).unwrap();
                if let Some(op) = operand {
                    // A plain read: the return retains for the caller
                    // below, and the exit pass releases the slot's own
                    // reference — not the caller's.
                    self.emit_operand(fb, op)?;
                    if is_managed(self.env.types, function.return_type) {
                        self.call_helper(fb, H::Retain)?;
                    }
                }
                self.exit_release_pass(fb)?;
                fb.op(op::RETURN);
            }
            Terminator::Unreachable => {
                // Mirrors the 5C lowering: a trap ends the process; the
                // unreleased references are reclaimed with the memory.
                fb.op(op::UNREACHABLE);
            }
            Terminator::SwitchEnum {
                subject, targets, ..
            } => {
                // The subject is a plain read (no retain — the
                // interpreter's rule); the tag is the i32 at offset 0.
                // The load consumes the subject pointer, so no tee.
                self.emit_operand(fb, subject)?;
                let tag = fb.scratch(Val::I32);
                fb.load32(ENUM_TAG as u32);
                fb.s(tag);
                let targets: &[BasicBlockId] = self.env.program.switch_targets(*targets);
                // `if0 {…} else { if1 {…} else { if2 {…} } }`: n arms open
                // n `if`s (n−1 `else`s), so n `end`s close the chain.
                for (i, &target) in targets.iter().enumerate() {
                    fb.g(tag);
                    fb.i32c(i as i64);
                    fb.op(op::I32_EQ);
                    fb.if_();
                    fb.i32c(positions[&target] as i64);
                    fb.s(self.cur_local);
                    if i + 1 < targets.len() {
                        fb.else_();
                    }
                }
                for _ in 0..targets.len() {
                    fb.end();
                }
            }
        }
        Ok(())
    }

    /// At every `Return`/trap: every managed local that still owns a
    /// reference releases it, in declaration order. Transfers never
    /// null their source, so the pass is unconditional — each retain
    /// balances against exactly one release (overwrite or exit).
    fn exit_release_pass(&self, fb: &mut FB) -> Result<(), CodegenError> {
        let function = self.env.program.function(self.fn_id).unwrap();
        for &local in self.env.program.function_locals(function) {
            let descriptor = self.env.program.local(local).unwrap();
            // 5D2b: the closure frame's capture slots are views of the
            // env (the env slot owns the reference) — releasing them
            // here would double-free at env destruction.
            if matches!(descriptor.kind, lpp_mir::MirLocalKind::Capture) {
                continue;
            }
            let ty = descriptor.ty;
            if is_managed(self.env.types, ty) {
                let index = *self
                    .local_index
                    .get(&local)
                    .ok_or_else(|| unsupported("unknown local", Some(self.fn_id)))?;
                fb.g(index);
                self.call_helper(fb, H::Release)?;
            }
        }
        Ok(())
    }

    fn call_helper(&self, fb: &mut FB, h: H) -> Result<(), CodegenError> {
        let index = *self.env.helper_index.get(&h).ok_or_else(|| {
            CodegenError::new(
                Some(self.fn_id),
                CodegenErrorKind::ObjectEmissionFailed(format!("helper not registered: {h:?}")),
            )
        })?;
        fb.call(index);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Block layout: reverse post-order (deterministic; any order is correct for
// the dispatch loop, RPO just keeps forward edges local).
// ---------------------------------------------------------------------------

fn block_layout(
    program: &MirProgram,
    function: &lpp_mir::MirFunction,
) -> (Vec<(u32, BasicBlockId)>, HashMap<BasicBlockId, u32>) {
    let entry = function.entry;
    let mut seen = HashSet::new();
    let mut stack: Vec<(BasicBlockId, bool)> = vec![(entry, false)];
    let mut post: Vec<BasicBlockId> = Vec::new();
    while let Some((blk, processed)) = stack.pop() {
        if processed {
            post.push(blk);
        } else if seen.insert(blk) {
            stack.push((blk, true));
            if let Some(block) = program.block(blk) {
                for succ in successors(&block.terminator, program) {
                    if !seen.contains(&succ) {
                        stack.push((succ, false));
                    }
                }
            }
        }
    }
    post.reverse();
    let order = post
        .iter()
        .enumerate()
        .map(|(i, &b)| (i as u32, b))
        .collect();
    let positions: HashMap<BasicBlockId, u32> = post
        .iter()
        .enumerate()
        .map(|(i, &b)| (b, i as u32))
        .collect();
    (order, positions)
}

fn successors(terminator: &Terminator, program: &MirProgram) -> Vec<BasicBlockId> {
    match terminator {
        Terminator::Goto(t) => vec![*t],
        Terminator::Branch {
            then_block,
            else_block,
            ..
        } => vec![*then_block, *else_block],
        Terminator::SwitchEnum { targets, .. } => program.switch_targets(*targets).to_vec(),
        Terminator::Return(_) | Terminator::Unreachable => vec![],
    }
}

// ---------------------------------------------------------------------------
// Runtime helpers.
// ---------------------------------------------------------------------------

/// Box a call result of user type `ty` into the task result word
/// (`i64`): Int as-is, Float bit-cast, Bool/Char/managed zero-extended,
/// Void as 0.
fn box_result_word(fb: &mut FB, types: &TypeInterner, ty: TypeId) {
    if is_void(types, ty) {
        fb.i64c(0);
        return;
    }
    match local_val(types, ty).expect("thunk result types lower") {
        Val::I64 => {}
        Val::F64 => {
            fb.op(op::I64_REINTERPRET_F64);
        }
        Val::I32 => {
            fb.op(op::I64_EXTEND_I32_U);
        }
    }
}

/// `__lpp_task_thunk{n}` (5D2b slice 3) — the task code for the async
/// (or zero-parameter sync, for `spawn`) function `n`: load the
/// argument slots of the task env tuple (offset `16+8i`; managed slots
/// retained for the call), call the function with `env = NULL`, and
/// box the result into the task result word.
fn lower_task_thunk(
    env: &Env<'_>,
    function_id: MirFunctionId,
) -> Result<(Vec<Val>, Vec<Val>, Vec<Val>, Vec<u8>), CodegenError> {
    let function = env.program.function(function_id).unwrap();
    let user_params: Vec<MirLocalId> = env
        .program
        .function_parameters(function)
        .iter()
        .copied()
        .filter(|&p| {
            !matches!(
                env.program.local(p).unwrap().kind,
                lpp_mir::MirLocalKind::Capture
            )
        })
        .collect();
    let callee_index = *env
        .user_index
        .get(&function_id)
        .ok_or_else(|| unsupported("undispatched thunk callee", Some(function_id)))?;
    let mut fb = FB::new(1);
    // The task env tuple is the thunk's parameter (local 0); the
    // callee's dispatch-ABI env is NULL (plain functions ignore it).
    fb.i32c(0);
    for (i, &param) in user_params.iter().enumerate() {
        let ty = env.program.local(param).unwrap().ty;
        let offset = 16 + 8 * i as u32;
        if is_managed(env.types, ty) {
            // Managed slot: i64 word → i32 pointer, retained for the
            // call (the tuple keeps its own reference).
            fb.g(0).load64(offset).op(op::I32_WRAP_I64);
            fb.call(env.helper_index[&H::Retain]);
        } else {
            match local_val(env.types, ty)? {
                Val::I64 => {
                    fb.g(0).load64(offset);
                }
                Val::F64 => {
                    fb.g(0).loadf64(offset);
                }
                Val::I32 => {
                    fb.g(0).load64(offset).op(op::I32_WRAP_I64);
                }
            }
        }
    }
    // The thunk takes (env) -> i64 result word; the callee's
    // dispatch index is the call_indirect element index, applied by
    // the TaskPoll pass.
    fb.call(callee_index);
    box_result_word(&mut fb, env.types, function.return_type);
    Ok((vec![Val::I32], vec![Val::I64], fb.extras, fb.body))
}

/// `__lpp_closure_thunk{n}` (5D2b slice 3) — the task code for the
/// zero-parameter closure `n`: the wrapper tuple's slot 16 holds the
/// closure env; invoke the closure through the dispatch table and box
/// the result into the task result word.
fn lower_closure_thunk(
    env: &Env<'_>,
    function_id: MirFunctionId,
) -> Result<(Vec<Val>, Vec<Val>, Vec<Val>, Vec<u8>), CodegenError> {
    let function = env.program.function(function_id).unwrap();
    let call_type = *env
        .closure_call_type
        .get(&function.ty)
        .ok_or_else(|| unsupported("unregistered closure thunk type", Some(function_id)))?;
    let mut fb = FB::new(1);
    // The closure env: the wrapper tuple's slot 16 holds the closure
    // capsule pointer; the env word is the capsule's env slot. (A
    // bare-function wrapper has slot 16 = 0: NULL env.)
    let cap = fb.scratch(Val::I32);
    fb.g(0).load64(16).op(op::I32_WRAP_I64).s(cap);
    // The capsule's code word is the closure's TABLE position
    // (user_count + its user slot). call_indirect pops the element
    // index LAST (top of the stack), so push the env first and the
    // index on top.
    fb.g(cap).load64(8).op(op::I32_WRAP_I64);
    fb.g(cap).load64(0).op(op::I32_WRAP_I64);
    fb.call_indirect(call_type, env.dispatch_table);
    box_result_word(&mut fb, env.types, function.return_type);
    Ok((vec![Val::I32], vec![Val::I64], fb.extras, fb.body))
}

fn lower_helper(
    env: &Env<'_>,
    h: H,
    fd_helper_idx: u32,
    fd_write_import_idx: u32,
    fd_read_import_idx: u32,
    proc_exit_import_idx: u32,
) -> Result<(Vec<Val>, Vec<Val>, Vec<Val>, Vec<u8>), CodegenError> {
    let mut fb = match h {
        H::FdWrite | H::FdRead => FB::new(2),
        H::Input => FB::new(0),
        H::PrintInt => FB::new(1),
        H::PrintBool => FB::new(1),
        H::PrintStr => FB::new(1),
        H::PrintFloat => FB::new(1),
        H::WriteStr => FB::new(1),
        H::StrEq => FB::new(2),
        H::StrLen => FB::new(1),
        H::Alloc => FB::new(1),
        H::ArcAlloc => FB::new(2),
        H::Retain => FB::new(1),
        H::Release => FB::new(1),
        H::ListNew => FB::new(1),
        H::ListPushI64 => FB::new(2),
        H::ListPushF64 => FB::new(2),
        H::ListPushPtr => FB::new(2),
        H::ListLen => FB::new(1),
        H::ListGetI64 => FB::new(2),
        H::ListGetF64 => FB::new(2),
        H::ListGetPtr => FB::new(2),
        H::ListSetI64 => FB::new(3),
        H::ListSetF64 => FB::new(3),
        H::ListSetPtr => FB::new(3),
        H::SliceNewList | H::SliceNewStr => FB::new(3),
        H::SliceLen | H::StrSliceToStr => FB::new(1),
        H::SliceGetI64 | H::SliceGetF64 | H::SliceGetPtr | H::StrSliceGet => FB::new(2),
        H::MapNew | H::MapEnsure | H::MapLen => FB::new(1),
        H::MapFindI64
        | H::MapFindStr
        | H::MapGetI64
        | H::MapGetStr
        | H::MapHasI64
        | H::MapHasStr
        | H::MapRemoveI64
        | H::MapRemoveStr => FB::new(2),
        H::MapPutI64 | H::MapPutStr => FB::new(3),
        H::TaskNew => FB::new(3),
        H::TaskPoll => FB::new(1),
        H::TaskAwait => FB::new(1),
        H::TaskDestroy => FB::new(1),
        // ── 5D2b slice 4, batch 3: string builtins ──
        H::StrConcat => FB::new(2),
        H::StrContains => FB::new(2),
        H::StrStartsWith => FB::new(2),
        H::StrEndsWith => FB::new(2),
        H::StrFind => FB::new(2),
        H::StrReplace | H::StrSubstr => FB::new(3),
        H::StrRepeat | H::CharAt => FB::new(2),
        H::StrSplit => FB::new(2),
        H::Ord => FB::new(1),
        H::Chr => FB::new(1),
        H::StrTrim => FB::new(1),
        H::StrLower => FB::new(1),
        H::StrUpper => FB::new(1),
        H::IntToStr => FB::new(1),
        H::StrToInt => FB::new(1),
        H::FloatToStr => FB::new(1),
        H::BoolToStr => FB::new(1),
        H::U64ToStr => FB::new(1),
        H::U64ToHex => FB::new(1),
        H::StrToU64 => FB::new(1),
        H::Log2 | H::Exp2 | H::Sin | H::Cos => FB::new(1),
        H::Pow => FB::new(2),
    };
    let results = match h {
        H::FdRead => vec![Val::I32],
        H::Input => vec![Val::I32],
        H::StrEq => vec![Val::I32],
        H::StrLen => vec![Val::I64],
        // Retain passes the value through (callers keep using it).
        H::Alloc | H::ArcAlloc | H::Retain | H::ListNew | H::ListGetPtr => vec![Val::I32],
        H::ListLen | H::ListGetI64 | H::SliceLen | H::SliceGetI64 => vec![Val::I64],
        H::ListGetF64 | H::SliceGetF64 => vec![Val::F64],
        H::SliceNewList
        | H::SliceNewStr
        | H::SliceGetPtr
        | H::StrSliceGet
        | H::StrSliceToStr
        | H::MapNew => vec![Val::I32],
        H::MapFindI64
        | H::MapFindStr
        | H::MapGetI64
        | H::MapGetStr
        | H::MapHasI64
        | H::MapHasStr
        | H::MapLen => vec![Val::I64],
        // TaskNew returns the node pointer as an i64 word (zero-
        // extended in the body; callers narrow for the wasm ABI).
        H::TaskNew => vec![Val::I64],
        H::TaskPoll | H::TaskAwait => vec![Val::I64],
        // ── 5D2b slice 4, batch 3: string builtins ──
        H::StrConcat
        | H::StrReplace
        | H::StrSubstr
        | H::StrRepeat
        | H::CharAt
        | H::Chr
        | H::StrTrim
        | H::StrLower
        | H::StrUpper
        | H::IntToStr
        | H::FloatToStr
        | H::BoolToStr
        | H::U64ToStr
        | H::U64ToHex => vec![Val::I32],
        H::StrSplit => vec![Val::I32],
        H::StrContains | H::StrStartsWith | H::StrEndsWith => vec![Val::I32],
        H::StrFind | H::StrToInt | H::StrToU64 | H::Ord => vec![Val::I64],
        H::Log2 | H::Exp2 | H::Pow | H::Sin | H::Cos => vec![Val::F64],
        _ => vec![],
    };
    let params = match h {
        H::FdWrite | H::FdRead => vec![Val::I32, Val::I32],
        H::Input => vec![],
        H::PrintInt => vec![Val::I64],
        H::PrintBool => vec![Val::I32],
        H::PrintStr => vec![Val::I32],
        H::PrintFloat => vec![Val::F64],
        H::WriteStr => vec![Val::I32],
        H::StrEq => vec![Val::I32, Val::I32],
        H::StrLen => vec![Val::I32],
        H::Alloc => vec![Val::I32],
        H::ArcAlloc => vec![Val::I32, Val::I32],
        H::Retain => vec![Val::I32],
        H::Release => vec![Val::I32],
        H::ListNew => vec![Val::I32],
        H::ListPushI64 | H::ListGetI64 => vec![Val::I32, Val::I64],
        H::ListPushF64 => vec![Val::I32, Val::F64],
        // The getter's second parameter is the index (an Int, i64) —
        // the bodies multiply it by 8 in i64.
        H::ListGetF64 | H::ListGetPtr => vec![Val::I32, Val::I64],
        H::ListPushPtr => vec![Val::I32, Val::I32],
        H::ListLen => vec![Val::I32],
        H::ListSetI64 => vec![Val::I32, Val::I64, Val::I64],
        H::ListSetF64 => vec![Val::I32, Val::I64, Val::F64],
        H::ListSetPtr => vec![Val::I32, Val::I64, Val::I32],
        H::SliceNewList | H::SliceNewStr => vec![Val::I32, Val::I64, Val::I64],
        H::SliceLen | H::StrSliceToStr => vec![Val::I32],
        H::SliceGetI64 | H::SliceGetF64 | H::SliceGetPtr | H::StrSliceGet => {
            vec![Val::I32, Val::I64]
        }
        H::MapNew => vec![Val::I32],
        H::MapEnsure | H::MapLen => vec![Val::I32],
        H::MapFindI64 | H::MapGetI64 | H::MapHasI64 | H::MapRemoveI64 => vec![Val::I32, Val::I64],
        H::MapFindStr | H::MapGetStr | H::MapHasStr | H::MapRemoveStr => vec![Val::I32, Val::I32],
        H::MapPutI64 => vec![Val::I32, Val::I64, Val::I64],
        H::MapPutStr => vec![Val::I32, Val::I32, Val::I64],
        H::TaskNew => vec![Val::I32, Val::I32, Val::I32],
        H::TaskPoll => vec![Val::I32],
        H::TaskAwait => vec![Val::I32],
        H::TaskDestroy => vec![Val::I32],
        // ── 5D2b slice 4, batch 3: string builtins ──
        H::StrConcat | H::StrContains | H::StrStartsWith | H::StrEndsWith | H::StrFind => {
            vec![Val::I32, Val::I32]
        }
        H::StrReplace => vec![Val::I32, Val::I32, Val::I32],
        H::StrSubstr => vec![Val::I32, Val::I64, Val::I64],
        H::StrRepeat | H::CharAt => vec![Val::I32, Val::I64],
        H::StrSplit => vec![Val::I32, Val::I64],
        H::Ord => vec![Val::I32],
        H::Chr => vec![Val::I64],
        H::StrTrim | H::StrLower | H::StrUpper | H::StrToInt | H::StrToU64 => {
            vec![Val::I32]
        }
        H::IntToStr | H::U64ToStr | H::U64ToHex => vec![Val::I64],
        H::FloatToStr => vec![Val::F64],
        H::BoolToStr => vec![Val::I32],
        H::Log2 | H::Exp2 | H::Sin | H::Cos => vec![Val::F64],
        H::Pow => vec![Val::F64, Val::F64],
    };
    emit_helper_body(
        &mut fb,
        h,
        env,
        fd_helper_idx,
        fd_write_import_idx,
        fd_read_import_idx,
        proc_exit_import_idx,
    )?;
    Ok((params, results, fb.extras, fb.body))
}

/// Store `w` digits of `src` (i64 local), most-significant first with
/// leading-zero padding, at NUM_BUF+pos..+pos+w. Digits are extracted
/// least-significant first and written to the final slots in place.
/// Bounded: exactly `w` iterations (w <= 6 by construction).
fn emit_digits(fb: &mut FB, pos: u32, w: u32, k: u32, d: u32, src: u32) {
    fb.i32c(0).s(k);
    fb.loop_(); // $dg
    fb.g(k).g(w).op(op::I32_LT_U).if_(); // $dgi
    fb.g(src)
        .i64c(10)
        .op(op::I64_REM_U)
        .op(op::I32_WRAP_I64)
        .s(d);
    fb.g(src).i64c(10).op(op::I64_DIV_U).s(src);
    fb.i32c(NUM_BUF as i64)
        .g(pos)
        .op(op::I32_ADD)
        .g(w)
        .op(op::I32_ADD)
        .i32c(1)
        .op(op::I32_SUB)
        .g(k)
        .op(op::I32_SUB);
    fb.g(d).i32c(48).op(op::I32_ADD);
    fb.store8(0);
    fb.g(k).i32c(1).op(op::I32_ADD).s(k);
    fb.br(1); // continue $dg
    fb.end(); // $dgi
    fb.end(); // $dg
    fb.g(pos).g(w).op(op::I32_ADD).s(pos);
}

/// Store `w` zero bytes at NUM_BUF+pos.. and advance pos by `w`.
/// Bounded: at most `w` iterations (w <= 5 by construction).
fn emit_zeros(fb: &mut FB, pos: u32, w: u32, k: u32) {
    fb.i32c(0).s(k);
    fb.loop_(); // $zr
    fb.g(k).g(w).op(op::I32_LT_U).if_(); // $zri
    fb.i32c(NUM_BUF as i64).g(pos).op(op::I32_ADD);
    fb.i32c(48);
    fb.store8(0);
    fb.g(pos).i32c(1).op(op::I32_ADD).s(pos);
    fb.g(k).i32c(1).op(op::I32_ADD).s(k);
    fb.br(1); // continue $zr
    fb.end(); // $zri
    fb.end(); // $zr
}

/// p10 = 10^w exactly (w <= 6 by construction). Bounded: `w` iterations.
fn emit_pow10(fb: &mut FB, w: u32, p10: u32, k: u32) {
    fb.i64c(1).s(p10);
    fb.i32c(0).s(k);
    fb.loop_(); // $pw
    fb.g(k).g(w).op(op::I32_LT_U).if_(); // $pwi
    fb.g(p10).i64c(10).op(op::I64_MUL).s(p10);
    fb.g(k).i32c(1).op(op::I32_ADD).s(k);
    fb.br(1); // continue $pw
    fb.end(); // $pwi
    fb.end(); // $pw
}

fn emit_helper_body(
    fb: &mut FB,
    h: H,
    env: &Env<'_>,
    fd_helper_idx: u32,
    fd_write_import_idx: u32,
    fd_read_import_idx: u32,
    proc_exit_import_idx: u32,
) -> Result<(), CodegenError> {
    // Helper-to-helper calls go through the pre-registered index map
    // (the pre-scan closed the transitive needs).
    let idx = |h: H| {
        *env.helper_index
            .get(&h)
            .expect("pre-scan closed the helper needs")
    };
    match h {
        H::FdWrite => {
            fb.i32c(IOVEC_BUF as i64);
            fb.g(0);
            fb.store32(0);
            fb.i32c((IOVEC_BUF + 4) as i64);
            fb.g(1);
            fb.store32(0);
            fb.i32c(1); // fd = stdout
            fb.i32c(IOVEC_BUF as i64);
            fb.i32c(1); // iovec count
            fb.i32c(FD_IO_OUT as i64);
            fb.call(fd_write_import_idx);
            fb.op(op::DROP);
        }
        H::FdRead => {
            fb.i32c(IOVEC_BUF as i64);
            fb.g(0);
            fb.store32(0);
            fb.i32c((IOVEC_BUF + 4) as i64);
            fb.g(1);
            fb.store32(0);
            fb.i32c(0); // fd = stdin
            fb.i32c(IOVEC_BUF as i64);
            fb.i32c(1); // iovec count
            fb.i32c(FD_IO_OUT as i64);
            fb.call(fd_read_import_idx);
            // A WASI read error is an empty read at the language boundary.
            fb.if_();
            fb.i32c(0).op(op::RETURN);
            fb.end();
            fb.i32c(FD_IO_OUT as i64).load32(0);
        }
        H::Input => {
            const INPUT_CAP: i64 = 4096;
            let raw = fb.scratch(Val::I32);
            let len = fb.scratch(Val::I32);
            let out = fb.scratch(Val::I32);
            fb.i32c(INPUT_CAP).call(idx(H::Alloc)).s(raw);
            fb.i32c(0).s(len);
            // Read one byte at a time so one input() call cannot consume the
            // following line. The 4096-byte cap is explicit and deterministic.
            fb.block();
            fb.loop_();
            fb.g(len).i32c(INPUT_CAP).op(op::I32_GE_U).br_if(1);
            fb.g(raw)
                .g(len)
                .op(op::I32_ADD)
                .i32c(1)
                .call(idx(H::FdRead))
                .op(op::I32_EQZ)
                .br_if(1);
            fb.g(raw)
                .g(len)
                .op(op::I32_ADD)
                .load8(0)
                .i32c(10)
                .op(op::I32_EQ)
                .br_if(1);
            fb.g(len).i32c(1).op(op::I32_ADD).s(len);
            fb.br(0);
            fb.end();
            fb.end();
            // Strip the CR in a CRLF line.
            fb.g(len).op(op::I32_EQZ).if_();
            fb.else_();
            fb.g(raw)
                .g(len)
                .op(op::I32_ADD)
                .i32c(1)
                .op(op::I32_SUB)
                .load8(0)
                .i32c(13)
                .op(op::I32_EQ)
                .if_();
            fb.g(len).i32c(1).op(op::I32_SUB).s(len);
            fb.end();
            fb.end();
            fb.g(len)
                .i32c(4)
                .op(op::I32_ADD)
                .i32c(0)
                .call(idx(H::ArcAlloc))
                .s(out);
            fb.g(out).g(len).store32(0);
            fb.g(out).i32c(4).op(op::I32_ADD);
            fb.g(raw);
            fb.g(len);
            fb.memory_copy();
            fb.g(out);
        }
        H::PrintInt => {
            let mag = fb.scratch(Val::I64);
            let neg = fb.scratch(Val::I32);
            let cur = fb.scratch(Val::I32);
            let d = fb.scratch(Val::I32);
            // neg = v < 0
            fb.g(0);
            fb.i64c(0);
            fb.op(op::I64_LT_S);
            fb.s(neg);
            // mag = v; if neg, mag = 0 - v
            fb.g(0);
            fb.s(mag);
            fb.g(neg);
            fb.if_();
            fb.i64c(0);
            fb.g(0);
            fb.op(op::I64_SUB);
            fb.s(mag);
            fb.end();
            // write digits downward from NUM_BUF_END. `mag` holds the
            // unsigned magnitude (wrapping negation already handled
            // i64::MIN, whose magnitude 2^63 only fits as u64), so the
            // digit loop must divide unsigned.
            fb.i32c(NUM_BUF_END as i64);
            fb.s(cur);
            fb.block(); // $done
            fb.loop_(); // $d
            fb.g(mag);
            fb.i64c(10);
            fb.op(op::I64_REM_U);
            fb.op(op::I32_WRAP_I64);
            fb.s(d);
            fb.g(mag);
            fb.i64c(10);
            fb.op(op::I64_DIV_U);
            fb.s(mag);
            fb.g(cur);
            fb.i32c(1);
            fb.op(op::I32_SUB);
            fb.g(d);
            fb.i32c(48);
            fb.op(op::I32_ADD);
            fb.store8(0);
            fb.g(cur);
            fb.i32c(1);
            fb.op(op::I32_SUB);
            fb.s(cur);
            fb.g(mag);
            fb.op(op::I64_EQZ);
            fb.br_if(1); // $done
            fb.br(0); // $d
            fb.end(); // $d
            fb.end(); // $done
            // sign
            fb.g(neg);
            fb.if_();
            fb.g(cur);
            fb.i32c(1);
            fb.op(op::I32_SUB); // addr = cur - 1
            fb.i32c(45); // '-' byte
            fb.store8(0);
            fb.g(cur);
            fb.i32c(1);
            fb.op(op::I32_SUB);
            fb.s(cur);
            fb.end();
            // write [cur .. NUM_BUF_END)
            fb.g(cur);
            fb.i32c(NUM_BUF_END as i64);
            fb.g(cur);
            fb.op(op::I32_SUB);
            fb.call(fd_helper_idx);
            // newline
            fb.i32c(NUM_BUF as i64);
            fb.i32c(10);
            fb.store8(0);
            fb.i32c(NUM_BUF as i64);
            fb.i32c(1);
            fb.call(fd_helper_idx);
        }
        H::PrintBool => {
            fb.g(0);
            fb.i32c(1);
            fb.op(op::I32_EQ);
            fb.if_();
            fb.i32c(NUM_BUF as i64);
            fb.i32c(49);
            fb.store8(0);
            fb.i32c((NUM_BUF + 1) as i64);
            fb.i32c(10);
            fb.store8(0);
            fb.i32c(NUM_BUF as i64);
            fb.i32c(2);
            fb.call(fd_helper_idx);
            fb.else_();
            fb.i32c(NUM_BUF as i64);
            fb.i32c(48);
            fb.store8(0);
            fb.i32c((NUM_BUF + 1) as i64);
            fb.i32c(10);
            fb.store8(0);
            fb.i32c(NUM_BUF as i64);
            fb.i32c(2);
            fb.call(fd_helper_idx);
            fb.end();
        }
        H::PrintStr => {
            fb.g(0);
            fb.i32c(4);
            fb.op(op::I32_ADD);
            fb.g(0);
            fb.load32(0);
            fb.call(fd_helper_idx);
            fb.i32c(NUM_BUF as i64);
            fb.i32c(10);
            fb.store8(0);
            fb.i32c(NUM_BUF as i64);
            fb.i32c(1);
            fb.call(fd_helper_idx);
        }
        H::WriteStr => {
            fb.g(0);
            fb.i32c(4);
            fb.op(op::I32_ADD);
            fb.g(0);
            fb.load32(0);
            fb.call(fd_helper_idx);
        }
        H::StrEq => {
            let la = fb.scratch(Val::I32);
            let lb = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I32);
            let ba = fb.scratch(Val::I32);
            let bb = fb.scratch(Val::I32);
            fb.g(0);
            fb.load32(0);
            fb.s(la);
            fb.g(1);
            fb.load32(0);
            fb.s(lb);
            fb.g(la);
            fb.g(lb);
            fb.op(op::I32_NE);
            fb.if_();
            fb.i32c(0);
            fb.op(op::RETURN);
            fb.end();
            fb.i32c(0);
            fb.s(i);
            fb.block(); // $done
            fb.loop_(); // $c
            fb.g(i);
            fb.g(la);
            fb.op(op::I32_GE_S);
            fb.br_if(1);
            fb.g(0);
            fb.i32c(4);
            fb.op(op::I32_ADD);
            fb.g(i);
            fb.op(op::I32_ADD);
            fb.load8(0);
            fb.s(ba);
            fb.g(1);
            fb.i32c(4);
            fb.op(op::I32_ADD);
            fb.g(i);
            fb.op(op::I32_ADD);
            fb.load8(0);
            fb.s(bb);
            fb.g(ba);
            fb.g(bb);
            fb.op(op::I32_NE);
            fb.if_();
            fb.i32c(0);
            fb.op(op::RETURN);
            fb.end();
            fb.g(i);
            fb.i32c(1);
            fb.op(op::I32_ADD);
            fb.s(i);
            fb.br(0);
            fb.end(); // $c
            fb.end(); // $done
            fb.i32c(1);
            fb.op(op::RETURN);
        }
        H::StrLen => {
            fb.g(0);
            fb.load32(0);
            fb.op(op::I64_EXTEND_I32_U);
        }
        // ── freestanding numeric helpers ────────────────────────────────
        H::Log2 => {
            let bits = fb.scratch(Val::I64);
            let exponent = fb.scratch(Val::I64);
            let mantissa = fb.scratch(Val::F64);
            let t = fb.scratch(Val::F64);
            let t2 = fb.scratch(Val::F64);
            let term = fb.scratch(Val::F64);
            let sum = fb.scratch(Val::F64);
            let i = fb.scratch(Val::I32);
            fb.g(0).op(op::I64_REINTERPRET_F64).s(bits);
            fb.g(bits)
                .i64c(52)
                .op(op::I64_SHR_U)
                .i64c(2047)
                .op(op::I64_AND)
                .i64c(1023)
                .op(op::I64_SUB)
                .s(exponent);
            fb.g(bits)
                .i64c(0x800f_ffff_ffff_ffffu64 as i64)
                .op(op::I64_AND);
            fb.i64c(1023).i64c(52).op(op::I64_SHL).op(op::I64_OR);
            fb.op(op::F64_REINTERPRET_I64).s(mantissa);
            fb.g(mantissa).f64c(1.0).op(op::F64_SUB);
            fb.g(mantissa)
                .f64c(1.0)
                .op(op::F64_ADD)
                .op(op::F64_DIV)
                .s(t);
            fb.g(t).g(t).op(op::F64_MUL).s(t2);
            fb.g(t).s(term);
            fb.g(t).s(sum);
            fb.i32c(3).s(i);
            fb.block();
            fb.loop_();
            fb.g(i).i32c(25).op(op::I32_GE_S).br_if(1);
            fb.g(term).g(t2).op(op::F64_MUL).s(term);
            fb.g(sum)
                .g(term)
                .g(i)
                .op(op::F64_CONVERT_I32_S)
                .op(op::F64_DIV)
                .op(op::F64_ADD)
                .s(sum);
            fb.g(i).i32c(2).op(op::I32_ADD).s(i);
            fb.br(0);
            fb.end();
            fb.end();
            fb.g(exponent).op(op::F64_CONVERT_I64_S);
            fb.g(sum)
                .f64c(2.0)
                .op(op::F64_MUL)
                .f64c(std::f64::consts::LN_2)
                .op(op::F64_DIV)
                .op(op::F64_ADD);
        }
        H::Exp2 => {
            let integer = fb.scratch(Val::I64);
            let fraction = fb.scratch(Val::F64);
            let u = fb.scratch(Val::F64);
            let term = fb.scratch(Val::F64);
            let sum = fb.scratch(Val::F64);
            let i = fb.scratch(Val::I32);
            fb.g(0).f64c(1024.0).op(op::F64_GE).if_();
            fb.i64c(0x7ff0_0000_0000_0000)
                .op(op::F64_REINTERPRET_I64)
                .op(op::RETURN);
            fb.end();
            fb.g(0).f64c(-1075.0).op(op::F64_LT).if_();
            fb.f64c(0.0).op(op::RETURN);
            fb.end();
            fb.g(0).op(op::F64_FLOOR).op(op::I64_TRUNC_F64_S).s(integer);
            fb.g(0)
                .g(integer)
                .op(op::F64_CONVERT_I64_S)
                .op(op::F64_SUB)
                .s(fraction);
            fb.g(integer).i64c(-1022).op(op::I64_LT_S).if_();
            fb.i64c(-1022).s(integer);
            fb.end();
            fb.g(integer).i64c(1023).op(op::I64_GT_S).if_();
            fb.i64c(1023).s(integer);
            fb.end();
            fb.g(fraction)
                .f64c(std::f64::consts::LN_2)
                .op(op::F64_MUL)
                .s(u);
            fb.f64c(1.0).s(sum);
            fb.f64c(1.0).s(term);
            fb.i32c(1).s(i);
            fb.block();
            fb.loop_();
            fb.g(i).i32c(15).op(op::I32_GE_S).br_if(1);
            fb.g(term)
                .g(u)
                .op(op::F64_MUL)
                .g(i)
                .op(op::F64_CONVERT_I32_S)
                .op(op::F64_DIV)
                .s(term);
            fb.g(sum).g(term).op(op::F64_ADD).s(sum);
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.br(0);
            fb.end();
            fb.end();
            fb.g(sum);
            fb.g(integer)
                .i64c(1023)
                .op(op::I64_ADD)
                .i64c(52)
                .op(op::I64_SHL)
                .op(op::F64_REINTERPRET_I64);
            fb.op(op::F64_MUL);
        }
        H::Pow => {
            let integer_exp = fb.scratch(Val::I64);
            let result = fb.scratch(Val::F64);
            fb.g(1).f64c(0.0).op(op::F64_EQ).if_();
            fb.f64c(1.0).op(op::RETURN);
            fb.end();
            fb.g(0).f64c(1.0).op(op::F64_EQ).if_();
            fb.f64c(1.0).op(op::RETURN);
            fb.end();
            fb.g(0).f64c(0.0).op(op::F64_EQ).if_();
            fb.g(1).f64c(0.0).op(op::F64_LT).if_();
            fb.i64c(0x7ff0_0000_0000_0000)
                .op(op::F64_REINTERPRET_I64)
                .op(op::RETURN);
            fb.end();
            fb.f64c(0.0).op(op::RETURN);
            fb.end();
            fb.g(0).f64c(0.0).op(op::F64_LT).if_();
            fb.g(1).op(op::F64_TRUNC).g(1).op(op::F64_NE).if_();
            fb.f64c(f64::NAN).op(op::RETURN);
            fb.end();
            fb.g(1).op(op::I64_TRUNC_F64_S).s(integer_exp);
            fb.g(0).op(op::F64_NEG).call(idx(H::Log2));
            fb.g(1).op(op::F64_MUL).call(idx(H::Exp2)).s(result);
            fb.g(integer_exp)
                .i64c(1)
                .op(op::I64_AND)
                .op(op::I32_WRAP_I64)
                .if_();
            fb.g(result).op(op::F64_NEG).s(result);
            fb.end();
            fb.g(result).op(op::RETURN);
            fb.end();
            fb.g(0).call(idx(H::Log2));
            fb.g(1).op(op::F64_MUL).call(idx(H::Exp2));
        }
        H::Sin | H::Cos => {
            let reduced = fb.scratch(Val::F64);
            let square = fb.scratch(Val::F64);
            let term = fb.scratch(Val::F64);
            let sum = fb.scratch(Val::F64);
            let k = fb.scratch(Val::I32);
            // Reduce to [-pi, pi) before evaluating a bounded Taylor series.
            fb.g(0).f64c(std::f64::consts::PI).op(op::F64_ADD);
            fb.f64c(std::f64::consts::TAU)
                .op(op::F64_DIV)
                .op(op::F64_FLOOR)
                .f64c(std::f64::consts::TAU)
                .op(op::F64_MUL)
                .s(reduced);
            fb.g(0).g(reduced).op(op::F64_SUB).s(reduced);
            fb.g(reduced).g(reduced).op(op::F64_MUL).s(square);
            if h == H::Sin {
                fb.g(reduced).s(term);
                fb.g(reduced).s(sum);
            } else {
                fb.f64c(1.0).s(term);
                fb.f64c(1.0).s(sum);
            }
            fb.i32c(1).s(k);
            fb.block();
            fb.loop_();
            fb.g(k).i32c(12).op(op::I32_GE_S).br_if(1);
            fb.g(term).g(square).op(op::F64_MUL).op(op::F64_NEG);
            if h == H::Sin {
                fb.g(k).i32c(2).op(op::I32_MUL).op(op::F64_CONVERT_I32_S);
                fb.g(k)
                    .i32c(2)
                    .op(op::I32_MUL)
                    .i32c(1)
                    .op(op::I32_ADD)
                    .op(op::F64_CONVERT_I32_S);
            } else {
                fb.g(k)
                    .i32c(2)
                    .op(op::I32_MUL)
                    .i32c(1)
                    .op(op::I32_SUB)
                    .op(op::F64_CONVERT_I32_S);
                fb.g(k).i32c(2).op(op::I32_MUL).op(op::F64_CONVERT_I32_S);
            }
            fb.op(op::F64_MUL).op(op::F64_DIV).s(term);
            fb.g(sum).g(term).op(op::F64_ADD).s(sum);
            fb.g(k).i32c(1).op(op::I32_ADD).s(k);
            fb.br(0);
            fb.end();
            fb.end();
            fb.g(sum);
        }
        // ── 5D2a ARC heap ───────────────────────────────────────────────
        H::Alloc => {
            let hptr = fb.scratch(Val::I32);
            let new = fb.scratch(Val::I32);
            let need = fb.scratch(Val::I32);
            // size = (size + 7) & -8
            fb.g(0)
                .i32c(7)
                .op(op::I32_ADD)
                .i32c(-8)
                .op(op::I32_AND)
                .s(0);
            // h = heap; new = h + size
            fb.gget(GLOBAL_HEAP).s(hptr);
            fb.g(hptr).g(0).op(op::I32_ADD).s(new);
            // if (u32)new > memory.size() << 16: grow max(need, 32)
            fb.g(new)
                .memory_size()
                .i32c(16)
                .op(op::I32_SHL)
                .op(op::I32_GT_U)
                .if_();
            // need = (new + 65535) >>> 16 - memory.size()
            fb.g(new)
                .i32c(65535)
                .op(op::I32_ADD)
                .i32c(16)
                .op(op::I32_SHR_U)
                .memory_size()
                .op(op::I32_SUB)
                .s(need);
            fb.g(need)
                .i32c(32)
                .g(need)
                .i32c(32)
                .op(op::I32_GT_U)
                .op(op::SELECT)
                .memory_grow();
            fb.i32c(-1).op(op::I32_EQ).if_().op(op::UNREACHABLE).end();
            fb.end();
            // heap = new; return h
            fb.g(new).gset(GLOBAL_HEAP);
            fb.g(hptr);
        }
        H::ArcAlloc => {
            let a = fb.scratch(Val::I32);
            // a = Alloc(size + 24); header [rc][drop-index][magic "ARC1"]
            fb.g(0)
                .i32c(ARC_HEADER)
                .op(op::I32_ADD)
                .call(idx(H::Alloc))
                .s(a);
            fb.g(a).i64c(1).store64(0);
            fb.g(a).g(1).op(op::I64_EXTEND_I32_U).store64(8);
            fb.g(a).i64c(ARC_MAGIC).store64(16);
            // payload = a + 24
            fb.g(a).i32c(ARC_HEADER).op(op::I32_ADD);
        }
        H::Retain => {
            fb.g(0).if_();
            // if rc != IMMORTAL: rc += 1
            fb.g(0)
                .i32c(24)
                .op(op::I32_SUB)
                .load64(0)
                .i64c(IMMORTAL_RC)
                .op(op::I64_NE)
                .if_();
            fb.g(0).i32c(24).op(op::I32_SUB); // addr
            fb.g(0).i32c(24).op(op::I32_SUB).load64(0); // rc
            fb.i64c(1).op(op::I64_ADD).store64(0);
            fb.end();
            fb.end();
            // Pass the value through: the caller's copy is still owned.
            fb.g(0);
        }
        H::Release => {
            let rc = fb.scratch(Val::I64);
            let drop_idx = fb.scratch(Val::I32);
            fb.g(0).if_();
            fb.g(0).i32c(24).op(op::I32_SUB).load64(0).s(rc);
            fb.g(rc).i64c(IMMORTAL_RC).op(op::I64_NE).if_();
            // rc < 1 is a double release: fail loudly.
            fb.g(rc)
                .i64c(1)
                .op(op::I64_LT_S)
                .if_()
                .op(op::UNREACHABLE)
                .end();
            fb.g(rc).i64c(1).op(op::I64_EQ).if_();
            // Dying: mark, then dispatch the destructor if present.
            fb.g(0).i32c(24).op(op::I32_SUB).i64c(0).store64(0);
            fb.g(0)
                .i32c(24)
                .op(op::I32_SUB)
                .load64(8)
                .op(op::I32_WRAP_I64)
                .s(drop_idx);
            fb.g(drop_idx).if_();
            fb.g(0).g(drop_idx).call_indirect(env.drop_call_type, 0); // drop table
            fb.end();
            fb.else_();
            fb.g(0)
                .i32c(24)
                .op(op::I32_SUB)
                .g(rc)
                .i64c(1)
                .op(op::I64_SUB)
                .store64(0);
            fb.end();
            fb.end();
            fb.end();
        }
        H::PrintFloat => {
            // The oracle prints floats with Rust `{:.6}`: `NaN`, `inf`,
            // `-inf`, a sign for negative zero, and no exponent form.
            // string_ptr points at the entry's len field; the raw bytes
            // start 4 bytes past it (PrintStr applies the same +4).
            let lit = |s: &str| *env.string_ptr.get(s).expect("print float literal pooled") + 4;
            let dash = lit("-");
            let dot = lit(".");
            let nan = lit("NaN\n");
            let inf = lit("inf\n");
            let neginf = lit("-inf\n");
            let zeros = lit(".000000\n");
            let neg = fb.scratch(Val::I32);
            let n = fb.scratch(Val::I64);
            let ip = fb.scratch(Val::I64);
            let cur = fb.scratch(Val::I32);
            let d = fb.scratch(Val::I32);
            let pos = fb.scratch(Val::I32);
            macro_rules! write_u64 {
                ($v:expr) => {{
                    // digits written downward from NUM_BUF_END
                    fb.i32c(NUM_BUF_END as i64).s(cur);
                    fb.block(); // $done
                    fb.loop_(); // $d
                    fb.g($v)
                        .i64c(10)
                        .op(op::I64_REM_U)
                        .op(op::I32_WRAP_I64)
                        .s(d);
                    fb.g($v).i64c(10).op(op::I64_DIV_U).s($v);
                    fb.g(cur)
                        .i32c(1)
                        .op(op::I32_SUB)
                        .g(d)
                        .i32c(48)
                        .op(op::I32_ADD)
                        .store8(0);
                    fb.g(cur).i32c(1).op(op::I32_SUB).s(cur);
                    fb.g($v).op(op::I64_EQZ).br_if(1); // $done
                    fb.br(0); // $d
                    fb.end(); // $d
                    fb.end(); // $done
                    // write [cur .. NUM_BUF_END)
                    fb.g(cur)
                        .i32c(NUM_BUF_END as i64)
                        .g(cur)
                        .op(op::I32_SUB)
                        .call(fd_helper_idx);
                }};
            }
            // NaN?
            fb.g(0).g(0).op(op::F64_NE).if_();
            fb.i32c(nan as i64).i32c(4).call(fd_helper_idx);
            fb.op(op::RETURN);
            fb.end();
            // ±inf?
            fb.g(0)
                .f64c(0.0)
                .op(op::F64_MUL)
                .f64c(0.0)
                .op(op::F64_NE)
                .if_();
            fb.g(0).f64c(0.0).op(op::F64_LT).if_();
            fb.i32c(neginf as i64).i32c(5).call(fd_helper_idx);
            fb.else_();
            fb.i32c(inf as i64).i32c(4).call(fd_helper_idx);
            fb.end();
            fb.op(op::RETURN);
            fb.end();
            // sign (a sign-bit test: Rust's `{:.6}` prints `-0.000000`)
            fb.g(0)
                .op(op::I64_REINTERPRET_F64)
                .i64c(i64::MIN)
                .op(op::I64_AND)
                .i64c(0)
                .op(op::I64_NE)
                .s(neg);
            fb.g(0).op(op::F64_ABS).s(0);
            fb.g(neg).if_();
            fb.i32c(dash as i64).i32c(1).call(fd_helper_idx);
            fb.end();
            // fixed 6-digit fraction for |x| < 9e12
            fb.g(0).f64c(9.0e12).op(op::F64_LT).if_();
            fb.g(0)
                .f64c(1_000_000.0)
                .op(op::F64_MUL)
                .f64c(0.5)
                .op(op::F64_ADD)
                .op(op::I64_TRUNC_F64_U)
                .s(n);
            fb.g(n).i64c(1_000_000).op(op::I64_DIV_U).s(ip);
            write_u64!(ip);
            fb.i32c(dot as i64).i32c(1).call(fd_helper_idx);
            // frac = n % 1e6, exactly 6 digits into NUM_BUF
            fb.g(n).i64c(1_000_000).op(op::I64_REM_U).s(n);
            fb.i32c(6).s(pos);
            fb.loop_();
            fb.g(pos).i32c(1).op(op::I32_SUB).t(pos);
            fb.i32c(NUM_BUF as i64).op(op::I32_ADD);
            fb.g(n)
                .i64c(10)
                .op(op::I64_REM_U)
                .op(op::I32_WRAP_I64)
                .i32c(48)
                .op(op::I32_ADD)
                .store8(0);
            fb.g(n).i64c(10).op(op::I64_DIV_U).s(n);
            fb.g(pos).br_if(0);
            fb.end();
            fb.i32c(NUM_BUF as i64).i32c(6).call(fd_helper_idx);
            // newline
            fb.i32c(NUM_BUF as i64).i32c(10).store8(0);
            fb.i32c(NUM_BUF as i64).i32c(1).call(fd_helper_idx);
            fb.else_();
            // |x| >= 9e12: integer part + ".000000\n"
            fb.g(0).op(op::I64_TRUNC_F64_U).s(ip);
            write_u64!(ip);
            fb.i32c(zeros as i64).i32c(8).call(fd_helper_idx);
            fb.end();
        }
        // ── 5D2a lists (payloads of i64 slots) ─────────────────────────
        H::ListNew => {
            let l = fb.scratch(Val::I32);
            let slot = env.list_drop_slot.expect("list destroyer registered");
            fb.i32c(32).i32c(slot as i64).call(idx(H::ArcAlloc)).s(l);
            fb.g(l).i64c(0).store64(LIST_DATA as u32);
            fb.g(l).i64c(0).store64(LIST_LEN as u32);
            fb.g(l).i64c(0).store64(LIST_CAP as u32);
            fb.g(l)
                .g(0)
                .op(op::I64_EXTEND_I32_U)
                .store64(LIST_IS_ARC as u32);
            fb.g(l);
        }
        H::ListPushI64 => {
            list_push_body(fb, idx, false, false);
        }
        H::ListPushF64 => {
            list_push_body(fb, idx, true, false);
        }
        H::ListPushPtr => {
            list_push_body(fb, idx, false, true);
        }
        H::ListLen => {
            fb.g(0).op(op::I32_EQZ).if_().i64c(0).op(op::RETURN).end();
            fb.g(0).load64(LIST_LEN as u32);
        }
        H::ListGetI64 => {
            list_bounds_check(fb);
            fb.g(0)
                .load64(LIST_DATA as u32)
                .op(op::I32_WRAP_I64)
                .g(1)
                .i64c(8)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .load64(0);
        }
        H::ListGetF64 => {
            list_bounds_check(fb);
            fb.g(0)
                .load64(LIST_DATA as u32)
                .op(op::I32_WRAP_I64)
                .g(1)
                .i64c(8)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .loadf64(0);
        }
        H::ListGetPtr => {
            list_bounds_check(fb);
            fb.g(0)
                .load64(LIST_DATA as u32)
                .op(op::I32_WRAP_I64)
                .g(1)
                .i64c(8)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .load64(0)
                .op(op::I32_WRAP_I64);
        }
        H::ListSetI64 => {
            list_bounds_check(fb);
            let addr = fb.scratch(Val::I32);
            fb.g(0)
                .load64(LIST_DATA as u32)
                .op(op::I32_WRAP_I64)
                .g(1)
                .i64c(8)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .s(addr);
            fb.g(addr).g(2).store64(0);
        }
        H::ListSetF64 => {
            list_bounds_check(fb);
            let addr = fb.scratch(Val::I32);
            fb.g(0)
                .load64(LIST_DATA as u32)
                .op(op::I32_WRAP_I64)
                .g(1)
                .i64c(8)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .s(addr);
            fb.g(addr).g(2).storef64(0);
        }
        H::ListSetPtr => {
            // Releases the replaced element and retains the new one
            // (runtime-internal, mirroring `lpp_list_set_arc`).
            list_bounds_check(fb);
            let addr = fb.scratch(Val::I32);
            fb.g(0)
                .load64(LIST_DATA as u32)
                .op(op::I32_WRAP_I64)
                .g(1)
                .i64c(8)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .s(addr);
            fb.g(addr)
                .load64(0)
                .op(op::I32_WRAP_I64)
                .call(idx(H::Release));
            // Retain passes the value through; drop the pass-through
            // (the store below re-pushes it).
            fb.g(2).call(idx(H::Retain)).op(op::DROP);
            // Stores pop (value, addr): address first, value on top.
            fb.g(addr);
            fb.g(2).op(op::I64_EXTEND_I32_U).store64(0);
        }
        // ── borrowed list/string slices ──────────────────────────────
        H::SliceNewList => {
            slice_new_body(fb, idx(H::ListLen), idx(H::Alloc));
        }
        H::SliceNewStr => {
            slice_new_body(fb, idx(H::StrLen), idx(H::Alloc));
        }
        H::SliceLen => {
            fb.g(0).op(op::I32_EQZ).if_().op(op::UNREACHABLE).end();
            fb.g(0).load64(SLICE_LEN as u32);
        }
        H::SliceGetI64 => {
            slice_source_and_index(fb);
            fb.call(idx(H::ListGetI64));
        }
        H::SliceGetF64 => {
            slice_source_and_index(fb);
            fb.call(idx(H::ListGetF64));
        }
        H::SliceGetPtr => {
            slice_source_and_index(fb);
            fb.call(idx(H::ListGetPtr));
        }
        H::StrSliceGet => {
            slice_bounds_check(fb);
            let source = fb.scratch(Val::I32);
            let absolute = fb.scratch(Val::I64);
            let base = fb.scratch(Val::I32);
            let payload = fb.scratch(Val::I32);
            fb.g(0).load32(SLICE_BASE as u32).s(source);
            fb.g(0)
                .load64(SLICE_START as u32)
                .g(1)
                .op(op::I64_ADD)
                .s(absolute);
            fb.i32c(29).call(idx(H::Alloc)).s(base);
            fb.g(base).i64c(IMMORTAL_RC).store64(0);
            fb.g(base).i64c(0).store64(8);
            fb.g(base).i64c(ARC_MAGIC).store64(16);
            fb.g(base)
                .i32c(STR_HEADER as i64)
                .op(op::I32_ADD)
                .s(payload);
            fb.g(payload).i32c(1).store32(0);
            fb.g(payload).i32c(4).op(op::I32_ADD);
            fb.g(source)
                .i32c(4)
                .op(op::I32_ADD)
                .g(absolute)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .load8(0);
            fb.store8(0);
            fb.g(payload);
        }
        H::StrSliceToStr => {
            let source = fb.scratch(Val::I32);
            let start = fb.scratch(Val::I64);
            let length = fb.scratch(Val::I64);
            let base = fb.scratch(Val::I32);
            let payload = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I64);
            fb.g(0).op(op::I32_EQZ).if_().op(op::UNREACHABLE).end();
            fb.g(0).load32(SLICE_BASE as u32).s(source);
            fb.g(0).load64(SLICE_START as u32).s(start);
            fb.g(0).load64(SLICE_LEN as u32).s(length);
            fb.i32c(28)
                .g(length)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .call(idx(H::Alloc))
                .s(base);
            fb.g(base).i64c(IMMORTAL_RC).store64(0);
            fb.g(base).i64c(0).store64(8);
            fb.g(base).i64c(ARC_MAGIC).store64(16);
            fb.g(base)
                .i32c(STR_HEADER as i64)
                .op(op::I32_ADD)
                .s(payload);
            fb.g(payload).g(length).op(op::I32_WRAP_I64).store32(0);
            fb.i64c(0).s(i);
            fb.loop_();
            fb.g(i).g(length).op(op::I64_LT_U).if_();
            fb.g(payload)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD);
            fb.g(source)
                .i32c(4)
                .op(op::I32_ADD)
                .g(start)
                .g(i)
                .op(op::I64_ADD)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .load8(0);
            fb.store8(0);
            fb.end();
            fb.g(i).i64c(1).op(op::I64_ADD).s(i);
            fb.g(i).g(length).op(op::I64_LT_U).br_if(0);
            fb.end();
            fb.g(payload);
        }
        // ── deterministic Int/String-key maps with Int values ────────
        H::MapNew => {
            let map = fb.scratch(Val::I32);
            fb.i32c(MAP_SIZE).call(idx(H::Alloc)).s(map);
            fb.g(map).i64c(0).store64(MAP_DATA as u32);
            fb.g(map).i64c(0).store64(MAP_LEN as u32);
            fb.g(map).i64c(0).store64(MAP_CAP as u32);
            fb.g(map)
                .g(0)
                .op(op::I64_EXTEND_I32_U)
                .store64(MAP_STR_KEYS as u32);
            fb.g(map);
        }
        H::MapEnsure => {
            let cap = fb.scratch(Val::I64);
            let new_cap = fb.scratch(Val::I64);
            let old_data = fb.scratch(Val::I32);
            let new_data = fb.scratch(Val::I32);
            let len = fb.scratch(Val::I64);
            let i = fb.scratch(Val::I64);
            fb.g(0).load64(MAP_LEN as u32).s(len);
            fb.g(0).load64(MAP_CAP as u32).t(cap);
            // `cap` remains on the stack from `tee`; trap only if `cap < len`.
            fb.g(len).op(op::I64_LT_U).if_().op(op::UNREACHABLE).end();
            fb.g(len).g(cap).op(op::I64_LT_U).if_().op(op::RETURN).end();
            fb.g(cap).op(op::I64_EQZ).if_();
            fb.i64c(MAP_INITIAL_CAP).s(new_cap);
            fb.else_();
            fb.g(cap).i64c(2).op(op::I64_MUL).s(new_cap);
            fb.end();
            fb.g(0)
                .load64(MAP_DATA as u32)
                .op(op::I32_WRAP_I64)
                .s(old_data);
            fb.g(new_cap)
                .i64c(MAP_ENTRY_SIZE)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .call(idx(H::Alloc))
                .s(new_data);
            fb.i64c(0).s(i);
            fb.loop_();
            fb.g(i).g(len).op(op::I64_LT_U).if_();
            fb.g(new_data)
                .g(i)
                .i64c(MAP_ENTRY_SIZE)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD);
            fb.g(old_data)
                .g(i)
                .i64c(MAP_ENTRY_SIZE)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .load64(0);
            fb.store64(0);
            fb.g(new_data)
                .g(i)
                .i64c(MAP_ENTRY_SIZE)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD);
            fb.g(old_data)
                .g(i)
                .i64c(MAP_ENTRY_SIZE)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .load64(8);
            fb.store64(8);
            fb.end();
            fb.g(i).i64c(1).op(op::I64_ADD).s(i);
            fb.g(i).g(len).op(op::I64_LT_U).br_if(0);
            fb.end();
            fb.g(0)
                .g(new_data)
                .op(op::I64_EXTEND_I32_U)
                .store64(MAP_DATA as u32);
            fb.g(0).g(new_cap).store64(MAP_CAP as u32);
        }
        H::MapFindI64 | H::MapFindStr => {
            let len = fb.scratch(Val::I64);
            let data = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I64);
            let entry = fb.scratch(Val::I32);
            fb.g(0).op(op::I32_EQZ).if_().i64c(-1).op(op::RETURN).end();
            fb.g(0).load64(MAP_LEN as u32).s(len);
            fb.g(0).load64(MAP_DATA as u32).op(op::I32_WRAP_I64).s(data);
            fb.i64c(0).s(i);
            fb.block();
            fb.loop_();
            fb.g(i).g(len).op(op::I64_GE_U).br_if(1);
            fb.g(data)
                .g(i)
                .i64c(MAP_ENTRY_SIZE)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .s(entry);
            if h == H::MapFindStr {
                fb.g(entry)
                    .load64(0)
                    .op(op::I32_WRAP_I64)
                    .g(1)
                    .call(idx(H::StrEq));
            } else {
                fb.g(entry).load64(0).g(1).op(op::I64_EQ);
            }
            fb.if_().g(i).op(op::RETURN).end();
            fb.g(i).i64c(1).op(op::I64_ADD).s(i);
            fb.br(0);
            fb.end();
            fb.end();
            fb.i64c(-1);
        }
        H::MapPutI64 | H::MapPutStr => {
            let found = fb.scratch(Val::I64);
            let len = fb.scratch(Val::I64);
            let data = fb.scratch(Val::I32);
            let entry = fb.scratch(Val::I32);
            fb.g(0)
                .g(1)
                .call(idx(if h == H::MapPutStr {
                    H::MapFindStr
                } else {
                    H::MapFindI64
                }))
                .s(found);
            fb.g(found).i64c(0).op(op::I64_GE_S).if_();
            fb.g(0)
                .load64(MAP_DATA as u32)
                .op(op::I32_WRAP_I64)
                .g(found)
                .i64c(MAP_ENTRY_SIZE)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .g(2)
                .store64(8);
            fb.else_();
            fb.g(0).call(idx(H::MapEnsure));
            fb.g(0).load64(MAP_LEN as u32).s(len);
            fb.g(0).load64(MAP_DATA as u32).op(op::I32_WRAP_I64).s(data);
            fb.g(data)
                .g(len)
                .i64c(MAP_ENTRY_SIZE)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .s(entry);
            fb.g(entry);
            fb.g(1);
            if h == H::MapPutStr {
                fb.op(op::I64_EXTEND_I32_U);
            }
            fb.store64(0);
            fb.g(entry).g(2).store64(8);
            fb.g(0)
                .g(len)
                .i64c(1)
                .op(op::I64_ADD)
                .store64(MAP_LEN as u32);
            fb.end();
        }
        H::MapGetI64 | H::MapGetStr => {
            let found = fb.scratch(Val::I64);
            fb.g(0)
                .g(1)
                .call(idx(if h == H::MapGetStr {
                    H::MapFindStr
                } else {
                    H::MapFindI64
                }))
                .s(found);
            fb.g(found).i64c(0).op(op::I64_LT_S).if_();
            fb.i64c(0).op(op::RETURN);
            fb.end();
            fb.g(0)
                .load64(MAP_DATA as u32)
                .op(op::I32_WRAP_I64)
                .g(found)
                .i64c(MAP_ENTRY_SIZE)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .load64(8);
        }
        H::MapHasI64 | H::MapHasStr => {
            fb.g(0).g(1).call(idx(if h == H::MapHasStr {
                H::MapFindStr
            } else {
                H::MapFindI64
            }));
            fb.i64c(0).op(op::I64_GE_S).op(op::I64_EXTEND_I32_U);
        }
        H::MapRemoveI64 | H::MapRemoveStr => {
            let found = fb.scratch(Val::I64);
            let last = fb.scratch(Val::I64);
            let data = fb.scratch(Val::I32);
            let destination = fb.scratch(Val::I32);
            let source = fb.scratch(Val::I32);
            fb.g(0)
                .g(1)
                .call(idx(if h == H::MapRemoveStr {
                    H::MapFindStr
                } else {
                    H::MapFindI64
                }))
                .s(found);
            fb.g(found).i64c(0).op(op::I64_GE_S).if_();
            fb.g(0)
                .load64(MAP_LEN as u32)
                .i64c(1)
                .op(op::I64_SUB)
                .s(last);
            fb.g(0).load64(MAP_DATA as u32).op(op::I32_WRAP_I64).s(data);
            fb.g(found).g(last).op(op::I64_NE).if_();
            fb.g(data)
                .g(found)
                .i64c(MAP_ENTRY_SIZE)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .s(destination);
            fb.g(data)
                .g(last)
                .i64c(MAP_ENTRY_SIZE)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .s(source);
            fb.g(destination).g(source).load64(0).store64(0);
            fb.g(destination).g(source).load64(8).store64(8);
            fb.end();
            fb.g(0).g(last).store64(MAP_LEN as u32);
            fb.end();
        }
        H::MapLen => {
            fb.g(0).op(op::I32_EQZ).if_().i64c(0).op(op::RETURN).end();
            fb.g(0).load64(MAP_LEN as u32);
        }
        // ── 5D2b slice 3: the task runtime ──
        H::TaskNew => {
            // NULL code or environment exits 101 (shim parity), then a
            // 32-byte ARC node: {code i64@0, env i64@8, result i64@16,
            // state i32@24, result_managed i32@28}.
            fb.g(0).i32c(0).op(op::I32_EQ);
            fb.g(1).i32c(0).op(op::I32_EQ);
            fb.op(op::I32_OR);
            fb.if_();
            fb.i32c(101);
            fb.call(proc_exit_import_idx);
            fb.end();
            fb.i32c(32);
            fb.i32c(env.task_drop_slot.expect("task slot planned") as i64);
            fb.call(idx(H::ArcAlloc));
            let node = fb.scratch(Val::I32);
            fb.t(node);
            fb.g(node).g(0).op(op::I64_EXTEND_I32_S).store64(0);
            fb.g(node).g(1).op(op::I64_EXTEND_I32_S).store64(8);
            fb.g(node).i64c(0).store64(16);
            fb.g(node).i32c(0).store32(24);
            fb.g(node).g(2).store32(28);
            // The ArcAlloc result (the tee'd node pointer) is the
            // result: widen it to the i64 result word.
            fb.op(op::I64_EXTEND_I32_U);
        }
        H::TaskPoll => {
            // state 0→1, run the task code, store the result, state 2.
            // Polling a non-fresh (or NULL) task exits 101.
            fb.g(0).i32c(0).op(op::I32_EQ);
            fb.g(0).load32(24).i32c(0).op(op::I32_NE);
            fb.op(op::I32_OR);
            fb.if_();
            fb.i32c(101);
            fb.call(proc_exit_import_idx);
            fb.end();
            fb.g(0).i32c(1).store32(24);
            // Task-code call: the thunk takes (env) -> result word;
            // the callee's dispatch index (the stored code word) is
            // the call_indirect element index.
            // Task-code call: thunk (env) -> result word, element
            // index = the stored code word (on top of the stack).
            // Task-code call: thunk (env) -> result word; the
            // stored code word is the call_indirect element index
            // (pushed last = on top of the stack).
            fb.g(0).load64(8).op(op::I32_WRAP_I64);
            fb.g(0).load64(0).op(op::I32_WRAP_I64);
            fb.call_indirect(env.task_code_call_type, env.dispatch_table);
            let result = fb.scratch(Val::I64);
            // Tee the word, DROP the stack copy (the local is the
            // canonical slot), store it into the node, and return a
            // re-read of the local — the exit stack is exactly the
            // one return word.
            fb.t(result);
            fb.op(op::DROP);
            fb.g(0).g(result).store64(16);
            fb.g(0).i32c(2).store32(24);
            fb.g(result);
        }
        H::TaskAwait => {
            // A NULL task (or one with a NULL env) exits 101; a fresh
            // task is polled; a managed result is retained for the
            // awaiter (the task keeps its share).
            fb.g(0).i32c(0).op(op::I32_EQ);
            fb.g(0).load64(8).op(op::I64_EQZ);
            fb.op(op::I32_OR);
            fb.if_();
            fb.i32c(101);
            fb.call(proc_exit_import_idx);
            fb.end();
            fb.g(0).load32(24).i32c(0).op(op::I32_EQ);
            fb.if_();
            fb.g(0).call(idx(H::TaskPoll)).op(op::DROP);
            fb.end();
            fb.g(0).load32(28).i32c(0).op(op::I32_NE);
            fb.if_();
            fb.g(0).load64(16).op(op::I32_WRAP_I64).call(idx(H::Retain));
            // Retain passes the value through; the result word is
            // re-read below, so drop the retained copy.
            fb.op(op::DROP);
            fb.end();
            fb.g(0).load64(16);
        }
        H::TaskDestroy => {
            // Release the task node (its destructor frees the env
            // tuple and, once resolved, the managed result).
            fb.g(0).call(idx(H::Release));
        }
        // ── 5D2b slice 4, batch 3: string builtins ────────────────────
        // A new string is an immortal ARC node: `[rc][drop 0][magic]`
        // + `[len i32][utf8]`, returned as `base + 24`. The output
        // is composed into the NUM_BUF scratch first, then copied.
        H::StrConcat => {
            let la = fb.scratch(Val::I32);
            let lb = fb.scratch(Val::I32);
            let rlen = fb.scratch(Val::I32);
            let base = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I32);
            fb.g(0).load32(0).s(la);
            fb.g(1).load32(0).s(lb);
            fb.g(la).g(lb).op(op::I32_ADD).s(rlen);
            fb.i32c(28)
                .g(rlen)
                .op(op::I32_ADD)
                .call(idx(H::Alloc))
                .s(base);
            fb.g(base).i64c(IMMORTAL_RC).store64(0);
            fb.g(base).i64c(0).store64(8);
            fb.g(base).i64c(ARC_MAGIC).store64(16);
            fb.g(base).g(rlen).store32(24);
            // left bytes
            fb.i32c(0).s(i);
            fb.loop_(); // $c
            fb.g(i).g(la).op(op::I32_LT_U).if_(); // $in
            fb.g(base).i32c(28).op(op::I32_ADD).g(i).op(op::I32_ADD);
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0);
            fb.store8(0);
            fb.end(); // $in
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.g(i).g(la).op(op::I32_LT_U).br_if(0); // $c
            fb.end(); // $c
            // right bytes
            fb.i32c(0).s(i);
            fb.loop_(); // $c
            fb.g(i).g(lb).op(op::I32_LT_U).if_(); // $in
            fb.g(base)
                .i32c(28)
                .op(op::I32_ADD)
                .g(la)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD);
            fb.g(1)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0);
            fb.store8(0);
            fb.end(); // $in
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.g(i).g(lb).op(op::I32_LT_U).br_if(0); // $c
            fb.end(); // $c
            fb.g(base).i32c(24).op(op::I32_ADD);
        }
        H::StrSubstr => {
            let slen = fb.scratch(Val::I32);
            let copy = fb.scratch(Val::I32);
            let start = fb.scratch(Val::I32);
            let out = fb.scratch(Val::I32);
            fb.g(0).load32(0).s(slen);
            fb.i32c(0).s(copy);
            fb.i32c(0).s(start);
            fb.g(1).i64c(0).op(op::I64_GE_S);
            fb.g(2).i64c(0).op(op::I64_GT_S).op(op::I32_AND);
            fb.g(1)
                .g(slen)
                .op(op::I64_EXTEND_I32_U)
                .op(op::I64_LT_U)
                .op(op::I32_AND)
                .if_();
            fb.g(1).op(op::I32_WRAP_I64).s(start);
            fb.g(slen).g(start).op(op::I32_SUB).s(copy);
            fb.g(2)
                .g(copy)
                .op(op::I64_EXTEND_I32_U)
                .op(op::I64_LT_U)
                .if_();
            fb.g(2).op(op::I32_WRAP_I64).s(copy);
            fb.end();
            fb.end();
            fb.g(copy)
                .i32c(4)
                .op(op::I32_ADD)
                .i32c(0)
                .call(idx(H::ArcAlloc))
                .s(out);
            fb.g(out).g(copy).store32(0);
            fb.g(out).i32c(4).op(op::I32_ADD);
            fb.g(0).i32c(4).op(op::I32_ADD).g(start).op(op::I32_ADD);
            fb.g(copy);
            fb.memory_copy();
            fb.g(out);
        }
        H::StrRepeat => {
            let slen = fb.scratch(Val::I32);
            let total = fb.scratch(Val::I64);
            let out = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I64);
            fb.g(0).load32(0).s(slen);
            fb.i64c(0).s(total);
            fb.g(1).i64c(0).op(op::I64_GT_S).if_();
            fb.g(slen)
                .op(op::I64_EXTEND_I32_U)
                .g(1)
                .op(op::I64_MUL)
                .s(total);
            fb.g(total).i64c(0x7fff_fff0).op(op::I64_GT_U).if_();
            fb.op(op::UNREACHABLE);
            fb.end();
            fb.end();
            fb.g(total)
                .op(op::I32_WRAP_I64)
                .i32c(4)
                .op(op::I32_ADD)
                .i32c(0)
                .call(idx(H::ArcAlloc))
                .s(out);
            fb.g(out).g(total).op(op::I32_WRAP_I64).store32(0);
            fb.g(total).op(op::I64_EQZ).if_();
            fb.g(out).op(op::RETURN);
            fb.end();
            fb.i64c(0).s(i);
            fb.block();
            fb.loop_();
            fb.g(i).g(1).op(op::I64_GE_S).br_if(1);
            fb.g(out).i32c(4).op(op::I32_ADD);
            fb.g(i)
                .g(slen)
                .op(op::I64_EXTEND_I32_U)
                .op(op::I64_MUL)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD);
            fb.g(0).i32c(4).op(op::I32_ADD);
            fb.g(slen);
            fb.memory_copy();
            fb.g(i).i64c(1).op(op::I64_ADD).s(i);
            fb.br(0);
            fb.end();
            fb.end();
            fb.g(out);
        }
        H::StrSplit => {
            let list = fb.scratch(Val::I32);
            let len = fb.scratch(Val::I32);
            let start = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I32);
            let piece_len = fb.scratch(Val::I32);
            let piece = fb.scratch(Val::I32);
            let boundary = fb.scratch(Val::I32);
            fb.i32c(1).call(idx(H::ListNew)).s(list);
            fb.g(0).load32(0).s(len);
            fb.i32c(0).s(start);
            fb.i32c(0).s(i);
            fb.block();
            fb.loop_();
            fb.g(i).g(len).op(op::I32_EQ).s(boundary);
            fb.g(i).g(len).op(op::I32_LT_U).if_();
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0)
                .g(1)
                .op(op::I32_WRAP_I64)
                .op(op::I32_EQ)
                .if_();
            fb.i32c(1).s(boundary);
            fb.end();
            fb.end();
            fb.g(boundary).if_();
            fb.g(i).g(start).op(op::I32_SUB).s(piece_len);
            fb.g(piece_len)
                .i32c(4)
                .op(op::I32_ADD)
                .i32c(0)
                .call(idx(H::ArcAlloc))
                .s(piece);
            fb.g(piece).g(piece_len).store32(0);
            fb.g(piece).i32c(4).op(op::I32_ADD);
            fb.g(0).i32c(4).op(op::I32_ADD).g(start).op(op::I32_ADD);
            fb.g(piece_len);
            fb.memory_copy();
            fb.g(list).g(piece).call(idx(H::ListPushPtr));
            fb.g(i).i32c(1).op(op::I32_ADD).s(start);
            fb.end();
            fb.g(i).g(len).op(op::I32_EQ).br_if(1);
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.br(0);
            fb.end();
            fb.end();
            fb.g(list);
        }
        H::CharAt => {
            let len = fb.scratch(Val::I32);
            let out_len = fb.scratch(Val::I32);
            let out = fb.scratch(Val::I32);
            fb.g(0).load32(0).s(len);
            fb.i32c(0).s(out_len);
            fb.g(1).i64c(0).op(op::I64_GE_S);
            fb.g(1)
                .g(len)
                .op(op::I64_EXTEND_I32_U)
                .op(op::I64_LT_U)
                .op(op::I32_AND)
                .if_();
            fb.i32c(1).s(out_len);
            fb.end();
            fb.g(out_len)
                .i32c(4)
                .op(op::I32_ADD)
                .i32c(0)
                .call(idx(H::ArcAlloc))
                .s(out);
            fb.g(out).g(out_len).store32(0);
            fb.g(out_len).if_();
            fb.g(out).i32c(4).op(op::I32_ADD);
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(1)
                .op(op::I32_WRAP_I64)
                .op(op::I32_ADD)
                .load8(0);
            fb.store8(0);
            fb.end();
            fb.g(out);
        }
        H::Ord => {
            let len = fb.scratch(Val::I32);
            let lead = fb.scratch(Val::I32);
            let extra = fb.scratch(Val::I32);
            let code = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I32);
            let byte = fb.scratch(Val::I32);
            fb.g(0).load32(0).s(len);
            fb.g(len).op(op::I32_EQZ).if_();
            fb.i64c(0).op(op::RETURN);
            fb.end();
            fb.g(0).i32c(4).op(op::I32_ADD).load8(0).s(lead);
            fb.g(lead).i32c(0x80).op(op::I32_LT_U).if_();
            fb.g(lead).op(op::I64_EXTEND_I32_U).op(op::RETURN);
            fb.end();
            fb.g(lead)
                .i32c(0xe0)
                .op(op::I32_AND)
                .i32c(0xc0)
                .op(op::I32_EQ)
                .if_();
            fb.i32c(1).s(extra);
            fb.g(lead).i32c(0x1f).op(op::I32_AND).s(code);
            fb.else_();
            fb.g(lead)
                .i32c(0xf0)
                .op(op::I32_AND)
                .i32c(0xe0)
                .op(op::I32_EQ)
                .if_();
            fb.i32c(2).s(extra);
            fb.g(lead).i32c(0x0f).op(op::I32_AND).s(code);
            fb.else_();
            fb.g(lead)
                .i32c(0xf8)
                .op(op::I32_AND)
                .i32c(0xf0)
                .op(op::I32_EQ)
                .if_();
            fb.i32c(3).s(extra);
            fb.g(lead).i32c(0x07).op(op::I32_AND).s(code);
            fb.else_();
            fb.g(lead).op(op::I64_EXTEND_I32_U).op(op::RETURN);
            fb.end();
            fb.end();
            fb.end();
            fb.g(len).g(extra).op(op::I32_LE_U).if_();
            fb.g(lead).op(op::I64_EXTEND_I32_U).op(op::RETURN);
            fb.end();
            fb.i32c(1).s(i);
            fb.block();
            fb.loop_();
            fb.g(i).g(extra).op(op::I32_GT_U).br_if(1);
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0)
                .s(byte);
            fb.g(byte)
                .i32c(0xc0)
                .op(op::I32_AND)
                .i32c(0x80)
                .op(op::I32_NE)
                .if_();
            fb.g(lead).op(op::I64_EXTEND_I32_U).op(op::RETURN);
            fb.end();
            fb.g(code)
                .i32c(6)
                .op(op::I32_SHL)
                .g(byte)
                .i32c(0x3f)
                .op(op::I32_AND)
                .op(op::I32_OR)
                .s(code);
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.br(0);
            fb.end();
            fb.end();
            // Reject overlong encodings, surrogates, and out-of-range scalars.
            fb.g(extra).i32c(1).op(op::I32_EQ);
            fb.g(code).i32c(0x80).op(op::I32_LT_U).op(op::I32_AND);
            fb.g(extra).i32c(2).op(op::I32_EQ);
            fb.g(code).i32c(0x800).op(op::I32_LT_U).op(op::I32_AND);
            fb.op(op::I32_OR);
            fb.g(extra).i32c(3).op(op::I32_EQ);
            fb.g(code).i32c(0x10000).op(op::I32_LT_U).op(op::I32_AND);
            fb.op(op::I32_OR);
            fb.g(code).i32c(0x10ffff).op(op::I32_GT_U).op(op::I32_OR);
            fb.g(code).i32c(0xd800).op(op::I32_GE_U);
            fb.g(code).i32c(0xdfff).op(op::I32_LE_U).op(op::I32_AND);
            fb.op(op::I32_OR).if_();
            fb.g(lead).op(op::I64_EXTEND_I32_U).op(op::RETURN);
            fb.end();
            fb.g(code).op(op::I64_EXTEND_I32_U);
        }
        H::Chr => {
            let valid = fb.scratch(Val::I32);
            let code = fb.scratch(Val::I32);
            let len = fb.scratch(Val::I32);
            let out = fb.scratch(Val::I32);
            fb.g(0).i64c(0).op(op::I64_GE_S);
            fb.g(0)
                .i64c(0x10ffff)
                .op(op::I64_LE_U)
                .op(op::I32_AND)
                .s(valid);
            fb.g(0).op(op::I32_WRAP_I64).s(code);
            fb.g(code).i32c(0xd800).op(op::I32_GE_U);
            fb.g(code)
                .i32c(0xdfff)
                .op(op::I32_LE_U)
                .op(op::I32_AND)
                .if_();
            fb.i32c(0).s(valid);
            fb.end();
            fb.i32c(0).s(len);
            fb.g(valid).if_();
            fb.g(code).i32c(0x80).op(op::I32_LT_U).if_();
            fb.i32c(1).s(len);
            fb.else_();
            fb.g(code).i32c(0x800).op(op::I32_LT_U).if_();
            fb.i32c(2).s(len);
            fb.else_();
            fb.g(code).i32c(0x10000).op(op::I32_LT_U).if_();
            fb.i32c(3).s(len);
            fb.else_();
            fb.i32c(4).s(len);
            fb.end();
            fb.end();
            fb.end();
            fb.end();
            fb.g(len)
                .i32c(4)
                .op(op::I32_ADD)
                .i32c(0)
                .call(idx(H::ArcAlloc))
                .s(out);
            fb.g(out).g(len).store32(0);
            fb.g(len).i32c(1).op(op::I32_EQ).if_();
            fb.g(out).i32c(4).op(op::I32_ADD).g(code).store8(0);
            fb.end();
            fb.g(len).i32c(2).op(op::I32_EQ).if_();
            fb.g(out).i32c(4).op(op::I32_ADD);
            fb.g(code)
                .i32c(6)
                .op(op::I32_SHR_U)
                .i32c(0xc0)
                .op(op::I32_OR)
                .store8(0);
            fb.g(out).i32c(5).op(op::I32_ADD);
            fb.g(code)
                .i32c(0x3f)
                .op(op::I32_AND)
                .i32c(0x80)
                .op(op::I32_OR)
                .store8(0);
            fb.end();
            fb.g(len).i32c(3).op(op::I32_EQ).if_();
            fb.g(out).i32c(4).op(op::I32_ADD);
            fb.g(code)
                .i32c(12)
                .op(op::I32_SHR_U)
                .i32c(0xe0)
                .op(op::I32_OR)
                .store8(0);
            fb.g(out).i32c(5).op(op::I32_ADD);
            fb.g(code)
                .i32c(6)
                .op(op::I32_SHR_U)
                .i32c(0x3f)
                .op(op::I32_AND)
                .i32c(0x80)
                .op(op::I32_OR)
                .store8(0);
            fb.g(out).i32c(6).op(op::I32_ADD);
            fb.g(code)
                .i32c(0x3f)
                .op(op::I32_AND)
                .i32c(0x80)
                .op(op::I32_OR)
                .store8(0);
            fb.end();
            fb.g(len).i32c(4).op(op::I32_EQ).if_();
            fb.g(out).i32c(4).op(op::I32_ADD);
            fb.g(code)
                .i32c(18)
                .op(op::I32_SHR_U)
                .i32c(0xf0)
                .op(op::I32_OR)
                .store8(0);
            fb.g(out).i32c(5).op(op::I32_ADD);
            fb.g(code)
                .i32c(12)
                .op(op::I32_SHR_U)
                .i32c(0x3f)
                .op(op::I32_AND)
                .i32c(0x80)
                .op(op::I32_OR)
                .store8(0);
            fb.g(out).i32c(6).op(op::I32_ADD);
            fb.g(code)
                .i32c(6)
                .op(op::I32_SHR_U)
                .i32c(0x3f)
                .op(op::I32_AND)
                .i32c(0x80)
                .op(op::I32_OR)
                .store8(0);
            fb.g(out).i32c(7).op(op::I32_ADD);
            fb.g(code)
                .i32c(0x3f)
                .op(op::I32_AND)
                .i32c(0x80)
                .op(op::I32_OR)
                .store8(0);
            fb.end();
            fb.g(out);
        }
        H::StrContains => {
            let la = fb.scratch(Val::I32);
            let lb = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I32);
            let j = fb.scratch(Val::I32);
            fb.g(0).load32(0).s(la);
            fb.g(1).load32(0).s(lb);
            fb.g(lb).i32c(0).op(op::I32_EQ).if_();
            fb.i32c(1);
            fb.op(op::RETURN);
            fb.end();
            fb.g(lb).g(la).op(op::I32_GT_U).if_();
            fb.i32c(0);
            fb.op(op::RETURN);
            fb.end();
            fb.i32c(0).s(i);
            fb.block(); // $outer
            fb.loop_(); // $outl
            fb.g(i)
                .g(la)
                .g(lb)
                .op(op::I32_SUB)
                .op(op::I32_GT_S)
                .br_if(1); // exhausted: exit $outer
            fb.i32c(0).s(j);
            fb.block(); // $inner
            fb.loop_(); // $inl
            fb.g(j).g(lb).op(op::I32_GE_U).br_if(1); // full match: exit $inner
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .g(j)
                .op(op::I32_ADD)
                .load8(0);
            fb.g(1)
                .i32c(4)
                .op(op::I32_ADD)
                .g(j)
                .op(op::I32_ADD)
                .load8(0)
                .op(op::I32_NE)
                .br_if(1); // mismatch: exit $inner
            fb.g(j).i32c(1).op(op::I32_ADD).s(j);
            fb.br(0); // $inl
            fb.end(); // $inl
            fb.end(); // $inner
            fb.g(j).g(lb).op(op::I32_EQ).if_(); // matched
            fb.i32c(1);
            fb.op(op::RETURN);
            fb.end();
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.br(0); // $outl
            fb.end(); // $outl
            fb.end(); // $outer
            fb.i32c(0);
        }
        H::StrStartsWith => {
            let la = fb.scratch(Val::I32);
            let lb = fb.scratch(Val::I32);
            let j = fb.scratch(Val::I32);
            let ca = fb.scratch(Val::I32);
            let cb = fb.scratch(Val::I32);
            fb.g(0).load32(0).s(la);
            fb.g(1).load32(0).s(lb);
            fb.g(lb).g(la).op(op::I32_GT_U).if_();
            fb.i32c(0);
            fb.op(op::RETURN);
            fb.end();
            fb.i32c(0).s(j);
            fb.loop_(); // $c
            fb.g(j).g(lb).op(op::I32_LT_U).if_(); // $in
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(j)
                .op(op::I32_ADD)
                .load8(0)
                .s(ca);
            fb.g(1)
                .i32c(4)
                .op(op::I32_ADD)
                .g(j)
                .op(op::I32_ADD)
                .load8(0)
                .s(cb);
            fb.g(ca).g(cb).op(op::I32_NE).if_();
            fb.i32c(0);
            fb.op(op::RETURN);
            fb.end();
            fb.end(); // $in
            fb.g(j).i32c(1).op(op::I32_ADD).s(j);
            fb.g(j).g(lb).op(op::I32_LT_U).br_if(0); // $c
            fb.end(); // $c
            fb.i32c(1);
        }
        H::StrEndsWith => {
            let la = fb.scratch(Val::I32);
            let lb = fb.scratch(Val::I32);
            let off = fb.scratch(Val::I32);
            let j = fb.scratch(Val::I32);
            let ca = fb.scratch(Val::I32);
            let cb = fb.scratch(Val::I32);
            fb.g(0).load32(0).s(la);
            fb.g(1).load32(0).s(lb);
            fb.g(lb).g(la).op(op::I32_GT_U).if_();
            fb.i32c(0);
            fb.op(op::RETURN);
            fb.end();
            fb.g(la).g(lb).op(op::I32_SUB).s(off);
            fb.i32c(0).s(j);
            fb.loop_(); // $c
            fb.g(j).g(lb).op(op::I32_LT_U).if_(); // $in
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(off)
                .op(op::I32_ADD)
                .g(j)
                .op(op::I32_ADD)
                .load8(0)
                .s(ca);
            fb.g(1)
                .i32c(4)
                .op(op::I32_ADD)
                .g(j)
                .op(op::I32_ADD)
                .load8(0)
                .s(cb);
            fb.g(ca).g(cb).op(op::I32_NE).if_();
            fb.i32c(0);
            fb.op(op::RETURN);
            fb.end();
            fb.end(); // $in
            fb.g(j).i32c(1).op(op::I32_ADD).s(j);
            fb.g(j).g(lb).op(op::I32_LT_U).br_if(0); // $c
            fb.end(); // $c
            fb.i32c(1);
        }
        H::StrFind => {
            let la = fb.scratch(Val::I32);
            let lb = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I32);
            let j = fb.scratch(Val::I32);
            let ca = fb.scratch(Val::I32);
            let cb = fb.scratch(Val::I32);
            fb.g(0).load32(0).s(la);
            fb.g(1).load32(0).s(lb);
            fb.g(lb).i32c(0).op(op::I32_EQ).if_();
            fb.i64c(0);
            fb.op(op::RETURN);
            fb.end();
            fb.g(lb).g(la).op(op::I32_GT_U).if_();
            fb.i64c(-1);
            fb.op(op::RETURN);
            fb.end();
            fb.i32c(0).s(i);
            fb.block(); // $outer
            fb.loop_(); // $outl
            fb.g(i)
                .g(la)
                .g(lb)
                .op(op::I32_SUB)
                .op(op::I32_GT_S)
                .br_if(1); // exhausted: exit $outer
            fb.i32c(0).s(j);
            fb.block(); // $inner
            fb.loop_(); // $inl
            fb.g(j).g(lb).op(op::I32_GE_U).br_if(1); // full match: exit $inner
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .g(j)
                .op(op::I32_ADD)
                .load8(0)
                .s(ca);
            fb.g(1)
                .i32c(4)
                .op(op::I32_ADD)
                .g(j)
                .op(op::I32_ADD)
                .load8(0)
                .s(cb);
            fb.g(ca).g(cb).op(op::I32_NE).br_if(1); // mismatch: exit $inner
            fb.g(j).i32c(1).op(op::I32_ADD).s(j);
            fb.br(0); // $inl
            fb.end(); // $inl
            fb.end(); // $inner
            fb.g(j).g(lb).op(op::I32_EQ).if_(); // matched
            fb.g(i).op(op::I64_EXTEND_I32_U);
            fb.op(op::RETURN);
            fb.end();
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.br(0); // $outl
            fb.end(); // $outl
            fb.end(); // $outer
            fb.i64c(-1);
        }
        H::StrReplace => {
            let lt = fb.scratch(Val::I32);
            let lo = fb.scratch(Val::I32);
            let ln = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I32);
            let j = fb.scratch(Val::I32);
            let m = fb.scratch(Val::I32);
            let rlen = fb.scratch(Val::I32);
            let base = fb.scratch(Val::I32);
            let k = fb.scratch(Val::I32);
            let o = fb.scratch(Val::I32);
            let ca = fb.scratch(Val::I32);
            let cb = fb.scratch(Val::I32);
            fb.g(0).load32(0).s(lt);
            fb.g(1).load32(0).s(lo);
            fb.g(2).load32(0).s(ln);
            // The oracle returns the text unchanged when old is empty. The
            // result is a distinct owner, so retain the aliased allocation.
            fb.g(lo).i32c(0).op(op::I32_EQ).if_();
            fb.g(0).call(idx(H::Retain));
            fb.op(op::RETURN);
            fb.end();
            // Pass 1: the result length.
            fb.i32c(0).s(i);
            fb.i32c(0).s(rlen);
            fb.loop_(); // $p1
            fb.g(i).g(lt).op(op::I32_LT_U).if_(); // $p1in
            fb.i32c(0).s(m);
            fb.g(i).g(lo).op(op::I32_ADD).g(lt).op(op::I32_LE_U).if_(); // $can
            fb.i32c(0).s(j);
            fb.i32c(1).s(m);
            fb.block(); // $match
            fb.loop_(); // $ml
            fb.g(j).g(lo).op(op::I32_GE_U).br_if(1); // full match
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .g(j)
                .op(op::I32_ADD)
                .load8(0)
                .s(ca);
            fb.g(1)
                .i32c(4)
                .op(op::I32_ADD)
                .g(j)
                .op(op::I32_ADD)
                .load8(0)
                .s(cb);
            fb.g(ca).g(cb).op(op::I32_NE).if_(); // $mis
            fb.i32c(0).s(m);
            fb.end(); // $mis
            fb.g(m).i32c(0).op(op::I32_EQ).br_if(1); // mismatch
            fb.g(j).i32c(1).op(op::I32_ADD).s(j);
            fb.br(0); // $ml
            fb.end(); // $ml
            fb.end(); // $match
            fb.end(); // $can
            fb.g(m).if_();
            fb.g(rlen).g(ln).op(op::I32_ADD).s(rlen);
            fb.g(i).g(lo).op(op::I32_ADD).s(i);
            fb.else_();
            fb.g(rlen).i32c(1).op(op::I32_ADD).s(rlen);
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.end();
            fb.br(1); // $p1
            fb.end(); // $p1in
            fb.end(); // $p1
            fb.i32c(28)
                .g(rlen)
                .op(op::I32_ADD)
                .call(idx(H::Alloc))
                .s(base);
            fb.g(base).i64c(IMMORTAL_RC).store64(0);
            fb.g(base).i64c(0).store64(8);
            fb.g(base).i64c(ARC_MAGIC).store64(16);
            fb.g(base).g(rlen).store32(24);
            // Pass 2: the copy.
            fb.i32c(0).s(i);
            fb.i32c(0).s(o);
            fb.loop_(); // $p2
            fb.g(i).g(lt).op(op::I32_LT_U).if_(); // $p2in
            fb.i32c(0).s(m);
            fb.g(i).g(lo).op(op::I32_ADD).g(lt).op(op::I32_LE_U).if_(); // $can
            fb.i32c(0).s(j);
            fb.i32c(1).s(m);
            fb.block(); // $match
            fb.loop_(); // $ml
            fb.g(j).g(lo).op(op::I32_GE_U).br_if(1); // full match
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .g(j)
                .op(op::I32_ADD)
                .load8(0)
                .s(ca);
            fb.g(1)
                .i32c(4)
                .op(op::I32_ADD)
                .g(j)
                .op(op::I32_ADD)
                .load8(0)
                .s(cb);
            fb.g(ca).g(cb).op(op::I32_NE).if_(); // $mis
            fb.i32c(0).s(m);
            fb.end(); // $mis
            fb.g(m).i32c(0).op(op::I32_EQ).br_if(1); // mismatch
            fb.g(j).i32c(1).op(op::I32_ADD).s(j);
            fb.br(0); // $ml
            fb.end(); // $ml
            fb.end(); // $match
            fb.end(); // $can
            fb.g(m).if_();
            fb.i32c(0).s(k);
            fb.loop_(); // $cpn
            fb.g(k).g(ln).op(op::I32_LT_U).if_(); // $cpnin
            fb.g(base)
                .i32c(28)
                .op(op::I32_ADD)
                .g(o)
                .op(op::I32_ADD)
                .g(k)
                .op(op::I32_ADD);
            fb.g(2)
                .i32c(4)
                .op(op::I32_ADD)
                .g(k)
                .op(op::I32_ADD)
                .load8(0);
            fb.store8(0);
            fb.end(); // $cpnin
            fb.g(k).i32c(1).op(op::I32_ADD).s(k);
            fb.g(k).g(ln).op(op::I32_LT_U).br_if(0); // $cpn
            fb.end(); // $cpn
            fb.g(o).g(ln).op(op::I32_ADD).s(o);
            fb.g(i).g(lo).op(op::I32_ADD).s(i);
            fb.else_();
            fb.g(base).i32c(28).op(op::I32_ADD).g(o).op(op::I32_ADD);
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0);
            fb.store8(0);
            fb.g(o).i32c(1).op(op::I32_ADD).s(o);
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.end();
            fb.br(1); // $p2
            fb.end(); // $p2in
            fb.end(); // $p2
            fb.g(base).i32c(24).op(op::I32_ADD);
        }
        H::StrTrim => {
            let la = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I32);
            let j = fb.scratch(Val::I32);
            let rlen = fb.scratch(Val::I32);
            let base = fb.scratch(Val::I32);
            let c = fb.scratch(Val::I32);
            let k = fb.scratch(Val::I32);
            fb.g(0).load32(0).s(la);
            // Skip leading spaces/tabs/newlines/carriage returns.
            fb.i32c(0).s(i);
            fb.loop_(); // $f
            fb.g(i).g(la).op(op::I32_LT_U).if_(); // $fin
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0)
                .s(c);
            fb.g(c)
                .i32c(32)
                .op(op::I32_EQ)
                .g(c)
                .i32c(9)
                .op(op::I32_EQ)
                .op(op::I32_OR)
                .g(c)
                .i32c(10)
                .op(op::I32_EQ)
                .op(op::I32_OR)
                .g(c)
                .i32c(13)
                .op(op::I32_EQ)
                .op(op::I32_OR);
            fb.if_(); // $ws
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.br(2); // continue $f
            fb.end(); // $ws
            fb.br(0); // break $f
            fb.end(); // $fin
            fb.end(); // $f
            // Skip the same set from the end.
            fb.g(la).s(j);
            fb.loop_(); // $b
            fb.g(j).i32c(0).op(op::I32_GT_U).if_(); // $bin
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(j)
                .i32c(1)
                .op(op::I32_SUB)
                .op(op::I32_ADD)
                .load8(0)
                .s(c);
            fb.g(c)
                .i32c(32)
                .op(op::I32_EQ)
                .g(c)
                .i32c(9)
                .op(op::I32_EQ)
                .op(op::I32_OR)
                .g(c)
                .i32c(10)
                .op(op::I32_EQ)
                .op(op::I32_OR)
                .g(c)
                .i32c(13)
                .op(op::I32_EQ)
                .op(op::I32_OR);
            fb.if_(); // $ws
            fb.g(j).i32c(1).op(op::I32_SUB).s(j);
            fb.br(2); // continue $b
            fb.end(); // $ws
            fb.br(0); // break $b
            fb.end(); // $bin
            fb.end(); // $b
            fb.g(j).g(i).op(op::I32_SUB).s(rlen);
            // All-whitespace text: i == la > 0 == j would underflow.
            fb.g(rlen).i32c(0).op(op::I32_LT_S).if_();
            fb.i32c(0).s(rlen);
            fb.end();
            fb.i32c(28)
                .g(rlen)
                .op(op::I32_ADD)
                .call(idx(H::Alloc))
                .s(base);
            fb.g(base).i64c(IMMORTAL_RC).store64(0);
            fb.g(base).i64c(0).store64(8);
            fb.g(base).i64c(ARC_MAGIC).store64(16);
            fb.g(base).g(rlen).store32(24);
            fb.i32c(0).s(k);
            fb.loop_(); // $cp
            fb.g(k).g(rlen).op(op::I32_LT_U).if_(); // $cpin
            fb.g(base).i32c(28).op(op::I32_ADD).g(k).op(op::I32_ADD);
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .g(k)
                .op(op::I32_ADD)
                .load8(0);
            fb.store8(0);
            fb.end(); // $cpin
            fb.g(k).i32c(1).op(op::I32_ADD).s(k);
            fb.g(k).g(rlen).op(op::I32_LT_U).br_if(0); // $cp
            fb.end(); // $cp
            fb.g(base).i32c(24).op(op::I32_ADD);
        }
        H::StrLower => {
            let la = fb.scratch(Val::I32);
            let base = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I32);
            let c = fb.scratch(Val::I32);
            fb.g(0).load32(0).s(la);
            fb.i32c(28)
                .g(la)
                .op(op::I32_ADD)
                .call(idx(H::Alloc))
                .s(base);
            fb.g(base).i64c(IMMORTAL_RC).store64(0);
            fb.g(base).i64c(0).store64(8);
            fb.g(base).i64c(ARC_MAGIC).store64(16);
            fb.g(base).g(la).store32(24);
            fb.i32c(0).s(i);
            fb.loop_(); // $c
            fb.g(i).g(la).op(op::I32_LT_U).if_(); // $in
            fb.g(base).i32c(28).op(op::I32_ADD).g(i).op(op::I32_ADD);
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0)
                .s(c);
            fb.g(c).i32c(32).op(op::I32_ADD);
            fb.g(c);
            fb.g(c)
                .i32c(65)
                .op(op::I32_GE_U)
                .g(c)
                .i32c(90)
                .op(op::I32_LE_U)
                .op(op::I32_AND);
            fb.op(op::SELECT);
            fb.store8(0);
            fb.end(); // $in
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.g(i).g(la).op(op::I32_LT_U).br_if(0); // $c
            fb.end(); // $c
            fb.g(base).i32c(24).op(op::I32_ADD);
        }
        H::StrUpper => {
            let la = fb.scratch(Val::I32);
            let base = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I32);
            let c = fb.scratch(Val::I32);
            fb.g(0).load32(0).s(la);
            fb.i32c(28)
                .g(la)
                .op(op::I32_ADD)
                .call(idx(H::Alloc))
                .s(base);
            fb.g(base).i64c(IMMORTAL_RC).store64(0);
            fb.g(base).i64c(0).store64(8);
            fb.g(base).i64c(ARC_MAGIC).store64(16);
            fb.g(base).g(la).store32(24);
            fb.i32c(0).s(i);
            fb.loop_(); // $c
            fb.g(i).g(la).op(op::I32_LT_U).if_(); // $in
            fb.g(base).i32c(28).op(op::I32_ADD).g(i).op(op::I32_ADD);
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0)
                .s(c);
            fb.g(c).i32c(32).op(op::I32_SUB);
            fb.g(c);
            fb.g(c)
                .i32c(97)
                .op(op::I32_GE_U)
                .g(c)
                .i32c(122)
                .op(op::I32_LE_U)
                .op(op::I32_AND);
            fb.op(op::SELECT);
            fb.store8(0);
            fb.end(); // $in
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.g(i).g(la).op(op::I32_LT_U).br_if(0); // $c
            fb.end(); // $c
            fb.g(base).i32c(24).op(op::I32_ADD);
        }
        H::IntToStr => {
            let neg = fb.scratch(Val::I32);
            let u = fb.scratch(Val::I64);
            let cnt = fb.scratch(Val::I32);
            let rlen = fb.scratch(Val::I32);
            let base = fb.scratch(Val::I32);
            let pos = fb.scratch(Val::I32);
            let d = fb.scratch(Val::I32);
            let u2 = fb.scratch(Val::I64);
            fb.g(0).i64c(0).op(op::I64_LT_S).s(neg);
            // u = |v| in u64 (wrapping negation handles i64::MIN).
            fb.i64c(0).g(0).op(op::I64_SUB);
            fb.g(0);
            fb.g(neg);
            fb.op(op::SELECT);
            fb.s(u);
            fb.i32c(1).s(cnt);
            fb.g(u).s(u2);
            fb.loop_(); // $dc
            fb.g(u2).i64c(10).op(op::I64_GE_U).if_(); // $dci
            fb.g(u2).i64c(10).op(op::I64_DIV_U).s(u2);
            fb.g(cnt).i32c(1).op(op::I32_ADD).s(cnt);
            fb.br(1); // continue $dc (br 0 from inside the if would EXIT the loop)
            fb.end(); // $dci
            fb.end(); // $dc
            fb.g(cnt).g(neg).op(op::I32_ADD).s(rlen);
            fb.i32c(28)
                .g(rlen)
                .op(op::I32_ADD)
                .call(idx(H::Alloc))
                .s(base);
            fb.g(base).i64c(IMMORTAL_RC).store64(0);
            fb.g(base).i64c(0).store64(8);
            fb.g(base).i64c(ARC_MAGIC).store64(16);
            fb.g(base).g(rlen).store32(24);
            fb.i32c(28)
                .g(rlen)
                .op(op::I32_ADD)
                .i32c(1)
                .op(op::I32_SUB)
                .s(pos); // offset, not address
            fb.block(); // $done
            fb.loop_(); // $dig
            fb.g(u).i64c(10).op(op::I64_REM_U).op(op::I32_WRAP_I64).s(d);
            fb.g(base).g(pos).op(op::I32_ADD);
            fb.g(d).i32c(48).op(op::I32_ADD);
            fb.store8(0);
            fb.g(pos).i32c(1).op(op::I32_SUB).s(pos);
            fb.g(u).i64c(10).op(op::I64_DIV_U).s(u);
            fb.g(u).op(op::I64_EQZ).br_if(1); // $done
            fb.br(0); // $dig
            fb.end(); // $dig
            fb.end(); // $done
            fb.g(neg).if_();
            fb.g(base).i32c(28).op(op::I32_ADD);
            fb.i32c(45);
            fb.store8(0);
            fb.end();
            fb.g(base).i32c(24).op(op::I32_ADD);
        }
        H::U64ToStr => {
            let u = fb.scratch(Val::I64);
            let u2 = fb.scratch(Val::I64);
            let cnt = fb.scratch(Val::I32);
            let rlen = fb.scratch(Val::I32);
            let base = fb.scratch(Val::I32);
            let pos = fb.scratch(Val::I32);
            let d = fb.scratch(Val::I32);
            fb.g(0).s(u);
            fb.i32c(1).s(cnt);
            fb.g(u).s(u2);
            fb.loop_(); // $dc
            fb.g(u2).i64c(10).op(op::I64_GE_U).if_(); // $dci
            fb.g(u2).i64c(10).op(op::I64_DIV_U).s(u2);
            fb.g(cnt).i32c(1).op(op::I32_ADD).s(cnt);
            fb.br(1); // continue $dc (br 0 from inside the if would EXIT the loop)
            fb.end(); // $dci
            fb.end(); // $dc
            fb.g(cnt).s(rlen);
            fb.i32c(28)
                .g(rlen)
                .op(op::I32_ADD)
                .call(idx(H::Alloc))
                .s(base);
            fb.g(base).i64c(IMMORTAL_RC).store64(0);
            fb.g(base).i64c(0).store64(8);
            fb.g(base).i64c(ARC_MAGIC).store64(16);
            fb.g(base).g(rlen).store32(24);
            fb.i32c(28)
                .g(rlen)
                .op(op::I32_ADD)
                .i32c(1)
                .op(op::I32_SUB)
                .s(pos); // offset, not address
            fb.block(); // $done
            fb.loop_(); // $dig
            fb.g(u).i64c(10).op(op::I64_REM_U).op(op::I32_WRAP_I64).s(d);
            fb.g(base).g(pos).op(op::I32_ADD);
            fb.g(d).i32c(48).op(op::I32_ADD);
            fb.store8(0);
            fb.g(pos).i32c(1).op(op::I32_SUB).s(pos);
            fb.g(u).i64c(10).op(op::I64_DIV_U).s(u);
            fb.g(u).op(op::I64_EQZ).br_if(1); // $done
            fb.br(0); // $dig
            fb.end(); // $dig
            fb.end(); // $done
            fb.g(base).i32c(24).op(op::I32_ADD);
        }
        H::U64ToHex => {
            let u = fb.scratch(Val::I64);
            let u2 = fb.scratch(Val::I64);
            let cnt = fb.scratch(Val::I32);
            let rlen = fb.scratch(Val::I32);
            let base = fb.scratch(Val::I32);
            let pos = fb.scratch(Val::I32);
            let d = fb.scratch(Val::I32);
            fb.g(0).s(u);
            fb.i32c(1).s(cnt);
            fb.g(u).s(u2);
            fb.loop_(); // $dc
            fb.g(u2).i64c(16).op(op::I64_GE_U).if_(); // $dci
            fb.g(u2).i64c(16).op(op::I64_DIV_U).s(u2);
            fb.g(cnt).i32c(1).op(op::I32_ADD).s(cnt);
            fb.br(1); // continue $dc (br 0 from inside the if would EXIT the loop)
            fb.end(); // $dci
            fb.end(); // $dc
            fb.g(cnt).s(rlen);
            fb.i32c(28)
                .g(rlen)
                .op(op::I32_ADD)
                .call(idx(H::Alloc))
                .s(base);
            fb.g(base).i64c(IMMORTAL_RC).store64(0);
            fb.g(base).i64c(0).store64(8);
            fb.g(base).i64c(ARC_MAGIC).store64(16);
            fb.g(base).g(rlen).store32(24);
            fb.i32c(28)
                .g(rlen)
                .op(op::I32_ADD)
                .i32c(1)
                .op(op::I32_SUB)
                .s(pos); // offset, not address
            fb.block(); // $done
            fb.loop_(); // $dig
            fb.g(u).i64c(16).op(op::I64_REM_U).op(op::I32_WRAP_I64).s(d);
            fb.g(base).g(pos).op(op::I32_ADD);
            // 0-9 -> '0'-'9', 10-15 -> 'a'-'f'
            fb.g(d).i32c(48).op(op::I32_ADD);
            fb.g(d).i32c(87).op(op::I32_ADD);
            fb.g(d).i32c(10).op(op::I32_LT_U);
            fb.op(op::SELECT);
            fb.store8(0);
            fb.g(pos).i32c(1).op(op::I32_SUB).s(pos);
            fb.g(u).i64c(16).op(op::I64_DIV_U).s(u);
            fb.g(u).op(op::I64_EQZ).br_if(1); // $done
            fb.br(0); // $dig
            fb.end(); // $dig
            fb.end(); // $done
            fb.g(base).i32c(24).op(op::I32_ADD);
        }
        H::BoolToStr => {
            let lit = |s: &str| *env.string_ptr.get(s).expect("bool to str literal pooled") + 4;
            let t = lit("true");
            let f = lit("false");
            let len = fb.scratch(Val::I32);
            let base = fb.scratch(Val::I32);
            let src = fb.scratch(Val::I32);
            let k = fb.scratch(Val::I32);
            // "true" = 4 chars, "false" = 5 chars: len = 5 - v
            fb.i32c(5).g(0).op(op::I32_SUB).s(len);
            fb.i32c(28)
                .g(len)
                .op(op::I32_ADD)
                .call(idx(H::Alloc))
                .s(base);
            fb.g(base).i64c(IMMORTAL_RC).store64(0);
            fb.g(base).i64c(0).store64(8);
            fb.g(base).i64c(ARC_MAGIC).store64(16);
            fb.g(base).g(len).store32(24);
            fb.i32c(t as i64);
            fb.i32c(f as i64);
            fb.g(0);
            fb.op(op::SELECT); // v ? "false" : "true"
            fb.s(src);
            fb.i32c(0).s(k);
            fb.loop_(); // $c
            fb.g(k).g(len).op(op::I32_LT_U).if_(); // $in
            fb.g(base).i32c(28).op(op::I32_ADD).g(k).op(op::I32_ADD);
            fb.g(src).g(k).op(op::I32_ADD).load8(0);
            fb.store8(0);
            fb.end(); // $in
            fb.g(k).i32c(1).op(op::I32_ADD).s(k);
            fb.g(k).g(len).op(op::I32_LT_U).br_if(0); // $c
            fb.end(); // $c
            fb.g(base).i32c(24).op(op::I32_ADD);
        }
        H::StrToInt => {
            let la = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I32);
            let c = fb.scratch(Val::I32);
            let d = fb.scratch(Val::I32);
            let v = fb.scratch(Val::I64);
            let sgn = fb.scratch(Val::I32);
            let converted = fb.scratch(Val::I32);
            let limit = fb.scratch(Val::I64);
            fb.g(0).load32(0).s(la);
            fb.i32c(0).s(i);
            fb.i32c(0).s(converted);
            fb.i64c(0).s(v);
            fb.i32c(1).s(sgn);
            // Skip C strtoll whitespace.
            fb.block(); // $wsb
            fb.loop_(); // $ws
            fb.g(i).g(la).op(op::I32_LT_U).if_(); // $wsi
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0)
                .s(c);
            fb.g(c)
                .i32c(32)
                .op(op::I32_EQ)
                .g(c)
                .i32c(9)
                .op(op::I32_EQ)
                .op(op::I32_OR)
                .g(c)
                .i32c(10)
                .op(op::I32_EQ)
                .op(op::I32_OR)
                .g(c)
                .i32c(11)
                .op(op::I32_EQ)
                .op(op::I32_OR)
                .g(c)
                .i32c(12)
                .op(op::I32_EQ)
                .op(op::I32_OR)
                .g(c)
                .i32c(13)
                .op(op::I32_EQ)
                .op(op::I32_OR);
            fb.if_(); // $isws
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.br(2); // continue $ws loop: 0=$isws, 1=$wsi, 2=$ws (br to the IF exits the loop in V8)
            fb.end(); // $isws
            fb.br(2); // break $ws: 0=$wsi, 1=$ws, 2=$wsb
            fb.end(); // $wsi
            fb.end(); // $ws
            fb.end(); // $wsb
            // Optional sign.
            fb.g(i).g(la).op(op::I32_LT_U).if_(); // $sgi
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0)
                .s(c);
            fb.g(c).i32c(45).op(op::I32_EQ).if_(); // '-'
            fb.i32c(-1).s(sgn);
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.else_();
            fb.g(c).i32c(43).op(op::I32_EQ).if_(); // '+'
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.end();
            fb.end();
            fb.end(); // $sgi
            // Leading decimal digits, saturating at i64::MAX.
            fb.block(); // $dgb
            fb.loop_(); // $dg
            fb.g(i).g(la).op(op::I32_LT_U).if_(); // $dgi
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0)
                .i32c(48)
                .op(op::I32_SUB)
                .s(d);
            fb.g(d)
                .i32c(0)
                .op(op::I32_GE_S)
                .g(d)
                .i32c(9)
                .op(op::I32_LE_S)
                .op(op::I32_AND)
                .if_(); // $dig
            fb.i64c(9223372036854775807)
                .g(d)
                .op(op::I64_EXTEND_I32_S)
                .op(op::I64_SUB)
                .i64c(10)
                .op(op::I64_DIV_S)
                .s(limit);
            fb.g(v).g(limit).op(op::I64_GT_S).if_(); // overflow
            fb.i64c(9223372036854775807).s(v);
            fb.else_();
            fb.g(v)
                .i64c(10)
                .op(op::I64_MUL)
                .g(d)
                .op(op::I64_EXTEND_I32_S)
                .op(op::I64_ADD)
                .s(v);
            fb.end();
            fb.i32c(1).s(converted);
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.else_();
            fb.br(3); // non-digit: break $dg: 0=$dig,1=$dgi,2=$dg,3=$dgb
            fb.end(); // $dig
            fb.br(1); // digit consumed: $dgi end (loop top)
            fb.end(); // $dgi
            fb.end(); // $dg
            fb.end(); // $dgb
            fb.g(converted).op(op::I32_EQZ).if_();
            fb.i64c(0);
            fb.op(op::RETURN);
            fb.end();
            // Result-typed ifs: the branches produce the i64 result.
            fb.g(sgn).i32c(0).op(op::I32_LT_S).if_i64(); // $neg
            fb.g(v).i64c(9223372036854775807).op(op::I64_EQ).if_i64(); // $ismax
            fb.i64c(-9223372036854775807).i64c(1).op(op::I64_SUB);
            fb.else_();
            fb.i64c(0).g(v).op(op::I64_SUB);
            fb.end(); // $ismax
            fb.else_(); // $neg
            fb.g(v);
            fb.end(); // $neg
        }
        H::StrToU64 => {
            let la = fb.scratch(Val::I32);
            let i = fb.scratch(Val::I32);
            let c = fb.scratch(Val::I32);
            let c2 = fb.scratch(Val::I32);
            let d = fb.scratch(Val::I32);
            let v = fb.scratch(Val::I64);
            let hex = fb.scratch(Val::I32);
            fb.g(0).load32(0).s(la);
            fb.i32c(0).s(i);
            fb.i64c(0).s(v);
            fb.i32c(0).s(hex);
            // Skip whitespace (no VT/FF here: the v1 u64 parser).
            fb.block(); // $wsb
            fb.loop_(); // $ws
            fb.g(i).g(la).op(op::I32_LT_U).if_(); // $wsi
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0)
                .s(c);
            fb.g(c)
                .i32c(32)
                .op(op::I32_EQ)
                .g(c)
                .i32c(9)
                .op(op::I32_EQ)
                .op(op::I32_OR)
                .g(c)
                .i32c(10)
                .op(op::I32_EQ)
                .op(op::I32_OR)
                .g(c)
                .i32c(13)
                .op(op::I32_EQ)
                .op(op::I32_OR);
            fb.if_(); // $isws
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.br(2); // continue $ws loop: 0=$isws, 1=$wsi, 2=$ws (br to the IF exits the loop in V8)
            fb.end(); // $isws
            fb.br(2); // break $ws: 0=$wsi, 1=$ws, 2=$wsb
            fb.end(); // $wsi
            fb.end(); // $ws
            fb.end(); // $wsb
            // Optional 0x / 0X prefix.
            fb.g(i).i32c(1).op(op::I32_ADD).g(la).op(op::I32_LT_U).if_(); // $hx
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0)
                .s(c);
            fb.g(c).i32c(48).op(op::I32_EQ).if_(); // '0'
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .i32c(1)
                .op(op::I32_ADD)
                .op(op::I32_ADD)
                .load8(0)
                .s(c2);
            fb.g(c2)
                .i32c(120)
                .op(op::I32_EQ)
                .g(c2)
                .i32c(88)
                .op(op::I32_EQ)
                .op(op::I32_OR)
                .if_();
            fb.g(i).i32c(2).op(op::I32_ADD).s(i);
            fb.i32c(1).s(hex);
            fb.end();
            fb.end();
            fb.end(); // $hx
            // Digits (wrapping arithmetic, the v1 contract).
            fb.block(); // $dgb
            fb.loop_(); // $dg
            fb.g(i).g(la).op(op::I32_LT_U).if_(); // $dgi
            fb.g(0)
                .i32c(4)
                .op(op::I32_ADD)
                .g(i)
                .op(op::I32_ADD)
                .load8(0)
                .s(c);
            fb.g(hex).if_(); // hex digits
            fb.i32c(-1).s(d);
            fb.g(c)
                .i32c(48)
                .op(op::I32_GE_U)
                .g(c)
                .i32c(57)
                .op(op::I32_LE_U)
                .op(op::I32_AND)
                .if_();
            fb.g(c).i32c(48).op(op::I32_SUB).s(d);
            fb.else_();
            fb.g(c)
                .i32c(97)
                .op(op::I32_GE_U)
                .g(c)
                .i32c(102)
                .op(op::I32_LE_U)
                .op(op::I32_AND)
                .if_();
            fb.g(c).i32c(87).op(op::I32_SUB).s(d);
            fb.else_();
            fb.g(c)
                .i32c(65)
                .op(op::I32_GE_U)
                .g(c)
                .i32c(70)
                .op(op::I32_LE_U)
                .op(op::I32_AND)
                .if_();
            fb.g(c).i32c(55).op(op::I32_SUB).s(d);
            fb.end();
            fb.end();
            fb.end();
            fb.g(d).i32c(0).op(op::I32_GE_S).if_(); // $hd
            fb.g(v)
                .i64c(4)
                .op(op::I64_SHL)
                .g(d)
                .op(op::I64_EXTEND_I32_S)
                .op(op::I64_OR)
                .s(v);
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.else_();
            fb.br(4); // not a hex digit: break $dg: 0=$hd,1=hex,2=$dgi,3=$dg,4=$dgb
            fb.end(); // $hd
            fb.else_(); // decimal digits
            fb.g(c).i32c(48).op(op::I32_SUB).s(d);
            fb.g(d)
                .i32c(0)
                .op(op::I32_GE_S)
                .g(d)
                .i32c(9)
                .op(op::I32_LE_S)
                .op(op::I32_AND)
                .if_();
            fb.g(v)
                .i64c(10)
                .op(op::I64_MUL)
                .g(d)
                .op(op::I64_EXTEND_I32_S)
                .op(op::I64_ADD)
                .s(v);
            fb.g(i).i32c(1).op(op::I32_ADD).s(i);
            fb.else_();
            fb.br(4); // not a decimal digit: break $dg: 0=$dd,1=hex,2=$dgi,3=$dg,4=$dgb
            fb.end(); // $dd
            fb.end();
            fb.br(1); // digit consumed: $dgi end (loop top)
            fb.end(); // $dgi
            fb.end(); // $dg
            fb.end(); // $dgb
            fb.g(v);
        }
        H::FloatToStr => {
            // The oracle's format_percent_g: C `%g` with six significant
            // digits — trailing zeros stripped, exponent form when the
            // exponent is below -4 or at/above 6.
            //
            // Modeled on rustc's flt2dec (library/core/src/num/imp/flt2dec):
            // classify first, then generate exact digits, then render.
            // Every loop here has a provably bounded trip count, so no
            // finite input can spin (rustc requires totality; we have it
            // by construction).
            let lit = |s: &str| *env.string_ptr.get(s).expect("float to str literal pooled") + 4;
            let nan = lit("nan");
            let inf = lit("inf");
            let neginf = lit("-inf");
            let negzero = lit("-0");
            let pos = fb.scratch(Val::I32);
            let rlen = fb.scratch(Val::I32);
            let base = fb.scratch(Val::I32);
            let exp = fb.scratch(Val::I32);
            let t = fb.scratch(Val::F64);
            let sigval = fb.scratch(Val::I64);
            let frac = fb.scratch(Val::F64);
            let gt = fb.scratch(Val::I32);
            let eq = fb.scratch(Val::I32);
            let odd = fb.scratch(Val::I32);
            let up = fb.scratch(Val::I32);
            let sig = fb.scratch(Val::I32);
            let p10 = fb.scratch(Val::I64);
            let d0 = fb.scratch(Val::I64);
            let rest = fb.scratch(Val::I64);
            let head = fb.scratch(Val::I64);
            let tail = fb.scratch(Val::I64);
            let k = fb.scratch(Val::I32);
            let w = fb.scratch(Val::I32);
            let z = fb.scratch(Val::I32);
            let ae = fb.scratch(Val::I32);
            let neg = fb.scratch(Val::I32);
            let d = fb.scratch(Val::I32);
            let split = fb.scratch(Val::I32);
            // ── phase 0: classify; each branch writes NUM_BUF, sets pos,
            // and exits through the if/else shape (no fall-through into
            // the normalizing loops). ──
            fb.g(0).g(0).op(op::F64_NE).if_(); // NaN
            for (off, src) in [(0u32, nan), (1u32, nan), (2u32, nan)] {
                fb.i32c((NUM_BUF + off) as i64);
                fb.i32c(src as i64)
                    .i32c(off as i64)
                    .op(op::I32_ADD)
                    .load8(0);
                fb.store8(0);
            }
            fb.i32c(3).s(pos);
            fb.else_();
            fb.g(0)
                .f64c(0.0)
                .op(op::F64_MUL)
                .f64c(0.0)
                .op(op::F64_NE)
                .if_(); // ±inf
            fb.g(0)
                .op(op::I64_REINTERPRET_F64)
                .i64c(i64::MIN)
                .op(op::I64_AND)
                .i64c(0)
                .op(op::I64_NE)
                .s(neg);
            fb.g(neg).if_();
            for (off, src) in [
                (0u32, neginf),
                (1u32, neginf),
                (2u32, neginf),
                (3u32, neginf),
            ] {
                fb.i32c((NUM_BUF + off) as i64);
                fb.i32c(src as i64)
                    .i32c(off as i64)
                    .op(op::I32_ADD)
                    .load8(0);
                fb.store8(0);
            }
            fb.i32c(4).s(pos);
            fb.else_();
            for (off, src) in [(0u32, inf), (1u32, inf), (2u32, inf)] {
                fb.i32c((NUM_BUF + off) as i64);
                fb.i32c(src as i64)
                    .i32c(off as i64)
                    .op(op::I32_ADD)
                    .load8(0);
                fb.store8(0);
            }
            fb.i32c(3).s(pos);
            fb.end(); // sign of ±inf
            fb.else_();
            // ── finite ──
            fb.g(0)
                .op(op::I64_REINTERPRET_F64)
                .i64c(i64::MIN)
                .op(op::I64_AND)
                .i64c(0)
                .op(op::I64_NE)
                .s(neg);
            fb.g(0).f64c(0.0).op(op::F64_EQ).if_(); // ±0
            fb.g(neg).if_();
            for (off, src) in [(0u32, negzero), (1u32, negzero)] {
                fb.i32c((NUM_BUF + off) as i64);
                fb.i32c(src as i64)
                    .i32c(off as i64)
                    .op(op::I32_ADD)
                    .load8(0);
                fb.store8(0);
            }
            fb.i32c(2).s(pos);
            fb.else_();
            fb.i32c(NUM_BUF as i64);
            fb.i32c(48);
            fb.store8(0);
            fb.i32c(1).s(pos);
            fb.end();
            fb.else_();
            // ── phase 1: |val| = 10^exp, 1 <= |val| < 10 (bounded: each
            // pass divides/multiplies by 10; a finite non-zero f64
            // converges). ──
            fb.g(0).op(op::F64_ABS).s(t);
            fb.i32c(0).s(exp);
            fb.loop_(); // $up
            fb.g(t).f64c(10.0).op(op::F64_GE).if_(); // $upi
            fb.g(t).f64c(10.0).op(op::F64_DIV).s(t);
            fb.g(exp).i32c(1).op(op::I32_ADD).s(exp);
            fb.br(1); // continue $up
            fb.end(); // $upi
            fb.end(); // $up
            fb.loop_(); // $dn
            fb.g(t).f64c(1.0).op(op::F64_LT).if_(); // $dni
            fb.g(t).f64c(10.0).op(op::F64_MUL).s(t);
            fb.g(exp).i32c(1).op(op::I32_SUB).s(exp);
            fb.br(1); // continue $dn
            fb.end(); // $dni
            fb.end(); // $dn
            // ── phase 2: six significant digits, correctly rounded ──
            fb.g(t).f64c(100000.0).op(op::F64_MUL).s(t);
            fb.g(t).op(op::I64_TRUNC_F64_U).s(sigval);
            fb.g(t)
                .g(sigval)
                .op(op::F64_CONVERT_I64_S)
                .op(op::F64_SUB)
                .s(frac);
            fb.g(frac).f64c(0.5).op(op::F64_GT).s(gt);
            fb.g(frac).f64c(0.5).op(op::F64_EQ).s(eq);
            fb.g(sigval)
                .i64c(1)
                .op(op::I64_REM_U)
                .i64c(1)
                .op(op::I64_EQ)
                .s(odd);
            fb.g(gt).g(eq).g(odd).op(op::I32_AND).op(op::I32_OR).s(up);
            fb.g(up).if_();
            fb.g(sigval).i64c(1).op(op::I64_ADD).s(sigval);
            fb.end();
            fb.g(sigval).i64c(1000000).op(op::I64_EQ).if_(); // carry
            fb.i64c(100000).s(sigval);
            fb.g(exp).i32c(1).op(op::I32_ADD).s(exp);
            fb.end();
            // Strip trailing zeros: sigval keeps `sig` digits (>= 1).
            fb.i32c(6).s(sig);
            fb.loop_(); // $tz
            fb.g(sigval)
                .i64c(10)
                .op(op::I64_REM_U)
                .i64c(0)
                .op(op::I64_EQ)
                .g(sig)
                .i32c(1)
                .op(op::I32_GT_S)
                .op(op::I32_AND)
                .if_(); // $tzi
            fb.g(sigval).i64c(10).op(op::I64_DIV_U).s(sigval);
            fb.g(sig).i32c(1).op(op::I32_SUB).s(sig);
            fb.br(1); // continue $tz
            fb.end(); // $tzi
            fb.end(); // $tz
            // ── phase 3: sign ──
            fb.g(neg).if_();
            fb.i32c(NUM_BUF as i64);
            fb.i32c(45);
            fb.store8(0);
            fb.i32c(1).s(pos);
            fb.else_();
            fb.i32c(0).s(pos);
            fb.end();
            // ── phase 4: render (C %g: exponent form when exp < -4 or
            // exp >= 6, else plain form) ──
            fb.g(exp)
                .i32c(-4)
                .op(op::I32_LT_S)
                .g(exp)
                .i32c(6)
                .op(op::I32_GE_S)
                .op(op::I32_OR)
                .if_(); // ── exponent form ──
            fb.g(sig).i32c(1).op(op::I32_SUB).s(z);
            emit_pow10(fb, z, p10, k); // p10 = 10^(sig-1)
            fb.g(sigval).g(p10).op(op::I64_DIV_U).s(d0);
            fb.g(sigval).g(p10).op(op::I64_REM_U).s(rest);
            fb.i32c(1).s(w);
            emit_digits(fb, pos, w, k, d, d0); // leading digit
            fb.g(sig).i32c(1).op(op::I32_GT_S).if_(); // more digits
            fb.i32c(NUM_BUF as i64).g(pos).op(op::I32_ADD);
            fb.i32c(46); // '.'
            fb.store8(0);
            fb.g(pos).i32c(1).op(op::I32_ADD).s(pos);
            fb.g(sig).i32c(1).op(op::I32_SUB).s(w);
            emit_digits(fb, pos, w, k, d, rest); // padded to sig-1
            fb.end();
            fb.i32c(NUM_BUF as i64).g(pos).op(op::I32_ADD);
            fb.i32c(101); // 'e'
            fb.store8(0);
            fb.g(pos).i32c(1).op(op::I32_ADD).s(pos);
            fb.g(exp).i32c(0).op(op::I32_LT_S).if_();
            fb.i32c(NUM_BUF as i64).g(pos).op(op::I32_ADD);
            fb.i32c(45); // '-'
            fb.store8(0);
            fb.else_();
            fb.i32c(NUM_BUF as i64).g(pos).op(op::I32_ADD);
            fb.i32c(43); // '+'
            fb.store8(0);
            fb.end();
            fb.g(pos).i32c(1).op(op::I32_ADD).s(pos);
            fb.g(exp);
            fb.i32c(0).g(exp).op(op::I32_SUB);
            fb.g(exp).i32c(0).op(op::I32_GE_S);
            fb.op(op::SELECT); // ae = |exp|
            fb.s(ae);
            fb.g(ae).i32c(100).op(op::I32_GE_S).if_();
            fb.i32c(3).s(w);
            fb.else_();
            fb.g(ae).i32c(10).op(op::I32_GE_S).if_();
            fb.i32c(2).s(w);
            fb.else_();
            fb.i32c(NUM_BUF as i64).g(pos).op(op::I32_ADD);
            fb.i32c(48); // leading '0'
            fb.store8(0);
            fb.g(pos).i32c(1).op(op::I32_ADD).s(pos);
            fb.i32c(1).s(w);
            fb.end();
            fb.end();
            fb.g(ae).op(op::I64_EXTEND_I32_S).s(d0);
            emit_digits(fb, pos, w, k, d, d0);
            fb.else_(); // ── plain form ──
            fb.g(exp).i32c(0).op(op::I32_GE_S).if_(); // exp >= 0
            fb.g(exp)
                .g(sig)
                .i32c(1)
                .op(op::I32_SUB)
                .op(op::I32_GE_S)
                .if_(); // whole number
            fb.g(sig).s(w);
            emit_digits(fb, pos, w, k, d, sigval);
            fb.g(exp)
                .g(sig)
                .i32c(1)
                .op(op::I32_SUB)
                .op(op::I32_SUB)
                .s(z); // z = exp - sig + 1 (>= 0 here)
            emit_zeros(fb, pos, z, k);
            fb.else_(); // split: digits '.' digits
            fb.g(exp).i32c(1).op(op::I32_ADD).s(split);
            fb.g(sig).g(split).op(op::I32_SUB).s(z);
            emit_pow10(fb, z, p10, k); // p10 = 10^(sig-split)
            fb.g(sigval).g(p10).op(op::I64_DIV_U).s(head);
            fb.g(sigval).g(p10).op(op::I64_REM_U).s(tail);
            fb.g(split).s(w);
            emit_digits(fb, pos, w, k, d, head);
            fb.i32c(NUM_BUF as i64).g(pos).op(op::I32_ADD);
            fb.i32c(46); // '.'
            fb.store8(0);
            fb.g(pos).i32c(1).op(op::I32_ADD).s(pos);
            fb.g(sig).g(split).op(op::I32_SUB).s(w);
            emit_digits(fb, pos, w, k, d, tail);
            fb.end(); // whole number
            fb.else_(); // 0.00…digits
            fb.i32c(NUM_BUF as i64).g(pos).op(op::I32_ADD);
            fb.i32c(48); // '0'
            fb.store8(0);
            fb.g(pos).i32c(1).op(op::I32_ADD).s(pos);
            fb.i32c(NUM_BUF as i64).g(pos).op(op::I32_ADD);
            fb.i32c(46); // '.'
            fb.store8(0);
            fb.g(pos).i32c(1).op(op::I32_ADD).s(pos);
            fb.i32c(0)
                .g(exp)
                .op(op::I32_SUB)
                .i32c(1)
                .op(op::I32_SUB)
                .s(z);
            emit_zeros(fb, pos, z, k);
            fb.g(sig).s(w);
            emit_digits(fb, pos, w, k, d, sigval);
            fb.end(); // exp >= 0
            fb.end(); // plain form
            fb.end(); // finite non-zero
            fb.end(); // inf
            fb.end(); // NaN
            // ── epilogue: the scratch became the string ──
            fb.g(pos).s(rlen);
            fb.i32c(28)
                .g(rlen)
                .op(op::I32_ADD)
                .call(idx(H::Alloc))
                .s(base);
            fb.g(base).i64c(IMMORTAL_RC).store64(0);
            fb.g(base).i64c(0).store64(8);
            fb.g(base).i64c(ARC_MAGIC).store64(16);
            fb.g(base).g(rlen).store32(24);
            fb.i32c(0).s(k);
            fb.loop_(); // $cp
            fb.g(k).g(rlen).op(op::I32_LT_U).if_(); // $cpin
            fb.g(base).i32c(28).op(op::I32_ADD).g(k).op(op::I32_ADD);
            fb.i32c(NUM_BUF as i64).g(k).op(op::I32_ADD).load8(0);
            fb.store8(0);
            fb.end(); // $cpin
            fb.g(k).i32c(1).op(op::I32_ADD).s(k);
            fb.g(k).g(rlen).op(op::I32_LT_U).br_if(0); // $cp
            fb.end(); // $cp
            fb.g(base).i32c(24).op(op::I32_ADD);
        }
    }
    Ok(())
}

/// `null → trap; index out of [0, len) → trap` — the loud-failure
/// contract shared by every list read/write.
fn slice_new_body(fb: &mut FB, source_len_helper: u32, alloc_helper: u32) {
    let source_len = fb.scratch(Val::I64);
    let view = fb.scratch(Val::I32);

    fb.g(0).op(op::I32_EQZ);
    fb.g(1).i64c(0).op(op::I64_LT_S).op(op::I32_OR);
    fb.g(2).i64c(0).op(op::I64_LT_S).op(op::I32_OR);
    fb.if_().op(op::UNREACHABLE).end();

    fb.g(0).call(source_len_helper).s(source_len);
    // Check `start <= source_len && len <= source_len - start` without
    // forming `start + len`, so signed overflow cannot bypass the guard.
    fb.g(1).g(source_len).op(op::I64_GT_S);
    fb.g(2)
        .g(source_len)
        .g(1)
        .op(op::I64_SUB)
        .op(op::I64_GT_S)
        .op(op::I32_OR);
    fb.if_().op(op::UNREACHABLE).end();

    fb.i32c(SLICE_SIZE).call(alloc_helper).s(view);
    fb.g(view).g(0).store32(SLICE_BASE as u32);
    fb.g(view).g(1).store64(SLICE_START as u32);
    fb.g(view).g(2).store64(SLICE_LEN as u32);
    fb.g(view);
}

fn slice_bounds_check(fb: &mut FB) {
    fb.g(0).op(op::I32_EQZ);
    fb.g(1).i64c(0).op(op::I64_LT_S).op(op::I32_OR);
    fb.g(1)
        .g(0)
        .load64(SLICE_LEN as u32)
        .op(op::I64_GE_S)
        .op(op::I32_OR);
    fb.if_().op(op::UNREACHABLE).end();
}

/// Leave `(source, absolute_index)` on the stack after checking the view index.
fn slice_source_and_index(fb: &mut FB) {
    slice_bounds_check(fb);
    fb.g(0).load32(SLICE_BASE as u32);
    fb.g(0).load64(SLICE_START as u32).g(1).op(op::I64_ADD);
}

fn list_bounds_check(fb: &mut FB) {
    fb.g(0).op(op::I32_EQZ).if_().op(op::UNREACHABLE).end();
    let len = fb.scratch(Val::I64);
    fb.g(0).load64(LIST_LEN as u32).s(len);
    fb.g(1)
        .g(len)
        .op(op::I64_GE_S)
        .if_()
        .op(op::UNREACHABLE)
        .end();
    fb.g(1)
        .i64c(0)
        .op(op::I64_LT_S)
        .if_()
        .op(op::UNREACHABLE)
        .end();
}

/// The shared push prologue (grow ×2 from 8 with a copied bump
/// "realloc") plus the per-class element store. Params: (list, value).
fn list_push_body(fb: &mut FB, idx: impl Fn(H) -> u32, is_f64: bool, is_ptr: bool) {
    let len = fb.scratch(Val::I64);
    let cap = fb.scratch(Val::I64);
    let data = fb.scratch(Val::I32);
    let newdata = fb.scratch(Val::I32);
    let newcap = fb.scratch(Val::I64);
    fb.g(0).op(op::I32_EQZ).if_().op(op::UNREACHABLE).end();
    fb.g(0).load64(LIST_LEN as u32).s(len);
    fb.g(0).load64(LIST_CAP as u32).s(cap);
    fb.g(0)
        .load64(LIST_DATA as u32)
        .op(op::I32_WRAP_I64)
        .s(data);
    fb.g(len).g(cap).op(op::I64_EQ).if_();
    // newcap = cap == 0 ? 8 : cap * 2
    fb.i64c(LIST_NEW_CAP)
        .g(cap)
        .i64c(2)
        .op(op::I64_MUL)
        .g(cap)
        .op(op::I64_EQZ)
        .op(op::SELECT)
        .s(newcap);
    fb.g(newcap)
        .i64c(LIST_CAP_MAX)
        .op(op::I64_GT_S)
        .if_()
        .op(op::UNREACHABLE)
        .end();
    // newdata = Alloc(newcap * 8); copy oldcap * 8 bytes
    fb.g(newcap)
        .i64c(8)
        .op(op::I64_MUL)
        .op(op::I32_WRAP_I64)
        .call(idx(H::Alloc))
        .s(newdata);
    fb.g(newdata)
        .g(data)
        .g(cap)
        .i64c(8)
        .op(op::I64_MUL)
        .op(op::I32_WRAP_I64)
        .memory_copy();
    fb.g(0)
        .g(newdata)
        .op(op::I64_EXTEND_I32_U)
        .store64(LIST_DATA as u32);
    fb.g(0).g(newcap).store64(LIST_CAP as u32);
    fb.g(newdata).s(data);
    fb.end();
    // data[len] = value; len += 1
    fb.g(data)
        .g(len)
        .op(op::I32_WRAP_I64)
        .i32c(8)
        .op(op::I32_MUL)
        .op(op::I32_ADD);
    fb.g(1);
    if is_f64 {
        fb.storef64(0);
    } else if is_ptr {
        fb.op(op::I64_EXTEND_I32_U);
        fb.store64(0);
    } else {
        fb.store64(0);
    }
    fb.g(0)
        .g(len)
        .i64c(1)
        .op(op::I64_ADD)
        .store64(LIST_LEN as u32);
}

// ---------------------------------------------------------------------------
// Generated destructors (5D2a): one per nominal, `lpp_drop_s{raw}` /
// `lpp_drop_e{raw}`, plus `lpp_drop_list`. Each takes the payload
// pointer and releases the managed children; `Release` dispatches
// nested destructors through the table.
// ---------------------------------------------------------------------------

fn lower_destructor(
    env: &Env<'_>,
    aggregate: MirAggregateId,
) -> Result<(String, Vec<Val>, Vec<Val>, Vec<Val>, Vec<u8>), CodegenError> {
    let layout = env
        .layouts
        .get(&aggregate)
        .ok_or_else(|| unsupported("unlaid-out nominal", None))?
        .clone();
    let agg = env.program.aggregate(aggregate).unwrap();
    let tag = match agg.kind {
        lpp_mir::MirAggregateKind::Struct => 's',
        lpp_mir::MirAggregateKind::Enum => 'e',
    };
    let name = format!("lpp_drop_{tag}{}", aggregate.raw());
    let release_idx = *env
        .helper_index
        .get(&H::Release)
        .expect("Release registered");
    let mut fb = FB::new(1);
    match layout.kind {
        lpp_mir::MirAggregateKind::Struct => {
            for slot in &layout.struct_fields {
                if is_managed(env.types, slot.ty) {
                    fb.g(0);
                    fb.load32(slot.offset);
                    fb.call(release_idx);
                }
            }
        }
        lpp_mir::MirAggregateKind::Enum => {
            let n = layout.variant_fields.len();
            let tag_local = fb.scratch(Val::I32);
            fb.g(0).load32(ENUM_TAG as u32).s(tag_local);
            for (ordinal, slots) in layout.variant_fields.iter().enumerate() {
                fb.g(tag_local);
                fb.i32c(ordinal as i64);
                fb.op(op::I32_EQ);
                fb.if_();
                for slot in slots {
                    if is_managed(env.types, slot.ty) {
                        fb.g(0);
                        fb.load32(slot.offset);
                        fb.call(release_idx);
                    }
                }
                if ordinal + 1 < n {
                    fb.else_();
                }
            }
            // Each `if` (including the last, else-less one) needs its `end`:
            // the chain nests, so all ends come after the final branch.
            for _ in 0..n {
                fb.end();
            }
        }
    }
    Ok((name, vec![Val::I32], vec![], fb.extras, fb.body))
}

fn lower_list_destructor(
    env: &Env<'_>,
) -> Result<(String, Vec<Val>, Vec<Val>, Vec<Val>, Vec<u8>), CodegenError> {
    let release_idx = *env
        .helper_index
        .get(&H::Release)
        .expect("Release registered");
    let mut fb = FB::new(1);
    let i = fb.scratch(Val::I64);
    // if is_arc: for each element, Release (the container's own
    // reference to each element)
    fb.g(0)
        .load64(LIST_IS_ARC as u32)
        .op(op::I32_WRAP_I64)
        .if_();
    fb.i64c(0).s(i);
    fb.block(); // $done
    fb.loop_(); // $d
    fb.g(0).load64(LIST_LEN as u32).g(i).op(op::I64_EQ).br_if(1); // $done
    fb.g(0)
        .load64(LIST_DATA as u32)
        .op(op::I32_WRAP_I64)
        .g(i)
        .i64c(8)
        .op(op::I64_MUL)
        .op(op::I32_WRAP_I64)
        .op(op::I32_ADD)
        .load64(0)
        .op(op::I32_WRAP_I64)
        .call(release_idx);
    fb.g(i).i64c(1).op(op::I64_ADD).s(i);
    fb.br(0); // $d
    fb.end(); // $d
    fb.end(); // $done
    fb.end(); // if
    Ok((
        "lpp_drop_list".to_string(),
        vec![Val::I32],
        vec![],
        fb.extras,
        fb.body,
    ))
}
