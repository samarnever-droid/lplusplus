use super::*;

// ── v1 runtime imports ─────────────────────────────────────────────────────
// The list-family symbols are registry entries (`BuiltinAbi` in the
// checked-in generated table); the ARC allocator/retainer/releaser are
// freestanding-runtime symbols the backend declares by name, as 5B did
// for `fmod`.

pub(super) const IMP_PRINT_STR: &str = "lpp_print_str";
pub(super) const IMP_PANIC_DIV_ZERO: &str = "lpp_panic_div_zero";
pub(super) const IMP_FMOD: &str = "fmod";
pub(super) const IMP_STR_EQ: &str = "lpp_str_eq";
pub(super) const IMP_STR_CONCAT: &str = "lpp_str_concat";
pub(super) const IMP_EMPTY_STR: &str = "lpp_empty_str";
pub(super) const IMP_INT_TO_STR: &str = "lpp_int_to_str";
pub(super) const IMP_FLOAT_TO_STR: &str = "lpp_float_to_str";
pub(super) const IMP_BOOL_TO_STR: &str = "lpp_bool_to_str";
pub(super) const IMP_ARC_ALLOC: &str = "lpp_arc_alloc_with_destructor";
pub(super) const IMP_ARC_RETAIN: &str = "lpp_arc_retain";
pub(super) const IMP_ARC_RELEASE: &str = "lpp_arc_release";
pub(super) const IMP_LIST_NEW: &str = "lpp_list_new";
pub(super) const IMP_LIST_NEW_ARC: &str = "lpp_list_new_arc";
pub(super) const IMP_LIST_PUSH: &str = "lpp_list_push";
pub(super) const IMP_LIST_PUSH_ARC: &str = "lpp_list_push_arc";
pub(super) const IMP_LIST_PUSH_FLOAT: &str = "lpp_list_push_float";
pub(super) const IMP_LIST_PUSH_BOOL: &str = "lpp_list_push_bool";
pub(super) const IMP_LIST_GET: &str = "lpp_list_get";
pub(super) const IMP_LIST_GET_ARC: &str = "lpp_list_get_arc";
pub(super) const IMP_LIST_GET_FLOAT: &str = "lpp_list_get_float";
pub(super) const IMP_LIST_GET_BOOL: &str = "lpp_list_get_bool";
pub(super) const IMP_LIST_SET: &str = "lpp_list_set";
pub(super) const IMP_LIST_SET_ARC: &str = "lpp_list_set_arc";
pub(super) const IMP_LIST_SET_FLOAT: &str = "lpp_list_set_float";
pub(super) const IMP_LIST_SET_BOOL: &str = "lpp_list_set_bool";
pub(super) const IMP_LIST_LEN: &str = "lpp_list_len";
pub(super) const IMP_CLOSURE_DESTROY: &str = "lpp_closure_destroy";
// Cycle-collector ABI (see `lpp_runtime::collect`): mark a `Shared`
// allocation collectible with a trace function, report children during
// trial deletion, and run the final collection from `main`.
pub(super) const IMP_MARK_COLLECTIBLE: &str = "lpp_arc_mark_collectible";
pub(super) const IMP_TRACE_VISIT: &str = "lpp_trace_visit";
pub(super) const IMP_COLLECT_CYCLES: &str = "lpp_arc_collect_cycles";
pub(super) const IMP_LIST_TRACE: &str = "lpp_list_trace";
pub(super) const IMP_TUPLE_TRACE: &str = "lpp_tuple_trace";
pub(super) const IMP_TASK_TRACE: &str = "lpp_task_trace";
pub(super) const IMP_CLOSURE_TRACE: &str = "lpp_closure_trace";
pub(super) const IMP_TUPLE_ALLOC: &str = "lpp_tuple_alloc";
pub(super) const IMP_TASK_NEW: &str = "lpp_task_new";
pub(super) const IMP_TASK_POLL: &str = "lpp_task_poll";
pub(super) const IMP_TASK_AWAIT: &str = "lpp_task_await";
pub(super) const IMP_TASK_DESTROY: &str = "lpp_task_destroy";
pub(super) const IMP_PGO_HIT: &str = "lpp_pgo_hit";
pub(super) const IMP_PGO_FLUSH: &str = "lpp_pgo_flush";
pub(super) const IMP_PRINT_INT: &str = "lpp_print_int";
pub(super) const IMP_PRINT_FLOAT: &str = "lpp_print_float";
pub(super) const IMP_PRINT_BOOL: &str = "lpp_print_bool";
pub(super) const IMP_VEC_CHECKSUM: &str = "lpp_vec_i64_checksum";

