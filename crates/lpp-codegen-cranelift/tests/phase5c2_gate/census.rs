use super::*;

const IMPORT_UNIVERSE: &[&str] = &[
    "fmod",
    "lpp_abs",
    "lpp_add_checked",
    "lpp_add_wrap",
    "lpp_arc_alloc_with_destructor",
    "lpp_arc_collect_cycles",
    "lpp_arc_mark_collectible",
    "lpp_arc_release",
    "lpp_arc_retain",
    "lpp_closure_trace",
    "lpp_list_trace",
    "lpp_task_trace",
    "lpp_trace_visit",
    "lpp_tuple_trace",
    "lpp_bool_to_str",
    "lpp_bswap16",
    "lpp_bswap32",
    "lpp_bswap64",
    "lpp_ceil",
    "lpp_closure_destroy",
    "lpp_clz64",
    "lpp_ctz64",
    "lpp_div_u",
    "lpp_eprint_str",
    "lpp_float_to_str",
    "lpp_floor",
    "lpp_ge_u",
    "lpp_gt_u",
    "lpp_int_to_str",
    "lpp_le_u",
    "lpp_list_get",
    "lpp_list_get_arc",
    "lpp_list_get_bool",
    "lpp_list_get_float",
    "lpp_list_len",
    "lpp_list_new",
    "lpp_list_new_arc",
    "lpp_list_push",
    "lpp_list_push_arc",
    "lpp_list_push_bool",
    "lpp_list_push_float",
    "lpp_list_set",
    "lpp_list_set_arc",
    "lpp_list_set_bool",
    "lpp_list_set_float",
    "lpp_lt_u",
    "lpp_max",
    "lpp_max_u",
    "lpp_min",
    "lpp_min_u",
    "lpp_mul_checked",
    "lpp_mul_wrap",
    "lpp_popcount64",
    "lpp_pow",
    "lpp_print_bool",
    "lpp_print_float",
    "lpp_print_int",
    "lpp_print_str",
    "lpp_rem_u",
    "lpp_rotl32",
    "lpp_rotl64",
    "lpp_rotr32",
    "lpp_rotr64",
    "lpp_shl_u",
    "lpp_shr_u",
    "lpp_slice_get",
    "lpp_slice_get_bool",
    "lpp_slice_init",
    "lpp_slice_len",
    "lpp_sqrt",
    "lpp_str_concat",
    "lpp_str_contains",
    "lpp_str_ends_with",
    "lpp_str_find",
    "lpp_str_len",
    "lpp_str_lower",
    "lpp_str_replace",
    "lpp_str_slice_to_str",
    "lpp_str_starts_with",
    "lpp_str_to_int",
    "lpp_str_to_u64",
    "lpp_str_trim",
    "lpp_str_upper",
    "lpp_sub_checked",
    "lpp_sub_wrap",
    "lpp_task_await",
    "lpp_task_destroy",
    "lpp_task_new",
    "lpp_task_poll",
    "lpp_trunc_i16",
    "lpp_trunc_i32",
    "lpp_trunc_i8",
    "lpp_trunc_u16",
    "lpp_trunc_u32",
    "lpp_trunc_u8",
    "lpp_tuple_alloc",
    "lpp_u64_to_hex",
    "lpp_u64_to_str",
    "lpp_vec_i64_checksum",
    "lpp_write_str",
];

// ── the four family name lists (the 5C2 table policy) ─────────────────────

const FAMILY_A_NAMES: &[&str] = &[
    "print",
    "print_str",
    "eprint_str",
    "print_int",
    "print_float",
    "print_bool",
    "write_str",
    "str_concat",
    "str_len",
    "str_contains",
    "str_starts_with",
    "str_ends_with",
    "str_find",
    "str_replace",
    "str_trim",
    "str_to_lower",
    "str_lower",
    "str_to_upper",
    "str_upper",
    "int_to_str",
    "str_to_int",
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
    "list_new",
    "list_push",
    "list_get",
    "list_set",
    "list_len",
];

const FAMILY_C_NAMES: &[&str] = &[
    "slice",
    "str_slice",
    "slice_len",
    "slice_get",
    "lpp_slice_get_bool",
    "slice_to_str",
    "str_slice_to_str",
];

fn is_family_a(name: &str) -> bool {
    FAMILY_A_NAMES.contains(&name)
        || (name.starts_with("lpp_") && FAMILY_A_NAMES.contains(&&name[4..]))
}

fn is_family_b(name: &str) -> bool {
    name.starts_with("vec_") || name == "lpp_vec_i64_checksum"
}

fn is_family_c(name: &str) -> bool {
    FAMILY_C_NAMES.contains(&name)
}

fn is_family_map(name: &str) -> bool {
    name.starts_with("map_") || name.starts_with("lpp_map_")
}

// ── gate 1: differential closures + tasks ─────────────────────────────────

