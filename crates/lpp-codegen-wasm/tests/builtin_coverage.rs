//! Every builtin a program resolves to must be lowerable by the wasm backend.
//!
//! This is the check that would have caught the `len(list)` bug. `len` is one
//! *source* spelling for three runtime symbols (`str_len`, `slice_len`,
//! `list_len`); `abi/builtins.toml` maps it to `lpp_list_len`, and the wasm
//! backend only lowers the list helper. So `len(list)` type-checked, resolved to
//! the generic `len` builtin, and was rejected at codegen with
//! `E5003: UnrepresentableBuiltin` — even though `tests/test_empty_list_inference.lpp`
//! in this very repository uses exactly that spelling.
//!
//! Per-spelling specialisations (`_str` map keys, `_float` values) are resolved
//! by the type checker (`specialize_polymorphic_builtin`), so the fact recorded
//! on each call expression is the *final* runtime symbol. Walking the corpus,
//! lowering every file that type-checks, and asserting that each recorded
//! builtin is one the backend implements turns "some file might use an
//! unsupported spelling" into a test failure with a file and a builtin name.
//!
//! It deliberately does not require every file to *compile*: many fixtures are
//! for features the wasm slice does not cover (`list_insert`, `map_keys`, the
//! SamarOS kernel). It requires that the failure is a known, named gap rather
//! than an accidental one.
//!
//! That is why there are two lists below. `WASM_LOWERED` is what the backend
//! implements; `KNOWN_GAPS` is what the corpus uses but the backend does not
//! implement yet. A builtin in neither list fails the test. Without the second
//! list the test is unusable — the corpus exercises 174 builtins outside the
//! backend's reach, and widening `WASM_LOWERED` to hold them would turn a real
//! audit into a rubber stamp.
//!
//! Symbols are compared *after* stripping the `lpp_` prefix, because that is
//! exactly what the backend does before dispatching (`lib.rs`: `let short =
//! name.strip_prefix("lpp_").unwrap_or(name)`). The type checker resolves to
//! `lpp_abs` while the backend lowers `abs`; comparing raw names would report
//! seven phantom gaps.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    lower_package,
};
use lpp_types::{ShadowInferenceOptions, infer_hir_package};

/// Builtins the wasm backend lowers, as `lpp_`-stripped names (not ids) so a
/// reader can diff this list against `crates/lpp-codegen-wasm/src/lib.rs`'s
/// `match short`.
const WASM_LOWERED: &[&str] = &[
    // printing + strings
    "print",
    "print_str",
    "print_int",
    "print_float",
    "print_bool",
    "write_str",
    "str_len",
    "str_eq",
    "str_concat",
    "str_contains",
    "str_starts_with",
    "str_ends_with",
    "str_find",
    "str_replace",
    "str_substr",
    "str_repeat",
    "str_split",
    "str_trim",
    "str_lower",
    "str_upper",
    "str_to_int",
    "str_to_u64",
    "char_at",
    "ord",
    "chr",
    "int_to_str",
    "float_to_str",
    "bool_to_str",
    "u64_to_str",
    "u64_to_hex",
    // lists
    "len",
    "list_len",
    "list_new",
    "list_push",
    "list_get",
    "list_set",
    // maps
    "map_new",
    "map_put",
    "map_put_str",
    "map_put_float",
    "map_put_str_float",
    "map_get",
    "map_get_str",
    "map_get_float",
    "map_get_str_float",
    "map_has",
    "map_has_str",
    "map_remove",
    "map_remove_str",
    "map_len",
    // slices
    "slice",
    "str_slice",
    "slice_len",
    "slice_get",
    "slice_get_bool",
    // The element-typed spellings the checker picks when a view's element is a
    // `Str`/`Bool` (HANDOFF.md §5.10); codegen routes both through its
    // `slice_get` arm.
    "slice_get_str",
    // numerics
    "abs",
    "min",
    "max",
    "int_pow",
    "ceil",
    "floor",
    "sqrt",
    "sin",
    "cos",
    "pow",
    // io
    //
    // `input` is a source spelling the backend dispatches directly (it lowers
    // to the same read as `input_line`); the checker does not rewrite it.
    "input",
    "input_line",
    "input_eof",
    // filesystem
    "append_file",
    "delete_file",
    "file_exists",
    "read_file",
    "write_file",
];

