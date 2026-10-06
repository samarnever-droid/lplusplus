//! Per-function lowering from the 5B/5C MIR slice to cranelift IR.
//!
//! The lowering matches the v1 cranelift backend's conventions:
//! cranelift *variables* for MIR locals (sound under the 4B
//! definite-initialization invariant), the 24-byte string-constant
//! header handed out at `base + 24`, hardware integer division (no
//! software trap, identical to v1), and the interpreter's exact
//! operator semantics (wrapping arithmetic, signed comparisons,
//! IEEE-754 floats, 0/1 bool lattice).
//!
//! The 5C aggregate data surface adds: managed values (structs,
//! enums, lists, strings) as `I64` heap pointers with the v1
//! freestanding ARC header in front of the payload; struct layouts
//! from the v1 `struct_layout` rule; enums with an `I64` tag at
//! offset 0; dense `SwitchEnum` dispatch on the tag; projected
//! load/store through field and list-index steps; and the ARC
//! traffic that mirrors the interpreter's consume-vs-borrow rules.
//!
//! **ARC model (retain-on-transfer).** A managed value is retained
//! exactly when a new owner is created — a `Use(Copy)`, a `Load` of a
//! managed place, a `Store` of a `Copy` into a bare local or struct
//! field, a constructed field or list element that was a `Copy` of a
//! local, a call argument, and a returned value. ARC-list element
//! stores retain inside `lpp_list_set_arc` (runtime-internal) instead.
//! A release is emitted when an owning slot is overwritten (cell
//! death, managed struct field replacement) and — at every `Return` —
//! by a release pass over all managed locals in declaration order.
//! Transfers never null their source: the source keeps its reference
//! and releases it on overwrite or exit, which balances the transfer's
//! retain. Read-after-move is impossible (the 4B verifier rejects
//! it), so the retained-then-released source is never observed.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use cranelift_codegen::entity::EntityRef;
use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{
    AbiParam, Block, FuncRef, InstBuilder, MemFlags, StackSlotData, StackSlotKind, TrapCode,
    Type as CLType, UserFuncName, Value, types as cltypes,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module};
use cranelift_object::ObjectModule;
use lpp_codegen_api::{CodegenError, CodegenErrorKind};
use lpp_mir::{
    BasicBlockId, BinaryOperator, Constant, InstructionKind, MirAggregateId, MirFunction,
    MirFunctionId, MirFunctionKind, MirLocalId, MirProgram, MirStringId, Operand, PlaceProjection,
    Rvalue, Terminator, UnaryOperator,
};
use lpp_types::{BuiltinId, PrimitiveType, TypeId, TypeInterner, TypeKind};

use crate::layout::{AggregateIndex, AggregateLayout, aggregate_layout};
use crate::{LIST_NEW, PRINT_STR};

/// The 24-byte v1 string-constant header, then payload, then NUL.
const LPP_ARC_MAGIC: u32 = 0x4152_4331;
const STRING_HEADER_OFFSET: i64 = 24;

// ── v1 runtime imports ─────────────────────────────────────────────────────
// The list-family symbols are registry entries (`BuiltinAbi` in the
// checked-in generated table); the ARC allocator/retainer/releaser are
// freestanding-runtime symbols the backend declares by name, as 5B did
// for `fmod`.

const IMP_PRINT_STR: &str = "lpp_print_str";
const IMP_FMOD: &str = "fmod";
const IMP_STR_EQ: &str = "lpp_str_eq";
const IMP_STR_CONCAT: &str = "lpp_str_concat";
const IMP_INT_TO_STR: &str = "lpp_int_to_str";
const IMP_FLOAT_TO_STR: &str = "lpp_float_to_str";
const IMP_BOOL_TO_STR: &str = "lpp_bool_to_str";
const IMP_ARC_ALLOC: &str = "lpp_arc_alloc_with_destructor";
const IMP_ARC_RETAIN: &str = "lpp_arc_retain";
const IMP_ARC_RELEASE: &str = "lpp_arc_release";
const IMP_LIST_NEW: &str = "lpp_list_new";
const IMP_LIST_NEW_ARC: &str = "lpp_list_new_arc";
const IMP_LIST_PUSH: &str = "lpp_list_push";
const IMP_LIST_PUSH_ARC: &str = "lpp_list_push_arc";
const IMP_LIST_PUSH_FLOAT: &str = "lpp_list_push_float";
const IMP_LIST_PUSH_BOOL: &str = "lpp_list_push_bool";
const IMP_LIST_GET: &str = "lpp_list_get";
const IMP_LIST_GET_ARC: &str = "lpp_list_get_arc";
const IMP_LIST_GET_FLOAT: &str = "lpp_list_get_float";
const IMP_LIST_GET_BOOL: &str = "lpp_list_get_bool";
const IMP_LIST_SET: &str = "lpp_list_set";
const IMP_LIST_SET_ARC: &str = "lpp_list_set_arc";
const IMP_LIST_SET_FLOAT: &str = "lpp_list_set_float";
const IMP_LIST_SET_BOOL: &str = "lpp_list_set_bool";
const IMP_LIST_LEN: &str = "lpp_list_len";
const IMP_CLOSURE_DESTROY: &str = "lpp_closure_destroy";
const IMP_TUPLE_ALLOC: &str = "lpp_tuple_alloc";
const IMP_TASK_NEW: &str = "lpp_task_new";
const IMP_TASK_POLL: &str = "lpp_task_poll";
const IMP_TASK_AWAIT: &str = "lpp_task_await";
const IMP_TASK_DESTROY: &str = "lpp_task_destroy";
const IMP_PRINT_INT: &str = "lpp_print_int";
const IMP_PRINT_FLOAT: &str = "lpp_print_float";
const IMP_PRINT_BOOL: &str = "lpp_print_bool";
const IMP_VEC_CHECKSUM: &str = "lpp_vec_i64_checksum";

/// The machine signature of one runtime import.
/// The oracle-supported builtin names (Family A, 71 at this commit):
/// the `eval_builtin` arms of the ARC oracle. The gate recomputes the
/// census from the registry; any registry drift fails the gate.
const FAMILY_A: &[&str] = &[
    "print",
    "print_str",
    "eprint_str",
    "print_int",
    "print_float",
    "input",
    "print_bool",
    "write_str",
    "str_concat",
    "str_len",
    "str_contains",
    "str_starts_with",
    "str_ends_with",
    "str_find",
    "str_replace",
    "char_at",
    "chr",
    "ord",
    "str_substr",
    "str_trim",
    "str_to_lower",
    "str_lower",
    "str_to_upper",
    "str_upper",
    "int_to_str",
    "str_to_int",
    "parse_int",
    "float_to_str",
    "bool_to_str",
    "u64_to_str",
    "u64_to_hex",
    "str_to_u64",
    "abs",
    "min",
    "max",
    "min_u",
    "max_u",
    "lt_u",
    "le_u",
    "gt_u",
    "ge_u",
    "shr_u",
    "shl_u",
    "div_u",
    "rem_u",
    "popcount64",
    "clz64",
    "ctz64",
    "bswap16",
    "bswap32",
    "bswap64",
    "rotl64",
    "rotr64",
    "rotl32",
    "rotr32",
    "trunc_u8",
    "trunc_u16",
    "trunc_u32",
    "trunc_i8",
    "trunc_i16",
    "trunc_i32",
    "prefetch",
    "prefetch_read",
    "prefetch_write",
    "add_checked",
    "sub_checked",
    "mul_checked",
    "add_wrap",
    "sub_wrap",
    "mul_wrap",
    "floor",
    "ceil",
    "pow",
    "sqrt",
    "fmod",
    "sin",
    "cos",
    "int_pow",
    "str_repeat",
    "str_split",
    // IO file-ops slice (E5003): plain runtime-symbol calls. Each takes/returns
    // `Str` as an `i64` pointer and `Int` as `i64`; none is a bool predicate, so
    // the FAMILY_A call path returns their `i64` result unreduced.
    "read_file",
    "write_file",
    "append_file",
    "delete_file",
    "file_exists",
    "file_size",
    "file_copy",
    "file_move",
    "dir_create",
    "dir_remove",
    "path_exists",
    "command_output",
    // Time/random and host-concurrency builtins. These are direct frozen ABI
    // runtime-symbol calls; handles are opaque i64 process-local pointers.
    "time_ms",
    "lpp_time_ms",
    "random",
    "lpp_random",
    "random_range",
    "lpp_random_range",
    "random_seed",
    "lpp_random_seed",
    "rng_new",
    "rng_next",
    "rng_range",
    "rng_float",
    "rng_free",
    "clock_new",
    "clock_now",
    "clock_advance",
    "clock_free",
    "atomic_new",
    "atomic_free",
    "atomic_load",
    "atomic_load_acq",
    "atomic_load_relaxed",
    "atomic_store",
    "atomic_store_rel",
    "atomic_store_relaxed",
    "atomic_add",
    "atomic_sub",
    "atomic_and",
    "atomic_or",
    "atomic_xor",
    "atomic_swap",
    "atomic_cas",
    "atomic_cas_weak",
    "atomic_load32",
    "atomic_store32",
    "atomic_add32",
    "atomic_cas32",
    "atomic_fence",
    "atomic_fence_acq",
    "atomic_fence_rel",
    "cpu_pause",
    "mutex_new",
    "mutex_lock",
    "mutex_trylock",
    "mutex_unlock",
    "mutex_free",
    "rwlock_new",
    "rwlock_rdlock",
    "rwlock_wrlock",
    "rwlock_rdunlock",
    "rwlock_wrunlock",
    "rwlock_free",
    "cpu_count",
    "thread_spawn",
    "lpp_thread_spawn",
    "thread_join",
    "thread_pin",
    "thread_id",
    "list_new",
    "list_push",
    "list_get",
    "list_set",
    "list_len",
    "len",
    // List slice (E5003): all lowering params/results are i64/void, so they take
    // the generic FAMILY_A runtime-symbol call path (no per-element-class
    // dispatch — that is only needed for push/get/set, handled above).
    "list_insert",
    "list_remove",
    "list_swap",
    "list_reverse",
    "list_reserve",
    "list_capacity",
    "list_truncate",
    "list_clear",
    "list_sort",
    "list_sort_desc",
    "list_sort_u",
    "list_index_of",
    "list_binary_search",
    "list_extend",
    // Registry-declared alias spellings. The ABI table exposes several builtins
    // under their C symbol names as well (entry pairs such as print_int /
    // lpp_print_int). Each already has an `import_signature` arm and a runtime
    // symbol; FAMILY_A merely omitted the `lpp_` spelling, so source that used
    // it hit E5003. `lpp_str_eq` has no clean twin in the table at all, so it is
    // a first-class spelling rather than a redundant alias.
    "lpp_print_int",
    "lpp_print_float",
    "lpp_abs",
    "lpp_str_eq",
    "lpp_pow",
    "lpp_min",
    "lpp_max",
    "lpp_floor",
    "lpp_ceil",
];

/// The slice-handle builtin names (Family C, 7 at this commit).
const FAMILY_C: &[&str] = &[
    "slice",
    "str_slice",
    "slice_len",
    "slice_get",
    "lpp_slice_get_bool",
    "slice_to_str",
    "str_slice_to_str",
];

fn import_signature(name: &str) -> (&'static [CLType], Option<CLType>) {
    match name {
        IMP_PRINT_STR => (&[cltypes::I64], None),
        IMP_FMOD => (&[cltypes::F64, cltypes::F64], Some(cltypes::F64)),
        IMP_ARC_ALLOC => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        IMP_ARC_RETAIN | IMP_ARC_RELEASE => (&[cltypes::I64], None),
        IMP_LIST_NEW | IMP_LIST_NEW_ARC => (&[], Some(cltypes::I64)),
        IMP_LIST_PUSH | IMP_LIST_PUSH_ARC => (&[cltypes::I64, cltypes::I64], None),
        IMP_LIST_PUSH_FLOAT => (&[cltypes::I64, cltypes::F64], None),
        IMP_LIST_PUSH_BOOL => (&[cltypes::I64, cltypes::I8], None),
        IMP_LIST_GET | IMP_LIST_GET_ARC => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        IMP_LIST_GET_FLOAT => (&[cltypes::I64, cltypes::I64], Some(cltypes::F64)),
        IMP_LIST_GET_BOOL => (&[cltypes::I64, cltypes::I64], Some(cltypes::I8)),
        IMP_LIST_SET | IMP_LIST_SET_ARC => (&[cltypes::I64, cltypes::I64, cltypes::I64], None),
        IMP_LIST_SET_FLOAT => (&[cltypes::I64, cltypes::I64, cltypes::F64], None),
        IMP_LIST_SET_BOOL => (&[cltypes::I64, cltypes::I64, cltypes::I8], None),
        IMP_LIST_LEN => (&[cltypes::I64], Some(cltypes::I64)),
        // List slice runtime symbols.
        "lpp_list_insert" => (&[cltypes::I64, cltypes::I64, cltypes::I64], None),
        "lpp_list_remove" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_list_swap" => (&[cltypes::I64, cltypes::I64, cltypes::I64], None),
        "lpp_list_reverse" => (&[cltypes::I64], None),
        "lpp_list_reserve" => (&[cltypes::I64, cltypes::I64], None),
        "lpp_list_capacity" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_list_truncate" => (&[cltypes::I64, cltypes::I64], None),
        "lpp_list_clear" => (&[cltypes::I64], None),
        "lpp_list_sort" => (&[cltypes::I64], None),
        "lpp_list_sort_desc" => (&[cltypes::I64], None),
        "lpp_list_sort_u" => (&[cltypes::I64], None),
        "lpp_list_index_of" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_list_binary_search" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_list_extend" => (&[cltypes::I64, cltypes::I64], None),
        "lpp_eprint_str" => (&[cltypes::I64], None),
        "lpp_slice_init" => (&[cltypes::I64; 5], Some(cltypes::I64)),
        "lpp_slice_len" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_slice_get" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_str_slice_get" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_slice_get_bool" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I8)),
        "lpp_str_slice_to_str" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_print_int" => (&[cltypes::I64], None),
        "lpp_print_float" => (&[cltypes::F64], None),
        "lpp_print_bool" => (&[cltypes::I8], None),
        "lpp_write_str" => (&[cltypes::I64], None),
        "lpp_str_concat" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_str_len" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_str_contains" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_str_starts_with" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_str_ends_with" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_str_find" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_str_replace" => (
            &[cltypes::I64, cltypes::I64, cltypes::I64],
            Some(cltypes::I64),
        ),
        "lpp_char_at" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_chr" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_ord" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_str_substr" => (
            &[cltypes::I64, cltypes::I64, cltypes::I64],
            Some(cltypes::I64),
        ),
        "lpp_str_trim" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_str_eq" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_str_lower" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_str_upper" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_int_to_str" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_str_to_int" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_parse_int" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_input" => (&[], Some(cltypes::I64)),
        // IO file-ops slice: str/int in, str/int out (pointers are I64).
        "lpp_read_file" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_write_file" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_append_file" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_delete_file" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_file_exists" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_file_size" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_file_copy" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_file_move" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_dir_create" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_dir_remove" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_path_exists" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_command_output" => (&[cltypes::I64], Some(cltypes::I64)),
        // Time/random/clock.
        "lpp_time_ms" | "lpp_random" | "lpp_cpu_count" | "lpp_thread_id" => {
            (&[], Some(cltypes::I64))
        }
        "lpp_random_seed"
        | "lpp_rng_free"
        | "lpp_clock_free"
        | "lpp_atomic_free"
        | "lpp_mutex_lock"
        | "lpp_mutex_unlock"
        | "lpp_mutex_free"
        | "lpp_rwlock_rdlock"
        | "lpp_rwlock_wrlock"
        | "lpp_rwlock_rdunlock"
        | "lpp_rwlock_wrunlock"
        | "lpp_rwlock_free" => (&[cltypes::I64], None),
        "lpp_atomic_fence" | "lpp_atomic_fence_acq" | "lpp_atomic_fence_rel" | "lpp_cpu_pause" => {
            (&[], None)
        }
        "lpp_clock_advance"
        | "lpp_atomic_store"
        | "lpp_atomic_store_rel"
        | "lpp_atomic_store_relaxed"
        | "lpp_atomic_store32" => (&[cltypes::I64, cltypes::I64], None),
        "lpp_random_range" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_rng_new"
        | "lpp_rng_next"
        | "lpp_clock_new"
        | "lpp_clock_now"
        | "lpp_atomic_new"
        | "lpp_atomic_load"
        | "lpp_atomic_load_acq"
        | "lpp_atomic_load_relaxed"
        | "lpp_atomic_load32"
        | "lpp_mutex_trylock"
        | "lpp_thread_join"
        | "lpp_thread_pin" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_rng_range" | "lpp_atomic_cas" | "lpp_atomic_cas_weak" | "lpp_atomic_cas32" => (
            &[cltypes::I64, cltypes::I64, cltypes::I64],
            Some(cltypes::I64),
        ),
        "lpp_rng_float" => (&[cltypes::I64], Some(cltypes::F64)),
        "lpp_atomic_add" | "lpp_atomic_sub" | "lpp_atomic_and" | "lpp_atomic_or"
        | "lpp_atomic_xor" | "lpp_atomic_swap" | "lpp_atomic_add32" | "lpp_thread_spawn" => {
            (&[cltypes::I64, cltypes::I64], Some(cltypes::I64))
        }
        "lpp_mutex_new" | "lpp_rwlock_new" => (&[], Some(cltypes::I64)),
        // Network / HTTP runtime (frozen v1 ABI, now backed by the Rust runtime).
        "lpp_net_dial" | "lpp_net_dial_udp" => (
            &[cltypes::I64, cltypes::I64, cltypes::I64],
            Some(cltypes::I64),
        ),
        "lpp_net_listen" | "lpp_net_listen_udp" | "lpp_net_accept" => {
            (&[cltypes::I64], Some(cltypes::I64))
        }
        "lpp_net_accept_timeout"
        | "lpp_net_send"
        | "lpp_net_send_all"
        | "lpp_net_recv"
        | "lpp_net_recv_udp"
        | "lpp_net_set_timeout"
        | "lpp_net_set_nonblocking"
        | "lpp_net_poll"
        | "lpp_net_connect" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_net_close" => (&[cltypes::I64], None),
        "lpp_net_set_deadline" => (
            &[cltypes::I64, cltypes::I64, cltypes::I64],
            Some(cltypes::I64),
        ),
        "lpp_net_set_keepalive" => (
            &[
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
                cltypes::I64,
            ],
            Some(cltypes::I64),
        ),
        "lpp_net_resolve" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_http_get" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_http_post" => (
            &[cltypes::I64, cltypes::I64, cltypes::I64, cltypes::I64],
            Some(cltypes::I64),
        ),
        "lpp_float_to_str" => (&[cltypes::F64], Some(cltypes::I64)),
        "lpp_bool_to_str" => (&[cltypes::I8], Some(cltypes::I64)),
        "lpp_u64_to_str" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_u64_to_hex" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_str_to_u64" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_abs" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_min" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_max" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_min_u" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_max_u" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_lt_u" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_le_u" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_gt_u" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_ge_u" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_shr_u" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_shl_u" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_div_u" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_rem_u" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_popcount64" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_clz64" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_ctz64" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_bswap16" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_bswap32" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_bswap64" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_rotl64" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_rotr64" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_rotl32" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_rotr32" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_trunc_u8" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_trunc_u16" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_trunc_u32" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_trunc_i8" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_trunc_i16" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_trunc_i32" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_prefetch" | "lpp_prefetch_write" => (&[cltypes::I64], None),
        "lpp_add_checked" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_sub_checked" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_mul_checked" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_add_wrap" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_sub_wrap" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_mul_wrap" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_floor" => (&[cltypes::F64], Some(cltypes::F64)),
        "lpp_ceil" => (&[cltypes::F64], Some(cltypes::F64)),
        "lpp_pow" => (&[cltypes::F64, cltypes::F64], Some(cltypes::F64)),
        "lpp_sqrt" => (&[cltypes::F64], Some(cltypes::F64)),
        "lpp_sin" => (&[cltypes::F64], Some(cltypes::F64)),
        "lpp_cos" => (&[cltypes::F64], Some(cltypes::F64)),
        "lpp_int_pow" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_str_repeat" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_str_split" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_vec_i64_checksum" => (&[cltypes::I64], Some(cltypes::I64)),
        IMP_CLOSURE_DESTROY => (&[cltypes::I64], None),
        IMP_TUPLE_ALLOC => (&[cltypes::I64; 3], Some(cltypes::I64)),
        IMP_TASK_NEW => (&[cltypes::I64; 3], Some(cltypes::I64)),
        IMP_TASK_POLL => (&[cltypes::I64], Some(cltypes::I64)),
        IMP_TASK_AWAIT => (&[cltypes::I64], Some(cltypes::I64)),
        IMP_TASK_DESTROY => (&[cltypes::I64], None),
        // Hash map runtime (Family MAP). All handles/keys/values pass as i64;
        // the `_str` key variants take the key pointer as i64 too, and the
        // `_float` value variants use f64 for the value slot.
        "lpp_map_new" | "lpp_map_new_arc" => (&[], Some(cltypes::I64)),
        "lpp_map_put" => (&[cltypes::I64, cltypes::I64, cltypes::I64], None),
        "lpp_map_put_str" => (&[cltypes::I64, cltypes::I64, cltypes::I64], None),
        "lpp_map_put_float" => (&[cltypes::I64, cltypes::I64, cltypes::F64], None),
        "lpp_map_put_str_float" => (&[cltypes::I64, cltypes::I64, cltypes::F64], None),
        "lpp_map_get" | "lpp_map_get_str" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_map_get_float" | "lpp_map_get_str_float" => {
            (&[cltypes::I64, cltypes::I64], Some(cltypes::F64))
        }
        "lpp_map_has" | "lpp_map_has_str" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_map_remove" | "lpp_map_remove_str" => (&[cltypes::I64, cltypes::I64], None),
        "lpp_map_len" | "lpp_map_capacity" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_map_keys" | "lpp_map_values" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_map_clear" => (&[cltypes::I64], None),
        _ => unreachable!("import signatures are a closed set"),
    }
}

/// Family MAP: the hash map builtins. Every spelling — bare (`map_put`) or the
/// `lpp_`-prefixed alias (`lpp_map_put`) — routes to a `lpp_map_*` runtime
/// symbol. They lower like Family A (a direct call to the descriptor's symbol),
/// separated only so the family census stays legible.
fn is_family_map(name: &str) -> bool {
    name.starts_with("map_") || name.starts_with("lpp_map_")
}

/// Whether a map builtin returns a boolean membership flag. The frozen ABI
/// spells the result as `i64`, but the checker types it as `Bool`, so codegen
/// must reduce the runtime's `i64` 0/1 to a machine `i8`.
fn is_map_predicate(name: &str) -> bool {
    matches!(
        name,
        "map_has" | "map_has_str" | "lpp_map_has" | "lpp_map_has_str"
    )
}

