use std::sync::Arc;

use lpp_common::{FileId, Span};

use crate::Token;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenRange {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxKind {
    Attribute,
    Function,
    Struct,
    Enum,
    Trait,
    Impl,
    Extern,
    Const,
    TypeAlias,
    Import,
    FromImport,
    If,
    Elif,
    Else,
    While,
    For,
    Match,
    MatchArm,
    Return,
    Break,
    Continue,
    Binding,
    Assignment,
    Expression,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxNode {
    pub kind: SyntaxKind,
    pub span: Span,
    pub tokens: TokenRange,
    pub children: Vec<SyntaxNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub kind: ItemKind,
    pub name: Option<String>,
    pub public: bool,
    pub attributes: Vec<Attribute>,
    pub syntax_index: u32,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Function,
    Struct,
    Enum,
    Trait,
    Impl,
    Extern,
    Const,
    TypeAlias,
    Import,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportKind {
    Module { alias: Option<String> },
    Selective { names: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    pub path: ModulePath,
    pub kind: ImportKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModulePath {
    components: Vec<String>,
}

impl ModulePath {
    pub fn new(components: Vec<String>) -> Result<Self, InvalidModulePath> {
        if components.is_empty() || components.iter().any(String::is_empty) {
            return Err(InvalidModulePath);
        }
        Ok(Self { components })
    }

    #[must_use]
    pub fn components(&self) -> &[String] {
        &self.components
    }

    #[must_use]
    pub fn slash_path(&self) -> String {
        self.components.join("/")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidModulePath;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedModule {
    pub file: FileId,
    pub source: Arc<str>,
    pub tokens: Vec<Token>,
    pub items: Vec<Item>,
    pub imports: Vec<Import>,
    pub syntax: Vec<SyntaxNode>,
}

impl ParsedModule {
    #[must_use]
    pub fn reconstruct(&self) -> &str {
        &self.source
    }

    pub fn remap_file(&mut self, file: FileId) {
        self.file = file;
        for token in &mut self.tokens {
            token.span.file = file;
        }
        for item in &mut self.items {
            item.span.file = file;
            for attribute in &mut item.attributes {
                attribute.span.file = file;
            }
        }
        for import in &mut self.imports {
            import.span.file = file;
        }
        for node in &mut self.syntax {
            remap_node(node, file);
        }
    }
}

fn remap_node(node: &mut SyntaxNode, file: FileId) {
    node.span.file = file;
    for child in &mut node.children {
        remap_node(child, file);
    }
}