/// The machine signature of one runtime import.
/// The oracle-supported builtin names (Family A, 71 at this commit):
/// the `eval_builtin` arms of the ARC oracle. The gate recomputes the
/// census from the registry; any registry drift fails the gate.
pub(super) const FAMILY_A: &[&str] = &[
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
    "list_pop",
    "list_free",
    "buf_alloc",
    "buf_free",
    "buf_free_at_exit",
    "buf_len",
    "buf_get8",
    "buf_set8",
    "buf_set32le",
    "buf_get32le",
    "buf_set16le",
    "buf_get16le",
    "buf_set16be",
    "buf_get16be",
    "buf_set32be",
    "buf_get32be",
    "buf_set64le",
    "buf_get64le",
    "buf_set64be",
    "buf_get64be",
    "buf_copy",
    "buf_read",
    "buf_write",
    "buf_crc32",
    "buf_to_str",
    "buf_read_str",
    "buf_write_str",
    "command_exec",
    "env_get",
    "env_set",
    "dir_list",
    "path_join",
    "sleep",
    "sleep_ms",
    "exit",
    "write_int",
    "write_uint",
    "write_float",
    "write_bool",
    "write_char",
    "write_nl",
    "ewrite_str",
    "flush",
    "tan",
    "int_to_float",
    "float_to_int",
    "add_overflows",
    "mul_overflows",
    "str_cmp",
    "lpp_str_cmp",
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
    "get",
    "json_parse",
    "json_get_int",
    "json_get_str",
    "json_get_obj",
    "json_free",
    "json_escape",
    "file_open",
    "file_close",
    "file_pread",
    "file_pwrite",
    "file_fsync",
    "file_fdatasync",
    "file_ftruncate",
    "fd_size",
    "sys_mem_total",
    "sys_mem_free",
    "sys_cpu_usage",
    "sys_uptime",
    "lpp_c_malloc",
    "lpp_c_free",
    "lpp_c_load_u8",
    "lpp_c_store_u8",
    "lpp_c_load_i32",
    "lpp_c_store_i32",
    "lpp_c_load_i64",
    "lpp_c_store_i64",
    "dlopen",
    "dlsym",
    "dlclose",
    "dlerror",
    "args_count",
    "args_get",
    "args",
    "cwd",
    "chdir",
    "is_dir",
    "is_file",
    "time_ns",
    "str_join",
    "str_lines",
    "str_pad_left",
    "str_pad_right",
];

pub(super) fn is_family_a(name: &str) -> bool {
    FAMILY_A.contains(&name) || FAMILY_A.contains(&name.strip_prefix("lpp_").unwrap_or(name))
}

/// The slice-handle builtin names (Family C, 7 at this commit).
pub(super) const FAMILY_C: &[&str] = &[
    "slice",
    "str_slice",
    "slice_len",
    "slice_get",
    "lpp_slice_get_bool",
    "slice_to_str",
    "str_slice_to_str",
];

