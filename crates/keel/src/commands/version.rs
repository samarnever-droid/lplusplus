//! `keel version` — inspect and update package semantic versions.

use std::path::Path;

use crate::cli::{VersionAction, VersionPart};

pub fn version(directory: &Path, action: Option<&VersionAction>) -> Result<(), String> {
    let workspace = lpp_pm::Workspace::discover(directory).map_err(|error| error.to_string())?;
    let member_index = select_member(&workspace, directory)?;
    let member = &workspace.members[member_index];

    let Some(action) = action else {
        println!("{} {}", member.name(), member.manifest.version());
        return Ok(());
    };

    let next = match action {
        VersionAction::Set { version } => lpp_pm::Version::parse(version)
            .ok_or_else(|| format!("invalid semantic version '{version}'"))?
            .to_string(),
        VersionAction::Bump { part } => bump(member.manifest.version(), *part)?,
    };

    // Validate the lockfile update before touching the manifest so a version
    // bump that would violate a workspace dependency requirement is rejected
    // without leaving the project half-updated.
    let lock_path = workspace.root.join("Keel.lock");
    let updated_lock = prepare_lock_update(&lock_path, member.name(), &next)?;
    update_manifest(&member.dir.join("Keel.toml"), &next)?;
    if let Some(lock) = updated_lock {
        lock.save_atomic(&lock_path)
            .map_err(|error| error.to_string())?;
    }
    println!("{} {}", member.name(), next);
    Ok(())
}

fn select_member(workspace: &lpp_pm::Workspace, directory: &Path) -> Result<usize, String> {
    let current = directory
        .canonicalize()
        .map_err(|error| format!("cannot resolve {}: {error}", directory.display()))?;
    workspace
        .members
        .iter()
        .enumerate()
        .filter(|(_, member)| current == member.dir || current.starts_with(&member.dir))
        .max_by_key(|(_, member)| member.dir.components().count())
        .map(|(index, _)| index)
        .or_else(|| (workspace.members.len() == 1).then_some(0))
        .ok_or_else(|| {
            "workspace root is virtual; run `keel version` from a member directory".to_string()
        })
}

fn bump(current: &str, part: VersionPart) -> Result<String, String> {
    let parsed = lpp_pm::Version::parse(current)
        .ok_or_else(|| format!("manifest contains invalid semantic version '{current}'"))?;
    let version = parsed.as_semver();
    let (major, minor, patch) = match part {
        VersionPart::Major => (
            version
                .major
                .checked_add(1)
                .ok_or_else(|| "major version overflow".to_string())?,
            0,
            0,
        ),
        VersionPart::Minor => (
            version.major,
            version
                .minor
                .checked_add(1)
                .ok_or_else(|| "minor version overflow".to_string())?,
            0,
        ),
        VersionPart::Patch => (
            version.major,
            version.minor,
            version
                .patch
                .checked_add(1)
                .ok_or_else(|| "patch version overflow".to_string())?,
        ),
    };
    Ok(lpp_pm::Version::new(major, minor, patch).to_string())
}

fn update_manifest(path: &Path, version: &str) -> Result<(), String> {
    let source = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let mut document = source
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| error.to_string())?;
    let package = document
        .get_mut("package")
        .and_then(toml_edit::Item::as_table_mut)
        .ok_or_else(|| format!("{} has no [package] table", path.display()))?;
    package.insert("version", toml_edit::value(version));
    let rendered = document.to_string();
    lpp_pm::manifest::Manifest::parse(&rendered).map_err(|error| error.to_string())?;
    std::fs::write(path, rendered)
        .map_err(|error| format!("cannot update {}: {error}", path.display()))
}

fn prepare_lock_update(
    path: &Path,
    package: &str,
    version: &str,
) -> Result<Option<lpp_pm::Lock>, String> {
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    let mut lock = lpp_pm::Lock::parse(&source).map_err(|error| error.to_string())?;
    let Some(locked) = lock
        .packages
        .iter_mut()
        .find(|candidate| candidate.name == package && candidate.source != "registry")
    else {
        return Ok(None);
    };
    locked.version = version.to_string();
    lock.validate().map_err(|error| error.to_string())?;
    Ok(Some(lock))
}
