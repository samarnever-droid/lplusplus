use std::fmt;
use std::marker::PhantomData;
use std::ops::{Index, IndexMut};

use crate::ids::ArenaId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arena<I, T> {
    values: Vec<T>,
    marker: PhantomData<fn(I) -> I>,
}

impl<I, T> Arena<I, T> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            values: Vec::new(),
            marker: PhantomData,
        }
    }

    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            values: Vec::with_capacity(capacity),
            marker: PhantomData,
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &T> {
        self.values.iter()
    }

    pub fn iter_mut(&mut self) -> impl ExactSizeIterator<Item = &mut T> {
        self.values.iter_mut()
    }
}

impl<I: ArenaId, T> Arena<I, T> {
    pub fn alloc(&mut self, value: T) -> Result<I, ArenaExhausted> {
        let id = I::from_index(self.values.len()).ok_or(ArenaExhausted)?;
        self.values.push(value);
        Ok(id)
    }

    pub fn enumerate(&self) -> impl ExactSizeIterator<Item = (I, &T)> {
        self.values.iter().enumerate().map(|(index, value)| {
            (
                I::from_index(index).expect("allocated arena indices fit their ID type"),
                value,
            )
        })
    }

    #[must_use]
    pub fn get(&self, id: I) -> Option<&T> {
        self.values.get(id.index())
    }

    pub fn get_mut(&mut self, id: I) -> Option<&mut T> {
        self.values.get_mut(id.index())
    }
}

impl<I: ArenaId, T> Default for Arena<I, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<I: ArenaId, T> Index<I> for Arena<I, T> {
    type Output = T;

    fn index(&self, id: I) -> &Self::Output {
        &self.values[id.index()]
    }
}

impl<I: ArenaId, T> IndexMut<I> for Arena<I, T> {
    fn index_mut(&mut self, id: I) -> &mut Self::Output {
        &mut self.values[id.index()]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArenaExhausted;

impl fmt::Display for ArenaExhausted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("typed arena exhausted its 32-bit ID space")
    }
}

impl std::error::Error for ArenaExhausted {}
