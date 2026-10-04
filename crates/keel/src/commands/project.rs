//! `keel new` / `init` / `add` / `remove` — project + manifest management.

use std::path::{Path, PathBuf};

const TEMPLATE_MAIN: &str = "fn main() {\n    // L++ entry point\n}\n";

fn manifest_template(name: &str) -> String {
    format!(
        "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\n"
    )
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn load(dir: &Path) -> Result<lpp_pm::manifest::Manifest, String> {
    let p = dir.join("Keel.toml");
    if !p.exists() {
        return Err("no Keel.toml in the current directory (run `keel init` first)".to_string());
    }
    lpp_pm::manifest::Manifest::parse(&std::fs::read_to_string(&p).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

fn save(dir: &Path, manifest: &lpp_pm::manifest::Manifest) -> Result<(), String> {
    let toml = manifest.to_toml().map_err(|e| e.to_string())?;
    std::fs::write(dir.join("Keel.toml"), toml).map_err(|e| e.to_string())
}

/// Create a new project in a fresh `<name>/` directory under `parent`.
pub fn new_project(name: &str, parent: &Path) -> Result<PathBuf, String> {
    if !valid_name(name) {
        return Err(format!(
            "invalid package name '{name}' (use lowercase letters, digits, and hyphens)"
        ));
    }
    let dir = parent.join(name);
    if dir.exists() {
        return Err(format!("'{name}' already exists"));
    }
    std::fs::create_dir_all(dir.join("src")).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("Keel.toml"), manifest_template(name)).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("src/main.lpp"), TEMPLATE_MAIN).map_err(|e| e.to_string())?;
    println!("created project '{name}' at {}", dir.display());
    Ok(dir)
}

/// Initialize a project in `dir` (idempotent: never overwrites existing files).
pub fn init(dir: &Path) -> Result<(), String> {
    let manifest = dir.join("Keel.toml");
    if !manifest.exists() {
        let name = dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("project")
            .to_string();
        std::fs::write(&manifest, manifest_template(&name)).map_err(|e| e.to_string())?;
        println!("created Keel.toml");
    }
    let main = dir.join("src/main.lpp");
    if !main.exists() {
        std::fs::create_dir_all(dir.join("src")).map_err(|e| e.to_string())?;
        std::fs::write(&main, TEMPLATE_MAIN).map_err(|e| e.to_string())?;
        println!("created src/main.lpp");
    }
    Ok(())
}

/// Add a dependency to the project's `[dependencies]`.
pub fn add_dep(dir: &Path, name: &str, req: &str) -> Result<(), String> {
    if !valid_name(name) {
        return Err(format!("invalid package name '{name}'"));
    }
    let mut manifest = load(dir)?;
    if manifest.dependencies.contains_key(name) {
        return Err(format!("'{name}' is already a dependency"));
    }
    manifest.dependencies.insert(
        name.to_string(),
        lpp_pm::manifest::Dependency::Version(req.to_string()),
    );
    save(dir, &manifest)?;
    println!("added {name} = \"{req}\" to Keel.toml");
    Ok(())
}

/// Remove a dependency from the project's `[dependencies]`.
pub fn remove_dep(dir: &Path, name: &str) -> Result<(), String> {
    let mut manifest = load(dir)?;
    if manifest.dependencies.remove(name).is_none() {
        return Err(format!("'{name}' is not a dependency"));
    }
    save(dir, &manifest)?;
    println!("removed {name} from Keel.toml");
    Ok(())
}
