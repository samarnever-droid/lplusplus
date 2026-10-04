//! Source storage and stable byte-based locations.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// Stable identity assigned to a source file within one compiler session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FileId(u32);

impl FileId {
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// Half-open byte range in one source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Span {
    pub file: FileId,
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(file: FileId, start: usize, end: usize) -> Result<Self, SourceMapError> {
        if start > end {
            return Err(SourceMapError::ReversedSpan { start, end });
        }
        let start = u32::try_from(start).map_err(|_| SourceMapError::SourceTooLarge)?;
        let end = u32::try_from(end).map_err(|_| SourceMapError::SourceTooLarge)?;
        Ok(Self { file, start, end })
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }
}

/// Immutable source text and precomputed line starts.
#[derive(Debug, Clone)]
pub struct SourceFile {
    id: FileId,
    name: Arc<str>,
    text: Arc<str>,
    line_starts: Arc<[u32]>,
}

impl SourceFile {
    #[must_use]
    pub const fn id(&self) -> FileId {
        self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.text.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Convert a byte offset into one-based line and Unicode-scalar column.
    pub fn line_column(&self, offset: u32) -> Result<(usize, usize), SourceMapError> {
        let offset = offset as usize;
        if offset > self.text.len() || !self.text.is_char_boundary(offset) {
            return Err(SourceMapError::InvalidOffset {
                file: self.id,
                offset,
            });
        }

        let line_index = self
            .line_starts
            .partition_point(|line_start| (*line_start as usize) <= offset)
            .saturating_sub(1);
        let line_start = self.line_starts[line_index] as usize;
        let column = self.text[line_start..offset].chars().count() + 1;
        Ok((line_index + 1, column))
    }

    /// Return the source line containing `offset`, without its line ending.
    pub fn line_at(&self, offset: u32) -> Result<&str, SourceMapError> {
        let (line, _) = self.line_column(offset)?;
        let start = self.line_starts[line - 1] as usize;
        let end = self
            .line_starts
            .get(line)
            .map_or(self.text.len(), |next| *next as usize);
        Ok(self.text[start..end].trim_end_matches(['\r', '\n']))
    }

    pub fn validate_span(&self, span: Span) -> Result<(), SourceMapError> {
        if span.file != self.id {
            return Err(SourceMapError::WrongFile {
                expected: self.id,
                actual: span.file,
            });
        }
        let start = span.start as usize;
        let end = span.end as usize;
        if start > end
            || end > self.text.len()
            || !self.text.is_char_boundary(start)
            || !self.text.is_char_boundary(end)
        {
            return Err(SourceMapError::InvalidSpan(span));
        }
        Ok(())
    }
}

/// Session-local source storage. A `BTreeMap` keeps all observable iteration
/// ordered by `FileId`.
#[derive(Debug, Default, Clone)]
pub struct SourceMap {
    files: BTreeMap<FileId, SourceFile>,
    next_id: u32,
}

impl SourceMap {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            files: BTreeMap::new(),
            next_id: 0,
        }
    }

    pub fn add_file(
        &mut self,
        name: impl Into<Arc<str>>,
        text: impl Into<Arc<str>>,
    ) -> Result<FileId, SourceMapError> {
        let id = FileId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(SourceMapError::TooManyFiles)?;
        let text = text.into();
        if text.len() > u32::MAX as usize {
            return Err(SourceMapError::SourceTooLarge);
        }

        let mut line_starts = vec![0];
        for (index, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push((index + 1) as u32);
            }
        }
        let file = SourceFile {
            id,
            name: name.into(),
            text,
            line_starts: line_starts.into(),
        };
        self.files.insert(id, file);
        Ok(id)
    }

    #[must_use]
    pub fn get(&self, id: FileId) -> Option<&SourceFile> {
        self.files.get(&id)
    }

    pub fn file(&self, id: FileId) -> Result<&SourceFile, SourceMapError> {
        self.get(id).ok_or(SourceMapError::UnknownFile(id))
    }

    pub fn span(&self, file: FileId, start: usize, end: usize) -> Result<Span, SourceMapError> {
        let span = Span::new(file, start, end)?;
        self.file(file)?.validate_span(span)?;
        Ok(span)
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &SourceFile> {
        self.files.values()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceMapError {
    TooManyFiles,
    SourceTooLarge,
    UnknownFile(FileId),
    ReversedSpan { start: usize, end: usize },
    WrongFile { expected: FileId, actual: FileId },
    InvalidOffset { file: FileId, offset: usize },
    InvalidSpan(Span),
}

impl fmt::Display for SourceMapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyFiles => formatter.write_str("source map exhausted its file ID space"),
            Self::SourceTooLarge => formatter.write_str("source file exceeds the 4 GiB span limit"),
            Self::UnknownFile(file) => write!(formatter, "unknown source file {}", file.raw()),
            Self::ReversedSpan { start, end } => {
                write!(formatter, "span start {start} is after end {end}")
            }
            Self::WrongFile { expected, actual } => write!(
                formatter,
                "span belongs to file {}, expected file {}",
                actual.raw(),
                expected.raw()
            ),
            Self::InvalidOffset { file, offset } => {
                write!(
                    formatter,
                    "invalid byte offset {offset} in file {}",
                    file.raw()
                )
            }
            Self::InvalidSpan(span) => write!(
                formatter,
                "invalid byte span {}..{} in file {}",
                span.start,
                span.end,
                span.file.raw()
            ),
        }
    }
}

impl std::error::Error for SourceMapError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assigns_stable_ids_and_iterates_in_order() {
        let mut sources = SourceMap::new();
        let first = sources.add_file("a.lpp", "first").unwrap();
        let second = sources.add_file("b.lpp", "second").unwrap();

        assert_eq!(first.raw(), 0);
        assert_eq!(second.raw(), 1);
        assert_eq!(
            sources.iter().map(SourceFile::name).collect::<Vec<_>>(),
            ["a.lpp", "b.lpp"]
        );
    }

    #[test]
    fn reports_unicode_columns_from_byte_offsets() {
        let mut sources = SourceMap::new();
        let file = sources.add_file("unicode.lpp", "αβ\nvalue").unwrap();
        let source = sources.file(file).unwrap();

        assert_eq!(source.line_column("α".len() as u32).unwrap(), (1, 2));
        assert_eq!(source.line_column(5).unwrap(), (2, 1));
        assert!(source.line_column(1).is_err());
    }

    #[test]
    fn rejects_out_of_bounds_and_reversed_spans() {
        let mut sources = SourceMap::new();
        let file = sources.add_file("main.lpp", "abc").unwrap();

        assert!(sources.span(file, 3, 2).is_err());
        assert!(sources.span(file, 0, 4).is_err());
        assert_eq!(sources.span(file, 1, 3).unwrap().start, 1);
    }
}
