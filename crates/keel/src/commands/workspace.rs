//! Compatibility-oriented workspace inspection for the `lpp workspace` route.

use std::fmt::Write as _;
use std::path::Path;

pub fn members(directory: &Path) -> Result<(), String> {
    let workspace = discover(directory)?;
    print!("{}", render_members(&workspace));
    Ok(())
}

pub fn graph(directory: &Path) -> Result<(), String> {
    let workspace = discover(directory)?;
    print!("{}", render_graph(&workspace));
    Ok(())
}

pub fn discover(directory: &Path) -> Result<lpp_pm::Workspace, String> {
    lpp_pm::Workspace::discover(directory).map_err(|error| error.to_string())
}

pub fn render_members(workspace: &lpp_pm::Workspace) -> String {
    let mut output = String::new();
    writeln!(output, "Workspace: {}", workspace.root.display()).unwrap();
    for member in &workspace.members {
        let relative = member
            .dir
            .strip_prefix(&workspace.root)
            .unwrap_or(&member.dir);
        let relative = if relative.as_os_str().is_empty() {
            Path::new(".")
        } else {
            relative
        };
        writeln!(
            output,
            "  {} @ {} ({})",
            member.name(),
            member.manifest.version(),
            relative.display()
        )
        .unwrap();
    }
    output
}

pub fn render_graph(workspace: &lpp_pm::Workspace) -> String {
    let mut output = String::new();
    writeln!(
        output,
        "Workspace dependency graph: {}",
        workspace.root.display()
    )
    .unwrap();
    for member in &workspace.members {
        if member.manifest.dependencies.is_empty() {
            writeln!(output, "  {} -> (none)", member.name()).unwrap();
        } else {
            writeln!(
                output,
                "  {} -> {}",
                member.name(),
                member
                    .manifest
                    .dependencies
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .unwrap();
        }
    }
    output
}

pub fn require_member<'a>(
    workspace: &'a lpp_pm::Workspace,
    package: &str,
) -> Result<&'a lpp_pm::Member, String> {
    workspace
        .members
        .iter()
        .find(|member| member.name() == package)
        .ok_or_else(|| format!("workspace member not found: {package}"))
}