/// Family NET: frozen v1 socket/HTTP builtins that are representable on native
/// targets through the Rust runtime. Both bare and `lpp_` spellings lower to
/// the same `lpp_*` symbol recorded in the ABI descriptor.
fn is_family_net(name: &str) -> bool {
    let name = name.strip_prefix("lpp_").unwrap_or(name);
    matches!(
        name,
        "net_dial"
            | "net_dial_udp"
            | "net_listen"
            | "net_listen_udp"
            | "net_accept"
            | "net_accept_timeout"
            | "net_send"
            | "net_send_all"
            | "net_recv"
            | "net_recv_udp"
            | "net_close"
            | "net_set_deadline"
            | "net_set_timeout"
            | "net_set_nonblocking"
            | "net_poll"
            | "net_set_keepalive"
            | "net_resolve"
            | "net_connect"
            | "http_get"
            | "http_post"
    )
}

/// The pre-scan's result: what to declare, in what order.
pub(crate) struct Plan {
    /// Export symbol per function (a source-level `main` exports as
    /// `lpp_main`; the C-ABI `main` wrapper is generated separately).
    pub(crate) export_names: BTreeMap<MirFunctionId, String>,
    /// Runtime imports, in stable (lexicographic) order.
    pub(crate) imports: BTreeSet<&'static str>,
    /// Nominal aggregates that are constructed, in `MirAggregateId`
    /// order — one generated destructor each.
    pub(crate) destructors: Vec<MirAggregateId>,
    /// Closures with at least one capture, in `MirFunctionId` order —
    /// one generated env destructor (`lpp_drop_c{n}`) each, releasing
    /// the managed capture slots.
    pub(crate) closure_dtors: Vec<MirFunctionId>,
    /// Async functions whose task thunk is used, in `MirFunctionId`
    /// order (`__lpp_task_thunk{n}`).
    pub(crate) task_thunks: Vec<MirFunctionId>,
    /// Zero-user-parameter closures, in `MirFunctionId` order — the
    /// capsule code pointer for each is the generated task-code
    /// trampoline (`__lpp_closure_thunk{n}`), because
    /// `lpp_task_new`'s code pointer must have the
    /// `int64_t (*)(void*)` shape while the closure function returns
    /// its user result type.
    pub(crate) closure_thunks: Vec<MirFunctionId>,
    /// Provenance class of every function-value local (closure vs
    /// materialized function): the two capsule call ABIs differ by
    /// the leading `env` parameter, so a call site needs the class.
    pub(crate) value_classes: BTreeMap<(MirFunctionId, MirLocalId), ValueClass>,
    /// Nominal type id to aggregate instance (place resolution).
    pub(crate) aggregates: AggregateIndex,
    /// Layouts for every nominal in the program.
    pub(crate) layouts: HashMap<MirAggregateId, AggregateLayout>,
}

fn err(function: MirFunctionId, kind: CodegenErrorKind) -> CodegenError {
    CodegenError::new(Some(function), kind)
}

fn unsupported(function: MirFunctionId, construct: &'static str) -> CodegenError {
    err(
        function,
        CodegenErrorKind::UnsupportedConstruct { construct },
    )
}

fn verify_failed(function: Option<MirFunctionId>, what: String) -> CodegenError {
    CodegenError::new(function, CodegenErrorKind::IrVerificationFailed(what))
}

fn emission_failed(what: String) -> CodegenError {
    CodegenError::new(None, CodegenErrorKind::ObjectEmissionFailed(what))
}

/// The machine type of a value type, or `None` for the value types the
/// slice does not represent. Managed values (strings, lists, nominals)
/// are heap pointers; `Void` is handled by the caller.
fn machine_type(types: &TypeInterner, ty: TypeId) -> Option<CLType> {
    Some(match types.kind(ty) {
        TypeKind::Primitive(PrimitiveType::Int) => cltypes::I64,
        TypeKind::Primitive(PrimitiveType::Float) => cltypes::F64,
        TypeKind::Primitive(PrimitiveType::Bool)
        | TypeKind::Primitive(PrimitiveType::U8)
        | TypeKind::Primitive(PrimitiveType::I8) => cltypes::I8,
        TypeKind::Primitive(PrimitiveType::U16) | TypeKind::Primitive(PrimitiveType::I16) => {
            cltypes::I16
        }
        TypeKind::Primitive(PrimitiveType::String) => cltypes::I64,
        TypeKind::Primitive(PrimitiveType::Char)
        | TypeKind::Primitive(PrimitiveType::U32)
        | TypeKind::Primitive(PrimitiveType::I32) => cltypes::I32,
        TypeKind::List(_) => cltypes::I64,
        // A slice view is a pointer to a (stack-allocated) view record.
        TypeKind::Slice(_) => cltypes::I64,
        TypeKind::Nominal { .. } => cltypes::I64,
        TypeKind::Task(_) => cltypes::I64,
        TypeKind::Function { .. } => cltypes::I64,
        // A tuple value is a pointer to its flat heap record (see `alloc_tuple`).
        TypeKind::Tuple(_) => cltypes::I64,
        TypeKind::Primitive(PrimitiveType::VectorI64x2) => cltypes::I64X2,
        TypeKind::Primitive(PrimitiveType::StrSlice) => cltypes::I64,
        _ => return None,
    })
}

fn is_void_type(types: &TypeInterner, ty: TypeId) -> bool {
    matches!(types.kind(ty), TypeKind::Primitive(PrimitiveType::Void))
}

/// Whether a value type is ARC-managed (a v1 runtime heap object with a
/// 24-byte header in front of its payload).
fn is_managed(types: &TypeInterner, ty: TypeId) -> bool {
    matches!(
        types.kind(ty),
        TypeKind::Primitive(PrimitiveType::String)
            | TypeKind::List(_)
            | TypeKind::Nominal { .. }
            | TypeKind::Task(_)
            | TypeKind::Function { .. }
    )
}

/// The typed-rejection label for a non-slice type.
fn unsupported_construct_for(kind: TypeKind) -> &'static str {
    match kind {
        TypeKind::Error => "error type",
        TypeKind::Never => "never type",
        TypeKind::Tuple(_) => "tuple",
        TypeKind::Map { .. } => "map",
        TypeKind::Slice(_) => "slice",
        TypeKind::Task(_) => "task",
        TypeKind::Function { .. } => "function value",
        TypeKind::List(_) => "list",
        TypeKind::Nominal { .. } => "nominal",
        TypeKind::GenericParameter(_) => "generic parameter",
        TypeKind::BoundVariable(_) => "bound variable",
        TypeKind::InferenceVariable(_) => "inference variable",
        TypeKind::UnresolvedName { .. } => "unresolved name",
        TypeKind::Primitive(primitive) => match primitive {
            PrimitiveType::Void => "void",
            PrimitiveType::U8
            | PrimitiveType::U16
            | PrimitiveType::U32
            | PrimitiveType::I8
            | PrimitiveType::I16
            | PrimitiveType::I32 => "narrow primitive",
            PrimitiveType::StrSlice => "str slice",
            PrimitiveType::VectorI64x2 => "vector",
            _ => unreachable!("the slice primitives are mapped"),
        },
    }
}

fn check_value_type(
    types: &TypeInterner,
    ty: TypeId,
    function: MirFunctionId,
) -> Result<CLType, CodegenError> {
    machine_type(types, ty)
        .ok_or_else(|| unsupported(function, unsupported_construct_for(types.kind(ty))))
}

/// The element class of a `List` for runtime-op dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ElementClass {
    Scalar,
    Float,
    Bool,
    Managed,
}

fn element_class(types: &TypeInterner, element: TypeId) -> ElementClass {
    match types.kind(element) {
        TypeKind::Primitive(PrimitiveType::Float) => ElementClass::Float,
        TypeKind::Primitive(PrimitiveType::Bool) => ElementClass::Bool,
        TypeKind::Primitive(PrimitiveType::String)
        | TypeKind::List(_)
        | TypeKind::Nominal { .. }
        | TypeKind::Function { .. } => ElementClass::Managed,
        TypeKind::Primitive(
            PrimitiveType::Int
            | PrimitiveType::Char
            | PrimitiveType::U8
            | PrimitiveType::U16
            | PrimitiveType::U32
            | PrimitiveType::I8
            | PrimitiveType::I16
            | PrimitiveType::I32,
        ) => ElementClass::Scalar,
        _ => unreachable!("list elements are checked by the pre-scan"),
    }
}

/// Validate that `element` is a type a runtime list can actually store.
///
/// This is deliberately stricter than `check_value_type`: a SIMD vector is a
/// perfectly valid machine value (`I64X2`) and a tuple/map is a valid value
/// type elsewhere, but the runtime list ops only cover scalar / float / bool /
/// managed (ARC) slots — there is no list storage class for a vector. Rejecting
/// an unstoreable element here, at the construction site in the pre-scan, turns
/// what used to be an `unreachable!` panic in `element_class` into a clean typed
/// `E5001` rejection. The accepted set mirrors `element_class` exactly.
fn check_list_element(
    types: &TypeInterner,
    element: TypeId,
    function: MirFunctionId,
) -> Result<(), CodegenError> {
    match types.kind(element) {
        TypeKind::Primitive(
            PrimitiveType::Int
            | PrimitiveType::Char
            | PrimitiveType::U8
            | PrimitiveType::U16
            | PrimitiveType::U32
            | PrimitiveType::I8
            | PrimitiveType::I16
            | PrimitiveType::I32
            | PrimitiveType::Float
            | PrimitiveType::Bool
            | PrimitiveType::String,
        )
        | TypeKind::List(_)
        | TypeKind::Nominal { .. }
        | TypeKind::Function { .. } => Ok(()),
        other => Err(unsupported(function, unsupported_construct_for(other))),
    }
}

/// Whether a language type is a function value or a list of function
/// values (both carry the provenance class of what they hold).
fn is_function_value_kind(types: &TypeInterner, kind: TypeKind) -> bool {
    match kind {
        TypeKind::Function { .. } => true,
        TypeKind::List(element) => {
            matches!(types.kind(element), TypeKind::Function { .. })
        }
        _ => false,
    }
}

/// The machine type of an operand in a value position.
fn operand_type(
    program: &MirProgram,
    types: &TypeInterner,
    operand: &Operand,
    function: MirFunctionId,
) -> Result<CLType, CodegenError> {
    match operand {
        Operand::Copy(local) => {
            let ty = program
                .local(*local)
                .expect("validated MIR retains every local")
                .ty;
            check_value_type(types, ty, function)
        }
        Operand::Constant(constant) => Ok(match constant {
            Constant::Integer(_) => cltypes::I64,
            Constant::FloatBits(_) => cltypes::F64,
            Constant::Bool(_) => cltypes::I8,
            Constant::Character { .. } => cltypes::I32,
            Constant::String { .. } => cltypes::I64,
        }),
        Operand::Function(function_id) => {
            let callee = program
                .function(*function_id)
                .expect("validated MIR retains functions");
            check_value_type(types, callee.ty, function)
        }
    }
}

/// The language type kind of an operand (the machine type alone
/// cannot tell a string pointer from an integer).
fn operand_language_kind(
    program: &MirProgram,
    types: &TypeInterner,
    operand: &Operand,
) -> Option<TypeKind> {
    Some(match operand {
        Operand::Copy(local) => types.kind(
            program
                .local(*local)
                .expect("validated MIR retains every local")
                .ty,
        ),
        Operand::Constant(constant) => match constant {
            Constant::Integer(_) | Constant::Character { .. } => {
                TypeKind::Primitive(PrimitiveType::Int)
            }
            Constant::FloatBits(_) => TypeKind::Primitive(PrimitiveType::Float),
            Constant::Bool(_) => TypeKind::Primitive(PrimitiveType::Bool),
            Constant::String { .. } => TypeKind::Primitive(PrimitiveType::String),
        },
        Operand::Function(_) => return None,
    })
}

// ── place resolution ───────────────────────────────────────────────────────
//
// A place's projection chain is resolved once (in the pre-scan) into
// concrete steps: each field step carries its payload offset, each
// list step its element type and index operand. `Downcast` contributes
// no step: the dispatch that justifies it is the dominating
// `SwitchEnum`.

#[derive(Debug, Clone, Copy)]
enum PlaceStep {
    /// A struct field (or an enum field through a downcast) at `offset`
    /// within the current base payload.
    Field {
        offset: u32,
        ty: TypeId,
        managed: bool,
    },
    /// A list element (the index operand is checked by the pre-scan).
    ListIndex { element: TypeId, index: Operand },
}

struct PlaceResolution {
    root: MirLocalId,
    steps: Vec<PlaceStep>,
    ty: TypeId,
}

/// Every `ListIndex` step of a resolved place is a runtime element
/// read: the final step of a `Load`, and every intermediate step of a
/// load or store chain. Record the matching `lpp_list_get*` import.
fn record_chain_gets(
    types: &TypeInterner,
    resolution: &PlaceResolution,
    imports: &mut BTreeSet<&'static str>,
) {
    for step in resolution.steps.iter() {
        if let PlaceStep::ListIndex { element, .. } = step {
            match element_class(types, *element) {
                ElementClass::Managed => {
                    imports.insert(IMP_LIST_GET_ARC);
                }
                ElementClass::Scalar => {
                    imports.insert(IMP_LIST_GET);
                }
                ElementClass::Float => {
                    imports.insert(IMP_LIST_GET_FLOAT);
                }
                ElementClass::Bool => {
                    imports.insert(IMP_LIST_GET_BOOL);
                }
            }
        }
    }
}

/// Resolve `place` into root + concrete steps, computing every field
/// offset from the program's own aggregate descriptors.
fn resolve_place(
    program: &MirProgram,
    types: &TypeInterner,
    place: lpp_mir::MirPlaceId,
    aggregates: &AggregateIndex,
    layouts: &HashMap<MirAggregateId, AggregateLayout>,
    mir_id: MirFunctionId,
    for_store: bool,
) -> Result<PlaceResolution, CodegenError> {
    let descriptor = program.place(place).expect("validated MIR retains places");
    let projections: Vec<PlaceProjection> = program.place_projections(descriptor).to_vec();
    if for_store && matches!(projections.last(), Some(PlaceProjection::Downcast(_))) {
        return Err(unsupported(mir_id, "store through downcast"));
    }

    let mut current_ty = program
        .local(descriptor.root)
        .expect("validated MIR retains locals")
        .ty;
    let mut steps = Vec::new();
    // The enum variant made active by the most recent downcast.
    let mut active_enum: Option<(MirAggregateId, u32 /* variant ordinal */)> = None;

    for (position, projection) in projections.iter().enumerate() {
        let is_last = position + 1 == projections.len();
        match projection {
            PlaceProjection::Downcast(variant) => {
                let TypeKind::Nominal { .. } = types.kind(current_ty) else {
                    return Err(unsupported(mir_id, "downcast of non-nominal"));
                };
                let aggregate = aggregates
                    .aggregate_for(current_ty)
                    .ok_or_else(|| unsupported(mir_id, "unmapped nominal type"))?;
                let agg = program
                    .aggregate(aggregate)
                    .expect("validated MIR retains every aggregate");
                if agg.kind != lpp_mir::MirAggregateKind::Enum {
                    return Err(unsupported(mir_id, "downcast of struct"));
                }
                let variant = program
                    .variant(*variant)
                    .expect("validated MIR retains every variant");
                if variant.aggregate != aggregate {
                    return Err(unsupported(mir_id, "downcast aggregate mismatch"));
                }
                // A downcast is a no-op at the machine level: the
                // dominating SwitchEnum already dispatched on the tag.
                active_enum = Some((aggregate, variant.ordinal));
            }
            PlaceProjection::Field(field) => {
                let TypeKind::Nominal { .. } = types.kind(current_ty) else {
                    return Err(unsupported(mir_id, "field of non-nominal"));
                };
                let aggregate = aggregates
                    .aggregate_for(current_ty)
                    .ok_or_else(|| unsupported(mir_id, "unmapped nominal type"))?;
                let layout = layouts
                    .get(&aggregate)
                    .expect("pre-scan laid out every nominal");
                let slots = match layout.kind {
                    lpp_mir::MirAggregateKind::Struct => &layout.struct_fields,
                    lpp_mir::MirAggregateKind::Enum => {
                        let (enum_aggregate, ordinal) = active_enum
                            .take()
                            .ok_or_else(|| unsupported(mir_id, "enum field without downcast"))?;
                        if enum_aggregate != aggregate {
                            return Err(unsupported(mir_id, "downcast aggregate mismatch"));
                        }
                        &layout.variant_fields[ordinal as usize]
                    }
                };
                let slot = slots
                    .iter()
                    .find(|slot| slot.field == *field)
                    .ok_or_else(|| unsupported(mir_id, "unknown field offset"))?;
                let managed = is_managed(types, slot.ty);
                if !is_last && !managed {
                    return Err(unsupported(mir_id, "projection through scalar field"));
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
                    return Err(unsupported(mir_id, "index of non-list"));
                };
                match index {
                    Operand::Copy(local) => {
                        let ty = program
                            .local(*local)
                            .expect("validated MIR retains every local")
                            .ty;
                        if types.kind(ty) != TypeKind::Primitive(PrimitiveType::Int) {
                            return Err(unsupported(mir_id, "non-int list index"));
                        }
                    }
                    Operand::Constant(Constant::Integer(_)) => {}
                    _ => return Err(unsupported(mir_id, "non-int list index")),
                }
                if !is_last && !is_managed(types, element) {
                    return Err(unsupported(
                        mir_id,
                        "projection through scalar list element",
                    ));
                }
                steps.push(PlaceStep::ListIndex {
                    element,
                    index: *index,
                });
                current_ty = element;
            }
            PlaceProjection::TupleField(index) => {
                let Some((slots, _)) = crate::layout::tuple_layout(types, current_ty) else {
                    return Err(unsupported(mir_id, "tuple field of non-tuple"));
                };
                let (offset, element) = slots
                    .get(*index as usize)
                    .copied()
                    .ok_or_else(|| unsupported(mir_id, "tuple field out of range"))?;
                let managed = is_managed(types, element);
                if !is_last && !managed {
                    return Err(unsupported(mir_id, "projection through scalar tuple field"));
                }
                steps.push(PlaceStep::Field {
                    offset,
                    ty: element,
                    managed,
                });
                current_ty = element;
            }
        }
    }

    // A bare local (no steps) or a field/list-index target is a valid
    // store site; a trailing downcast was rejected above, and tuple
    // projections never appear.
    if current_ty != descriptor.ty {
        return Err(unsupported(mir_id, "place type mismatch"));
    }
    Ok(PlaceResolution {
        root: descriptor.root,
        steps,
        ty: current_ty,
    })
}

/// Walk the whole program and type-reject anything outside the slice
/// before the module holds any declaration: a backend either lowers
/// the program completely or reports the exact first rejection.
pub(crate) fn pre_scan(
    program: &MirProgram,
    types: &TypeInterner,
    names: &dyn lpp_codegen_api::NameResolver,
) -> Result<Plan, CodegenError> {
    let aggregates = AggregateIndex::build(program);
    let mut layouts = HashMap::new();
    for (id, _) in program.aggregates() {
        layouts.insert(id, aggregate_layout(program, types, id));
    }

    let mut export_names = BTreeMap::new();
    let mut used_export_names: BTreeSet<String> = BTreeSet::new();
    let mut imports: BTreeSet<&'static str> = BTreeSet::new();
    let mut destructors: Vec<MirAggregateId> = Vec::new();
    let mut closure_dtors: Vec<MirFunctionId> = Vec::new();
    let mut task_thunks: Vec<MirFunctionId> = Vec::new();
    let mut closure_thunks: Vec<MirFunctionId> = Vec::new();
    let mut value_classes: BTreeMap<(MirFunctionId, MirLocalId), ValueClass> = BTreeMap::new();

    for (mir_id, function) in program.functions() {
        {
            let mut scan = ScanState {
                imports: &mut imports,
                destructors: &mut destructors,
                closure_dtors: &mut closure_dtors,
                task_thunks: &mut task_thunks,
                closure_thunks: &mut closure_thunks,
                value_classes: &mut value_classes,
            };
            check_function_shape(
                program,
                types,
                function,
                mir_id,
                &aggregates,
                &layouts,
                &mut scan,
            )?;
        }

        // A source-level `main` exports as the internal `lpp_main`;
        // the C-ABI `main` wrapper is generated separately. Closure
        // functions are unnamed in the MIR and export as `lpp_c{n}`
        // (n = the MirFunctionId, in function-id order).
        match function
            .name
            .as_ref()
            .and_then(|symbol| names.resolve(symbol.raw()))
        {
            Some(name) if name == "main" => {
                export_names.insert(mir_id, "lpp_main".to_owned());
            }
            Some(name) => {
                // Distinct functions can share a source name (e.g. `describe`
                // implemented for several types via `impl` blocks). Such methods
                // are only ever called by function id, so a colliding linkage
                // name is disambiguated with the function id to keep symbols
                // unique.
                let mut export = name.to_owned();
                if used_export_names.contains(&export) {
                    export = format!("{name}_{}", mir_id.raw());
                }
                used_export_names.insert(export.clone());
                export_names.insert(mir_id, export);
            }
            None if matches!(function.kind, MirFunctionKind::Closure) => {
                export_names.insert(mir_id, format!("lpp_c{}", mir_id.raw()));
            }
            None => return Err(unsupported(mir_id, "unnamed function")),
        }
    }

    // An async entry `main` is drained by the generated `main`
    // wrapper through its task thunk (the wrapper never calls
    // `lpp_main` directly), so the thunk must exist even though no
    // MIR instruction references it.
    let main_id = export_names
        .iter()
        .find(|(_, name)| name.as_str() == "lpp_main")
        .map(|(id, _)| *id);
    if let (Some(main_id), Some(main_fn)) = (main_id, main_id.and_then(|id| program.function(id))) {
        if matches!(main_fn.kind, MirFunctionKind::Async) {
            task_thunks.push(main_id);
            imports.insert(IMP_TUPLE_ALLOC);
            imports.insert(IMP_TASK_NEW);
            imports.insert(IMP_TASK_AWAIT);
            imports.insert(IMP_TASK_DESTROY);
        }
    }

    destructors.sort_unstable();
    destructors.dedup();
    closure_dtors.sort_unstable();
    closure_dtors.dedup();
    task_thunks.sort_unstable();
    task_thunks.dedup();
    closure_thunks.sort_unstable();
    closure_thunks.dedup();

    Ok(Plan {
        export_names,
        imports,
        destructors,
        closure_dtors,
        task_thunks,
        closure_thunks,
        value_classes,
        aggregates,
        layouts,
    })
}

/// The provenance class of a function-value local. A closure value
/// and a materialized function value are the same MIR type, but their
/// capsule code pointers have different call ABIs (the closure code
/// takes the capture env first), so the class is tracked per local.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueClass {
    Closure,
    FunctionValue,
}

/// The mutable scan state threaded through `check_function_shape`.
struct ScanState<'a> {
    imports: &'a mut BTreeSet<&'static str>,
    destructors: &'a mut Vec<MirAggregateId>,
    closure_dtors: &'a mut Vec<MirFunctionId>,
    task_thunks: &'a mut Vec<MirFunctionId>,
    closure_thunks: &'a mut Vec<MirFunctionId>,
    value_classes: &'a mut BTreeMap<(MirFunctionId, MirLocalId), ValueClass>,
}