/// Builtins the corpus resolves to that the wasm backend does **not** lower.
///
/// Every entry here is a real, deliberate gap: a call would fail at codegen
/// with `E5003: UnrepresentableBuiltin`. The point of the list is that the gap
/// is named and counted rather than silent. Adding an entry is a decision;
/// discovering an unlisted one is a bug.
///
/// This list is measured, not guessed: it is exactly what the type-checking
/// corpus resolves to minus `WASM_LOWERED`. If the backend grows one of these,
/// the test does not fail (it only checks the reverse direction), so prune as
/// they land.
const KNOWN_GAPS: &[&str] = &[
    // checked/wrapping integer arithmetic; `div_u`/`rem_u`/`shr_u` and the unsigned comparisons are interpreter- and cranelift-only today
    "add_checked",
    "add_wrap",
    "div_u",
    "ge_u",
    "gt_u",
    "le_u",
    "lt_u",
    "max_u",
    "min_u",
    "mul_checked",
    "rem_u",
    "shr_u",
    "sub_checked",
    // narrowing conversions
    "trunc_i16",
    "trunc_i8",
    "trunc_u16",
    "trunc_u32",
    "trunc_u8",
    // bit intrinsics and simd
    "bswap16",
    "bswap32",
    "bswap64",
    "clz64",
    "ctz64",
    "popcount64",
    "prefetch",
    "prefetch_write",
    "rotl32",
    "rotl64",
    "rotr32",
    "rotr64",
    "vec_i64_checksum",
    "vec_i64x2",
    "vec_i64x2_add",
    "vec_i64x2_and",
    "vec_i64x2_extract",
    "vec_i64x2_mul",
    "vec_i64x2_or",
    "vec_i64x2_shr",
    "vec_i64x2_splat",
    "vec_i64x2_sub",
    "vec_i64x2_sum",
    "vec_i64x2_xor",
    "vec_u8x16_eq",
    "vec_u8x16_movemask",
    "vec_u8x16_splat",
    // atomics
    "atomic_add",
    "atomic_add32",
    "atomic_and",
    "atomic_cas",
    "atomic_fence",
    "atomic_fence_acq",
    "atomic_fence_rel",
    "atomic_free",
    "atomic_load",
    "atomic_load32",
    "atomic_new",
    "atomic_or",
    "atomic_store",
    "atomic_store32",
    "atomic_sub",
    "atomic_xor",
    // buffers (the `buf_*` family)
    "buf_alloc",
    "buf_copy",
    "buf_crc32",
    "buf_free",
    "buf_free_at_exit",
    "buf_get16le",
    "buf_get32le",
    "buf_get8",
    "buf_len",
    "buf_read",
    "buf_read_str",
    "buf_set16le",
    "buf_set32le",
    "buf_set8",
    "buf_write",
    "buf_write_str",
    // list methods beyond push/get/set/len
    "list_binary_search",
    "list_capacity",
    "list_clear",
    "list_extend",
    "list_free",
    "list_index_of",
    "list_insert",
    "list_remove",
    "list_reserve",
    "list_reverse",
    "list_sort",
    "list_sort_desc",
    "list_sort_u",
    "list_swap",
    "list_truncate",
    // map methods beyond put/get/has/remove/len
    "map_capacity",
    "map_clear",
    "map_keys",
    "map_values",
    // string conversions
    "fmod",
    "int_to_float",
    "parse_int",
    "slice_to_str",
    "str_slice_to_str",
    // clock, rng and time
    "clock_advance",
    "clock_free",
    "clock_new",
    "clock_now",
    "random_range",
    "rng_float",
    "rng_free",
    "rng_new",
    "rng_next",
    "rng_range",
    "time_ms",
    // filesystem
    "dir_create",
    "dir_remove",
    "file_copy",
    "file_move",
    "file_size",
    "path_exists",
    // process, environment, threads
    "command_exec",
    "command_output",
    "cpu_count",
    "cpu_pause",
    "env_get",
    "env_set",
    "exit",
    "thread_id",
    // networking
    "net_accept",
    "net_close",
    "net_connect",
    "net_dial",
    "net_listen",
    "net_recv",
    "net_resolve",
    "net_send",
    "net_send_all",
    // synchronisation
    "mutex_free",
    "mutex_lock",
    "mutex_new",
    "mutex_trylock",
    "mutex_unlock",
    "rwlock_free",
    "rwlock_new",
    "rwlock_rdlock",
    "rwlock_rdunlock",
    "rwlock_wrlock",
    "rwlock_wrunlock",
    // gui and webview (inherently host-side)
    "gui_clear",
    "gui_dialog_message",
    "gui_draw_circle",
    "gui_draw_line",
    "gui_draw_rect",
    "gui_draw_rounded_rect",
    "gui_draw_text",
    "gui_get_ticks_ms",
    "gui_key_down",
    "gui_measure_text_width",
    "gui_mouse_down",
    "gui_mouse_x",
    "gui_mouse_y",
    "gui_present",
    "gui_window_close",
    "gui_window_create",
    "gui_window_height",
    "gui_window_is_open",
    "gui_window_poll_events",
    "gui_window_width",
    "webview_destroy",
    "webview_navigate",
    "webview_run",
    "webview_window_create",
];

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the codesitter crate lives under <root>/crates")
        .to_path_buf()
}

fn collect(directory: &Path, output: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries {
        let entry = entry.expect("a readable directory entry");
        let path = entry.path();
        if path.is_dir() {
            if path
                .file_name()
                .is_some_and(|name| name == "target" || name == ".git")
            {
                continue;
            }
            collect(&path, output);
        } else if path.extension().is_some_and(|extension| extension == "lpp") {
            output.push(path);
        }
    }
}

