//! `keel list` — list workspace packages and direct dependencies.

use std::fmt::Write as _;
use std::path::Path;

pub fn list(directory: &Path) -> Result<(), String> {
    let workspace = lpp_pm::Workspace::discover(directory).map_err(|error| error.to_string())?;
    print!("{}", render(&workspace));
    Ok(())
}

pub fn render(workspace: &lpp_pm::Workspace) -> String {
    let mut output = String::new();
    for member in &workspace.members {
        writeln!(output, "{} {}", member.name(), member.manifest.version()).unwrap();
        if member.manifest.dependencies.is_empty() {
            writeln!(output, "  (no dependencies)").unwrap();
            continue;
        }
        for (name, dependency) in &member.manifest.dependencies {
            let source = if let Some(path) = dependency.path() {
                format!("path:{path}")
            } else if let Some(git) = dependency.git() {
                format!("git:{git}")
            } else {
                "registry".to_string()
            };
            writeln!(output, "  {name} {} [{source}]", dependency.version()).unwrap();
        }
    }
    output
}