#[test]
fn compiles_are_deterministic_and_the_census_matches_the_contract() {
    for (label, corpus) in [
        ("closure_task", CLOSURE_TASK_CORPUS),
        ("builtin", BUILTIN_CORPUS),
        ("simd_slice", SIMD_SLICE_CORPUS),
    ] {
        let (program, types, package) = pipeline(corpus);
        let names = Names(&package.names.symbols);
        let first = compile(&program, &types, &names);
        let second = compile(&program, &types, &names);
        assert_eq!(
            first.object, second.object,
            "{label}: objects differ byte-wise"
        );
        assert_eq!(
            first.exported_symbols, second.exported_symbols,
            "{label}: export censuses differ"
        );
        assert_eq!(
            first.imported_symbols, second.imported_symbols,
            "{label}: import censuses differ"
        );
    }

    let (program, types, package) = pipeline(CLOSURE_TASK_CORPUS);
    let names = Names(&package.names.symbols);
    let module = compile(&program, &types, &names);

    // Real host-format relocatable with the generated C-ABI entry.
    #[cfg(target_os = "windows")]
    assert_eq!(
        &module.object[..2],
        b"\x64\x86",
        "object is not x86-64 COFF"
    );
    #[cfg(target_os = "macos")]
    assert_eq!(
        &module.object[..4],
        b"\xcf\xfa\xed\xfe",
        "object is not 64-bit Mach-O"
    );
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    assert_eq!(&module.object[..4], b"\x7fELF", "object is not ELF");
    assert_eq!(module.entry.as_deref(), Some("main"));

    // Exports: `main` + `lpp_main`, every user function by source
    // name, one `lpp_c{n}` per closure function, one `lpp_drop_s{n}`
    // per struct, one `lpp_drop_c{n}` per capturing closure, and the
    // used thunks (`__lpp_closure_thunk{n}` for zero-parameter
    // closures, `__lpp_task_thunk{n}` for the async functions whose
    // task path is taken).
    let mut expected_exports = BTreeSet::from(["main".to_owned(), "lpp_main".to_owned()]);
    for (_, function) in program.functions() {
        if let Some(symbol) = function.name.as_ref()
            && let Some(name) = package.names.symbols.resolve(*symbol)
            && name != "main"
        {
            expected_exports.insert(name.to_owned());
        }
    }
    // The 4D Shared set: aggregates/closure-envs of these types also
    // get a generated cycle-collector trace beside the destructor.
    let shared: std::collections::BTreeSet<lpp_types::TypeId> =
        compute_ownership_plan(&program, &types)
            .expect("ownership plan")
            .pinned_types()
            .into_iter()
            .collect();
    for (id, aggregate) in program.aggregates() {
        let tag = match aggregate.kind {
            lpp_mir::MirAggregateKind::Struct => 's',
            lpp_mir::MirAggregateKind::Enum => 'e',
        };
        expected_exports.insert(format!("lpp_drop_{tag}{}", id.raw()));
        if shared.contains(&aggregate.ty) {
            expected_exports.insert(format!("lpp_trace_{tag}{}", id.raw()));
        }
    }

    let async_thunk_used: BTreeSet<MirFunctionId> =
        used_task_thunks(&program, main_function(&program, &package.names.symbols));

    for (id, function) in program.functions() {
        if !matches!(function.kind, MirFunctionKind::Closure) {
            continue;
        }
        expected_exports.insert(format!("lpp_c{}", id.raw()));
        let captures = program
            .function_parameters(function)
            .iter()
            .filter(|local| {
                matches!(
                    program.local(**local).expect("local").kind,
                    lpp_mir::MirLocalKind::Capture
                )
            })
            .count();
        let user_params = program
            .function_parameters(function)
            .iter()
            .filter(|local| {
                !matches!(
                    program.local(**local).expect("local").kind,
                    lpp_mir::MirLocalKind::Capture
                )
            })
            .count();
        if captures > 0 {
            expected_exports.insert(format!("lpp_drop_c{}", id.raw()));
            if shared.contains(&function.ty) {
                expected_exports.insert(format!("lpp_trace_c{}", id.raw()));
            }
        }
        if user_params == 0 {
            expected_exports.insert(format!("__lpp_closure_thunk{}", id.raw()));
        }
    }
    for &id in &async_thunk_used {
        expected_exports.insert(format!("__lpp_task_thunk{}", id.raw()));
    }

    assert_eq!(
        module.exported_symbols, expected_exports,
        "export census diverges from the contract"
    );

    // Imports: inside the closed runtime-symbol universe, and the
    // closure/task surface is actually present in this corpus.
    for symbol in module.imported_symbols.iter() {
        assert!(
            IMPORT_UNIVERSE.contains(&symbol.as_str()),
            "import {symbol} escapes the closed universe"
        );
    }
    for required in [
        "lpp_arc_alloc_with_destructor",
        "lpp_arc_release",
        "lpp_arc_retain",
        "lpp_closure_destroy",
        "lpp_tuple_alloc",
        "lpp_task_new",
        "lpp_task_poll",
        "lpp_task_await",
        "lpp_task_destroy",
        "lpp_print_str",
        "lpp_list_push",
        "lpp_list_push_arc",
        "lpp_list_get_arc",
        "lpp_list_len",
        "lpp_str_len",
    ] {
        assert!(
            module.imported_symbols.contains(required),
            "corpus uses {required}; the import is missing"
        );
    }
}

