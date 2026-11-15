//! `keel migrate` — upgrade legacy `lpp.toml` / `lpp.lock` manifests to modern `Keel.toml` / `Keel.lock`.

use std::path::{Path, PathBuf};

/// Convert legacy `lpp.toml` source text to validated modern `Keel.toml` text.
pub fn convert_manifest(raw: &str) -> Result<String, String> {
    let mut doc: toml_edit::DocumentMut = raw
        .parse()
        .map_err(|e| format!("failed to parse lpp.toml: {e}"))?;

    let has_package = doc
        .get("package")
        .and_then(|item| item.as_table())
        .is_some();
    let has_workspace = doc
        .get("workspace")
        .and_then(|item| item.as_table())
        .is_some();

    if !has_package && !has_workspace {
        return Err("legacy manifest has neither [package] nor [workspace]".to_string());
    }

    if let Some(pkg) = doc.get_mut("package").and_then(|item| item.as_table_mut()) {
        if !pkg.contains_key("edition") {
            pkg.insert("edition", toml_edit::value("2024"));
        }
        if pkg.contains_key("author") {
            let single_author = pkg
                .get("author")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned);
            if !pkg.contains_key("authors")
                && let Some(author) = single_author
            {
                let mut arr = toml_edit::Array::new();
                arr.push(author);
                pkg.insert(
                    "authors",
                    toml_edit::Item::Value(toml_edit::Value::Array(arr)),
                );
            }
            pkg.remove("author");
        }
        if pkg
            .get("entry")
            .and_then(|v| v.as_str())
            .is_some_and(|e| e == "src/main.lpp" || e == "src/lib.lpp")
        {
            pkg.remove("entry");
        }
    }

    if has_package && doc.get("dependencies").is_none() {
        doc.insert(
            "dependencies",
            toml_edit::Item::Table(toml_edit::Table::new()),
        );
    }

    let rendered = doc.to_string();
    if has_package {
        lpp_pm::manifest::Manifest::parse(&rendered)
            .map_err(|e| format!("migrated manifest failed validation: {e}"))?;
    }
    Ok(rendered)
}

/// Convert a legacy `lpp.lock` document into a validated modern `Keel.lock` (`version = 2`).
pub fn convert_lockfile(raw: &str) -> Result<String, String> {
    if let Ok(existing) = lpp_pm::Lock::parse(raw) {
        return existing.to_toml().map_err(|e| e.to_string());
    }

    let doc: toml_edit::DocumentMut = raw
        .parse()
        .map_err(|e| format!("failed to parse lpp.lock: {e}"))?;

    let mut lock = lpp_pm::Lock {
        lock_version: lpp_pm::lock::LOCK_VERSION,
        registry: doc
            .get("registry")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned),
        packages: Vec::new(),
    };

    let entries = doc
        .get("packages")
        .or_else(|| doc.get("package"))
        .and_then(|item| item.as_array_of_tables());
    if let Some(tables) = entries {
        for table in tables {
            let Some(name) = table.get("name").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(version) = table.get("version").and_then(|v| v.as_str()) else {
                continue;
            };
            let source = table
                .get("source")
                .and_then(|v| v.as_str())
                .unwrap_or("registry")
                .to_string();
            let checksum = table
                .get("checksum")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned);
            let deps = table
                .get("deps")
                .or_else(|| table.get("dependencies"))
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| item.as_str().map(ToOwned::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            lock.packages.push(lpp_pm::LockedPkg {
                name: name.to_string(),
                version: version.to_string(),
                source,
                checksum,
                deps,
                dep_reqs: Default::default(),
            });
        }
    }

    lock.to_toml()
        .map_err(|e| format!("migrated lockfile failed validation: {e}"))
}

fn atomic_write(path: &Path, contents: &str) -> Result<(), String> {
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&tmp, contents).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