/// Minimal filesystem over one in-memory file: the corpus files are lowered one
/// at a time and must not drag their imports in.
struct OneFile {
    path: PathBuf,
    text: String,
}

impl FileSystem for OneFile {
    fn is_file(&self, path: &Path) -> Result<bool, FileSystemError> {
        Ok(path == self.path)
    }
    fn canonicalize(&self, path: &Path) -> Result<PathBuf, FileSystemError> {
        // The graph builder canonicalizes the package *root* before the entry
        // file, so the root has to resolve too.
        if path == self.path || self.path.starts_with(path) {
            Ok(path.to_owned())
        } else {
            Err(FileSystemError::new("canonicalize", path, "not found"))
        }
    }
    fn read_to_string(&self, _path: &Path) -> Result<String, FileSystemError> {
        Ok(self.text.clone())
    }
}

#[test]
fn every_builtin_used_in_the_corpus_is_lowered_by_the_wasm_backend() {
    let root = repository_root();
    let mut files = Vec::new();
    for directory in [
        "tests",
        "test",
        "stdlib",
        "examples",
        "benchmarks",
        "packages",
        "SamarOS",
    ] {
        collect(&root.join(directory), &mut files);
    }
    files.sort();
    assert!(files.len() > 100, "collected only {} files", files.len());

    let lowered_builtins = WASM_LOWERED.iter().copied().collect::<BTreeSet<_>>();
    let known_gaps = KNOWN_GAPS.iter().copied().collect::<BTreeSet<_>>();
    // The backend dispatches on the `lpp_`-stripped name; compare the same way.
    assert!(
        WASM_LOWERED.iter().all(|name| !name.starts_with("lpp_")),
        "`WASM_LOWERED` holds stripped names; `KNOWN_GAPS` too"
    );
    let mut per_builtin: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut checked = 0usize;

    for path in files {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        if text.lines().any(|line| {
            let line = line.trim_start();
            line.starts_with("import ") || line.starts_with("from ")
        }) {
            continue;
        }
        let relative = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .display()
            .to_string();
        let fs_one = OneFile {
            path: path.clone(),
            text,
        };
        let Ok(graph) = GraphBuilder::new(&fs_one).build(GraphRequest::new(
            &path,
            PackageSpec::new("builtin-audit", path.parent().unwrap_or(&root)),
        )) else {
            continue;
        };
        let Ok(package) = lower_package(&graph, ResolutionMode::Namespaced) else {
            continue;
        };
        let Ok(types) = infer_hir_package(&package, ShadowInferenceOptions::default()) else {
            // A file that does not type-check cannot reach codegen at all.
            continue;
        };
        checked += 1;
        for (_expression, fact) in types.builtins.expressions() {
            let name = fact.builtin.descriptor().name;
            let short = name.strip_prefix("lpp_").unwrap_or(name);
            per_builtin
                .entry(short.to_owned())
                .or_default()
                .insert(relative.clone());
        }
    }

    assert!(
        checked > 50,
        "only {checked} files type-checked; the corpus filter or the pipeline changed"
    );

    let mut offenders: BTreeMap<&str, &BTreeSet<String>> = BTreeMap::new();
    for (name, files) in &per_builtin {
        if !lowered_builtins.contains(name.as_str()) && !known_gaps.contains(name.as_str()) {
            offenders.insert(name.as_str(), files);
        }
    }
    assert!(
        offenders.is_empty(),
        "{} type-checking files use builtins that are neither lowered by the \
         wasm backend nor listed in `KNOWN_GAPS` (each fails with E5003):\n{:#?}\n\n\
         If one of these is a generic spelling, the type checker must specialise \
         it in `specialize_polymorphic_builtin` (that is how `len(list)` was \
         fixed); if it is a real, accepted gap, add it to `KNOWN_GAPS` in the \
         same commit that documents it.",
        checked,
        offenders
    );

    // The gap list must stay honest in the other direction too: an entry that
    // the corpus no longer uses, or that the backend now lowers, is dead
    // documentation.
    let stale: Vec<&str> = KNOWN_GAPS
        .iter()
        .copied()
        .filter(|name| !per_builtin.contains_key(*name) && !lowered_builtins.contains(name))
        .collect();
    assert!(
        stale.is_empty(),
        "`KNOWN_GAPS` entries the corpus no longer resolves to: {stale:#?}\n\
         Either the fixture changed (drop the entry) or the builtin landed in \
         the backend (drop the entry and add it to `WASM_LOWERED`)."
    );

    // `WASM_LOWERED` overlapping `KNOWN_GAPS` is a contradiction.
    let overlap: Vec<&str> = lowered_builtins
        .intersection(&known_gaps)
        .copied()
        .collect();
    assert!(
        overlap.is_empty(),
        "listed as both lowered and a known gap: {overlap:#?}"
    );
}
