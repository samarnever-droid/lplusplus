use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use lpp_runtime_abi::{AbiRegistry, generate};

fn main() {
    if let Err(error) = run(env::args().skip(1)) {
        eprintln!("lpp-abi-gen: {error}");
        std::process::exit(1);
    }
}

fn run(arguments: impl IntoIterator<Item = String>) -> Result<(), String> {
    let mut arguments = arguments.into_iter();
    let schema = arguments
        .next()
        .map_or_else(|| PathBuf::from("abi/builtins.toml"), PathBuf::from);
    let root = arguments
        .next()
        .map_or_else(|| PathBuf::from("."), PathBuf::from);
    if arguments.next().is_some() {
        return Err("usage: lpp-abi-gen [schema] [workspace-root]".to_string());
    }

    let input = fs::read_to_string(&schema)
        .map_err(|error| format!("cannot read {}: {error}", schema.display()))?;
    let registry = AbiRegistry::parse(&input).map_err(|error| error.to_string())?;
    let generated = generate(&registry).map_err(|error| error.to_string())?;

    write(&root.join("abi/generated/builtins.rs"), &generated.rust)?;
    write(&root.join("abi/generated/v1.symbols"), &generated.symbols)?;
    write(&root.join("runtime/include/lpp_abi.h"), &generated.c_header)?;
    write(
        &root.join("docs/reference/BUILTINS_GENERATED.md"),
        &generated.markdown,
    )?;
    Ok(())
}

fn write(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    fs::write(path, content).map_err(|error| format!("cannot write {}: {error}", path.display()))
}
