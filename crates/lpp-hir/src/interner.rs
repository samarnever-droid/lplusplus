use std::collections::HashMap;
use std::sync::Arc;

use crate::ids::{ArenaId, Symbol};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringInterner {
    by_text: HashMap<Arc<str>, Symbol>,
    by_symbol: Vec<Arc<str>>,
}

impl StringInterner {
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(0)
    }

    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            by_text: HashMap::with_capacity(capacity),
            by_symbol: Vec::with_capacity(capacity),
        }
    }

    pub fn intern(&mut self, text: &str) -> Result<Symbol, InternerExhausted> {
        if let Some(symbol) = self.by_text.get(text) {
            return Ok(*symbol);
        }
        let symbol = Symbol::from_index(self.by_symbol.len()).ok_or(InternerExhausted)?;
        let text: Arc<str> = Arc::from(text);
        self.by_symbol.push(text.clone());
        self.by_text.insert(text, symbol);
        Ok(symbol)
    }

    #[must_use]
    pub fn get(&self, text: &str) -> Option<Symbol> {
        self.by_text.get(text).copied()
    }

    #[must_use]
    pub fn resolve(&self, symbol: Symbol) -> Option<&str> {
        self.by_symbol.get(symbol.index()).map(AsRef::as_ref)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.by_symbol.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_symbol.is_empty()
    }
}

impl Default for StringInterner {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InternerExhausted;

impl std::fmt::Display for InternerExhausted {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("string interner exhausted its 32-bit symbol space")
    }
}

impl std::error::Error for InternerExhausted {}
