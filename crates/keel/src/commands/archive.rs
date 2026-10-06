//! Deterministic and confined Keel package archives.

use std::path::{Component, Path, PathBuf};

const MAX_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;

pub fn pack(root: &Path) -> Result<Vec<u8>, String> {
    if !root.join("Keel.toml").is_file() {
        return Err("package has no Keel.toml".to_string());
    }
    if !root.join("src").is_dir() {
        return Err("package has no src/ directory".to_string());
    }
    let mut files = Vec::new();
    collect(root, root, &mut files)?;
    files.sort();
    if files.len() > MAX_ENTRIES {
        return Err(format!("package contains more than {MAX_ENTRIES} files"));
    }

    let encoder = flate2::GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    builder.mode(tar::HeaderMode::Deterministic);
    for relative in files {
        let absolute = root.join(&relative);
        let bytes = std::fs::read(&absolute).map_err(|e| format!("{}: {e}", absolute.display()))?;
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_cksum();
        builder
            .append_data(&mut header, &relative, bytes.as_slice())
            .map_err(|e| format!("archive {}: {e}", relative.display()))?;
    }
    builder.finish().map_err(|e| e.to_string())?;
    let encoder = builder.into_inner().map_err(|e| e.to_string())?;
    let bytes = encoder.finish().map_err(|e| e.to_string())?;
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(format!(
            "compressed package is {} bytes; limit is {MAX_ARCHIVE_BYTES}",
            bytes.len()
        ));
    }
    Ok(bytes)
}

pub fn unpack(bytes: &[u8], destination: &Path) -> Result<(), String> {
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(format!(
            "compressed package is {} bytes; limit is {MAX_ARCHIVE_BYTES}",
            bytes.len()
        ));
    }
    std::fs::create_dir_all(destination).map_err(|e| e.to_string())?;
    let decoder = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|e| format!("invalid tar archive: {e}"))?;
    let mut count = 0usize;
    let mut extracted = 0u64;
    let mut seen = std::collections::BTreeSet::new();
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("invalid tar entry: {e}"))?;
        count += 1;
        if count > MAX_ENTRIES {
            return Err(format!("package contains more than {MAX_ENTRIES} entries"));
        }
        let path = entry
            .path()
            .map_err(|e| format!("invalid tar path: {e}"))?
            .into_owned();
        validate_relative_path(&path)?;
        if !seen.insert(path.clone()) {
            return Err(format!("duplicate archive path: {}", path.display()));
        }
        let entry_type = entry.header().entry_type();
        if !entry_type.is_file() && !entry_type.is_dir() {
            return Err(format!(
                "unsupported archive entry type for {} (links and special files are forbidden)",
                path.display()
            ));
        }
        extracted = extracted.saturating_add(entry.header().size().map_err(|e| e.to_string())?);
        if extracted > MAX_EXTRACTED_BYTES {
            return Err(format!(
                "extracted package exceeds {MAX_EXTRACTED_BYTES} bytes"
            ));
        }
        let inside = entry
            .unpack_in(destination)
            .map_err(|e| format!("extract {}: {e}", path.display()))?;
        if !inside {
            return Err(format!(
                "archive path escaped destination: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

/// Hash a staged tree without following links. Generated staging metadata is
/// excluded so it cannot validate itself.
pub fn tree_hash(root: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    collect_tree(root, root, &mut files)?;
    files.sort();
    let mut accumulator = String::new();
    for relative in files {
        if relative == Path::new("lpp.toml") || relative == Path::new(".keel-stage") {
            continue;
        }
        let bytes = std::fs::read(root.join(&relative)).map_err(|e| e.to_string())?;
        accumulator.push_str(&relative.to_string_lossy().replace('\\', "/"));
        accumulator.push(':');
        accumulator.push_str(&lpp_pm::fingerprint::hash_bytes(&bytes));
        accumulator.push('\n');
    }
    Ok(lpp_pm::fingerprint::hash_bytes(accumulator.as_bytes()))
}

fn collect(root: &Path, current: &Path, output: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut entries = std::fs::read_dir(current)
        .map_err(|e| format!("{}: {e}", current.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let relative = path.strip_prefix(root).map_err(|e| e.to_string())?;
        if excluded(relative) {
            continue;
        }
        let metadata = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "package contains symlink {}; symlinks are forbidden",
                relative.display()
            ));
        }
        if metadata.is_dir() {
            collect(root, &path, output)?;
        } else if metadata.is_file() {
            output.push(relative.to_path_buf());
        } else {
            return Err(format!(
                "package contains special file {}",
                relative.display()
            ));
        }
    }
    Ok(())
}

fn collect_tree(root: &Path, current: &Path, output: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut entries = std::fs::read_dir(current)
        .map_err(|e| format!("{}: {e}", current.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let relative = path.strip_prefix(root).map_err(|e| e.to_string())?;
        let metadata = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "staged package contains symlink {}",
                relative.display()
            ));
        }
        if metadata.is_dir() {
            collect_tree(root, &path, output)?;
        } else if metadata.is_file() {
            output.push(relative.to_path_buf());
        } else {
            return Err(format!(
                "staged package contains special file {}",
                relative.display()
            ));
        }
    }
    Ok(())
}

fn excluded(relative: &Path) -> bool {
    relative.components().any(|component| {
        let Component::Normal(name) = component else {
            return true;
        };
        let name = name.to_string_lossy();
        matches!(
            name.as_ref(),
            ".git" | "target" | ".lpp_packages" | "Keel.lock"
        ) || name == ".env"
            || name.starts_with(".env.")
    })
}

fn validate_relative_path(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty() {
        return Err("archive contains an empty path".to_string());
    }
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!("unsafe archive path: {}", path.display()));
            }
        }
    }
    Ok(())
}
