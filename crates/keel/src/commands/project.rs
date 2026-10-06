//! `keel new` / `init` / `add` / `remove` — project + manifest management.

use std::path::{Path, PathBuf};

const TEMPLATE_MAIN: &str = "def main():\n    print(\"Hello from L++\")\n";

fn manifest_template(name: &str) -> String {
    format!(
        "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\n"
    )
}

fn validate_name(name: &str) -> Result<(), String> {
    lpp_pm::validation::package_name(name).map_err(|error| error.to_string())
}

fn load_text(dir: &Path) -> Result<String, String> {
    let path = dir.join("Keel.toml");
    if !path.exists() {
        return Err("no Keel.toml in the current directory (run `keel init` first)".to_string());
    }
    std::fs::read_to_string(path).map_err(|error| error.to_string())
}

fn save_validated(dir: &Path, text: String) -> Result<(), String> {
    lpp_pm::manifest::Manifest::parse(&text).map_err(|error| error.to_string())?;
    let path = dir.join("Keel.toml");
    let tmp = path.with_extension(format!("toml.tmp-{}", std::process::id()));
    std::fs::write(&tmp, text).map_err(|error| error.to_string())?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|error| error.to_string())?;
    }
    std::fs::rename(tmp, path).map_err(|error| error.to_string())
}

pub fn new_project(name: &str, parent: &Path) -> Result<PathBuf, String> {
    validate_name(name)?;
    let dir = parent.join(name);
    if dir.exists() {
        return Err(format!("'{name}' already exists"));
    }
    std::fs::create_dir_all(dir.join("src")).map_err(|error| error.to_string())?;
    std::fs::write(dir.join("Keel.toml"), manifest_template(name))
        .map_err(|error| error.to_string())?;
    std::fs::write(dir.join("src/main.lpp"), TEMPLATE_MAIN).map_err(|error| error.to_string())?;
    println!("created project '{name}' at {}", dir.display());
    Ok(dir)
}

/// Initialize a project without overwriting user files.
pub fn init(dir: &Path) -> Result<(), String> {
    let manifest = dir.join("Keel.toml");
    if !manifest.exists() {
        let name = dir
            .canonicalize()
            .unwrap_or_else(|_| dir.to_path_buf())
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("project")
            .to_string();
        validate_name(&name).map_err(|_| {
            format!(
                "directory name '{name}' is not a valid package name; rename it or create Keel.toml explicitly"
            )
        })?;
        std::fs::write(&manifest, manifest_template(&name)).map_err(|error| error.to_string())?;
        println!("created Keel.toml");
    } else {
        lpp_pm::manifest::Manifest::parse(
            &std::fs::read_to_string(&manifest).map_err(|e| e.to_string())?,
        )
        .map_err(|error| error.to_string())?;
    }
    let main = dir.join("src/main.lpp");
    if !main.exists() {
        std::fs::create_dir_all(dir.join("src")).map_err(|error| error.to_string())?;
        std::fs::write(&main, TEMPLATE_MAIN).map_err(|error| error.to_string())?;
        println!("created src/main.lpp");
    }
    Ok(())
}

pub fn add_dep(dir: &Path, name: &str, requirement: &str) -> Result<(), String> {
    validate_name(name)?;
    lpp_pm::validation::requirement(requirement).map_err(|error| error.to_string())?;
    let source = load_text(dir)?;
    // Validate before editing, then preserve comments and unknown tables with
    // toml_edit instead of serializing through the typed subset.
    lpp_pm::manifest::Manifest::parse(&source).map_err(|error| error.to_string())?;
    let mut document = source
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| error.to_string())?;
    let dependencies = document
        .entry("dependencies")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .ok_or_else(|| "[dependencies] must be a TOML table".to_string())?;
    if dependencies.contains_key(name) {
        return Err(format!("'{name}' is already a dependency"));
    }
    dependencies.insert(name, toml_edit::value(requirement));
    save_validated(dir, document.to_string())?;
    println!("added {name} = \"{requirement}\" to Keel.toml");
    Ok(())
}

pub fn remove_dep(dir: &Path, name: &str) -> Result<(), String> {
    validate_name(name)?;
    let source = load_text(dir)?;
    lpp_pm::manifest::Manifest::parse(&source).map_err(|error| error.to_string())?;
    let mut document = source
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| error.to_string())?;
    let removed = document
        .get_mut("dependencies")
        .and_then(toml_edit::Item::as_table_mut)
        .and_then(|dependencies| dependencies.remove(name));
    if removed.is_none() {
        return Err(format!("'{name}' is not a dependency"));
    }
    save_validated(dir, document.to_string())?;
    println!("removed {name} from Keel.toml");
    Ok(())
}
