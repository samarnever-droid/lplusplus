use std::fmt;
use std::marker::PhantomData;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListRange<T> {
    start: u32,
    len: u32,
    marker: PhantomData<fn(T) -> T>,
}

impl<T> ListRange<T> {
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

impl<T> Default for ListRange<T> {
    fn default() -> Self {
        Self::empty()
    }
}

impl<T> fmt::Debug for ListRange<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "ListRange({}..{})",
            self.start,
            self.start.saturating_add(self.len),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListStore<T> {
    values: Vec<T>,
}

impl<T: Copy> ListStore<T> {
    pub(crate) const fn new() -> Self {
        Self { values: Vec::new() }
    }

    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            values: Vec::with_capacity(capacity),
        }
    }

    pub(crate) fn extend(&mut self, values: &[T]) -> Result<ListRange<T>, StorageExhausted> {
        let start = u32::try_from(self.values.len()).map_err(|_| StorageExhausted)?;
        let len = u32::try_from(values.len()).map_err(|_| StorageExhausted)?;
        self.values
            .len()
            .checked_add(values.len())
            .and_then(|end| u32::try_from(end).ok())
            .ok_or(StorageExhausted)?;
        self.values.extend_from_slice(values);
        Ok(ListRange {
            start,
            len,
            marker: PhantomData,
        })
    }

    pub(crate) fn get(&self, range: ListRange<T>) -> &[T] {
        let start = range.start as usize;
        &self.values[start..start + range.len as usize]
    }

    pub(crate) fn get_mut(&mut self, range: ListRange<T>) -> &mut [T] {
        let start = range.start as usize;
        &mut self.values[start..start + range.len as usize]
    }

    pub(crate) fn len(&self) -> usize {
        self.values.len()
    }
}

impl<T: Copy> Default for ListStore<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageExhausted;

impl fmt::Display for StorageExhausted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MIR storage exhausted its 32-bit range space")
    }
}

impl std::error::Error for StorageExhausted {}
