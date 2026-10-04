//! Phase 5A gate — the codegen contract holds.
//!
//! 1. The machine lattice is exactly {I8, I32, I64, F64, I64X2} and
//!    both registry mappings are total and exact.
//! 2. Builtin lowering parity against the checked-in generated table:
//!    518/518 entries agree on symbol, arity, and machine types;
//!    empty-symbol builtins (e.g. `print`) are exactly the
//!    non-runtime-lowerable set.
//! 3. The semantic-vs-machine divergence census is pinned with exact
//!    counts (boxed vectors, arity expansion).
//! 4. The target lattice is exactly {x86_64, aarch64, wasm32_wasip1}
//!    (the wasm32 target is added by 5D) with fixed deterministic
//!    triples and pointer widths.
//! 5. The `E5xxx` table maps one stable code per kind.

use lpp_codegen_api::{
    BuiltinLowering, CodegenError, CodegenErrorKind, MachineType, Target, machine_type_for_abi,
    runtime_symbol_lowerable, semantic_machine_type,
};
use lpp_mir::MirFunctionId;
use lpp_runtime_abi::generated::{AbiType, BUILTINS, BuiltinAbi, SemanticAbiType};
use lpp_runtime_abi::v1_registry;
use lpp_types::BuiltinId;

const MACHINE_LATTICE: &[MachineType] = &[
    MachineType::I8,
    MachineType::I32,
    MachineType::I64,
    MachineType::F64,
    MachineType::I64X2,
];

#[test]
fn machine_lattice_is_bounded_and_mappings_are_total() {
    assert_eq!(MachineType::all(), MACHINE_LATTICE);

    // machine ABI table: 5 variants, exact mapping, Void -> None.
    assert_eq!(machine_type_for_abi(AbiType::Bool), Some(MachineType::I8));
    assert_eq!(machine_type_for_abi(AbiType::F64), Some(MachineType::F64));
    assert_eq!(machine_type_for_abi(AbiType::I32), Some(MachineType::I32));
    assert_eq!(machine_type_for_abi(AbiType::I64), Some(MachineType::I64));
    assert_eq!(machine_type_for_abi(AbiType::Void), None);

    // semantic table: 9 variants, exact mapping, Void -> None,
    // VectorI64x2 is the only I64X2 source.
    assert_eq!(
        semantic_machine_type(SemanticAbiType::Any),
        Some(MachineType::I64)
    );
    assert_eq!(
        semantic_machine_type(SemanticAbiType::Bool),
        Some(MachineType::I8)
    );
    assert_eq!(
        semantic_machine_type(SemanticAbiType::F64),
        Some(MachineType::F64)
    );
    assert_eq!(
        semantic_machine_type(SemanticAbiType::I32),
        Some(MachineType::I32)
    );
    assert_eq!(
        semantic_machine_type(SemanticAbiType::I64),
        Some(MachineType::I64)
    );
    assert_eq!(
        semantic_machine_type(SemanticAbiType::Str),
        Some(MachineType::I64)
    );
    assert_eq!(
        semantic_machine_type(SemanticAbiType::StrSlice),
        Some(MachineType::I64)
    );
    assert_eq!(
        semantic_machine_type(SemanticAbiType::VectorI64x2),
        Some(MachineType::I64X2)
    );
    assert_eq!(semantic_machine_type(SemanticAbiType::Void), None);
}

fn assert_builtin_parity(builtin: BuiltinId, expected: &BuiltinAbi) {
    assert_eq!(
        builtin.descriptor(),
        expected,
        "descriptor parity for builtin {expected:?}"
    );

    let descriptor = builtin.descriptor();
    let lowerable = runtime_symbol_lowerable(builtin);
    assert_eq!(
        lowerable,
        !descriptor.symbol.is_empty(),
        "lowerability for {:?}",
        descriptor.name
    );
    if !lowerable {
        return;
    }

    // The machine table is the only source of import signatures.
    let lowering = BuiltinLowering::from_builtin(builtin)
        .unwrap_or_else(|| panic!("lowering for {:?} must exist", descriptor.name));
    assert_eq!(lowering.name, descriptor.name);
    assert_eq!(lowering.symbol, descriptor.symbol);
    assert_eq!(lowering.parameters.len(), descriptor.parameters.len());
    for (index, (machine_abi, expected_machine)) in descriptor
        .parameters
        .iter()
        .zip(lowering.parameters.iter())
        .enumerate()
    {
        assert_eq!(
            *expected_machine,
            machine_type_for_abi(*machine_abi).unwrap_or_else(|| {
                panic!(
                    "{:?}: machine ABI {machine_abi:?} unmapped",
                    descriptor.name
                )
            }),
            "{:?}: parameter {index} machine type",
            descriptor.name
        );
    }
    let expected_result = machine_type_for_abi(descriptor.result);
    assert_eq!(
        lowering.result, expected_result,
        "{:?}: result machine type",
        descriptor.name
    );
}

