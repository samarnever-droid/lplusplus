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
//! * The module imports a handful of `wasi_snapshot_preview1` functions
//!   (`fd_write`, plus `fd_read`, `proc_exit`, `clock_time_get` and
//!   `poll_oneoff` only when the program needs them). Any WASI-capable
//!   runtime (wasmtime, wasmer, wazero, browser shims) can execute it.
//! * Exports `_start` (the WASI command entry point, wrapping the L++ `main`
//!   function) and the linear `memory`.
//! * All heap objects use the same 24-byte ARC header layout
//!   (`[refcount i64][destructor table index i64][magic i64]`) sitting just
//!   before the payload, an `i32` wasm table for indirect calls, and a bump
//!   allocator over linear memory. Memory is never recycled inside a run —
//!   reference counts and destructors still run exactly like they do
//!   natively (so destructor *observability* is preserved), but the dead
//!   bytes are only reclaimed when the process exits. See
//!   "Heap and ARC" below.
//! * Strings are `[len i32][bytes…]` with the value pointing at `len`.
//!   Static literals additionally carry a faux immortal ARC header so
//!   retain/release on any string is uniformly safe.
//!
//! ## Supported language surface (v2)
//!
//! Everything the native backends support *except* what the WASI sandbox
//! physically cannot do and what has not been ported yet:
//!
//! * scalars, arithmetic/branching/loops, functions, recursion (v1)
//! * `struct` (heap ARC, stack-promoted, and arena/self-referential forms)
//! * `enum` variants with payloads and `match` (struct machinery + `__tag`)
//! * tuples (fixed layout with the native ownership prefix)
//! * `List[T]` for every supported element class, incl. for-in loops
//! * closures, `FuncRef`, trait dispatch (`call_indirect`)
//! * single-thread `async`/`await` tasks (run-to-completion executor)
//! * borrowed zero-copy slices (`slice`, `str_slice`, `slice_get`, …)
//! * `Map[K, V]` (open-addressing hash map ported from `runtime/lpp_map.c`)
//! * dynamic strings: concat/substr/trim/upper/lower/replace/repeat/split/
//!   find/contains/starts_with/ends_with, char_at/ord/chr
//! * conversions: int/float/bool/str to string and back
//!   (`float_to_str` formats like C `%g`)
//! * numerics: abs/min/max/int_pow/pow/sqrt/floor/ceil/sin/cos/tan,
//!   random family (xorshift seeded from the WASI clock), time_ms,
//!   sleep_ms, exit
//! * `input()` from stdin (one line, trailing newline stripped)
//!
//! Rejected with precise diagnostics (native targets are unaffected):
//! OS threads (`spawn`), C FFI/extern symbols, SIMD vectors, networking,
//! GUI, host system metrics, process spawning, and the runtime-library
//! features not yet ported (JSON, byte buffers, file system).
//!
//! ## Control-flow strategy
//!
//! MIR is a goto-based CFG; WebAssembly is structured. Instead of needing a
//! full Relooper, functions are laid out in reverse post-order inside one
//! `block…loop` dispatcher; forward edges are plain `br`s and back-edges
//! bounce through a `br_table` dispatcher once per iteration. Any CFG —
//! reducible or not — is handled without special cases.

use std::collections::{HashMap, HashSet};

use crate::ast::BinaryOperator;
use crate::layout::{struct_layout, tuple_layout, tuple_runtime_metadata};
use crate::mir::ir::*;
use crate::type_facts::{AbiClass, ListElementClass};
use crate::types::{StructTypeId, TypeRef, TypeTable};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
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
/// * Pointers (`Str`, structs, lists, tuples, closures, tasks, slices) are
///   `i32` offsets into linear memory (wasm32).
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

#[allow(dead_code)]
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
    pub const I32_STORE: u8 = 0x36;
    pub const I64_STORE: u8 = 0x37;
    pub const F64_STORE: u8 = 0x39;
    pub const I32_STORE8: u8 = 0x3a;
    pub const I32_CONST: u8 = 0x41;
    pub const I64_CONST: u8 = 0x42;
    pub const F64_CONST: u8 = 0x44;
    pub const I32_EQZ: u8 = 0x45;
    pub const I32_EQ: u8 = 0x46;
    pub const I32_NE: u8 = 0x47;
    pub const I32_LT_S: u8 = 0x48;
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
    pub const I64_LE_S: u8 = 0x57;
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
    pub const F64_ABS: u8 = 0x99;
    pub const F64_NEG: u8 = 0x9a;
    pub const F64_CEIL: u8 = 0x9b;
    pub const F64_FLOOR: u8 = 0x9c;
    pub const F64_TRUNC: u8 = 0x9d;
    pub const F64_NEAREST: u8 = 0x9e;
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
    pub const BLOCK_VOID: u8 = 0x40; // empty block type
    // Pseudo-opcodes emitted as multi-byte sequences by dedicated helpers.
    pub const MEMORY_SIZE_PREFIX: u8 = 0x3f;
    pub const MEMORY_GROW_PREFIX: u8 = 0x40;
    pub const BULK_PREFIX: u8 = 0xfc;
    pub const MEMORY_COPY_SUB: u64 = 10;
    pub const MEMORY_FILL_SUB: u64 = 11;
}

// ── Static memory layout ─────────────────────────────────────────────────────

/// iovec for `fd_write`/`fd_read`: `[ptr i32][len i32]` at offsets 0 and 4.
const IOVEC_BUF: u32 = 0;
const IOVEC_LEN: u32 = 4;
/// `fd_write`/`fd_read` write the byte count here.
const FD_IO_OUT: u32 = 8;
/// 64-byte scratch used by the number formatters.
const NUM_BUF: u32 = 16;
const NUM_BUF_SIZE: u32 = 64;
/// Line buffer consumed by `input()` (mirrors the native 4096-byte `fgets`).
const INPUT_BUF: u32 = NUM_BUF + NUM_BUF_SIZE; // 80
const INPUT_BUF_SIZE: u32 = 4096;
/// WASI clock result cell (i64 nanoseconds).
const CLOCK_BUF: u32 = INPUT_BUF + INPUT_BUF_SIZE; // 4176 (8-aligned)
/// WASI poll_oneoff event output scratch.
const EVENT_BUF: u32 = CLOCK_BUF + 8; // 4184
/// WASI subscription descriptor for `sleep_ms` (56 bytes, padded to 64).
const SUB_BUF: u32 = EVENT_BUF + 32; // 4216
/// Start of the static literal pool. Each entry is
/// `[rc i64][drop i64][magic i64][len i32][bytes…]`, 8-aligned; code receives
/// a pointer to the `len` field (i.e. the ARC payload pointer).
const POOL_START: u32 = SUB_BUF + 64; // 4280

/// ARC header layout, relative to the payload pointer `p`:
/// `p-24` = refcount, `p-16` = destructor wasm-table index (0 = none),
/// `p-8` = magic cookie.
const ARC_HEADER_SIZE: i32 = 24;
#[allow(dead_code)]
const ARC_RC_OFF: i32 = -24;
#[allow(dead_code)]
const ARC_DROP_OFF: i32 = -16;
/// Immortal sentinel for static literals: retain/release is a no-op.
const ARC_IMMORTAL: i64 = i64::MIN;

/// Global slots.
const GLOBAL_HEAP: u32 = 0;
const GLOBAL_RNG: u32 = 1;

// ── WASI imports ─────────────────────────────────────────────────────────────

/// Every WASI function this backend can import. Imports are only emitted
/// when at least one emitted helper will call them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Wasi {
    FdWrite,
    FdRead,
    ProcExit,
    ClockTimeGet,
    PollOneoff,
    EnvironSizesGet,
    EnvironGet,
}

impl Wasi {
    fn module(self) -> &'static str {
        "wasi_snapshot_preview1"
    }

    fn field(self) -> &'static str {
        match self {
            Wasi::FdWrite => "fd_write",
            Wasi::FdRead => "fd_read",
            Wasi::ProcExit => "proc_exit",
            Wasi::ClockTimeGet => "clock_time_get",
            Wasi::PollOneoff => "poll_oneoff",
            Wasi::EnvironSizesGet => "environ_sizes_get",
            Wasi::EnvironGet => "environ_get",
        }
    }

    fn signature(self) -> (Vec<Val>, Vec<Val>) {
        match self {
            Wasi::FdWrite | Wasi::FdRead | Wasi::PollOneoff => (
                vec![Val::I32, Val::I32, Val::I32, Val::I32],
                vec![Val::I32],
            ),
            Wasi::ProcExit => (vec![Val::I32], vec![]),
            Wasi::ClockTimeGet => (vec![Val::I32, Val::I64, Val::I32], vec![Val::I32]),
            Wasi::EnvironSizesGet | Wasi::EnvironGet => {
                (vec![Val::I32, Val::I32], vec![Val::I32])
            }
        }
    }
}

// ── Synthesized runtime helpers ──────────────────────────────────────────────

/// Which synthesized runtime functions a module can need. Emission order is
/// derived `Ord` order, so function indices stay deterministic between runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Helper {
    // ARC core (always grouped first by variant order).
    TrapStub,
    Alloc,
    ArcAlloc,
    Retain,
    Release,
    StrAlloc,
    StrNew,
    // I/O and formatting.
    WriteFd,
    Write,
    FmtU64,
    WriteU64,
    PrintInt,
    PrintBool,
    PrintFloat,
    PrintStr,
    PanicMsg,
    Panic2,
    Panic3,
    Exit,
    Input,
    EnvMatch,
    EnvGet,
    // Strings.
    StrLen,
    StrEq,
    StrConcat,
    StrSubstr,
    StrTrim,
    StrUpper,
    StrLower,
    IntToStr,
    FloatToStr,
    BoolToStr,
    StrToInt,
    CharAt,
    Ord,
    Chr,
    StrFindFrom,
    StrFind,
    StrContains,
    StrStartsWith,
    StrEndsWith,
    StrRepeat,
    StrReplace,
    StrSplit,
    // Lists.
    ListNew,
    ListPush,
    ListSet,
    ListGet,
    ListLen,
    ListDestroy,
    // Maps.
    MapNew,
    MapLen,
    HashStr,
    HashInt,
    MapProbe,
    MapRehash,
    MapPut,
    MapGet,
    MapHas,
    MapRemove,
    MapDestroy,
    // Tuples / closures / tasks.
    TupleAlloc,
    TupleDestroy,
    ClosureDestroy,
    TaskNew,
    TaskRun,
    TaskAwait,
    TaskPoll,
    TaskDestroy,
    // Slices.
    SliceNew,
    SliceLen,
    SliceGet,
    StrSliceGet,
    StrSliceToStr,
    // Math / time.
    IntPow,
    Log2,
    Exp2,
    Pow,
    Reduce2Pi,
    SinPoly,
    CosPoly,
    Trig,
    TimeSeed,
    Random,
    RandomRange,
    TimeMs,
    SleepMs,
}

fn helper_signature(helper: Helper) -> (Vec<Val>, Vec<Val>) {
    use Val::*;
    match helper {
        Helper::TrapStub => (vec![], vec![]),
        Helper::Alloc => (vec![I32], vec![I32]),
        Helper::ArcAlloc => (vec![I32, I32], vec![I32]),
        Helper::Retain | Helper::Release => (vec![I32], vec![]),
        Helper::StrAlloc => (vec![I32], vec![I32]),
        Helper::StrNew => (vec![I32, I32], vec![I32]),
        Helper::WriteFd => (vec![I32, I32, I32], vec![]),
        Helper::Write => (vec![I32, I32], vec![]),
        Helper::FmtU64 => (vec![I64], vec![I32]),
        Helper::WriteU64 => (vec![I64], vec![]),
        Helper::PrintInt => (vec![I64], vec![]),
        Helper::PrintBool => (vec![I32], vec![]),
        Helper::PrintFloat => (vec![F64], vec![]),
        Helper::PrintStr => (vec![I32], vec![]),
        Helper::PanicMsg => (vec![I32], vec![]),
        Helper::Panic2 => (vec![I32, I64, I32, I64], vec![]),
        Helper::Panic3 => (vec![I32, I64, I32, I64, I32, I64], vec![]),
        Helper::Exit => (vec![I64], vec![]),
        Helper::Input => (vec![], vec![I32]),
        Helper::EnvMatch => (vec![I32, I32], vec![I32]),
        Helper::EnvGet => (vec![I32], vec![I32]),
        Helper::StrLen => (vec![I32], vec![I64]),
        Helper::StrEq => (vec![I32, I32], vec![I32]),
        Helper::StrConcat => (vec![I32, I32], vec![I32]),
        Helper::StrSubstr => (vec![I32, I64, I64], vec![I32]),
        Helper::StrTrim | Helper::StrUpper | Helper::StrLower => (vec![I32], vec![I32]),
        Helper::IntToStr => (vec![I64], vec![I32]),
        Helper::FloatToStr => (vec![F64], vec![I32]),
        Helper::BoolToStr => (vec![I32], vec![I32]),
        Helper::StrToInt => (vec![I32], vec![I64]),
        Helper::CharAt => (vec![I32, I64], vec![I32]),
        Helper::Ord => (vec![I32], vec![I64]),
        Helper::Chr => (vec![I64], vec![I32]),
        Helper::StrFindFrom => (vec![I32, I32, I32], vec![I32]),
        Helper::StrFind => (vec![I32, I32], vec![I64]),
        Helper::StrContains | Helper::StrStartsWith | Helper::StrEndsWith => {
            (vec![I32, I32], vec![I32])
        }
        Helper::StrRepeat => (vec![I32, I64], vec![I32]),
        Helper::StrReplace => (vec![I32, I32, I32], vec![I32]),
        Helper::StrSplit => (vec![I32, I64], vec![I32]),
        Helper::ListNew => (vec![I32], vec![I32]),
        Helper::ListPush => (vec![I32, I64], vec![]),
        Helper::ListSet => (vec![I32, I64, I64], vec![]),
        Helper::ListGet => (vec![I32, I64], vec![I64]),
        Helper::ListLen => (vec![I32], vec![I64]),
        Helper::ListDestroy => (vec![I32], vec![]),
        Helper::MapNew => (vec![I32], vec![I32]),
        Helper::MapLen => (vec![I32], vec![I64]),
        Helper::HashStr => (vec![I32], vec![I64]),
        Helper::HashInt => (vec![I64], vec![I64]),
        Helper::MapProbe => (vec![I32, I64, I64], vec![I64]),
        Helper::MapRehash => (vec![I32, I64], vec![]),
        Helper::MapPut => (vec![I32, I64, I64, I64], vec![]),
        Helper::MapGet | Helper::MapHas => (vec![I32, I64, I64], vec![I64]),
        Helper::MapRemove => (vec![I32, I64, I64], vec![]),
        Helper::MapDestroy => (vec![I32], vec![]),
        Helper::TupleAlloc => (vec![I32, I64, I64], vec![I32]),
        Helper::TupleDestroy | Helper::ClosureDestroy | Helper::TaskDestroy => {
            (vec![I32], vec![])
        }
        Helper::TaskNew => (vec![I64, I32, I64], vec![I32]),
        Helper::TaskRun => (vec![I32], vec![I64]),
        Helper::TaskAwait | Helper::TaskPoll => (vec![I32], vec![I64]),
        Helper::SliceNew => (vec![I32, I64, I64, I64], vec![I32]),
        Helper::SliceLen => (vec![I32], vec![I64]),
        Helper::SliceGet => (vec![I32, I64], vec![I64]),
        Helper::StrSliceGet => (vec![I32, I64], vec![I32]),
        Helper::StrSliceToStr => (vec![I32], vec![I32]),
        Helper::IntPow => (vec![I64, I64], vec![I64]),
        Helper::Log2 | Helper::Exp2 => (vec![F64], vec![F64]),
        Helper::Pow => (vec![F64, F64], vec![F64]),
        Helper::Reduce2Pi | Helper::SinPoly | Helper::CosPoly => (vec![F64], vec![F64]),
        Helper::Trig => (vec![F64, I32], vec![F64]),
        Helper::TimeSeed => (vec![], vec![I64]),
        Helper::Random => (vec![], vec![I64]),
        Helper::RandomRange => (vec![I64, I64], vec![I64]),
        Helper::TimeMs => (vec![], vec![I64]),
        Helper::SleepMs => (vec![I64], vec![]),
    }
}

// ── Feature validation & planning scan ───────────────────────────────────────

/// Builtins the wasm backend implements natively inside the module.
const SUPPORTED_BUILTINS: &[&str] = &[
    // printing & scalar predicates
    "lpp_print_int",
    "lpp_print_bool",
    "lpp_print_float",
    "lpp_print_str",
    "lpp_str_len",
    "lpp_str_eq",
    "fmod",
    // dynamic strings
    "lpp_str_concat",
    "lpp_str_substr",
    "lpp_str_trim",
    "lpp_str_upper",
    "lpp_str_lower",
    "lpp_int_to_str",
    "lpp_float_to_str",
    "lpp_bool_to_str",
    "lpp_str_to_int",
    "lpp_parse_int",
    "lpp_char_at",
    "lpp_chr",
    "lpp_ord",
    "lpp_str_contains",
    "lpp_str_find",
    "lpp_str_starts_with",
    "lpp_str_ends_with",
    "lpp_str_repeat",
    "lpp_str_replace",
    "lpp_str_split",
    // process & environment
    "lpp_input",
    "lpp_exit",
    "lpp_env_get",
    // math & time
    "lpp_abs",
    "lpp_min",
    "lpp_max",
    "lpp_int_pow",
    "lpp_pow",
    "lpp_floor",
    "lpp_ceil",
    "lpp_sqrt",
    "lpp_sin",
    "lpp_cos",
    "lpp_tan",
    "lpp_int_to_float",
    "lpp_float_to_int",
    "lpp_random_seed",
    "lpp_random",
    "lpp_random_range",
    "lpp_time_ms",
    "lpp_sleep_ms",
    // lists
    "lpp_list_new",
    "lpp_list_new_arc",
    "lpp_list_push",
    "lpp_list_push_bool",
    "lpp_list_push_float",
    "lpp_list_push_arc",
    "lpp_list_get",
    "lpp_list_get_bool",
    "lpp_list_get_float",
    "lpp_list_get_arc",
    "lpp_list_set",
    "lpp_list_set_bool",
    "lpp_list_set_float",
    "lpp_list_set_arc",
    "lpp_list_len",
    "lpp_list_free",
    // maps
    "lpp_map_new",
    "lpp_map_new_arc",
    "lpp_map_put",
    "lpp_map_put_str",
    "lpp_map_put_float",
    "lpp_map_put_str_float",
    "lpp_map_get",
    "lpp_map_get_str",
    "lpp_map_get_float",
    "lpp_map_get_str_float",
    "lpp_map_has",
    "lpp_map_has_str",
    "lpp_map_remove",
    "lpp_map_remove_str",
    "lpp_map_len",
    // arc & arena plumbing (wasm arena = plain ARC, see module docs)
    "lpp_arena_begin",
    "lpp_arena_release",
    "lpp_arena_retain",
    "lpp_arena_release_node",
    "lpp_arc_retain",
    "lpp_arc_retain_local",
    "lpp_arc_release",
    "lpp_arc_release_local",
    "lpp_free_str",
    "lpp_alloc",
    "lpp_free",
    // task builtins (assets of async rvalues)
    "lpp_task_destroy",
    "lpp_task_poll",
    "lpp_executor_run",
    "lpp_task_await",
    // slice builtins (MIR mostly uses rvalues; symbols are accepted too)
    "lpp_slice_len",
    "lpp_slice_get",
    "lpp_slice_get_bool",
    "lpp_slice_get_float",
    "lpp_str_slice_get",
    "lpp_str_slice_to_str",
];

/// Feature families the WASI sandbox physically cannot provide. Used for
/// precise rejection diagnostics instead of a bare "unknown builtin".
fn unsupported_builtin_family(symbol: &str) -> Option<&'static str> {
    let short = symbol.strip_prefix("lpp_").unwrap_or(symbol);
    if short.starts_with("net_") {
        return Some("network sockets (WASI preview1 has no socket API)");
    }
    if short.starts_with("gui_") {
        return Some("GUI windows (a WebAssembly sandbox has no display server)");
    }
    if short.starts_with("sys_") {
        return Some("host system metrics (WASI exposes no /proc equivalent)");
    }
    if short.starts_with("command_") || short == "exit_code" {
        return Some("spawning child processes (not part of WASI preview1)");
    }
    if short.starts_with("json_") {
        return Some("the JSON runtime library (not yet ported to the wasm backend)");
    }
    if short.starts_with("buf_") {
        return Some("raw byte buffers (not yet ported to the wasm backend)");
    }
    if short == "read_file"
        || short == "write_file"
        || short == "append_file"
        || short == "file_exists"
        || short == "path_exists"
        || short == "file_size"
        || short == "file_copy"
        || short == "file_move"
        || short == "delete_file"
        || short == "dir_create"
        || short == "dir_list"
        || short == "dir_remove"
        || short == "path_join"
    {
        return Some("file system access (WASI preopens are not wired up in the wasm backend yet)");
    }
    if short == "env_set" {
        return Some("mutating the process environment (WASI environments are immutable)");
    }
    if short.starts_with("vec_i64x2") || short == "vec_i64_checksum" {
        return Some("128-bit SIMD vectors (kept native-only for now)");
    }
    if short == "thread_spawn" || short == "thread_join" {
        return Some("OS threads (WebAssembly/WASI preview1 has no preemptive threads)");
    }
    None
}

fn wasm_type_error(ty: &TypeRef, where_: &str) -> String {
    let feature = match ty {
        TypeRef::VectorI64x2 => "SIMD vector types".to_string(),
        other => format!("type {:?}", other),
    };
    format!(
        "WebAssembly backend does not support {} (in {}). Native targets are unaffected; only the wasm32 backend is limited.",
        feature, where_
    )
}

fn validate_local_type(ty: &TypeRef, where_: &str) -> Result<(), String> {
    if matches!(ty, TypeRef::VectorI64x2) {
        return Err(wasm_type_error(ty, where_));
    }
    Ok(())
}

/// Result of walking the whole program once before codegen.
struct ProgramScan {
    /// Builtin symbol → number of call sites (drives helper planning).
    builtin_uses: HashMap<String, u32>,
    /// MIR functions whose address is taken (closures, FuncRef, vtables).
    addressable: Vec<FuncId>,
    /// Async MIR functions that need a task thunk.
    task_fns: Vec<FuncId>,
}

fn scan_rvalue(
    rvalue: &Rvalue,
    fn_name: &str,
    scan: &mut ProgramScan,
    addressable: &mut HashSet<FuncId>,
) -> Result<(), String> {
    let unsupported = |feature: &str| -> String {
        format!(
            "WebAssembly backend does not support {} (in '{}')",
            feature, fn_name
        )
    };
    match rvalue {
        Rvalue::Use(_)
        | Rvalue::Move(_)
        | Rvalue::BinaryOp(..)
        | Rvalue::CallDirect(..)
        | Rvalue::CallIndirect(..)
        | Rvalue::AllocateTuple(..)
        | Rvalue::TupleField(..)
        | Rvalue::MakeSlice { .. }
        | Rvalue::SliceLen(_)
        | Rvalue::SliceGet(..)
        | Rvalue::SliceToStr(_)
        | Rvalue::MakeTask(..)
        | Rvalue::Await(_)
        | Rvalue::FieldAccess(..)
        | Rvalue::AllocateArcStruct(..)
        | Rvalue::AllocateArenaStruct(..)
        | Rvalue::AllocateStackStruct(..)
        | Rvalue::AllocateList(_) => Ok(()),
        Rvalue::MakeClosure(mir_func_id, _) | Rvalue::MakeStackClosure(mir_func_id, _) => {
            addressable.insert(*mir_func_id);
            Ok(())
        }
        Rvalue::FuncRef(mir_func_id) => {
            addressable.insert(*mir_func_id);
            Ok(())
        }
        Rvalue::AllocateStruct(_) => Err(unsupported(
            "raw struct allocation (the same legacy form the native AOT rejects)",
        )),
        Rvalue::SpawnThread(_) => Err(format!(
            "WebAssembly backend does not support OS threads ('spawn') in '{}': WASI preview1 has no preemptive threading. \
             Use async/await tasks, which run on the deterministic single-thread executor.",
            fn_name
        )),
        Rvalue::BuiltinCall(symbol, _) => {
            if SUPPORTED_BUILTINS.contains(&symbol.as_str()) {
                *scan.builtin_uses.entry(symbol.clone()).or_insert(0) += 1;
                Ok(())
            } else if let Some(family) = unsupported_builtin_family(symbol) {
                Err(format!(
                    "WebAssembly backend does not support builtin '{}' ({}). Native targets are unaffected.",
                    symbol, family
                ))
            } else {
                Err(format!(
                    "WebAssembly backend does not provide builtin '{}' (C FFI/extern symbols are unavailable on wasm32; \
                     referenced in function '{}')",
                    symbol, fn_name
                ))
            }
        }
    }
}

/// Validate the whole program for the wasm target and collect planning data.
fn validate_program(program: &MirProgram) -> Result<ProgramScan, String> {
    let mut scan = ProgramScan {
        builtin_uses: HashMap::new(),
        addressable: Vec::new(),
        task_fns: Vec::new(),
    };
    let mut addressable: HashSet<FuncId> = HashSet::new();

    let mut functions: Vec<&MirFunction> = program.functions.values().collect();
    functions.sort_by_key(|f| f.id.0);

    for function in functions {
        if function.is_async {
            scan.task_fns.push(function.id);
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
                        scan_rvalue(rvalue, &function.name, &mut scan, &mut addressable)?
                    }
                    MirInstr::AssignField { .. } | MirInstr::Retain(_) | MirInstr::Release(_) => {}
                }
            }
        }
    }
    let mut sorted: Vec<FuncId> = addressable.into_iter().collect();
    sorted.sort_by_key(|f| f.0);
    scan.addressable = sorted;
    Ok(scan)
}

// ── Function body builder ────────────────────────────────────────────────────

/// Instruction emission helper for one function body. Tracks declared extra
/// (non-parameter) locals so helpers can allocate scratch locals by type.
struct FB {
    body: Vec<u8>,
    /// Declared extra locals (wasm local groups, in declaration order).
    extras: Vec<Val>,
    /// Number of parameters — extra local indices start after them.
    params: u32,
}

impl FB {
    fn new(params: u32) -> Self {
        Self {
            body: Vec::with_capacity(128),
            extras: Vec::new(),
            params,
        }
    }

    /// Allocate a scratch local of the given type, returning its index.
    fn scratch(&mut self, val: Val) -> u32 {
        let index = self.params + self.extras.len() as u32;
        self.extras.push(val);
        index
    }

    fn op(&mut self, op: u8) -> &mut Self {
        self.body.push(op);
        self
    }

    fn i32c(&mut self, v: i64) -> &mut Self {
        self.body.push(op::I32_CONST);
        sleb(&mut self.body, v);
        self
    }

    fn i64c(&mut self, v: i64) -> &mut Self {
        self.body.push(op::I64_CONST);
        sleb(&mut self.body, v);
        self
    }

    fn f64c(&mut self, v: f64) -> &mut Self {
        self.body.push(op::F64_CONST);
        self.body.extend_from_slice(&v.to_le_bytes());
        self
    }

    fn g(&mut self, local: u32) -> &mut Self {
        self.body.push(op::LOCAL_GET);
        uleb(&mut self.body, local as u64);
        self
    }

    fn s(&mut self, local: u32) -> &mut Self {
        self.body.push(op::LOCAL_SET);
        uleb(&mut self.body, local as u64);
        self
    }

    fn t(&mut self, local: u32) -> &mut Self {
        self.body.push(op::LOCAL_TEE);
        uleb(&mut self.body, local as u64);
        self
    }

    fn gget(&mut self, global: u32) -> &mut Self {
        self.body.push(op::GLOBAL_GET);
        uleb(&mut self.body, global as u64);
        self
    }

    fn gset(&mut self, global: u32) -> &mut Self {
        self.body.push(op::GLOBAL_SET);
        uleb(&mut self.body, global as u64);
        self
    }

    fn call(&mut self, index: u32) -> &mut Self {
        self.body.push(op::CALL);
        uleb(&mut self.body, index as u64);
        self
    }

    fn call_indirect(&mut self, type_index: u32) -> &mut Self {
        self.body.push(op::CALL_INDIRECT);
        uleb(&mut self.body, type_index as u64);
        self.body.push(0x00); // table 0
        self
    }

    fn block(&mut self) -> &mut Self {
        self.body.push(op::BLOCK);
        self.body.push(op::BLOCK_VOID);
        self
    }

    fn loop_(&mut self) -> &mut Self {
        self.body.push(op::LOOP);
        self.body.push(op::BLOCK_VOID);
        self
    }

    fn if_(&mut self) -> &mut Self {
        self.body.push(op::IF);
        self.body.push(op::BLOCK_VOID);
        self
    }

    fn else_(&mut self) -> &mut Self {
        self.body.push(op::ELSE);
        self
    }

    fn end(&mut self) -> &mut Self {
        self.body.push(op::END);
        self
    }

    fn br(&mut self, depth: u32) -> &mut Self {
        self.body.push(op::BR);
        uleb(&mut self.body, depth as u64);
        self
    }

    fn br_if(&mut self, depth: u32) -> &mut Self {
        self.body.push(op::BR_IF);
        uleb(&mut self.body, depth as u64);
        self
    }

    fn load32(&mut self, offset: u32) -> &mut Self {
        self.body.push(op::I32_LOAD);
        self.body.extend_from_slice(&[0]);
        uleb(&mut self.body, offset as u64);
        self
    }

    fn store32(&mut self, offset: u32) -> &mut Self {
        self.body.push(op::I32_STORE);
        self.body.extend_from_slice(&[0]);
        uleb(&mut self.body, offset as u64);
        self
    }

    fn load64(&mut self, offset: u32) -> &mut Self {
        self.body.push(op::I64_LOAD);
        self.body.extend_from_slice(&[0]);
        uleb(&mut self.body, offset as u64);
        self
    }

    fn store64(&mut self, offset: u32) -> &mut Self {
        self.body.push(op::I64_STORE);
        self.body.extend_from_slice(&[0]);
        uleb(&mut self.body, offset as u64);
        self
    }

    fn loadf64(&mut self, offset: u32) -> &mut Self {
        self.body.push(op::F64_LOAD);
        self.body.extend_from_slice(&[0]);
        uleb(&mut self.body, offset as u64);
        self
    }

    fn storef64(&mut self, offset: u32) -> &mut Self {
        self.body.push(op::F64_STORE);
        self.body.extend_from_slice(&[0]);
        uleb(&mut self.body, offset as u64);
        self
    }

    fn load8(&mut self, offset: u32) -> &mut Self {
        self.body.push(op::I32_LOAD8_U);
        self.body.extend_from_slice(&[0]);
        uleb(&mut self.body, offset as u64);
        self
    }

    fn store8(&mut self, offset: u32) -> &mut Self {
        self.body.push(op::I32_STORE8);
        self.body.extend_from_slice(&[0]);
        uleb(&mut self.body, offset as u64);
        self
    }

    fn memory_copy(&mut self) -> &mut Self {
        self.body.push(op::BULK_PREFIX);
        uleb(&mut self.body, op::MEMORY_COPY_SUB);
        self.body.extend_from_slice(&[0, 0]);
        self
    }

    fn memory_fill(&mut self) -> &mut Self {
        self.body.push(op::BULK_PREFIX);
        uleb(&mut self.body, op::MEMORY_FILL_SUB);
        self.body.push(0);
        self
    }

    fn memory_size(&mut self) -> &mut Self {
        self.body.push(op::MEMORY_SIZE_PREFIX);
        self.body.push(0);
        self
    }

    fn memory_grow(&mut self) -> &mut Self {
        self.body.push(op::MEMORY_GROW_PREFIX);
        self.body.push(0);
        self
    }
}

// ── Compiler ─────────────────────────────────────────────────────────────────

/// Drop-fn wasm-table seats: table[0] is a trap stub (null placeholder),
/// tables seats 1..=S hold one synthesized destructor per struct type, then
/// come the generic runtime destructors, then async task thunks, then the
/// user functions whose address can be taken.
struct TablePlan {
    /// Number of struct destructors (seats 1..=struct_drops).
    struct_drops: u32,
    tuple_destroy: u32,
    closure_destroy: u32,
    list_destroy: u32,
    map_destroy: u32,
    task_destroy: u32,
    /// First seat of the async thunk range.
    thunks_start: u32,
    /// First seat of the addressable user-function range.
    addressable_start: u32,
    /// Total table size.
    total: u32,
}

struct WasmCompiler<'a> {
    program: &'a MirProgram,
    type_table: &'a TypeTable,
    weak_fields: &'a HashSet<(StructTypeId, String)>,

    // Function index assignment.
    imports: Vec<Wasi>,
    import_index: HashMap<Wasi, u32>,
    fn_index: HashMap<FuncId, u32>,
    helper_index: HashMap<Helper, u32>,
    struct_drop_fn: HashMap<StructTypeId, u32>,
    thunk_fn: HashMap<FuncId, u32>,
    start_index: u32,
    /// wasm function index → signature, for the validator tests.
    all_sigs: Vec<(Vec<Val>, Vec<Val>)>,

    // Wasm function table (call_indirect targets).
    table: TablePlan,
    table_seat: HashMap<FuncId, u32>,
    /// Async MIR functions in deterministic thunk order (for MakeTask seats).
    task_order: Vec<FuncId>,

    // Registered deduped signatures: (params, results) → type index.
    types: Vec<(Vec<Val>, Vec<Val>)>,
    type_map: HashMap<(Vec<Val>, Vec<Val>), u32>,

    /// Interned literal → payload address (pointer handed to code).
    literals: HashMap<String, u32>,
    /// Absolute-address scratch buffer; valid bytes start at POOL_START.
    pool: Vec<u8>,
    /// Address of the immortal empty string (allocated lazily, cached).
    empty_str: Option<u32>,

    /// Per wasm-function-index function names (name section).
    names: Vec<(u32, String)>,

    /// Helpers selected by the planning scan.
    helpers: Vec<Helper>,
}

