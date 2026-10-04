use std::fmt;
use std::path::{Path, PathBuf};

pub trait FileSystem {
    fn is_file(&self, path: &Path) -> Result<bool, FileSystemError>;
    fn canonicalize(&self, path: &Path) -> Result<PathBuf, FileSystemError>;
    fn read_to_string(&self, path: &Path) -> Result<String, FileSystemError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct OsFileSystem;

impl FileSystem for OsFileSystem {
    fn is_file(&self, path: &Path) -> Result<bool, FileSystemError> {
        match std::fs::metadata(path) {
            Ok(metadata) => Ok(metadata.is_file()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(FileSystemError::new("inspect", path, error)),
        }
    }

    fn canonicalize(&self, path: &Path) -> Result<PathBuf, FileSystemError> {
        std::fs::canonicalize(path)
            .map_err(|error| FileSystemError::new("canonicalize", path, error))
    }

    fn read_to_string(&self, path: &Path) -> Result<String, FileSystemError> {
        std::fs::read_to_string(path).map_err(|error| FileSystemError::new("read", path, error))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSystemError {
    pub operation: &'static str,
    pub path: PathBuf,
    pub message: String,
}

impl FileSystemError {
    pub fn new(operation: &'static str, path: &Path, error: impl fmt::Display) -> Self {
        Self {
            operation,
            path: path.to_owned(),
            message: error.to_string(),
        }
    }
}

impl fmt::Display for FileSystemError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "failed to {} '{}': {}",
            self.operation,
            self.path.display(),
            self.message
        )
    }
}

impl std::error::Error for FileSystemError {}