/// Record (or conflict-check) the provenance class of a
/// function-value local: a local holding both a closure and a
/// materialized function mixes the two call ABIs.
fn record_value_class(
    scan: &mut ScanState,
    mir_id: MirFunctionId,
    local: MirLocalId,
    class: ValueClass,
) -> Result<(), CodegenError> {
    if let Some(existing) = scan.value_classes.get(&(mir_id, local)) {
        if *existing != class {
            return Err(unsupported(mir_id, "mixed function value origins"));
        }
        return Ok(());
    }
    scan.value_classes.insert((mir_id, local), class);
    Ok(())
}

/// The user parameters of a function (capture parameters excluded —
/// a closure's ABI is the env pointer plus the user parameters).
fn user_parameters(program: &MirProgram, function: &MirFunction) -> Vec<MirLocalId> {
    program
        .function_parameters(function)
        .iter()
        .copied()
        .filter(|local| {
            !matches!(
                program
                    .local(*local)
                    .expect("validated MIR retains locals")
                    .kind,
                lpp_mir::MirLocalKind::Capture
            )
        })
        .collect()
}

fn check_function_shape(
    program: &MirProgram,
    types: &TypeInterner,
    function: &MirFunction,
    mir_id: MirFunctionId,
    aggregates: &AggregateIndex,
    layouts: &HashMap<MirAggregateId, AggregateLayout>,
    scan: &mut ScanState,
) -> Result<(), CodegenError> {
    // Plain functions, closure functions (lowered with the env
    // pointer as their leading parameter), and async functions
    // (called through generated task thunks) all lower.
    match function.kind {
        MirFunctionKind::Function | MirFunctionKind::Closure | MirFunctionKind::Async => {}
    }

    let mut has_managed_local = false;
    for &local in program.function_locals(function) {
        let ty = program
            .local(local)
            .expect("validated MIR retains locals")
            .ty;
        if !machine_type(types, ty).is_some() && !is_void_type(types, ty) {
            return Err(unsupported(
                mir_id,
                unsupported_construct_for(types.kind(ty)),
            ));
        }
        has_managed_local |= is_managed(types, ty);
        if matches!(types.kind(ty), TypeKind::Function { .. })
            && matches!(
                program
                    .local(local)
                    .expect("validated MIR retains locals")
                    .kind,
                lpp_mir::MirLocalKind::Parameter
            )
        {
            // Function-typed parameters are capsule values supplied by the caller.
            // The current source surface most commonly passes closures here; mark
            // the parameter as closure-compatible so indirect calls have a known
            // ABI instead of failing as "unknown origin".
            record_value_class(scan, mir_id, local, ValueClass::Closure)?;
        }
    }
    let return_managed = is_managed(types, function.return_type);
    if !machine_type(types, function.return_type).is_some()
        && !is_void_type(types, function.return_type)
    {
        return Err(unsupported(
            mir_id,
            unsupported_construct_for(types.kind(function.return_type)),
        ));
    }
    if has_managed_local {
        // Every managed slot is released by the exit pass, and every
        // transfer that sources from a local retains.
        scan.imports.insert(IMP_ARC_RELEASE);
        scan.imports.insert(IMP_ARC_RETAIN);
    }
    if return_managed {
        // The return retains for the caller (the callee's exit pass
        // releases the slot's own reference).
        scan.imports.insert(IMP_ARC_RETAIN);
    }

    let check_operand = |operand: &Operand| operand_type(program, types, operand, mir_id);
    let local_ty = |local: MirLocalId| {
        program
            .local(local)
            .expect("validated MIR retains locals")
            .ty
    };

    for &block in program.function_blocks(function) {
        let basic = program.block(block).expect("validated MIR retains blocks");
        for &instruction in program.block_instructions(basic) {
            let instr = program
                .instruction(instruction)
                .expect("validated MIR retains instructions");
            match &instr.kind {
                InstructionKind::Store { place, value } => {
                    let resolution =
                        resolve_place(program, types, *place, aggregates, layouts, mir_id, true)?;
                    check_operand(value)?;
                    if matches!(value, Operand::Copy(_)) {
                        match resolution.steps.last() {
                            // Bare local or struct field: the backend
                            // retains for the new owner.
                            None | Some(PlaceStep::Field { managed: true, .. }) => {
                                scan.imports.insert(IMP_ARC_RETAIN);
                            }
                            // The runtime's lpp_list_set_arc retains
                            // (and releases the replaced element)
                            // internally; a scalar field replacement
                            // releases nothing.
                            _ => {}
                        }
                    }
                    if let Some(PlaceStep::ListIndex { element, .. }) = resolution.steps.last() {
                        match element_class(types, *element) {
                            ElementClass::Managed => {
                                scan.imports.insert(IMP_LIST_SET_ARC);
                            }
                            ElementClass::Scalar => {
                                scan.imports.insert(IMP_LIST_SET);
                            }
                            ElementClass::Float => {
                                scan.imports.insert(IMP_LIST_SET_FLOAT);
                            }
                            ElementClass::Bool => {
                                scan.imports.insert(IMP_LIST_SET_BOOL);
                            }
                        };
                        // A list of function values inherits the
                        // provenance class of what it stores.
                        if matches!(types.kind(*element), TypeKind::Function { .. })
                            && let Operand::Copy(source) = value
                        {
                            let class = scan
                                .value_classes
                                .get(&(mir_id, *source))
                                .copied()
                                .ok_or_else(|| {
                                    unsupported(mir_id, "function value of unknown origin")
                                })?;
                            record_value_class(scan, mir_id, resolution.root, class)?;
                        }
                    }
                    record_chain_gets(types, &resolution, scan.imports);
                }
                InstructionKind::Assign { target, value } => {
                    let target_ty = local_ty(*target);
                    if !machine_type(types, target_ty).is_some() && !is_void_type(types, target_ty)
                    {
                        return Err(unsupported(
                            mir_id,
                            unsupported_construct_for(types.kind(target_ty)),
                        ));
                    }

                    match value {
                        Rvalue::Use(operand) => {
                            check_operand(operand)?;
                            if let Operand::Copy(local) = operand
                                && is_managed(types, local_ty(*local))
                            {
                                scan.imports.insert(IMP_ARC_RETAIN);
                            }
                            // Function value materialization: capsule
                            // plus, for async functions, the task
                            // thunk that adapts the runtime argument
                            // layout.
                            if let Operand::Function(function_id) = operand {
                                let callee_fn = program
                                    .function(*function_id)
                                    .expect("validated MIR retains functions");
                                scan.imports.insert(IMP_ARC_ALLOC);
                                scan.imports.insert(IMP_CLOSURE_DESTROY);
                                match callee_fn.kind {
                                    MirFunctionKind::Async => {
                                        scan.task_thunks.push(*function_id);
                                        // The value's capsule env is
                                        // an empty task-env tuple
                                        // (spawnable values must keep
                                        // a non-NULL env).
                                        scan.imports.insert(IMP_TUPLE_ALLOC);
                                    }
                                    // A sync value calls `lpp_f`
                                    // directly through the capsule
                                    // (no thunk: the capsule's code
                                    // pointer is the function itself,
                                    // the env is NULL).
                                    MirFunctionKind::Function => {}
                                    MirFunctionKind::Closure => {
                                        return Err(unsupported(
                                            mir_id,
                                            "closure as value operand",
                                        ));
                                    }
                                }
                                record_value_class(
                                    scan,
                                    mir_id,
                                    *target,
                                    ValueClass::FunctionValue,
                                )?;
                            }
                            // Copying a function-value local (or a list
                            // of function values) keeps the provenance
                            // class.
                            if let Operand::Copy(local) = operand
                                && is_function_value_kind(types, types.kind(local_ty(*local)))
                            {
                                let class = scan.value_classes.get(&(mir_id, *local)).copied();
                                match class {
                                    Some(class) => {
                                        record_value_class(scan, mir_id, *target, class)?;
                                    }
                                    // A list copy taken before any push
                                    // (or list_new itself) has no class
                                    // yet; the later push to that local
                                    // establishes it. Direct function
                                    // values must keep a strict chain.
                                    None => {
                                        if !matches!(
                                            types.kind(local_ty(*local)),
                                            TypeKind::List(_)
                                        ) {
                                            return Err(unsupported(
                                                mir_id,
                                                "function value of unknown origin",
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                        Rvalue::Unary { operator, operand } => {
                            let ty = check_operand(operand)?;
                            match (operator, ty) {
                                (UnaryOperator::Negate, t) if t.is_int() => {}
                                (UnaryOperator::Negate, cltypes::F64) => {}
                                (UnaryOperator::Not, cltypes::I8) => {}
                                _ => {
                                    return Err(unsupported(mir_id, "invalid unary operands"));
                                }
                            }
                        }
                        Rvalue::Binary {
                            left,
                            operator,
                            right,
                        } => {
                            let left_is_string = operand_language_kind(program, types, left)
                                == Some(TypeKind::Primitive(PrimitiveType::String));
                            // String `+` concatenates contents through
                            // `lpp_str_concat`, coercing a scalar right operand
                            // to a string first. The machine-type check does not
                            // apply here (a float right operand is stringified,
                            // not added), so it is skipped on this path.
                            if *operator == BinaryOperator::Add && left_is_string {
                                scan.imports.insert(IMP_STR_CONCAT);
                                match operand_language_kind(program, types, right) {
                                    Some(TypeKind::Primitive(PrimitiveType::String)) => {}
                                    Some(TypeKind::Primitive(PrimitiveType::Int)) => {
                                        scan.imports.insert(IMP_INT_TO_STR);
                                    }
                                    Some(TypeKind::Primitive(PrimitiveType::Float)) => {
                                        scan.imports.insert(IMP_FLOAT_TO_STR);
                                    }
                                    Some(TypeKind::Primitive(PrimitiveType::Bool)) => {
                                        scan.imports.insert(IMP_BOOL_TO_STR);
                                    }
                                    _ => {
                                        return Err(unsupported(
                                            mir_id,
                                            "string concatenation with non-scalar",
                                        ));
                                    }
                                }
                            } else {
                                let left_ty = check_operand(left)?;
                                let right_ty = check_operand(right)?;
                                check_binary(*operator, left_ty, right_ty, mir_id)?;
                                if *operator == BinaryOperator::Modulo && left_ty == cltypes::F64 {
                                    scan.imports.insert(IMP_FMOD);
                                }
                                // String equality compares the contents, not the
                                // heap pointers.
                                if matches!(
                                    *operator,
                                    BinaryOperator::Equal | BinaryOperator::NotEqual
                                ) && left_is_string
                                {
                                    scan.imports.insert(IMP_STR_EQ);
                                }
                            }
                        }
                        Rvalue::Call { callee, arguments } => {
                            let user_arity = |callee_ty: TypeId| match types.kind(callee_ty) {
                                TypeKind::Function { parameters, result } => {
                                    (types.list(parameters).len(), result)
                                }
                                _ => (0, callee_ty),
                            };
                            let (user_arity, result_ty) = match callee {
                                Operand::Function(callee_id) => {
                                    let callee_fn = program
                                        .function(*callee_id)
                                        .expect("validated MIR retains functions");
                                    match callee_fn.kind {
                                        // Closures are called through
                                        // their capsule, never as a
                                        // function operand.
                                        MirFunctionKind::Closure => {
                                            return Err(unsupported(
                                                mir_id,
                                                "closure as function operand",
                                            ));
                                        }
                                        // An async call builds the
                                        // task env tuple and the task
                                        // handle through the thunk.
                                        MirFunctionKind::Async => {
                                            scan.task_thunks.push(*callee_id);
                                            scan.imports.insert(IMP_TUPLE_ALLOC);
                                            scan.imports.insert(IMP_TASK_NEW);
                                        }
                                        MirFunctionKind::Function => {}
                                    }
                                    let user_params: usize = program
                                        .function_parameters(callee_fn)
                                        .iter()
                                        .filter(|local| {
                                            !matches!(
                                                program
                                                    .local(**local)
                                                    .expect("validated MIR retains locals")
                                                    .kind,
                                                lpp_mir::MirLocalKind::Capture
                                            )
                                        })
                                        .count();
                                    (user_params, callee_fn.return_type)
                                }
                                Operand::Copy(local) => {
                                    // The provenance class decides the
                                    // capsule code ABI; a value of
                                    // unknown origin never reaches a
                                    // call site.
                                    if scan.value_classes.get(&(mir_id, *local)).is_none() {
                                        return Err(unsupported(
                                            mir_id,
                                            "function value of unknown origin",
                                        ));
                                    }
                                    let callee_ty = local_ty(*local);
                                    let (arity, result) = user_arity(callee_ty);
                                    if !matches!(types.kind(callee_ty), TypeKind::Function { .. }) {
                                        return Err(unsupported(mir_id, "non-function callee"));
                                    }
                                    // A call through an async function
                                    // value builds the task env tuple
                                    // and the handle; the capsule's
                                    // code pointer is the task thunk.
                                    if matches!(types.kind(result), TypeKind::Task(_)) {
                                        scan.imports.insert(IMP_TUPLE_ALLOC);
                                        scan.imports.insert(IMP_TASK_NEW);
                                    }
                                    (arity, result)
                                }
                                _ => return Err(unsupported(mir_id, "non-function callee")),
                            };
                            if !machine_type(types, result_ty).is_some()
                                && !is_void_type(types, result_ty)
                            {
                                return Err(unsupported(
                                    mir_id,
                                    unsupported_construct_for(types.kind(result_ty)),
                                ));
                            }
                            let args = program.operands(*arguments);
                            if args.len() != user_arity {
                                return Err(unsupported(mir_id, "call arity mismatch"));
                            }
                            for operand in args {
                                check_operand(operand)?;
                                // A managed argument retains for the
                                // callee (released by the callee's
                                // exit pass) or, for an async call,
                                // for the task env tuple.
                                if let Operand::Copy(local) = *operand
                                    && is_managed(types, local_ty(local))
                                {
                                    scan.imports.insert(IMP_ARC_RETAIN);
                                }
                            }
                        }
                        Rvalue::Load(place) => {
                            let resolution = resolve_place(
                                program, types, *place, aggregates, layouts, mir_id, false,
                            )?;
                            check_value_type(types, resolution.ty, mir_id)?;
                            if is_managed(types, resolution.ty) {
                                // A field/element read gains a
                                // reference.
                                scan.imports.insert(IMP_ARC_RETAIN);
                            }
                            // A list read of a function value
                            // propagates the list's provenance class.
                            if let Some(PlaceStep::ListIndex { element, .. }) =
                                resolution.steps.last()
                                && matches!(types.kind(*element), TypeKind::Function { .. })
                            {
                                let class = scan
                                    .value_classes
                                    .get(&(mir_id, resolution.root))
                                    .copied()
                                    .ok_or_else(|| {
                                        unsupported(mir_id, "function value of unknown origin")
                                    })?;
                                record_value_class(scan, mir_id, *target, class)?;
                            }
                            record_chain_gets(types, &resolution, scan.imports);
                        }
                        Rvalue::ListLen(operand) => {
                            let ty = check_operand(operand)?;
                            if ty != cltypes::I64 {
                                return Err(unsupported(mir_id, "list length of non-list"));
                            }
                            let operand_ty = match operand {
                                Operand::Copy(local) => local_ty(*local),
                                _ => return Err(unsupported(mir_id, "list length of non-list")),
                            };
                            if !matches!(types.kind(operand_ty), TypeKind::List(_)) {
                                return Err(unsupported(mir_id, "list length of non-list"));
                            }
                            scan.imports.insert(IMP_LIST_LEN);
                        }
                        Rvalue::List(operands) => {
                            let TypeKind::List(element) = types.kind(target_ty) else {
                                return Err(unsupported(mir_id, "list literal of non-list"));
                            };
                            check_list_element(types, element, mir_id)?;
                            let arc = is_managed(types, element);
                            scan.imports
                                .insert(if arc { IMP_LIST_NEW_ARC } else { IMP_LIST_NEW });
                            for operand in program.operands(*operands) {
                                check_operand(operand)?;
                                match element_class(types, element) {
                                    ElementClass::Managed => {
                                        scan.imports.insert(IMP_LIST_PUSH_ARC);
                                    }
                                    ElementClass::Scalar => {
                                        scan.imports.insert(IMP_LIST_PUSH);
                                    }
                                    ElementClass::Float => {
                                        scan.imports.insert(IMP_LIST_PUSH_FLOAT);
                                    }
                                    ElementClass::Bool => {
                                        scan.imports.insert(IMP_LIST_PUSH_BOOL);
                                    }
                                };
                                // A list literal of function values
                                // inherits the element's provenance
                                // class.
                                if matches!(types.kind(element), TypeKind::Function { .. })
                                    && let Operand::Copy(source) = *operand
                                {
                                    let class = scan
                                        .value_classes
                                        .get(&(mir_id, source))
                                        .copied()
                                        .ok_or_else(|| {
                                            unsupported(mir_id, "function value of unknown origin")
                                        })?;
                                    record_value_class(scan, mir_id, *target, class)?;
                                }
                            }
                        }
                        Rvalue::ConstructStruct { aggregate, fields } => {
                            let agg = program
                                .aggregate(*aggregate)
                                .expect("validated MIR retains every aggregate");
                            if agg.kind != lpp_mir::MirAggregateKind::Struct {
                                return Err(unsupported(mir_id, "struct construction of enum"));
                            }
                            for operand in program.operands(*fields) {
                                check_operand(operand)?;
                                if let Operand::Copy(local) = *operand
                                    && is_managed(types, local_ty(local))
                                {
                                    scan.imports.insert(IMP_ARC_RETAIN);
                                }
                            }
                            scan.imports.insert(IMP_ARC_ALLOC);
                            scan.destructors.push(*aggregate);
                        }
                        Rvalue::ConstructVariant {
                            aggregate,
                            variant,
                            fields,
                        } => {
                            let agg = program
                                .aggregate(*aggregate)
                                .expect("validated MIR retains every aggregate");
                            if agg.kind != lpp_mir::MirAggregateKind::Enum {
                                return Err(unsupported(mir_id, "variant construction of struct"));
                            }
                            let variant_descriptor = program
                                .variant(*variant)
                                .expect("validated MIR retains every variant");
                            if variant_descriptor.aggregate != *aggregate {
                                return Err(unsupported(mir_id, "variant aggregate mismatch"));
                            }
                            for operand in program.operands(*fields) {
                                check_operand(operand)?;
                                if let Operand::Copy(local) = *operand
                                    && is_managed(types, local_ty(local))
                                {
                                    scan.imports.insert(IMP_ARC_RETAIN);
                                }
                            }
                            scan.imports.insert(IMP_ARC_ALLOC);
                            scan.destructors.push(*aggregate);
                        }
                        Rvalue::Builtin { builtin, arguments } => {
                            match *builtin {
                                PRINT_STR => {
                                    scan.imports.insert(IMP_PRINT_STR);
                                    let args = program.operands(*arguments);
                                    let descriptor = PRINT_STR.descriptor();
                                    if args.len() != descriptor.parameters.len() {
                                        return Err(CodegenError::new(
                                            Some(mir_id),
                                            CodegenErrorKind::AbiMismatch {
                                                symbol: descriptor.symbol.to_owned(),
                                                expected_arity: descriptor.parameters.len() as u32,
                                                actual_arity: args.len() as u32,
                                            },
                                        ));
                                    }
                                    for operand in args {
                                        check_operand(operand)?;
                                    }
                                }
                                LIST_NEW => {
                                    let TypeKind::List(element) = types.kind(target_ty) else {
                                        return Err(unsupported(
                                            mir_id,
                                            "list allocation of non-list",
                                        ));
                                    };
                                    check_value_type(types, element, mir_id)?;
                                    if is_managed(types, element) {
                                        scan.imports.insert(IMP_LIST_NEW_ARC);
                                    } else {
                                        scan.imports.insert(IMP_LIST_NEW);
                                    }
                                    let args = program.operands(*arguments);
                                    let descriptor = LIST_NEW.descriptor();
                                    if args.len() != descriptor.parameters.len() {
                                        return Err(CodegenError::new(
                                            Some(mir_id),
                                            CodegenErrorKind::AbiMismatch {
                                                symbol: descriptor.symbol.to_owned(),
                                                expected_arity: descriptor.parameters.len() as u32,
                                                actual_arity: args.len() as u32,
                                            },
                                        ));
                                    }
                                    for operand in args {
                                        check_operand(operand)?;
                                    }
                                }
                                other => {
                                    'family: {
                                        let descriptor = other.descriptor();
                                        let name = descriptor.name;
                                        let args = program.operands(*arguments);
                                        // Family A: print dispatches on
                                        // the argument's semantic type;
                                        // the rest import the
                                        // descriptor's v1 symbol.
                                        if name == "print" {
                                            let printer = match args.first() {
                                                Some(Operand::Constant(constant)) => match constant
                                                {
                                                    Constant::Integer(_)
                                                    | Constant::Character { .. } => IMP_PRINT_INT,
                                                    Constant::FloatBits(_) => IMP_PRINT_FLOAT,
                                                    Constant::Bool(_) => IMP_PRINT_BOOL,
                                                    Constant::String { .. } => IMP_PRINT_STR,
                                                },
                                                Some(Operand::Copy(local)) => {
                                                    match types.kind(local_ty(*local)) {
                                                        TypeKind::Primitive(
                                                            PrimitiveType::Int
                                                            | PrimitiveType::Char,
                                                        ) => IMP_PRINT_INT,
                                                        TypeKind::Primitive(
                                                            PrimitiveType::Float,
                                                        ) => IMP_PRINT_FLOAT,
                                                        TypeKind::Primitive(
                                                            PrimitiveType::Bool,
                                                        ) => IMP_PRINT_BOOL,
                                                        TypeKind::Primitive(
                                                            PrimitiveType::String,
                                                        ) => IMP_PRINT_STR,
                                                        _ => {
                                                            return Err(unsupported(
                                                                mir_id,
                                                                "print of unsupported argument",
                                                            ));
                                                        }
                                                    }
                                                }
                                                _ => {
                                                    return Err(unsupported(
                                                        mir_id,
                                                        "print of non-local argument",
                                                    ));
                                                }
                                            };
                                            scan.imports.insert(printer);
                                            if args.len() != 1 {
                                                return Err(unsupported(mir_id, "print arity"));
                                            }
                                            check_operand(args.first().expect("print argument"))?;
                                            break 'family;
                                        }
                                        // The list builtins dispatch on
                                        // the element class (the
                                        // descriptor's symbol is the
                                        // scalar spelling).
                                        if name == "list_push"
                                            || name == "list_set"
                                            || name == "list_get"
                                        {
                                            let Operand::Copy(list_local) = args[0] else {
                                                return Err(unsupported(
                                                    mir_id,
                                                    "list builtin of non-local list",
                                                ));
                                            };
                                            let TypeKind::List(element) =
                                                types.kind(local_ty(list_local))
                                            else {
                                                return Err(unsupported(
                                                    mir_id,
                                                    "list builtin of non-list",
                                                ));
                                            };
                                            if args.len() != descriptor.semantic_parameters.len() {
                                                return Err(CodegenError::new(
                                                    Some(mir_id),
                                                    CodegenErrorKind::AbiMismatch {
                                                        symbol: descriptor.symbol.to_owned(),
                                                        expected_arity: descriptor
                                                            .semantic_parameters
                                                            .len()
                                                            as u32,
                                                        actual_arity: args.len() as u32,
                                                    },
                                                ));
                                            }
                                            for operand in args {
                                                check_operand(operand)?;
                                            }
                                            match element_class(types, element) {
                                                ElementClass::Managed => {
                                                    scan.imports.insert(match name {
                                                        "list_push" => IMP_LIST_PUSH_ARC,
                                                        "list_set" => IMP_LIST_SET_ARC,
                                                        _ => IMP_LIST_GET_ARC,
                                                    });
                                                    if name == "list_get" {
                                                        // An element read
                                                        // gains a reference.
                                                        scan.imports.insert(IMP_ARC_RETAIN);
                                                    }
                                                    // Function values in
                                                    // lists carry their
                                                    // provenance class: a
                                                    // push records it on
                                                    // the list, a read
                                                    // propagates it to the
                                                    // result local.
                                                    if matches!(
                                                        types.kind(element),
                                                        TypeKind::Function { .. }
                                                    ) {
                                                        match name {
                                                            "list_get" => {
                                                                let class = scan
                                                                    .value_classes
                                                                    .get(&(mir_id, list_local))
                                                                    .copied()
                                                                    .ok_or_else(|| {
                                                                        unsupported(
                                                                            mir_id,
                                                                            "function value of unknown origin",
                                                                        )
                                                                    })?;
                                                                record_value_class(
                                                                    scan, mir_id, *target, class,
                                                                )?;
                                                            }
                                                            _ => {
                                                                if let Operand::Copy(source) =
                                                                    args[1]
                                                                {
                                                                    let class = scan
                                                                        .value_classes
                                                                        .get(&(mir_id, source))
                                                                        .copied()
                                                                        .ok_or_else(|| {
                                                                            unsupported(
                                                                                mir_id,
                                                                                "function value of unknown origin",
                                                                            )
                                                                        })?;
                                                                    record_value_class(
                                                                        scan, mir_id, list_local,
                                                                        class,
                                                                    )?;
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                                ElementClass::Scalar => {
                                                    scan.imports.insert(match name {
                                                        "list_push" => IMP_LIST_PUSH,
                                                        "list_set" => IMP_LIST_SET,
                                                        _ => IMP_LIST_GET,
                                                    });
                                                }
                                                ElementClass::Float => {
                                                    scan.imports.insert(match name {
                                                        "list_push" => IMP_LIST_PUSH_FLOAT,
                                                        "list_set" => IMP_LIST_SET_FLOAT,
                                                        _ => IMP_LIST_GET_FLOAT,
                                                    });
                                                }
                                                ElementClass::Bool => {
                                                    scan.imports.insert(match name {
                                                        "list_push" => IMP_LIST_PUSH_BOOL,
                                                        "list_set" => IMP_LIST_SET_BOOL,
                                                        _ => IMP_LIST_GET_BOOL,
                                                    });
                                                }
                                            }
                                            break 'family;
                                        }
                                        if FAMILY_A.contains(&name) {
                                            if descriptor.symbol.is_empty() {
                                                return Err(CodegenError::new(
                                                    Some(mir_id),
                                                    CodegenErrorKind::UnrepresentableBuiltin {
                                                        builtin: other,
                                                        reason: "no v1 symbol",
                                                    },
                                                ));
                                            }
                                            scan.imports.insert(descriptor.symbol);
                                            if args.len() != descriptor.semantic_parameters.len() {
                                                return Err(CodegenError::new(
                                                    Some(mir_id),
                                                    CodegenErrorKind::AbiMismatch {
                                                        symbol: descriptor.symbol.to_owned(),
                                                        expected_arity: descriptor
                                                            .semantic_parameters
                                                            .len()
                                                            as u32,
                                                        actual_arity: args.len() as u32,
                                                    },
                                                ));
                                            }
                                            for operand in args {
                                                check_operand(operand)?;
                                            }
                                            break 'family;
                                        }
                                        // Family B: SIMD — every entry
                                        // lowers to native 128-bit
                                        // instructions except the
                                        // scalar checksum, which imports
                                        // the deterministic v1 loop.
                                        if name.starts_with("vec_") {
                                            if name == "vec_i64_checksum" {
                                                scan.imports.insert(IMP_VEC_CHECKSUM);
                                            }
                                            if args.len() != descriptor.semantic_parameters.len() {
                                                return Err(CodegenError::new(
                                                    Some(mir_id),
                                                    CodegenErrorKind::AbiMismatch {
                                                        symbol: descriptor.symbol.to_owned(),
                                                        expected_arity: descriptor
                                                            .semantic_parameters
                                                            .len()
                                                            as u32,
                                                        actual_arity: args.len() as u32,
                                                    },
                                                ));
                                            }
                                            for operand in args {
                                                check_operand(operand)?;
                                            }
                                            break 'family;
                                        }
                                        // Family C: slice handles.
                                        if FAMILY_C.contains(&name) {
                                            if descriptor.symbol.is_empty() {
                                                return Err(CodegenError::new(
                                                    Some(mir_id),
                                                    CodegenErrorKind::UnrepresentableBuiltin {
                                                        builtin: other,
                                                        reason: "no v1 symbol",
                                                    },
                                                ));
                                            }
                                            scan.imports.insert(descriptor.symbol);
                                            // `slice_get` on a `StrSlice` dispatches to
                                            // `lpp_str_slice_get` at lowering time (the v1 ABI
                                            // has no dedicated builtin), so declare that import
                                            // too when the receiver is a string slice.
                                            if name == "slice_get"
                                                && matches!(
                                                    args.first().and_then(|operand| {
                                                        operand_language_kind(
                                                            program, types, operand,
                                                        )
                                                    }),
                                                    Some(TypeKind::Primitive(
                                                        PrimitiveType::StrSlice
                                                    ))
                                                )
                                            {
                                                scan.imports.insert("lpp_str_slice_get");
                                            }
                                            if args.len() != descriptor.semantic_parameters.len() {
                                                return Err(CodegenError::new(
                                                    Some(mir_id),
                                                    CodegenErrorKind::AbiMismatch {
                                                        symbol: descriptor.symbol.to_owned(),
                                                        expected_arity: descriptor
                                                            .semantic_parameters
                                                            .len()
                                                            as u32,
                                                        actual_arity: args.len() as u32,
                                                    },
                                                ));
                                            }
                                            for operand in args {
                                                check_operand(operand)?;
                                            }
                                            break 'family;
                                        }
                                        // Family MAP: hash map runtime symbols.
                                        if is_family_map(name) {
                                            if descriptor.symbol.is_empty() {
                                                return Err(CodegenError::new(
                                                    Some(mir_id),
                                                    CodegenErrorKind::UnrepresentableBuiltin {
                                                        builtin: other,
                                                        reason: "no v1 symbol",
                                                    },
                                                ));
                                            }
                                            scan.imports.insert(descriptor.symbol);
                                            if args.len() != descriptor.semantic_parameters.len() {
                                                return Err(CodegenError::new(
                                                    Some(mir_id),
                                                    CodegenErrorKind::AbiMismatch {
                                                        symbol: descriptor.symbol.to_owned(),
                                                        expected_arity: descriptor
                                                            .semantic_parameters
                                                            .len()
                                                            as u32,
                                                        actual_arity: args.len() as u32,
                                                    },
                                                ));
                                            }
                                            for operand in args {
                                                check_operand(operand)?;
                                            }
                                            break 'family;
                                        }
                                        // Family NET: host-backed sockets/HTTP.
                                        if is_family_net(name) {
                                            if descriptor.symbol.is_empty() {
                                                return Err(CodegenError::new(
                                                    Some(mir_id),
                                                    CodegenErrorKind::UnrepresentableBuiltin {
                                                        builtin: other,
                                                        reason: "no v1 symbol",
                                                    },
                                                ));
                                            }
                                            scan.imports.insert(descriptor.symbol);
                                            if args.len() != descriptor.semantic_parameters.len() {
                                                return Err(CodegenError::new(
                                                    Some(mir_id),
                                                    CodegenErrorKind::AbiMismatch {
                                                        symbol: descriptor.symbol.to_owned(),
                                                        expected_arity: descriptor
                                                            .semantic_parameters
                                                            .len()
                                                            as u32,
                                                        actual_arity: args.len() as u32,
                                                    },
                                                ));
                                            }
                                            for operand in args {
                                                check_operand(operand)?;
                                            }
                                            break 'family;
                                        }
                                        // Family D: typed rejection at
                                        // the exact function.
                                        return Err(CodegenError::new(
                                            Some(mir_id),
                                            CodegenErrorKind::UnrepresentableBuiltin {
                                                builtin: other,
                                                reason: "not in the 5C2 table policy",
                                            },
                                        ));
                                    }
                                }
                            }
                        }
                        Rvalue::Tuple(elements) => {
                            for operand in program.operands(*elements) {
                                check_operand(operand)?;
                                if let Operand::Copy(local) = *operand
                                    && is_managed(types, local_ty(local))
                                {
                                    scan.imports.insert(IMP_ARC_RETAIN);
                                }
                            }
                            scan.imports.insert(IMP_ARC_ALLOC);
                        }
                        Rvalue::MakeClosure {
                            function: closure_id,
                            captures,
                        } => {
                            let closure_fn = program
                                .function(*closure_id)
                                .expect("validated MIR retains functions");
                            if !matches!(closure_fn.kind, MirFunctionKind::Closure) {
                                return Err(unsupported(mir_id, "closure over non-closure"));
                            }
                            // The builder passes the captures in the
                            // closure's capture-parameter order.
                            let capture_params = program
                                .function_parameters(closure_fn)
                                .iter()
                                .filter(|local| {
                                    matches!(
                                        program
                                            .local(**local)
                                            .expect("validated MIR retains locals")
                                            .kind,
                                        lpp_mir::MirLocalKind::Capture
                                    )
                                })
                                .count();
                            let capture_args: &[Operand] = program.operands(*captures);
                            if capture_params != capture_args.len() {
                                return Err(unsupported(mir_id, "capture count mismatch"));
                            }
                            scan.imports.insert(IMP_ARC_ALLOC);
                            scan.imports.insert(IMP_TUPLE_ALLOC);
                            scan.imports.insert(IMP_CLOSURE_DESTROY);
                            // A zero-user-parameter closure is
                            // spawnable: its capsule code pointer is
                            // the task-code trampoline.
                            if user_parameters(program, closure_fn).is_empty() {
                                scan.closure_thunks.push(*closure_id);
                            }
                            if !capture_args.is_empty() {
                                scan.closure_dtors.push(*closure_id);
                            }
                            record_value_class(scan, mir_id, *target, ValueClass::Closure)?;
                            for operand in capture_args {
                                check_operand(operand)?;
                                if let Operand::Copy(local) = *operand
                                    && is_managed(types, local_ty(local))
                                {
                                    // The env slot retains the capture
                                    // for the capsule's lifetime.
                                    scan.imports.insert(IMP_ARC_RETAIN);
                                }
                            }
                        }
                        Rvalue::Await(operand) => {
                            let local = match operand {
                                Operand::Copy(local) => *local,
                                _ => return Err(unsupported(mir_id, "await of non-local task")),
                            };
                            let TypeKind::Task(inner) = types.kind(local_ty(local)) else {
                                return Err(unsupported(mir_id, "await of non-task"));
                            };
                            check_value_type(types, inner, mir_id)?;
                            scan.imports.insert(IMP_TASK_AWAIT);
                        }
                        Rvalue::Spawn(operand) => {
                            let local = match operand {
                                Operand::Copy(local) => *local,
                                _ => return Err(unsupported(mir_id, "spawn of non-local value")),
                            };
                            if scan.value_classes.get(&(mir_id, local)).is_none() {
                                return Err(unsupported(
                                    mir_id,
                                    "function value of unknown origin",
                                ));
                            }
                            let TypeKind::Function { parameters, result } =
                                types.kind(local_ty(local))
                            else {
                                return Err(unsupported(mir_id, "spawn of non-function"));
                            };
                            // Spawned values run with no user
                            // arguments (the oracle's arity rule):
                            // the capsule's code pointer is the
                            // task-code trampoline, and the capsule's
                            // env is already task-env shaped.
                            if !types.list(parameters).is_empty() {
                                return Err(unsupported(mir_id, "spawn of parameterized value"));
                            }
                            if !machine_type(types, result).is_some()
                                && !is_void_type(types, result)
                            {
                                return Err(unsupported(
                                    mir_id,
                                    unsupported_construct_for(types.kind(result)),
                                ));
                            }
                            scan.imports.insert(IMP_TASK_NEW);
                            scan.imports.insert(IMP_TASK_POLL);
                            scan.imports.insert(IMP_TASK_DESTROY);
                            scan.imports.insert(IMP_ARC_RETAIN);
                        }
                    }
                }
            }

            match &basic.terminator {
                Terminator::Goto(_) => {}
                Terminator::Branch { condition, .. } => {
                    let ty = check_operand(condition)?;
                    // A `Bool` (i8) or an integer handle (i64) is a valid
                    // condition: the branch lowering compares it `!= 0`, so an
                    // `Int`-typed predicate (`file_exists(p)`, `map_has(m, k)`)
                    // reads as truthy without an explicit `== 1`.
                    if ty != cltypes::I8 && ty != cltypes::I64 {
                        return Err(unsupported(mir_id, "non-bool branch condition"));
                    }
                }
                Terminator::SwitchEnum {
                    subject,
                    aggregate,
                    targets,
                } => {
                    let ty = check_operand(subject)?;
                    if ty != cltypes::I64 {
                        return Err(unsupported(mir_id, "switch over non-nominal"));
                    }
                    let subject_ty = match subject {
                        Operand::Copy(local) => local_ty(*local),
                        _ => return Err(unsupported(mir_id, "switch over non-local")),
                    };
                    let subject_aggregate = aggregates
                        .aggregate_for(subject_ty)
                        .ok_or_else(|| unsupported(mir_id, "switch over non-enum"))?;
                    let agg = program
                        .aggregate(subject_aggregate)
                        .expect("validated MIR retains every aggregate");
                    if agg.kind != lpp_mir::MirAggregateKind::Enum {
                        return Err(unsupported(mir_id, "switch over struct"));
                    }
                    if subject_aggregate != *aggregate {
                        return Err(unsupported(mir_id, "switch aggregate mismatch"));
                    }
                    let variant_count = program.aggregate_variants(agg).len();
                    if program.switch_targets(*targets).len() != variant_count {
                        return Err(unsupported(mir_id, "switch enum target count"));
                    }
                }
                Terminator::Return(operand) => {
                    if let Some(operand) = operand {
                        let ty = check_operand(operand)?;
                        if machine_type(types, function.return_type) != Some(ty) {
                            return Err(unsupported(mir_id, "return type mismatch"));
                        }
                    }
                }
                Terminator::Unreachable => {}
            }
        }
    }

    Ok(())
}

fn check_binary(
    operator: BinaryOperator,
    left: CLType,
    right: CLType,
    function: MirFunctionId,
) -> Result<(), CodegenError> {
    let invalid = || Err(unsupported(function, "invalid binary operands"));
    let integer_operands = left.is_int() && right.is_int();
    if left != right && !integer_operands {
        return invalid();
    }
    let comparison = matches!(
        operator,
        BinaryOperator::Equal
            | BinaryOperator::NotEqual
            | BinaryOperator::Less
            | BinaryOperator::Greater
            | BinaryOperator::LessEqual
            | BinaryOperator::GreaterEqual
    );
    let arithmetic = matches!(
        operator,
        BinaryOperator::Add
            | BinaryOperator::Subtract
            | BinaryOperator::Multiply
            | BinaryOperator::Divide
            | BinaryOperator::Modulo
    );
    let bitwise = matches!(
        operator,
        BinaryOperator::BitAnd
            | BinaryOperator::BitOr
            | BinaryOperator::BitXor
            | BinaryOperator::ShiftLeft
            | BinaryOperator::ShiftRight
    );
    if integer_operands {
        if arithmetic || bitwise || comparison {
            return Ok(());
        }
        if left == cltypes::I8
            && right == cltypes::I8
            && matches!(
                operator,
                BinaryOperator::LogicalAnd | BinaryOperator::LogicalOr
            )
        {
            return Ok(());
        }
        return invalid();
    }
    match left {
        cltypes::F64 if arithmetic || comparison => Ok(()),
        _ => invalid(),
    }
}

/// The shared per-function data an instruction pass reads: bundled so
/// method signatures stay short. The (invariant-lifetime) builder is
/// always passed separately.
struct Inst<'a> {
    program: &'a MirProgram,
    types: &'a TypeInterner,
    local_vars: &'a HashMap<MirLocalId, Variable>,
    env_var: Option<Variable>,
    capture_slots: &'a HashMap<MirLocalId, u32>,
    mir_id: MirFunctionId,
    function: &'a MirFunction,
}

/// The module-side state of one compilation.
pub(crate) struct Lowering<'m> {
    module: &'m mut ObjectModule,
    aggregates: AggregateIndex,
    layouts: HashMap<MirAggregateId, AggregateLayout>,
    func_ids: BTreeMap<MirFunctionId, FuncId>,
    dtor_ids: BTreeMap<MirAggregateId, FuncId>,
    closure_dtor_ids: BTreeMap<MirFunctionId, FuncId>,
    task_thunk_ids: BTreeMap<MirFunctionId, FuncId>,
    closure_thunk_ids: BTreeMap<MirFunctionId, FuncId>,
    value_classes: BTreeMap<(MirFunctionId, MirLocalId), ValueClass>,
    import_ids: BTreeMap<&'static str, FuncId>,
    exported: BTreeSet<String>,
    imported: BTreeSet<String>,
    entry: Option<String>,
    lpp_main_id: Option<FuncId>,
    lpp_main_function_id: Option<MirFunctionId>,
    emitted_strings: BTreeMap<MirStringId, DataId>,
}

impl<'m> Lowering<'m> {
    pub(crate) fn new(
        module: &'m mut ObjectModule,
        aggregates: AggregateIndex,
        layouts: HashMap<MirAggregateId, AggregateLayout>,
        value_classes: BTreeMap<(MirFunctionId, MirLocalId), ValueClass>,
    ) -> Self {
        Self {
            module,
            aggregates,
            layouts,
            func_ids: BTreeMap::new(),
            dtor_ids: BTreeMap::new(),
            closure_dtor_ids: BTreeMap::new(),
            task_thunk_ids: BTreeMap::new(),
            closure_thunk_ids: BTreeMap::new(),
            value_classes,
            import_ids: BTreeMap::new(),
            exported: BTreeSet::new(),
            imported: BTreeSet::new(),
            entry: None,
            lpp_main_id: None,
            lpp_main_function_id: None,
            emitted_strings: BTreeMap::new(),
        }
    }

    pub(crate) fn exported_symbols(&self) -> BTreeSet<String> {
        self.exported.clone()
    }

    pub(crate) fn imported_symbols(&self) -> BTreeSet<String> {
        self.imported.clone()
    }

    pub(crate) fn entry(&self) -> Option<String> {
        self.entry.clone()
    }

    fn external_name(&self, logical: &str) -> String {
        if self.module.isa().triple().binary_format == target_lexicon::BinaryFormat::Macho {
            format!("_{logical}")
        } else {
            logical.to_string()
        }
    }

    /// Declare the module's symbols: runtime imports first (only the
    /// ones the pre-scan found used, in stable order), then generated
    /// destructors in `MirAggregateId` order, then user exports in
    /// `MirFunctionId` order.
    pub(crate) fn declare(
        &mut self,
        plan: &Plan,
        program: &MirProgram,
        types: &TypeInterner,
    ) -> Result<(), CodegenError> {
        for name in &plan.imports {
            let (params, result) = import_signature(name);
            let mut sig = self.module.make_signature();
            for param in params {
                sig.params.push(AbiParam::new(*param));
            }
            if let Some(result) = result {
                sig.returns.push(AbiParam::new(result));
            }
            let external_name = self.external_name(name);
            let id = self
                .module
                .declare_function(&external_name, Linkage::Import, &sig)
                .map_err(|e| emission_failed(format!("declare import {name}: {e:?}")))?;
            self.import_ids.insert(name, id);
            self.imported.insert(name.to_string());
        }

        for &aggregate in &plan.destructors {
            let agg = program
                .aggregate(aggregate)
                .expect("validated MIR retains every aggregate");
            let tag = match agg.kind {
                lpp_mir::MirAggregateKind::Struct => 's',
                lpp_mir::MirAggregateKind::Enum => 'e',
            };
            let name = format!("lpp_drop_{tag}{}", aggregate.raw());
            let mut sig = self.module.make_signature();
            sig.params.push(AbiParam::new(cltypes::I64));
            let id = self
                .module
                .declare_function(&name, Linkage::Export, &sig)
                .map_err(|e| emission_failed(format!("declare destructor {name}: {e:?}")))?;
            self.dtor_ids.insert(aggregate, id);
            self.exported.insert(name);
        }

        for &function_id in &plan.closure_dtors {
            let name = format!("lpp_drop_c{}", function_id.raw());
            let mut sig = self.module.make_signature();
            sig.params.push(AbiParam::new(cltypes::I64));
            let id = self
                .module
                .declare_function(&name, Linkage::Export, &sig)
                .map_err(|e| emission_failed(format!("declare closure dtor {name}: {e:?}")))?;
            self.closure_dtor_ids.insert(function_id, id);
            self.exported.insert(name);
        }

        for &function_id in &plan.task_thunks {
            let name = format!("__lpp_task_thunk{}", function_id.raw());
            let mut sig = self.module.make_signature();
            sig.params.push(AbiParam::new(cltypes::I64));
            sig.returns.push(AbiParam::new(cltypes::I64));
            let id = self
                .module
                .declare_function(&name, Linkage::Export, &sig)
                .map_err(|e| emission_failed(format!("declare task thunk {name}: {e:?}")))?;
            self.task_thunk_ids.insert(function_id, id);
            self.exported.insert(name);
        }

        for &function_id in &plan.closure_thunks {
            let name = format!("__lpp_closure_thunk{}", function_id.raw());
            let mut sig = self.module.make_signature();
            sig.params.push(AbiParam::new(cltypes::I64));
            sig.returns.push(AbiParam::new(cltypes::I64));
            let id = self
                .module
                .declare_function(&name, Linkage::Export, &sig)
                .map_err(|e| emission_failed(format!("declare closure thunk {name}: {e:?}")))?;
            self.closure_thunk_ids.insert(function_id, id);
            self.exported.insert(name);
        }

        for (mir_id, function) in program.functions() {
            let name = plan
                .export_names
                .get(&mir_id)
                .expect("pre_scan named every function");
            let is_closure = matches!(function.kind, MirFunctionKind::Closure);
            let mut sig = self.module.make_signature();
            // A closure's ABI is the env pointer plus the user
            // parameters (the capture parameters are env slots).
            if is_closure {
                sig.params.push(AbiParam::new(cltypes::I64));
            }
            for &param in program.function_parameters(function) {
                if is_closure
                    && matches!(
                        program
                            .local(param)
                            .expect("validated MIR retains locals")
                            .kind,
                        lpp_mir::MirLocalKind::Capture
                    )
                {
                    continue;
                }
                let ty = program
                    .local(param)
                    .expect("validated MIR retains locals")
                    .ty;
                sig.params
                    .push(AbiParam::new(check_value_type(types, ty, mir_id)?));
            }
            if let Some(cl) = machine_type(types, function.return_type) {
                sig.returns.push(AbiParam::new(cl));
            }
            let id = self
                .module
                .declare_function(name, Linkage::Export, &sig)
                .map_err(|e| emission_failed(format!("declare export {name}: {e:?}")))?;
            if name == "lpp_main" {
                self.lpp_main_id = Some(id);
                self.lpp_main_function_id = Some(mir_id);
            }
            self.func_ids.insert(mir_id, id);
            self.exported.insert(name.clone());
        }

        Ok(())
    }

    /// Lower every generated destructor in `MirAggregateId` order.
    pub(crate) fn lower_destructors(
        &mut self,
        types: &TypeInterner,
        plan: &Plan,
    ) -> Result<(), CodegenError> {
        for &aggregate in &plan.destructors {
            self.lower_destructor(types, aggregate)?;
        }
        Ok(())
    }

    fn lower_destructor(
        &mut self,
        types: &TypeInterner,
        aggregate: MirAggregateId,
    ) -> Result<(), CodegenError> {
        let func_id = *self
            .dtor_ids
            .get(&aggregate)
            .expect("declare ran before lowering");
        let layout = self
            .layouts
            .get(&aggregate)
            .expect("pre-scan laid out every nominal")
            .clone();

        let mut ctx = self.module.make_context();
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(cltypes::I64));
        ctx.func.signature = sig;
        ctx.func.name = UserFuncName::user(2, aggregate.raw());
        {
            let mut fn_ctx = FunctionBuilderContext::new();
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fn_ctx);
            let entry = builder.create_block();
            let payload = builder.append_block_param(entry, cltypes::I64);
            builder.seal_block(entry);
            builder.switch_to_block(entry);

            match layout.kind {
                lpp_mir::MirAggregateKind::Struct => {
                    for slot in &layout.struct_fields {
                        if is_managed(types, slot.ty) {
                            let old = builder.ins().load(
                                cltypes::I64,
                                MemFlags::new(),
                                payload,
                                slot.offset as i32,
                            );
                            self.arc_release(&mut builder, old);
                        }
                    }
                    builder.ins().return_(&[]);
                }
                lpp_mir::MirAggregateKind::Enum => {
                    let tag = builder
                        .ins()
                        .load(cltypes::I64, MemFlags::new(), payload, 0);
                    let variant_blocks: Vec<Block> = layout
                        .variant_fields
                        .iter()
                        .map(|_| builder.create_block())
                        .collect();
                    // The cascade dispatches from the entry block while
                    // it is still current (cranelift forbids revisiting
                    // a filled block); the variant blocks are still
                    // pristine targets.
                    emit_tag_cascade(&mut builder, tag, &variant_blocks);
                    for (ordinal, slots) in layout.variant_fields.iter().enumerate() {
                        let variant_block = variant_blocks[ordinal];
                        builder.seal_block(variant_block);
                        builder.switch_to_block(variant_block);
                        for slot in slots {
                            if is_managed(types, slot.ty) {
                                let old = builder.ins().load(
                                    cltypes::I64,
                                    MemFlags::new(),
                                    payload,
                                    slot.offset as i32,
                                );
                                self.arc_release(&mut builder, old);
                            }
                        }
                        builder.ins().return_(&[]);
                    }
                }
            }
            builder.seal_all_blocks();
            builder.finalize();
        }

        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| verify_failed(None, format!("define destructor: {e:?}")))?;
        Ok(())
    }

    /// Lower every function in `MirFunctionId` order, then the
    /// generated closure-env destructors and task/closure thunks in
    /// plan order.
    pub(crate) fn lower_functions(
        &mut self,
        program: &MirProgram,
        types: &TypeInterner,
        plan: &Plan,
    ) -> Result<(), CodegenError> {
        for (mir_id, function) in program.functions() {
            self.lower_function(program, types, mir_id, function)?;
        }
        for &function_id in &plan.closure_dtors {
            self.lower_closure_dtor(program, types, function_id)?;
        }
        for &function_id in &plan.task_thunks {
            self.lower_task_thunk(program, types, function_id)?;
        }
        for &function_id in &plan.closure_thunks {
            self.lower_closure_thunk(program, types, function_id)?;
        }
        Ok(())
    }

    /// `lpp_drop_c{n}` — releases the closure env's managed capture
    /// slots (offsets `8*i`), in capture order.
    fn lower_closure_dtor(
        &mut self,
        program: &MirProgram,
        types: &TypeInterner,
        function_id: MirFunctionId,
    ) -> Result<(), CodegenError> {
        let func_id = *self
            .closure_dtor_ids
            .get(&function_id)
            .expect("declare ran before lowering");
        let function = program
            .function(function_id)
            .expect("validated MIR retains functions");
        let captures: Vec<(MirLocalId, TypeId)> = program
            .function_parameters(function)
            .iter()
            .filter(|local| {
                matches!(
                    program
                        .local(**local)
                        .expect("validated MIR retains locals")
                        .kind,
                    lpp_mir::MirLocalKind::Capture
                )
            })
            .map(|local| {
                (
                    *local,
                    program
                        .local(*local)
                        .expect("validated MIR retains locals")
                        .ty,
                )
            })
            .collect();

        let mut ctx = self.module.make_context();
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(cltypes::I64));
        ctx.func.signature = sig;
        ctx.func.name = UserFuncName::user(3, function_id.raw());
        {
            let mut fn_ctx = FunctionBuilderContext::new();
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fn_ctx);
            let entry = builder.create_block();
            let env = builder.append_block_param(entry, cltypes::I64);
            builder.seal_block(entry);
            builder.switch_to_block(entry);
            for (i, (_local, ty)) in captures.iter().enumerate() {
                if is_managed(types, *ty) {
                    let old =
                        builder
                            .ins()
                            .load(cltypes::I64, MemFlags::new(), env, (8 * i) as i32);
                    self.arc_release(&mut builder, old);
                }
            }
            builder.ins().return_(&[]);
            builder.seal_all_blocks();
            builder.finalize();
        }
        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| verify_failed(Some(function_id), format!("define closure dtor: {e:?}")))?;
        Ok(())
    }

    /// `__lpp_task_thunk{n}` — the task code for async function
    /// `n`: loads the argument slots from the task env tuple
    /// (offsets `16 + 8*i`), retains the managed ones, calls the
    /// function, and boxes the result.
    fn lower_task_thunk(
        &mut self,
        program: &MirProgram,
        types: &TypeInterner,
        function_id: MirFunctionId,
    ) -> Result<(), CodegenError> {
        let func_id = *self
            .task_thunk_ids
            .get(&function_id)
            .expect("declare ran before lowering");
        let function = program
            .function(function_id)
            .expect("validated MIR retains functions");

        let mut ctx = self.module.make_context();
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(cltypes::I64));
        sig.returns.push(AbiParam::new(cltypes::I64));
        ctx.func.signature = sig;
        ctx.func.name = UserFuncName::user(4, function_id.raw());
        {
            let mut fn_ctx = FunctionBuilderContext::new();
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fn_ctx);
            let entry = builder.create_block();
            let env = builder.append_block_param(entry, cltypes::I64);
            builder.seal_block(entry);
            builder.switch_to_block(entry);
            let mut args: Vec<Value> = Vec::new();
            for (i, &param) in user_parameters(program, function).iter().enumerate() {
                let ty = program
                    .local(param)
                    .expect("validated MIR retains locals")
                    .ty;
                let mut v =
                    builder
                        .ins()
                        .load(cltypes::I64, MemFlags::new(), env, (16 + 8 * i) as i32);
                if is_managed(types, ty) {
                    v = self.arc_retain_value(&mut builder, v);
                }
                args.push(v);
            }
            let callee_ref = self
                .module
                .declare_func_in_func(self.func_ids[&function_id], builder.func);
            let call = builder.ins().call(callee_ref, &args);
            let result = machine_type(types, function.return_type)
                .is_some()
                .then(|| builder.func.dfg.first_result(call));
            let boxed = box_result(types, &mut builder, result, function.return_type);
            builder.ins().return_(&[boxed]);
            builder.seal_all_blocks();
            builder.finalize();
        }
        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| verify_failed(Some(function_id), format!("define task thunk: {e:?}")))?;
        Ok(())
    }

    /// `__lpp_closure_thunk{n}` — the task code for the
    /// zero-user-parameter closure `n`: the task env slot holds the
    /// closure env (offset `16`); invoke the closure and box the
    /// result.
    fn lower_closure_thunk(
        &mut self,
        program: &MirProgram,
        types: &TypeInterner,
        function_id: MirFunctionId,
    ) -> Result<(), CodegenError> {
        let func_id = *self
            .closure_thunk_ids
            .get(&function_id)
            .expect("declare ran before lowering");
        let function = program
            .function(function_id)
            .expect("validated MIR retains functions");

        let mut ctx = self.module.make_context();
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(cltypes::I64));
        sig.returns.push(AbiParam::new(cltypes::I64));
        ctx.func.signature = sig;
        ctx.func.name = UserFuncName::user(5, function_id.raw());
        {
            let mut fn_ctx = FunctionBuilderContext::new();
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fn_ctx);
            let entry = builder.create_block();
            // The thunk's single parameter is the capsule env: a
            // 1-slot wrapper tuple whose slot 16 holds the raw closure
            // env (a NULL slot for zero-capture closures), so the same
            // code pointer serves the direct call and the spawned task.
            let wrapper = builder.append_block_param(entry, cltypes::I64);
            builder.seal_block(entry);
            builder.switch_to_block(entry);
            let closure_env = builder
                .ins()
                .load(cltypes::I64, MemFlags::new(), wrapper, 16);
            let callee_ref = self
                .module
                .declare_func_in_func(self.func_ids[&function_id], builder.func);
            let call = builder.ins().call(callee_ref, &[closure_env]);
            let result = machine_type(types, function.return_type)
                .is_some()
                .then(|| builder.func.dfg.first_result(call));
            let boxed = box_result(types, &mut builder, result, function.return_type);
            builder.ins().return_(&[boxed]);
            builder.seal_all_blocks();
            builder.finalize();
        }
        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| {
                verify_failed(Some(function_id), format!("define closure thunk: {e:?}"))
            })?;
        Ok(())
    }

    /// `lpp_arc_retain(value)` — the v1 ABI returns void, so the
    /// retained value is the input itself.
    fn arc_retain_value(&mut self, builder: &mut FunctionBuilder, value: Value) -> Value {
        self.arc_retain(builder, value);
        value
    }

    /// `lpp_arc_alloc_with_destructor(size, dtor)` with a raw
    /// destructor function reference (the aggregate dtors, the
    /// closure capsule's `lpp_closure_destroy`).
    fn arc_alloc_ptr(
        &mut self,
        builder: &mut FunctionBuilder,
        size: u32,
        dtor_ref: FuncRef,
    ) -> Value {
        let func_ref = self.import_ref(builder, IMP_ARC_ALLOC);
        let size_value = builder.ins().iconst(cltypes::I64, size as i64);
        let dtor_addr = builder.ins().func_addr(cltypes::I64, dtor_ref);
        let call = builder.ins().call(func_ref, &[size_value, dtor_addr]);
        builder.func.dfg.first_result(call)
    }

    /// The task result `managed` flag: `1` when the user result
    /// behind a `Task` return type is ARC-managed.
    fn task_managed_flag(types: &TypeInterner, return_type: TypeId) -> i64 {
        let inner = match types.kind(return_type) {
            TypeKind::Task(inner) => inner,
            _ => return_type,
        };
        if is_managed(types, inner) { 1 } else { 0 }
    }

    /// `lpp_task_new(code, env, managed)`.
    fn task_new(
        &mut self,
        builder: &mut FunctionBuilder,
        code: Value,
        env: Value,
        managed: i64,
    ) -> Value {
        let func_ref = self.import_ref(builder, IMP_TASK_NEW);
        let managed_value = builder.ins().iconst(cltypes::I64, managed);
        let call = builder.ins().call(func_ref, &[code, env, managed_value]);
        builder.func.dfg.first_result(call)
    }

    /// The task env tuple for a call's arguments:
    /// `lpp_tuple_alloc(16 + 8*n, mask, packed_offsets)` with each
    /// argument stored at `16 + 8*i` (managed arguments retained for
    /// the tuple, which owns them).
    fn task_env_for_args(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        args_ops: &[Operand],
    ) -> Result<Value, CodegenError> {
        let mut mask = 0i64;
        let mut offsets = 0i64;
        for (i, operand) in args_ops.iter().enumerate() {
            if let Operand::Copy(local) = operand
                && is_managed(
                    inst.types,
                    inst.program
                        .local(*local)
                        .expect("validated MIR retains locals")
                        .ty,
                )
            {
                mask |= 1i64 << i;
            }
            offsets |= ((16 + 8 * i) as i64) << (16 * i as i64);
        }
        let size = 16 + 8 * args_ops.len() as i64;
        let func_ref = self.import_ref(builder, IMP_TUPLE_ALLOC);
        let size_value = builder.ins().iconst(cltypes::I64, size);
        let mask_value = builder.ins().iconst(cltypes::I64, mask);
        let offsets_value = builder.ins().iconst(cltypes::I64, offsets);
        let call = builder
            .ins()
            .call(func_ref, &[size_value, mask_value, offsets_value]);
        let env = builder.func.dfg.first_result(call);
        for (i, operand) in args_ops.iter().enumerate() {
            let value = self.owned_value(inst, builder, operand)?;
            builder
                .ins()
                .store(MemFlags::new(), value, env, (16 + 8 * i) as i32);
        }
        Ok(env)
    }

    /// `Use(Operand::Function)` materialization: the 16-byte capsule
    /// `[code, env]`. A sync value points at `lpp_f` with a `NULL`
    /// env; an async value points at its task thunk with an empty
    /// task-env tuple (spawned values need a non-NULL env).
    fn materialize_function_value(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        function_id: MirFunctionId,
    ) -> Result<Value, CodegenError> {
        let function = inst
            .program
            .function(function_id)
            .expect("validated MIR retains functions");
        let dtor_id = *self
            .import_ids
            .get(IMP_CLOSURE_DESTROY)
            .expect("pre-scan declared lpp_closure_destroy");
        let dtor_ref = self.module.declare_func_in_func(dtor_id, builder.func);
        let capsule = self.arc_alloc_ptr(builder, 16, dtor_ref);
        let (code_ref, env) = match function.kind {
            MirFunctionKind::Async => {
                let thunk_ref = self
                    .module
                    .declare_func_in_func(self.task_thunk_ids[&function_id], builder.func);
                let code = builder.ins().func_addr(cltypes::I64, thunk_ref);
                let tuple_ref = self.import_ref(builder, IMP_TUPLE_ALLOC);
                let zero = builder.ins().iconst(cltypes::I64, 0);
                let size = builder.ins().iconst(cltypes::I64, 16);
                let call = builder.ins().call(tuple_ref, &[size, zero, zero]);
                let env = builder.func.dfg.first_result(call);
                (code, env)
            }
            MirFunctionKind::Function => {
                let fn_ref = self
                    .module
                    .declare_func_in_func(self.func_ids[&function_id], builder.func);
                let code = builder.ins().func_addr(cltypes::I64, fn_ref);
                (code, builder.ins().iconst(cltypes::I64, 0))
            }
            MirFunctionKind::Closure => {
                return Err(unsupported(inst.mir_id, "closure as value operand"));
            }
        };
        builder.ins().store(MemFlags::new(), code_ref, capsule, 0);
        builder.ins().store(MemFlags::new(), env, capsule, 8);
        Ok(capsule)
    }

    /// The 5C2 builtin table policy beyond the 5C arms: `print`
    /// dispatch, Family A (import of the descriptor's v1 symbol),
    /// Family B (native 128-bit SIMD, the scalar checksum imported),
    /// and Family C (slice handles over the v1 slice runtime).
    fn lower_builtin_families(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        builtin: BuiltinId,
        args: &[Operand],
    ) -> Result<Option<Value>, CodegenError> {
        let descriptor = builtin.descriptor();
        let name = descriptor.name;

        // Family A: `print` dispatches on the argument's semantic
        // type; everything else imports the descriptor's v1 symbol
        // (arguments are plain reads — the v1 C functions never take
        // ownership of their string/list arguments).
        if name == "print" {
            // Family A `print` dispatches on the argument's semantic type.
            // A char argument lowers to i32 but the v1 print-int symbol
            // carries the ABI's i64 int, so it is widened before the call
            // (int arguments are already i64).
            let (printer, is_char) = match args.first().expect("print argument") {
                Operand::Constant(constant) => match constant {
                    Constant::Integer(_) => (IMP_PRINT_INT, false),
                    Constant::Character { .. } => (IMP_PRINT_INT, true),
                    Constant::FloatBits(_) => (IMP_PRINT_FLOAT, false),
                    Constant::Bool(_) => (IMP_PRINT_BOOL, false),
                    Constant::String { .. } => (IMP_PRINT_STR, false),
                },
                Operand::Copy(local) => match inst.types.kind(
                    inst.program
                        .local(*local)
                        .expect("validated MIR retains locals")
                        .ty,
                ) {
                    TypeKind::Primitive(PrimitiveType::Int) => (IMP_PRINT_INT, false),
                    TypeKind::Primitive(PrimitiveType::Char) => (IMP_PRINT_INT, true),
                    TypeKind::Primitive(PrimitiveType::Float) => (IMP_PRINT_FLOAT, false),
                    TypeKind::Primitive(PrimitiveType::Bool) => (IMP_PRINT_BOOL, false),
                    TypeKind::Primitive(PrimitiveType::String) => (IMP_PRINT_STR, false),
                    _ => return Err(unsupported(inst.mir_id, "print of unsupported argument")),
                },
                _ => return Err(unsupported(inst.mir_id, "print of non-local argument")),
            };
            let mut value =
                self.operand_value(inst, builder, args.first().expect("print argument"))?;
            if is_char {
                value = builder.ins().uextend(cltypes::I64, value);
            }
            let func_ref = self.import_ref(builder, printer);
            builder.ins().call(func_ref, &[value]);
            // print consumes its value: the reference dies with it.
            self.empty_moved_local(inst, builder, args.first().expect("print argument"));
            return Ok(None);
        }
        if name == "list_push" || name == "list_set" {
            return self.lower_list_mutation(inst, builder, name, args);
        }
        if name == "list_get" {
            return self.lower_list_get(inst, builder, args);
        }
        if FAMILY_A.contains(&name) {
            let symbol = descriptor.symbol;
            let mut call_args: Vec<Value> = Vec::with_capacity(args.len());
            for operand in args {
                call_args.push(self.operand_value(inst, builder, operand)?);
            }
            let func_ref = self.import_ref(builder, symbol);
            let call = builder.ins().call(func_ref, &call_args);
            if name == "write_str" {
                // write_str consumes its value: the reference dies with it.
                self.empty_moved_local(inst, builder, args.first().expect("write_str argument"));
            }
            let has_result =
                !matches!(descriptor.result, lpp_runtime_abi::generated::AbiType::Void);
            let result = has_result.then(|| builder.func.dfg.first_result(call));
            return Ok(match result {
                // The v1 C predicates return an `i64` 0/1, but the
                // language result is a `bool` (the oracle yields a
                // `Bool`): reduce to the machine bool.
                Some(raw)
                    if matches!(
                        descriptor.semantic_result,
                        lpp_runtime_abi::generated::SemanticAbiType::Bool
                    ) =>
                {
                    Some(builder.ins().ireduce(cltypes::I8, raw))
                }
                other => other,
            });
        }
        if name.starts_with("vec_") {
            return self.lower_simd(inst, builder, builtin, name, args);
        }
        if FAMILY_C.contains(&name) {
            return self.lower_slice(inst, builder, builtin, name, args);
        }
        if is_family_map(name) {
            let symbol = descriptor.symbol;
            let mut call_args: Vec<Value> = Vec::with_capacity(args.len());
            for operand in args {
                call_args.push(self.operand_value(inst, builder, operand)?);
            }
            let func_ref = self.import_ref(builder, symbol);
            let call = builder.ins().call(func_ref, &call_args);
            let has_result =
                !matches!(descriptor.result, lpp_runtime_abi::generated::AbiType::Void);
            let result = has_result.then(|| builder.func.dfg.first_result(call));
            return Ok(match result {
                // Membership predicates: the runtime returns an i64 0/1 but the
                // language result is a machine bool (i8).
                Some(raw) if is_map_predicate(name) => {
                    Some(builder.ins().ireduce(cltypes::I8, raw))
                }
                other => other,
            });
        }
        if is_family_net(name) {
            let symbol = descriptor.symbol;
            let mut call_args: Vec<Value> = Vec::with_capacity(args.len());
            for operand in args {
                call_args.push(self.operand_value(inst, builder, operand)?);
            }
            let func_ref = self.import_ref(builder, symbol);
            let call = builder.ins().call(func_ref, &call_args);
            let has_result =
                !matches!(descriptor.result, lpp_runtime_abi::generated::AbiType::Void);
            return Ok(has_result.then(|| builder.func.dfg.first_result(call)));
        }
        Err(CodegenError::new(
            Some(inst.mir_id),
            CodegenErrorKind::UnrepresentableBuiltin {
                builtin,
                reason: "not in the 5C2 table policy",
            },
        ))
    }

    /// Family B: the SIMD entries lowered to native 128-bit
    /// instructions (the 5B deferral lifted) — the v1 cranelift
    /// backend's `lower_vector_builtin` semantics. Only the scalar
    /// checksum imports the deterministic v1 loop.
    fn vector_from_lanes(&mut self, builder: &mut FunctionBuilder, lanes: [Value; 2]) -> Value {
        let value = builder.ins().splat(cltypes::I64X2, lanes[0]);
        builder.ins().insertlane(value, lanes[1], 1u8)
    }

    fn lower_simd(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        builtin: BuiltinId,
        name: &str,
        args: &[Operand],
    ) -> Result<Option<Value>, CodegenError> {
        let value = match name {
            "vec_i64x2" => {
                let lanes = [
                    self.operand_value(inst, builder, &args[0])?,
                    self.operand_value(inst, builder, &args[1])?,
                ];
                Some(self.vector_from_lanes(builder, lanes))
            }
            "vec_i64x2_splat" => {
                let lane = self.operand_value(inst, builder, &args[0])?;
                Some(self.vector_from_lanes(builder, [lane, lane]))
            }
            "vec_i64x2_not" => {
                let val = self.operand_value(inst, builder, &args[0])?;
                Some(builder.ins().bnot(val))
            }
            "vec_i64x2_add" | "vec_i64x2_sub" | "vec_i64x2_mul" | "vec_i64x2_xor"
            | "vec_i64x2_and" | "vec_i64x2_or" => {
                let left = self.operand_value(inst, builder, &args[0])?;
                let right = self.operand_value(inst, builder, &args[1])?;
                Some(match name {
                    "vec_i64x2_add" => builder.ins().iadd(left, right),
                    "vec_i64x2_sub" => builder.ins().isub(left, right),
                    "vec_i64x2_mul" => builder.ins().imul(left, right),
                    "vec_i64x2_and" => builder.ins().band(left, right),
                    "vec_i64x2_or" => builder.ins().bor(left, right),
                    _ => builder.ins().bxor(left, right),
                })
            }
            "vec_i64x2_shr" => {
                let shift = match &args[1] {
                    Operand::Constant(Constant::Integer(value)) => *value,
                    _ => {
                        return Err(unsupported(
                            inst.mir_id,
                            "vector shift amount must be a constant integer",
                        ));
                    }
                };
                let left = self.operand_value(inst, builder, &args[0])?;
                let mut lanes = [builder.ins().iconst(cltypes::I64, 0); 2];
                for lane in 0..2u8 {
                    let item = builder.ins().extractlane(left, lane);
                    lanes[lane as usize] = builder.ins().sshr_imm(item, shift);
                }
                Some(self.vector_from_lanes(builder, lanes))
            }
            "vec_i64x2_shr_var" => {
                let left = self.operand_value(inst, builder, &args[0])?;
                let right = self.operand_value(inst, builder, &args[1])?;
                let left0 = builder.ins().extractlane(left, 0);
                let left1 = builder.ins().extractlane(left, 1);
                let right0 = builder.ins().extractlane(right, 0);
                let right1 = builder.ins().extractlane(right, 1);
                let lanes = [
                    builder.ins().sshr(left0, right0),
                    builder.ins().sshr(left1, right1),
                ];
                Some(self.vector_from_lanes(builder, lanes))
            }
            "vec_i64x2_extract" => {
                let lane = match &args[1] {
                    Operand::Constant(Constant::Integer(index)) if (0..2).contains(index) => {
                        *index as u8
                    }
                    _ => {
                        return Err(unsupported(
                            inst.mir_id,
                            "vector extract lane must be a constant 0 or 1",
                        ));
                    }
                };
                let value = self.operand_value(inst, builder, &args[0])?;
                Some(builder.ins().extractlane(value, lane))
            }
            "vec_i64x2_sum" => {
                let value = self.operand_value(inst, builder, &args[0])?;
                let mut result = builder.ins().extractlane(value, 0);
                for lane in 1..2u8 {
                    let item = builder.ins().extractlane(value, lane);
                    result = builder.ins().iadd(result, item);
                }
                Some(result)
            }
            "vec_u8x16_splat" => {
                let byte_val = self.operand_value(inst, builder, &args[0])?;
                let byte_val = builder.ins().ireduce(cltypes::I8, byte_val);
                let byte_vec = builder.ins().splat(cltypes::I8X16, byte_val);
                let flags =
                    MemFlags::new().with_endianness(cranelift_codegen::ir::Endianness::Little);
                Some(builder.ins().bitcast(cltypes::I64X2, flags, byte_vec))
            }
            "vec_u8x16_eq" => {
                let left = self.operand_value(inst, builder, &args[0])?;
                let right = self.operand_value(inst, builder, &args[1])?;
                let flags =
                    MemFlags::new().with_endianness(cranelift_codegen::ir::Endianness::Little);
                let left_u8 = builder.ins().bitcast(cltypes::I8X16, flags, left);
                let right_u8 = builder.ins().bitcast(cltypes::I8X16, flags, right);
                let cmp = builder.ins().icmp(IntCC::Equal, left_u8, right_u8);
                Some(builder.ins().bitcast(cltypes::I64X2, flags, cmp))
            }
            // Both movemask spellings share the v1 symbol and the
            // native lowering.
            "vec_u8x16_movemask" | "vec_movemask8" => {
                let val = self.operand_value(inst, builder, &args[0])?;
                let flags =
                    MemFlags::new().with_endianness(cranelift_codegen::ir::Endianness::Little);
                let val_u8 = builder.ins().bitcast(cltypes::I8X16, flags, val);
                let mut mask = builder.ins().iconst(cltypes::I64, 0);
                for lane in 0..16u8 {
                    let b = builder.ins().extractlane(val_u8, lane);
                    let bit = builder.ins().ushr_imm(b, 7);
                    let bit64 = builder.ins().uextend(cltypes::I64, bit);
                    let shifted = builder.ins().ishl_imm(bit64, lane as i64);
                    mask = builder.ins().bor(mask, shifted);
                }
                Some(mask)
            }
            "vec_i64_checksum" => {
                let n = self.operand_value(inst, builder, &args[0])?;
                let func_ref = self.import_ref(builder, IMP_VEC_CHECKSUM);
                let call = builder.ins().call(func_ref, &[n]);
                Some(builder.func.dfg.first_result(call))
            }
            _ => {
                return Err(CodegenError::new(
                    Some(inst.mir_id),
                    CodegenErrorKind::UnrepresentableBuiltin {
                        builtin,
                        reason: "unknown simd builtin",
                    },
                ));
            }
        };
        Ok(value)
    }

    /// Family C: slice handles — views over list/string storage,
    /// allocated in a 40-byte explicit stack slot (the v1
    /// `ExplicitSlot` layout), validated by the v1 slice runtime.
    fn lower_slice(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        builtin: BuiltinId,
        name: &str,
        args: &[Operand],
    ) -> Result<Option<Value>, CodegenError> {
        match name {
            "slice" | "str_slice" => {
                let (base, start, length) = (
                    self.operand_value(inst, builder, &args[0])?,
                    self.operand_value(inst, builder, &args[1])?,
                    self.operand_value(inst, builder, &args[2])?,
                );
                let slot = builder.func.create_sized_stack_slot(StackSlotData {
                    kind: StackSlotKind::ExplicitSlot,
                    size: 40,
                    align_shift: 3,
                });
                let storage = builder.ins().stack_addr(cltypes::I64, slot, 0);
                let kind = if name == "str_slice" { 0 } else { 1 };
                let kind_value = builder.ins().iconst(cltypes::I64, kind);
                let func_ref = self.import_ref(builder, "lpp_slice_init");
                let call = builder
                    .ins()
                    .call(func_ref, &[storage, base, start, length, kind_value]);
                Ok(Some(builder.func.dfg.first_result(call)))
            }
            "slice_len" | "slice_get" | "lpp_slice_get_bool" | "slice_to_str"
            | "str_slice_to_str" => {
                // `slice_get` on a `StrSlice` reads a character and returns a
                // fresh 1-char ARC string, so it dispatches to the string-slice
                // runtime entry point instead of the numeric one. The v1 ABI is
                // frozen (no dedicated builtin), so this overload is resolved
                // here from the receiver's language type.
                let symbol = if name == "slice_get"
                    && matches!(
                        args.first().and_then(|operand| {
                            operand_language_kind(inst.program, inst.types, operand)
                        }),
                        Some(TypeKind::Primitive(PrimitiveType::StrSlice))
                    ) {
                    "lpp_str_slice_get"
                } else {
                    builtin.descriptor().symbol
                };
                let mut call_args: Vec<Value> = Vec::new();
                for operand in args {
                    call_args.push(self.operand_value(inst, builder, operand)?);
                }
                let func_ref = self.import_ref(builder, symbol);
                let call = builder.ins().call(func_ref, &call_args);
                let has_result = !matches!(
                    builtin.descriptor().result,
                    lpp_runtime_abi::generated::AbiType::Void
                );
                Ok(has_result.then(|| builder.func.dfg.first_result(call)))
            }
            _ => Err(unsupported(inst.mir_id, "unknown slice builtin")),
        }
    }

    /// The oracle empties a moved value argument's source local (the
    /// reference dies with the move), so the frame-exit pass never
    /// releases it twice. Non-local and unmanaged operands stay.
    fn empty_moved_local(&mut self, inst: &Inst, builder: &mut FunctionBuilder, operand: &Operand) {
        let Operand::Copy(local) = operand else {
            return;
        };
        let ty = inst
            .program
            .local(*local)
            .expect("validated MIR retains locals")
            .ty;
        if !is_managed(inst.types, ty) {
            return;
        }
        let variable = *inst.local_vars.get(local).expect("local variable");
        let zero = builder.ins().iconst(cltypes::I64, 0);
        builder.def_var(variable, zero);
    }

    /// `list_push(list, value)` / `list_set(list, index, value)`:
    /// element-class dispatch on the list's element, the value
    /// argument moving into the list (the list owns the reference,
    /// the source local is emptied).
    fn lower_list_mutation(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        name: &str,
        args: &[Operand],
    ) -> Result<Option<Value>, CodegenError> {
        let list = self.operand_value(inst, builder, &args[0])?;
        let TypeKind::List(element) = inst.types.kind(
            inst.program
                .local(match &args[0] {
                    Operand::Copy(local) => *local,
                    _ => return Err(unsupported(inst.mir_id, "list mutation of non-local list")),
                })
                .expect("validated MIR retains locals")
                .ty,
        ) else {
            return Err(unsupported(inst.mir_id, "list mutation of non-list"));
        };
        let value_operand = if name == "list_push" {
            &args[1]
        } else {
            &args[2]
        };
        match element_class(inst.types, element) {
            ElementClass::Managed => {
                let value = self.operand_value(inst, builder, value_operand)?;
                let import = if name == "list_push" {
                    IMP_LIST_PUSH_ARC
                } else {
                    IMP_LIST_SET_ARC
                };
                let func_ref = self.import_ref(builder, import);
                if name == "list_push" {
                    builder.ins().call(func_ref, &[list, value]);
                } else {
                    let index = self.operand_value(inst, builder, &args[1])?;
                    builder.ins().call(func_ref, &[list, index, value]);
                }
                self.empty_moved_local(inst, builder, value_operand);
            }
            ElementClass::Scalar => {
                let value = self.operand_value(inst, builder, value_operand)?;
                let stored = coerce_to(builder, value, cltypes::I64);
                let import = if name == "list_push" {
                    IMP_LIST_PUSH
                } else {
                    IMP_LIST_SET
                };
                let func_ref = self.import_ref(builder, import);
                if name == "list_push" {
                    builder.ins().call(func_ref, &[list, stored]);
                } else {
                    let index = self.operand_value(inst, builder, &args[1])?;
                    builder.ins().call(func_ref, &[list, index, stored]);
                }
            }
            ElementClass::Float => {
                let value = self.operand_value(inst, builder, value_operand)?;
                let stored = coerce_to(builder, value, cltypes::F64);
                let import = if name == "list_push" {
                    IMP_LIST_PUSH_FLOAT
                } else {
                    IMP_LIST_SET_FLOAT
                };
                let func_ref = self.import_ref(builder, import);
                if name == "list_push" {
                    builder.ins().call(func_ref, &[list, stored]);
                } else {
                    let index = self.operand_value(inst, builder, &args[1])?;
                    builder.ins().call(func_ref, &[list, index, stored]);
                }
            }
            ElementClass::Bool => {
                let value = self.operand_value(inst, builder, value_operand)?;
                let stored = coerce_to(builder, value, cltypes::I8);
                let import = if name == "list_push" {
                    IMP_LIST_PUSH_BOOL
                } else {
                    IMP_LIST_SET_BOOL
                };
                let func_ref = self.import_ref(builder, import);
                if name == "list_push" {
                    builder.ins().call(func_ref, &[list, stored]);
                } else {
                    let index = self.operand_value(inst, builder, &args[1])?;
                    builder.ins().call(func_ref, &[list, index, stored]);
                }
            }
        }
        Ok(None)
    }

    /// `list_get(list, index)`: element-class dispatch; an element
    /// read gains a reference (the list keeps its own).
    fn lower_list_get(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        args: &[Operand],
    ) -> Result<Option<Value>, CodegenError> {
        let list = self.operand_value(inst, builder, &args[0])?;
        let index = self.operand_value(inst, builder, &args[1])?;
        let TypeKind::List(element) = inst.types.kind(
            inst.program
                .local(match &args[0] {
                    Operand::Copy(local) => *local,
                    _ => return Err(unsupported(inst.mir_id, "list get of non-local list")),
                })
                .expect("validated MIR retains locals")
                .ty,
        ) else {
            return Err(unsupported(inst.mir_id, "list get of non-list"));
        };
        let value = match element_class(inst.types, element) {
            ElementClass::Managed => {
                let func_ref = self.import_ref(builder, IMP_LIST_GET_ARC);
                let call = builder.ins().call(func_ref, &[list, index]);
                let v = builder.func.dfg.first_result(call);
                self.arc_retain(builder, v);
                v
            }
            ElementClass::Scalar => {
                let func_ref = self.import_ref(builder, IMP_LIST_GET);
                let call = builder.ins().call(func_ref, &[list, index]);
                let v = builder.func.dfg.first_result(call);
                coerce_to(
                    builder,
                    v,
                    machine_type(inst.types, element).expect("element type"),
                )
            }
            ElementClass::Float => {
                let func_ref = self.import_ref(builder, IMP_LIST_GET_FLOAT);
                let call = builder.ins().call(func_ref, &[list, index]);
                builder.func.dfg.first_result(call)
            }
            ElementClass::Bool => {
                let func_ref = self.import_ref(builder, IMP_LIST_GET_BOOL);
                let call = builder.ins().call(func_ref, &[list, index]);
                builder.func.dfg.first_result(call)
            }
        };
        Ok(Some(value))
    }

    /// The generated C-ABI `main` wrapper: call `lpp_main`, return 0.
    pub(crate) fn lower_main_wrapper(
        &mut self,
        program: &MirProgram,
        types: &TypeInterner,
    ) -> Result<(), CodegenError> {
        let Some(lpp_main) = self.lpp_main_id else {
            return Ok(());
        };
        let Some(main_fn_id) = self.lpp_main_function_id else {
            return Ok(());
        };
        let main_fn = program
            .function(main_fn_id)
            .expect("the main declaration retained its function");

        let mut sig = self.module.make_signature();
        sig.returns.push(AbiParam::new(cltypes::I32));
        let external_main = self.external_name("main");
        let main_id = self
            .module
            .declare_function(&external_main, Linkage::Export, &sig)
            .map_err(|e| emission_failed(format!("declare export main: {e:?}")))?;
        self.exported.insert("main".to_owned());
        self.entry = Some("main".to_owned());

        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;
        ctx.func.name = UserFuncName::user(1, 0);
        {
            let mut fn_ctx = FunctionBuilderContext::new();
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fn_ctx);
            let entry = builder.create_block();
            builder.switch_to_block(entry);
            if matches!(main_fn.kind, MirFunctionKind::Async) {
                // The async entry is drained here — the execution
                // boundary holds no references: empty env tuple,
                // task, await, destroy.
                let tuple_ref = self.import_ref(&mut builder, IMP_TUPLE_ALLOC);
                let size = builder.ins().iconst(cltypes::I64, 16);
                let zero = builder.ins().iconst(cltypes::I64, 0);
                let call = builder.ins().call(tuple_ref, &[size, zero, zero]);
                let env = builder.func.dfg.first_result(call);
                let thunk_ref = self
                    .module
                    .declare_func_in_func(self.task_thunk_ids[&main_fn_id], builder.func);
                let code = builder.ins().func_addr(cltypes::I64, thunk_ref);
                let task = self.task_new(
                    &mut builder,
                    code,
                    env,
                    Self::task_managed_flag(types, main_fn.return_type),
                );
                let await_ref = self.import_ref(&mut builder, IMP_TASK_AWAIT);
                builder.ins().call(await_ref, &[task]);
                let destroy_ref = self.import_ref(&mut builder, IMP_TASK_DESTROY);
                builder.ins().call(destroy_ref, &[task]);
            } else {
                let main_ref = self.module.declare_func_in_func(lpp_main, builder.func);
                builder.ins().call(main_ref, &[]);
            }
            let zero = builder.ins().iconst(cltypes::I32, 0);
            builder.ins().return_(&[zero]);
            builder.seal_all_blocks();
            builder.finalize();
        }
        self.module
            .define_function(main_id, &mut ctx)
            .map_err(|e| verify_failed(None, format!("define main wrapper: {e:?}")))?;

        Ok(())
    }

    // ── ARC helpers ────────────────────────────────────────────────────────

    fn import_ref(&mut self, builder: &mut FunctionBuilder, name: &'static str) -> FuncRef {
        let id = *self
            .import_ids
            .get(name)
            .unwrap_or_else(|| panic!("pre_scan did not declare import {name}"));
        self.module.declare_func_in_func(id, builder.func)
    }

    /// A new owner is created: `lpp_arc_retain(ptr)`.
    fn arc_retain(&mut self, builder: &mut FunctionBuilder, value: Value) {
        let func_ref = self.import_ref(builder, IMP_ARC_RETAIN);
        builder.ins().call(func_ref, &[value]);
    }

    /// An owning slot dies: `lpp_arc_release(ptr)` (null-safe and
    /// immortal-safe in the v1 runtime, so the call is unconditional).
    fn arc_release(&mut self, builder: &mut FunctionBuilder, value: Value) {
        let func_ref = self.import_ref(builder, IMP_ARC_RELEASE);
        builder.ins().call(func_ref, &[value]);
    }

    /// `lpp_arc_alloc_with_destructor(size, dtor)` — zero-initialized
    /// payload, refcount 1, the aggregate's generated destructor.
    /// Allocate an ARC block for a structural tuple. Tuples are unmanaged (never
    /// retained or released), so the block carries a null destructor and simply
    /// lives for the process; the ARC header keeps the pointer shape identical to
    /// structs so tuple-field projection reuses the offset-load path.
    fn alloc_tuple(&mut self, builder: &mut FunctionBuilder, size: u32) -> Value {
        let func_ref = self.import_ref(builder, IMP_ARC_ALLOC);
        let size_value = builder.ins().iconst(cltypes::I64, size as i64);
        let null_dtor = builder.ins().iconst(cltypes::I64, 0);
        let call = builder.ins().call(func_ref, &[size_value, null_dtor]);
        builder.func.dfg.first_result(call)
    }

    fn arc_alloc(
        &mut self,
        builder: &mut FunctionBuilder,
        size: u32,
        aggregate: MirAggregateId,
    ) -> Value {
        let func_ref = self.import_ref(builder, IMP_ARC_ALLOC);
        let size_value = builder.ins().iconst(cltypes::I64, size as i64);
        let dtor_ref = self
            .module
            .declare_func_in_func(self.dtor_ids[&aggregate], builder.func);
        let dtor_addr = builder.ins().func_addr(cltypes::I64, dtor_ref);
        let call = builder.ins().call(func_ref, &[size_value, dtor_addr]);
        builder.func.dfg.first_result(call)
    }

    /// The exit release pass: every managed local of the function, in
    /// declaration order (the interpreter's rule at `Return`).
    fn exit_release_pass(&mut self, inst: &Inst, builder: &mut FunctionBuilder) {
        for &local in inst.program.function_locals(inst.function) {
            let descriptor = inst
                .program
                .local(local)
                .expect("validated MIR retains locals");
            if matches!(descriptor.kind, lpp_mir::MirLocalKind::Capture) {
                continue;
            }
            if is_managed(inst.types, descriptor.ty) {
                let variable = *inst.local_vars.get(&local).expect("local variable");
                let value = builder.use_var(variable);
                self.arc_release(builder, value);
            }
        }
    }

    // ── inst.function lowering ─────────────────────────────────────────────────

    fn lower_function(
        &mut self,
        program: &MirProgram,
        types: &TypeInterner,
        mir_id: MirFunctionId,
        function: &MirFunction,
    ) -> Result<(), CodegenError> {
        let func_id = *self
            .func_ids
            .get(&mir_id)
            .expect("declare ran before lowering");

        let is_closure = matches!(function.kind, MirFunctionKind::Closure);
        let capture_locals: Vec<MirLocalId> = program
            .function_parameters(function)
            .iter()
            .copied()
            .filter(|local| {
                matches!(
                    program
                        .local(*local)
                        .expect("validated MIR retains locals")
                        .kind,
                    lpp_mir::MirLocalKind::Capture
                )
            })
            .collect();
        let mut capture_slots: HashMap<MirLocalId, u32> = HashMap::new();
        for (i, &local) in capture_locals.iter().enumerate() {
            capture_slots.insert(local, i as u32);
        }

        let mut sig = self.module.make_signature();
        if is_closure {
            sig.params.push(AbiParam::new(cltypes::I64));
        }
        for &param in program.function_parameters(function) {
            if is_closure && capture_locals.contains(&param) {
                continue;
            }
            let ty = program
                .local(param)
                .expect("validated MIR retains locals")
                .ty;
            sig.params
                .push(AbiParam::new(check_value_type(types, ty, mir_id)?));
        }
        if let Some(cl) = machine_type(types, function.return_type) {
            sig.returns.push(AbiParam::new(cl));
        }

        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;
        ctx.func.name = UserFuncName::user(0, mir_id.raw());
        let mut fn_ctx = FunctionBuilderContext::new();
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fn_ctx);

            // Every MIR local is a cranelift variable. Soundness comes
            // from the 4B definite-initialization invariant: a
            // variable is only read where a def dominates.
            let locals = program.function_locals(function);
            let mut local_vars: HashMap<MirLocalId, Variable> = HashMap::new();
            for (index, &local) in locals.iter().enumerate() {
                let ty = program
                    .local(local)
                    .expect("validated MIR retains locals")
                    .ty;
                let cl = machine_type(types, ty).unwrap_or(cltypes::I8);
                let variable = Variable::new(index);
                builder.declare_var(variable, cl);
                local_vars.insert(local, variable);
            }

            let mut cl_blocks: HashMap<BasicBlockId, Block> = HashMap::new();
            for &block in program.function_blocks(function) {
                cl_blocks.insert(block, builder.create_block());
            }

            let entry = *cl_blocks
                .get(&function.entry)
                .expect("every function has its entry block");
            builder.switch_to_block(entry);
            builder.append_block_params_for_function_params(entry);
            let param_vals: Vec<Value> = builder.block_params(entry).to_vec();
            let params = program.function_parameters(function);
            let env_var = if is_closure {
                let env_val = param_vals[0];
                let variable = Variable::new(locals.len());
                builder.declare_var(variable, cltypes::I64);
                builder.def_var(variable, env_val);
                for (i, &local) in capture_locals.iter().enumerate() {
                    let view =
                        builder
                            .ins()
                            .load(cltypes::I64, MemFlags::new(), env_val, (8 * i) as i32);
                    let variable = *local_vars
                        .get(&local)
                        .expect("capture parameters are locals");
                    builder.def_var(variable, view);
                }
                Some(variable)
            } else {
                None
            };
            let mut user_index = 0usize;
            for &param in params {
                if is_closure && capture_locals.contains(&param) {
                    continue;
                }
                let variable = *local_vars.get(&param).expect("parameters are locals");
                builder.def_var(
                    variable,
                    param_vals[if is_closure { 1 } else { 0 } + user_index],
                );
                user_index += 1;
            }

            // Zero-init every non-parameter value local; the 4B
            // invariant keeps the reads sound. Managed locals zero to
            // null, which the v1 ARC runtime treats as a no-op.
            for &local in locals {
                if !params.contains(&local) {
                    let ty = program
                        .local(local)
                        .expect("validated MIR retains locals")
                        .ty;
                    if let Some(cl) = machine_type(types, ty) {
                        let zero = match cl {
                            cltypes::I8 => builder.ins().iconst(cltypes::I8, 0),
                            cltypes::I16 => builder.ins().iconst(cltypes::I16, 0),
                            cltypes::I32 => builder.ins().iconst(cltypes::I32, 0),
                            cltypes::I64 => builder.ins().iconst(cltypes::I64, 0),
                            cltypes::F64 => builder.ins().f64const(0.0),
                            cltypes::I64X2 => {
                                let zero_lane = builder.ins().iconst(cltypes::I64, 0);
                                builder.ins().splat(cltypes::I64X2, zero_lane)
                            }
                            _ => unreachable!("slice machine types"),
                        };
                        let variable = *local_vars.get(&local).expect("local variable");
                        builder.def_var(variable, zero);
                    }
                }
            }

            // The entry block is lowered first (it already holds the
            // parameter defs and zero inits); cranelift forbids
            // switching back to a block that is not pristine. Every
            // other block is lowered in `MirFunctionId`-block order.
            let blocks = program.function_blocks(function);
            let inst = Inst {
                program,
                types,
                local_vars: &local_vars,
                env_var,
                capture_slots: &capture_slots,
                mir_id,
                function,
            };
            // The cursor is already at the entry block (it holds the
            // parameter defs and zero inits), so its body lowers
            // without switching; every other block switches first.
            self.lower_block(&inst, &mut builder, &cl_blocks, function.entry, false)?;
            for &block in blocks {
                if block == function.entry {
                    continue;
                }
                self.lower_block(&inst, &mut builder, &cl_blocks, block, true)?;
            }

            builder.seal_all_blocks();
            builder.finalize();
        }

        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| verify_failed(Some(mir_id), format!("define function: {e:?}")))?;

        Ok(())
    }

    fn lower_block(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        cl_blocks: &HashMap<BasicBlockId, Block>,
        block: BasicBlockId,
        switch: bool,
    ) -> Result<(), CodegenError> {
        let cl_block = *cl_blocks.get(&block).expect("block mapping");
        let basic = inst
            .program
            .block(block)
            .expect("validated MIR retains blocks");
        if switch {
            builder.switch_to_block(cl_block);
        }
        for &instruction in inst.program.block_instructions(basic) {
            let instr = inst
                .program
                .instruction(instruction)
                .expect("validated MIR retains instructions");
            self.lower_instr(inst, builder, instr)?;
        }
        self.lower_terminator(inst, builder, &basic.terminator, cl_blocks)
    }

    fn lower_instr(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        instruction: &lpp_mir::Instruction,
    ) -> Result<(), CodegenError> {
        match &instruction.kind {
            InstructionKind::Store { place, value } => {
                self.lower_store(inst, builder, *place, value)
            }
            InstructionKind::Assign { target, value } => {
                self.lower_assign(inst, builder, *target, value)
            }
        }
    }

    /// A value operand that becomes an owner: a `Copy` of a managed
    /// local retains for the new owner (the source keeps its own
    /// reference and releases it on overwrite or exit).
    fn owned_value(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        operand: &Operand,
    ) -> Result<Value, CodegenError> {
        let value = self.operand_value(inst, builder, operand)?;
        if let Operand::Copy(local) = operand {
            let ty = inst
                .program
                .local(*local)
                .expect("validated MIR retains every local")
                .ty;
            if is_managed(inst.types, ty) {
                self.arc_retain(builder, value);
            }
        }
        Ok(value)
    }

    /// Walk the projection chain up to (but excluding) the final step.
    /// Intermediate steps always yield a base pointer: a struct field
    /// load or an ARC-list element read.
    fn place_chain_base(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        resolution: &PlaceResolution,
    ) -> Result<Value, CodegenError> {
        let mut base = {
            let variable = *inst
                .local_vars
                .get(&resolution.root)
                .expect("validated MIR retains local variables");
            builder.use_var(variable)
        };
        for step in resolution.steps.iter().take(resolution.steps.len() - 1) {
            base = match step {
                PlaceStep::Field { offset, .. } => {
                    builder
                        .ins()
                        .load(cltypes::I64, MemFlags::new(), base, *offset as i32)
                }
                PlaceStep::ListIndex { index, .. } => {
                    let idx = self.operand_value(inst, builder, index)?;
                    let func_ref = self.import_ref(builder, IMP_LIST_GET_ARC);
                    let call = builder.ins().call(func_ref, &[base, idx]);
                    builder.func.dfg.first_result(call)
                }
            };
        }
        Ok(base)
    }

    fn lower_store(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        place: lpp_mir::MirPlaceId,
        value: &Operand,
    ) -> Result<(), CodegenError> {
        let resolution = resolve_place(
            inst.program,
            inst.types,
            place,
            &self.aggregates,
            &self.layouts,
            inst.mir_id,
            true,
        )?;

        if resolution.steps.is_empty() {
            // Bare local: cell death, then the new value lands. A
            // `Copy` source retains for the new owner first (the
            // interpreter's order: the rvalue completes before the
            // old value dies).
            //
            // A capture local is a *view* of the env slot: the slot
            // owns the reference, the local holds none. The writeback
            // below performs the single old-value release; releasing
            // the local's value here as well would double-free the
            // captured object (the env slot and the view alias one
            // reference).
            let is_capture = inst.capture_slots.contains_key(&resolution.root);
            let new_value = self.owned_value(inst, builder, value)?;
            let variable = *inst
                .local_vars
                .get(&resolution.root)
                .expect("validated MIR retains local variables");
            if is_managed(inst.types, resolution.ty) && !is_capture {
                let old = builder.use_var(variable);
                self.arc_release(builder, old);
            }
            builder.def_var(variable, new_value);
            // A capture store is the env-slot writeback: the env slot
            // is the reference owner, the local is a view refreshed to
            // the stored value (retain-then-release order, as above).
            if let Some(&slot) = inst.capture_slots.get(&resolution.root) {
                let env = builder.use_var(inst.env_var.expect("closure env"));
                if is_managed(inst.types, resolution.ty) {
                    let old =
                        builder
                            .ins()
                            .load(cltypes::I64, MemFlags::new(), env, 8 * slot as i32);
                    self.arc_release(builder, old);
                }
                builder
                    .ins()
                    .store(MemFlags::new(), new_value, env, 8 * slot as i32);
            }
            return Ok(());
        }

        match resolution.steps.last().expect("non-empty projections") {
            PlaceStep::Field {
                offset,
                ty,
                managed,
                ..
            } => {
                let base = self.place_chain_base(inst, builder, &resolution)?;
                let new_value = self.owned_value(inst, builder, value)?;
                if *managed {
                    let old =
                        builder
                            .ins()
                            .load(cltypes::I64, MemFlags::new(), base, *offset as i32);
                    self.arc_release(builder, old);
                }
                let field_type = machine_type(inst.types, *ty).expect("checked by the pre-scan");
                let stored = coerce_to(builder, new_value, field_type);
                builder
                    .ins()
                    .store(MemFlags::new(), stored, base, *offset as i32);
                Ok(())
            }
            PlaceStep::ListIndex { element, .. } => {
                let base = self.place_chain_base(inst, builder, &resolution)?;
                let idx = {
                    let index = resolution.steps.last().expect("non-empty projections");
                    let PlaceStep::ListIndex { index, .. } = index else {
                        unreachable!("list index step")
                    };
                    self.operand_value(inst, builder, index)?
                };
                match element_class(inst.types, *element) {
                    ElementClass::Managed => {
                        // lpp_list_set_arc retains the new element and
                        // releases the replaced one (the interpreter's
                        // store_place, runtime-internal).
                        let new_value = self.operand_value(inst, builder, value)?;
                        let func_ref = self.import_ref(builder, IMP_LIST_SET_ARC);
                        builder.ins().call(func_ref, &[base, idx, new_value]);
                    }
                    ElementClass::Scalar => {
                        let raw = self.operand_value(inst, builder, value)?;
                        let stored = coerce_to(builder, raw, cltypes::I64);
                        let func_ref = self.import_ref(builder, IMP_LIST_SET);
                        builder.ins().call(func_ref, &[base, idx, stored]);
                    }
                    ElementClass::Float => {
                        let raw = self.operand_value(inst, builder, value)?;
                        let stored = coerce_to(builder, raw, cltypes::F64);
                        let func_ref = self.import_ref(builder, IMP_LIST_SET_FLOAT);
                        builder.ins().call(func_ref, &[base, idx, stored]);
                    }
                    ElementClass::Bool => {
                        let raw = self.operand_value(inst, builder, value)?;
                        let stored = coerce_to(builder, raw, cltypes::I8);
                        let func_ref = self.import_ref(builder, IMP_LIST_SET_BOOL);
                        builder.ins().call(func_ref, &[base, idx, stored]);
                    }
                }
                Ok(())
            }
        }
    }

    fn lower_assign(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        target: MirLocalId,
        value: &Rvalue,
    ) -> Result<(), CodegenError> {
        let target_ty = inst
            .program
            .local(target)
            .expect("validated MIR retains locals")
            .ty;
        let is_void = is_void_type(inst.types, target_ty);

        let produced = match value {
            Rvalue::Use(operand) => {
                let v = if let Operand::Function(function_id) = operand {
                    self.materialize_function_value(inst, builder, *function_id)?
                } else {
                    self.owned_value(inst, builder, operand)?
                };
                Some(v)
            }
            Rvalue::Unary { operator, operand } => {
                let value = self.operand_value(inst, builder, operand)?;
                let ty = builder.func.dfg.value_type(value);
                let result = match (operator, ty) {
                    (UnaryOperator::Negate, t) if t.is_int() => builder.ins().ineg(value),
                    (UnaryOperator::Negate, cltypes::F64) => builder.ins().fneg(value),
                    (UnaryOperator::Not, cltypes::I8) => {
                        let one = builder.ins().iconst(cltypes::I8, 1);
                        builder.ins().bxor(value, one)
                    }
                    _ => return Err(unsupported(inst.mir_id, "invalid unary operands")),
                };
                Some(result)
            }
            Rvalue::Binary {
                left,
                operator,
                right,
            } => {
                let l = self.operand_value(inst, builder, left)?;
                let r = self.operand_value(inst, builder, right)?;
                let left_is_string = operand_language_kind(inst.program, inst.types, left)
                    == Some(TypeKind::Primitive(PrimitiveType::String));
                // String `+` is contents concatenation, not pointer
                // arithmetic: route it through `lpp_str_concat`, which
                // borrows both operands and returns a fresh owned string.
                if *operator == BinaryOperator::Add && left_is_string {
                    // Coerce a scalar right operand to a string before
                    // concatenation (`"n=" + 3`, and f-string interpolation of a
                    // non-string value).
                    let right_str = match operand_language_kind(inst.program, inst.types, right) {
                        Some(TypeKind::Primitive(PrimitiveType::String)) => r,
                        Some(TypeKind::Primitive(PrimitiveType::Int)) => {
                            let f = self.import_ref(builder, IMP_INT_TO_STR);
                            let c = builder.ins().call(f, &[r]);
                            builder.func.dfg.first_result(c)
                        }
                        Some(TypeKind::Primitive(PrimitiveType::Float)) => {
                            let f = self.import_ref(builder, IMP_FLOAT_TO_STR);
                            let c = builder.ins().call(f, &[r]);
                            builder.func.dfg.first_result(c)
                        }
                        Some(TypeKind::Primitive(PrimitiveType::Bool)) => {
                            let f = self.import_ref(builder, IMP_BOOL_TO_STR);
                            let c = builder.ins().call(f, &[r]);
                            builder.func.dfg.first_result(c)
                        }
                        _ => return Err(unsupported(inst.mir_id, "string concat with non-scalar")),
                    };
                    let func_ref = self.import_ref(builder, IMP_STR_CONCAT);
                    let call_inst = builder.ins().call(func_ref, &[l, right_str]);
                    Some(builder.func.dfg.first_result(call_inst))
                } else {
                    let string_eq =
                        matches!(*operator, BinaryOperator::Equal | BinaryOperator::NotEqual)
                            && left_is_string;
                    let left_kind = operand_language_kind(inst.program, inst.types, left);
                    let right_kind = operand_language_kind(inst.program, inst.types, right);
                    let (l, r, ty) = normalize_integer_operands(
                        builder,
                        l,
                        r,
                        is_signed_integer_kind(left_kind),
                        is_signed_integer_kind(right_kind),
                    );
                    Some(self.lower_binary(builder, *operator, ty, string_eq, l, r)?)
                }
            }
            Rvalue::Call { callee, arguments } => {
                let args_ops = inst.program.operands(*arguments);
                match callee {
                    Operand::Function(callee_id) => {
                        let callee_fn = inst
                            .program
                            .function(*callee_id)
                            .expect("validated MIR retains functions");
                        match callee_fn.kind {
                            // Closures are called through their
                            // capsule; the pre-scan rejects the
                            // function-operand spelling.
                            MirFunctionKind::Closure => {
                                return Err(unsupported(
                                    inst.mir_id,
                                    "closure as function operand",
                                ));
                            }
                            // An async call builds the task env tuple
                            // and the handle through the thunk.
                            MirFunctionKind::Async => {
                                let env = self.task_env_for_args(inst, builder, args_ops)?;
                                let thunk_ref = self.module.declare_func_in_func(
                                    self.task_thunk_ids[callee_id],
                                    builder.func,
                                );
                                let code = builder.ins().func_addr(cltypes::I64, thunk_ref);
                                let task = self.task_new(
                                    builder,
                                    code,
                                    env,
                                    Self::task_managed_flag(inst.types, callee_fn.return_type),
                                );
                                Some(task)
                            }
                            MirFunctionKind::Function => {
                                let mut args: Vec<Value> = Vec::with_capacity(args_ops.len());
                                for operand in args_ops {
                                    // A managed argument retains for
                                    // the callee (the callee's exit
                                    // pass releases it); the source
                                    // keeps its own reference.
                                    args.push(self.owned_value(inst, builder, operand)?);
                                }
                                let func_ref = self
                                    .module
                                    .declare_func_in_func(self.func_ids[callee_id], builder.func);
                                let call_inst = builder.ins().call(func_ref, &args);
                                (machine_type(inst.types, callee_fn.return_type).is_some())
                                    .then(|| builder.func.dfg.first_result(call_inst))
                            }
                        }
                    }
                    Operand::Copy(local) => {
                        // A capsule call: the code pointer and env come
                        // from the capsule; the provenance class picks
                        // the ABI.
                        let class = self
                            .value_classes
                            .get(&(inst.mir_id, *local))
                            .copied()
                            .expect("pre-scan classified every function value");
                        let callee_ty = {
                            inst.program
                                .local(*local)
                                .expect("validated MIR retains locals")
                                .ty
                        };
                        let (param_tys, result_ty) = match inst.types.kind(callee_ty) {
                            TypeKind::Function { parameters, result } => {
                                (inst.types.list(parameters).to_vec(), result)
                            }
                            _ => return Err(unsupported(inst.mir_id, "non-function callee")),
                        };
                        let capsule = self.operand_value(inst, builder, &Operand::Copy(*local))?;
                        let code = builder
                            .ins()
                            .load(cltypes::I64, MemFlags::new(), capsule, 0);
                        match class {
                            ValueClass::Closure => {
                                let env =
                                    builder
                                        .ins()
                                        .load(cltypes::I64, MemFlags::new(), capsule, 8);
                                if param_tys.is_empty() {
                                    // The zero-parameter capsule code
                                    // is the task-code trampoline:
                                    // (env) -> boxed I64.
                                    let mut sig = self.module.make_signature();
                                    sig.params.push(AbiParam::new(cltypes::I64));
                                    sig.returns.push(AbiParam::new(cltypes::I64));
                                    let sig_ref = builder.import_signature(sig);
                                    let call = builder.ins().call_indirect(sig_ref, code, &[env]);
                                    let boxed = builder.func.dfg.first_result(call);
                                    unbox_result(inst.types, builder, boxed, result_ty)
                                } else {
                                    let mut sig = self.module.make_signature();
                                    sig.params.push(AbiParam::new(cltypes::I64));
                                    for param_ty in &param_tys {
                                        sig.params.push(AbiParam::new(
                                            machine_type(inst.types, *param_ty)
                                                .expect("pre-scan checked the types"),
                                        ));
                                    }
                                    if let Some(cl) = machine_type(inst.types, result_ty) {
                                        sig.returns.push(AbiParam::new(cl));
                                    }
                                    let sig_ref = builder.import_signature(sig);
                                    let mut call_args: Vec<Value> = vec![env];
                                    for operand in args_ops {
                                        call_args.push(self.owned_value(inst, builder, operand)?);
                                    }
                                    let call =
                                        builder.ins().call_indirect(sig_ref, code, &call_args);
                                    (machine_type(inst.types, result_ty).is_some())
                                        .then(|| builder.func.dfg.first_result(call))
                                }
                            }
                            ValueClass::FunctionValue => {
                                if matches!(inst.types.kind(result_ty), TypeKind::Task(_)) {
                                    // A call through an async function
                                    // value constructs a task; the
                                    // capsule code is the task thunk.
                                    let env = self.task_env_for_args(inst, builder, args_ops)?;
                                    let task = self.task_new(
                                        builder,
                                        code,
                                        env,
                                        Self::task_managed_flag(inst.types, result_ty),
                                    );
                                    Some(task)
                                } else {
                                    let mut sig = self.module.make_signature();
                                    for param_ty in &param_tys {
                                        sig.params.push(AbiParam::new(
                                            machine_type(inst.types, *param_ty)
                                                .expect("pre-scan checked the types"),
                                        ));
                                    }
                                    if let Some(cl) = machine_type(inst.types, result_ty) {
                                        sig.returns.push(AbiParam::new(cl));
                                    }
                                    let sig_ref = builder.import_signature(sig);
                                    let mut call_args: Vec<Value> =
                                        Vec::with_capacity(args_ops.len());
                                    for operand in args_ops {
                                        call_args.push(self.owned_value(inst, builder, operand)?);
                                    }
                                    let call =
                                        builder.ins().call_indirect(sig_ref, code, &call_args);
                                    (machine_type(inst.types, result_ty).is_some())
                                        .then(|| builder.func.dfg.first_result(call))
                                }
                            }
                        }
                    }
                    _ => return Err(unsupported(inst.mir_id, "non-function callee")),
                }
            }
            Rvalue::Load(place) => {
                let resolution = resolve_place(
                    inst.program,
                    inst.types,
                    *place,
                    &self.aggregates,
                    &self.layouts,
                    inst.mir_id,
                    false,
                )?;
                if resolution.steps.is_empty() {
                    Some(self.operand_value(inst, builder, &Operand::Copy(resolution.root))?)
                } else {
                    let base = self.place_chain_base(inst, builder, &resolution)?;
                    let loaded = match resolution.steps.last().expect("non-empty projections") {
                        PlaceStep::Field {
                            offset,
                            ty,
                            managed,
                            ..
                        } => {
                            let field_type =
                                machine_type(inst.types, *ty).expect("checked by the pre-scan");
                            let v = builder.ins().load(
                                field_type,
                                MemFlags::new(),
                                base,
                                *offset as i32,
                            );
                            if *managed {
                                // A field read gains a reference (the
                                // container keeps its own).
                                self.arc_retain(builder, v);
                            }
                            v
                        }
                        PlaceStep::ListIndex { element, index } => {
                            let idx = self.operand_value(inst, builder, index)?;
                            match element_class(inst.types, *element) {
                                ElementClass::Managed => {
                                    let func_ref = self.import_ref(builder, IMP_LIST_GET_ARC);
                                    let call = builder.ins().call(func_ref, &[base, idx]);
                                    let v = builder.func.dfg.first_result(call);
                                    // An element read gains a reference.
                                    self.arc_retain(builder, v);
                                    v
                                }
                                ElementClass::Scalar => {
                                    let func_ref = self.import_ref(builder, IMP_LIST_GET);
                                    let call = builder.ins().call(func_ref, &[base, idx]);
                                    let v = builder.func.dfg.first_result(call);
                                    coerce_to(
                                        builder,
                                        v,
                                        machine_type(inst.types, *element).expect("element type"),
                                    )
                                }
                                ElementClass::Float => {
                                    let func_ref = self.import_ref(builder, IMP_LIST_GET_FLOAT);
                                    let call = builder.ins().call(func_ref, &[base, idx]);
                                    builder.func.dfg.first_result(call)
                                }
                                ElementClass::Bool => {
                                    let func_ref = self.import_ref(builder, IMP_LIST_GET_BOOL);
                                    let call = builder.ins().call(func_ref, &[base, idx]);
                                    builder.func.dfg.first_result(call)
                                }
                            }
                        }
                    };
                    Some(loaded)
                }
            }
            Rvalue::ListLen(operand) => {
                let list = self.operand_value(inst, builder, operand)?;
                let func_ref = self.import_ref(builder, IMP_LIST_LEN);
                let call = builder.ins().call(func_ref, &[list]);
                Some(builder.func.dfg.first_result(call))
            }
            Rvalue::List(operands) => {
                let TypeKind::List(element) = inst.types.kind(target_ty) else {
                    return Err(unsupported(inst.mir_id, "list literal of non-list"));
                };
                let arc = is_managed(inst.types, element);
                let list = {
                    let func_ref =
                        self.import_ref(builder, if arc { IMP_LIST_NEW_ARC } else { IMP_LIST_NEW });
                    let call = builder.ins().call(func_ref, &[]);
                    builder.func.dfg.first_result(call)
                };
                for operand in inst.program.operands(*operands) {
                    // Elements enter the list as owners: a `Copy`
                    // element retains for the list (push_arc is
                    // runtime-internal for managed elements; a scalar
                    // push stores bits). The source keeps its own
                    // reference.
                    let v = self.owned_value(inst, builder, operand)?;
                    match element_class(inst.types, element) {
                        ElementClass::Managed => {
                            // The element already carries the list's
                            // reference from `owned_value`; push the
                            // pointer without another retain.
                            let func_ref = self.import_ref(builder, IMP_LIST_PUSH_ARC);
                            builder.ins().call(func_ref, &[list, v]);
                        }
                        ElementClass::Scalar => {
                            let stored = coerce_to(builder, v, cltypes::I64);
                            let func_ref = self.import_ref(builder, IMP_LIST_PUSH);
                            builder.ins().call(func_ref, &[list, stored]);
                        }
                        ElementClass::Float => {
                            let stored = coerce_to(builder, v, cltypes::F64);
                            let func_ref = self.import_ref(builder, IMP_LIST_PUSH_FLOAT);
                            builder.ins().call(func_ref, &[list, stored]);
                        }
                        ElementClass::Bool => {
                            let stored = coerce_to(builder, v, cltypes::I8);
                            let func_ref = self.import_ref(builder, IMP_LIST_PUSH_BOOL);
                            builder.ins().call(func_ref, &[list, stored]);
                        }
                    }
                }
                Some(list)
            }
            Rvalue::ConstructStruct { aggregate, fields } => {
                let layout = self
                    .layouts
                    .get(aggregate)
                    .expect("pre-scan laid out every nominal")
                    .clone();
                let ptr = self.arc_alloc(builder, layout.total_size, *aggregate);
                let values = inst.program.operands(*fields);
                if values.len() != layout.struct_fields.len() {
                    return Err(unsupported(inst.mir_id, "struct field count"));
                }
                for (value_operand, slot) in values.iter().zip(layout.struct_fields.iter()) {
                    let v = self.owned_value(inst, builder, value_operand)?;
                    let stored = coerce_to(
                        builder,
                        v,
                        machine_type(inst.types, slot.ty).expect("field type"),
                    );
                    builder
                        .ins()
                        .store(MemFlags::new(), stored, ptr, slot.offset as i32);
                }
                Some(ptr)
            }
            Rvalue::ConstructVariant {
                aggregate,
                variant,
                fields,
            } => {
                let layout = self
                    .layouts
                    .get(aggregate)
                    .expect("pre-scan laid out every nominal")
                    .clone();
                let ordinal = inst
                    .program
                    .variant(*variant)
                    .expect("validated MIR retains every variant")
                    .ordinal;
                let ptr = self.arc_alloc(builder, layout.total_size, *aggregate);
                let tag = builder.ins().iconst(cltypes::I64, ordinal as i64);
                builder.ins().store(MemFlags::new(), tag, ptr, 0);
                let slots = &layout.variant_fields[ordinal as usize];
                let values = inst.program.operands(*fields);
                if values.len() != slots.len() {
                    return Err(unsupported(inst.mir_id, "variant field count"));
                }
                for (value_operand, slot) in values.iter().zip(slots.iter()) {
                    let v = self.owned_value(inst, builder, value_operand)?;
                    let stored = coerce_to(
                        builder,
                        v,
                        machine_type(inst.types, slot.ty).expect("field type"),
                    );
                    builder
                        .ins()
                        .store(MemFlags::new(), stored, ptr, slot.offset as i32);
                }
                Some(ptr)
            }
            Rvalue::Builtin { builtin, arguments } => {
                let args = inst.program.operands(*arguments);
                match *builtin {
                    PRINT_STR => {
                        let value = self.operand_value(inst, builder, &args[0])?;
                        let func_ref = self.import_ref(builder, IMP_PRINT_STR);
                        builder.ins().call(func_ref, &[value]);
                        None
                    }
                    LIST_NEW => {
                        let TypeKind::List(element) = inst.types.kind(target_ty) else {
                            return Err(unsupported(inst.mir_id, "list allocation of non-list"));
                        };
                        let arc = is_managed(inst.types, element);
                        let func_ref = self
                            .import_ref(builder, if arc { IMP_LIST_NEW_ARC } else { IMP_LIST_NEW });
                        let call = builder.ins().call(func_ref, &[]);
                        Some(builder.func.dfg.first_result(call))
                    }
                    other => self.lower_builtin_families(inst, builder, other, args)?,
                }
            }
            Rvalue::Tuple(elements) => {
                let (slots, total_size) = crate::layout::tuple_layout(inst.types, target_ty)
                    .ok_or_else(|| unsupported(inst.mir_id, "tuple of non-tuple type"))?;
                let values = inst.program.operands(*elements);
                if values.len() != slots.len() {
                    return Err(unsupported(inst.mir_id, "tuple element count"));
                }
                let ptr = self.alloc_tuple(builder, total_size);
                for (value_operand, (offset, element_ty)) in values.iter().zip(slots.iter()) {
                    let v = self.owned_value(inst, builder, value_operand)?;
                    let stored = coerce_to(
                        builder,
                        v,
                        machine_type(inst.types, *element_ty).expect("tuple element type"),
                    );
                    builder
                        .ins()
                        .store(MemFlags::new(), stored, ptr, *offset as i32);
                }
                Some(ptr)
            }
            Rvalue::MakeClosure {
                function: closure_id,
                captures,
            } => {
                let closure_fn = inst
                    .program
                    .function(*closure_id)
                    .expect("validated MIR retains functions");
                let capture_args = inst.program.operands(*captures);
                let zero_params = user_parameters(inst.program, closure_fn).is_empty();
                // The raw capture env (slots at 8*i) — NULL when the
                // closure captures nothing.
                let raw_env = if capture_args.is_empty() {
                    builder.ins().iconst(cltypes::I64, 0)
                } else {
                    let dtor_ref = self
                        .module
                        .declare_func_in_func(self.closure_dtor_ids[closure_id], builder.func);
                    let env =
                        self.arc_alloc_ptr(builder, (8 * capture_args.len()) as u32, dtor_ref);
                    for (i, operand) in capture_args.iter().enumerate() {
                        let captured_ty = match operand {
                            Operand::Copy(local) => {
                                inst.program
                                    .local(*local)
                                    .expect("validated MIR retains locals")
                                    .ty
                            }
                            _ => return Err(unsupported(inst.mir_id, "non-local capture")),
                        };
                        let value = self.operand_value(inst, builder, operand)?;
                        let value = if is_managed(inst.types, captured_ty) {
                            self.arc_retain_value(builder, value)
                        } else {
                            value
                        };
                        builder
                            .ins()
                            .store(MemFlags::new(), value, env, 8 * i as i32);
                    }
                    env
                };
                // The capsule: [code, env].
                let dtor_id = *self
                    .import_ids
                    .get(IMP_CLOSURE_DESTROY)
                    .expect("pre-scan declared lpp_closure_destroy");
                let dtor_ref = self.module.declare_func_in_func(dtor_id, builder.func);
                let capsule = self.arc_alloc_ptr(builder, 16, dtor_ref);
                let code_ref = if zero_params {
                    self.closure_thunk_ids[closure_id]
                } else {
                    self.func_ids[closure_id]
                };
                let fn_ref = self.module.declare_func_in_func(code_ref, builder.func);
                let code = builder.ins().func_addr(cltypes::I64, fn_ref);
                builder.ins().store(MemFlags::new(), code, capsule, 0);
                if zero_params {
                    // Spawnable: the capsule env is the 1-slot task
                    // env tuple holding the raw env (a NULL slot is
                    // released as a no-op), so the same code pointer
                    // serves the direct call and the spawned task.
                    let tuple_ref = self.import_ref(builder, IMP_TUPLE_ALLOC);
                    let size = builder.ins().iconst(cltypes::I64, 24);
                    let mask = builder.ins().iconst(cltypes::I64, 1);
                    let offsets = builder.ins().iconst(cltypes::I64, 16);
                    let call = builder.ins().call(tuple_ref, &[size, mask, offsets]);
                    let wrapper = builder.func.dfg.first_result(call);
                    // The allocation reference of the raw env is
                    // transferred to the wrapper slot (released by the
                    // tuple destructor at wrapper death) — no retain:
                    // retaining would leak one reference per closure.
                    let wrapped = raw_env;
                    builder.ins().store(MemFlags::new(), wrapped, wrapper, 16);
                    builder.ins().store(MemFlags::new(), wrapper, capsule, 8);
                } else {
                    builder.ins().store(MemFlags::new(), raw_env, capsule, 8);
                }
                Some(capsule)
            }
            Rvalue::Await(operand) => {
                let Operand::Copy(local) = operand else {
                    return Err(unsupported(inst.mir_id, "await of non-local task"));
                };
                let task = self.operand_value(inst, builder, operand)?;
                let func_ref = self.import_ref(builder, IMP_TASK_AWAIT);
                let call = builder.ins().call(func_ref, &[task]);
                let boxed = builder.func.dfg.first_result(call);
                let TypeKind::Task(inner) = inst.types.kind(
                    inst.program
                        .local(*local)
                        .expect("validated MIR retains locals")
                        .ty,
                ) else {
                    return Err(unsupported(inst.mir_id, "await of non-task"));
                };
                unbox_result(inst.types, builder, boxed, inner)
            }
            Rvalue::Spawn(operand) => {
                let Operand::Copy(local) = operand else {
                    return Err(unsupported(inst.mir_id, "spawn of non-local value"));
                };
                let capsule = self.operand_value(inst, builder, &Operand::Copy(*local))?;
                let code = builder
                    .ins()
                    .load(cltypes::I64, MemFlags::new(), capsule, 0);
                let env = builder
                    .ins()
                    .load(cltypes::I64, MemFlags::new(), capsule, 8);
                // The task holds the capsule's env for the closure's
                // (eager) lifetime; the capsule keeps its own.
                let env_retained = self.arc_retain_value(builder, env);
                let callee_ty = inst
                    .program
                    .local(*local)
                    .expect("validated MIR retains locals")
                    .ty;
                let TypeKind::Function { result, .. } = inst.types.kind(callee_ty) else {
                    return Err(unsupported(inst.mir_id, "spawn of non-function"));
                };
                let task = self.task_new(
                    builder,
                    code,
                    env_retained,
                    Self::task_managed_flag(inst.types, result),
                );
                let poll_ref = self.import_ref(builder, IMP_TASK_POLL);
                builder.ins().call(poll_ref, &[task]);
                let destroy_ref = self.import_ref(builder, IMP_TASK_DESTROY);
                builder.ins().call(destroy_ref, &[task]);
                None
            }
        };

        if let Some(value) = produced
            && !is_void
        {
            // Cell death: the slot's old reference (if any) is
            // released after the new value is fully produced — the
            // interpreter's order (a self-assignment retains before it
            // releases).
            let variable = *inst
                .local_vars
                .get(&target)
                .expect("validated MIR retains local variables");
            if is_managed(inst.types, target_ty) {
                let old = builder.use_var(variable);
                self.arc_release(builder, old);
            }
            // Context-typed integer literals enter MIR as canonical i64
            // constants and are materialized through a local carrying the
            // inferred fixed-width type. Coerce at the assignment boundary so
            // Cranelift's SSA variable type agrees with that language type.
            let value = machine_type(inst.types, target_ty)
                .map(|expected| coerce_to(builder, value, expected))
                .unwrap_or(value);
            builder.def_var(variable, value);
        }
        Ok(())
    }

    fn lower_binary(
        &mut self,
        builder: &mut FunctionBuilder,
        operator: BinaryOperator,
        ty: CLType,
        string_eq: bool,
        l: Value,
        r: Value,
    ) -> Result<Value, CodegenError> {
        let mut compare = |cc_int: IntCC, cc_float: FloatCC| -> Value {
            let bool_val = match ty {
                cltypes::F64 => builder.ins().fcmp(cc_float, l, r),
                _ => builder.ins().icmp(cc_int, l, r),
            };
            let one = builder.ins().iconst(cltypes::I8, 1);
            let zero = builder.ins().iconst(cltypes::I8, 0);
            builder.ins().select(bool_val, one, zero)
        };
        Ok(match operator {
            BinaryOperator::Add => match ty {
                cltypes::F64 => builder.ins().fadd(l, r),
                _ => builder.ins().iadd(l, r),
            },
            BinaryOperator::Subtract => match ty {
                cltypes::F64 => builder.ins().fsub(l, r),
                _ => builder.ins().isub(l, r),
            },
            BinaryOperator::Multiply => match ty {
                cltypes::F64 => builder.ins().fmul(l, r),
                _ => builder.ins().imul(l, r),
            },
            BinaryOperator::Divide => match ty {
                cltypes::F64 => builder.ins().fdiv(l, r),
                _ => builder.ins().sdiv(l, r),
            },
            BinaryOperator::Modulo if ty == cltypes::F64 => {
                let func_ref = self.import_ref(builder, IMP_FMOD);
                let call_inst = builder.ins().call(func_ref, &[l, r]);
                builder.func.dfg.first_result(call_inst)
            }
            BinaryOperator::Modulo => builder.ins().srem(l, r),
            BinaryOperator::BitAnd => builder.ins().band(l, r),
            BinaryOperator::BitOr => builder.ins().bor(l, r),
            BinaryOperator::BitXor => builder.ins().bxor(l, r),
            BinaryOperator::ShiftLeft => builder.ins().ishl(l, r),
            BinaryOperator::ShiftRight => builder.ins().sshr(l, r),
            BinaryOperator::Equal if string_eq => {
                let func_ref = self.import_ref(builder, IMP_STR_EQ);
                let call_inst = builder.ins().call(func_ref, &[l, r]);
                let raw = builder.func.dfg.first_result(call_inst);
                builder.ins().ireduce(cltypes::I8, raw)
            }
            BinaryOperator::Equal => compare(IntCC::Equal, FloatCC::Equal),
            BinaryOperator::NotEqual if string_eq => {
                // `lpp_str_eq` returns 1 when the contents match, so
                // inequality is `eq_result == 0`.
                let func_ref = self.import_ref(builder, IMP_STR_EQ);
                let call_inst = builder.ins().call(func_ref, &[l, r]);
                let raw = builder.func.dfg.first_result(call_inst);
                let zero = builder.ins().iconst(cltypes::I64, 0);
                // `icmp` already yields the I8 condition value.
                builder.ins().icmp(IntCC::Equal, raw, zero)
            }
            BinaryOperator::NotEqual => compare(IntCC::NotEqual, FloatCC::NotEqual),
            BinaryOperator::Less => compare(IntCC::SignedLessThan, FloatCC::LessThan),
            BinaryOperator::Greater => compare(IntCC::SignedGreaterThan, FloatCC::GreaterThan),
            BinaryOperator::LessEqual => {
                compare(IntCC::SignedLessThanOrEqual, FloatCC::LessThanOrEqual)
            }
            BinaryOperator::GreaterEqual => {
                compare(IntCC::SignedGreaterThanOrEqual, FloatCC::GreaterThanOrEqual)
            }
            BinaryOperator::LogicalAnd => builder.ins().band(l, r),
            BinaryOperator::LogicalOr => builder.ins().bor(l, r),
        })
    }

    fn lower_terminator(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        terminator: &Terminator,
        cl_blocks: &HashMap<BasicBlockId, Block>,
    ) -> Result<(), CodegenError> {
        match terminator {
            Terminator::Goto(target) => {
                let block = *cl_blocks
                    .get(target)
                    .expect("validated CFG retains jump targets");
                builder.ins().jump(block, &[]);
                Ok(())
            }
            Terminator::Branch {
                condition,
                then_block,
                else_block,
            } => {
                let cond = self.operand_value(inst, builder, condition)?;
                let cond_bool = builder.ins().icmp_imm(IntCC::NotEqual, cond, 0);
                let then_block = *cl_blocks
                    .get(then_block)
                    .expect("validated CFG retains branch targets");
                let else_block = *cl_blocks
                    .get(else_block)
                    .expect("validated CFG retains branch targets");
                builder
                    .ins()
                    .brif(cond_bool, then_block, &[], else_block, &[]);
                Ok(())
            }
            Terminator::SwitchEnum {
                subject,
                aggregate,
                targets,
            } => {
                let _ = aggregate;
                // The subject is a plain read (no retain — the
                // interpreter's rule); the tag is the I64 at offset 0.
                let subject_value = self.operand_value(inst, builder, subject)?;
                let tag = builder
                    .ins()
                    .load(cltypes::I64, MemFlags::new(), subject_value, 0);
                let target_blocks: Vec<Block> = inst
                    .program
                    .switch_targets(*targets)
                    .iter()
                    .map(|&target| {
                        *cl_blocks
                            .get(&target)
                            .expect("validated CFG retains switch targets")
                    })
                    .collect();
                emit_tag_cascade(builder, tag, &target_blocks);
                Ok(())
            }
            Terminator::Return(operand) => {
                let return_value = if let Some(operand) = operand {
                    let value = self.operand_value(inst, builder, operand)?;
                    // A managed return retains for the caller: the
                    // exit pass below releases the slot's own
                    // reference, not the caller's.
                    if is_managed(inst.types, inst.function.return_type) {
                        self.arc_retain(builder, value);
                    }
                    Some(value)
                } else {
                    None
                };
                // The exit release pass: every managed slot that still
                // owns a reference releases it, in declaration order.
                self.exit_release_pass(inst, builder);
                let values: Vec<Value> = return_value.into_iter().collect();
                builder.ins().return_(&values);
                Ok(())
            }
            Terminator::Unreachable => {
                builder.ins().trap(TrapCode::unwrap_user(1));
                Ok(())
            }
        }
    }

    // ── operand values ────────────────────────────────────────────────────

    /// An operand in a value position: variable loads, constants, and
    /// deduplicated string data.
    fn operand_value(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        operand: &Operand,
    ) -> Result<Value, CodegenError> {
        match operand {
            Operand::Copy(local) => {
                let variable = *inst
                    .local_vars
                    .get(local)
                    .expect("validated MIR retains local variables");
                Ok(builder.use_var(variable))
            }
            Operand::Constant(constant) => Ok(match constant {
                Constant::Integer(value) => builder.ins().iconst(cltypes::I64, *value),
                Constant::FloatBits(bits) => builder.ins().f64const(f64::from_bits(*bits)),
                Constant::Bool(value) => builder
                    .ins()
                    .iconst(cltypes::I8, i64::from(u8::from(*value))),
                Constant::Character {
                    origin: _,
                    character,
                } => builder.ins().iconst(cltypes::I32, *character as i64),
                Constant::String { origin: _, string } => {
                    self.string_constant(inst, builder, *string)?
                }
            }),
            Operand::Function(_) => Err(unsupported(inst.mir_id, "inst.function value")),
        }
    }

    /// String constants use the v1 runtime layout, deduplicated per
    /// `MirStringId` (the interpreter interns literals per inst.program, so
    /// pointer identity of equal literals matches between the object
    /// and the oracle).
    fn string_constant(
        &mut self,
        inst: &Inst,
        builder: &mut FunctionBuilder,
        string: MirStringId,
    ) -> Result<Value, CodegenError> {
        if let Some(&data_id) = self.emitted_strings.get(&string) {
            let local_id = self.module.declare_data_in_func(data_id, builder.func);
            let base = builder.ins().symbol_value(cltypes::I64, local_id);
            return Ok(builder.ins().iadd_imm(base, STRING_HEADER_OFFSET));
        }

        let text = inst
            .program
            .string(string)
            .cloned()
            .expect("validated MIR retains every string");
        let symbol_name = format!("lpp_str_{}", string.raw());

        let data_id = self
            .module
            .declare_data(&symbol_name, Linkage::Export, false, false)
            .map_err(|e| {
                emission_failed(format!(
                    "declare string data {symbol_name} in {:?}: {e:?}",
                    inst.mir_id
                ))
            })?;

        let mut bytes = Vec::with_capacity(STRING_HEADER_OFFSET as usize + text.len() + 1);
        bytes.extend_from_slice(&LPP_ARC_MAGIC.to_le_bytes());
        bytes.extend_from_slice(&LPP_ARC_MAGIC.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 16]);
        bytes.extend_from_slice(text.as_bytes());
        bytes.push(0);

        let mut data_ctx = DataDescription::new();
        data_ctx.define(bytes.into_boxed_slice());
        data_ctx.set_align(16);
        self.module.define_data(data_id, &data_ctx).map_err(|e| {
            emission_failed(format!(
                "define string data {symbol_name} in {:?}: {e:?}",
                inst.mir_id
            ))
        })?;
        self.emitted_strings.insert(string, data_id);

        let local_id = self.module.declare_data_in_func(data_id, builder.func);
        let base = builder.ins().symbol_value(cltypes::I64, local_id);
        Ok(builder.ins().iadd_imm(base, STRING_HEADER_OFFSET))
    }
}