impl<'a> WasmCompiler<'a> {
    fn new(
        program: &'a MirProgram,
        type_table: &'a TypeTable,
        weak_fields: &'a HashSet<(StructTypeId, String)>,
    ) -> Self {
        Self {
            program,
            type_table,
            weak_fields,
            imports: Vec::new(),
            import_index: HashMap::new(),
            fn_index: HashMap::new(),
            helper_index: HashMap::new(),
            struct_drop_fn: HashMap::new(),
            thunk_fn: HashMap::new(),
            start_index: 0,
            all_sigs: Vec::new(),
            table: TablePlan {
                struct_drops: 0,
                tuple_destroy: 0,
                closure_destroy: 0,
                list_destroy: 0,
                map_destroy: 0,
                task_destroy: 0,
                thunks_start: 0,
                addressable_start: 0,
                total: 1,
            },
            table_seat: HashMap::new(),
            task_order: Vec::new(),
            types: Vec::new(),
            type_map: HashMap::new(),
            literals: HashMap::new(),
            pool: vec![0u8; POOL_START as usize],
            empty_str: None,
            names: Vec::new(),
            helpers: Vec::new(),
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

    /// The registered (deduped) type index of a helper's own signature —
    /// used for `call_indirect` through destructors.
    fn drop_call_type(&mut self) -> u32 {
        self.register_type(vec![Val::I32], vec![])
    }

    fn task_call_type(&mut self) -> u32 {
        self.register_type(vec![Val::I32], vec![Val::I64])
    }

    // ── Static string pool ───────────────────────────────────────────────

    /// Intern bytes into the static pool, returning the payload pointer
    /// (address of the length field; the ARC header precedes it).
    fn intern(&mut self, bytes: &[u8]) -> u32 {
        let key = String::from_utf8_lossy(bytes).into_owned();
        self.intern_key(&key)
    }

    /// Byte address of an interned literal's *content*. `intern` returns the
    /// payload pointer `…[u32 len][bytes]` so literals double as `Str`
    /// values; raw `Write`/`WriteFd` calls must skip the 4-byte length
    /// prefix, otherwise they emit the length byte instead of the content
    /// (this once printed every trailing '\n' as 0x01 in stdout).
    fn lit_addr(&mut self, bytes: &[u8]) -> u32 {
        self.intern(bytes) + 4
    }

    fn intern_key(&mut self, key: &str) -> u32 {
        if let Some(&addr) = self.literals.get(key) {
            return addr;
        }
        // 8-align each entry so the faux ARC header stays aligned.
        while self.pool.len() % 8 != 0 {
            self.pool.push(0);
        }
        let base = self.pool.len() as u32;
        self.pool.extend_from_slice(&ARC_IMMORTAL.to_le_bytes()); // refcount
        self.pool.extend_from_slice(&0i64.to_le_bytes()); // no destructor
        self.pool.extend_from_slice(&0x4C505057u64.to_le_bytes()); // "LPPW" magic
        self.pool.extend_from_slice(&(key.len() as u32).to_le_bytes());
        self.pool.extend_from_slice(key.as_bytes());
        let payload = base + ARC_HEADER_SIZE as u32;
        self.literals.insert(key.to_string(), payload);
        payload
    }

    fn lookup_literal(&self, value: &str) -> u32 {
        *self
            .literals
            .get(value)
            .expect("string literals are interned during the pre-codegen walk")
    }

    fn empty_string_addr(&mut self) -> u32 {
        if let Some(addr) = self.empty_str {
            return addr;
        }
        let addr = self.intern_key("");
        self.empty_str = Some(addr);
        addr
    }

    /// The bump-allocator heap begins right after the static pool.
    fn heap_base(&self) -> u32 {
        ((self.pool.len() as u32) + 7) & !7
    }

    // ── Planning ─────────────────────────────────────────────────────────

    fn use_import(&mut self, wasi: Wasi) {
        if !self.imports.contains(&wasi) {
            self.imports.push(wasi);
        }
    }

    /// Helpers a helper itself calls. The planner closes the seed set under
    /// this relation so an emitted helper never references a missing one.
    fn helper_deps(helper: Helper) -> &'static [Helper] {
        match helper {
            Helper::TrapStub => &[],
            Helper::Alloc => &[],
            Helper::ArcAlloc => &[Helper::Alloc],
            Helper::Retain => &[],
            Helper::Release => &[],
            Helper::StrAlloc => &[Helper::ArcAlloc],
            Helper::StrNew => &[Helper::StrAlloc],
            Helper::WriteFd => &[],
            Helper::Write => &[Helper::WriteFd],
            Helper::FmtU64 => &[],
            Helper::WriteU64 => &[Helper::FmtU64, Helper::Write],
            Helper::PrintInt => &[Helper::Write, Helper::WriteU64],
            Helper::PrintBool => &[Helper::Write],
            Helper::PrintFloat => &[Helper::Write, Helper::WriteU64],
            Helper::PrintStr => &[Helper::Write],
            Helper::PanicMsg => &[Helper::WriteFd, Helper::Write],
            Helper::Panic2 => &[Helper::WriteFd, Helper::Write, Helper::WriteU64],
            Helper::Panic3 => &[Helper::WriteFd, Helper::Write, Helper::WriteU64],
            Helper::Exit => &[],
            Helper::Input => &[Helper::StrNew],
            Helper::EnvGet => &[Helper::StrNew, Helper::Alloc, Helper::EnvMatch],
            Helper::EnvMatch => &[],
            Helper::StrLen => &[],
            Helper::StrEq => &[],
            Helper::StrConcat => &[Helper::StrAlloc],
            Helper::StrSubstr => &[Helper::StrAlloc],
            Helper::StrTrim => &[Helper::StrAlloc],
            Helper::StrUpper => &[Helper::StrAlloc],
            Helper::StrLower => &[Helper::StrAlloc],
            Helper::IntToStr => &[Helper::StrAlloc, Helper::FmtU64],
            Helper::FloatToStr => &[Helper::StrNew, Helper::StrAlloc, Helper::FmtU64],
            Helper::BoolToStr => &[],
            Helper::StrToInt => &[],
            Helper::CharAt => &[Helper::StrAlloc, Helper::PanicMsg],
            Helper::Ord => &[],
            Helper::Chr => &[Helper::StrAlloc],
            Helper::StrFindFrom => &[],
            Helper::StrFind => &[Helper::StrFindFrom],
            Helper::StrContains => &[Helper::StrFindFrom],
            Helper::StrStartsWith => &[],
            Helper::StrEndsWith => &[],
            Helper::StrRepeat => &[Helper::StrAlloc, Helper::PanicMsg],
            Helper::StrReplace => &[Helper::StrAlloc, Helper::StrFindFrom, Helper::StrNew],
            Helper::StrSplit => &[
                Helper::StrNew,
                Helper::ListNew,
                Helper::ListPush,
                Helper::Release,
            ],
            Helper::ListNew => &[Helper::ArcAlloc],
            Helper::ListPush => &[Helper::Alloc, Helper::Retain, Helper::PanicMsg],
            Helper::ListSet => &[Helper::Retain, Helper::Release, Helper::Panic2, Helper::PanicMsg],
            Helper::ListGet => &[Helper::Panic2, Helper::PanicMsg],
            Helper::ListLen => &[],
            Helper::ListDestroy => &[Helper::Release],
            Helper::MapNew => &[Helper::ArcAlloc, Helper::Alloc],
            Helper::MapLen => &[],
            Helper::HashStr => &[],
            Helper::HashInt => &[],
            Helper::MapProbe => &[Helper::HashStr, Helper::HashInt, Helper::StrEq],
            Helper::MapRehash => &[Helper::Alloc, Helper::HashStr, Helper::HashInt],
            Helper::MapPut => &[Helper::MapProbe, Helper::MapRehash, Helper::Retain, Helper::Release],
            Helper::MapGet => &[Helper::MapProbe],
            Helper::MapHas => &[Helper::MapProbe],
            Helper::MapRemove => &[Helper::MapProbe, Helper::Release],
            Helper::MapDestroy => &[Helper::Release],
            Helper::TupleAlloc => &[Helper::ArcAlloc],
            Helper::TupleDestroy => &[Helper::Release],
            Helper::ClosureDestroy => &[Helper::Release],
            Helper::TaskNew => &[Helper::ArcAlloc, Helper::PanicMsg],
            Helper::TaskRun => &[Helper::PanicMsg],
            Helper::TaskAwait => &[Helper::TaskRun, Helper::Retain],
            Helper::TaskPoll => &[Helper::TaskRun],
            Helper::TaskDestroy => &[Helper::Release],
            Helper::SliceNew => &[
                Helper::Alloc,
                Helper::PanicMsg,
                Helper::Panic2,
                Helper::Panic3,
                Helper::StrLen,
                Helper::ListLen,
            ],
            Helper::SliceLen => &[Helper::PanicMsg],
            Helper::SliceGet => &[Helper::Panic2, Helper::PanicMsg, Helper::ListGet],
            Helper::StrSliceGet => &[Helper::Panic2, Helper::PanicMsg, Helper::StrAlloc],
            Helper::StrSliceToStr => &[Helper::StrNew, Helper::PanicMsg],
            Helper::IntPow => &[],
            Helper::Log2 => &[],
            Helper::Exp2 => &[],
            Helper::Pow => &[Helper::Log2, Helper::Exp2],
            Helper::Reduce2Pi => &[],
            Helper::SinPoly => &[],
            Helper::CosPoly => &[],
            Helper::Trig => &[Helper::Reduce2Pi, Helper::SinPoly, Helper::CosPoly],
            Helper::TimeSeed => &[],
            Helper::Random => &[Helper::TimeSeed],
            Helper::RandomRange => &[Helper::Random],
            Helper::TimeMs => &[],
            Helper::SleepMs => &[],
        }
    }

    /// WASI imports a helper needs.
    fn helper_imports(helper: Helper) -> &'static [Wasi] {
        match helper {
            Helper::WriteFd => &[Wasi::FdWrite],
            Helper::Input => &[Wasi::FdRead],
            Helper::Exit => &[Wasi::ProcExit],
            Helper::TimeSeed | Helper::TimeMs => &[Wasi::ClockTimeGet],
            Helper::SleepMs => &[Wasi::PollOneoff],
            Helper::EnvGet => &[Wasi::EnvironSizesGet, Wasi::EnvironGet],
            _ => &[],
        }
    }

