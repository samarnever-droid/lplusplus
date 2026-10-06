use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{PmError, Result};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Durably write a sibling temporary file and atomically replace `path` where
/// the platform supports replacement-by-rename. Temporary names include a
/// process-local sequence so concurrent threads never collide.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(io_error)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("keel-data");
    let temporary = parent.join(format!(
        ".{name}.tmp-{}-{}",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));

    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(io_error)?;
        file.write_all(bytes).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        drop(file);

        #[cfg(not(windows))]
        std::fs::rename(&temporary, path).map_err(io_error)?;

        // `std::fs::rename` cannot replace an existing file on Windows. Keep
        // the unavoidable compatibility fallback isolated to that platform;
        // unique temporary names still prevent writer collisions.
        #[cfg(windows)]
        {
            if path.exists() {
                std::fs::remove_file(path).map_err(io_error)?;
            }
            std::fs::rename(&temporary, path).map_err(io_error)?;
        }

        #[cfg(unix)]
        {
            let directory = std::fs::File::open(parent).map_err(io_error)?;
            directory.sync_all().map_err(io_error)?;
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn io_error(error: std::io::Error) -> PmError {
    PmError::Io(error.to_string())
}