/// A free inst.function so it can be shared by the terminator and the
/// destructor lowering without borrowing the mutable module twice.
fn emit_tag_cascade(builder: &mut FunctionBuilder, tag: Value, targets: &[Block]) {
    let count = targets.len();
    if count == 1 {
        // A single-variant dense switch is an unconditional jump.
        builder.ins().jump(targets[0], &[]);
        return;
    }
    let cascade: Vec<Block> = (0..count).map(|_| builder.create_block()).collect();
    // The dispatch block takes its one terminator here; the cascade
    // blocks are still pristine when their bodies are filled below.
    builder.ins().jump(cascade[0], &[]);
    for index in 0..count {
        builder.switch_to_block(cascade[index]);
        if index + 1 < count {
            let expected = builder.ins().iconst(cltypes::I64, index as i64);
            let matches_ordinal = builder.ins().icmp(IntCC::Equal, tag, expected);
            builder.ins().brif(
                matches_ordinal,
                targets[index],
                &[],
                cascade[index + 1],
                &[],
            );
        } else {
            // The dense ordinal cannot be out of range in verified
            // MIR; the final target takes the rest.
            builder.ins().jump(targets[index], &[]);
        }
    }
}

/// Match `value` to the exact machine type of the storage site.
/// Box a user result into the `I64` task-result shape (the v1 thunk
/// rule): void → `0`, bool uextend, float bit-cast, `I64` unchanged.
fn box_result(
    types: &TypeInterner,
    builder: &mut FunctionBuilder,
    value: Option<Value>,
    return_type: TypeId,
) -> Value {
    match machine_type(types, return_type) {
        None => builder.ins().iconst(cltypes::I64, 0),
        Some(cltypes::I64) => value.expect("I64 result"),
        Some(cltypes::I8) => {
            let v = value.expect("bool result");
            builder.ins().uextend(cltypes::I64, v)
        }
        Some(cltypes::F64) => {
            let v = value.expect("float result");
            builder.ins().bitcast(cltypes::I64, MemFlags::new(), v)
        }
        Some(_) => unreachable!("slice machine types"),
    }
}