    /// Decide which helpers to emit from the builtin usage scan and the
    /// MIR rvalue surface. Seeds are collected per feature, then closed under
    /// `helper_deps`. WASI imports follow from the final helper set.
    fn plan_helpers(&mut self, scan: &ProgramScan) {
        let uses = &scan.builtin_uses;
        let mut helpers: HashSet<Helper> = HashSet::new();
        let mut need = |h: Helper| {
            helpers.insert(h);
        };

        // Table-seat holders are unconditional: the elem segment always
        // references them.
        for h in [
            Helper::TrapStub,
            Helper::TupleDestroy,
            Helper::ClosureDestroy,
            Helper::ListDestroy,
            Helper::MapDestroy,
            Helper::TaskDestroy,
        ] {
            need(h);
        }

        // Feature flags from the MIR rvalue surface.
        let mut want_lists = false;
        let mut want_slices = false;
        let mut want_tuples = false;
        for function in self.program.functions.values() {
            for block in &function.blocks {
                for instr in &block.instrs {
                    if let MirInstr::Assign(_, rvalue) = instr {
                        match rvalue {
                            Rvalue::AllocateList(_) => want_lists = true,
                            Rvalue::MakeSlice { .. }
                            | Rvalue::SliceLen(_)
                            | Rvalue::SliceGet(..)
                            | Rvalue::SliceToStr(_) => want_slices = true,
                            Rvalue::AllocateTuple(..) | Rvalue::MakeTask(..) => {
                                want_tuples = true
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        let any = |names: &[&str]| names.iter().any(|n| uses.contains_key(*n));

        if uses.contains_key("lpp_print_int") {
            need(Helper::PrintInt);
        }
        if uses.contains_key("lpp_print_bool") {
            need(Helper::PrintBool);
        }
        if uses.contains_key("lpp_print_float") {
            need(Helper::PrintFloat);
        }
        if uses.contains_key("lpp_print_str") {
            need(Helper::PrintStr);
        }
        if uses.contains_key("lpp_str_len") {
            need(Helper::StrLen);
        }
        if uses.contains_key("lpp_str_eq") {
            need(Helper::StrEq);
        }
        if uses.contains_key("lpp_str_concat") {
            need(Helper::StrConcat);
        }
        if uses.contains_key("lpp_str_substr") {
            need(Helper::StrSubstr);
        }
        if uses.contains_key("lpp_str_trim") {
            need(Helper::StrTrim);
        }
        if uses.contains_key("lpp_str_upper") {
            need(Helper::StrUpper);
        }
        if uses.contains_key("lpp_str_lower") {
            need(Helper::StrLower);
        }
        if uses.contains_key("lpp_int_to_str") {
            need(Helper::IntToStr);
        }
        if uses.contains_key("lpp_float_to_str") {
            need(Helper::FloatToStr);
        }
        if uses.contains_key("lpp_bool_to_str") {
            need(Helper::BoolToStr);
        }
        if uses.contains_key("lpp_str_to_int") || uses.contains_key("lpp_parse_int") {
            need(Helper::StrToInt);
        }
        if uses.contains_key("lpp_char_at") {
            need(Helper::CharAt);
        }
        if uses.contains_key("lpp_ord") {
            need(Helper::Ord);
        }
        if uses.contains_key("lpp_chr") {
            need(Helper::Chr);
        }
        if uses.contains_key("lpp_str_find") {
            need(Helper::StrFind);
        }
        if uses.contains_key("lpp_str_contains") {
            need(Helper::StrContains);
        }
        if uses.contains_key("lpp_str_starts_with") {
            need(Helper::StrStartsWith);
        }
        if uses.contains_key("lpp_str_ends_with") {
            need(Helper::StrEndsWith);
        }
        if uses.contains_key("lpp_str_repeat") {
            need(Helper::StrRepeat);
        }
        if uses.contains_key("lpp_str_replace") {
            need(Helper::StrReplace);
        }
        if uses.contains_key("lpp_str_split") {
            need(Helper::StrSplit);
        }
        if uses.contains_key("lpp_input") {
            need(Helper::Input);
        }
        if uses.contains_key("lpp_exit") {
            need(Helper::Exit);
        }
        if uses.contains_key("lpp_env_get") {
            need(Helper::EnvGet);
        }
        if uses.contains_key("lpp_int_pow") {
            need(Helper::IntPow);
        }
        if uses.contains_key("lpp_pow") {
            need(Helper::Pow);
        }
        if any(&["lpp_sin", "lpp_cos", "lpp_tan"]) {
            need(Helper::Trig);
        }
        if any(&["lpp_random", "lpp_random_range"]) {
            need(Helper::Random);
        }
        if uses.contains_key("lpp_random_range") {
            need(Helper::RandomRange);
        }
        if uses.contains_key("lpp_time_ms") {
            need(Helper::TimeMs);
        }
        if uses.contains_key("lpp_sleep_ms") {
            need(Helper::SleepMs);
        }
        if any(&[
            "lpp_list_new",
            "lpp_list_new_arc",
            "lpp_list_push",
            "lpp_list_push_bool",
            "lpp_list_push_float",
            "lpp_list_push_arc",
            "lpp_list_get",
            "lpp_list_get_bool",
            "lpp_list_get_float",
            "lpp_list_get_arc",
            "lpp_list_set",
            "lpp_list_set_bool",
            "lpp_list_set_float",
            "lpp_list_set_arc",
            "lpp_list_len",
            "lpp_list_free",
        ]) {
            want_lists = true;
        }
        if uses.contains_key("lpp_task_poll") {
            need(Helper::TaskPoll);
        }
        if uses.contains_key("lpp_task_await") {
            need(Helper::TaskAwait);
        }
        if uses.contains_key("lpp_executor_run") {
            need(Helper::TaskRun);
        }
        if uses.contains_key("lpp_slice_len") {
            need(Helper::SliceLen);
        }
        if any(&["lpp_slice_get", "lpp_slice_get_bool", "lpp_slice_get_float"]) {
            need(Helper::SliceGet);
            want_slices = true;
        }
        if uses.contains_key("lpp_str_slice_get") {
            need(Helper::StrSliceGet);
            want_slices = true;
        }
        if uses.contains_key("lpp_str_slice_to_str") {
            need(Helper::StrSliceToStr);
            want_slices = true;
        }
        if any(&[
            "lpp_map_put",
            "lpp_map_put_str",
            "lpp_map_put_float",
            "lpp_map_put_str_float",
        ]) {
            need(Helper::MapPut);
        }
        if any(&[
            "lpp_map_get",
            "lpp_map_get_str",
            "lpp_map_get_float",
            "lpp_map_get_str_float",
        ]) {
            need(Helper::MapGet);
        }
        if any(&["lpp_map_has", "lpp_map_has_str"]) {
            need(Helper::MapHas);
        }
        if any(&["lpp_map_remove", "lpp_map_remove_str"]) {
            need(Helper::MapRemove);
        }
        if any(&[
            "lpp_map_new",
            "lpp_map_new_arc",
            "lpp_map_put",
            "lpp_map_put_str",
            "lpp_map_put_float",
            "lpp_map_put_str_float",
            "lpp_map_get",
            "lpp_map_get_str",
            "lpp_map_get_float",
            "lpp_map_get_str_float",
            "lpp_map_has",
            "lpp_map_has_str",
            "lpp_map_remove",
            "lpp_map_remove_str",
            "lpp_map_len",
        ]) {
            need(Helper::MapNew);
            need(Helper::MapLen);
        }
        if any(&["lpp_arc_retain", "lpp_arc_retain_local", "lpp_arena_retain"]) {
            need(Helper::Retain);
        }
        if any(&[
            "lpp_arc_release",
            "lpp_arc_release_local",
            "lpp_arena_release_node",
            "lpp_free_str",
            "lpp_list_free",
            "lpp_task_destroy",
        ]) {
            need(Helper::Release);
        }
        if uses.contains_key("lpp_alloc") {
            need(Helper::Alloc);
        }

        // Feature-driven seeds.
        if want_lists {
            for h in [
                Helper::ListNew,
                Helper::ListPush,
                Helper::ListGet,
                Helper::ListSet,
                Helper::ListLen,
            ] {
                need(h);
            }
        }
        if want_slices {
            for h in [
                Helper::SliceNew,
                Helper::SliceLen,
                Helper::SliceGet,
                Helper::StrSliceGet,
                Helper::StrSliceToStr,
            ] {
                need(h);
            }
        }
        if want_tuples {
            need(Helper::TupleAlloc);
        }
        if !scan.task_fns.is_empty() {
            // The async entrypoint wrapper also builds an empty env tuple.
            for h in [
                Helper::TaskNew,
                Helper::TaskRun,
                Helper::TaskAwait,
                Helper::TupleAlloc,
            ] {
                need(h);
            }
        }
        // Any Retain/Release instruction or managed allocation pulls the ARC
        // core in; cheaper to seed it whenever the program is non-trivial.
        let mut has_arc_instrs = false;
        for function in self.program.functions.values() {
            for block in &function.blocks {
                for instr in &block.instrs {
                    match instr {
                        MirInstr::Retain(_) | MirInstr::Release(_) => has_arc_instrs = true,
                        MirInstr::Assign(_, rvalue) => match rvalue {
                            Rvalue::AllocateTuple(..)
                            | Rvalue::MakeClosure(..)
                            | Rvalue::MakeTask(..)
                            | Rvalue::AllocateArcStruct(_)
                            | Rvalue::AllocateArenaStruct(..)
                            | Rvalue::AllocateStackStruct(_)
                            | Rvalue::AllocateList(_) => has_arc_instrs = true,
                            Rvalue::MakeStackClosure(..) => has_arc_instrs = true,
                            _ => {}
                        },
                        MirInstr::AssignField { .. } => {}
                    }
                }
            }
        }
        if has_arc_instrs {
            need(Helper::Retain);
            need(Helper::Release);
            need(Helper::Alloc);
            need(Helper::ArcAlloc);
        }
        // One destructor body is synthesized per struct type unconditionally,
        // and it calls Release for managed fields.
        if !self.type_table.definitions.is_empty() {
            need(Helper::Release);
        }

        // Fixpoint: close under helper-internal call dependencies.
        loop {
            let current: Vec<Helper> = helpers.iter().copied().collect();
            let mut grew = false;
            for helper in current {
                for dep in Self::helper_deps(helper) {
                    if helpers.insert(*dep) {
                        grew = true;
                    }
                }
            }
            if !grew {
                break;
            }
        }

        // WASI imports follow from the final helper set.
        for helper in &helpers {
            for wasi in Self::helper_imports(*helper) {
                self.use_import(*wasi);
            }
        }

        // Deterministic emission order.
        let mut sorted: Vec<Helper> = helpers.into_iter().collect();
        sorted.sort();
        self.helpers = sorted;
    }

    /// Assign every function index: imports, user functions, synthesized
    /// destructors/thunks, helpers, `_start`. Also assigns wasm-table seats.
    fn plan_indices(&mut self, scan: &ProgramScan) {
        // WASI imports come first, in sorted order for determinism.
        self.imports.sort();
        for wasi in self.imports.clone() {
            let idx = self.all_sigs.len() as u32;
            let sig = wasi.signature();
            self.register_type(sig.0.clone(), sig.1.clone());
            self.all_sigs.push(sig);
            self.import_index.insert(wasi, idx);
            self.names.push((idx, format!("wasi.{}", wasi.field())));
        }

        // User functions in deterministic MIR id order.
        let mut functions: Vec<&MirFunction> = self.program.functions.values().collect();
        functions.sort_by_key(|f| f.id.0);
        for function in &functions {
            let sig = (user_params(function), user_results(function));
            let idx = self.all_sigs.len() as u32;
            self.register_type(sig.0.clone(), sig.1.clone());
            self.all_sigs.push(sig);
            self.fn_index.insert(function.id, idx);
            self.names.push((idx, function.name.clone()));
        }

        // One destructor per struct type, in type-table order.
        let drop_sig = (vec![Val::I32], vec![]);
        for (i, definition) in self.type_table.definitions.iter().enumerate() {
            let idx = self.all_sigs.len() as u32;
            self.register_type(drop_sig.0.clone(), drop_sig.1.clone());
            self.all_sigs.push(drop_sig.clone());
            self.struct_drop_fn.insert(StructTypeId(i), idx);
            self.names
                .push((idx, format!("__lpp_wasm_drop_{}", definition.name)));
        }

        // Async task thunks.
        self.task_order = scan.task_fns.clone();
        let thunk_sig = (vec![Val::I32], vec![Val::I64]);
        for func_id in &scan.task_fns {
            let idx = self.all_sigs.len() as u32;
            self.register_type(thunk_sig.0.clone(), thunk_sig.1.clone());
            self.all_sigs.push(thunk_sig.clone());
            self.thunk_fn.insert(*func_id, idx);
            self.names.push((idx, format!("__lpp_task_thunk_{}", func_id.0)));
        }

        // Helpers.
        for helper in self.helpers.clone() {
            let sig = helper_signature(helper);
            let idx = self.all_sigs.len() as u32;
            self.register_type(sig.0.clone(), sig.1.clone());
            self.all_sigs.push(sig);
            self.helper_index.insert(helper, idx);
            self.names
                .push((idx, format!("__lpp_wasm_{:?}", helper).to_lowercase()));
        }

        // `_start` last.
        let start_sig = (vec![], vec![]);
        self.start_index = self.all_sigs.len() as u32;
        self.register_type(start_sig.0.clone(), start_sig.1.clone());
        self.all_sigs.push(start_sig);
        self.names.push((self.start_index, "_start".to_string()));

        // ── Table seats ──
        let struct_drops = self.type_table.definitions.len() as u32;
        let mut seat = 1 + struct_drops;
        self.table.struct_drops = struct_drops;
        self.table.tuple_destroy = seat;
        seat += 1;
        self.table.closure_destroy = seat;
        seat += 1;
        self.table.list_destroy = seat;
        seat += 1;
        self.table.map_destroy = seat;
        seat += 1;
        self.table.task_destroy = seat;
        seat += 1;
        self.table.thunks_start = seat;
        seat += scan.task_fns.len() as u32;
        self.table.addressable_start = seat;
        for (i, func_id) in scan.addressable.iter().enumerate() {
            self.table_seat.insert(*func_id, seat + i as u32);
        }
        seat += scan.addressable.len() as u32;
        self.table.total = seat;
    }

    /// The wasm function index occupying a table seat (for the elem segment).
    fn table_seat_function(&self, seat: u32, scan: &ProgramScan) -> u32 {
        if seat == 0 {
            return self.helper_index[&Helper::TrapStub];
        }
        if seat <= self.table.struct_drops {
            return self.struct_drop_fn[&StructTypeId((seat - 1) as usize)];
        }
        if seat == self.table.tuple_destroy {
            return self.helper_index[&Helper::TupleDestroy];
        }
        if seat == self.table.closure_destroy {
            return self.helper_index[&Helper::ClosureDestroy];
        }
        if seat == self.table.list_destroy {
            return self.helper_index[&Helper::ListDestroy];
        }
        if seat == self.table.map_destroy {
            return self.helper_index[&Helper::MapDestroy];
        }
        if seat == self.table.task_destroy {
            return self.helper_index[&Helper::TaskDestroy];
        }
        if seat < self.table.addressable_start {
            let thunk_idx = (seat - self.table.thunks_start) as usize;
            return self.thunk_fn[&scan.task_fns[thunk_idx]];
        }
        let user_idx = (seat - self.table.addressable_start) as usize;
        self.fn_index[&scan.addressable[user_idx]]
    }
}

// ── MIR function lowering ────────────────────────────────────────────────────

/// Per-user-function scratch local slots (after the MIR locals).
#[allow(dead_code)]
const SCR_I32A: usize = 0;
#[allow(dead_code)]
const SCR_I32B: usize = 1;
#[allow(dead_code)]
const SCR_I64: usize = 2;
#[allow(dead_code)]
const SCR_F64: usize = 3;
#[allow(dead_code)]
const SCR_COUNT: usize = 4;

impl<'a> WasmCompiler<'a> {
    /// Compute the emission order of blocks: reverse post-order from the
    /// entry, with unreachable blocks appended so every referenced body
    /// still encodes (nothing can branch to them, but the binary must be
    /// well formed).
    fn block_layout(mir_fn: &MirFunction) -> Vec<BlockId> {
        let by_id: HashMap<BlockId, &MirBlock> =
            mir_fn.blocks.iter().map(|b| (b.id, b)).collect();
        let mut order: Vec<BlockId> = Vec::with_capacity(mir_fn.blocks.len());
        let mut visited: HashSet<BlockId> = HashSet::with_capacity(mir_fn.blocks.len());
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

    fn operand_val(&self, fb: &mut FB, operand: &Operand, local_index: &[u32]) {
        match operand {
            Operand::Local(id) | Operand::Borrowed(id) => {
                fb.g(local_index[id.0]);
            }
            Operand::Int(value) => {
                fb.i64c(*value);
            }
            Operand::Float(value) => {
                fb.f64c(*value);
            }
            Operand::Bool(value) => {
                fb.i32c(if *value { 1 } else { 0 });
            }
            Operand::String(value) => {
                let addr = self.lookup_literal(value);
                fb.i32c(addr as i64);
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

    /// Emit a store of `value` at `[address + offset]`, width from the ABI
    /// class. Stack: `[address, value] → []`.
    fn store_abi(&self, fb: &mut FB, abi: AbiClass, offset: u32) {
        match abi {
            AbiClass::I8 => {
                fb.store8(offset);
            }
            AbiClass::F64 => {
                fb.storef64(offset);
            }
            AbiClass::I64 | AbiClass::Void | AbiClass::VectorI64x2 => {
                fb.store64(offset);
            }
            AbiClass::Pointer => {
                // Pointer slots are 8 bytes wide so native layouts apply
                // unchanged; extend the i32 address into the full slot.
                fb.op(op::I64_EXTEND_I32_U);
                fb.store64(offset);
            }
        }
    }

    /// Emit a load of `[address + offset]`, width from the ABI class.
    /// Stack: `[address] → [value]`.
    fn load_abi(&self, fb: &mut FB, abi: AbiClass, offset: u32) {
        match abi {
            AbiClass::I8 => {
                fb.load8(offset);
            }
            AbiClass::F64 => {
                fb.loadf64(offset);
            }
            AbiClass::I64 | AbiClass::Void | AbiClass::VectorI64x2 => {
                fb.load64(offset);
            }
            AbiClass::Pointer => {
                fb.load64(offset);
                fb.op(op::I32_WRAP_I64);
            }
        }
    }

    /// Emit a comparison producing an i32 0/1.
    fn emit_compare(
        &mut self,
        fb: &mut FB,
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
        self.operand_val(fb, left, local_index);
        self.operand_val(fb, right, local_index);
        let opcode = match (class, operator) {
            (Val::F64, Eq) => op::F64_EQ,
            (Val::F64, NotEq) => op::F64_NE,
            (Val::F64, Less) => op::F64_LT,
            (Val::F64, Greater) => op::F64_GT,
            (Val::F64, LessEq) => op::F64_LE,
            (Val::F64, GreaterEq) => op::F64_GE,
            (Val::I64, Eq) => op::I64_EQ,
            (Val::I64, NotEq) => op::I64_NE,
            (Val::I64, Less) => op::I64_LT_S,
            (Val::I64, Greater) => op::I64_GT_S,
            (Val::I64, LessEq) => op::I64_LE_S,
            (Val::I64, GreaterEq) => op::I64_GE_S,
            (Val::I32, Eq) => op::I32_EQ,
            (Val::I32, NotEq) => op::I32_NE,
            (Val::I32, Less) => op::I32_LT_S,
            (Val::I32, Greater) => op::I32_GT_S,
            (Val::I32, LessEq) => op::I32_LE_S,
            (Val::I32, GreaterEq) => op::I32_GE_S,
            _ => {
                return Err(format!(
                    "WebAssembly backend: operator {:?} is not a comparison",
                    operator
                ));
            }
        };
        fb.op(opcode);
        Ok(())
    }

    /// Emit a binary operation. Leaves exactly one value on the stack.
    fn emit_binary(
        &mut self,
        fb: &mut FB,
        operator: &BinaryOperator,
        left: &Operand,
        right: &Operand,
        local_index: &[u32],
        locals: &[LocalDecl],
    ) -> Result<(), String> {
        use BinaryOperator::*;
        match operator {
            Eq | NotEq | Less | Greater | LessEq | GreaterEq => {
                self.emit_compare(fb, operator, left, right, local_index, locals)?;
            }
            Add | Subtract | Multiply | Divide | Modulo => {
                let class = Self::operand_class(left, locals);
                if class == Val::F64 {
                    match operator {
                        Modulo => {
                            // libm fmod(x, y) == x - trunc(x / y) * y, without
                            // the libm call. Operands are re-emitted; they are
                            // pure (locals/constants) so duplication is safe.
                            self.operand_val(fb, left, local_index);
                            self.operand_val(fb, left, local_index);
                            self.operand_val(fb, right, local_index);
                            fb.op(op::F64_DIV);
                            fb.op(op::F64_TRUNC);
                            self.operand_val(fb, right, local_index);
                            fb.op(op::F64_MUL);
                            fb.op(op::F64_SUB);
                        }
                        _ => {
                            self.operand_val(fb, left, local_index);
                            self.operand_val(fb, right, local_index);
                            fb.op(match operator {
                                Add => op::F64_ADD,
                                Subtract => op::F64_SUB,
                                Multiply => op::F64_MUL,
                                Divide => op::F64_DIV,
                                _ => unreachable!(),
                            });
                        }
                    }
                } else {
                    self.operand_val(fb, left, local_index);
                    self.operand_val(fb, right, local_index);
                    fb.op(match (class, operator) {
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
            Shl => {
                let class = Self::operand_class(left, locals);
                if class == Val::F64 {
                    return Err("WebAssembly backend: bitwise/shift operator Shl on Float is not supported".to_string());
                }
                match class {
                    Val::I64 => {
                        fb.i64c(0);
                        self.operand_val(fb, left, local_index);
                        self.operand_val(fb, right, local_index);
                        fb.op(op::I64_SHL);
                        self.operand_val(fb, right, local_index);
                        fb.i64c(64);
                        fb.op(op::I64_GE_U);
                        fb.op(op::SELECT);
                    }
                    Val::I32 => {
                        fb.i32c(0);
                        self.operand_val(fb, left, local_index);
                        self.operand_val(fb, right, local_index);
                        fb.op(op::I32_SHL);
                        self.operand_val(fb, right, local_index);
                        fb.i32c(32);
                        fb.op(op::I32_GE_U);
                        fb.op(op::SELECT);
                    }
                    _ => unreachable!(),
                }
            }
            Shr => {
                let class = Self::operand_class(left, locals);
                if class == Val::F64 {
                    return Err("WebAssembly backend: bitwise/shift operator Shr on Float is not supported".to_string());
                }
                match class {
                    Val::I64 => {
                        fb.i64c(0);
                        self.operand_val(fb, left, local_index);
                        fb.i64c(63);
                        self.operand_val(fb, right, local_index);
                        self.operand_val(fb, right, local_index);
                        fb.i64c(64);
                        fb.op(op::I64_GE_S);
                        fb.op(op::SELECT);
                        fb.op(op::I64_SHR_S);
                        self.operand_val(fb, right, local_index);
                        fb.i64c(0);
                        fb.op(op::I64_LT_S);
                        fb.op(op::SELECT);
                    }
                    Val::I32 => {
                        fb.i32c(0);
                        self.operand_val(fb, left, local_index);
                        fb.i32c(31);
                        self.operand_val(fb, right, local_index);
                        self.operand_val(fb, right, local_index);
                        fb.i32c(32);
                        fb.op(op::I32_GE_S);
                        fb.op(op::SELECT);
                        fb.op(op::I32_SHR_S);
                        self.operand_val(fb, right, local_index);
                        fb.i32c(0);
                        fb.op(op::I32_LT_S);
                        fb.op(op::SELECT);
                    }
                    _ => unreachable!(),
                }
            }
            And | Or | BitAnd | BitOr | BitXor => {
                let class = Self::operand_class(left, locals);
                if class == Val::F64 {
                    return Err(format!(
                        "WebAssembly backend: bitwise operator {:?} on Float is not supported",
                        operator
                    ));
                }
                self.operand_val(fb, left, local_index);
                self.operand_val(fb, right, local_index);
                fb.op(match (class, operator) {
                    (Val::I64, And) | (Val::I64, BitAnd) => op::I64_AND,
                    (Val::I64, Or) | (Val::I64, BitOr) => op::I64_OR,
                    (Val::I64, BitXor) => op::I64_XOR,
                    (Val::I32, And) | (Val::I32, BitAnd) => op::I32_AND,
                    (Val::I32, Or) | (Val::I32, BitOr) => op::I32_OR,
                    (Val::I32, BitXor) => op::I32_XOR,
                    _ => unreachable!(),
                });
            }
        }
        Ok(())
    }
}

impl<'a> WasmCompiler<'a> {
    /// Scratch local helper: the absolute wasm index of a per-function
    /// scratch slot.
    fn scr(local_count: usize, slot: usize) -> u32 {
        (local_count + slot) as u32
    }

    /// Wrap a freshly produced i64 in the class the destination expects.
    /// Used for `Await` and `SliceGet`, whose runtime payload is an i64 cell.
    fn convert_raw_to(&self, fb: &mut FB, ty: &TypeRef) {
        match val_of_type(ty) {
            Val::I64 => {}
            Val::F64 => {
                fb.op(op::F64_REINTERPRET_I64);
            }
            Val::I32 => {
                fb.op(op::I32_WRAP_I64);
            }
        }
    }

    /// Coerce a helper result of class `have` into the destination local's
    /// wasm class: pointer/predicate i32 widens into an Int slot and an Int
    /// i64 narrows into a Bool/pointer slot (the same bit reinterpretation
    /// the native 64-bit ABI performs).
    fn coerce_builtin_result(&self, fb: &mut FB, have: Val, dest_ty: &TypeRef) {
        let want = val_of_type(dest_ty);
        match (have, want) {
            (Val::I64, Val::I32) => {
                fb.op(op::I32_WRAP_I64);
            }
            (Val::I32, Val::I64) => {
                fb.op(op::I64_EXTEND_I32_U);
            }
            _ => {}
        }
    }

    /// Convert the i64 list-slot payload to a typed builtin's expected
    /// argument class, in place on the stack.
    fn convert_to_slot(&self, fb: &mut FB, from: Val) {
        match from {
            Val::I64 => {}
            Val::F64 => {
                fb.op(op::I64_REINTERPRET_F64);
            }
            Val::I32 => {
                fb.op(op::I64_EXTEND_I32_U);
            }
        }
    }

    /// Emit one rvalue, leaving exactly one value on the stack. When a call
    /// has no result, a placeholder i64 zero is produced (matching the other
    /// backends, which assign `iconst 0` into Void temporaries).
    #[allow(clippy::too_many_arguments)]
    fn emit_rvalue(
        &mut self,
        fb: &mut FB,
        rvalue: &Rvalue,
        local_index: &[u32],
        mir_fn: &MirFunction,
        dest: LocalId,
    ) -> Result<(), String> {
        let locals = &mir_fn.locals;
        let dest_ty = locals[dest.0].ty.clone();
        let scr_base = locals.len();
        match rvalue {
            Rvalue::Use(operand) => self.operand_val(fb, operand, local_index),
            Rvalue::Move(local) => {
                self.operand_val(fb, &Operand::Local(*local), local_index)
            }
            Rvalue::BinaryOp(operator, left, right) => {
                self.emit_binary(fb, operator, left, right, local_index, locals)?
            }
            Rvalue::CallDirect(target, args) => {
                let target_fn = &self.program.functions[target];
                for arg in args {
                    self.operand_val(fb, arg, local_index);
                }
                fb.call(self.fn_index[target]);
                if target_fn.return_type == TypeRef::Void {
                    fb.i64c(0);
                }
            }
            Rvalue::CallIndirect(callee, args) => {
                self.emit_call_indirect(fb, callee, args, local_index, mir_fn, &dest_ty)?;
            }
            Rvalue::FuncRef(mir_func_id) => {
                let seat = *self.table_seat.get(mir_func_id).ok_or_else(|| {
                    format!(
                        "WebAssembly backend: FuncRef of fn_{} has no table seat",
                        mir_func_id.0
                    )
                })?;
                fb.i64c(seat as i64);
            }
            Rvalue::AllocateTuple(types, values) => {
                let (layout, total_size) = tuple_layout(types);
                let (managed_mask, packed_offsets) = tuple_runtime_metadata(types);
                fb.i32c(total_size as i64);
                fb.i64c(managed_mask as i64);
                fb.i64c(packed_offsets as i64);
                fb.call(self.helper_index[&Helper::TupleAlloc]);
                fb.s(Self::scr(scr_base, SCR_I32A));
                for ((value, field), ty) in
                    values.iter().zip(layout.iter()).zip(types.iter())
                {
                    fb.g(Self::scr(scr_base, SCR_I32A));
                    self.operand_val(fb, value, local_index);
                    self.store_abi(fb, field.abi, field.offset as u32);
                    let _ = ty;
                }
                fb.g(Self::scr(scr_base, SCR_I32A));
            }
            Rvalue::TupleField(base, index) => {
                let base_id = match base {
                    Operand::Local(id) | Operand::Borrowed(id) => *id,
                    _ => return Err("tuple field base must be a local".to_string()),
                };
                let types = match &locals[base_id.0].ty {
                    TypeRef::Tuple(types) => types.clone(),
                    other => {
                        return Err(format!("tuple field base has type {:?}", other));
                    }
                };
                let (layout, _) = tuple_layout(&types);
                let field = layout
                    .get(*index)
                    .ok_or_else(|| format!("tuple field {} out of range", index))?;
                self.operand_val(fb, base, local_index);
                self.load_abi(fb, field.abi, field.offset as u32);
            }
            Rvalue::MakeSlice {
                base,
                start,
                length,
                kind,
            } => {
                self.operand_val(fb, base, local_index);
                self.operand_val(fb, start, local_index);
                self.operand_val(fb, length, local_index);
                fb.i64c(*kind as i64);
                fb.call(self.helper_index[&Helper::SliceNew]);
            }
            Rvalue::SliceLen(view) => {
                self.operand_val(fb, view, local_index);
                fb.call(self.helper_index[&Helper::SliceLen]);
            }
            Rvalue::SliceGet(view, index) => {
                let view_id = match view {
                    Operand::Local(id) | Operand::Borrowed(id) => *id,
                    _ => return Err("slice_get view must be a local".to_string()),
                };
                if locals[view_id.0].ty == TypeRef::StrSlice {
                    self.operand_val(fb, view, local_index);
                    self.operand_val(fb, index, local_index);
                    fb.call(self.helper_index[&Helper::StrSliceGet]);
                } else {
                    self.operand_val(fb, view, local_index);
                    self.operand_val(fb, index, local_index);
                    fb.call(self.helper_index[&Helper::SliceGet]);
                    self.convert_raw_to(fb, &dest_ty);
                }
            }
            Rvalue::SliceToStr(view) => {
                self.operand_val(fb, view, local_index);
                fb.call(self.helper_index[&Helper::StrSliceToStr]);
            }
            Rvalue::MakeTask(function_id, argument_types, arguments, result_type) => {
                let (layout, total_size) = tuple_layout(argument_types);
                let (managed_mask, packed_offsets) = tuple_runtime_metadata(argument_types);
                fb.i32c(total_size as i64);
                fb.i64c(managed_mask as i64);
                fb.i64c(packed_offsets as i64);
                fb.call(self.helper_index[&Helper::TupleAlloc]);
                fb.s(Self::scr(scr_base, SCR_I32A));
                for (argument, field) in arguments.iter().zip(layout.iter()) {
                    fb.g(Self::scr(scr_base, SCR_I32A));
                    self.operand_val(fb, argument, local_index);
                    self.store_abi(fb, field.abi, field.offset as u32);
                }
                let thunk_pos = self
                    .task_order
                    .iter()
                    .position(|id| id == function_id)
                    .ok_or_else(|| {
                        format!("missing task thunk for fn_{}", function_id.0)
                    })?;
                let seat = self.table.thunks_start + thunk_pos as u32;
                fb.i64c(seat as i64);
                fb.g(Self::scr(scr_base, SCR_I32A));
                fb.i64c(result_type.is_managed() as i64);
                fb.call(self.helper_index[&Helper::TaskNew]);
            }
            Rvalue::Await(task) => {
                self.operand_val(fb, task, local_index);
                fb.call(self.helper_index[&Helper::TaskAwait]);
                self.convert_raw_to(fb, &dest_ty);
            }
            Rvalue::FieldAccess(Operand::Local(base_id), field)
            | Rvalue::FieldAccess(Operand::Borrowed(base_id), field) => {
                let base_ty = &locals[base_id.0].ty;
                let TypeRef::Custom(struct_id) = base_ty else {
                    return Err(format!(
                        "Cannot read field '{}' on non-struct MIR local {:?}",
                        field, base_id
                    ));
                };
                let struct_def = &self.type_table.definitions[struct_id.0];
                let Some(field_index) = struct_def
                    .fields
                    .iter()
                    .position(|(name, _)| name == field)
                else {
                    return Err(format!(
                        "Field '{}' not found while lowering struct '{}'",
                        field, struct_def.name
                    ));
                };
                let (layout, _) = struct_layout(self.type_table, *struct_id);
                let field_layout = layout[field_index];
                fb.g(local_index[base_id.0]);
                self.load_abi(fb, field_layout.abi, field_layout.offset as u32);
            }
            Rvalue::FieldAccess(_, _) => {
                return Err("WebAssembly backend: struct field base must be a local".to_string());
            }
            Rvalue::AllocateArcStruct(TypeRef::Custom(struct_id))
            | Rvalue::AllocateArenaStruct(TypeRef::Custom(struct_id), _) => {
                // WebAssembly arena note: arena nodes are ordinary ARC objects
                // here. The native region reclaim is a memory-reuse
                // optimization; under the bump heap nothing is recycled, so
                // semantics are preserved while weak-field cycle breaking
                // behaves identically.
                let (_, layout_size) = struct_layout(self.type_table, *struct_id);
                fb.i32c(layout_size.max(1) as i64);
                fb.i32c((1 + struct_id.0) as i64); // destructor table seat
                fb.call(self.helper_index[&Helper::ArcAlloc]);
            }
            Rvalue::AllocateArenaStruct(other, _) => {
                return Err(format!(
                    "arena allocation requires a resolved custom struct type, got {:?}",
                    other
                ));
            }
            Rvalue::AllocateArcStruct(other) => {
                return Err(format!(
                    "AllocateArcStruct requires a resolved custom struct type, got {:?}",
                    other
                ));
            }
            Rvalue::AllocateStackStruct(TypeRef::Custom(struct_id)) => {
                let (_, layout_size) = struct_layout(self.type_table, *struct_id);
                fb.i32c(layout_size.max(1) as i64);
                fb.call(self.helper_index[&Helper::Alloc]);
            }
            Rvalue::AllocateStackStruct(other) => {
                return Err(format!(
                    "stack struct allocation requires a custom struct type, got {:?}",
                    other
                ));
            }
            Rvalue::AllocateStruct(_) => {
                return Err(
                    "raw struct allocation reached the wasm backend (rejected like the native AOT)"
                        .to_string(),
                );
            }
            Rvalue::AllocateList(element_ty) => {
                let is_arc = match element_ty.list_element_class() {
                    ListElementClass::Scalar
                    | ListElementClass::Bool
                    | ListElementClass::Float => 0,
                    ListElementClass::Arc => 1,
                    ListElementClass::Unsupported => {
                        return Err(format!(
                            "WebAssembly backend does not support List[{:?}] safely",
                            element_ty
                        ));
                    }
                };
                fb.i32c(is_arc);
                fb.call(self.helper_index[&Helper::ListNew]);
            }
            rv @ (Rvalue::MakeClosure(mir_func_id, args)
            | Rvalue::MakeStackClosure(mir_func_id, args)) => {
                let stack_closure = matches!(rv, Rvalue::MakeStackClosure(_, _));
                let seat = *self.table_seat.get(mir_func_id).ok_or_else(|| {
                    format!("closure target fn_{} has no table seat", mir_func_id.0)
                })?;
                fb.i32c(16);
                if stack_closure {
                    fb.call(self.helper_index[&Helper::Alloc]);
                } else {
                    fb.i32c(self.table.closure_destroy as i64);
                    fb.call(self.helper_index[&Helper::ArcAlloc]);
                }
                fb.s(Self::scr(scr_base, SCR_I32A));
                // word 0: function table seat; word 1: environment pointer.
                fb.g(Self::scr(scr_base, SCR_I32A));
                fb.i64c(seat as i64);
                fb.store64(0);
                let env_operand = args.first().ok_or_else(|| {
                    "internal error: closure construction is missing its environment".to_string()
                })?;
                fb.g(Self::scr(scr_base, SCR_I32A));
                if Self::operand_class(env_operand, locals) == Val::I32 {
                    self.operand_val(fb, env_operand, local_index);
                    fb.op(op::I64_EXTEND_I32_U);
                } else {
                    self.operand_val(fb, env_operand, local_index);
                }
                fb.store64(8);
                fb.g(Self::scr(scr_base, SCR_I32A));
            }
            Rvalue::SpawnThread(_) => {
                return Err(
                    "WebAssembly backend does not support OS threads ('spawn')".to_string(),
                );
            }
            Rvalue::BuiltinCall(symbol, args) => {
                self.emit_builtin(fb, symbol, args, local_index, mir_fn, &dest_ty)?;
            }
        }
        Ok(())
    }

    /// Emit `call_indirect` for a closure capsule or a raw function seat.
    #[allow(clippy::too_many_arguments)]
    fn emit_call_indirect(
        &mut self,
        fb: &mut FB,
        callee: &Operand,
        args: &[Operand],
        local_index: &[u32],
        mir_fn: &MirFunction,
        dest_ty: &TypeRef,
    ) -> Result<(), String> {
        let locals = &mir_fn.locals;
        let scr_base = locals.len();
        let is_direct_seat = match callee {
            Operand::Local(id) | Operand::Borrowed(id) => {
                // Only Int locals are raw table seats (from FuncRef/trait
                // vtables). Function-typed locals are closure capsules.
                matches!(locals[id.0].ty, TypeRef::Int)
            }
            _ => false,
        };
        let mut params: Vec<Val> = Vec::with_capacity(args.len() + 1);
        if !is_direct_seat {
            params.push(Val::I32); // closure environment
        }
        for arg in args {
            params.push(Self::operand_class(arg, locals));
        }
        let results = if *dest_ty == TypeRef::Void || matches!(dest_ty, TypeRef::Tuple(elems) if elems.is_empty()) {
            vec![]
        } else {
            vec![val_of_type(dest_ty)]
        };
        let type_index = self.register_type(params, results);
        if is_direct_seat {
            for arg in args {
                self.operand_val(fb, arg, local_index);
            }
            self.operand_val(fb, callee, local_index);
            fb.op(op::I32_WRAP_I64);
            fb.call_indirect(type_index);
        } else {
            self.operand_val(fb, callee, local_index);
            fb.s(Self::scr(scr_base, SCR_I32A));
            // env pointer first.
            fb.g(Self::scr(scr_base, SCR_I32A));
            fb.load64(8);
            fb.op(op::I32_WRAP_I64);
            for arg in args {
                self.operand_val(fb, arg, local_index);
            }
            fb.g(Self::scr(scr_base, SCR_I32A));
            fb.load64(0);
            fb.op(op::I32_WRAP_I64);
            fb.call_indirect(type_index);
        }
        if *dest_ty == TypeRef::Void {
            fb.i64c(0);
        }
        Ok(())
    }
}

impl<'a> WasmCompiler<'a> {
    /// Emit a builtin call. Leaves exactly one value on the stack.
    fn emit_builtin(
        &mut self,
        fb: &mut FB,
        symbol: &str,
        args: &[Operand],
        local_index: &[u32],
        mir_fn: &MirFunction,
        dest_ty: &TypeRef,
    ) -> Result<(), String> {
        let locals = &mir_fn.locals;
        let arg_vals = |this: &mut Self, fb: &mut FB, n: usize| {
            for arg in args.iter().take(n) {
                this.operand_val(fb, arg, local_index);
            }
        };
        /// Most builtins return Void like print; produce the placeholder.
        macro_rules! void_call {
            ($helper:expr, $n:expr) => {{
                arg_vals(self, fb, $n);
                fb.call(self.helper_index[&$helper]);
                fb.i64c(0);
            }};
        }
        macro_rules! value_call {
            ($helper:expr, $n:expr) => {{
                arg_vals(self, fb, $n);
                fb.call(self.helper_index[&$helper]);
            }};
        }
        match symbol {
            "lpp_print_int" => void_call!(Helper::PrintInt, 1),
            "lpp_print_bool" => void_call!(Helper::PrintBool, 1),
            "lpp_print_float" => void_call!(Helper::PrintFloat, 1),
            "lpp_print_str" => void_call!(Helper::PrintStr, 1),
            "lpp_str_len" => value_call!(Helper::StrLen, 1),
            "lpp_str_eq" => {
                value_call!(Helper::StrEq, 2);
                self.coerce_builtin_result(fb, Val::I32, dest_ty);
            }
            "fmod" => {
                if args.len() != 2 {
                    return Err("fmod expects exactly two arguments".to_string());
                }
                self.operand_val(fb, &args[0], local_index);
                self.operand_val(fb, &args[0], local_index);
                self.operand_val(fb, &args[1], local_index);
                fb.op(op::F64_DIV);
                fb.op(op::F64_TRUNC);
                self.operand_val(fb, &args[1], local_index);
                fb.op(op::F64_MUL);
                fb.op(op::F64_SUB);
            }
            "lpp_str_concat" => value_call!(Helper::StrConcat, 2),
            "lpp_str_substr" => value_call!(Helper::StrSubstr, 3),
            "lpp_str_trim" => value_call!(Helper::StrTrim, 1),
            "lpp_str_upper" => value_call!(Helper::StrUpper, 1),
            "lpp_str_lower" => value_call!(Helper::StrLower, 1),
            "lpp_int_to_str" => value_call!(Helper::IntToStr, 1),
            "lpp_float_to_str" => value_call!(Helper::FloatToStr, 1),
            "lpp_bool_to_str" => value_call!(Helper::BoolToStr, 1),
            "lpp_str_to_int" | "lpp_parse_int" => value_call!(Helper::StrToInt, 1),
            "lpp_char_at" => value_call!(Helper::CharAt, 2),
            "lpp_ord" => value_call!(Helper::Ord, 1),
            "lpp_chr" => value_call!(Helper::Chr, 1),
            "lpp_str_find" => value_call!(Helper::StrFind, 2),
            "lpp_str_contains" => {
                value_call!(Helper::StrContains, 2);
                self.coerce_builtin_result(fb, Val::I32, dest_ty);
            }
            "lpp_str_starts_with" => {
                value_call!(Helper::StrStartsWith, 2);
                self.coerce_builtin_result(fb, Val::I32, dest_ty);
            }
            "lpp_str_ends_with" => {
                value_call!(Helper::StrEndsWith, 2);
                self.coerce_builtin_result(fb, Val::I32, dest_ty);
            }
            "lpp_str_repeat" => value_call!(Helper::StrRepeat, 2),
            "lpp_str_replace" => value_call!(Helper::StrReplace, 3),
            "lpp_str_split" => {
                // Dest is the List[Str] pointer for the `split` builtin, or
                // an Int slot when the raw runtime symbol is used directly.
                value_call!(Helper::StrSplit, 2);
                self.coerce_builtin_result(fb, Val::I32, dest_ty);
            }
            "lpp_input" => value_call!(Helper::Input, 0),
            "lpp_exit" => void_call!(Helper::Exit, 1),
            "lpp_env_get" => value_call!(Helper::EnvGet, 1),
            "lpp_abs" => {
                // abs(x) = x < 0 ? 0 - x : x (plays for Int). `select` keeps
                // the FIRST arm when the condition fires, so (0 - x) goes on
                // the stack before x.
                fb.i64c(0);
                self.operand_val(fb, &args[0], local_index);
                fb.op(op::I64_SUB);
                self.operand_val(fb, &args[0], local_index);
                self.operand_val(fb, &args[0], local_index);
                fb.i64c(0);
                fb.op(op::I64_LT_S);
                fb.op(op::SELECT);
            }
            "lpp_min" | "lpp_max" => {
                self.operand_val(fb, &args[0], local_index);
                self.operand_val(fb, &args[1], local_index);
                self.operand_val(fb, &args[0], local_index);
                self.operand_val(fb, &args[1], local_index);
                fb.op(if symbol == "lpp_min" {
                    op::I64_LT_S
                } else {
                    op::I64_GT_S
                });
                fb.op(op::SELECT);
            }
            "lpp_int_pow" => value_call!(Helper::IntPow, 2),
            "lpp_pow" => value_call!(Helper::Pow, 2),
            "lpp_floor" => {
                arg_vals(self, fb, 1);
                fb.op(op::F64_FLOOR);
            }
            "lpp_ceil" => {
                arg_vals(self, fb, 1);
                fb.op(op::F64_CEIL);
            }
            "lpp_sqrt" => {
                arg_vals(self, fb, 1);
                fb.op(op::F64_SQRT);
            }
            "lpp_sin" => {
                arg_vals(self, fb, 1);
                fb.i32c(0);
                fb.call(self.helper_index[&Helper::Trig]);
            }
            "lpp_cos" => {
                arg_vals(self, fb, 1);
                fb.i32c(1);
                fb.call(self.helper_index[&Helper::Trig]);
            }
            "lpp_tan" => {
                arg_vals(self, fb, 1);
                fb.i32c(2);
                fb.call(self.helper_index[&Helper::Trig]);
            }
            "lpp_int_to_float" => {
                arg_vals(self, fb, 1);
                fb.op(op::F64_CONVERT_I64_S);
            }
            "lpp_float_to_int" => {
                arg_vals(self, fb, 1);
                fb.op(op::I64_TRUNC_F64_S);
            }
            "lpp_random_seed" => {
                arg_vals(self, fb, 1);
                fb.gset(GLOBAL_RNG);
                fb.i64c(0);
            }
            "lpp_random" => value_call!(Helper::Random, 0),
            "lpp_random_range" => value_call!(Helper::RandomRange, 2),
            "lpp_time_ms" => value_call!(Helper::TimeMs, 0),
            "lpp_sleep_ms" => void_call!(Helper::SleepMs, 1),
            // ── Lists ──
            "lpp_list_new" | "lpp_list_new_arc" => {
                // The symbol alone is not enough for non-MIR emission; the
                // _arc suffix decides the ownership mode.
                fb.i32c(if symbol == "lpp_list_new_arc" { 1 } else { 0 });
                fb.call(self.helper_index[&Helper::ListNew]);
            }
            "lpp_list_push" => void_call!(Helper::ListPush, 2),
            "lpp_list_push_arc" => {
                arg_vals(self, fb, 1);
                self.operand_val(fb, &args[1], local_index);
                fb.op(op::I64_EXTEND_I32_U);
                fb.call(self.helper_index[&Helper::ListPush]);
                fb.i64c(0);
            }
            "lpp_list_push_bool" => {
                arg_vals(self, fb, 1);
                self.operand_val(fb, &args[1], local_index);
                fb.op(op::I64_EXTEND_I32_U);
                fb.call(self.helper_index[&Helper::ListPush]);
                fb.i64c(0);
            }
            "lpp_list_push_float" => {
                arg_vals(self, fb, 1);
                self.operand_val(fb, &args[1], local_index);
                fb.op(op::I64_REINTERPRET_F64);
                fb.call(self.helper_index[&Helper::ListPush]);
                fb.i64c(0);
            }
            "lpp_list_get" => value_call!(Helper::ListGet, 2),
            "lpp_list_get_arc" => {
                value_call!(Helper::ListGet, 2);
                fb.op(op::I32_WRAP_I64);
            }
            "lpp_list_get_bool" => {
                value_call!(Helper::ListGet, 2);
                fb.i64c(0);
                fb.op(op::I64_NE);
            }
            "lpp_list_get_float" => {
                value_call!(Helper::ListGet, 2);
                fb.op(op::F64_REINTERPRET_I64);
            }
            "lpp_list_set" => void_call!(Helper::ListSet, 3),
            "lpp_list_set_arc" | "lpp_list_set_bool" => {
                arg_vals(self, fb, 2);
                self.operand_val(fb, &args[2], local_index);
                fb.op(op::I64_EXTEND_I32_U);
                fb.call(self.helper_index[&Helper::ListSet]);
                fb.i64c(0);
            }
            "lpp_list_set_float" => {
                arg_vals(self, fb, 2);
                self.operand_val(fb, &args[2], local_index);
                fb.op(op::I64_REINTERPRET_F64);
                fb.call(self.helper_index[&Helper::ListSet]);
                fb.i64c(0);
            }
            "lpp_list_len" => value_call!(Helper::ListLen, 1),
            "lpp_list_free" => {
                // Single reference release, never a raw free.
                arg_vals(self, fb, 1);
                fb.call(self.helper_index[&Helper::Release]);
                fb.i64c(0);
            }
            // ── Maps ──
            "lpp_map_new" | "lpp_map_new_arc" => {
                fb.i32c(if symbol == "lpp_map_new_arc" { 1 } else { 0 });
                fb.call(self.helper_index[&Helper::MapNew]);
            }
            "lpp_map_put" | "lpp_map_put_str" => {
                arg_vals(self, fb, 1);
                self.operand_val(fb, &args[1], local_index);
                self.convert_to_slot(fb, Self::operand_class(&args[1], locals));
                self.operand_val(fb, &args[2], local_index);
                self.convert_to_slot(fb, Self::operand_class(&args[2], locals));
                fb.i64c(if symbol == "lpp_map_put_str" { 1 } else { 0 });
                fb.call(self.helper_index[&Helper::MapPut]);
                fb.i64c(0);
            }
            "lpp_map_put_float" | "lpp_map_put_str_float" => {
                arg_vals(self, fb, 1);
                self.operand_val(fb, &args[1], local_index);
                self.convert_to_slot(fb, Self::operand_class(&args[1], locals));
                self.operand_val(fb, &args[2], local_index);
                fb.op(op::I64_REINTERPRET_F64);
                fb.i64c(if symbol == "lpp_map_put_str_float" { 1 } else { 0 });
                fb.call(self.helper_index[&Helper::MapPut]);
                fb.i64c(0);
            }
            "lpp_map_get" | "lpp_map_get_str" => {
                arg_vals(self, fb, 1);
                self.operand_val(fb, &args[1], local_index);
                self.convert_to_slot(fb, Self::operand_class(&args[1], locals));
                fb.i64c(if symbol == "lpp_map_get_str" { 1 } else { 0 });
                fb.call(self.helper_index[&Helper::MapGet]);
                if val_of_type(dest_ty) == Val::I32 {
                    // Map[_, Str-or-struct]: callers expect a pointer class.
                    fb.op(op::I32_WRAP_I64);
                }
            }
            "lpp_map_get_float" | "lpp_map_get_str_float" => {
                arg_vals(self, fb, 1);
                self.operand_val(fb, &args[1], local_index);
                self.convert_to_slot(fb, Self::operand_class(&args[1], locals));
                fb.i64c(if symbol == "lpp_map_get_str_float" { 1 } else { 0 });
                fb.call(self.helper_index[&Helper::MapGet]);
                fb.op(op::F64_REINTERPRET_I64);
            }
            "lpp_map_has" | "lpp_map_has_str" => {
                arg_vals(self, fb, 1);
                self.operand_val(fb, &args[1], local_index);
                self.convert_to_slot(fb, Self::operand_class(&args[1], locals));
                fb.i64c(if symbol == "lpp_map_has_str" { 1 } else { 0 });
                fb.call(self.helper_index[&Helper::MapHas]);
                // `map_has` lowers to a Bool destination while the raw
                // `lpp_map_has` runtime symbol is typed Int; MapHas yields
                // an i64 0/1 so both classes are served by one coerce.
                self.coerce_builtin_result(fb, Val::I64, dest_ty);
            }
            "lpp_map_remove" | "lpp_map_remove_str" => {
                arg_vals(self, fb, 1);
                self.operand_val(fb, &args[1], local_index);
                self.convert_to_slot(fb, Self::operand_class(&args[1], locals));
                fb.i64c(if symbol == "lpp_map_remove_str" { 1 } else { 0 });
                fb.call(self.helper_index[&Helper::MapRemove]);
                fb.i64c(0);
            }
            "lpp_map_len" => value_call!(Helper::MapLen, 1),
            // ── ARC & arena ──
            "lpp_arc_retain" | "lpp_arc_retain_local" | "lpp_arena_retain" => {
                arg_vals(self, fb, 1);
                fb.call(self.helper_index[&Helper::Retain]);
                fb.i64c(0);
            }
            "lpp_arc_release"
            | "lpp_arc_release_local"
            | "lpp_arena_release_node"
            | "lpp_free_str"
            | "lpp_task_destroy" => {
                arg_vals(self, fb, 1);
                fb.call(self.helper_index[&Helper::Release]);
                fb.i64c(0);
            }
            "lpp_arena_begin" => {
                // Arena regions are a native allocator optimization; under
                // the wasm bump heap the token is a harmless placeholder.
                fb.i64c(1);
            }
            "lpp_arena_release" => {
                arg_vals(self, fb, 1);
                fb.op(op::DROP);
                fb.i64c(0);
            }
            "lpp_alloc" => {
                arg_vals(self, fb, 1);
                fb.op(op::I32_WRAP_I64);
                fb.call(self.helper_index[&Helper::Alloc]);
                // `alloc` is typed to return Int; widen the i32 pointer.
                self.coerce_builtin_result(fb, Val::I32, dest_ty);
            }
            "lpp_free" => {
                arg_vals(self, fb, 1);
                fb.op(op::DROP);
                fb.i64c(0);
            }
            "lpp_task_await" => {
                arg_vals(self, fb, 1);
                fb.call(self.helper_index[&Helper::TaskAwait]);
                self.convert_raw_to(fb, dest_ty);
            }
            "lpp_task_poll" => value_call!(Helper::TaskPoll, 1),
            "lpp_executor_run" => {
                arg_vals(self, fb, 1);
                fb.call(self.helper_index[&Helper::TaskRun]);
            }
            "lpp_slice_len" => value_call!(Helper::SliceLen, 1),
            "lpp_slice_get" => {
                value_call!(Helper::SliceGet, 2);
                self.convert_raw_to(fb, dest_ty);
            }
            "lpp_slice_get_bool" => {
                value_call!(Helper::SliceGet, 2);
                fb.i64c(0);
                fb.op(op::I64_NE);
            }
            "lpp_slice_get_float" => {
                value_call!(Helper::SliceGet, 2);
                fb.op(op::F64_REINTERPRET_I64);
            }
            "lpp_str_slice_get" => value_call!(Helper::StrSliceGet, 2),
            "lpp_str_slice_to_str" => value_call!(Helper::StrSliceToStr, 1),
            other => {
                return Err(format!(
                    "WebAssembly backend does not provide builtin '{}'",
                    other
                ));
            }
        }
        Ok(())
    }
}

impl<'a> WasmCompiler<'a> {
    /// Emit a branch from body position `from` to layout position `target`.
    ///
    /// `nest` counts extra structured constructs the branch sits inside
    /// (e.g. 1 when emitted inside an `if` arm) — each one shifts every
    /// label depth by one, because the enclosing `if` is itself a label.
    fn emit_branch(
        fb: &mut FB,
        from: usize,
        target: usize,
        total: usize,
        disp: u32,
        nest: usize,
    ) {
        if target > from {
            fb.br((target - from - 1 + nest) as u32);
        } else {
            fb.i32c(target as i64);
            fb.s(disp);
            fb.br((total - 1 - from + nest) as u32);
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
                match instr {
                    MirInstr::Assign(_, rvalue) => self.preintern_rvalue(rvalue),
                    MirInstr::AssignField { value, .. } => self.preintern_operand(value),
                    MirInstr::Retain(_) | MirInstr::Release(_) => {}
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

    fn lower_instr(
        &mut self,
        fb: &mut FB,
        instr: &MirInstr,
        local_index: &[u32],
        mir_fn: &MirFunction,
    ) -> Result<(), String> {
        let locals = &mir_fn.locals;
        match instr {
            MirInstr::Assign(dest, rvalue) => {
                self.emit_rvalue(fb, rvalue, local_index, mir_fn, *dest)?;
                fb.s(local_index[dest.0]);
            }
            MirInstr::AssignField { base, field, value } => {
                let base_ty = &locals[base.0].ty;
                let TypeRef::Custom(struct_id) = base_ty else {
                    return Err(format!(
                        "Cannot assign field '{}' on non-struct MIR local {:?}",
                        field, base
                    ));
                };
                let struct_def = &self.type_table.definitions[struct_id.0];
                let Some(field_index) = struct_def
                    .fields
                    .iter()
                    .position(|(name, _)| name == field)
                else {
                    return Err(format!(
                        "Field '{}' not found while lowering struct '{}'",
                        field, struct_def.name
                    ));
                };
                let (layout, _) = struct_layout(self.type_table, *struct_id);
                let field_layout = layout[field_index];
                fb.g(local_index[base.0]);
                self.operand_val(fb, value, local_index);
                self.store_abi(fb, field_layout.abi, field_layout.offset as u32);
            }
            MirInstr::Retain(local) => {
                // A stack payload has no ARC header and must never reach this
                // path, exactly like the native backend (cranelift/lower.rs).
                if locals[local.0].ownership.is_copy() {
                    return Err(format!("attempted to retain stack local {:?}", local));
                }
                fb.g(local_index[local.0]);
                fb.call(self.helper_index[&Helper::Retain]);
            }
            MirInstr::Release(local) => {
                // `pass_arc` also uses Release as the lifetime-end opcode for
                // a promoted stack struct / stack closure capsule. Those carry
                // no ARC header, so the destructor is invoked directly —
                // exactly like the native backend.
                if locals[local.0].ownership.is_copy() {
                    match &locals[local.0].ty {
                        TypeRef::Custom(struct_id) => {
                            fb.g(local_index[local.0]);
                            let drop_fn = *self.struct_drop_fn.get(struct_id).ok_or_else(|| {
                                format!("missing stack destructor for struct {:?}", struct_id)
                            })?;
                            fb.call(drop_fn);
                            return Ok(());
                        }
                        TypeRef::Function => {
                            fb.g(local_index[local.0]);
                            fb.call(self.helper_index[&Helper::ClosureDestroy]);
                            return Ok(());
                        }
                        _ => {}
                    }
                }
                fb.g(local_index[local.0]);
                fb.call(self.helper_index[&Helper::Release]);
            }
        }
        Ok(())
    }

    /// wasm `if` consumes i32; Bool already is i32-canonical 0/1, Int needs a
    /// nonzero test, and Float is rejected by typecheck but mapped defensively.
    fn coerce_to_i32(&self, fb: &mut FB, class: Val) {
        match class {
            Val::I32 => {}
            Val::I64 => {
                fb.op(op::I64_EQZ);
                fb.op(op::I32_EQZ);
            }
            Val::F64 => {
                fb.f64c(0.0);
                fb.op(op::F64_NE);
            }
        }
    }

    fn lower_terminator(
        &mut self,
        fb: &mut FB,
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
                Self::emit_branch(fb, from, position[target], total, disp, 0);
            }
            Terminator::If {
                cond,
                then_block,
                else_block,
            } => {
                let class = Self::operand_class(cond, &mir_fn.locals);
                self.operand_val(fb, cond, local_index);
                self.coerce_to_i32(fb, class);
                fb.if_();
                Self::emit_branch(fb, from, position[then_block], total, disp, 1);
                fb.else_();
                Self::emit_branch(fb, from, position[else_block], total, disp, 1);
                fb.end();
            }
            Terminator::IfCmp {
                op: operator,
                left,
                right,
                then_block,
                else_block,
            } => {
                self.emit_compare(fb, operator, left, right, local_index, &mir_fn.locals)?;
                fb.if_();
                Self::emit_branch(fb, from, position[then_block], total, disp, 1);
                fb.else_();
                Self::emit_branch(fb, from, position[else_block], total, disp, 1);
                fb.end();
            }
            Terminator::Return(Some(operand)) | Terminator::ReturnOwned(operand) => {
                self.operand_val(fb, operand, local_index);
                fb.op(op::RETURN);
            }
            Terminator::Return(None) => {
                if mir_fn.return_type != TypeRef::Void {
                    match val_of_type(&mir_fn.return_type) {
                        Val::F64 => {
                            fb.f64c(0.0);
                        }
                        Val::I32 => {
                            fb.i32c(0);
                        }
                        Val::I64 => {
                            fb.i64c(0);
                        }
                    }
                }
                fb.op(op::RETURN);
            }
            Terminator::Unreachable => {
                fb.op(op::UNREACHABLE);
            }
        }
        Ok(())
    }

    /// Lower one MIR function into the block/loop dispatcher form.
    fn lower_function(&mut self, mir_fn: &MirFunction) -> Result<(Vec<Val>, Vec<u8>), String> {
        if mir_fn.blocks.is_empty() {
            return Err(format!("MIR function '{}' has no blocks", mir_fn.name));
        }
        let (local_index, extra_locals) = Self::local_indices(mir_fn);
        let local_count = mir_fn.locals.len();
        let disp_local = (local_count + SCR_COUNT) as u32;

        let layout = Self::block_layout(mir_fn);
        let mut position: HashMap<BlockId, usize> = HashMap::with_capacity(layout.len());
        for (pos, block_id) in layout.iter().enumerate() {
            position.insert(*block_id, pos);
        }
        let total = layout.len();
        let entry_pos = position[&mir_fn.blocks.first().unwrap().id];

        self.preintern_function(mir_fn);

        let mut fb = FB::new(mir_fn.params.len() as u32);
        for extra in &extra_locals {
            fb.extras.push(*extra);
        }
        // Four scratch locals + the dispatcher state.
        fb.extras
            .extend_from_slice(&[Val::I32, Val::I32, Val::I64, Val::F64, Val::I32]);

        // Dispatch state := entry block position.
        fb.i32c(entry_pos as i64);
        fb.s(disp_local);

        // (block $bad (loop $dispatch (block $L_{n-1} … (block $L_0 …
        fb.block();
        fb.loop_();
        for _ in 0..total {
            fb.block();
        }
        // br_table $L_0 … $L_{n-1}, default $bad (depth total + 1).
        fb.g(disp_local);
        fb.op(op::BR_TABLE);
        uleb(&mut fb.body, total as u64);
        for label in 0..total {
            uleb(&mut fb.body, label as u64);
        }
        uleb(&mut fb.body, (total + 1) as u64);

        for (pos, block_id) in layout.iter().enumerate() {
            fb.end();
            let block = &mir_fn.blocks[block_id.0];
            for instr in &block.instrs {
                self.lower_instr(&mut fb, instr, &local_index, mir_fn)?;
            }
            self.lower_terminator(
                &mut fb,
                &block.terminator,
                pos,
                &position,
                total,
                disp_local,
                &local_index,
                mir_fn,
            )?;
        }
        // Close the loop and the $bad block; emit the trap at function
        // depth. The br_table default lands at the end of $bad, so the pad
        // still traps — but a validator treats a block's `end` as reachable
        // (branch targets land there), so trapping only *inside* $bad would
        // leave the function-level end reachable with an empty stack, which
        // type-checks only for void functions. With the trap at function
        // depth the tail is unreachable for any result arity.
        fb.end();
        fb.end();
        fb.op(op::UNREACHABLE);
        // Function-level final end.
        fb.end();

        Ok((fb.extras, fb.body))
    }
}

// ── Synthesized functions ────────────────────────────────────────────────────

impl<'a> WasmCompiler<'a> {
    /// Body of the ARC destructor for one struct type: release every managed
    /// field exactly once, skipping fields demoted to weak by the cycle
    /// breaker (stored without a retain, so releasing would over-release).
    fn drop_body(&mut self, struct_id: StructTypeId) -> (Vec<Val>, Vec<u8>) {
        let definition = &self.type_table.definitions[struct_id.0];
        let (layout, _) = struct_layout(self.type_table, struct_id);
        let mut fb = FB::new(1);
        for ((field_name, field_type), field_layout) in
            definition.fields.iter().zip(layout.iter())
        {
            if self
                .weak_fields
                .contains(&(struct_id, field_name.clone()))
            {
                continue;
            }
            if field_type.is_managed() {
                fb.g(0);
                fb.load64(field_layout.offset as u32);
                fb.op(op::I32_WRAP_I64);
                fb.call(self.helper_index[&Helper::Release]);
            }
        }
        fb.end();
        (fb.extras, fb.body)
    }

    /// Body of a task thunk `(env: i32) -> i64`: load the arguments from the
    /// task environment tuple, call the async user function, and box the
    /// result into the raw i64 cell (`Bool` zero-extends, `Float` reinterprets,
    /// pointers zero-extend, `Void` becomes 0).
    fn thunk_body(&mut self, func_id: FuncId) -> Result<(Vec<Val>, Vec<u8>), String> {
        let function = &self.program.functions[&func_id];
        let parameter_types: Vec<TypeRef> = function
            .params
            .iter()
            .map(|id| function.locals[id.0].ty.clone())
            .collect();
        let (layout, _) = tuple_layout(&parameter_types);
        let mut fb = FB::new(1);
        for field in &layout {
            fb.g(0);
            self.load_abi(&mut fb, field.abi, field.offset as u32);
        }
        fb.call(self.fn_index[&func_id]);
        match &function.return_type {
            TypeRef::Void => {
                fb.i64c(0);
            }
            TypeRef::Bool => {
                fb.op(op::I64_EXTEND_I32_U);
            }
            TypeRef::Float => {
                fb.op(op::I64_REINTERPRET_F64);
            }
            other if val_of_type(other) == Val::I32 => {
                fb.op(op::I64_EXTEND_I32_U);
            }
            _ => {}
        }
        fb.end();
        Ok((fb.extras, fb.body))
    }

    /// `_start`: the WASI entry point. A synchronous `main` is called
    /// directly; an `async def main` is wrapped in an empty-environment task
    /// and driven to completion, mirroring the native entrypoint wrapper.
    fn body_start(&mut self, main_id: FuncId) -> Result<(Vec<Val>, Vec<u8>), String> {
        let main_fn = &self.program.functions[&main_id];
        if !main_fn.is_async {
            let mut fb = FB::new(0);
            fb.call(self.fn_index[&main_id]);
            if main_fn.return_type != TypeRef::Void {
                fb.op(op::DROP);
            }
            fb.end();
            return Ok((fb.extras, fb.body));
        }
        let thunk_pos = self
            .task_order
            .iter()
            .position(|id| *id == main_id)
            .ok_or_else(|| "async main has no task thunk".to_string())?;
        let seat = self.table.thunks_start + thunk_pos as u32;
        let mut fb = FB::new(0);
        let env = fb.scratch(Val::I32);
        let task = fb.scratch(Val::I32);
        // env = lpp_tuple_alloc(16, 0, 0)
        fb.i32c(16);
        fb.i64c(0);
        fb.i64c(0);
        fb.call(self.helper_index[&Helper::TupleAlloc]);
        fb.s(env);
        // task = lpp_task_new(thunk, env, managed = 0)
        fb.i64c(seat as i64);
        fb.g(env);
        fb.i64c(0);
        fb.call(self.helper_index[&Helper::TaskNew]);
        fb.s(task);
        // lpp_task_await(task); lpp_arc_release(task)
        fb.g(task);
        fb.call(self.helper_index[&Helper::TaskAwait]);
        fb.op(op::DROP);
        fb.g(task);
        fb.call(self.helper_index[&Helper::Release]);
        fb.end();
        Ok((fb.extras, fb.body))
    }
}

// ── Helper function bodies: ARC core ─────────────────────────────────────────

impl<'a> WasmCompiler<'a> {
    fn helper_body(&mut self, helper: Helper) -> Result<(Vec<Val>, Vec<u8>), String> {
        Ok(match helper {
            Helper::TrapStub => self.h_trap_stub(),
            Helper::Alloc => self.h_alloc(),
            Helper::ArcAlloc => self.h_arc_alloc(),
            Helper::Retain => self.h_retain(),
            Helper::Release => self.h_release(),
            Helper::StrAlloc => self.h_str_alloc(),
            Helper::StrNew => self.h_str_new(),
            Helper::WriteFd => self.h_write_fd(),
            Helper::Write => self.h_write(),
            Helper::FmtU64 => self.h_fmt_u64(),
            Helper::WriteU64 => self.h_write_u64(),
            Helper::PrintInt => self.h_print_int(),
            Helper::PrintBool => self.h_print_bool(),
            Helper::PrintFloat => self.h_print_float(),
            Helper::PrintStr => self.h_print_str(),
            Helper::PanicMsg => self.h_panic_msg(),
            Helper::Panic2 => self.h_panic2(),
            Helper::Panic3 => self.h_panic3(),
            Helper::Exit => self.h_exit(),
            Helper::Input => self.h_input(),
            Helper::EnvMatch => self.h_env_match(),
            Helper::EnvGet => self.h_env_get(),
            Helper::StrLen => self.h_str_len(),
            Helper::StrEq => self.h_str_eq(),
            Helper::StrConcat => self.h_str_concat(),
            Helper::StrSubstr => self.h_str_substr(),
            Helper::StrTrim => self.h_str_trim(),
            Helper::StrUpper => self.h_str_case(true),
            Helper::StrLower => self.h_str_case(false),
            Helper::IntToStr => self.h_int_to_str(),
            Helper::FloatToStr => self.h_float_to_str(),
            Helper::BoolToStr => self.h_bool_to_str(),
            Helper::StrToInt => self.h_str_to_int(),
            Helper::CharAt => self.h_char_at(),
            Helper::Ord => self.h_ord(),
            Helper::Chr => self.h_chr(),
            Helper::StrFindFrom => self.h_str_find_from(),
            Helper::StrFind => self.h_str_find(),
            Helper::StrContains => self.h_str_contains(),
            Helper::StrStartsWith => self.h_str_starts_with(),
            Helper::StrEndsWith => self.h_str_ends_with(),
            Helper::StrRepeat => self.h_str_repeat(),
            Helper::StrReplace => self.h_str_replace(),
            Helper::StrSplit => self.h_str_split(),
            Helper::ListNew => self.h_list_new(),
            Helper::ListPush => self.h_list_push(),
            Helper::ListSet => self.h_list_set(),
            Helper::ListGet => self.h_list_get(),
            Helper::ListLen => self.h_list_len(),
            Helper::ListDestroy => self.h_list_destroy(),
            Helper::MapNew => self.h_map_new(),
            Helper::MapLen => self.h_map_len(),
            Helper::HashStr => self.h_hash_str(),
            Helper::HashInt => self.h_hash_int(),
            Helper::MapProbe => self.h_map_probe(),
            Helper::MapRehash => self.h_map_rehash(),
            Helper::MapPut => self.h_map_put(),
            Helper::MapGet => self.h_map_get(),
            Helper::MapHas => self.h_map_has(),
            Helper::MapRemove => self.h_map_remove(),
            Helper::MapDestroy => self.h_map_destroy(),
            Helper::TupleAlloc => self.h_tuple_alloc(),
            Helper::TupleDestroy => self.h_tuple_destroy(),
            Helper::ClosureDestroy => self.h_closure_destroy(),
            Helper::TaskNew => self.h_task_new(),
            Helper::TaskRun => self.h_task_run(),
            Helper::TaskAwait => self.h_task_await(),
            Helper::TaskPoll => self.h_task_poll(),
            Helper::TaskDestroy => self.h_task_destroy(),
            Helper::SliceNew => self.h_slice_new(),
            Helper::SliceLen => self.h_slice_len(),
            Helper::SliceGet => self.h_slice_get(),
            Helper::StrSliceGet => self.h_str_slice_get(),
            Helper::StrSliceToStr => self.h_str_slice_to_str(),
            Helper::IntPow => self.h_int_pow(),
            Helper::Log2 => self.h_log2(),
            Helper::Exp2 => self.h_exp2(),
            Helper::Pow => self.h_pow(),
            Helper::Reduce2Pi => self.h_reduce_2pi(),
            Helper::SinPoly => self.h_sin_poly(),
            Helper::CosPoly => self.h_cos_poly(),
            Helper::Trig => self.h_trig(),
            Helper::TimeSeed => self.h_time_seed(),
            Helper::Random => self.h_random(),
            Helper::RandomRange => self.h_random_range(),
            Helper::TimeMs => self.h_time_ms(),
            Helper::SleepMs => self.h_sleep_ms(),
        })
    }

    fn h_trap_stub(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(0);
        fb.op(op::UNREACHABLE);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `Alloc(size) -> ptr`: 8-aligned bump allocation with `memory.grow`.
    fn h_alloc(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        let h = fb.scratch(Val::I32);
        let new = fb.scratch(Val::I32);
        let need = fb.scratch(Val::I32);
        // size = (size + 7) & -8
        fb.g(0).i32c(7).op(op::I32_ADD).i32c(-8).op(op::I32_AND).s(0);
        // h = heap; new = h + size
        fb.gget(GLOBAL_HEAP).s(h);
        fb.g(h).g(0).op(op::I32_ADD).s(new);
        // if (u32)new > memory.size() << 16: grow
        fb.g(new);
        fb.memory_size();
        fb.i32c(16).op(op::I32_SHL);
        fb.op(op::I32_GT_U);
        fb.if_();
        // need = (new + 65535) >>> 16 - memory.size()
        fb.g(new).i32c(65535).op(op::I32_ADD);
        fb.i32c(16).op(op::I32_SHR_U);
        fb.memory_size().op(op::I32_SUB).s(need);
        // grow max(need, 32) pages
        fb.g(need).i32c(32).g(need).i32c(32).op(op::I32_GT_U).op(op::SELECT);
        fb.memory_grow();
        fb.i32c(-1).op(op::I32_EQ);
        fb.if_().op(op::UNREACHABLE).end();
        fb.end();
        // heap = new; return h
        fb.g(new).gset(GLOBAL_HEAP);
        fb.g(h);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `ArcAlloc(size, drop_index) -> payload`: 24-byte header + zeroed payload.
    fn h_arc_alloc(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(2);
        let a = fb.scratch(Val::I32);
        // a = Alloc(size + 24)
        fb.g(0).i32c(ARC_HEADER_SIZE as i64).op(op::I32_ADD);
        fb.call(self.helper_index[&Helper::Alloc]);
        fb.s(a);
        // header: rc = 1, drop = drop_index, magic
        fb.g(a).i64c(1).store64(0);
        fb.g(a).g(1).op(op::I64_EXTEND_I32_U).store64(8);
        fb.g(a).i64c(0x4C505057).store64(16);
        // payload = a + 24
        fb.g(a).i32c(ARC_HEADER_SIZE as i64).op(op::I32_ADD);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `Retain(p)`: no-op for null and immortal literals.
    fn h_retain(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        fb.g(0).if_();
        // if rc != IMMORTAL: rc += 1
        fb.g(0).i32c(24).op(op::I32_SUB).load64(0);
        fb.i64c(ARC_IMMORTAL).op(op::I64_NE);
        fb.if_();
        fb.g(0).i32c(24).op(op::I32_SUB); // addr
        fb.g(0).i32c(24).op(op::I32_SUB).load64(0); // rc
        fb.i64c(1).op(op::I64_ADD);
        fb.store64(0);
        fb.end();
        fb.end();
        fb.end();
        (fb.extras, fb.body)
    }

    /// `Release(p)`: decrement, and on the last reference run the destructor
    /// through the wasm table. Memory itself is never recycled (bump heap).
    fn h_release(&mut self) -> (Vec<Val>, Vec<u8>) {
        let drop_type = self.drop_call_type();
        let mut fb = FB::new(1);
        let rc = fb.scratch(Val::I64);
        let drop_idx = fb.scratch(Val::I32);
        fb.g(0).if_();
        fb.g(0).i32c(24).op(op::I32_SUB).load64(0).s(rc);
        fb.g(rc).i64c(ARC_IMMORTAL).op(op::I64_NE);
        fb.if_();
        // rc < 1 is a double release: fail loudly.
        fb.g(rc).i64c(1).op(op::I64_LT_S);
        fb.if_().op(op::UNREACHABLE).end();
        fb.g(rc).i64c(1).op(op::I64_EQ);
        fb.if_();
        // Dying: mark, then dispatch the destructor if present.
        fb.g(0).i32c(24).op(op::I32_SUB).i64c(0).store64(0);
        fb.g(0).i32c(16).op(op::I32_SUB).load64(0);
        fb.op(op::I32_WRAP_I64).s(drop_idx);
        fb.g(drop_idx).if_();
        fb.g(0).g(drop_idx).call_indirect(drop_type);
        fb.end();
        fb.else_();
        fb.g(0).i32c(24).op(op::I32_SUB);
        fb.g(rc).i64c(1).op(op::I64_SUB);
        fb.store64(0);
        fb.end();
        fb.end();
        fb.end();
        fb.end();
        (fb.extras, fb.body)
    }

    /// `StrAlloc(len) -> str`: an ARC string slot with `len` set, bytes zeroed.
    fn h_str_alloc(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        let out = fb.scratch(Val::I32);
        fb.g(0).i32c(4).op(op::I32_ADD).i32c(0);
        fb.call(self.helper_index[&Helper::ArcAlloc]);
        fb.t(out);
        fb.g(0).store32(0);
        fb.g(out);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `StrNew(src, len) -> str`: allocate + byte copy.
    fn h_str_new(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(2);
        let out = fb.scratch(Val::I32);
        fb.g(1).call(self.helper_index[&Helper::StrAlloc]).s(out);
        fb.g(out).i32c(4).op(op::I32_ADD).g(0).g(1).memory_copy();
        fb.g(out);
        fb.end();
        (fb.extras, fb.body)
    }

    // ── I/O and formatting ───────────────────────────────────────────────

    /// `WriteFd(ptr, len, fd)`: the raw WASI write.
    fn h_write_fd(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(3);
        fb.i32c(IOVEC_BUF as i64).g(0).store32(0);
        fb.i32c(IOVEC_LEN as i64).g(1).store32(0);
        fb.g(2).i32c(0).i32c(1).i32c(FD_IO_OUT as i64);
        fb.call(self.import_index[&Wasi::FdWrite]);
        fb.op(op::DROP);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `Write(ptr, len)` = stdout.
    fn h_write(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(2);
        fb.g(0).g(1).i32c(1).call(self.helper_index[&Helper::WriteFd]);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `FmtU64(v) -> count`: emit the unsigned decimal digits of `v` into
    /// the *tail* of NUM_BUF (digits end at NUM_BUF + 64).
    fn h_fmt_u64(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        let pos = fb.scratch(Val::I32);
        fb.i32c(NUM_BUF_SIZE as i64).s(pos);
        fb.block();
        fb.loop_();
        fb.g(pos).i32c(1).op(op::I32_SUB).t(pos).i32c(NUM_BUF as i64).op(op::I32_ADD);
        fb.g(0).i64c(10).op(op::I64_REM_U).op(op::I32_WRAP_I64);
        fb.i32c(48).op(op::I32_ADD).store8(0);
        fb.g(0).i64c(10).op(op::I64_DIV_U).s(0);
        fb.g(0).op(op::I64_EQZ).br_if(1);
        fb.br(0);
        fb.end();
        fb.end();
        fb.i32c(NUM_BUF_SIZE as i64).g(pos).op(op::I32_SUB);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `WriteU64(v)`: print v's digits to stdout (no newline).
    fn h_write_u64(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        let cnt = fb.scratch(Val::I32);
        fb.g(0).call(self.helper_index[&Helper::FmtU64]).s(cnt);
        fb.i32c((NUM_BUF + NUM_BUF_SIZE) as i64).g(cnt).op(op::I32_SUB);
        fb.g(cnt);
        fb.call(self.helper_index[&Helper::Write]);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `PrintInt(x)`: matches the native "%lld\n".
    fn h_print_int(&mut self) -> (Vec<Val>, Vec<u8>) {
        let dash = self.lit_addr(b"-");
        let nl = self.lit_addr(b"\n");
        let mut fb = FB::new(1);
        let mag = fb.scratch(Val::I64);
        fb.g(0).i64c(0).op(op::I64_LT_S).if_();
        fb.i32c(dash as i64).i32c(1).call(self.helper_index[&Helper::Write]);
        fb.i64c(0).g(0).op(op::I64_SUB).s(mag);
        fb.else_();
        fb.g(0).s(mag);
        fb.end();
        fb.g(mag).call(self.helper_index[&Helper::WriteU64]);
        fb.i32c(nl as i64).i32c(1).call(self.helper_index[&Helper::Write]);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `PrintBool(b)`: native prints "1"/"0" plus newline.
    fn h_print_bool(&mut self) -> (Vec<Val>, Vec<u8>) {
        let one = self.lit_addr(b"1\n");
        let zero = self.lit_addr(b"0\n");
        let mut fb = FB::new(1);
        fb.g(0).if_();
        fb.i32c(one as i64).i32c(2).call(self.helper_index[&Helper::Write]);
        fb.else_();
        fb.i32c(zero as i64).i32c(2).call(self.helper_index[&Helper::Write]);
        fb.end();
        fb.end();
        (fb.extras, fb.body)
    }

    /// `PrintStr(s)`: bytes + "\n" (matches `puts`).
    fn h_print_str(&mut self) -> (Vec<Val>, Vec<u8>) {
        let nl = self.lit_addr(b"\n");
        let mut fb = FB::new(1);
        fb.g(0).i32c(4).op(op::I32_ADD);
        fb.g(0).load32(0);
        fb.call(self.helper_index[&Helper::Write]);
        fb.i32c(nl as i64).i32c(1).call(self.helper_index[&Helper::Write]);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `PrintFloat(x)`: matches the native "%f\n" (see v1 semantics).
    fn h_print_float(&mut self) -> (Vec<Val>, Vec<u8>) {
        let dash = self.lit_addr(b"-");
        let dot = self.lit_addr(b".");
        let nl = self.lit_addr(b"\n");
        let nan = self.lit_addr(b"nan\n");
        let inf = self.lit_addr(b"inf\n");
        let neginf = self.lit_addr(b"-inf\n");
        let zeros = self.lit_addr(b".000000\n");
        let mut fb = FB::new(1);
        let neg = fb.scratch(Val::I32);
        let n = fb.scratch(Val::I64);
        let pos = fb.scratch(Val::I32);
        // NaN?
        fb.g(0).g(0).op(op::F64_NE).if_();
        fb.i32c(nan as i64).i32c(4).call(self.helper_index[&Helper::Write]);
        fb.op(op::RETURN).end();
        // ±inf?
        fb.g(0).f64c(0.0).op(op::F64_MUL).f64c(0.0).op(op::F64_NE).if_();
        fb.g(0).f64c(0.0).op(op::F64_LT).if_();
        fb.i32c(neginf as i64).i32c(5).call(self.helper_index[&Helper::Write]);
        fb.else_();
        fb.i32c(inf as i64).i32c(4).call(self.helper_index[&Helper::Write]);
        fb.end();
        fb.op(op::RETURN).end();
        // sign
        fb.g(0).f64c(0.0).op(op::F64_LT).s(neg);
        fb.g(0).op(op::F64_ABS).s(0);
        fb.g(neg).if_();
        fb.i32c(dash as i64).i32c(1).call(self.helper_index[&Helper::Write]);
        fb.end();
        // fixed 6-digit fraction for |x| < 9e12
        fb.g(0).f64c(9.0e12).op(op::F64_LT).if_();
        fb.g(0).f64c(1_000_000.0).op(op::F64_MUL).f64c(0.5).op(op::F64_ADD);
        fb.op(op::I64_TRUNC_F64_U).s(n);
        fb.g(n).i64c(1_000_000).op(op::I64_DIV_U);
        fb.call(self.helper_index[&Helper::WriteU64]);
        fb.i32c(dot as i64).i32c(1).call(self.helper_index[&Helper::Write]);
        fb.g(n).i64c(1_000_000).op(op::I64_REM_U).s(n);
        fb.i32c(6).s(pos);
        fb.loop_();
        fb.g(pos).i32c(1).op(op::I32_SUB).t(pos).i32c(NUM_BUF as i64).op(op::I32_ADD);
        fb.g(n).i64c(10).op(op::I64_REM_U).op(op::I32_WRAP_I64);
        fb.i32c(48).op(op::I32_ADD).store8(0);
        fb.g(n).i64c(10).op(op::I64_DIV_U).s(n);
        fb.g(pos).br_if(0);
        fb.end();
        fb.i32c(NUM_BUF as i64).i32c(6).call(self.helper_index[&Helper::Write]);
        fb.i32c(nl as i64).i32c(1).call(self.helper_index[&Helper::Write]);
        fb.else_();
        fb.g(0).op(op::I64_TRUNC_F64_U);
        fb.call(self.helper_index[&Helper::WriteU64]);
        fb.i32c(zeros as i64).i32c(8).call(self.helper_index[&Helper::Write]);
        fb.end();
        fb.end();
        (fb.extras, fb.body)
    }

    /// `PanicMsg(msg)`: write msg + newline to stderr, then trap.
    fn h_panic_msg(&mut self) -> (Vec<Val>, Vec<u8>) {
        let nl = self.lit_addr(b"\n");
        let mut fb = FB::new(1);
        fb.g(0).i32c(4).op(op::I32_ADD).g(0).load32(0).i32c(2);
        fb.call(self.helper_index[&Helper::WriteFd]);
        fb.i32c(nl as i64).i32c(1).i32c(2).call(self.helper_index[&Helper::WriteFd]);
        fb.op(op::UNREACHABLE);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `Panic2(pre, a, mid, b)`: `pre` + dec(a) + `mid` + dec(b) + newline to
    /// stderr, then trap. Used for bounds diagnostics.
    fn h_panic2(&mut self) -> (Vec<Val>, Vec<u8>) {
        let nl = self.lit_addr(b"\n");
        let dash = self.lit_addr(b"-");
        let mut fb = FB::new(4);
        let cnt = fb.scratch(Val::I32);
        let nm = fb.scratch(Val::I64);
        // Signed decimal, matching the native `%lld` diagnostics. The
        // magnitude must be selected through a local: every structured
        // instruction we emit has the empty block type, so an if/else that
        // leaves a value on the stack would be invalid wasm.
        macro_rules! num {
            ($v:expr) => {{
                fb.g($v).i64c(0).op(op::I64_LT_S).if_();
                fb.i32c(dash as i64).i32c(1).i32c(2).call(self.helper_index[&Helper::WriteFd]);
                fb.i64c(0).g($v).op(op::I64_SUB).s(nm);
                fb.else_();
                fb.g($v).s(nm);
                fb.end();
                fb.g(nm).call(self.helper_index[&Helper::FmtU64]).s(cnt);
                fb.i32c((NUM_BUF + NUM_BUF_SIZE) as i64).g(cnt).op(op::I32_SUB);
                fb.g(cnt).i32c(2).call(self.helper_index[&Helper::WriteFd]);
            }};
        }
        macro_rules! lit {
            ($v:expr) => {{
                fb.g($v).i32c(4).op(op::I32_ADD).g($v).load32(0).i32c(2);
                fb.call(self.helper_index[&Helper::WriteFd]);
            }};
        }
        lit!(0);
        num!(1);
        lit!(2);
        num!(3);
        fb.i32c(nl as i64).i32c(1).i32c(2).call(self.helper_index[&Helper::WriteFd]);
        fb.op(op::UNREACHABLE);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `Exit(code)`: proc_exit with the low 32 bits of the code.
    fn h_exit(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        fb.g(0).op(op::I32_WRAP_I64);
        fb.call(self.import_index[&Wasi::ProcExit]);
        fb.op(op::UNREACHABLE);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `Input() -> str`: one stdin line, trailing '\n' stripped (fgets
    /// semantics with the same native 4095-byte ceiling).
    fn h_input(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(0);
        let pos = fb.scratch(Val::I32);
        fb.i32c(0).s(pos);
        fb.block();
        fb.loop_();
        fb.g(pos).i32c((INPUT_BUF_SIZE - 1) as i64).op(op::I32_GE_U).br_if(1);
        fb.i32c(IOVEC_BUF as i64).i32c(INPUT_BUF as i64).g(pos).op(op::I32_ADD).store32(0);
        fb.i32c(IOVEC_LEN as i64).i32c(1).store32(0);
        fb.i32c(0).i32c(0).i32c(1).i32c(FD_IO_OUT as i64);
        fb.call(self.import_index[&Wasi::FdRead]);
        fb.br_if(1); // errno != 0 → stop (empty/failed read)
        fb.i32c(FD_IO_OUT as i64).load32(0).op(op::I32_EQZ).br_if(1); // EOF
        fb.i32c(INPUT_BUF as i64).g(pos).op(op::I32_ADD).load8(0);
        fb.i32c(10).op(op::I32_EQ).br_if(1); // newline (not stored)
        fb.g(pos).i32c(1).op(op::I32_ADD).s(pos);
        fb.br(0);
        fb.end();
        fb.end();
        fb.i32c(INPUT_BUF as i64).g(pos);
        fb.call(self.helper_index[&Helper::StrNew]);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `EnvMatch(name, entry) -> value_ptr | 0`: does a NUL-terminated
    /// `entry` start with `name=`? If so returns the value pointer.
    fn h_env_match(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(2);
        let j = fb.scratch(Val::I32);
        let c = fb.scratch(Val::I32);
        fb.i32c(0).s(j);
        fb.block();
        fb.loop_();
        fb.g(1).g(j).op(op::I32_ADD).load8(0).t(c);
        fb.op(op::I32_EQZ).br_if(1); // end of entry → no match
        fb.g(j).g(0).load32(0).op(op::I32_GE_U).if_();
        // j >= name length: only '=' continues a match.
        fb.g(c).i32c(61).op(op::I32_EQ).if_();
        fb.g(1).g(0).load32(0).op(op::I32_ADD).i32c(1).op(op::I32_ADD);
        fb.op(op::RETURN);
        fb.end();
        fb.i32c(0).op(op::RETURN);
        fb.end();
        // c must equal name[j]
        fb.g(c).g(0).i32c(4).op(op::I32_ADD).g(j).op(op::I32_ADD).load8(0);
        fb.op(op::I32_NE).if_().i32c(0).op(op::RETURN).end();
        fb.g(j).i32c(1).op(op::I32_ADD).s(j);
        fb.br(0);
        fb.end();
        fb.end();
        fb.i32c(0);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `EnvGet(name) -> str`: WASI environ scan; "" when unset.
    fn h_env_get(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        let count = fb.scratch(Val::I32);
        let pbuf = fb.scratch(Val::I32);
        let sbuf = fb.scratch(Val::I32);
        let i = fb.scratch(Val::I32);
        let e = fb.scratch(Val::I32);
        let v = fb.scratch(Val::I32);
        let vlen = fb.scratch(Val::I32);
        // environ_sizes_get(&count, &bufsz); reuse the clock scratch cells.
        fb.i32c(CLOCK_BUF as i64).i32c((CLOCK_BUF + 4) as i64);
        fb.call(self.import_index[&Wasi::EnvironSizesGet]);
        fb.if_();
        fb.i32c(self.empty_string_addr() as i64).op(op::RETURN);
        fb.end();
        fb.i32c(CLOCK_BUF as i64).load32(0).s(count);
        fb.g(count).op(op::I32_EQZ).if_();
        fb.i32c(self.empty_string_addr() as i64).op(op::RETURN);
        fb.end();
        fb.g(count).i32c(4).op(op::I32_MUL).call(self.helper_index[&Helper::Alloc]).s(pbuf);
        fb.i32c((CLOCK_BUF + 4) as i64).load32(0);
        fb.call(self.helper_index[&Helper::Alloc]).s(sbuf);
        fb.g(pbuf).g(sbuf).call(self.import_index[&Wasi::EnvironGet]);
        fb.if_();
        fb.i32c(self.empty_string_addr() as i64).op(op::RETURN);
        fb.end();
        fb.i32c(0).s(i);
        fb.block();
        fb.loop_();
        fb.g(i).g(count).op(op::I32_GE_U).br_if(1);
        fb.g(pbuf).g(i).i32c(4).op(op::I32_MUL).op(op::I32_ADD).load32(0).s(e);
        fb.g(0).g(e).call(self.helper_index[&Helper::EnvMatch]).s(v);
        fb.g(v).if_();
        // value length (NUL scan)
        fb.i32c(0).s(vlen);
        fb.block();
        fb.loop_();
        fb.g(v).g(vlen).op(op::I32_ADD).load8(0).op(op::I32_EQZ).br_if(1);
        fb.g(vlen).i32c(1).op(op::I32_ADD).s(vlen);
        fb.br(0);
        fb.end();
        fb.end();
        fb.g(v).g(vlen).call(self.helper_index[&Helper::StrNew]);
        fb.op(op::RETURN);
        fb.end();
        fb.g(i).i32c(1).op(op::I32_ADD).s(i);
        fb.br(0);
        fb.end();
        fb.end();
        fb.i32c(self.empty_string_addr() as i64);
        fb.end();
        (fb.extras, fb.body)
    }

    // ── Strings ──────────────────────────────────────────────────────────

    /// `StrLen(s) -> i64`.
    fn h_str_len(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        fb.g(0).load32(0).op(op::I64_EXTEND_I32_U);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `StrEq(a, b) -> i32`.
    fn h_str_eq(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(2);
        let la = fb.scratch(Val::I32);
        let lb = fb.scratch(Val::I32);
        let i = fb.scratch(Val::I32);
        fb.g(0).load32(0).s(la);
        fb.g(1).load32(0).s(lb);
        fb.g(la).g(lb).op(op::I32_NE).if_().i32c(0).op(op::RETURN).end();
        fb.i32c(0).s(i);
        fb.loop_();
        fb.g(i).g(la).op(op::I32_GE_U).if_().i32c(1).op(op::RETURN).end();
        fb.g(0).i32c(4).op(op::I32_ADD).g(i).op(op::I32_ADD).load8(0);
        fb.g(1).i32c(4).op(op::I32_ADD).g(i).op(op::I32_ADD).load8(0);
        fb.op(op::I32_NE).if_().i32c(0).op(op::RETURN).end();
        fb.g(i).i32c(1).op(op::I32_ADD).s(i);
        fb.br(0);
        fb.end();
        fb.op(op::UNREACHABLE);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `StrConcat(a, b) -> str`.
    fn h_str_concat(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(2);
        let la = fb.scratch(Val::I32);
        let lb = fb.scratch(Val::I32);
        let out = fb.scratch(Val::I32);
        fb.g(0).load32(0).s(la);
        fb.g(1).load32(0).s(lb);
        fb.g(la).g(lb).op(op::I32_ADD).call(self.helper_index[&Helper::StrAlloc]).s(out);
        fb.g(out).i32c(4).op(op::I32_ADD).g(0).i32c(4).op(op::I32_ADD).g(la).memory_copy();
        fb.g(out).i32c(4).op(op::I32_ADD).g(la).op(op::I32_ADD);
        fb.g(1).i32c(4).op(op::I32_ADD).g(lb).memory_copy();
        fb.g(out);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `StrSubstr(s, start, len) -> str` (native clamping semantics).
    fn h_str_substr(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(3);
        let slen = fb.scratch(Val::I64);
        let copy = fb.scratch(Val::I64);
        let out = fb.scratch(Val::I32);
        fb.g(0).load32(0).op(op::I64_EXTEND_I32_U).s(slen);
        fb.g(1).i64c(0).op(op::I64_LT_S).if_().i64c(0).s(1).end();
        fb.g(1).g(slen).op(op::I64_GT_S).if_();
        fb.i32c(self.empty_string_addr() as i64).op(op::RETURN);
        fb.end();
        // copy = remain; if !(len<0 || len>remain) → copy = len
        fb.g(slen).g(1).op(op::I64_SUB).s(copy);
        fb.g(2).i64c(0).op(op::I64_LT_S);
        fb.g(2).g(copy).op(op::I64_GT_S);
        fb.op(op::I32_OR);
        fb.if_();
        fb.else_();
        fb.g(2).s(copy);
        fb.end();
        fb.g(copy).op(op::I32_WRAP_I64).call(self.helper_index[&Helper::StrAlloc]).s(out);
        fb.g(out).i32c(4).op(op::I32_ADD);
        fb.g(0).i32c(4).op(op::I32_ADD).g(1).op(op::I32_WRAP_I64).op(op::I32_ADD);
        fb.g(copy).op(op::I32_WRAP_I64);
        fb.memory_copy();
        fb.g(out);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `StrTrim(s) -> str` (native whitespace set: space, \t, \n, \r).
    fn h_str_trim(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        let lo = fb.scratch(Val::I32);
        let hi = fb.scratch(Val::I32);
        let out = fb.scratch(Val::I32);
        let c = fb.scratch(Val::I32);
        macro_rules! is_ws {
            () => {{
                // c on stack → ws=1/0 for {32,9,10,13}
                fb.t(c);
                fb.i32c(32).op(op::I32_EQ);
                fb.g(c).i32c(9).op(op::I32_EQ).op(op::I32_OR);
                fb.g(c).i32c(10).op(op::I32_EQ).op(op::I32_OR);
                fb.g(c).i32c(13).op(op::I32_EQ).op(op::I32_OR);
            }};
        }
        // lo = first non-ws offset
        fb.i32c(0).s(lo);
        fb.block();
        fb.loop_();
        fb.g(lo).g(0).load32(0).op(op::I32_GE_U).br_if(1);
        fb.g(0).i32c(4).op(op::I32_ADD).g(lo).op(op::I32_ADD).load8(0);
        is_ws!();
        fb.op(op::I32_EQZ).br_if(1);
        fb.g(lo).i32c(1).op(op::I32_ADD).s(lo);
        fb.br(0);
        fb.end();
        fb.end();
        // hi = one past the last non-ws
        fb.g(0).load32(0).s(hi);
        fb.block();
        fb.loop_();
        fb.g(hi).g(lo).op(op::I32_LE_S).br_if(1);
        fb.g(0).i32c(4).op(op::I32_ADD).g(hi).op(op::I32_ADD).i32c(1).op(op::I32_SUB).load8(0);
        is_ws!();
        fb.op(op::I32_EQZ).br_if(1);
        fb.g(hi).i32c(1).op(op::I32_SUB).s(hi);
        fb.br(0);
        fb.end();
        fb.end();
        // out = s[lo..hi]
        fb.g(hi).g(lo).op(op::I32_SUB).call(self.helper_index[&Helper::StrAlloc]).s(out);
        fb.g(out).i32c(4).op(op::I32_ADD);
        fb.g(0).i32c(4).op(op::I32_ADD).g(lo).op(op::I32_ADD);
        fb.g(hi).g(lo).op(op::I32_SUB);
        fb.memory_copy();
        fb.g(out);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `StrUpper`/`StrLower`: ASCII case mapping.
    fn h_str_case(&mut self, upper: bool) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        let len = fb.scratch(Val::I32);
        let out = fb.scratch(Val::I32);
        let i = fb.scratch(Val::I32);
        let c = fb.scratch(Val::I32);
        let (lo_b, hi_b, delta) = if upper { (97, 122, -32) } else { (65, 90, 32) };
        fb.g(0).load32(0).s(len);
        fb.g(len).call(self.helper_index[&Helper::StrAlloc]).s(out);
        fb.i32c(0).s(i);
        fb.block();
        fb.loop_();
        fb.g(i).g(len).op(op::I32_GE_U).br_if(1);
        fb.g(0).i32c(4).op(op::I32_ADD).g(i).op(op::I32_ADD).load8(0).t(c);
        fb.i32c(lo_b).op(op::I32_GE_S);
        fb.g(c).i32c(hi_b).op(op::I32_LE_S);
        fb.op(op::I32_AND);
        fb.if_();
        fb.g(c).i32c(delta).op(op::I32_ADD).s(c);
        fb.end();
        fb.g(out).i32c(4).op(op::I32_ADD).g(i).op(op::I32_ADD).g(c).store8(0);
        fb.g(i).i32c(1).op(op::I32_ADD).s(i);
        fb.br(0);
        fb.end();
        fb.end();
        fb.g(out);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `IntToStr(v) -> str` ("%lld").
    fn h_int_to_str(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        let mag = fb.scratch(Val::I64);
        let cnt = fb.scratch(Val::I32);
        let neg = fb.scratch(Val::I32);
        let out = fb.scratch(Val::I32);
        fb.g(0).i64c(0).op(op::I64_LT_S).s(neg);
        fb.g(neg).if_();
        fb.i64c(0).g(0).op(op::I64_SUB).s(mag);
        fb.else_();
        fb.g(0).s(mag);
        fb.end();
        fb.g(mag).call(self.helper_index[&Helper::FmtU64]).s(cnt);
        fb.g(cnt).g(neg).op(op::I32_ADD).call(self.helper_index[&Helper::StrAlloc]).s(out);
        fb.g(neg).if_();
        fb.g(out).i32c(4).op(op::I32_ADD).i32c(45).store8(0);
        fb.end();
        fb.g(out).i32c(4).op(op::I32_ADD).g(neg).op(op::I32_ADD);
        fb.i32c((NUM_BUF + NUM_BUF_SIZE) as i64).g(cnt).op(op::I32_SUB);
        fb.g(cnt).memory_copy();
        fb.g(out);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `BoolToStr(b) -> str`: the immortal "true"/"false" literals.
    fn h_bool_to_str(&mut self) -> (Vec<Val>, Vec<u8>) {
        let yes = self.intern(b"true");
        let no = self.intern(b"false");
        let mut fb = FB::new(1);
        fb.i32c(yes as i64).i32c(no as i64).g(0).op(op::SELECT);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `StrToInt(s) -> i64` (strtoll base 10).
    fn h_str_to_int(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        let i = fb.scratch(Val::I32);
        let neg = fb.scratch(Val::I32);
        let acc = fb.scratch(Val::I64);
        let c = fb.scratch(Val::I32);
        // skip isspace: 32 or 9..=13
        fb.i32c(0).s(i);
        fb.block();
        fb.loop_();
        fb.g(i).g(0).load32(0).op(op::I32_GE_U).br_if(1);
        fb.g(0).i32c(4).op(op::I32_ADD).g(i).op(op::I32_ADD).load8(0).t(c);
        fb.i32c(32).op(op::I32_EQ);
        fb.g(c).i32c(9).op(op::I32_GE_S);
        fb.g(c).i32c(13).op(op::I32_LE_S);
        fb.op(op::I32_AND).op(op::I32_OR);
        fb.op(op::I32_EQZ).br_if(1);
        fb.g(i).i32c(1).op(op::I32_ADD).s(i);
        fb.br(0);
        fb.end();
        fb.end();
        // sign
        fb.i32c(0).s(neg);
        fb.g(0).i32c(4).op(op::I32_ADD).g(i).op(op::I32_ADD).load8(0).t(c);
        fb.i32c(45).op(op::I32_EQ).if_();
        fb.i32c(1).s(neg);
        fb.g(i).i32c(1).op(op::I32_ADD).s(i);
        fb.else_();
        fb.g(c).i32c(43).op(op::I32_EQ).if_();
        fb.g(i).i32c(1).op(op::I32_ADD).s(i);
        fb.end();
        fb.end();
        // digits
        fb.i64c(0).s(acc);
        fb.block();
        fb.loop_();
        fb.g(i).g(0).load32(0).op(op::I32_GE_U).br_if(1);
        fb.g(0).i32c(4).op(op::I32_ADD).g(i).op(op::I32_ADD).load8(0).t(c);
        fb.i32c(48).op(op::I32_LT_S);
        fb.g(c).i32c(57).op(op::I32_GT_S);
        fb.op(op::I32_OR).br_if(1);
        fb.g(acc).i64c(10).op(op::I64_MUL);
        fb.g(c).i32c(48).op(op::I32_SUB).op(op::I64_EXTEND_I32_U);
        fb.op(op::I64_ADD).s(acc);
        fb.g(i).i32c(1).op(op::I32_ADD).s(i);
        fb.br(0);
        fb.end();
        fb.end();
        fb.i64c(0).g(acc).op(op::I64_SUB).g(acc).g(neg).op(op::SELECT);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `CharAt(s, idx) -> str` (1 char; traps out of bounds).
    fn h_char_at(&mut self) -> (Vec<Val>, Vec<u8>) {
        let msg = self.intern(b"char_at index out of bounds");
        let mut fb = FB::new(2);
        let out = fb.scratch(Val::I32);
        fb.g(1).i64c(0).op(op::I64_LT_S);
        fb.g(1).g(0).load32(0).op(op::I64_EXTEND_I32_U).op(op::I64_GE_S);
        fb.op(op::I32_OR).if_();
        fb.i32c(msg as i64).call(self.helper_index[&Helper::PanicMsg]);
        fb.end();
        fb.i32c(1).call(self.helper_index[&Helper::StrAlloc]).s(out);
        fb.g(out).i32c(4).op(op::I32_ADD);
        fb.g(0).i32c(4).op(op::I32_ADD).g(1).op(op::I32_WRAP_I64).op(op::I32_ADD).load8(0);
        fb.store8(0);
        fb.g(out);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `Ord(s) -> i64`: first byte, 0 for "".
    fn h_ord(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        fb.g(0).load32(0).op(op::I32_EQZ).if_().i64c(0).op(op::RETURN).end();
        fb.g(0).i32c(4).op(op::I32_ADD).load8(0).op(op::I64_EXTEND_I32_U);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `Chr(code) -> str`: one byte, code & 0xFF.
    fn h_chr(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        let out = fb.scratch(Val::I32);
        fb.i32c(1).call(self.helper_index[&Helper::StrAlloc]).s(out);
        fb.g(out).i32c(4).op(op::I32_ADD);
        fb.g(0).op(op::I32_WRAP_I64);
        fb.store8(0);
        fb.g(out);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `StrFindFrom(h, n, from) -> offset | -1` (strstr semantics).
    fn h_str_find_from(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(3);
        let lh = fb.scratch(Val::I32);
        let ln = fb.scratch(Val::I32);
        let i = fb.scratch(Val::I32);
        let j = fb.scratch(Val::I32);
        fb.g(0).load32(0).s(lh);
        fb.g(1).load32(0).s(ln);
        // empty needle matches at min(from, lh)
        fb.g(ln).op(op::I32_EQZ).if_();
        fb.g(2).g(lh).g(2).g(lh).op(op::I32_LT_S).op(op::SELECT);
        fb.op(op::RETURN);
        fb.end();
        fb.g(2).s(i);
        fb.block(); // $notfound (depth 1 from outer loop)
        fb.loop_(); // outer (depth 0)
        fb.g(i).g(ln).op(op::I32_ADD).g(lh).op(op::I32_GT_S).br_if(1);
        fb.i32c(0).s(j);
        fb.block(); // $next (from inner loop: depth 1)
        fb.loop_(); // inner (depth 0)
        fb.g(j).g(ln).op(op::I32_GE_U).if_();
        fb.g(i).op(op::RETURN); // full needle matched
        fb.end();
        fb.g(0).i32c(4).op(op::I32_ADD).g(i).op(op::I32_ADD).g(j).op(op::I32_ADD).load8(0);
        fb.g(1).i32c(4).op(op::I32_ADD).g(j).op(op::I32_ADD).load8(0);
        fb.op(op::I32_NE).br_if(1); // mismatch → $next
        fb.g(j).i32c(1).op(op::I32_ADD).s(j);
        fb.br(0);
        fb.end();
        fb.end(); // $next
        fb.g(i).i32c(1).op(op::I32_ADD).s(i);
        fb.br(0);
        fb.end();
        fb.end(); // $notfound
        fb.i32c(-1);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `StrFind(h, n) -> i64`.
    fn h_str_find(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(2);
        fb.g(0).g(1).i32c(0).call(self.helper_index[&Helper::StrFindFrom]);
        fb.op(op::I64_EXTEND_I32_S);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `StrContains(h, n) -> i32`.
    fn h_str_contains(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(2);
        fb.g(0).g(1).i32c(0).call(self.helper_index[&Helper::StrFindFrom]);
        fb.i32c(-1).op(op::I32_NE);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `StrStartsWith(s, p) -> i32`.
    fn h_str_starts_with(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(2);
        let ls = fb.scratch(Val::I32);
        let lp = fb.scratch(Val::I32);
        let i = fb.scratch(Val::I32);
        fb.g(0).load32(0).s(ls);
        fb.g(1).load32(0).s(lp);
        fb.g(lp).g(ls).op(op::I32_GT_S).if_().i32c(0).op(op::RETURN).end();
        fb.i32c(0).s(i);
        fb.loop_();
        fb.g(i).g(lp).op(op::I32_GE_U).if_().i32c(1).op(op::RETURN).end();
        fb.g(0).i32c(4).op(op::I32_ADD).g(i).op(op::I32_ADD).load8(0);
        fb.g(1).i32c(4).op(op::I32_ADD).g(i).op(op::I32_ADD).load8(0);
        fb.op(op::I32_NE).if_().i32c(0).op(op::RETURN).end();
        fb.g(i).i32c(1).op(op::I32_ADD).s(i);
        fb.br(0);
        fb.end();
        fb.op(op::UNREACHABLE);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `StrEndsWith(s, x) -> i32`.
    fn h_str_ends_with(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(2);
        let ls = fb.scratch(Val::I32);
        let lx = fb.scratch(Val::I32);
        let off = fb.scratch(Val::I32);
        let i = fb.scratch(Val::I32);
        fb.g(0).load32(0).s(ls);
        fb.g(1).load32(0).s(lx);
        fb.g(lx).g(ls).op(op::I32_GT_S).if_().i32c(0).op(op::RETURN).end();
        fb.g(ls).g(lx).op(op::I32_SUB).s(off);
        fb.i32c(0).s(i);
        fb.loop_();
        fb.g(i).g(lx).op(op::I32_GE_U).if_().i32c(1).op(op::RETURN).end();
        fb.g(0).i32c(4).op(op::I32_ADD).g(off).op(op::I32_ADD).g(i).op(op::I32_ADD).load8(0);
        fb.g(1).i32c(4).op(op::I32_ADD).g(i).op(op::I32_ADD).load8(0);
        fb.op(op::I32_NE).if_().i32c(0).op(op::RETURN).end();
        fb.g(i).i32c(1).op(op::I32_ADD).s(i);
        fb.br(0);
        fb.end();
        fb.op(op::UNREACHABLE);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `StrRepeat(s, n) -> str`.
    fn h_str_repeat(&mut self) -> (Vec<Val>, Vec<u8>) {
        let msg = self.intern(b"str_repeat: length overflow");
        let mut fb = FB::new(2);
        let slen = fb.scratch(Val::I32);
        let total = fb.scratch(Val::I64);
        let out = fb.scratch(Val::I32);
        let i = fb.scratch(Val::I64);
        fb.g(1).i64c(0).op(op::I64_LE_S).if_();
        fb.i32c(self.empty_string_addr() as i64).op(op::RETURN);
        fb.end();
        fb.g(0).load32(0).s(slen);
        fb.g(slen).op(op::I32_EQZ).if_();
        fb.i32c(self.empty_string_addr() as i64).op(op::RETURN);
        fb.end();
        fb.g(slen).op(op::I64_EXTEND_I32_U).g(1).op(op::I64_MUL).s(total);
        fb.g(total).i64c(0x7fff_fff0).op(op::I64_GT_S).if_();
        fb.i32c(msg as i64).call(self.helper_index[&Helper::PanicMsg]);
        fb.end();
        fb.g(total).op(op::I32_WRAP_I64).call(self.helper_index[&Helper::StrAlloc]).s(out);
        fb.i64c(0).s(i);
        fb.block();
        fb.loop_();
        fb.g(i).g(1).op(op::I64_GE_S).br_if(1);
        fb.g(out).i32c(4).op(op::I32_ADD);
        fb.g(i).op(op::I32_WRAP_I64).g(slen).op(op::I32_MUL).op(op::I32_ADD);
        fb.g(0).i32c(4).op(op::I32_ADD).g(slen).memory_copy();
        fb.g(i).i64c(1).op(op::I64_ADD).s(i);
        fb.br(0);
        fb.end();
        fb.end();
        fb.g(out);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `StrReplace(s, old, new) -> str`.
    fn h_str_replace(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(3);
        let slen = fb.scratch(Val::I32);
        let olen = fb.scratch(Val::I32);
        let nlen = fb.scratch(Val::I32);
        let count = fb.scratch(Val::I32);
        let pos = fb.scratch(Val::I32);
        let found = fb.scratch(Val::I32);
        let out = fb.scratch(Val::I32);
        let src = fb.scratch(Val::I32);
        let dst = fb.scratch(Val::I32);
        fb.g(0).load32(0).s(slen);
        fb.g(1).load32(0).s(olen);
        fb.g(2).load32(0).s(nlen);
        // empty needle → plain copy (native contract)
        fb.g(olen).op(op::I32_EQZ).if_();
        fb.g(0).i32c(4).op(op::I32_ADD).g(slen).call(self.helper_index[&Helper::StrNew]);
        fb.op(op::RETURN);
        fb.end();
        // pass 1: count occurrences
        fb.i32c(0).s(count);
        fb.i32c(0).s(pos);
        fb.block();
        fb.loop_();
        fb.g(0).g(1).g(pos).call(self.helper_index[&Helper::StrFindFrom]).s(found);
        fb.g(found).i32c(-1).op(op::I32_EQ).br_if(1);
        fb.g(count).i32c(1).op(op::I32_ADD).s(count);
        fb.g(found).g(olen).op(op::I32_ADD).s(pos);
        fb.br(0);
        fb.end();
        fb.end();
        // outlen = slen + count * (nlen - olen)
        fb.g(slen).g(count).g(nlen).g(olen).op(op::I32_SUB).op(op::I32_MUL).op(op::I32_ADD);
        fb.call(self.helper_index[&Helper::StrAlloc]).s(out);
        // pass 2: splice
        fb.i32c(0).s(src);
        fb.i32c(0).s(dst);
        fb.block();
        fb.loop_();
        fb.g(0).g(1).g(src).call(self.helper_index[&Helper::StrFindFrom]).s(found);
        fb.g(found).i32c(-1).op(op::I32_EQ).br_if(1);
        // prefix
        fb.g(out).i32c(4).op(op::I32_ADD).g(dst).op(op::I32_ADD);
        fb.g(0).i32c(4).op(op::I32_ADD).g(src).op(op::I32_ADD);
        fb.g(found).g(src).op(op::I32_SUB).t(pos);
        fb.memory_copy();
        fb.g(dst).g(pos).op(op::I32_ADD).s(dst);
        // replacement
        fb.g(out).i32c(4).op(op::I32_ADD).g(dst).op(op::I32_ADD);
        fb.g(2).i32c(4).op(op::I32_ADD).g(nlen).memory_copy();
        fb.g(dst).g(nlen).op(op::I32_ADD).s(dst);
        fb.g(found).g(olen).op(op::I32_ADD).s(src);
        fb.br(0);
        fb.end();
        fb.end();
        // tail
        fb.g(out).i32c(4).op(op::I32_ADD).g(dst).op(op::I32_ADD);
        fb.g(0).i32c(4).op(op::I32_ADD).g(src).op(op::I32_ADD);
        fb.g(slen).g(src).op(op::I32_SUB);
        fb.memory_copy();
        fb.g(out);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `StrSplit(s, delim) -> List[Str]`.
    fn h_str_split(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(2);
        let list = fb.scratch(Val::I32);
        let len = fb.scratch(Val::I32);
        let start = fb.scratch(Val::I32);
        let i = fb.scratch(Val::I32);
        let piece = fb.scratch(Val::I32);
        macro_rules! push_piece {
            ($end:expr) => {{
                fb.g(0).i32c(4).op(op::I32_ADD).g(start).op(op::I32_ADD);
                fb.g($end).g(start).op(op::I32_SUB);
                fb.call(self.helper_index[&Helper::StrNew]).s(piece);
                fb.g(list).g(piece).op(op::I64_EXTEND_I32_U);
                fb.call(self.helper_index[&Helper::ListPush]);
                fb.g(piece).call(self.helper_index[&Helper::Release]);
            }};
        }
        fb.i32c(1).call(self.helper_index[&Helper::ListNew]).s(list);
        fb.g(0).load32(0).s(len);
        fb.g(len).op(op::I32_EQZ).if_().g(list).op(op::RETURN).end();
        fb.i32c(0).s(start);
        fb.i32c(0).s(i);
        fb.block();
        fb.loop_();
        fb.g(i).g(len).op(op::I32_GE_U).br_if(1);
        fb.g(0).i32c(4).op(op::I32_ADD).g(i).op(op::I32_ADD).load8(0);
        fb.g(1).op(op::I32_WRAP_I64).op(op::I32_EQ).if_();
        push_piece!(i);
        fb.g(i).i32c(1).op(op::I32_ADD).s(start);
        fb.end();
        fb.g(i).i32c(1).op(op::I32_ADD).s(i);
        fb.br(0);
        fb.end();
        fb.end();
        push_piece!(len);
        fb.g(list);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `FloatToStr(x) -> str`: C `%g` formatting (6 significant digits,
    /// trailing zeros trimmed, `%e` outside exponent range [-4, 6)).
    fn h_float_to_str(&mut self) -> (Vec<Val>, Vec<u8>) {
        let nan = self.intern(b"nan");
        let inf = self.intern(b"inf");
        let ninf = self.intern(b"-inf");
        let mut fb = FB::new(1);
        let neg = fb.scratch(Val::I32);
        let e = fb.scratch(Val::I32);
        let n = fb.scratch(Val::I64);
        let k = fb.scratch(Val::I32);
        let cur = fb.scratch(Val::I32);
        let intd = fb.scratch(Val::I32);
        let cnt = fb.scratch(Val::I32);
        let zc = fb.scratch(Val::I32);
        let ae = fb.scratch(Val::I64);
        // Return NUM_BUF[0..cur] as a string.
        macro_rules! finish {
            () => {{
                fb.i32c(NUM_BUF as i64);
                fb.g(cur).i32c(NUM_BUF as i64).op(op::I32_SUB);
                fb.call(self.helper_index[&Helper::StrNew]);
            }};
        }
        // Copy `cnt` digit bytes: `[dst, src]` are already on the stack.
        macro_rules! copy_digits_dst {
            () => {{
                fb.g(cnt).memory_copy();
                fb.g(cur).g(cnt).op(op::I32_ADD).s(cur);
            }};
        }
        // Write a run of `cnt` zero bytes at the cursor.
        macro_rules! zero_run {
            () => {{
                fb.i32c(0).s(zc);
                fb.block();
                fb.loop_();
                fb.g(zc).g(cnt).op(op::I32_GE_S).br_if(1);
                fb.g(cur).i32c(48).store8(0);
                fb.g(cur).i32c(1).op(op::I32_ADD).s(cur);
                fb.g(zc).i32c(1).op(op::I32_ADD).s(zc);
                fb.br(0);
                fb.end();
                fb.end();
            }};
        }
        // NaN / ±inf first (native prints words, no digits).
        fb.g(0).g(0).op(op::F64_NE).if_();
        fb.i32c(nan as i64).op(op::RETURN).end();
        fb.g(0).f64c(0.0).op(op::F64_MUL).f64c(0.0).op(op::F64_NE).if_();
        fb.g(0).f64c(0.0).op(op::F64_LT).if_();
        fb.i32c(ninf as i64).op(op::RETURN);
        fb.else_();
        fb.i32c(inf as i64).op(op::RETURN);
        fb.end();
        // Close the ±inf guard as well: both arms above return, so the
        // position here is formally reachable and the normal formatting path
        // must live at function depth, not inside the guard frame.
        fb.end();
        // Sign from the bit pattern so -0.0 prints "-0" like printf.
        fb.g(0).op(op::I64_REINTERPRET_F64).i64c(0).op(op::I64_LT_S).s(neg);
        fb.g(0).op(op::F64_ABS).s(0);
        // Zero: "0" (with sign).
        fb.g(0).f64c(0.0).op(op::F64_EQ).if_();
        fb.i32c(NUM_BUF as i64).s(cur);
        fb.g(neg).if_();
        fb.g(cur).i32c(45).store8(0);
        fb.g(cur).i32c(1).op(op::I32_ADD).s(cur);
        fb.end();
        fb.g(cur).i32c(48).store8(0);
        fb.g(cur).i32c(1).op(op::I32_ADD).s(cur);
        finish!();
        fb.op(op::RETURN);
        fb.end();
        // Normalize x into [1, 10) tracking the decimal exponent.
        fb.i32c(0).s(e);
        fb.block();
        fb.loop_();
        fb.g(0).f64c(10.0).op(op::F64_LT).br_if(1);
        fb.g(0).f64c(10.0).op(op::F64_DIV).s(0);
        fb.g(e).i32c(1).op(op::I32_ADD).s(e);
        fb.br(0);
        fb.end();
        fb.end();
        fb.block();
        fb.loop_();
        fb.g(0).f64c(1.0).op(op::F64_GE).br_if(1);
        fb.g(0).f64c(10.0).op(op::F64_MUL).s(0);
        fb.g(e).i32c(1).op(op::I32_SUB).s(e);
        fb.br(0);
        fb.end();
        fb.end();
        // Six rounded digits: n = u64(x * 1e5 + 0.5), carry hop to n=100000.
        fb.g(0).f64c(100_000.0).op(op::F64_MUL).f64c(0.5).op(op::F64_ADD);
        fb.op(op::I64_TRUNC_F64_U).s(n);
        fb.g(n).i64c(1_000_000).op(op::I64_GE_U).if_();
        fb.g(n).i64c(10).op(op::I64_DIV_U).s(n);
        fb.g(e).i32c(1).op(op::I32_ADD).s(e);
        fb.end();
        // Digit bytes land at NUM_BUF + 58..NUM_BUF + 64 (exactly 6).
        fb.g(n).call(self.helper_index[&Helper::FmtU64]).op(op::DROP);
        // k = index of the last nonzero digit (0-based).
        fb.i32c(5).s(k);
        fb.block();
        fb.loop_();
        fb.g(k).op(op::I32_EQZ).br_if(1);
        fb.i32c((NUM_BUF + 58) as i64).g(k).op(op::I32_ADD).load8(0);
        fb.i32c(48).op(op::I32_NE).br_if(1);
        fb.g(k).i32c(1).op(op::I32_SUB).s(k);
        fb.br(0);
        fb.end();
        fb.end();
        // Assemble at NUM_BUF head.
        fb.i32c(NUM_BUF as i64).s(cur);
        fb.g(neg).if_();
        fb.g(cur).i32c(45).store8(0);
        fb.g(cur).i32c(1).op(op::I32_ADD).s(cur);
        fb.end();
        // %f branch when -4 <= e < 6, else %e.
        fb.g(e).i32c(-4).op(op::I32_GE_S);
        fb.g(e).i32c(6).op(op::I32_LT_S);
        fb.op(op::I32_AND);
        fb.if_();
        // intd = e + 1
        fb.g(e).i32c(1).op(op::I32_ADD).s(intd);
        fb.g(intd).i32c(0).op(op::I32_LE_S).if_();
        // "0." + (-intd) zeros + digits[0..=k]
        fb.g(cur).i32c(48).store8(0);
        fb.g(cur).i32c(1).op(op::I32_ADD).s(cur);
        fb.g(cur).i32c(46).store8(0);
        fb.g(cur).i32c(1).op(op::I32_ADD).s(cur);
        fb.i32c(0).g(intd).op(op::I32_SUB).s(cnt);
        zero_run!();
        fb.g(k).i32c(1).op(op::I32_ADD).s(cnt);
        fb.g(cur);
        fb.i32c((NUM_BUF + 58) as i64);
        copy_digits_dst!();
        fb.else_();
        fb.g(intd).i32c(6).op(op::I32_GE_S).if_();
        // All 6 digits, then intd-6 zeros, no fraction.
        fb.i32c(6).s(cnt);
        fb.g(cur);
        fb.i32c((NUM_BUF + 58) as i64);
        copy_digits_dst!();
        fb.g(intd).i32c(6).op(op::I32_SUB).s(cnt);
        zero_run!();
        fb.else_();
        // 1..5 integer digits, then optional nonzero fraction.
        fb.g(intd).s(cnt);
        fb.g(cur);
        fb.i32c((NUM_BUF + 58) as i64);
        copy_digits_dst!();
        fb.g(k).g(intd).op(op::I32_GE_S).if_();
        fb.g(cur).i32c(46).store8(0);
        fb.g(cur).i32c(1).op(op::I32_ADD).s(cur);
        fb.g(k).g(intd).op(op::I32_SUB).i32c(1).op(op::I32_ADD).s(cnt);
        fb.g(cur);
        fb.i32c((NUM_BUF + 58) as i64).g(intd).op(op::I32_ADD);
        copy_digits_dst!();
        fb.end();
        fb.end();
        fb.end();
        fb.else_();
        // %e branch: d1 [ '.' rest ] 'e' sign exp2
        fb.i32c(1).s(cnt);
        fb.g(cur);
        fb.i32c((NUM_BUF + 58) as i64);
        copy_digits_dst!();
        fb.g(k).i32c(0).op(op::I32_GT_S).if_();
        fb.g(cur).i32c(46).store8(0);
        fb.g(cur).i32c(1).op(op::I32_ADD).s(cur);
        fb.g(k).s(cnt);
        fb.g(cur);
        fb.i32c((NUM_BUF + 59) as i64);
        copy_digits_dst!();
        fb.end();
        fb.g(cur).i32c(101).store8(0); // 'e'
        fb.g(cur).i32c(1).op(op::I32_ADD).s(cur);
        fb.g(e).op(op::I64_EXTEND_I32_S).s(ae);
        fb.g(ae).i64c(0).op(op::I64_LT_S).if_();
        fb.g(cur).i32c(45).store8(0); // '-'
        fb.i64c(0).g(ae).op(op::I64_SUB).s(ae);
        fb.else_();
        fb.g(cur).i32c(43).store8(0); // '+'
        fb.end();
        fb.g(cur).i32c(1).op(op::I32_ADD).s(cur);
        // Exponent digits (at least 2, more when >= 100).
        fb.g(ae).i64c(100).op(op::I64_GE_U).if_();
        fb.g(ae).call(self.helper_index[&Helper::FmtU64]).s(cnt);
        fb.g(cur);
        fb.i32c((NUM_BUF + NUM_BUF_SIZE) as i64).g(cnt).op(op::I32_SUB);
        copy_digits_dst!();
        fb.else_();
        fb.g(cur);
        fb.g(ae).i64c(10).op(op::I64_DIV_U).op(op::I32_WRAP_I64).i32c(48).op(op::I32_ADD);
        fb.store8(0);
        fb.g(cur).i32c(1).op(op::I32_ADD).s(cur);
        fb.g(cur);
        fb.g(ae).i64c(10).op(op::I64_REM_U).op(op::I32_WRAP_I64).i32c(48).op(op::I32_ADD);
        fb.store8(0);
        fb.g(cur).i32c(1).op(op::I32_ADD).s(cur);
        fb.end();
        fb.end();
        finish!();
        fb.end();
        (fb.extras, fb.body)
    }


    // ── Lists ────────────────────────────────────────────────────────────
    // Payload: [data i64][len i64][cap i64][is_arc i64].

    /// `ListNew(is_arc) -> list`.
    fn h_list_new(&mut self) -> (Vec<Val>, Vec<u8>) {
        let seat = self.table.list_destroy;
        let mut fb = FB::new(1);
        let l = fb.scratch(Val::I32);
        fb.i32c(32).i32c(seat as i64).call(self.helper_index[&Helper::ArcAlloc]).s(l);
        fb.g(l).i64c(0).store64(0);
        fb.g(l).i64c(0).store64(8);
        fb.g(l).i64c(0).store64(16);
        fb.g(l).g(0).op(op::I64_EXTEND_I32_U).store64(24);
        fb.g(l);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `ListLen(l) -> i64`.
    fn h_list_len(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        fb.g(0).op(op::I32_EQZ).if_().i64c(0).op(op::RETURN).end();
        fb.g(0).load64(8);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `ListPush(l, v)`: grow ×2 from 8 with a copied bump "realloc".
    fn h_list_push(&mut self) -> (Vec<Val>, Vec<u8>) {
        let null_msg = self.intern(b"push attempted on null list pointer");
        let cap_msg = self.intern(b"list capacity overflow");
        let mut fb = FB::new(2);
        let len = fb.scratch(Val::I64);
        let cap = fb.scratch(Val::I64);
        let data = fb.scratch(Val::I32);
        let newdata = fb.scratch(Val::I32);
        let newcap = fb.scratch(Val::I64);
        fb.g(0).op(op::I32_EQZ).if_();
        fb.i32c(null_msg as i64).call(self.helper_index[&Helper::PanicMsg]);
        fb.end();
        fb.g(0).load64(8).s(len);
        fb.g(0).load64(16).s(cap);
        fb.g(0).load64(0).op(op::I32_WRAP_I64).s(data);
        fb.g(len).g(cap).op(op::I64_EQ).if_();
        // newcap = cap == 0 ? 8 : cap * 2
        fb.i64c(8).g(cap).i64c(2).op(op::I64_MUL).g(cap).op(op::I64_EQZ).op(op::SELECT);
        fb.s(newcap);
        fb.g(newcap).i64c(0x1000_0000).op(op::I64_GT_S).if_();
        fb.i32c(cap_msg as i64).call(self.helper_index[&Helper::PanicMsg]);
        fb.end();
        // newdata = Alloc(newcap * 8); copy oldcap * 8 bytes
        fb.g(newcap).i64c(8).op(op::I64_MUL).op(op::I32_WRAP_I64);
        fb.call(self.helper_index[&Helper::Alloc]).s(newdata);
        fb.g(newdata).g(data).g(cap).i64c(8).op(op::I64_MUL).op(op::I32_WRAP_I64);
        fb.memory_copy();
        fb.g(0).g(newdata).op(op::I64_EXTEND_I32_U).store64(0);
        fb.g(0).g(newcap).store64(16);
        fb.g(newdata).s(data);
        fb.end();
        // retain element for ARC lists
        fb.g(0).load64(24).op(op::I32_WRAP_I64).if_();
        fb.g(1).op(op::I32_WRAP_I64).call(self.helper_index[&Helper::Retain]);
        fb.end();
        // data[len] = v; len += 1
        fb.g(data).g(len).op(op::I32_WRAP_I64).i32c(8).op(op::I32_MUL).op(op::I32_ADD);
        fb.g(1).store64(0);
        fb.g(0).g(len).i64c(1).op(op::I64_ADD).store64(8);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `ListGet(l, idx) -> i64` with bounds panic.
    fn h_list_get(&mut self) -> (Vec<Val>, Vec<u8>) {
        let null_msg = self.intern(b"list index access attempted on null list pointer");
        let pre = self.intern(b"list index out of bounds: index ");
        let mid = self.intern(b", len ");
        let mut fb = FB::new(2);
        fb.g(0).op(op::I32_EQZ).if_();
        fb.i32c(null_msg as i64).call(self.helper_index[&Helper::PanicMsg]);
        fb.end();
        fb.g(1).i64c(0).op(op::I64_LT_S);
        fb.g(1).g(0).load64(8).op(op::I64_GE_S);
        fb.op(op::I32_OR).if_();
        fb.i32c(pre as i64).g(1).i32c(mid as i64).g(0).load64(8);
        fb.call(self.helper_index[&Helper::Panic2]);
        fb.end();
        fb.g(0).load64(0).op(op::I32_WRAP_I64);
        fb.g(1).op(op::I32_WRAP_I64).i32c(8).op(op::I32_MUL).op(op::I32_ADD);
        fb.load64(0);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `ListSet(l, idx, v)`: retain incoming before dropping the old edge.
    fn h_list_set(&mut self) -> (Vec<Val>, Vec<u8>) {
        let null_msg = self.intern(b"list set attempted on null list pointer");
        let pre = self.intern(b"list index out of bounds on set: index ");
        let mid = self.intern(b", len ");
        let mut fb = FB::new(3);
        let slot = fb.scratch(Val::I32);
        fb.g(0).op(op::I32_EQZ).if_();
        fb.i32c(null_msg as i64).call(self.helper_index[&Helper::PanicMsg]);
        fb.end();
        fb.g(1).i64c(0).op(op::I64_LT_S);
        fb.g(1).g(0).load64(8).op(op::I64_GE_S);
        fb.op(op::I32_OR).if_();
        fb.i32c(pre as i64).g(1).i32c(mid as i64).g(0).load64(8);
        fb.call(self.helper_index[&Helper::Panic2]);
        fb.end();
        fb.g(0).load64(0).op(op::I32_WRAP_I64);
        fb.g(1).op(op::I32_WRAP_I64).i32c(8).op(op::I32_MUL).op(op::I32_ADD);
        fb.s(slot);
        fb.g(0).load64(24).op(op::I32_WRAP_I64).if_();
        fb.g(2).op(op::I32_WRAP_I64).call(self.helper_index[&Helper::Retain]);
        fb.g(slot).load64(0).op(op::I32_WRAP_I64).call(self.helper_index[&Helper::Release]);
        fb.end();
        fb.g(slot).g(2).store64(0);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `ListDestroy(p)`: release ARC elements; memory is left to the arena.
    fn h_list_destroy(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        let data = fb.scratch(Val::I32);
        let len = fb.scratch(Val::I64);
        let i = fb.scratch(Val::I32);
        fb.g(0).load64(24).op(op::I64_EQZ).if_().op(op::RETURN).end();
        fb.g(0).load64(0).op(op::I32_WRAP_I64).s(data);
        fb.g(0).load64(8).s(len);
        fb.i32c(0).s(i);
        fb.block();
        fb.loop_();
        fb.g(i).op(op::I64_EXTEND_I32_U).g(len).op(op::I64_GE_S).br_if(1);
        fb.g(data).g(i).i32c(8).op(op::I32_MUL).op(op::I32_ADD).load64(0);
        fb.op(op::I32_WRAP_I64).call(self.helper_index[&Helper::Release]);
        fb.g(i).i32c(1).op(op::I32_ADD).s(i);
        fb.br(0);
        fb.end();
        fb.end();
        fb.end();
        (fb.extras, fb.body)
    }

    // ── Maps ─────────────────────────────────────────────────────────────
    // Payload: [entries i64][cap i64][len i64][arc i64]; entries are 32-byte
    // cells [key i64][val i64][flags i64], flags = occupied(0/1/2) + is_str*4.

    /// `MapNew(arc) -> map`.
    fn h_map_new(&mut self) -> (Vec<Val>, Vec<u8>) {
        let seat = self.table.map_destroy;
        let mut fb = FB::new(1);
        let m = fb.scratch(Val::I32);
        fb.i32c(32).i32c(seat as i64).call(self.helper_index[&Helper::ArcAlloc]).s(m);
        fb.g(m).i32c(16 * 32).call(self.helper_index[&Helper::Alloc]);
        fb.op(op::I64_EXTEND_I32_U).store64(0);
        fb.g(m).i64c(16).store64(8);
        fb.g(m).i64c(0).store64(16);
        fb.g(m).g(0).op(op::I64_EXTEND_I32_U).store64(24);
        fb.g(m);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `MapLen(m) -> i64`.
    fn h_map_len(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        fb.g(0).op(op::I32_EQZ).if_().i64c(0).op(op::RETURN).end();
        fb.g(0).load64(16);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `HashStr(s) -> i64`: FNV-1a 64 over the string bytes.
    fn h_hash_str(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        let h = fb.scratch(Val::I64);
        let i = fb.scratch(Val::I32);
        fb.i64c(14695981039346656037u64 as i64).s(h);
        fb.i32c(0).s(i);
        fb.block();
        fb.loop_();
        fb.g(i).g(0).load32(0).op(op::I32_GE_U).br_if(1);
        fb.g(h);
        fb.g(0).i32c(4).op(op::I32_ADD).g(i).op(op::I32_ADD).load8(0);
        fb.op(op::I64_EXTEND_I32_U).op(op::I64_XOR);
        fb.i64c(1099511628211).op(op::I64_MUL).s(h);
        fb.g(i).i32c(1).op(op::I32_ADD).s(i);
        fb.br(0);
        fb.end();
        fb.end();
        fb.g(h);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `HashInt(k) -> i64`: the runtime's integer finalizer.
    fn h_hash_int(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        macro_rules! shl {
            ($n:expr) => {{
                fb.i64c($n).op(op::I64_SHL)
            }};
        }
        macro_rules! shru {
            ($n:expr) => {{
                fb.i64c($n).op(op::I64_SHR_U)
            }};
        }
        // k = (~k) + (k << 21)
        fb.g(0).i64c(-1).op(op::I64_XOR);
        fb.g(0);
        shl!(21);
        fb.op(op::I64_ADD).s(0);
        // k ^= k >> 24
        fb.g(0);
        fb.g(0);
        shru!(24);
        fb.op(op::I64_XOR).s(0);
        // k = (k + (k<<3)) + (k<<8)
        fb.g(0);
        fb.g(0);
        shl!(3);
        fb.op(op::I64_ADD);
        fb.g(0);
        shl!(8);
        fb.op(op::I64_ADD).s(0);
        // k ^= k >> 14
        fb.g(0);
        fb.g(0);
        shru!(14);
        fb.op(op::I64_XOR).s(0);
        // k = (k + (k<<2)) + (k<<4)
        fb.g(0);
        fb.g(0);
        shl!(2);
        fb.op(op::I64_ADD);
        fb.g(0);
        shl!(4);
        fb.op(op::I64_ADD).s(0);
        // k ^= k >> 28
        fb.g(0);
        fb.g(0);
        shru!(28);
        fb.op(op::I64_XOR).s(0);
        // k += k << 31
        fb.g(0);
        fb.g(0);
        shl!(31);
        fb.op(op::I64_ADD).s(0);
        fb.g(0);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `MapProbe(m, key, is_str) -> slot*2 + found`: linear-probe lookup from
    /// the entry's hash; stops at the first empty cell, remembering the first
    /// tombstone for the insertion path.
    fn h_map_probe(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(3);
        let h = fb.scratch(Val::I64);
        let cap = fb.scratch(Val::I64);
        let idx = fb.scratch(Val::I64);
        let start = fb.scratch(Val::I64);
        let tomb = fb.scratch(Val::I64);
        let f = fb.scratch(Val::I64);
        let addr = fb.scratch(Val::I32);
        let km = fb.scratch(Val::I32);
        // h = hash(key)
        fb.g(2).op(op::I64_EQZ).if_();
        fb.g(1).call(self.helper_index[&Helper::HashInt]).s(h);
        fb.else_();
        fb.g(1).op(op::I32_WRAP_I64).call(self.helper_index[&Helper::HashStr]).s(h);
        fb.end();
        fb.g(0).load64(8).s(cap);
        fb.g(h).g(cap).op(op::I64_REM_U).t(idx).s(start);
        fb.i64c(-1).s(tomb);
        fb.block(); // $stop — br 1 from loop level
        fb.loop_();
        // addr = entries + idx*32; f = flags
        fb.g(0).load64(0).op(op::I32_WRAP_I64);
        fb.g(idx).op(op::I32_WRAP_I64).i32c(32).op(op::I32_MUL).op(op::I32_ADD).t(addr);
        fb.load64(16).s(f);
        // occupied == 0 → stop
        fb.g(f).i64c(3).op(op::I64_AND).op(op::I64_EQZ).if_().br(2).end();
        fb.g(f).i64c(3).op(op::I64_AND).i64c(1).op(op::I64_EQ).if_();
        // key-kind must match: (f & 4 == 0) == (is_str == 0)
        fb.g(f).i64c(4).op(op::I64_AND).op(op::I64_EQZ);
        fb.g(2).op(op::I64_EQZ);
        fb.op(op::I32_EQ);
        fb.if_();
        // key match? (compared through km: the void if/else must stay
        // stack-balanced)
        fb.g(2).op(op::I64_EQZ).if_();
        fb.g(addr).load64(0).g(1).op(op::I64_EQ).s(km);
        fb.else_();
        fb.g(addr).load64(0).op(op::I32_WRAP_I64);
        fb.g(1).op(op::I32_WRAP_I64);
        fb.call(self.helper_index[&Helper::StrEq]).s(km);
        fb.end();
        fb.g(km).if_();
        fb.g(idx).i64c(2).op(op::I64_MUL).i64c(1).op(op::I64_OR);
        fb.op(op::RETURN);
        fb.end();
        fb.end();
        fb.else_();
        // tombstone: remember the first
        fb.g(tomb).i64c(-1).op(op::I64_EQ).if_();
        fb.g(idx).s(tomb);
        fb.end();
        fb.end();
        // idx = (idx + 1) % cap; wrapped fully → stop
        fb.g(idx).i64c(1).op(op::I64_ADD).g(cap).op(op::I64_REM_U).s(idx);
        fb.g(idx).g(start).op(op::I64_EQ).if_().br(2).end();
        fb.br(0);
        fb.end();
        fb.end();
        // insertion point: tombstone else the empty slot
        fb.g(tomb).g(idx).g(tomb).i64c(-1).op(op::I64_NE).op(op::SELECT);
        fb.i64c(2).op(op::I64_MUL);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `Panic3(pre, a, mid1, b, mid2, c)`: like `Panic2` but with three
    /// numeric fields (used by the slice range diagnostic).
    fn h_panic3(&mut self) -> (Vec<Val>, Vec<u8>) {
        let nl = self.lit_addr(b"\n");
        let dash = self.lit_addr(b"-");
        let mut fb = FB::new(6);
        let cnt = fb.scratch(Val::I32);
        let nm = fb.scratch(Val::I64);
        // See Panic2's note: branch arms balance through the local, never
        // through the validation stack.
        macro_rules! num {
            ($v:expr) => {{
                fb.g($v).i64c(0).op(op::I64_LT_S).if_();
                fb.i32c(dash as i64).i32c(1).i32c(2).call(self.helper_index[&Helper::WriteFd]);
                fb.i64c(0).g($v).op(op::I64_SUB).s(nm);
                fb.else_();
                fb.g($v).s(nm);
                fb.end();
                fb.g(nm).call(self.helper_index[&Helper::FmtU64]).s(cnt);
                fb.i32c((NUM_BUF + NUM_BUF_SIZE) as i64).g(cnt).op(op::I32_SUB);
                fb.g(cnt).i32c(2).call(self.helper_index[&Helper::WriteFd]);
            }};
        }
        macro_rules! lit {
            ($v:expr) => {{
                fb.g($v).i32c(4).op(op::I32_ADD).g($v).load32(0).i32c(2);
                fb.call(self.helper_index[&Helper::WriteFd]);
            }};
        }
        lit!(0);
        num!(1);
        lit!(2);
        num!(3);
        lit!(4);
        num!(5);
        fb.i32c(nl as i64).i32c(1).i32c(2).call(self.helper_index[&Helper::WriteFd]);
        fb.op(op::UNREACHABLE);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `MapRehash(m, new_cap)`: reinsert every occupied cell into a fresh
    /// zeroed table (parity with `runtime/lpp_map.c`).
    fn h_map_rehash(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(2);
        let old = fb.scratch(Val::I32);
        let oldcap = fb.scratch(Val::I64);
        let ne = fb.scratch(Val::I32);
        let i = fb.scratch(Val::I64);
        let idx = fb.scratch(Val::I64);
        let addr = fb.scratch(Val::I32);
        let ea = fb.scratch(Val::I32);
        let h64 = fb.scratch(Val::I64);
        fb.g(0).load64(0).op(op::I32_WRAP_I64).s(old);
        fb.g(0).load64(8).s(oldcap);
        fb.g(1).op(op::I32_WRAP_I64).i32c(32).op(op::I32_MUL);
        fb.call(self.helper_index[&Helper::Alloc]).s(ne);
        fb.g(0).g(ne).op(op::I64_EXTEND_I32_U).store64(0);
        fb.g(0).g(1).store64(8);
        fb.g(0).i64c(0).store64(16);
        fb.i64c(0).s(i);
        fb.block();
        fb.loop_();
        fb.g(i).g(oldcap).op(op::I64_GE_S).br_if(1);
        fb.g(old).g(i).op(op::I32_WRAP_I64).i32c(32).op(op::I32_MUL).op(op::I32_ADD).t(ea);
        fb.load64(16).i64c(3).op(op::I64_AND).i64c(1).op(op::I64_EQ).if_();
        // Re-hash the entry key into the new capacity (balanced through
        // h64: the void if/else must not leave a value).
        fb.g(ea).load64(16).i64c(4).op(op::I64_AND).op(op::I64_EQZ).if_();
        fb.g(ea).load64(0).call(self.helper_index[&Helper::HashInt]).s(h64);
        fb.else_();
        fb.g(ea).load64(0).op(op::I32_WRAP_I64).call(self.helper_index[&Helper::HashStr]).s(h64);
        fb.end();
        fb.g(h64).g(1).op(op::I64_REM_U).s(idx);
        // First non-occupied slot (no tombstones exist in a fresh table).
        fb.block();
        fb.loop_();
        fb.g(ne).g(idx).op(op::I32_WRAP_I64).i32c(32).op(op::I32_MUL).op(op::I32_ADD).t(addr);
        fb.load64(16).i64c(3).op(op::I64_AND).i64c(1).op(op::I64_NE).br_if(1);
        fb.g(idx).i64c(1).op(op::I64_ADD).g(1).op(op::I64_REM_U).s(idx);
        fb.br(0);
        fb.end();
        fb.end();
        // Move key/val/flags (flags keeps the is_str bit, occupied = 1).
        fb.g(addr).g(ea).load64(0).store64(0);
        fb.g(addr).g(ea).load64(8).store64(8);
        fb.g(addr).i64c(1).g(ea).load64(16).i64c(4).op(op::I64_AND).op(op::I64_OR).store64(16);
        fb.g(0).g(0).load64(16).i64c(1).op(op::I64_ADD).store64(16);
        fb.end();
        fb.g(i).i64c(1).op(op::I64_ADD).s(i);
        fb.br(0);
        fb.end();
        fb.end();
        fb.end();
        (fb.extras, fb.body)
    }

    /// `MapPut(m, key, val, is_str)`: grow at 70% occupancy, probe, then
    /// overwrite or insert (ARC values are retained; overwritten/replaced
    /// values released — identical edge semantics to the native map).
    fn h_map_put(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(4);
        let occ = fb.scratch(Val::I64);
        let i = fb.scratch(Val::I64);
        let addr = fb.scratch(Val::I32);
        let ncap = fb.scratch(Val::I64);
        let probe = fb.scratch(Val::I64);
        // Count non-empty cells (occupied or tombstone): flags & 3 != 0 ⟺ flags != 0.
        fb.i64c(0).s(occ);
        fb.i64c(0).s(i);
        fb.block();
        fb.loop_();
        fb.g(i).g(0).load64(8).op(op::I64_GE_S).br_if(1);
        fb.g(0).load64(0).op(op::I32_WRAP_I64);
        fb.g(i).op(op::I32_WRAP_I64).i32c(32).op(op::I32_MUL).op(op::I32_ADD);
        fb.load64(16).op(op::I64_EQZ).op(op::I32_EQZ);
        fb.op(op::I64_EXTEND_I32_U).g(occ).op(op::I64_ADD).s(occ);
        fb.g(i).i64c(1).op(op::I64_ADD).s(i);
        fb.br(0);
        fb.end();
        fb.end();
        // if occ * 10 >= cap * 7: rehash
        fb.g(occ).i64c(10).op(op::I64_MUL);
        fb.g(0).load64(8).i64c(7).op(op::I64_MUL);
        fb.op(op::I64_GE_S).if_();
        // new_cap = (len * 100 < cap * 35) ? cap : cap * 2, floor 16
        fb.g(0).load64(8);
        fb.g(0).load64(8).i64c(2).op(op::I64_MUL);
        fb.g(0).load64(16).i64c(100).op(op::I64_MUL);
        fb.g(0).load64(8).i64c(35).op(op::I64_MUL);
        fb.op(op::I64_LT_S).op(op::SELECT).s(ncap);
        fb.g(ncap).i64c(16).op(op::I64_LT_S).if_();
        fb.i64c(16).s(ncap);
        fb.end();
        fb.g(0).g(ncap).call(self.helper_index[&Helper::MapRehash]);
        fb.end();
        // probe returns slot*2+found
        fb.g(0).g(1).g(3).call(self.helper_index[&Helper::MapProbe]).s(probe);
        fb.g(probe).i64c(1).op(op::I64_AND).op(op::I32_WRAP_I64).if_();
        // Overwrite: retain the new edge before dropping the old one.
        fb.g(0).load64(0).op(op::I32_WRAP_I64);
        fb.g(probe).i64c(1).op(op::I64_SHR_U).op(op::I32_WRAP_I64).i32c(32).op(op::I32_MUL);
        fb.op(op::I32_ADD).s(addr);
        fb.g(0).load64(24).op(op::I32_WRAP_I64).if_();
        fb.g(2).op(op::I32_WRAP_I64).call(self.helper_index[&Helper::Retain]);
        fb.g(addr).load64(8).op(op::I32_WRAP_I64).call(self.helper_index[&Helper::Release]);
        fb.end();
        fb.g(addr).g(2).store64(8);
        fb.op(op::RETURN);
        fb.end();
        // Insert at the probed slot.
        fb.g(0).load64(0).op(op::I32_WRAP_I64);
        fb.g(probe).i64c(1).op(op::I64_SHR_U).op(op::I32_WRAP_I64).i32c(32).op(op::I32_MUL);
        fb.op(op::I32_ADD).s(addr);
        fb.g(0).load64(24).op(op::I32_WRAP_I64).if_();
        fb.g(2).op(op::I32_WRAP_I64).call(self.helper_index[&Helper::Retain]);
        fb.end();
        fb.g(addr).g(1).store64(0);
        fb.g(addr).g(2).store64(8);
        fb.g(addr).i64c(1).g(3).i64c(4).op(op::I64_MUL).op(op::I64_OR).store64(16);
        fb.g(0).g(0).load64(16).i64c(1).op(op::I64_ADD).store64(16);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `MapGet(m, key, is_str) -> val | 0`.
    fn h_map_get(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(3);
        let probe = fb.scratch(Val::I64);
        fb.g(0).op(op::I32_EQZ).if_().i64c(0).op(op::RETURN).end();
        fb.g(0).load64(16).op(op::I64_EQZ).if_().i64c(0).op(op::RETURN).end();
        fb.g(0).g(1).g(2).call(self.helper_index[&Helper::MapProbe]).s(probe);
        fb.g(probe).i64c(1).op(op::I64_AND).op(op::I32_WRAP_I64).if_();
        fb.g(0).load64(0).op(op::I32_WRAP_I64);
        fb.g(probe).i64c(1).op(op::I64_SHR_U).op(op::I32_WRAP_I64).i32c(32).op(op::I32_MUL);
        fb.op(op::I32_ADD).load64(8);
        fb.op(op::RETURN);
        fb.end();
        fb.i64c(0);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `MapHas(m, key, is_str) -> 0/1`.
    fn h_map_has(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(3);
        fb.g(0).op(op::I32_EQZ).if_().i64c(0).op(op::RETURN).end();
        fb.g(0).load64(16).op(op::I64_EQZ).if_().i64c(0).op(op::RETURN).end();
        fb.g(0).g(1).g(2).call(self.helper_index[&Helper::MapProbe]);
        fb.i64c(1).op(op::I64_AND);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `MapRemove(m, key, is_str)`: tombstone + release an ARC value.
    fn h_map_remove(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(3);
        let probe = fb.scratch(Val::I64);
        let addr = fb.scratch(Val::I32);
        fb.g(0).op(op::I32_EQZ).if_().op(op::RETURN).end();
        fb.g(0).load64(16).op(op::I64_EQZ).if_().op(op::RETURN).end();
        fb.g(0).g(1).g(2).call(self.helper_index[&Helper::MapProbe]).s(probe);
        fb.g(probe).i64c(1).op(op::I64_AND).op(op::I32_WRAP_I64).if_();
        fb.g(0).load64(0).op(op::I32_WRAP_I64);
        fb.g(probe).i64c(1).op(op::I64_SHR_U).op(op::I32_WRAP_I64).i32c(32).op(op::I32_MUL);
        fb.op(op::I32_ADD).s(addr);
        fb.g(0).load64(24).op(op::I32_WRAP_I64).if_();
        fb.g(addr).load64(8).op(op::I32_WRAP_I64).call(self.helper_index[&Helper::Release]);
        fb.end();
        fb.g(addr).i64c(2).store64(16);
        fb.g(0).g(0).load64(16).i64c(1).op(op::I64_SUB).store64(16);
        fb.end();
        fb.end();
        (fb.extras, fb.body)
    }

    /// `MapDestroy(m)`: release every ARC value still stored.
    fn h_map_destroy(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        let i = fb.scratch(Val::I64);
        let addr = fb.scratch(Val::I32);
        fb.g(0).load64(24).op(op::I64_EQZ).if_().op(op::RETURN).end();
        fb.i64c(0).s(i);
        fb.block();
        fb.loop_();
        fb.g(i).g(0).load64(8).op(op::I64_GE_S).br_if(1);
        fb.g(0).load64(0).op(op::I32_WRAP_I64);
        fb.g(i).op(op::I32_WRAP_I64).i32c(32).op(op::I32_MUL).op(op::I32_ADD).t(addr);
        fb.load64(16).i64c(3).op(op::I64_AND).i64c(1).op(op::I64_EQ).if_();
        fb.g(addr).load64(8).op(op::I32_WRAP_I64).call(self.helper_index[&Helper::Release]);
        fb.end();
        fb.g(i).i64c(1).op(op::I64_ADD).s(i);
        fb.br(0);
        fb.end();
        fb.end();
        fb.end();
        (fb.extras, fb.body)
    }

    // ── Tuples / closures / tasks ────────────────────────────────────────

    /// `TupleAlloc(size, managed_mask, packed_offsets) -> tuple`: ARC block
    /// with the native ownership prefix at offsets 0 and 8.
    fn h_tuple_alloc(&mut self) -> (Vec<Val>, Vec<u8>) {
        let seat = self.table.tuple_destroy;
        let mut fb = FB::new(3);
        let p = fb.scratch(Val::I32);
        fb.g(0).i32c(seat as i64).call(self.helper_index[&Helper::ArcAlloc]).s(p);
        fb.g(p).g(1).store64(0);
        fb.g(p).g(2).store64(8);
        fb.g(p);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `TupleDestroy(p)`: release each managed child tracked by the prefix.
    fn h_tuple_destroy(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        let mask = fb.scratch(Val::I64);
        let offsets = fb.scratch(Val::I64);
        let i = fb.scratch(Val::I32);
        let off = fb.scratch(Val::I64);
        fb.g(0).load64(0).s(mask);
        fb.g(0).load64(8).s(offsets);
        fb.i32c(0).s(i);
        fb.block();
        fb.loop_();
        fb.g(i).i32c(4).op(op::I32_GE_S).br_if(1);
        fb.g(mask).g(i).op(op::I64_EXTEND_I32_U).op(op::I64_SHR_U).i64c(1).op(op::I64_AND).op(op::I32_WRAP_I64).if_();
        fb.g(offsets);
        fb.g(i).i32c(16).op(op::I32_MUL).op(op::I64_EXTEND_I32_U);
        fb.op(op::I64_SHR_U).i64c(0xffff).op(op::I64_AND).s(off);
        fb.g(0).g(off).op(op::I32_WRAP_I64).op(op::I32_ADD);
        fb.load64(0).op(op::I32_WRAP_I64).call(self.helper_index[&Helper::Release]);
        fb.end();
        fb.g(i).i32c(1).op(op::I32_ADD).s(i);
        fb.br(0);
        fb.end();
        fb.end();
        fb.end();
        (fb.extras, fb.body)
    }

    /// `ClosureDestroy(p)`: release the captured environment word.
    fn h_closure_destroy(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        fb.g(0).load64(8).op(op::I32_WRAP_I64).call(self.helper_index[&Helper::Release]);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `TaskNew(thunk_seat, env, result_managed) -> task`: the 40-byte
    /// payload `[seat][env][result][state][managed]`.
    fn h_task_new(&mut self) -> (Vec<Val>, Vec<u8>) {
        let msg = self.intern(b"task creation requires code and environment");
        let seat = self.table.task_destroy;
        let mut fb = FB::new(3);
        let t = fb.scratch(Val::I32);
        fb.g(0).op(op::I64_EQZ);
        fb.g(1).op(op::I32_EQZ).op(op::I32_OR).if_();
        fb.i32c(msg as i64).call(self.helper_index[&Helper::PanicMsg]);
        fb.end();
        fb.i32c(40).i32c(seat as i64).call(self.helper_index[&Helper::ArcAlloc]).s(t);
        fb.g(t).g(0).store64(0);
        fb.g(t).g(1).op(op::I64_EXTEND_I32_U).store64(8);
        fb.g(t).i64c(0).store64(16);
        fb.g(t).i64c(0).store64(24);
        fb.g(t).g(2).store64(32);
        fb.g(t);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `TaskRun(t) -> result`: deterministic run-to-completion, panicking on
    /// null/recursive polling and idempotent on completion (poll semantics).
    fn h_task_run(&mut self) -> (Vec<Val>, Vec<u8>) {
        let msg_null = self.intern(b"attempted to poll a null task");
        let msg_rec = self.intern(b"concurrent or recursive polling of the same task");
        let task_type = self.task_call_type();
        let mut fb = FB::new(1);
        let res = fb.scratch(Val::I64);
        fb.g(0).op(op::I32_EQZ).if_();
        fb.i32c(msg_null as i64).call(self.helper_index[&Helper::PanicMsg]);
        fb.end();
        fb.g(0).load64(24).i64c(2).op(op::I64_EQ).if_();
        fb.g(0).load64(16).op(op::RETURN);
        fb.end();
        fb.g(0).load64(24).op(op::I64_EQZ).if_().else_();
        fb.i32c(msg_rec as i64).call(self.helper_index[&Helper::PanicMsg]);
        fb.end();
        fb.g(0).i64c(1).store64(24);
        fb.g(0).load64(8).op(op::I32_WRAP_I64);
        fb.g(0).load64(0).op(op::I32_WRAP_I64);
        fb.call_indirect(task_type);
        fb.s(res);
        fb.g(0).g(res).store64(16);
        fb.g(0).i64c(2).store64(24);
        fb.g(res);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `TaskPoll(t) -> 1` (runs the task; result stays with the task).
    fn h_task_poll(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        fb.g(0).call(self.helper_index[&Helper::TaskRun]);
        fb.op(op::DROP);
        fb.i64c(1);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `TaskAwait(t) -> result`: run, then hand the caller a fresh reference
    /// to a managed result (double-await stays defined).
    fn h_task_await(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        let res = fb.scratch(Val::I64);
        fb.g(0).call(self.helper_index[&Helper::TaskRun]).s(res);
        fb.g(0).load64(32).op(op::I32_WRAP_I64).if_();
        fb.g(res).op(op::I32_WRAP_I64).call(self.helper_index[&Helper::Retain]);
        fb.end();
        fb.g(res);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `TaskDestroy(t)`: release the environment and a managed completed
    /// result (the ARC destructor table entry for tasks).
    fn h_task_destroy(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        fb.g(0).load64(8).op(op::I32_WRAP_I64).call(self.helper_index[&Helper::Release]);
        fb.g(0).load64(24).i64c(2).op(op::I64_EQ).if_();
        fb.g(0).load64(32).op(op::I32_WRAP_I64).if_();
        fb.g(0).load64(16).op(op::I32_WRAP_I64).call(self.helper_index[&Helper::Release]);
        fb.end();
        fb.end();
        fb.end();
        (fb.extras, fb.body)
    }

    // ── Slices ───────────────────────────────────────────────────────────
    // 32-byte heap views: [base i64][start i64][length i64][kind i64],
    // kind 0 = string bytes, 1 = list slots. The native weak-liveness check
    // has no wasm counterpart possible or needed: under the bump heap the
    // base object's bytes are never reclaimed, so the view can never dangle.

    /// `SliceNew(base, start, length, kind) -> view` with native validation.
    fn h_slice_new(&mut self) -> (Vec<Val>, Vec<u8>) {
        let msg_base = self.intern(b"slice construction requires live storage and base");
        let inv_pre = self.intern(b"invalid slice range: start ");
        let inv_mid = self.intern(b", len ");
        let oob_pre = self.intern(b"slice range out of bounds: start ");
        let oob_mid1 = self.intern(b", len ");
        let oob_mid2 = self.intern(b", source len ");
        let mut fb = FB::new(4);
        let srclen = fb.scratch(Val::I64);
        let v = fb.scratch(Val::I32);
        fb.g(0).op(op::I32_EQZ).if_();
        fb.i32c(msg_base as i64).call(self.helper_index[&Helper::PanicMsg]);
        fb.end();
        // start < 0 || length < 0 || start > i64::MAX - length
        fb.g(1).i64c(0).op(op::I64_LT_S);
        fb.g(2).i64c(0).op(op::I64_LT_S).op(op::I32_OR);
        fb.g(1).i64c(i64::MAX).g(2).op(op::I64_SUB).op(op::I64_GT_S).op(op::I32_OR).if_();
        fb.i32c(inv_pre as i64).g(1).i32c(inv_mid as i64).g(2);
        fb.call(self.helper_index[&Helper::Panic2]);
        fb.end();
        // source_len = kind == 0 ? str length : list length (balanced
        // through the local — the void if/else must not leave a value).
        fb.g(3).op(op::I64_EQZ).if_();
        fb.g(0).call(self.helper_index[&Helper::StrLen]).s(srclen);
        fb.else_();
        fb.g(0).call(self.helper_index[&Helper::ListLen]).s(srclen);
        fb.end();
        // start > source_len || length > source_len - start
        fb.g(1).g(srclen).op(op::I64_GT_S);
        fb.g(2).g(srclen).g(1).op(op::I64_SUB).op(op::I64_GT_S).op(op::I32_OR).if_();
        fb.i32c(oob_pre as i64).g(1).i32c(oob_mid1 as i64).g(2).i32c(oob_mid2 as i64).g(srclen);
        fb.call(self.helper_index[&Helper::Panic3]);
        fb.end();
        fb.i32c(32).call(self.helper_index[&Helper::Alloc]).s(v);
        fb.g(v).g(0).op(op::I64_EXTEND_I32_U).store64(0);
        fb.g(v).g(1).store64(8);
        fb.g(v).g(2).store64(16);
        fb.g(v).g(3).store64(24);
        fb.g(v);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `SliceLen(view) -> i64`.
    fn h_slice_len(&mut self) -> (Vec<Val>, Vec<u8>) {
        let msg = self.intern(b"use of an uninitialized slice view");
        let mut fb = FB::new(1);
        fb.g(0).op(op::I32_EQZ).if_();
        fb.i32c(msg as i64).call(self.helper_index[&Helper::PanicMsg]);
        fb.end();
        fb.g(0).load64(0).op(op::I32_WRAP_I64).op(op::I32_EQZ).if_();
        fb.i32c(msg as i64).call(self.helper_index[&Helper::PanicMsg]);
        fb.end();
        fb.g(0).load64(16);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `SliceGet(view, index) -> i64` (list kind only).
    fn h_slice_get(&mut self) -> (Vec<Val>, Vec<u8>) {
        let msg_view = self.intern(b"use of an uninitialized slice view");
        let msg_kind = self.intern(b"numeric slice_get requires Slice[T]");
        let pre = self.intern(b"slice index out of bounds: index ");
        let mid = self.intern(b", len ");
        let mut fb = FB::new(2);
        fb.g(0).op(op::I32_EQZ).if_();
        fb.i32c(msg_view as i64).call(self.helper_index[&Helper::PanicMsg]);
        fb.end();
        fb.g(0).load64(0).op(op::I32_WRAP_I64).op(op::I32_EQZ).if_();
        fb.i32c(msg_view as i64).call(self.helper_index[&Helper::PanicMsg]);
        fb.end();
        fb.g(1).i64c(0).op(op::I64_LT_S);
        fb.g(1).g(0).load64(16).op(op::I64_GE_S).op(op::I32_OR).if_();
        fb.i32c(pre as i64).g(1).i32c(mid as i64).g(0).load64(16);
        fb.call(self.helper_index[&Helper::Panic2]);
        fb.end();
        fb.g(0).load64(24).i64c(1).op(op::I64_NE).if_();
        fb.i32c(msg_kind as i64).call(self.helper_index[&Helper::PanicMsg]);
        fb.end();
        fb.g(0).load64(0).op(op::I32_WRAP_I64);
        fb.g(0).load64(8).g(1).op(op::I64_ADD);
        fb.call(self.helper_index[&Helper::ListGet]);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `StrSliceGet(view, index) -> str`: the native 1-char ARC string (kind
    /// check first, then bounds — matching `lpp_str_slice_get`).
    fn h_str_slice_get(&mut self) -> (Vec<Val>, Vec<u8>) {
        let msg_view = self.intern(b"use of an uninitialized slice view");
        let msg_kind = self.intern(b"string slice_get requires StrSlice");
        let pre = self.intern(b"string slice index out of bounds: index ");
        let mid = self.intern(b", len ");
        let mut fb = FB::new(2);
        let out = fb.scratch(Val::I32);
        fb.g(0).op(op::I32_EQZ).if_();
        fb.i32c(msg_view as i64).call(self.helper_index[&Helper::PanicMsg]);
        fb.end();
        fb.g(0).load64(0).op(op::I32_WRAP_I64).op(op::I32_EQZ).if_();
        fb.i32c(msg_view as i64).call(self.helper_index[&Helper::PanicMsg]);
        fb.end();
        fb.g(0).load64(24).op(op::I64_EQZ).if_().else_();
        fb.i32c(msg_kind as i64).call(self.helper_index[&Helper::PanicMsg]);
        fb.end();
        fb.g(1).i64c(0).op(op::I64_LT_S);
        fb.g(1).g(0).load64(16).op(op::I64_GE_S).op(op::I32_OR).if_();
        fb.i32c(pre as i64).g(1).i32c(mid as i64).g(0).load64(16);
        fb.call(self.helper_index[&Helper::Panic2]);
        fb.end();
        fb.i32c(1).call(self.helper_index[&Helper::StrAlloc]).s(out);
        fb.g(out).i32c(4).op(op::I32_ADD);
        fb.g(0).load64(0).op(op::I32_WRAP_I64).i32c(4).op(op::I32_ADD);
        fb.g(0).load64(8).g(1).op(op::I64_ADD).op(op::I32_WRAP_I64).op(op::I32_ADD);
        fb.load8(0);
        fb.store8(0);
        fb.g(out);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `StrSliceToStr(view) -> str`.
    fn h_str_slice_to_str(&mut self) -> (Vec<Val>, Vec<u8>) {
        let msg_view = self.intern(b"use of an uninitialized slice view");
        let msg_kind = self.intern(b"slice_to_str requires StrSlice");
        let mut fb = FB::new(1);
        fb.g(0).op(op::I32_EQZ).if_();
        fb.i32c(msg_view as i64).call(self.helper_index[&Helper::PanicMsg]);
        fb.end();
        fb.g(0).load64(0).op(op::I32_WRAP_I64).op(op::I32_EQZ).if_();
        fb.i32c(msg_view as i64).call(self.helper_index[&Helper::PanicMsg]);
        fb.end();
        fb.g(0).load64(24).op(op::I64_EQZ).if_().else_();
        fb.i32c(msg_kind as i64).call(self.helper_index[&Helper::PanicMsg]);
        fb.end();
        fb.g(0).load64(0).op(op::I32_WRAP_I64).i32c(4).op(op::I32_ADD);
        fb.g(0).load64(8).op(op::I32_WRAP_I64).op(op::I32_ADD);
        fb.g(0).load64(16).op(op::I32_WRAP_I64);
        fb.call(self.helper_index[&Helper::StrNew]);
        fb.end();
        (fb.extras, fb.body)
    }

    // ── Math, random, time ───────────────────────────────────────────────

    /// `IntPow(base, exp) -> i64`: the native binary exponentiation loop.
    fn h_int_pow(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(2);
        let r = fb.scratch(Val::I64);
        fb.i64c(1).s(r);
        fb.block();
        fb.loop_();
        fb.g(1).i64c(0).op(op::I64_LE_S).br_if(1);
        fb.g(1).i64c(1).op(op::I64_AND).op(op::I32_WRAP_I64).if_();
        fb.g(r).g(0).op(op::I64_MUL).s(r);
        fb.end();
        fb.g(0).g(0).op(op::I64_MUL).s(0);
        fb.g(1).i64c(1).op(op::I64_SHR_S).s(1);
        fb.br(0);
        fb.end();
        fb.end();
        fb.g(r);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `Log2(x) -> f64` for x > 0: binary exponent extraction plus an
    /// `atanh` series for the [1, 2) mantissa (error well under 1e-12).
    fn h_log2(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        let bits = fb.scratch(Val::I64);
        let e = fb.scratch(Val::I64);
        let m = fb.scratch(Val::F64);
        let t = fb.scratch(Val::F64);
        let t2 = fb.scratch(Val::F64);
        let term = fb.scratch(Val::F64);
        let sum = fb.scratch(Val::F64);
        let i = fb.scratch(Val::I32);
        fb.g(0).op(op::I64_REINTERPRET_F64).s(bits);
        fb.g(bits).i64c(52).op(op::I64_SHR_U).i64c(2047).op(op::I64_AND).i64c(1023).op(op::I64_SUB).s(e);
        // m = mantissa re-biased into [1, 2)
        fb.g(bits).i64c(0x800f_ffff_ffff_ffffu64 as i64).op(op::I64_AND);
        fb.i64c(1023).i64c(52).op(op::I64_SHL).op(op::I64_OR);
        fb.op(op::F64_REINTERPRET_I64).s(m);
        // t = (m-1)/(m+1); ln(m) = 2 * sum(t^k / k, k odd)
        fb.g(m).f64c(1.0).op(op::F64_SUB);
        fb.g(m).f64c(1.0).op(op::F64_ADD).op(op::F64_DIV).s(t);
        fb.g(t).g(t).op(op::F64_MUL).s(t2);
        fb.g(t).s(term);
        fb.g(t).s(sum);
        fb.i32c(3).s(i);
        fb.block();
        fb.loop_();
        fb.g(i).i32c(25).op(op::I32_GE_S).br_if(1);
        fb.g(term).g(t2).op(op::F64_MUL).s(term);
        fb.g(sum).g(term).g(i).op(op::F64_CONVERT_I32_S).op(op::F64_DIV).op(op::F64_ADD).s(sum);
        fb.g(i).i32c(2).op(op::I32_ADD).s(i);
        fb.br(0);
        fb.end();
        fb.end();
        // e + 2*sum/ln2
        fb.g(e).op(op::F64_CONVERT_I64_S);
        fb.g(sum).f64c(2.0).op(op::F64_MUL).f64c(0.6931471805599453).op(op::F64_DIV).op(op::F64_ADD);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `Exp2(y) -> f64`: floor split, `exp(f * ln2)` Taylor series, and an
    /// exact power-of-two bit scale.
    fn h_exp2(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        let n = fb.scratch(Val::I64);
        let f = fb.scratch(Val::F64);
        let u = fb.scratch(Val::F64);
        let term = fb.scratch(Val::F64);
        let sum = fb.scratch(Val::F64);
        let i = fb.scratch(Val::I32);
        fb.g(0).f64c(1024.0).op(op::F64_GE).if_();
        fb.i64c(0x7ff0_0000_0000_0000).op(op::F64_REINTERPRET_I64).op(op::RETURN);
        fb.end();
        fb.g(0).f64c(-1075.0).op(op::F64_LT).if_();
        fb.f64c(0.0).op(op::RETURN);
        fb.end();
        fb.g(0).op(op::F64_FLOOR).op(op::I64_TRUNC_F64_S).s(n);
        fb.g(0).g(n).op(op::F64_CONVERT_I64_S).op(op::F64_SUB).s(f);
        // clamp the integer part into the normal-exponent range
        fb.g(n).i64c(-1022).op(op::I64_LT_S).if_();
        fb.i64c(-1022).s(n);
        fb.end();
        fb.g(n).i64c(1023).op(op::I64_GT_S).if_();
        fb.i64c(1023).s(n);
        fb.end();
        // exp(f * ln2), 15 Taylor terms
        fb.g(f).f64c(0.6931471805599453).op(op::F64_MUL).s(u);
        fb.f64c(1.0).s(sum);
        fb.f64c(1.0).s(term);
        fb.i32c(1).s(i);
        fb.block();
        fb.loop_();
        fb.g(i).i32c(15).op(op::I32_GE_S).br_if(1);
        fb.g(term).g(u).op(op::F64_MUL).g(i).op(op::F64_CONVERT_I32_S).op(op::F64_DIV).s(term);
        fb.g(sum).g(term).op(op::F64_ADD).s(sum);
        fb.g(i).i32c(1).op(op::I32_ADD).s(i);
        fb.br(0);
        fb.end();
        fb.end();
        // * 2^n via exponent bits
        fb.g(sum);
        fb.g(n).i64c(1023).op(op::I64_ADD).i64c(52).op(op::I64_SHL).op(op::F64_REINTERPRET_I64);
        fb.op(op::F64_MUL);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `Pow(x, y) -> f64`: IEEE-ish special cases over `exp2(y * log2(x))`.
    fn h_pow(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(2);
        let ny = fb.scratch(Val::I64);
        let r = fb.scratch(Val::F64);
        fb.g(1).f64c(0.0).op(op::F64_EQ).if_();
        fb.f64c(1.0).op(op::RETURN);
        fb.end();
        fb.g(0).f64c(1.0).op(op::F64_EQ).if_();
        fb.f64c(1.0).op(op::RETURN);
        fb.end();
        fb.g(0).f64c(0.0).op(op::F64_EQ).if_();
        fb.g(1).f64c(0.0).op(op::F64_LT).if_();
        fb.i64c(0x7ff0_0000_0000_0000).op(op::F64_REINTERPRET_I64).op(op::RETURN);
        fb.end();
        fb.f64c(0.0).op(op::RETURN);
        fb.end();
        // x < 0: only integral exponents are defined; the sign follows parity.
        fb.g(0).f64c(0.0).op(op::F64_LT).if_();
        fb.g(1).op(op::F64_TRUNC).g(1).op(op::F64_NE).if_();
        fb.f64c(f64::NAN).op(op::RETURN);
        fb.end();
        fb.g(1).op(op::I64_TRUNC_F64_S).s(ny);
        fb.g(0).op(op::F64_NEG).call(self.helper_index[&Helper::Log2]);
        fb.g(1).op(op::F64_MUL).call(self.helper_index[&Helper::Exp2]).s(r);
        fb.g(ny).i64c(1).op(op::I64_AND).op(op::I32_WRAP_I64).if_();
        fb.g(r).op(op::F64_NEG).s(r);
        fb.end();
        fb.g(r).op(op::RETURN);
        fb.end();
        fb.g(0).call(self.helper_index[&Helper::Log2]);
        fb.g(1).op(op::F64_MUL).call(self.helper_index[&Helper::Exp2]);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `Reduce2Pi(x) -> f64`: `x mod 2pi` in [0, 2pi) via the floor form.
    fn h_reduce_2pi(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        fb.g(0);
        fb.g(0).f64c(6.283185307179586).op(op::F64_DIV).op(op::F64_FLOOR);
        fb.f64c(6.283185307179586).op(op::F64_MUL);
        fb.op(op::F64_SUB);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `SinPoly(y) -> f64` on [0, pi/2]: 8 sine series terms.
    fn h_sin_poly(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        let yy = fb.scratch(Val::F64);
        let t = fb.scratch(Val::F64);
        let s = fb.scratch(Val::F64);
        let k = fb.scratch(Val::I32);
        fb.g(0).g(0).op(op::F64_MUL).s(yy);
        fb.g(0).s(t);
        fb.g(0).s(s);
        fb.i32c(1).s(k);
        fb.block();
        fb.loop_();
        fb.g(k).i32c(9).op(op::I32_GE_S).br_if(1);
        // t *= -yy / ((2k) * (2k+1))
        fb.g(t).g(yy).op(op::F64_MUL).op(op::F64_NEG);
        fb.g(k).i32c(2).op(op::I32_MUL).op(op::F64_CONVERT_I32_S);
        fb.g(k).i32c(2).op(op::I32_MUL).i32c(1).op(op::I32_ADD).op(op::F64_CONVERT_I32_S);
        fb.op(op::F64_MUL).op(op::F64_DIV).s(t);
        fb.g(s).g(t).op(op::F64_ADD).s(s);
        fb.g(k).i32c(1).op(op::I32_ADD).s(k);
        fb.br(0);
        fb.end();
        fb.end();
        fb.g(s);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `CosPoly(y) -> f64` on [0, pi/2]: 9 cosine series terms.
    fn h_cos_poly(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        let yy = fb.scratch(Val::F64);
        let t = fb.scratch(Val::F64);
        let s = fb.scratch(Val::F64);
        let k = fb.scratch(Val::I32);
        fb.g(0).g(0).op(op::F64_MUL).s(yy);
        fb.f64c(1.0).s(t);
        fb.f64c(1.0).s(s);
        fb.i32c(1).s(k);
        fb.block();
        fb.loop_();
        fb.g(k).i32c(10).op(op::I32_GE_S).br_if(1);
        // t *= -yy / ((2k-1) * (2k))
        fb.g(t).g(yy).op(op::F64_MUL).op(op::F64_NEG);
        fb.g(k).i32c(2).op(op::I32_MUL).i32c(1).op(op::I32_SUB).op(op::F64_CONVERT_I32_S);
        fb.g(k).i32c(2).op(op::I32_MUL).op(op::F64_CONVERT_I32_S);
        fb.op(op::F64_MUL).op(op::F64_DIV).s(t);
        fb.g(s).g(t).op(op::F64_ADD).s(s);
        fb.g(k).i32c(1).op(op::I32_ADD).s(k);
        fb.br(0);
        fb.end();
        fb.end();
        fb.g(s);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `Trig(x, which) -> f64`: which 0 = sin, 1 = cos, 2 = tan.
    /// Range-reduces to [0, 2pi), then maps quadrants onto the kernels — the
    /// same identities the freestanding native runtime uses.
    fn h_trig(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(2);
        let r = fb.scratch(Val::F64);
        let y = fb.scratch(Val::F64);
        let q = fb.scratch(Val::I32);
        let c = fb.scratch(Val::F64);
        let t = fb.scratch(Val::F64);
        const PI: f64 = 3.141592653589793;
        const PI_2: f64 = 1.5707963267948966;
        const PI_3_2: f64 = 4.71238898038469;
        fb.g(0).call(self.helper_index[&Helper::Reduce2Pi]).s(r);
        // tan(x) = sin/cos, with the freestanding-runtime zero-guard.
        fb.g(1).i32c(2).op(op::I32_EQ).if_();
        fb.g(r).i32c(1).call(self.helper_index[&Helper::Trig]).s(c);
        // tan = sin/cos with the zero guard; select between the arms through
        // a local so the void-blocktype if/else stays stack-balanced.
        fb.g(c).f64c(0.0).op(op::F64_NE).if_();
        fb.g(r).i32c(0).call(self.helper_index[&Helper::Trig]).g(c).op(op::F64_DIV).s(t);
        fb.else_();
        fb.f64c(0.0).s(t);
        fb.end();
        fb.g(t).op(op::RETURN);
        fb.end();
        // quadrant
        fb.g(r).f64c(PI_2).op(op::F64_LT).if_();
        fb.g(r).s(y);
        fb.i32c(0).s(q);
        fb.else_();
        fb.g(r).f64c(PI).op(op::F64_LT).if_();
        fb.g(r).f64c(PI_2).op(op::F64_SUB).s(y);
        fb.i32c(1).s(q);
        fb.else_();
        fb.g(r).f64c(PI_3_2).op(op::F64_LT).if_();
        fb.g(r).f64c(PI).op(op::F64_SUB).s(y);
        fb.i32c(2).s(q);
        fb.else_();
        fb.g(r).f64c(PI_3_2).op(op::F64_SUB).s(y);
        fb.i32c(3).s(q);
        fb.end();
        fb.end();
        fb.end();
        fb.g(1).i32c(0).op(op::I32_EQ).if_();
        fb.g(q).i32c(0).op(op::I32_EQ).if_();
        fb.g(y).call(self.helper_index[&Helper::SinPoly]);
        fb.op(op::RETURN);
        fb.end();
        fb.g(q).i32c(1).op(op::I32_EQ).if_();
        fb.g(y).call(self.helper_index[&Helper::CosPoly]);
        fb.op(op::RETURN);
        fb.end();
        fb.g(q).i32c(2).op(op::I32_EQ).if_();
        fb.g(y).call(self.helper_index[&Helper::SinPoly]);
        fb.op(op::F64_NEG).op(op::RETURN);
        fb.end();
        fb.g(y).call(self.helper_index[&Helper::CosPoly]);
        fb.op(op::F64_NEG).op(op::RETURN);
        fb.else_();
        fb.g(q).i32c(0).op(op::I32_EQ).if_();
        fb.g(y).call(self.helper_index[&Helper::CosPoly]);
        fb.op(op::RETURN);
        fb.end();
        fb.g(q).i32c(1).op(op::I32_EQ).if_();
        fb.g(y).call(self.helper_index[&Helper::SinPoly]);
        fb.op(op::F64_NEG).op(op::RETURN);
        fb.end();
        fb.g(q).i32c(2).op(op::I32_EQ).if_();
        fb.g(y).call(self.helper_index[&Helper::CosPoly]);
        fb.op(op::F64_NEG).op(op::RETURN);
        fb.end();
        fb.g(y).call(self.helper_index[&Helper::SinPoly]);
        fb.op(op::RETURN);
        fb.end();
        // Every quadrant path above returns, but a real wasm validator (and
        // our mini validator) still considers the position after this
        // if-else reachable with the frame's (void) result: leave an
        // explicit unreachable so the function tail type-checks.
        fb.op(op::UNREACHABLE);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `TimeSeed() -> i64`: lazily seed the xorshift state from the WASI
    /// realtime clock, then return the state.
    fn h_time_seed(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(0);
        fb.gget(GLOBAL_RNG).op(op::I64_EQZ).if_();
        fb.i32c(0).i64c(0).i32c(CLOCK_BUF as i64);
        fb.call(self.import_index[&Wasi::ClockTimeGet]);
        fb.op(op::DROP);
        fb.i32c(CLOCK_BUF as i64).load64(0);
        fb.i64c(0x1234567890abcdef).op(op::I64_XOR);
        fb.gset(GLOBAL_RNG);
        fb.end();
        fb.gget(GLOBAL_RNG);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `Random() -> i64`: xorshift(13,7,17) masked positive, exactly like
    /// `lpp_random` (including the shared seed state).
    fn h_random(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(0);
        let s = fb.scratch(Val::I64);
        fb.call(self.helper_index[&Helper::TimeSeed]).s(s);
        fb.g(s).g(s).i64c(13).op(op::I64_SHL).op(op::I64_XOR).s(s);
        fb.g(s).g(s).i64c(7).op(op::I64_SHR_U).op(op::I64_XOR).s(s);
        fb.g(s).g(s).i64c(17).op(op::I64_SHL).op(op::I64_XOR).s(s);
        fb.g(s).gset(GLOBAL_RNG);
        fb.g(s).i64c(0x7fff_ffff_ffff_ffff).op(op::I64_AND);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `RandomRange(lo, hi) -> i64`.
    fn h_random_range(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(2);
        fb.g(0).g(1).op(op::I64_GE_S).if_();
        fb.g(0).op(op::RETURN);
        fb.end();
        fb.g(0).call(self.helper_index[&Helper::Random]);
        fb.g(1).g(0).op(op::I64_SUB).op(op::I64_REM_U).op(op::I64_ADD);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `TimeMs() -> i64`: monotonic clock in milliseconds.
    fn h_time_ms(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(0);
        fb.i32c(1).i64c(0).i32c(CLOCK_BUF as i64);
        fb.call(self.import_index[&Wasi::ClockTimeGet]);
        fb.op(op::DROP);
        fb.i32c(CLOCK_BUF as i64).load64(0).i64c(1_000_000).op(op::I64_DIV_U);
        fb.end();
        (fb.extras, fb.body)
    }

    /// `SleepMs(ms)`: build a 56-byte clock subscription and `poll_oneoff`.
    fn h_sleep_ms(&mut self) -> (Vec<Val>, Vec<u8>) {
        let mut fb = FB::new(1);
        fb.g(0).i64c(0).op(op::I64_LT_S).if_();
        fb.i64c(0).s(0);
        fb.end();
        // WASI subscription_clock layout (subscription is 48 bytes total):
        // userdata@0, eventtype@8 (u8; 0 = clock), then the clock payload at
        // +16: clock_id@16 (u32), timeout@24 (u64 ns), precision@32 (u64),
        // flags@40 (u16; 0 = relative timeout). Zeroing defaults everything
        // else, so only clock_id (1 = monotonic) and the timeout are set.
        fb.i32c(SUB_BUF as i64).i32c(0).i32c(56).memory_fill();
        fb.i32c((SUB_BUF + 16) as i64).i32c(1).store32(0);
        fb.i32c((SUB_BUF + 24) as i64).g(0).i64c(1_000_000).op(op::I64_MUL).store64(0);
        fb.i32c(SUB_BUF as i64).i32c(EVENT_BUF as i64).i32c(1).i32c(FD_IO_OUT as i64);
        fb.call(self.import_index[&Wasi::PollOneoff]);
        fb.op(op::DROP);
        fb.end();
        (fb.extras, fb.body)
    }
}

// ── Module assembly ──────────────────────────────────────────────────────────

/// wasm parameters of a user function (Void params get the i64 placeholder,
/// mirroring the Cranelift ABI mapping).
fn user_params(function: &MirFunction) -> Vec<Val> {
    function
        .params
        .iter()
        .map(|id| val_of_type(&function.locals[id.0].ty))
        .collect()
}

/// wasm results of a user function (Void returns nothing at all).
fn user_results(function: &MirFunction) -> Vec<Val> {
    if function.return_type == TypeRef::Void {
        vec![]
    } else {
        vec![val_of_type(&function.return_type)]
    }
}

/// Encode the code-section locals list: runs of equal value types.
fn enc_locals(out: &mut Vec<u8>, locals: &[Val]) {
    let mut runs: Vec<(u32, u8)> = Vec::new();
    for local in locals {
        if let Some(last) = runs.last_mut() {
            if last.1 == local.byte() {
                last.0 += 1;
                continue;
            }
        }
        runs.push((1, local.byte()));
    }
    uleb(out, runs.len() as u64);
    for (count, byte) in runs {
        uleb(out, count as u64);
        out.push(byte);
    }
}

impl<'a> WasmCompiler<'a> {
    /// Plan, lower, and serialize the whole module.
    fn compile(mut self) -> Result<Vec<u8>, String> {
        let main_id = self
            .program
            .functions
            .values()
            .find(|f| f.name == "main")
            .map(|f| f.id)
            .ok_or_else(|| {
                "WebAssembly backend requires a 'main' function as the entry point".to_string()
            })?;

        let scan = validate_program(self.program)?;
        self.plan_helpers(&scan);
        self.plan_indices(&scan);

        // Lower everything (pool interning and type registration both keep
        // accumulating through this phase; sections are emitted afterwards).
        let mut functions: Vec<&MirFunction> = self.program.functions.values().collect();
        functions.sort_by_key(|f| f.id.0);
        let mut code_entries: Vec<(Vec<Val>, Vec<u8>)> = Vec::new();
        for function in &functions {
            code_entries.push(self.lower_function(function)?);
        }
        for i in 0..self.type_table.definitions.len() {
            code_entries.push(self.drop_body(StructTypeId(i)));
        }
        for func_id in &scan.task_fns {
            code_entries.push(self.thunk_body(*func_id)?);
        }
        for helper in self.helpers.clone() {
            code_entries.push(self.helper_body(helper)?);
        }
        code_entries.push(self.body_start(main_id)?);

        let import_count = self.imports.len();
        if code_entries.len() != self.all_sigs.len() - import_count {
            return Err(format!(
                "internal wasm planning error: {} code bodies for {} function slots",
                code_entries.len(),
                self.all_sigs.len() - import_count
            ));
        }

        let mut out = Vec::with_capacity(4096);
        out.extend_from_slice(&[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]);

        // 1 — type section.
        let mut sec = Vec::new();
        uleb(&mut sec, self.types.len() as u64);
        for (params, results) in &self.types {
            sec.push(0x60);
            uleb(&mut sec, params.len() as u64);
            for p in params {
                sec.push(p.byte());
            }
            uleb(&mut sec, results.len() as u64);
            for r in results {
                sec.push(r.byte());
            }
        }
        enc_section(&mut out, 1, &sec);

        // 2 — import section (functions only).
        if !self.imports.is_empty() {
            let mut sec = Vec::new();
            uleb(&mut sec, self.imports.len() as u64);
            for wasi in &self.imports {
                enc_name(&mut sec, wasi.module());
                enc_name(&mut sec, wasi.field());
                sec.push(0x00); // func
                let sig = wasi.signature();
                let ty = self.type_map[&sig];
                uleb(&mut sec, ty as u64);
            }
            enc_section(&mut out, 2, &sec);
        }

        // 3 — function section.
        let mut sec = Vec::new();
        uleb(&mut sec, code_entries.len() as u64);
        for sig in self.all_sigs.iter().skip(import_count) {
            let ty = self.type_map[sig];
            uleb(&mut sec, ty as u64);
        }
        enc_section(&mut out, 3, &sec);

        // 4 — table section (funcref, exact minimum).
        let mut sec = Vec::new();
        uleb(&mut sec, 1);
        sec.push(0x70); // funcref
        sec.push(0x00); // limits: min only
        uleb(&mut sec, self.table.total as u64);
        enc_section(&mut out, 4, &sec);

        // 5 — memory section (initial pages cover the static pool + base).
        let heap_base = self.heap_base();
        let min_pages = ((heap_base as u64) + 65535) / 65536;
        let mut sec = Vec::new();
        uleb(&mut sec, 1);
        sec.push(0x00); // limits: min only
        uleb(&mut sec, min_pages);
        enc_section(&mut out, 5, &sec);

        // 6 — globals: heap bump pointer + RNG state.
        let mut sec = Vec::new();
        uleb(&mut sec, 2);
        // GLOBAL_HEAP: mut i32 = heap_base
        sec.push(Val::I32.byte());
        sec.push(0x01); // mutable
        sec.push(op::I32_CONST);
        sleb(&mut sec, heap_base as i64);
        sec.push(op::END);
        // GLOBAL_RNG: mut i64 = 0
        sec.push(Val::I64.byte());
        sec.push(0x01);
        sec.push(op::I64_CONST);
        sleb(&mut sec, 0);
        sec.push(op::END);
        enc_section(&mut out, 6, &sec);

        // 7 — exports: memory + _start.
        let mut sec = Vec::new();
        uleb(&mut sec, 2);
        enc_name(&mut sec, "memory");
        sec.push(0x02);
        uleb(&mut sec, 0);
        enc_name(&mut sec, "_start");
        sec.push(0x00);
        uleb(&mut sec, self.start_index as u64);
        enc_section(&mut out, 7, &sec);

        // 9 — elem: active segment filling the table seats.
        let mut sec = Vec::new();
        uleb(&mut sec, 1);
        uleb(&mut sec, 0); // table 0
        sec.push(op::I32_CONST);
        sleb(&mut sec, 0);
        sec.push(op::END);
        uleb(&mut sec, self.table.total as u64);
        for seat in 0..self.table.total {
            let f = self.table_seat_function(seat, &scan);
            uleb(&mut sec, f as u64);
        }
        enc_section(&mut out, 9, &sec);

        // 10 — code.
        let mut sec = Vec::new();
        uleb(&mut sec, code_entries.len() as u64);
        for (locals, body) in &code_entries {
            let mut entry = Vec::new();
            enc_locals(&mut entry, locals);
            entry.extend_from_slice(body);
            uleb(&mut sec, entry.len() as u64);
            sec.extend_from_slice(&entry);
        }
        enc_section(&mut out, 10, &sec);

        // 11 — data: the static literal pool at POOL_START.
        if self.pool.len() > POOL_START as usize {
            let mut sec = Vec::new();
            uleb(&mut sec, 1);
            uleb(&mut sec, 0); // memory 0
            sec.push(op::I32_CONST);
            sleb(&mut sec, POOL_START as i64);
            sec.push(op::END);
            let bytes = &self.pool[POOL_START as usize..];
            uleb(&mut sec, bytes.len() as u64);
            sec.extend_from_slice(bytes);
            enc_section(&mut out, 11, &sec);
        }

        // 0 — names (function names subsection).
        let mut sec = Vec::new();
        enc_name(&mut sec, "name");
        let mut sub = Vec::new();
        uleb(&mut sub, self.names.len() as u64);
        for (index, name) in &self.names {
            uleb(&mut sub, *index as u64);
            enc_name(&mut sub, name);
        }
        sec.push(1);
        uleb(&mut sec, sub.len() as u64);
        sec.extend_from_slice(&sub);
        enc_section(&mut out, 0, &sec);

        Ok(out)
    }
}

/// Compile a validated, ownership-annotated MIR program to a wasm32-wasi
/// module. `weak_fields` lists the struct fields demoted to weak by the
/// native ownership analysis (their drop edge is skipped by destructors).
pub fn compile(
    program: &MirProgram,
    type_table: &TypeTable,
    weak_fields: &HashSet<(StructTypeId, String)>,
) -> Result<Vec<u8>, String> {
    WasmCompiler::new(program, type_table, weak_fields).compile()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Byte-level encoder checks ────────────────────────────────────────

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

    // ── MIR fixtures ─────────────────────────────────────────────────────

    fn mk_fn(
        id: usize,
        name: &str,
        tys: &[TypeRef],
        params: usize,
        blocks: Vec<MirBlock>,
        ret: TypeRef,
        is_async: bool,
    ) -> MirFunction {
        MirFunction {
            id: FuncId(id),
            name: name.to_string(),
            params: (0..params).map(LocalId).collect(),
            locals: tys
                .iter()
                .enumerate()
                .map(|(i, t)| LocalDecl {
                    id: LocalId(i),
                    ty: t.clone(),
                    is_mut: false,
                    debug_name: None,
                    binding_id: None,
                    ownership: Ownership::Owned,
                })
                .collect(),
            blocks,
            start_block: BlockId(0),
            return_type: ret,
            is_async,
        }
    }

    fn bb(id: usize, instrs: Vec<MirInstr>, terminator: Terminator) -> MirBlock {
        MirBlock {
            id: BlockId(id),
            instrs,
            terminator,
        }
    }

    fn program_with(fns: Vec<MirFunction>) -> MirProgram {
        let mut functions = HashMap::new();
        for f in fns {
            functions.insert(f.id, f);
        }
        MirProgram { functions }
    }

    fn no_weak() -> HashSet<(StructTypeId, String)> {
        HashSet::new()
    }

    #[test]
    fn locals_map_params_first() {
        let function = mk_fn(
            0,
            "f",
            &[TypeRef::Int, TypeRef::Bool],
            1,
            vec![],
            TypeRef::Void,
            false,
        );
        let (index_of, extras) = WasmCompiler::local_indices(&function);
        assert_eq!(index_of, vec![0, 1]);
        assert_eq!(extras, vec![Val::I32]);
    }

    // ── Feature rejections (clear diagnostics, never silent miscompiles) ──

    fn reject_case(rvalue: Rvalue) -> String {
        let main = mk_fn(
            0,
            "main",
            &[TypeRef::Int],
            0,
            vec![bb(
                0,
                vec![MirInstr::Assign(LocalId(0), rvalue)],
                Terminator::Return(None),
            )],
            TypeRef::Void,
            false,
        );
        compile(&program_with(vec![main]), &TypeTable::new(), &no_weak()).unwrap_err()
    }

    #[test]
    fn rejects_spawn_threads() {
        let error = reject_case(Rvalue::SpawnThread(Operand::Int(0)));
        assert!(error.contains("OS threads"), "unexpected error: {}", error);
    }

    #[test]
    fn rejects_network_builtins_with_family_message() {
        let error = reject_case(Rvalue::BuiltinCall(
            "lpp_net_connect".to_string(),
            vec![Operand::String("x".to_string()), Operand::Int(1)],
        ));
        assert!(
            error.contains("network sockets"),
            "unexpected error: {}",
            error
        );
    }

    #[test]
    fn rejects_file_builtins_with_family_message() {
        let error = reject_case(Rvalue::BuiltinCall(
            "lpp_read_file".to_string(),
            vec![Operand::String("x".to_string())],
        ));
        assert!(
            error.contains("file system access"),
            "unexpected error: {}",
            error
        );
    }

    #[test]
    fn rejects_unknown_builtins_as_ffi() {
        let error = reject_case(Rvalue::BuiltinCall("acme_custom".to_string(), vec![]));
        assert!(error.contains("C FFI"), "unexpected error: {}", error);
    }

    #[test]
    fn rejects_simd_locals() {
        let main = mk_fn(
            0,
            "main",
            &[TypeRef::VectorI64x2],
            0,
            vec![bb(0, vec![], Terminator::Return(None))],
            TypeRef::Void,
            false,
        );
        let error = compile(&program_with(vec![main]), &TypeTable::new(), &no_weak()).unwrap_err();
        assert!(
            error.contains("SIMD vector types"),
            "unexpected error: {}",
            error
        );
    }

    #[test]
    fn requires_a_main_function() {
        let helper = mk_fn(
            0,
            "helper",
            &[],
            0,
            vec![bb(0, vec![], Terminator::Return(None))],
            TypeRef::Void,
            false,
        );
        let error = compile(&program_with(vec![helper]), &TypeTable::new(), &no_weak())
            .unwrap_err();
        assert!(
            error.contains("requires a 'main' function"),
            "unexpected error: {}",
            error
        );
    }

    // ── Minimal module ───────────────────────────────────────────────────

    #[test]
    fn minimal_valid_module_emits() {
        let main = mk_fn(
            0,
            "main",
            &[],
            0,
            vec![bb(0, vec![], Terminator::Return(None))],
            TypeRef::Void,
            false,
        );
        let module = compile(&program_with(vec![main]), &TypeTable::new(), &no_weak())
            .expect("compiles");
        assert_eq!(
            &module[..8],
            &[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]
        );
        assert!(module.windows(6).any(|w| w == b"_start"));
        let parsed = parse_module(&module).expect("module parses");
        validate_module(&parsed).expect("module validates");
    }

    #[test]
    fn wasm_shifts_compile_and_validate() {
        let mut instrs = Vec::new();
        let mut tys = Vec::new();
        macro_rules! a {
            ($ty:expr, $rv:expr) => {{
                let id = LocalId(tys.len());
                tys.push($ty);
                instrs.push(MirInstr::Assign(id, $rv));
                id
            }};
        }
        a!(TypeRef::Int, Rvalue::BinaryOp(BinaryOperator::Shl, Operand::Int(1), Operand::Int(64)));
        a!(TypeRef::Int, Rvalue::BinaryOp(BinaryOperator::Shr, Operand::Int(1), Operand::Int(64)));
        a!(TypeRef::Int, Rvalue::BinaryOp(BinaryOperator::Shl, Operand::Int(1), Operand::Int(-1)));
        a!(TypeRef::Int, Rvalue::BinaryOp(BinaryOperator::Shr, Operand::Int(1), Operand::Int(-1)));
        let main = mk_fn(
            0,
            "main",
            &tys,
            0,
            vec![bb(0, instrs, Terminator::Return(None))],
            TypeRef::Void,
            false,
        );
        let module = compile(&program_with(vec![main]), &TypeTable::new(), &no_weak()).expect("compiles");
        let parsed = parse_module(&module).expect("module parses");
        validate_module(&parsed).expect("module validates");
    }

    // ── Rich end-to-end module ───────────────────────────────────────────

    /// A program touching structs, lists, maps, tuples, slices, closures,
    /// async tasks, strings, math, I/O helpers and the process builtins.
    fn rich_program() -> (MirProgram, TypeTable) {
        let mut tt = TypeTable::new();
        let node = tt.register_struct("Node".to_string());
        tt.definitions[node.0].fields = vec![
            ("value".to_string(), TypeRef::Int),
            ("name".to_string(), TypeRef::Str),
        ];
        let list_int = TypeRef::Generic("List".to_string(), vec![TypeRef::Int]);
        let list_str = TypeRef::Generic("List".to_string(), vec![TypeRef::Str]);
        let map_int = TypeRef::Generic("Map".to_string(), vec![TypeRef::Int, TypeRef::Int]);
        let task_int = TypeRef::Task(Box::new(TypeRef::Int));

        // fn helper(x: Int) -> Int { x * 2 }
        let helper = mk_fn(
            0,
            "helper",
            &[TypeRef::Int, TypeRef::Int],
            1,
            vec![bb(
                0,
                vec![MirInstr::Assign(
                    LocalId(1),
                    Rvalue::BinaryOp(
                        BinaryOperator::Multiply,
                        Operand::Local(LocalId(0)),
                        Operand::Int(2),
                    ),
                )],
                Terminator::Return(Some(Operand::Local(LocalId(1)))),
            )],
            TypeRef::Int,
            false,
        );

        // async fn compute(a: Int) -> Int { a + 1 }
        let compute = mk_fn(
            1,
            "compute",
            &[TypeRef::Int, TypeRef::Int],
            1,
            vec![bb(
                0,
                vec![MirInstr::Assign(
                    LocalId(1),
                    Rvalue::BinaryOp(
                        BinaryOperator::Add,
                        Operand::Local(LocalId(0)),
                        Operand::Int(1),
                    ),
                )],
                Terminator::Return(Some(Operand::Local(LocalId(1)))),
            )],
            TypeRef::Int,
            true,
        );

        // fn addenv(env: (Int), y: Int) -> Int { env.0 + y }
        let addenv = mk_fn(
            2,
            "addenv",
            &[TypeRef::Tuple(vec![TypeRef::Int]), TypeRef::Int, TypeRef::Int, TypeRef::Int],
            2,
            vec![bb(
                0,
                vec![
                    MirInstr::Assign(
                        LocalId(2),
                        Rvalue::TupleField(Operand::Local(LocalId(0)), 0),
                    ),
                    MirInstr::Assign(
                        LocalId(3),
                        Rvalue::BinaryOp(
                            BinaryOperator::Add,
                            Operand::Local(LocalId(2)),
                            Operand::Local(LocalId(1)),
                        ),
                    ),
                ],
                Terminator::Return(Some(Operand::Local(LocalId(3)))),
            )],
            TypeRef::Int,
            false,
        );

        // fn main() — one long entry block exercising the supported surface.
        let mut tys: Vec<TypeRef> = vec![];
        let mut instrs: Vec<MirInstr> = vec![];
        macro_rules! a {
            ($ty:expr, $rv:expr) => {{
                tys.push($ty);
                let id = LocalId(tys.len() - 1);
                instrs.push(MirInstr::Assign(id, $rv));
                id
            }};
        }

        // scalars, prints, basic strings
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_print_int".to_string(), vec![Operand::Int(42)]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_print_bool".to_string(), vec![Operand::Bool(true)]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_print_float".to_string(), vec![Operand::Float(2.5)]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_print_str".to_string(), vec![Operand::String("hello".to_string())]));
        a!(TypeRef::Str, Rvalue::BuiltinCall("lpp_str_concat".to_string(), vec![Operand::String("a".to_string()), Operand::String("b".to_string())]));
        a!(TypeRef::Str, Rvalue::BuiltinCall("lpp_str_substr".to_string(), vec![Operand::String("abcdef".to_string()), Operand::Int(1), Operand::Int(3)]));
        a!(TypeRef::Str, Rvalue::BuiltinCall("lpp_str_trim".to_string(), vec![Operand::String(" x ".to_string())]));
        a!(TypeRef::Str, Rvalue::BuiltinCall("lpp_str_upper".to_string(), vec![Operand::String("up".to_string())]));
        a!(TypeRef::Str, Rvalue::BuiltinCall("lpp_str_lower".to_string(), vec![Operand::String("DOWN".to_string())]));
        a!(TypeRef::Str, Rvalue::BuiltinCall("lpp_int_to_str".to_string(), vec![Operand::Int(42)]));
        a!(TypeRef::Str, Rvalue::BuiltinCall("lpp_float_to_str".to_string(), vec![Operand::Float(2.5)]));
        a!(TypeRef::Str, Rvalue::BuiltinCall("lpp_bool_to_str".to_string(), vec![Operand::Bool(false)]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_str_to_int".to_string(), vec![Operand::String("42".to_string())]));
        a!(TypeRef::Str, Rvalue::BuiltinCall("lpp_char_at".to_string(), vec![Operand::String("abc".to_string()), Operand::Int(1)]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_ord".to_string(), vec![Operand::String("A".to_string())]));
        a!(TypeRef::Str, Rvalue::BuiltinCall("lpp_chr".to_string(), vec![Operand::Int(65)]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_str_len".to_string(), vec![Operand::String("len".to_string())]));
        a!(TypeRef::Bool, Rvalue::BuiltinCall("lpp_str_eq".to_string(), vec![Operand::String("q".to_string()), Operand::String("q".to_string())]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_str_find".to_string(), vec![Operand::String("haystack".to_string()), Operand::String("st".to_string())]));
        a!(TypeRef::Bool, Rvalue::BuiltinCall("lpp_str_contains".to_string(), vec![Operand::String("hay".to_string()), Operand::String("ay".to_string())]));
        a!(TypeRef::Bool, Rvalue::BuiltinCall("lpp_str_starts_with".to_string(), vec![Operand::String("hay".to_string()), Operand::String("ha".to_string())]));
        a!(TypeRef::Bool, Rvalue::BuiltinCall("lpp_str_ends_with".to_string(), vec![Operand::String("hay".to_string()), Operand::String("ay".to_string())]));
        a!(TypeRef::Str, Rvalue::BuiltinCall("lpp_str_repeat".to_string(), vec![Operand::String("ab".to_string()), Operand::Int(2)]));
        a!(TypeRef::Str, Rvalue::BuiltinCall("lpp_str_replace".to_string(), vec![Operand::String("banana".to_string()), Operand::String("na".to_string()), Operand::String("NA".to_string())]));
        a!(list_str.clone(), Rvalue::BuiltinCall("lpp_str_split".to_string(), vec![Operand::String("a,b".to_string()), Operand::Int(44)]));

        // math
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_abs".to_string(), vec![Operand::Int(-3)]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_min".to_string(), vec![Operand::Int(1), Operand::Int(2)]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_max".to_string(), vec![Operand::Int(1), Operand::Int(2)]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_int_pow".to_string(), vec![Operand::Int(2), Operand::Int(10)]));
        a!(TypeRef::Float, Rvalue::BuiltinCall("lpp_pow".to_string(), vec![Operand::Float(2.0), Operand::Float(10.0)]));
        a!(TypeRef::Float, Rvalue::BuiltinCall("lpp_floor".to_string(), vec![Operand::Float(2.7)]));
        a!(TypeRef::Float, Rvalue::BuiltinCall("lpp_ceil".to_string(), vec![Operand::Float(2.1)]));
        a!(TypeRef::Float, Rvalue::BuiltinCall("lpp_sqrt".to_string(), vec![Operand::Float(4.0)]));
        a!(TypeRef::Float, Rvalue::BuiltinCall("lpp_sin".to_string(), vec![Operand::Float(1.0)]));
        a!(TypeRef::Float, Rvalue::BuiltinCall("lpp_cos".to_string(), vec![Operand::Float(1.0)]));
        a!(TypeRef::Float, Rvalue::BuiltinCall("lpp_tan".to_string(), vec![Operand::Float(1.0)]));
        a!(TypeRef::Float, Rvalue::BuiltinCall("fmod".to_string(), vec![Operand::Float(7.5), Operand::Float(2.0)]));
        a!(TypeRef::Float, Rvalue::BuiltinCall("lpp_int_to_float".to_string(), vec![Operand::Int(3)]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_float_to_int".to_string(), vec![Operand::Float(3.9)]));
        let arg41 = a!(TypeRef::Int, Rvalue::Use(Operand::Int(41)));

        // calls and closures
        a!(TypeRef::Int, Rvalue::CallDirect(FuncId(0), vec![Operand::Local(arg41)]));
        let envt = a!(TypeRef::Tuple(vec![TypeRef::Int]), Rvalue::AllocateTuple(vec![TypeRef::Int], vec![Operand::Int(3)]));
        let clos = a!(TypeRef::Function, Rvalue::MakeClosure(FuncId(2), vec![Operand::Local(envt)]));
        a!(TypeRef::Int, Rvalue::CallIndirect(Operand::Local(clos), vec![Operand::Int(5)]));
        let fref = a!(TypeRef::Int, Rvalue::FuncRef(FuncId(0)));
        a!(TypeRef::Int, Rvalue::CallIndirect(Operand::Local(fref), vec![Operand::Int(21)]));

        // async task
        let task = a!(task_int.clone(), Rvalue::MakeTask(FuncId(1), vec![TypeRef::Int], vec![Operand::Local(arg41)], TypeRef::Int));
        a!(TypeRef::Int, Rvalue::Await(Operand::Local(task)));

        // struct: heap + field mutation + arena variant
        let st = a!(TypeRef::Custom(node), Rvalue::AllocateArcStruct(TypeRef::Custom(node)));
        instrs.push(MirInstr::AssignField { base: st, field: "value".to_string(), value: Operand::Int(7) });
        instrs.push(MirInstr::AssignField { base: st, field: "name".to_string(), value: Operand::String("n".to_string()) });
        a!(TypeRef::Int, Rvalue::FieldAccess(Operand::Local(st), "value".to_string()));
        let tok = a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_arena_begin".to_string(), vec![]));
        a!(TypeRef::Custom(node), Rvalue::AllocateArenaStruct(TypeRef::Custom(node), Operand::Local(tok)));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_arena_release".to_string(), vec![Operand::Local(tok)]));
        instrs.push(MirInstr::Release(st));

        // list + slice views
        let li = a!(list_int.clone(), Rvalue::AllocateList(TypeRef::Int));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_list_push".to_string(), vec![Operand::Local(li), Operand::Int(7)]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_list_push_bool".to_string(), vec![Operand::Local(li), Operand::Bool(true)]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_list_push_float".to_string(), vec![Operand::Local(li), Operand::Float(1.5)]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_list_get".to_string(), vec![Operand::Local(li), Operand::Int(0)]));
        a!(TypeRef::Float, Rvalue::BuiltinCall("lpp_list_get_float".to_string(), vec![Operand::Local(li), Operand::Int(2)]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_list_set".to_string(), vec![Operand::Local(li), Operand::Int(0), Operand::Int(9)]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_list_len".to_string(), vec![Operand::Local(li)]));
        let svl = a!(TypeRef::Slice(Box::new(TypeRef::Int)), Rvalue::MakeSlice { base: Operand::Local(li), start: Operand::Int(0), length: Operand::Int(1), kind: 1 });
        a!(TypeRef::Int, Rvalue::SliceLen(Operand::Local(svl)));
        a!(TypeRef::Int, Rvalue::SliceGet(Operand::Local(svl), Operand::Int(0)));
        let sbase = a!(TypeRef::Str, Rvalue::BuiltinCall("lpp_str_concat".to_string(), vec![Operand::String("xy".to_string()), Operand::String("z".to_string())]));
        let svs = a!(TypeRef::StrSlice, Rvalue::MakeSlice { base: Operand::Local(sbase), start: Operand::Int(1), length: Operand::Int(1), kind: 0 });
        a!(TypeRef::Str, Rvalue::SliceGet(Operand::Local(svs), Operand::Int(0)));
        a!(TypeRef::Str, Rvalue::SliceToStr(Operand::Local(svs)));

        // map
        let mp = a!(map_int.clone(), Rvalue::BuiltinCall("lpp_map_new".to_string(), vec![]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_map_put".to_string(), vec![Operand::Local(mp), Operand::Int(1), Operand::Int(2)]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_map_put_str".to_string(), vec![Operand::Local(mp), Operand::String("k".to_string()), Operand::Int(3)]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_map_get".to_string(), vec![Operand::Local(mp), Operand::Int(1)]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_map_get_str".to_string(), vec![Operand::Local(mp), Operand::String("k".to_string())]));
        a!(TypeRef::Bool, Rvalue::BuiltinCall("lpp_map_has".to_string(), vec![Operand::Local(mp), Operand::Int(1)]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_map_remove".to_string(), vec![Operand::Local(mp), Operand::Int(1)]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_map_len".to_string(), vec![Operand::Local(mp)]));

        // random / time / process
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_random_seed".to_string(), vec![Operand::Int(1)]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_random".to_string(), vec![]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_random_range".to_string(), vec![Operand::Int(1), Operand::Int(10)]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_time_ms".to_string(), vec![]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_sleep_ms".to_string(), vec![Operand::Int(1)]));
        a!(TypeRef::Str, Rvalue::BuiltinCall("lpp_input".to_string(), vec![]));
        a!(TypeRef::Str, Rvalue::BuiltinCall("lpp_env_get".to_string(), vec![Operand::String("PATH".to_string())]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_alloc".to_string(), vec![Operand::Int(40)]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_free".to_string(), vec![Operand::Int(0)]));
        a!(TypeRef::Int, Rvalue::BuiltinCall("lpp_exit".to_string(), vec![Operand::Int(0)]));

        let main = mk_fn(
            3,
            "main",
            &tys,
            0,
            vec![bb(0, instrs, Terminator::Return(None))],
            TypeRef::Void,
            false,
        );
        (program_with(vec![helper, compute, addenv, main]), tt)
    }

    #[test]
    fn rich_program_compiles_and_validates() {
        let (program, tt) = rich_program();
        let module = compile(&program, &tt, &no_weak()).expect("compiles");
        let parsed = parse_module(&module).expect("module parses");
        validate_module(&parsed).expect("module validates");
        // hand-built programs cover every section shape
        assert!(!parsed.imports.is_empty(), "expected WASI imports");
        assert!(!parsed.elems.is_empty(), "expected table seats");
        assert!(!parsed.datas.is_empty(), "expected the static pool");
    }

    #[test]
    fn output_is_deterministic() {
        let (program, tt) = rich_program();
        let first = compile(&program, &tt, &no_weak()).expect("first compile");
        let second = compile(&program, &tt, &no_weak()).expect("second compile");
        assert_eq!(first, second, "wasm output must be byte-for-byte stable");
    }

    #[test]
    fn async_main_module_compiles_and_validates() {
        // async def main() covered by the synthesized task-wrapper _start.
        let main = mk_fn(
            0,
            "main",
            &[],
            0,
            vec![bb(0, vec![], Terminator::Return(None))],
            TypeRef::Void,
            true,
        );
        let module = compile(&program_with(vec![main]), &TypeTable::new(), &no_weak())
            .expect("compiles");
        let parsed = parse_module(&module).expect("module parses");
        validate_module(&parsed).expect("module validates");
    }

    #[test]
    fn control_flow_dispatcher_branches_validate() {
        // main with a two-block loop: bb0 (i<3? → bb1 : exit), bb1 → bb0.
        let main = mk_fn(
            0,
            "main",
            &[TypeRef::Int],
            0,
            vec![
                bb(
                    0,
                    vec![MirInstr::Assign(
                        LocalId(0),
                        Rvalue::BinaryOp(
                            BinaryOperator::Add,
                            Operand::Local(LocalId(0)),
                            Operand::Int(1),
                        ),
                    )],
                    Terminator::IfCmp {
                        op: BinaryOperator::Less,
                        left: Operand::Local(LocalId(0)),
                        right: Operand::Int(3),
                        then_block: BlockId(1),
                        else_block: BlockId(2),
                    },
                ),
                bb(1, vec![], Terminator::Goto(BlockId(0))),
                bb(2, vec![], Terminator::Return(None)),
            ],
            TypeRef::Void,
            false,
        );
        let module = compile(&program_with(vec![main]), &TypeTable::new(), &no_weak())
            .expect("compiles");
        let parsed = parse_module(&module).expect("module parses");
        validate_module(&parsed).expect("module validates");
    }

    /// Shared fixture for the all-bodies validation test: a tiny program
    /// with one struct (for drop bodies) and one async fn (for task
    /// thunks), with every helper planned so all signatures resolve.
    ///
    /// Forces every synthesized helper body through the validator,
    /// including ones the planning scan would not pick for a small
    /// program.
    fn validate_all_bodies(helpers: &[Helper], include_meta: bool) {
        let main = mk_fn(
            0,
            "main",
            &[],
            0,
            vec![bb(0, vec![], Terminator::Return(None))],
            TypeRef::Void,
            false,
        );
        let mut tt = TypeTable::new();
        let node = tt.register_struct("Node".to_string());
        tt.definitions[node.0].fields = vec![("name".to_string(), TypeRef::Str)];
        // include an async function so task thunks exist
        let compute = mk_fn(
            1,
            "compute",
            &[],
            0,
            vec![bb(0, vec![], Terminator::Return(None))],
            TypeRef::Void,
            true,
        );
        let program = program_with(vec![main, compute]);
        let weak = no_weak();
        let mut compiler = WasmCompiler::new(&program, &tt, &weak);
        let scan = validate_program(&program).expect("scan");
        compiler.helpers = Helper::all().to_vec();
        compiler.imports = vec![
            Wasi::FdWrite,
            Wasi::FdRead,
            Wasi::ProcExit,
            Wasi::ClockTimeGet,
            Wasi::PollOneoff,
            Wasi::EnvironSizesGet,
            Wasi::EnvironGet,
        ];
        compiler.plan_indices(&scan);
        if include_meta {
            // user bodies
            let mut functions: Vec<&MirFunction> = program.functions.values().collect();
            functions.sort_by_key(|f| f.id.0);
            for function in &functions {
                let (locals, body) = compiler.lower_function(function).expect("lower");
                validate_one_body(&compiler, compiler.fn_index[&function.id], locals, body)
                    .unwrap_or_else(|e| panic!("user fn {}: {}", function.name, e));
            }
            for i in 0..tt.definitions.len() {
                let (locals, body) = compiler.drop_body(StructTypeId(i));
                validate_one_body(&compiler, compiler.struct_drop_fn[&StructTypeId(i)], locals, body)
                    .unwrap_or_else(|e| panic!("drop body {}: {}", i, e));
            }
            for func_id in &scan.task_fns {
                let (locals, body) = compiler.thunk_body(*func_id).expect("thunk");
                validate_one_body(&compiler, compiler.thunk_fn[func_id], locals, body)
                    .unwrap_or_else(|e| panic!("thunk fn_{}: {}", func_id.0, e));
            }
            let (locals, body) = compiler.body_start(FuncId(0)).expect("start");
            validate_one_body(&compiler, compiler.start_index, locals, body)
                .unwrap_or_else(|e| panic!("_start: {}", e));
        }
        for helper in helpers {
            let (locals, body) = compiler
                .helper_body(*helper)
                .unwrap_or_else(|e| panic!("helper {:?} errors: {}", helper, e));
            validate_one_body(&compiler, compiler.helper_index[helper], locals, body)
                .unwrap_or_else(|e| panic!("helper {:?}: {}", helper, e));
        }
    }

    #[test]
    fn every_helper_body_validates() {
        validate_all_bodies(&Helper::all(), true);
    }

    // ── A small structural WebAssembly validator (tests only) ────────────

    impl Helper {
        /// Every helper variant, in enum (Ord) order.
        fn all() -> &'static [Helper] {
            &[
                Helper::TrapStub,
                Helper::Alloc,
                Helper::ArcAlloc,
                Helper::Retain,
                Helper::Release,
                Helper::StrAlloc,
                Helper::StrNew,
                Helper::WriteFd,
                Helper::Write,
                Helper::FmtU64,
                Helper::WriteU64,
                Helper::PrintInt,
                Helper::PrintBool,
                Helper::PrintFloat,
                Helper::PrintStr,
                Helper::PanicMsg,
                Helper::Panic2,
                Helper::Panic3,
                Helper::Exit,
                Helper::Input,
                Helper::EnvMatch,
                Helper::EnvGet,
                Helper::StrLen,
                Helper::StrEq,
                Helper::StrConcat,
                Helper::StrSubstr,
                Helper::StrTrim,
                Helper::StrUpper,
                Helper::StrLower,
                Helper::IntToStr,
                Helper::FloatToStr,
                Helper::BoolToStr,
                Helper::StrToInt,
                Helper::CharAt,
                Helper::Ord,
                Helper::Chr,
                Helper::StrFindFrom,
                Helper::StrFind,
                Helper::StrContains,
                Helper::StrStartsWith,
                Helper::StrEndsWith,
                Helper::StrRepeat,
                Helper::StrReplace,
                Helper::StrSplit,
                Helper::ListNew,
                Helper::ListPush,
                Helper::ListSet,
                Helper::ListGet,
                Helper::ListLen,
                Helper::ListDestroy,
                Helper::MapNew,
                Helper::MapLen,
                Helper::HashStr,
                Helper::HashInt,
                Helper::MapProbe,
                Helper::MapRehash,
                Helper::MapPut,
                Helper::MapGet,
                Helper::MapHas,
                Helper::MapRemove,
                Helper::MapDestroy,
                Helper::TupleAlloc,
                Helper::TupleDestroy,
                Helper::ClosureDestroy,
                Helper::TaskNew,
                Helper::TaskRun,
                Helper::TaskAwait,
                Helper::TaskPoll,
                Helper::TaskDestroy,
                Helper::SliceNew,
                Helper::SliceLen,
                Helper::SliceGet,
                Helper::StrSliceGet,
                Helper::StrSliceToStr,
                Helper::IntPow,
                Helper::Log2,
                Helper::Exp2,
                Helper::Pow,
                Helper::Reduce2Pi,
                Helper::SinPoly,
                Helper::CosPoly,
                Helper::Trig,
                Helper::TimeSeed,
                Helper::Random,
                Helper::RandomRange,
                Helper::TimeMs,
                Helper::SleepMs,
            ]
        }
    }

    const TI32: u8 = 0x7f;
    const TI64: u8 = 0x7e;
    const TF64: u8 = 0x7c;

    struct Reader<'a> {
        b: &'a [u8],
        p: usize,
    }

    impl<'a> Reader<'a> {
        fn new(b: &'a [u8]) -> Self {
            Reader { b, p: 0 }
        }
        fn byte(&mut self) -> Result<u8, String> {
            let v = *self.b.get(self.p).ok_or("unexpected end of bytes")?;
            self.p += 1;
            Ok(v)
        }
        fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
            if self.p + n > self.b.len() {
                return Err("unexpected end of bytes".to_string());
            }
            let s = &self.b[self.p..self.p + n];
            self.p += n;
            Ok(s)
        }
        fn uleb(&mut self) -> Result<u64, String> {
            let mut result = 0u64;
            let mut shift = 0u32;
            loop {
                let byte = self.byte()?;
                result |= ((byte & 0x7f) as u64) << shift;
                if byte & 0x80 == 0 {
                    return Ok(result);
                }
                shift += 7;
                if shift > 63 {
                    return Err("uleb128 too long".to_string());
                }
            }
        }
        fn sleb(&mut self) -> Result<i64, String> {
            let mut result = 0i64;
            let mut shift = 0u32;
            loop {
                let byte = self.byte()?;
                result |= (((byte & 0x7f) as i64) << shift) as i64;
                shift += 7;
                if byte & 0x80 == 0 {
                    if shift < 64 && byte & 0x40 != 0 {
                        result |= -1i64 << shift;
                    }
                    return Ok(result);
                }
                if shift > 63 {
                    return Err("sleb128 too long".to_string());
                }
            }
        }
        fn name(&mut self) -> Result<String, String> {
            let len = self.uleb()? as usize;
            let bytes = self.take(len)?;
            String::from_utf8(bytes.to_vec()).map_err(|_| "invalid utf-8 name".to_string())
        }
    }

    #[derive(Debug, Default)]
    struct ParsedModule {
        types: Vec<(Vec<u8>, Vec<u8>)>,
        imports: Vec<(String, String, u32)>, // (module, field, type index)
        funcs: Vec<u32>,                     // type indices
        table_min: u64,
        table_max: Option<u64>,
        memory_min: u64,
        globals: Vec<(u8, bool)>, // (value type, mutable)
        exports: Vec<(String, u8, u32)>,
        elems: Vec<(u64, Vec<u32>)>, // (table idx, funcidx list)
        code: Vec<(Vec<u8>, Vec<u8>)>, // (declared local type bytes, body)
        datas: Vec<(u64, Vec<u8>)>,    // (offset, bytes)
    }

    fn parse_module(bytes: &[u8]) -> Result<ParsedModule, String> {
        if bytes.len() < 8 || &bytes[..8] != b"\0asm\x01\0\0\0" {
            return Err("bad magic/version".to_string());
        }
        let mut module = ParsedModule {
            table_min: 0,
            table_max: None,
            ..ParsedModule::default()
        };
        let mut r = Reader::new(&bytes[8..]);
        while r.p < r.b.len() {
            let id = r.byte()?;
            let size = r.uleb()? as usize;
            let payload = r.take(size)?;
            let mut s = Reader::new(payload);
            match id {
                0 => {
                    // custom (names) — ignore contents but consume them
                    s.p = s.b.len();
                }
                1 => {
                    for _ in 0..s.uleb()? {
                        if s.byte()? != 0x60 {
                            return Err("type entry is not a functype".to_string());
                        }
                        let params = (0..s.uleb()?)
                            .map(|_| s.byte())
                            .collect::<Result<Vec<u8>, String>>()?;
                        let results = (0..s.uleb()?)
                            .map(|_| s.byte())
                            .collect::<Result<Vec<u8>, String>>()?;
                        module.types.push((params, results));
                    }
                }
                2 => {
                    for _ in 0..s.uleb()? {
                        let m = s.name()?;
                        let n = s.name()?;
                        let kind = s.byte()?;
                        if kind != 0 {
                            return Err("only function imports are emitted".to_string());
                        }
                        let ty = s.uleb()? as u32;
                        module.imports.push((m, n, ty));
                    }
                }
                3 => {
                    for _ in 0..s.uleb()? {
                        module.funcs.push(s.uleb()? as u32);
                    }
                }
                4 => {
                    for _ in 0..s.uleb()? {
                        if s.byte()? != 0x70 {
                            return Err("table is not funcref".to_string());
                        }
                        let flags = s.byte()?;
                        module.table_min = s.uleb()?;
                        if flags == 1 {
                            module.table_max = Some(s.uleb()?);
                        }
                    }
                }
                5 => {
                    for _ in 0..s.uleb()? {
                        let flags = s.byte()?;
                        module.memory_min = s.uleb()?;
                        if flags == 1 {
                            s.uleb()?;
                        }
                    }
                }
                6 => {
                    for _ in 0..s.uleb()? {
                        let ty = s.byte()?;
                        let mutable = match s.byte()? {
                            0 => false,
                            1 => true,
                            other => return Err(format!("bad mutability {}", other)),
                        };
                        // const init expr (single const instruction)
                        let op = s.byte()?;
                        match op {
                            0x41 | 0x42 => {
                                s.sleb()?;
                            }
                            0x44 => {
                                s.take(8)?;
                            }
                            other => return Err(format!("bad global init op {:#x}", other)),
                        }
                        if s.byte()? != 0x0b {
                            return Err("global init not terminated".to_string());
                        }
                        module.globals.push((ty, mutable));
                    }
                }
                7 => {
                    for _ in 0..s.uleb()? {
                        let n = s.name()?;
                        let kind = s.byte()?;
                        let idx = s.uleb()? as u32;
                        module.exports.push((n, kind, idx));
                    }
                }
                9 => {
                    for _ in 0..s.uleb()? {
                        let table = s.uleb()?;
                        if s.byte()? != 0x41 {
                            return Err("elem offset must be i32.const".to_string());
                        }
                        s.sleb()?;
                        if s.byte()? != 0x0b {
                            return Err("elem offset not terminated".to_string());
                        }
                        let mut items = Vec::new();
                        for _ in 0..s.uleb()? {
                            items.push(s.uleb()? as u32);
                        }
                        module.elems.push((table, items));
                    }
                }
                10 => {
                    for _ in 0..s.uleb()? {
                        let size = s.uleb()? as usize;
                        let body_bytes = s.take(size)?;
                        let mut cr = Reader::new(body_bytes);
                        let mut local_tys = Vec::new();
                        for _ in 0..cr.uleb()? {
                            let count = cr.uleb()?;
                            let ty = cr.byte()?;
                            for _ in 0..count {
                                local_tys.push(ty);
                            }
                        }
                        module.code.push((local_tys, cr.take(cr.b.len() - cr.p)?.to_vec()));
                    }
                }
                11 => {
                    for _ in 0..s.uleb()? {
                        if s.byte()? != 0 {
                            return Err("only active memory-0 data segments emitted".to_string());
                        }
                        if s.byte()? != 0x41 {
                            return Err("data offset must be i32.const".to_string());
                        }
                        let offset = s.sleb()? as u64;
                        if s.byte()? != 0x0b {
                            return Err("data offset not terminated".to_string());
                        }
                        let size = s.uleb()? as usize;
                        module.datas.push((offset, s.take(size)?.to_vec()));
                    }
                }
                other => return Err(format!("unexpected section id {}", other)),
            }
            if s.p != s.b.len() {
                return Err(format!("section {} has trailing bytes", id));
            }
        }
        Ok(module)
    }

    /// The signature of every function index (imports first).
    fn all_sigs(module: &ParsedModule) -> Result<Vec<(Vec<u8>, Vec<u8>)>, String> {
        let mut sigs = Vec::new();
        for (_, _, ty) in &module.imports {
            sigs.push(
                module
                    .types
                    .get(*ty as usize)
                    .cloned()
                    .ok_or("import type index out of range")?,
            );
        }
        for ty in &module.funcs {
            sigs.push(
                module
                    .types
                    .get(*ty as usize)
                    .cloned()
                    .ok_or("function type index out of range")?,
            );
        }
        Ok(sigs)
    }

    /// Validate structural invariants of one compiled module, then fully
    /// type-check every function body against the module's own tables.
    fn validate_module(module: &ParsedModule) -> Result<(), String> {
        if module.funcs.len() != module.code.len() {
            return Err(format!(
                "{} function entries but {} code bodies",
                module.funcs.len(),
                module.code.len()
            ));
        }
        let sigs = all_sigs(module)?;
        // elem targets exist
        let nfuncs = sigs.len() as u64;
        for (_, items) in &module.elems {
            for f in items {
                if (*f as u64) >= nfuncs {
                    return Err(format!("elem func index {} out of range", f));
                }
            }
            if items.len() as u64 != module.table_min {
                return Err("table size does not match the elem segment".to_string());
            }
        }
        // memory covers the data segments
        for (offset, bytes) in &module.datas {
            if offset + bytes.len() as u64 > module.memory_min * 65536 {
                return Err("data segment exceeds the initial memory".to_string());
            }
        }
        // _start export exists with the command signature
        let start = module
            .exports
            .iter()
            .find(|(n, k, _)| n.as_str() == "_start" && *k == 0)
            .ok_or("missing _start export")?;
        let start_idx = start.2 as usize;
        if sigs[start_idx] != (vec![], vec![]) {
            return Err("_start must have signature [] -> []".to_string());
        }
        if !module
            .exports
            .iter()
            .any(|(n, k, _)| n.as_str() == "memory" && *k == 2)
        {
            return Err("missing memory export".to_string());
        }
        if let Some(max) = module.table_max {
            if max < module.table_min {
                return Err("table max below min".to_string());
            }
        }
        for (i, (locals, body)) in module.code.iter().enumerate() {
            let fidx = module.imports.len() + i;
            validate_body(&sigs, &module.types, &module.globals, fidx, locals, body)
                .map_err(|e| format!("function {}: {}", fidx, e))?;
        }
        Ok(())
    }

    /// Validate one body emitted in-process (same checks as validate_module
    /// but sourced straight from the compiler's own tables).
    fn validate_one_body(
        compiler: &WasmCompiler<'_>,
        func_index: u32,
        declared: Vec<Val>,
        body: Vec<u8>,
    ) -> Result<(), String> {
        let sigs: Vec<(Vec<u8>, Vec<u8>)> = compiler
            .all_sigs
            .iter()
            .map(|(p, r)| {
                (
                    p.iter().map(|v| v.byte()).collect(),
                    r.iter().map(|v| v.byte()).collect(),
                )
            })
            .collect();
        let types: Vec<(Vec<u8>, Vec<u8>)> = compiler
            .types
            .iter()
            .map(|(p, r)| {
                (
                    p.iter().map(|v| v.byte()).collect(),
                    r.iter().map(|v| v.byte()).collect(),
                )
            })
            .collect();
        let globals = vec![(TI32, true), (TI64, true)];
        let local_bytes: Vec<u8> = declared.iter().map(|v| v.byte()).collect();
        validate_body(&sigs, &types, &globals, func_index as usize, &local_bytes, &body)
    }

    /// Type-check one function body. Every structured instruction the
    /// backend emits uses the empty block type, so frames only need to
    /// police stack balance, branch depths and call signatures.
    fn validate_body(
        sigs: &[(Vec<u8>, Vec<u8>)],
        types: &[(Vec<u8>, Vec<u8>)],
        globals: &[(u8, bool)],
        func_index: usize,
        declared: &[u8],
        body: &[u8],
    ) -> Result<(), String> {
        let (params, results) = sigs
            .get(func_index)
            .ok_or("function index out of range")?;
        let mut locals: Vec<u8> = params.clone();
        locals.extend_from_slice(declared);
        let mut r = Reader::new(body);

        struct Frame {
            base: usize,
            unreachable: bool,
            kind: u8,
        }
        let mut stack: Vec<Option<u8>> = Vec::new();
        let mut frames: Vec<Frame> = vec![Frame {
            base: 0,
            unreachable: false,
            kind: 0xff, // the implicit function frame
        }];

        macro_rules! underflow {
            () => {
                "value stack underflow".to_string()
            };
        }
        fn pop_impl(
            stack: &mut Vec<Option<u8>>,
            frames: &mut Vec<Frame>,
        ) -> Result<Option<u8>, String> {
            let frame = frames.last_mut().expect("function frame");
            if stack.len() == frame.base {
                // Polymorphic past a br/unreachable; no physical pop.
                if frame.unreachable {
                    return Ok(None);
                }
                return Err("value stack underflow".to_string());
            }
            Ok(stack.pop().unwrap())
        }
        macro_rules! pop_any {
            () => {
                pop_impl(&mut stack, &mut frames)?
            };
        }
        macro_rules! pop_t {
            ($t:expr) => {{
                match pop_any!() {
                    Some(got) if got != $t => {
                        return Err(format!("type mismatch: expected {:#x}, got {:#x}", $t, got));
                    }
                    _ => {}
                }
            }};
        }
        macro_rules! push {
            ($t:expr) => {
                stack.push(Some($t))
            };
        }
        macro_rules! mark_unreachable {
            () => {
                frames.last_mut().expect("frame").unreachable = true
            };
        }
        macro_rules! binop {
            ($t:expr) => {{
                pop_t!($t);
                pop_t!($t);
                push!($t);
            }};
        }
        macro_rules! cmpop {
            ($t:expr) => {{
                pop_t!($t);
                pop_t!($t);
                push!(TI32);
            }};
        }

        loop {
            let op = r.byte().map_err(|_| underflow!())?;
            match op {
                0x00 => mark_unreachable!(),                   // unreachable
                0x02 | 0x03 | 0x04 => {
                    // block / loop / if (only the empty block type is emitted)
                    if op == 0x04 {
                        pop_t!(TI32);
                    }
                    if r.byte()? != 0x40 {
                        return Err("non-empty block type".to_string());
                    }
                    frames.push(Frame {
                        base: stack.len(),
                        unreachable: false,
                        kind: op,
                    });
                }
                0x05 => {
                    // else
                    let frame = frames.last_mut().expect("frame");
                    if frame.kind != 0x04 {
                        return Err("else outside if".to_string());
                    }
                    if stack.len() != frame.base && !frame.unreachable {
                        return Err("stack imbalance at else".to_string());
                    }
                    stack.truncate(frame.base);
                    frame.unreachable = false;
                }
                0x0b => {
                    // end
                    let frame = frames.pop().expect("frame");
                    if frames.is_empty() {
                        // function level: the stack must equal the results
                        if stack.len() != results.len() && !frame.unreachable {
                            return Err(format!(
                                "result arity: expected {}, have {}",
                                results.len(),
                                stack.len()
                            ));
                        }
                        if !frame.unreachable {
                            for (i, want) in results.iter().enumerate() {
                                if let Some(got) = stack[i] {
                                    if got != *want {
                                        return Err(format!(
                                            "result {} type: expected {:#x}, got {:#x}",
                                            i, want, got
                                        ));
                                    }
                                }
                            }
                        }
                        if r.p != r.b.len() {
                            return Err("trailing bytes after function end".to_string());
                        }
                        return Ok(());
                    }
                    if stack.len() != frame.base && !frame.unreachable {
                        return Err("stack imbalance at end".to_string());
                    }
                    stack.truncate(frame.base);
                }
                0x0c => {
                    // br
                    let depth = r.uleb()? as usize;
                    if depth >= frames.len() {
                        return Err("br depth out of range".to_string());
                    }
                    mark_unreachable!();
                }
                0x0d => {
                    // br_if
                    let depth = r.uleb()? as usize;
                    pop_t!(TI32);
                    if depth >= frames.len() {
                        return Err("br_if depth out of range".to_string());
                    }
                }
                0x0e => {
                    // br_table
                    pop_t!(TI32);
                    let count = r.uleb()?;
                    let frames_now = frames.len();
                    for _ in 0..count {
                        if r.uleb()? as usize >= frames_now {
                            return Err("br_table label out of range".to_string());
                        }
                    }
                    if r.uleb()? as usize >= frames_now {
                        return Err("br_table default out of range".to_string());
                    }
                    mark_unreachable!();
                }
                0x0f => {
                    // return
                    for want in results.iter().rev() {
                        pop_t!(*want);
                    }
                    mark_unreachable!();
                }
                0x10 => {
                    // call
                    let idx = r.uleb()? as usize;
                    let (ps, rs) = sigs.get(idx).ok_or("call index out of range")?;
                    for p in ps.iter().rev() {
                        pop_t!(*p);
                    }
                    for res in rs {
                        push!(*res);
                    }
                }
                0x11 => {
                    // call_indirect
                    let ty = r.uleb()? as usize;
                    if r.byte()? != 0 {
                        return Err("call_indirect table index must be 0".to_string());
                    }
                    let (ps, rs) = types.get(ty).ok_or("call_indirect type out of range")?;
                    pop_t!(TI32);
                    for p in ps.iter().rev() {
                        pop_t!(*p);
                    }
                    for res in rs {
                        push!(*res);
                    }
                }
                0x1a => {
                    pop_any!();
                }
                0x1b => {
                    // select
                    pop_t!(TI32);
                    let a = pop_any!();
                    let b = pop_any!();
                    if let (Some(x), Some(y)) = (a, b) {
                        if x != y {
                            return Err("select arms disagree".to_string());
                        }
                    }
                    stack.push(a.or(b));
                }
                0x20 => {
                    let idx = r.uleb()? as usize;
                    push!(*locals.get(idx).ok_or("local.get out of range")?);
                }
                0x21 => {
                    let idx = r.uleb()? as usize;
                    let t = *locals.get(idx).ok_or("local.set out of range")?;
                    pop_t!(t);
                }
                0x22 => {
                    let idx = r.uleb()? as usize;
                    let t = *locals.get(idx).ok_or("local.tee out of range")?;
                    pop_t!(t);
                    push!(t);
                }
                0x23 => {
                    let idx = r.uleb()? as usize;
                    push!(globals.get(idx).ok_or("global.get out of range")?.0);
                }
                0x24 => {
                    let idx = r.uleb()? as usize;
                    let g = *globals.get(idx).ok_or("global.set out of range")?;
                    if !g.1 {
                        return Err("global.set on immutable global".to_string());
                    }
                    pop_t!(g.0);
                }
                0x28 | 0x2c | 0x2d => {
                    r.uleb()?;
                    r.uleb()?;
                    pop_t!(TI32);
                    push!(TI32);
                }
                0x29 | 0x30 | 0x31 | 0x32 | 0x33 | 0x34 | 0x35 => {
                    r.uleb()?;
                    r.uleb()?;
                    pop_t!(TI32);
                    push!(TI64);
                }
                0x2b => {
                    r.uleb()?;
                    r.uleb()?;
                    pop_t!(TI32);
                    push!(TF64);
                }
                0x36 | 0x3a | 0x3b => {
                    r.uleb()?;
                    r.uleb()?;
                    pop_t!(TI32);
                    pop_t!(TI32);
                }
                0x37 | 0x3c | 0x3d | 0x3e => {
                    r.uleb()?;
                    r.uleb()?;
                    pop_t!(TI64);
                    pop_t!(TI32);
                }
                0x39 => {
                    r.uleb()?;
                    r.uleb()?;
                    pop_t!(TF64);
                    pop_t!(TI32);
                }
                0x3f => {
                    if r.byte()? != 0 {
                        return Err("memory.size reserved byte".to_string());
                    }
                    push!(TI32);
                }
                0x40 => {
                    if r.byte()? != 0 {
                        return Err("memory.grow reserved byte".to_string());
                    }
                    pop_t!(TI32);
                    push!(TI32);
                }
                0x41 => {
                    r.sleb()?;
                    push!(TI32);
                }
                0x42 => {
                    r.sleb()?;
                    push!(TI64);
                }
                0x44 => {
                    r.take(8)?;
                    push!(TF64);
                }
                0x45 => {
                    pop_t!(TI32);
                    push!(TI32);
                }
                0x50 => {
                    pop_t!(TI64);
                    push!(TI32);
                }
                0x46..=0x4f => cmpop!(TI32),
                0x51..=0x5b => cmpop!(TI64),
                0x61..=0x66 => cmpop!(TF64),
                0x67 | 0x68 | 0x69 => {
                    pop_t!(TI32);
                    push!(TI32);
                }
                0x89 | 0x8a => {
                    pop_t!(TI64);
                    push!(TI64);
                }
                0x6a..=0x78 => binop!(TI32),
                0x7c..=0x88 => binop!(TI64),
                0xa0..=0xa6 => binop!(TF64),
                0x99..=0x9f => {
                    pop_t!(TF64);
                    push!(TF64);
                }
                0xa7 => {
                    pop_t!(TI64);
                    push!(TI32);
                }
                0xaa | 0xab => {
                    pop_t!(TF64);
                    push!(TI32);
                }
                0xac | 0xad => {
                    pop_t!(TI32);
                    push!(TI64);
                }
                0xb0 | 0xb1 => {
                    pop_t!(TF64);
                    push!(TI64);
                }
                0xb7 | 0xb8 => {
                    pop_t!(TI32);
                    push!(TF64);
                }
                0xb9 | 0xba => {
                    pop_t!(TI64);
                    push!(TF64);
                }
                0xbd => {
                    pop_t!(TF64);
                    push!(TI64);
                }
                0xbf => {
                    pop_t!(TI64);
                    push!(TF64);
                }
                0xfc => {
                    let sub = r.uleb()?;
                    match sub {
                        10 => {
                            if r.byte()? != 0 || r.byte()? != 0 {
                                return Err("memory.copy reserved bytes".to_string());
                            }
                            pop_t!(TI32);
                            pop_t!(TI32);
                            pop_t!(TI32);
                        }
                        11 => {
                            if r.byte()? != 0 {
                                return Err("memory.fill reserved byte".to_string());
                            }
                            pop_t!(TI32);
                            pop_t!(TI32);
                            pop_t!(TI32);
                        }
                        other => return Err(format!("unknown bulk op {}", other)),
                    }
                }
                other => return Err(format!("unhandled opcode {:#x}", other)),
            }
        }
    }
}
