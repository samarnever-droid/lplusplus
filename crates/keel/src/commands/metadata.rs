//! `keel metadata` — deterministic, script-friendly workspace metadata.

use std::fmt::Write as _;
use std::path::Path;

pub fn metadata(directory: &Path) -> Result<(), String> {
    let workspace = lpp_pm::Workspace::discover(directory).map_err(|error| error.to_string())?;
    let rendered = render(&workspace)?;
    print!("{rendered}");
    Ok(())
}

/// Render stable line-oriented metadata without terminal tables or colors.
/// Package order is the validated workspace declaration order; dependency
/// counts come directly from each package's manifest.
pub fn render(workspace: &lpp_pm::Workspace) -> Result<String, String> {
    let locked_packages = read_lock_count(&workspace.root)?;
    let mut output = String::new();
    writeln!(output, "workspace_root = {}", workspace.root.display()).unwrap();
    writeln!(output, "virtual_workspace = {}", workspace.virtual_root).unwrap();
    writeln!(output, "workspace_members = {}", workspace.members.len()).unwrap();
    writeln!(output, "locked_packages = {locked_packages}").unwrap();

    if workspace.members.len() == 1 {
        let member = &workspace.members[0];
        writeln!(output, "name = {}", member.name()).unwrap();
        writeln!(output, "version = {}", member.manifest.version()).unwrap();
        writeln!(output, "edition = {}", member.manifest.package.edition).unwrap();
        writeln!(
            output,
            "dependencies = {}",
            member.manifest.dependencies.len()
        )
        .unwrap();
    }

    for (index, member) in workspace.members.iter().enumerate() {
        writeln!(output, "member.{index}.name = {}", member.name()).unwrap();
        writeln!(
            output,
            "member.{index}.version = {}",
            member.manifest.version()
        )
        .unwrap();
        writeln!(
            output,
            "member.{index}.directory = {}",
            member.dir.display()
        )
        .unwrap();
        writeln!(
            output,
            "member.{index}.dependencies = {}",
            member.manifest.dependencies.len()
        )
        .unwrap();
    }
    Ok(output)
}

fn read_lock_count(root: &Path) -> Result<usize, String> {
    let path = root.join("Keel.lock");
    let document = match std::fs::read_to_string(&path) {
        Ok(document) => document,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    let lock = lpp_pm::Lock::parse(&document).map_err(|error| error.to_string())?;
    Ok(lock.packages.len())
}