#[test]
fn builtin_lowering_parity_across_the_whole_generated_table() {
    assert_eq!(BUILTINS.len(), 518, "table size drift");

    let registry = v1_registry().expect("v1 registry parses");
    assert_eq!(registry.builtins.len(), BUILTINS.len());
    for (raw, expected) in BUILTINS.iter().enumerate() {
        let builtin = BuiltinId::from_raw(raw as u32);
        let parsed = &registry.builtins[raw];
        // parsed schema and generated table must agree on identity.
        assert_eq!(parsed.name, expected.name, "name at {raw}");
        assert_eq!(parsed.symbol, expected.symbol, "symbol at {raw}");
        assert_builtin_parity(builtin, expected);
    }

    // `print` is the canonical empty-symbol builtin: no single runtime
    // import, hence not runtime-lowerable; the backend handles it by
    // name or rejects with E5003.
    let print = BUILTINS
        .iter()
        .position(|b| b.name == "print")
        .expect("print in table");
    assert!(BUILTINS[print].symbol.is_empty());
    assert!(!runtime_symbol_lowerable(BuiltinId::from_raw(print as u32)));
    assert!(BuiltinLowering::from_builtin(BuiltinId::from_raw(print as u32)).is_none());

    // Census: exactly the empty-symbol entries are non-lowerable.
    for (raw, entry) in BUILTINS.iter().enumerate() {
        let is_lowerable = BuiltinLowering::from_builtin(BuiltinId::from_raw(raw as u32)).is_some();
        assert_eq!(is_lowerable, !entry.symbol.is_empty(), "{:?}", entry.name);
    }
}

#[test]
fn semantic_vs_machine_divergence_census_is_pinned() {
    // The semantic and machine tables are not inverses. Pin the v1
    // divergences with exact counts so any table drift fails loudly:
    // v1 lowers VectorI64x2 as a boxed I64 pointer, and machine arity
    // may exceed semantic arity where lowering adds detail.
    let mut vector_results = 0;
    let mut vector_params = 0;
    let mut arity_expansions: Vec<(&str, usize, usize)> = Vec::new();
    for entry in BUILTINS {
        if entry.symbol.is_empty() {
            continue;
        }
        if entry.semantic_result == SemanticAbiType::VectorI64x2 {
            assert_eq!(
                entry.result,
                AbiType::I64,
                "{:?}: boxed vector result",
                entry.name
            );
            vector_results += 1;
        }
        let mut semantic_params = entry.semantic_parameters.iter();
        for machine_param in entry.parameters {
            if let Some(SemanticAbiType::VectorI64x2) = semantic_params.next() {
                assert_eq!(
                    *machine_param,
                    AbiType::I64,
                    "{:?}: boxed vector param",
                    entry.name
                );
                vector_params += 1;
            }
        }
        if entry.parameters.len() != entry.semantic_parameters.len() {
            arity_expansions.push((
                entry.name,
                entry.semantic_parameters.len(),
                entry.parameters.len(),
            ));
        }
    }
    assert_eq!(vector_results, 13, "vector result census drift");
    assert_eq!(vector_params, 22, "vector param census drift");
    arity_expansions.sort_unstable();
    assert_eq!(
        arity_expansions,
        vec![("slice", 3, 5), ("str_slice", 3, 5), ("vec_i64x2", 2, 4),],
        "arity expansion census drift"
    );
}

#[test]
fn target_lattice_is_bounded_with_fixed_triples() {
    assert_eq!(
        Target::all(),
        &[Target::X86_64, Target::Aarch64, Target::Wasm32Wasi]
    );
    assert_eq!(Target::X86_64.triple(), "x86_64-unknown-linux-gnu");
    assert_eq!(Target::Aarch64.triple(), "aarch64-unknown-linux-gnu");
    assert_eq!(Target::Wasm32Wasi.triple(), "wasm32-wasip1");
    assert_eq!(Target::X86_64.pointer_bits(), 64);
    assert_eq!(Target::Aarch64.pointer_bits(), 64);
    assert_eq!(Target::Wasm32Wasi.pointer_bits(), 32);
}

#[test]
fn e5xx_table_is_one_code_per_kind() {
    let spawn_index = BUILTINS
        .iter()
        .position(|b| b.name == "thread_spawn")
        .expect("thread_spawn in table");

    let cases = [
        (
            CodegenErrorKind::UnsupportedConstruct {
                construct: "MakeClosure",
            },
            "E5001",
        ),
        (CodegenErrorKind::UnsupportedTarget(Target::X86_64), "E5002"),
        (
            CodegenErrorKind::UnrepresentableBuiltin {
                builtin: BuiltinId::from_raw(spawn_index as u32),
                reason: "async not in 5B slice",
            },
            "E5003",
        ),
        (
            CodegenErrorKind::IrVerificationFailed("verifier rejected fn".into()),
            "E5004",
        ),
        (
            CodegenErrorKind::ObjectEmissionFailed("emit failed".into()),
            "E5005",
        ),
        (
            CodegenErrorKind::AbiMismatch {
                symbol: "lpp_print_str".into(),
                expected_arity: 1,
                actual_arity: 2,
            },
            "E5006",
        ),
    ];
    for (kind, code) in cases {
        let error = CodegenError::new(None, kind);
        assert_eq!(error.code(), code);
        assert!(error.to_string().starts_with(code));
    }
    let anchored = CodegenError::new(
        Some(MirFunctionId::from_raw(3)),
        CodegenErrorKind::IrVerificationFailed("x".into()),
    );
    assert_eq!(anchored.function, Some(MirFunctionId::from_raw(3)));
    assert_eq!(anchored.code(), "E5004");
}
