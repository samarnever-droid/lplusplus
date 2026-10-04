use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use lpp_runtime_abi::{generate, v1_registry};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn schema_reproduces_every_legacy_builtin_symbol() {
    let legacy = fs::read_to_string(workspace_root().join("src/builtins.rs")).unwrap();
    let legacy_symbols = legacy
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("symbol: \"")
                .and_then(|value| value.strip_suffix("\","))
                .filter(|value| !value.is_empty())
        })
        .collect::<BTreeSet<_>>();
    let registry = v1_registry().unwrap();

    assert_eq!(registry.builtins.len(), 518);
    assert_eq!(lpp_runtime_abi::generated::BUILTINS.len(), 518);
    assert_eq!(lpp_runtime_abi::generated::LPP_ABI_VERSION, 1);
    assert_eq!(registry.symbols().len(), 354);
    assert_eq!(registry.symbols(), legacy_symbols);
}

#[test]
fn checked_generated_outputs_are_current() {
    let root = workspace_root();
    let generated = generate(&v1_registry().unwrap()).unwrap();
    let outputs = [
        ("abi/generated/builtins.rs", generated.rust),
        ("abi/generated/v1.symbols", generated.symbols),
        ("runtime/include/lpp_abi.h", generated.c_header),
        ("docs/reference/BUILTINS_GENERATED.md", generated.markdown),
    ];

    for (relative, expected) in outputs {
        let actual = fs::read_to_string(root.join(relative)).unwrap();
        assert_eq!(actual, expected, "regenerate {relative} with lpp-abi-gen");
    }
}
