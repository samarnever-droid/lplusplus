use std::fmt;
use std::marker::PhantomData;

/// Compact range into an [`IdList`]. Empty ranges never index storage; ranges
/// returned by an empty append retain that append's deterministic position.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct IdRange<I> {
    start: u32,
    len: u32,
    marker: PhantomData<fn(I) -> I>,
}

impl<I> IdRange<I> {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            start: 0,
            len: 0,
            marker: PhantomData,
        }
    }

    #[must_use]
    pub const fn start(self) -> u32 {
        self.start
    }

    #[must_use]
    pub const fn len(self) -> usize {
        self.len as usize
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }
}

impl<I> Default for IdRange<I> {
    fn default() -> Self {
        Self::empty()
    }
}

impl<I> fmt::Debug for IdRange<I> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "IdRange({}..{})",
            self.start,
            self.start.saturating_add(self.len)
        )
    }
}

/// Append-only storage for variable-length HIR operands. Nodes retain an
/// eight-byte range instead of owning one heap allocation per call, tuple,
/// block, or declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdList<I> {
    values: Vec<I>,
}

impl<I: Copy> IdList<I> {
    #[must_use]
    pub const fn new() -> Self {
        Self { values: Vec::new() }
    }

    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            values: Vec::with_capacity(capacity),
        }
    }

    pub fn extend(&mut self, values: &[I]) -> Result<IdRange<I>, IdListExhausted> {
        let start = u32::try_from(self.values.len()).map_err(|_| IdListExhausted)?;
        let len = u32::try_from(values.len()).map_err(|_| IdListExhausted)?;
        self.values
            .len()
            .checked_add(values.len())
            .and_then(|end| u32::try_from(end).ok())
            .ok_or(IdListExhausted)?;
        self.values.extend_from_slice(values);
        Ok(IdRange {
            start,
            len,
            marker: PhantomData,
        })
    }

    #[must_use]
    pub fn get(&self, range: IdRange<I>) -> &[I] {
        let start = range.start as usize;
        &self.values[start..start + range.len as usize]
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    #[must_use]
    pub(crate) fn as_slice(&self) -> &[I] {
        &self.values
    }
}

impl<I: Copy> Default for IdList<I> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdListExhausted;

impl fmt::Display for IdListExhausted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("HIR list storage exhausted its 32-bit range space")
    }
}

impl std::error::Error for IdListExhausted {}
