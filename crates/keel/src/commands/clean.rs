//! `keel clean` — remove workspace-local build outputs.

use std::path::Path;

/// Remove the workspace's shared `target/` directory.
///
/// Dependency sources staged under `.lpp_packages/` and the global Keel cache
/// are intentionally preserved. `keel cache clean` owns the latter lifecycle.
pub fn clean(directory: &Path) -> Result<(), String> {
    let workspace = lpp_pm::Workspace::discover(directory).map_err(|error| error.to_string())?;
    let output = workspace.out_dir();

    // `Workspace::out_dir` is the authority, but retain a local safety check so
    // future workspace-layout changes cannot turn this recursive deletion into
    // an arbitrary-path primitive.
    if output.file_name().and_then(|name| name.to_str()) != Some("target") {
        return Err(format!(
            "refusing to clean unexpected output path {}",
            output.display()
        ));
    }

    let metadata = match std::fs::symlink_metadata(&output) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("already clean: {}", output.display());
            return Ok(());
        }
        Err(error) => {
            return Err(format!(
                "cannot inspect build output {}: {error}",
                output.display()
            ));
        }
    };

    if metadata.file_type().is_symlink() || metadata.is_file() {
        std::fs::remove_file(&output)
    } else if metadata.is_dir() {
        std::fs::remove_dir_all(&output)
    } else {
        return Err(format!(
            "refusing to clean unsupported filesystem entry {}",
            output.display()
        ));
    }
    .map_err(|error| format!("failed to clean {}: {error}", output.display()))?;

    println!("cleaned {}", output.display());
    Ok(())
}
