// Runtime Symbol Parity Gate — ensures all required builtins exist across all runtimes
use std::fs;
use std::path::Path;

fn load_complete_runtime(root: &Path, filename: &str) -> String {
    let content = fs::read_to_string(root.join(filename)).unwrap_or_default();
    let mut expanded = String::new();
    for line in content.lines() {
        if line.trim_start().starts_with("#include \"runtime/") {
            let rel = line.trim().trim_start_matches("#include \"").trim_end_matches('"');
            if let Ok(sub) = fs::read_to_string(root.join(rel)) {
                expanded.push_str(&sub);
                expanded.push('\n');
                continue;
            }
        }
        expanded.push_str(line);
        expanded.push('\n');
    }
    expanded
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
            missing.push(format!("standard runtime (lpp_runtime.c) missing {}", sym));
        }
        if !win_min.contains(sym) {
            missing.push(format!("windows freestanding runtime (windows_x86_64_min.c) missing {}", sym));
        }
        if !linux_min.contains(sym) {
            missing.push(format!("linux freestanding runtime (linux_x86_64_min.c) missing {}", sym));
        }
    }

    assert!(
        missing.is_empty(),
        "Runtime parity gate failed:\n{}",
        missing.join("\n")
    );
}

#[test]
fn test_pe_kernel32_linker_symbols() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let linker_src = fs::read_to_string(root.join("src/linker.rs")).expect("src/linker.rs missing");

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
            missing.push(format!("src/linker.rs missing Kernel32 symbol {}", api));
        }
    }

    assert!(
        missing.is_empty(),
        "Linker Kernel32 symbol table missing entries:\n{}",
        missing.join("\n")
    );
}
