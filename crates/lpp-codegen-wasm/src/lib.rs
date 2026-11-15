//! Phase 5D: hand-written binary WebAssembly backend (`wasm32-wasi`).
//!
//! Slice 1 (data surface): scalars (`Int`/`Float`/`Bool`/`Char`), strings,
//! control flow (`if`/`else`, loops), direct calls (including recursion),
//! and the `print`/`write_str`/`str_len` builtins — all over WASI.
//!
//! Slice 2 (5D2a, managed data surface): structs, enums, lists, and the
//! ARC heap. Managed values are 32-bit *payload* offsets into a bump
//! allocator that lives behind the static string pool (a `heap` global);
//! each node carries a WASM-specific 24-byte header
//! `[rc i64][drop-index i64][magic "ARC1"]` (payload at +24), also used by
//! the length-prefixed WASM string pool. This remains intentionally separate
//! from the compact 16-byte native x86-64 header: the WASM allocator packs its
//! size class into the magic word and stores a table index inline. The ARC model is
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

mod analysis;
mod cycle;
mod encode;
mod helper_body;
mod lower_functions;
mod wasm_compile;
use lower_functions::*;
mod helper_support;
use analysis::*;
use helper_support::*;
use wasm_compile::compile_module;
#[path = "lower_methods/lower_builtins.rs"]
mod lower_builtins;
#[path = "lower_methods/lower_calls.rs"]
mod lower_calls;
#[path = "lower_methods/lower_tail.rs"]
mod lower_tail;
#[path = "lower_methods/lower_values.rs"]
mod lower_values;
use helper_body::emit_helper_body;

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
// These offsets are deliberately WASM-only; native compact-header work does
// not alter this representation or the WASM string length prefix.
// ---------------------------------------------------------------------------
const IOVEC_BUF: u32 = 0;
const FD_IO_OUT: u32 = 8;
const NUM_BUF: u32 = 16;
const NUM_BUF_END: u32 = 80;
/// Read in 4 KiB chunks into `FS_BUF`.
const FS_CHUNK: u32 = 4096;
/// Low scratch the filesystem helpers own: `&fd`, `&nwritten`, `&nread` and
/// the 64-byte `filestat`. It must sit *outside* the string pool, so the pool
/// starts after it.
const FS_FD_OUT: u32 = 88;
const FS_NWRITTEN: u32 = 92;
const FS_NREAD: u32 = 84;
const FS_STAT: u32 = 96;
/// `fd_seek`'s u64 output: the file size, taken by seeking to the end.
const FS_SEEK_OUT: u32 = 104;
const FS_SCRATCH_END: u32 = 160;
const FS_BUF: u32 = 160;
const POOL_START: u32 = FS_SCRATCH_END + FS_CHUNK;
const STR_HEADER: u32 = 24;
/// WASI `rights` for a read-only open: path_open, fd_read, fd_seek, fd_tell.
const FS_RIGHTS_BASE: i64 = (1 << 0) | (1 << 1) | (1 << 2) | (1 << 5) | (1 << 6) | (1 << 13);
/// …plus fd_write and path_create_file for a writer.
const FS_RIGHTS_WRITE: i64 = FS_RIGHTS_BASE | (1 << 6) | (1 << 9) | (1 << 3) | (1 << 4);
/// Appending needs the same rights as writing (the offset comes from `fd_seek`).
const FS_RIGHTS_APPEND: i64 = FS_RIGHTS_WRITE;
const ARC_HEADER: i64 = 24;
const IMMORTAL_RC: i64 = i64::MAX;
const ARC_MAGIC: i64 = 0x41_52_43_31; // "ARC1"
/// The wasm heap-pointer global index (emitted only when the heap exists).
const GLOBAL_HEAP: u32 = 0;
/// Per-size-class free-list heads follow the bump pointer global. Class
/// exponents 5..=31 represent blocks from 32 bytes through 2 GiB.
const ARC_FREE_BIN_BASE: u32 = 1;
const ARC_FREE_BIN_COUNT: u32 = 27;
const ARC_MIN_EXP: i64 = 5;
const ARC_MAX_EXP: i64 = 31;

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
    // ── filesystem (path-based WASI imports) ──
    FileExists,
    ReadFile,
    WriteFile,
    AppendFile,
    DeleteFile,
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
    /// Import index of WASI's `proc_exit`, used by defined runtime panic guards.
    proc_exit_import_idx: u32,
    /// The funcref table index of the dispatch table (table 1 when a
    /// drop table exists, table 0 otherwise).
    dispatch_table: u8,
    /// 5D2b slice 3: function-value provenance (from the pre-scan).
    value_origins: BTreeMap<(MirFunctionId, MirLocalId), MirFunctionId>,
    /// Cycle-capable (`Shared`) types requiring Bacon-Rajan cycle collection.
    shared_types: std::collections::BTreeSet<TypeId>,
    /// Module index of `lpp_arc_mark_collectible`, when cycle collection is active.
    mark_collectible_idx: Option<u32>,
    /// Module index of `lpp_cycle_note_root`, when cycle collection is active.
    note_root_idx: Option<u32>,
    /// Module index of `lpp_arc_collect_cycles`, when cycle collection is active.
    collect_cycles_idx: Option<u32>,
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
        .arg("--enable-bulk-memory")
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