/// The async functions whose task path is taken: called as async
/// calls, used as function values, or the (async) entry — their
/// `__lpp_task_thunk{n}` is declared.
fn used_task_thunks(program: &MirProgram, entry: MirFunctionId) -> BTreeSet<MirFunctionId> {
    let mut used = BTreeSet::new();
    let entry_fn = program.function(entry).expect("entry exists");
    if matches!(entry_fn.kind, MirFunctionKind::Async) {
        used.insert(entry);
    }
    for (_, function) in program.functions() {
        for block_id in program.function_blocks(function) {
            let block = program.block(*block_id).expect("block retained");
            for &instruction_id in program.block_instructions(block) {
                let instruction = program
                    .instruction(instruction_id)
                    .expect("instruction retained");
                if let lpp_mir::InstructionKind::Assign { value, .. } = &instruction.kind {
                    match value {
                        lpp_mir::Rvalue::Call { callee, .. } => {
                            if let lpp_mir::Operand::Function(callee_id) = callee
                                && matches!(
                                    program.function(*callee_id).expect("callee retained").kind,
                                    MirFunctionKind::Async
                                )
                            {
                                used.insert(*callee_id);
                            }
                        }
                        lpp_mir::Rvalue::Use(operand) => {
                            if let lpp_mir::Operand::Function(function_id) = operand
                                && matches!(
                                    program
                                        .function(*function_id)
                                        .expect("function retained")
                                        .kind,
                                    MirFunctionKind::Async
                                )
                            {
                                used.insert(*function_id);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    used
}

// ── gate 5: table census + rejections ─────────────────────────────────────

fn expect_code(error: &CodegenError, code: &str, function: MirFunctionId) {
    assert_eq!(error.code(), code);
    assert_eq!(
        error.function,
        Some(function),
        "typed rejection must name the exact function"
    );
}

#[test]
fn builtin_table_census_and_family_d_rejection_are_exact() {
    // The registry enumerates to exactly 518 = Family A ∪ B ∪ C ∪ MAP ∪ D,
    // disjoint (by `BuiltinId`; a few names carry two ids).
    let builtins = lpp_runtime_abi::generated::BUILTINS;
    let mut counts = [0usize; 5];
    for builtin in builtins.iter() {
        let name = builtin.name;
        let family = if is_family_a(name) {
            0
        } else if is_family_b(name) {
            1
        } else if is_family_c(name) {
            2
        } else if is_family_map(name) {
            4
        } else {
            3
        };
        counts[family] += 1;
    }
    assert_eq!(
        builtins.len(),
        533,
        "the registry must enumerate 533 builtins"
    );
    assert_eq!(counts[0], 110, "family A ids (71 names, dual spellings)");
    assert_eq!(counts[1], 19, "family B ids (18 names + the checksum dual)");
    assert_eq!(counts[2], 7, "family C ids");
    // Family MAP: the hash map runtime, now lowered natively (was Family D).
    assert_eq!(counts[4], 30, "family MAP ids (hash map runtime)");
    assert_eq!(counts[3], 367, "remaining builtin ids");
    assert_eq!(
        counts.iter().sum::<usize>(),
        builtins.len(),
        "the families must partition the registry"
    );

    // A Family D builtin fails with the exact E5003 at the exact
    // function; tuples stay the only E5001 non-builtin rejection.
    let (program, types, package) = pipeline(
        "def main() -> Int:\n    w := webview_window_create(\"t\", 0, 0, 0)\n    return w\n",
    );
    let names = Names(&package.names.symbols);
    let options = CodegenOptions::new(Target::X86_64, &names);
    let error = CraneliftBackend
        .compile_module(&program, &types, &options)
        .err()
        .unwrap_or_else(|| panic!("webview: expected E5003, got Ok"));
    expect_code(
        &error,
        "E5003",
        main_function(&program, &package.names.symbols),
    );
    match &error.kind {
        CodegenErrorKind::UnrepresentableBuiltin { builtin, .. } => {
            assert_eq!(builtin.descriptor().name, "webview_window_create");
        }
        kind => panic!("webview: wrong kind {kind:?}"),
    }

    // Tuples are now lowered as flat heap records, so a tuple program compiles.
    let (program, types, package) =
        pipeline("def main() -> Int:\n    t := (1, 2)\n    return t.0\n");
    let names = Names(&package.names.symbols);
    let options = CodegenOptions::new(Target::X86_64, &names);
    CraneliftBackend
        .compile_module(&program, &types, &options)
        .unwrap_or_else(|e| panic!("tuple: expected Ok, got {e}"));
}