pub(super) fn import_signature(name: &str) -> (&'static [CLType], Option<CLType>) {
    match name {
        IMP_PGO_HIT | IMP_PGO_FLUSH => (&[cltypes::I64], None),
        IMP_PRINT_STR => (&[cltypes::I64], None),
        IMP_FMOD => (&[cltypes::F64, cltypes::F64], Some(cltypes::F64)),
        IMP_ARC_ALLOC => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        IMP_ARC_RETAIN | IMP_ARC_RELEASE => (&[cltypes::I64], None),
        IMP_MARK_COLLECTIBLE | IMP_TRACE_VISIT | IMP_LIST_TRACE | IMP_TUPLE_TRACE
        | IMP_TASK_TRACE | IMP_CLOSURE_TRACE => (&[cltypes::I64, cltypes::I64], None),
        IMP_COLLECT_CYCLES => (&[], None),
        IMP_PANIC_DIV_ZERO => (&[], None),
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
        "lpp_list_pop" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_list_free" => (&[cltypes::I64], None),
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
        "lpp_write_int" | "lpp_write_uint" | "lpp_write_char" | "lpp_ewrite_str" => {
            (&[cltypes::I64], None)
        }
        "lpp_write_float" => (&[cltypes::F64], None),
        "lpp_write_bool" => (&[cltypes::I8], None),
        "lpp_write_nl" | "lpp_flush" => (&[], None),
        "lpp_str_concat" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_empty_str" => (&[], Some(cltypes::I64)),
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
        "lpp_str_eq" | "lpp_str_cmp" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
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
        "lpp_command_output" | "lpp_command_exec" | "lpp_env_get" | "lpp_dir_list" => {
            (&[cltypes::I64], Some(cltypes::I64))
        }
        "lpp_env_set" | "lpp_path_join" => (&[cltypes::I64, cltypes::I64], Some(cltypes::I64)),
        "lpp_sleep_ms" | "lpp_exit" => (&[cltypes::I64], None),
        // Buffer runtime (Family A).
        "lpp_buf_alloc" | "lpp_buf_len" | "lpp_buf_read" | "lpp_buf_to_str" => {
            (&[cltypes::I64], Some(cltypes::I64))
        }
        "lpp_buf_free" | "lpp_buf_free_at_exit" => (&[cltypes::I64], None),
        "lpp_buf_get8" | "lpp_buf_get16le" | "lpp_buf_get16be" | "lpp_buf_get32le"
        | "lpp_buf_get32be" | "lpp_buf_get64le" | "lpp_buf_get64be" | "lpp_buf_write" => {
            (&[cltypes::I64, cltypes::I64], Some(cltypes::I64))
        }
        "lpp_buf_set8" | "lpp_buf_set16le" | "lpp_buf_set16be" | "lpp_buf_set32le"
        | "lpp_buf_set32be" | "lpp_buf_set64le" | "lpp_buf_set64be" => {
            (&[cltypes::I64, cltypes::I64, cltypes::I64], None)
        }
        "lpp_buf_crc32" | "lpp_buf_read_str" | "lpp_buf_write_str" => (
            &[cltypes::I64, cltypes::I64, cltypes::I64],
            Some(cltypes::I64),
        ),
        "lpp_buf_copy" => (&[cltypes::I64; 5], None),
        // JSON, positional file I/O, CLI args, system info, C pointers, dynamic loading.
        "lpp_json_parse" | "lpp_json_escape" | "lpp_str_lines" => {
            (&[cltypes::I64], Some(cltypes::I64))
        }
        "lpp_json_get_int" | "lpp_json_get_str" | "lpp_json_get_obj" | "lpp_str_join" => {
            (&[cltypes::I64, cltypes::I64], Some(cltypes::I64))
        }
        "lpp_json_free" => (&[cltypes::I64], None),
        "lpp_str_pad_left" | "lpp_str_pad_right" => (
            &[cltypes::I64, cltypes::I64, cltypes::I64],
            Some(cltypes::I64),
        ),
        "lpp_file_open" | "lpp_file_ftruncate" => {
            (&[cltypes::I64, cltypes::I64], Some(cltypes::I64))
        }
        "lpp_file_close" => (&[cltypes::I64], None),
        "lpp_file_pread" | "lpp_file_pwrite" => (&[cltypes::I64; 5], Some(cltypes::I64)),
        "lpp_file_fsync" | "lpp_file_fdatasync" | "lpp_fd_size" | "lpp_args_get" | "lpp_chdir"
        | "lpp_is_dir" | "lpp_is_file" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_args_count" | "lpp_args" | "lpp_cwd" | "lpp_time_ns" | "lpp_sys_mem_total"
        | "lpp_sys_mem_free" | "lpp_sys_cpu_usage" | "lpp_sys_uptime" | "dlerror" => {
            (&[], Some(cltypes::I64))
        }
        "lpp_c_malloc" | "dlclose" => (&[cltypes::I64], Some(cltypes::I64)),
        "lpp_c_free" => (&[cltypes::I64], None),
        "lpp_c_load_u8" | "lpp_c_load_i32" | "lpp_c_load_i64" | "dlopen" | "dlsym" => {
            (&[cltypes::I64, cltypes::I64], Some(cltypes::I64))
        }
        "lpp_c_store_u8" | "lpp_c_store_i32" | "lpp_c_store_i64" => {
            (&[cltypes::I64, cltypes::I64, cltypes::I64], None)
        }
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
        "lpp_add_overflows" | "lpp_mul_overflows" => {
            (&[cltypes::I64, cltypes::I64], Some(cltypes::I64))
        }
        "lpp_floor" => (&[cltypes::F64], Some(cltypes::F64)),
        "lpp_ceil" => (&[cltypes::F64], Some(cltypes::F64)),
        "lpp_pow" => (&[cltypes::F64, cltypes::F64], Some(cltypes::F64)),
        "lpp_sqrt" => (&[cltypes::F64], Some(cltypes::F64)),
        "lpp_sin" => (&[cltypes::F64], Some(cltypes::F64)),
        "lpp_cos" => (&[cltypes::F64], Some(cltypes::F64)),
        "lpp_tan" => (&[cltypes::F64], Some(cltypes::F64)),
        "lpp_int_to_float" => (&[cltypes::I64], Some(cltypes::F64)),
        "lpp_float_to_int" => (&[cltypes::F64], Some(cltypes::I64)),
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
pub(super) fn is_family_map(name: &str) -> bool {
    name.starts_with("map_") || name.starts_with("lpp_map_")
}

/// Whether a map builtin returns a boolean membership flag. The frozen ABI
/// spells the result as `i64`, but the checker types it as `Bool`, so codegen
/// must reduce the runtime's `i64` 0/1 to a machine `i8`.
pub(super) fn is_map_predicate(name: &str) -> bool {
    matches!(
        name,
        "map_has" | "map_has_str" | "lpp_map_has" | "lpp_map_has_str"
    )
}

/// Family NET: frozen v1 socket/HTTP builtins that are representable on native
/// targets through the Rust runtime. Both bare and `lpp_` spellings lower to
/// the same `lpp_*` symbol recorded in the ABI descriptor.
pub(super) fn is_family_net(name: &str) -> bool {
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
