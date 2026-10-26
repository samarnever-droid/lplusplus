// Runtime Symbol Parity Gate — ensures required builtins exist across runtimes.
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn expand_runtime_includes(root: &Path, source: &Path, visited: &mut BTreeSet<PathBuf>) -> String {
    let path = if source.is_absolute() {
        source.to_path_buf()
    } else {
        root.join(source)
    };
    let canonical = path.canonicalize().unwrap_or(path.clone());
    if !visited.insert(canonical.clone()) {
        return String::new();
    }
    let content = fs::read_to_string(&canonical).unwrap_or_default();
    let mut expanded = String::new();
    for line in content.lines() {
        let trimmed = line.trim_start();
        let Some(include) = trimmed.strip_prefix("#include \"") else {
            expanded.push_str(line);
            expanded.push('\n');
            continue;
        };
        let Some((include, _)) = include.split_once('"') else {
            expanded.push_str(line);
            expanded.push('\n');
            continue;
        };
        let include = Path::new(include);
        let candidates = [
            root.join(include),
            canonical.parent().unwrap_or(root).join(include),
        ];
        if let Some(path) = candidates.into_iter().find(|candidate| candidate.is_file()) {
            expanded.push_str(&expand_runtime_includes(root, &path, visited));
            expanded.push('\n');
        } else {
            expanded.push_str(line);
            expanded.push('\n');
        }
    }
    expanded
}

fn load_complete_runtime(root: &Path, filename: &str) -> String {
    expand_runtime_includes(root, Path::new(filename), &mut BTreeSet::new())
}

#[test]
fn test_freestanding_runtime_symbol_parity() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let standard_rt = load_complete_runtime(root, "lpp_runtime.c");
    let win_min = load_complete_runtime(root, "runtime/windows_x86_64_min.c");
    let linux_min = load_complete_runtime(root, "runtime/linux_x86_64_min.c");

    // Critical symbol list emitted by Cranelift AOT / MIR lowering
    let core_symbols = [
        "lpp_print_int",
        "lpp_print_float",
        "lpp_print_bool",
        "lpp_print_str",
        "lpp_arc_alloc",
        "lpp_arc_retain",
        "lpp_arc_release",
        // Cycle-collector ABI (ownership-plan Shared types): every
        // runtime must provide marking, collection, the trace-visit
        // sink, and traces for the runtime-owned aggregate shapes.
        "lpp_arc_mark_collectible",
        "lpp_arc_collect_cycles",
        "lpp_trace_visit",
        "lpp_list_trace",
        "lpp_tuple_trace",
        "lpp_task_trace",
        "lpp_closure_trace",
        "lpp_list_new",
        "lpp_list_new_arc",
        "lpp_list_push",
        "lpp_list_push_arc",
        "lpp_list_push_float",
        "lpp_list_push_bool",
        "lpp_list_get",
        "lpp_list_get_arc",
        "lpp_list_get_float",
        "lpp_list_get_bool",
        "lpp_list_set",
        "lpp_list_set_arc",
        "lpp_list_set_float",
        "lpp_list_set_bool",
        "lpp_list_pop",
        "lpp_list_len",
        "lpp_list_free",
        "lpp_buf_free_at_exit",
        "lpp_str_concat",
        "lpp_str_eq",
        "lpp_str_cmp",
        "lpp_slice_init",
        "lpp_slice_len",
        "lpp_slice_get",
    ];

    let mut missing = Vec::new();

    for sym in core_symbols {
        if !standard_rt.contains(sym) {
            missing.push(format!("standard runtime (lpp_runtime.c) missing {sym}"));
        }
        if !win_min.contains(sym) {
            missing.push(format!(
                "windows freestanding runtime (windows_x86_64_min.c) missing {sym}"
            ));
        }
        if !linux_min.contains(sym) {
            missing.push(format!(
                "linux freestanding runtime (linux_x86_64_min.c) missing {sym}"
            ));
        }
    }

    assert!(
        missing.is_empty(),
        "Runtime parity gate failed:\n{}",
        missing.join("\n")
    );
    for pgo_symbol in ["lpp_pgo_hit", "lpp_pgo_flush"] {
        assert!(
            standard_rt.contains(pgo_symbol),
            "standard runtime is missing PGO helper {pgo_symbol}"
        );
    }
}

#[test]
fn test_pe_kernel32_linker_symbols() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    // The Kernel32 import table is defined in the PE support module, separate
    // from the shared cross-format object/link model.
    let linker_src = fs::read_to_string(root.join("crates/lpp-linker/src/core/pe_support.rs"))
        .expect("crates/lpp-linker/src/core/pe_support.rs missing");

    let required_k32_apis = [
        "ExitProcess",
        "GetStdHandle",
        "WriteFile",
        "ReadFile",
        "CreateFileA",
        "GetFileSize",
        "GetFileSizeEx",
        "SetFilePointer",
        "SetFilePointerEx",
        "SetEndOfFile",
        "VirtualAlloc",
        "VirtualFree",
        "CloseHandle",
    ];

    let mut missing = Vec::new();
    for api in required_k32_apis {
        if !linker_src.contains(api) {
            missing.push(format!("lpp-linker missing Kernel32 symbol {api}"));
        }
    }

    assert!(
        missing.is_empty(),
        "Linker Kernel32 symbol table missing entries:\n{}",
        missing.join("\n")
    );
}