/// The inverse of `box_result` at a call site: unbox the `I64`
/// task/closure thunk result to the user result type (`None` for
/// void).
fn unbox_result(
    types: &TypeInterner,
    builder: &mut FunctionBuilder,
    boxed: Value,
    return_type: TypeId,
) -> Option<Value> {
    match machine_type(types, return_type) {
        None => None,
        Some(cltypes::I64) => Some(boxed),
        Some(cltypes::I8) => Some(builder.ins().ireduce(cltypes::I8, boxed)),
        Some(cltypes::F64) => Some(builder.ins().bitcast(cltypes::F64, MemFlags::new(), boxed)),
        Some(_) => unreachable!("slice machine types"),
    }
}

fn is_signed_integer_kind(kind: Option<TypeKind>) -> bool {
    matches!(
        kind,
        Some(TypeKind::Primitive(
            PrimitiveType::Int | PrimitiveType::I8 | PrimitiveType::I16 | PrimitiveType::I32,
        ))
    )
}

fn normalize_integer_operands(
    builder: &mut FunctionBuilder,
    left: Value,
    right: Value,
    left_signed: bool,
    right_signed: bool,
) -> (Value, Value, CLType) {
    let left_type = builder.func.dfg.value_type(left);
    let right_type = builder.func.dfg.value_type(right);
    if !(left_type.is_int() && right_type.is_int()) || left_type == right_type {
        return (left, right, left_type);
    }
    let target = if left_type.bits() >= right_type.bits() {
        left_type
    } else {
        right_type
    };
    (
        widen_integer_to(builder, left, target, left_signed),
        widen_integer_to(builder, right, target, right_signed),
        target,
    )
}

fn widen_integer_to(
    builder: &mut FunctionBuilder,
    value: Value,
    target: CLType,
    signed: bool,
) -> Value {
    let have = builder.func.dfg.value_type(value);
    if have == target {
        return value;
    }
    if have.bits() > target.bits() {
        return builder.ins().ireduce(target, value);
    }
    if signed {
        builder.ins().sextend(target, value)
    } else {
        builder.ins().uextend(target, value)
    }
}

fn coerce_to(builder: &mut FunctionBuilder, value: Value, target: CLType) -> Value {
    let have = builder.func.dfg.value_type(value);
    if have == target {
        return value;
    }
    if have.is_int() && target.is_int() {
        if have.bits() > target.bits() {
            builder.ins().ireduce(target, value)
        } else {
            builder.ins().uextend(target, value)
        }
    } else {
        value
    }
}