fn migrate_dir(dir: &Path, dry_run: bool, keep_legacy: bool) -> Result<bool, String> {
    let legacy = dir.join("lpp.toml");
    let modern = dir.join("Keel.toml");
    let mut changed = false;

    if legacy.is_file() {
        if modern.is_file() {
            return Err(format!(
                "both {} and {} exist; remove the legacy manifest manually before migrating",
                modern.display(),
                legacy.display()
            ));
        }
        let raw = std::fs::read_to_string(&legacy).map_err(|e| e.to_string())?;
        let converted = convert_manifest(&raw)?;
        if dry_run {
            println!("--- {} (preview) ---\n{}", modern.display(), converted);
        } else {
            atomic_write(&modern, &converted)?;
            if !keep_legacy {
                std::fs::remove_file(&legacy).map_err(|e| e.to_string())?;
            }
            println!("migrated {} -> {}", legacy.display(), modern.display());
        }
        changed = true;
    }

    let legacy_lock = dir.join("lpp.lock");
    let modern_lock = dir.join("Keel.lock");
    if legacy_lock.is_file() && !modern_lock.is_file() {
        let lock_raw = std::fs::read_to_string(&legacy_lock).map_err(|e| e.to_string())?;
        let converted_lock = convert_lockfile(&lock_raw)?;
        if dry_run {
            println!(
                "--- {} (preview) ---\n{}",
                modern_lock.display(),
                converted_lock
            );
        } else {
            atomic_write(&modern_lock, &converted_lock)?;
            if !keep_legacy {
                std::fs::remove_file(&legacy_lock).map_err(|e| e.to_string())?;
            }
            println!(
                "migrated {} -> {}",
                legacy_lock.display(),
                modern_lock.display()
            );
        }
        changed = true;
    }

    Ok(changed)
}

fn workspace_member_dirs(root: &Path) -> Vec<PathBuf> {
    let manifest_path = if root.join("Keel.toml").is_file() {
        root.join("Keel.toml")
    } else if root.join("lpp.toml").is_file() {
        root.join("lpp.toml")
    } else {
        return Vec::new();
    };
    let Ok(raw) = std::fs::read_to_string(manifest_path) else {
        return Vec::new();
    };
    let Ok(doc) = raw.parse::<toml_edit::DocumentMut>() else {
        return Vec::new();
    };
    let Some(members) = doc
        .get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(|m| m.as_array())
    else {
        return Vec::new();
    };

    let mut dirs = Vec::new();
    for item in members {
        let Some(pat) = item.as_str() else { continue };
        let pat = pat.trim();
        if let Some((prefix, suffix)) = pat.split_once('*') {
            let parent = root.join(prefix);
            let Ok(entries) = std::fs::read_dir(&parent) else {
                continue;
            };
            let suffix = suffix.trim_matches('/');
            for entry in entries.flatten() {
                if !entry.path().is_dir() {
                    continue;
                }
                let candidate = if suffix.is_empty() {
                    entry.path()
                } else {
                    entry.path().join(suffix)
                };
                if candidate.is_dir() {
                    dirs.push(candidate);
                }
            }
        } else {
            let candidate = root.join(pat);
            if candidate.is_dir() {
                dirs.push(candidate);
            }
        }
    }
    dirs.sort();
    dirs.dedup();
    dirs
}

/// Migrate a legacy `lpp.toml` package or workspace under `dir` to `Keel.toml`.
pub fn migrate(dir: &Path, dry_run: bool, keep_legacy: bool) -> Result<(), String> {
    let mut member_dirs = workspace_member_dirs(dir);
    let root_changed = migrate_dir(dir, dry_run, keep_legacy)?;
    if member_dirs.is_empty() {
        member_dirs = workspace_member_dirs(dir);
    }

    let mut any_changed = root_changed;
    for member in member_dirs {
        if migrate_dir(&member, dry_run, keep_legacy)? {
            any_changed = true;
        }
    }

    if !any_changed {
        if dir.join("Keel.toml").is_file() {
            println!("already using Keel.toml (nothing to migrate)");
            return Ok(());
        }
        return Err("no lpp.toml or Keel.toml found in the current directory".to_string());
    }
    Ok(())
}
