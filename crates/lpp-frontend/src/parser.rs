use std::sync::Arc;

use lpp_common::{Diagnostic, Span};

use crate::lexer::LexedFile;
use crate::syntax::{
    Attribute, Import, ImportKind, Item, ItemKind, ModulePath, ParsedModule, SyntaxKind,
    SyntaxNode, TokenRange,
};
use crate::token::{Keyword, Token, TokenKind};

pub fn parse(lexed: LexedFile) -> Result<ParsedModule, Vec<Diagnostic>> {
    Parser::new(lexed).run()
}

struct Parser {
    file: lpp_common::FileId,
    source: Arc<str>,
    tokens: Vec<Token>,
    significant: Vec<usize>,
    position: usize,
}

impl Parser {
    fn new(lexed: LexedFile) -> Self {
        let significant = lexed
            .tokens
            .iter()
            .enumerate()
            .filter_map(|(index, token)| (!token.kind.is_trivia()).then_some(index))
            .collect();
        Self {
            file: lexed.file,
            source: lexed.source,
            tokens: lexed.tokens,
            significant,
            position: 0,
        }
    }

    fn run(mut self) -> Result<ParsedModule, Vec<Diagnostic>> {
        self.validate_delimiters()?;
        let syntax = self.parse_block(None, true)?;
        if syntax.is_empty() {
            return self.fail(
                "E1100",
                "source file contains no declarations",
                self.eof_span(),
            );
        }

        let mut items = Vec::new();
        let mut imports = Vec::new();
        let mut attributes = Vec::new();
        for (syntax_index, node) in syntax.iter().enumerate() {
            if node.kind == SyntaxKind::Attribute {
                attributes.push(self.parse_attribute(node)?);
                continue;
            }
            let Some(kind) = item_kind(node.kind) else {
                return self.fail(
                    "E1102",
                    "statements are not allowed at module scope",
                    node.span,
                );
            };
            let public = self
                .line_tokens(node)
                .first()
                .is_some_and(|token| token.kind == TokenKind::Keyword(Keyword::Pub));
            let name = self.item_name(node);
            if matches!(node.kind, SyntaxKind::Import | SyntaxKind::FromImport) {
                imports.push(self.parse_import(node)?);
            }
            let span = Span {
                file: self.file,
                start: attributes
                    .first()
                    .map_or(node.span.start, |attribute| attribute.span.start),
                end: node.span.end,
            };
            items.push(Item {
                kind,
                name,
                public,
                attributes: std::mem::take(&mut attributes),
                syntax_index: u32::try_from(syntax_index)
                    .expect("source token count bounds syntax node count to u32"),
                span,
            });
        }
        if let Some(attribute) = attributes.first() {
            return self.fail(
                "E1103",
                "attribute is not attached to a declaration",
                attribute.span,
            );
        }

        Ok(ParsedModule {
            file: self.file,
            source: self.source,
            tokens: self.tokens,
            items,
            imports,
            syntax,
        })
    }

    fn parse_block(
        &mut self,
        parent: Option<SyntaxKind>,
        top_level: bool,
    ) -> Result<Vec<SyntaxNode>, Vec<Diagnostic>> {
        let mut nodes = Vec::new();
        loop {
            self.skip_newlines();
            match self.peek_kind() {
                Some(TokenKind::Dedent) => {
                    if top_level {
                        return self.fail(
                            "E1106",
                            "unexpected dedent at module scope",
                            self.peek_token().expect("dedent token").span,
                        );
                    }
                    self.position += 1;
                    break;
                }
                Some(TokenKind::Eof) | None => break,
                Some(TokenKind::Indent) => {
                    return self.fail(
                        "E1106",
                        "unexpected indentation",
                        self.peek_token().expect("indent token").span,
                    );
                }
                _ => {}
            }

            let start_position = self.position;
            while !matches!(
                self.peek_kind(),
                Some(TokenKind::Newline | TokenKind::Dedent | TokenKind::Eof) | None
            ) {
                self.position += 1;
            }
            let end_position = self.position;
            if start_position == end_position {
                continue;
            }
            let line = self.significant[start_position..end_position].to_vec();
            let kind = self.classify_line(&line, parent);
            self.validate_line(&line, kind, parent, top_level)?;
            if self.peek_kind() == Some(TokenKind::Newline) {
                self.position += 1;
            }

            let requires_block =
                requires_block(kind, parent) || self.line_ends_with(&line, TokenKind::Colon);
            self.skip_newlines();
            if self.peek_kind() == Some(TokenKind::Indent) && !requires_block {
                return self.fail(
                    "E1106",
                    "unexpected indentation after statement",
                    self.peek_token().expect("indent token").span,
                );
            }
            let children = if self.peek_kind() == Some(TokenKind::Indent) {
                self.position += 1;
                self.parse_block(Some(kind), false)?
            } else {
                if requires_block {
                    let span = self.token_at(*line.last().expect("nonempty line")).span;
                    return self.fail("E1104", "expected an indented block", span);
                }
                Vec::new()
            };
            let first_index = *line.first().expect("nonempty line");
            let last_index = *line.last().expect("nonempty line");
            let mut span = Span {
                file: self.file,
                start: self.token_at(first_index).span.start,
                end: self.token_at(last_index).span.end,
            };
            if let Some(last_child) = children.last() {
                span.end = last_child.span.end;
            }
            nodes.push(SyntaxNode {
                kind,
                span,
                tokens: TokenRange {
                    start: first_index,
                    end: last_index + 1,
                },
                children,
            });
        }
        Ok(nodes)
    }

    fn classify_line(&self, line: &[usize], parent: Option<SyntaxKind>) -> SyntaxKind {
        let mut offset = 0;
        if self.kind_at(line, offset) == Some(TokenKind::Keyword(Keyword::Pub)) {
            offset += 1;
        }
        let first = self.kind_at(line, offset);
        let second = self.kind_at(line, offset + 1);
        match (first, second) {
            (Some(TokenKind::At), _) => SyntaxKind::Attribute,
            (Some(TokenKind::Keyword(Keyword::Async)), Some(TokenKind::Keyword(Keyword::Def)))
            | (Some(TokenKind::Keyword(Keyword::Def)), _) => SyntaxKind::Function,
            (Some(TokenKind::Keyword(Keyword::Struct)), _) => SyntaxKind::Struct,
            (Some(TokenKind::Keyword(Keyword::Enum)), _) => SyntaxKind::Enum,
            (Some(TokenKind::Keyword(Keyword::Trait)), _) => SyntaxKind::Trait,
            (Some(TokenKind::Keyword(Keyword::Impl)), _) => SyntaxKind::Impl,
            (Some(TokenKind::Keyword(Keyword::Extern)), _) => SyntaxKind::Extern,
            (Some(TokenKind::Keyword(Keyword::Const)), _) => SyntaxKind::Const,
            (Some(TokenKind::Keyword(Keyword::Type)), _) => SyntaxKind::TypeAlias,
            (Some(TokenKind::Keyword(Keyword::Import)), _) => SyntaxKind::Import,
            (Some(TokenKind::Keyword(Keyword::From)), _) => SyntaxKind::FromImport,
            (Some(TokenKind::Keyword(Keyword::If)), _) => SyntaxKind::If,
            (Some(TokenKind::Keyword(Keyword::Elif)), _) => SyntaxKind::Elif,
            (Some(TokenKind::Keyword(Keyword::Else)), _) => SyntaxKind::Else,
            (Some(TokenKind::Keyword(Keyword::While)), _) => SyntaxKind::While,
            (Some(TokenKind::Keyword(Keyword::For)), _) => SyntaxKind::For,
            (Some(TokenKind::Keyword(Keyword::Match)), _) => SyntaxKind::Match,
            (Some(TokenKind::Keyword(Keyword::Return)), _) => SyntaxKind::Return,
            (Some(TokenKind::Keyword(Keyword::Break)), _) => SyntaxKind::Break,
            (Some(TokenKind::Keyword(Keyword::Continue)), _) => SyntaxKind::Continue,
            (Some(TokenKind::Keyword(Keyword::Mut)), _) => SyntaxKind::Binding,
            _ if parent == Some(SyntaxKind::Match)
                && self.line_ends_with(line, TokenKind::Colon) =>
            {
                SyntaxKind::MatchArm
            }
            _ if line.iter().any(|index| {
                matches!(
                    self.token_at(*index).kind,
                    TokenKind::Assign
                        | TokenKind::PlusEqual
                        | TokenKind::MinusEqual
                        | TokenKind::StarEqual
                        | TokenKind::SlashEqual
                        | TokenKind::PercentEqual
                )
            }) =>
            {
                SyntaxKind::Binding
            }
            _ if line
                .iter()
                .any(|index| self.token_at(*index).kind == TokenKind::Equal) =>
            {
                SyntaxKind::Assignment
            }
            _ => SyntaxKind::Expression,
        }
    }

    fn validate_line(
        &self,
        line: &[usize],
        kind: SyntaxKind,
        parent: Option<SyntaxKind>,
        top_level: bool,
    ) -> Result<(), Vec<Diagnostic>> {
        if top_level && kind == SyntaxKind::Attribute {
            return self.validate_attribute(line);
        }
        if top_level && item_kind(kind).is_none() {
            return self.fail(
                "E1102",
                "expected a declaration or import at module scope",
                self.line_span(line),
            );
        }
        if matches!(
            kind,
            SyntaxKind::Function
                | SyntaxKind::Struct
                | SyntaxKind::Enum
                | SyntaxKind::Trait
                | SyntaxKind::Const
                | SyntaxKind::TypeAlias
        ) {
            self.validate_declaration_header(line, kind)?;
        }
        if requires_block(kind, parent) && !self.line_ends_with(line, TokenKind::Colon) {
            return self.fail(
                "E1104",
                "block header must end with ':'",
                self.line_span(line),
            );
        }
        if is_control(kind)
            && line
                .iter()
                .position(|index| self.token_at(*index).kind == TokenKind::Colon)
                != Some(line.len() - 1)
        {
            return self.fail(
                "E1104",
                "a block header cannot contain an inline statement after ':'",
                self.line_span(line),
            );
        }
        self.validate_variadic(line)?;
        self.validate_tuple_arity(line)?;

        let has_colon = line
            .iter()
            .any(|index| self.token_at(*index).kind == TokenKind::Colon);
        let has_initializer = line.iter().any(|index| {
            matches!(
                self.token_at(*index).kind,
                TokenKind::Assign | TokenKind::Equal
            )
        });
        if !top_level
            && kind == SyntaxKind::Expression
            && self.kind_at(line, 0) == Some(TokenKind::Identifier)
            && has_colon
            && !has_initializer
            && !self.line_ends_with(line, TokenKind::Colon)
            && !matches!(
                parent,
                Some(SyntaxKind::Struct | SyntaxKind::Enum | SyntaxKind::Extern)
            )
        {
            return self.fail(
                "E1105",
                "typed binding requires '=' or ':=' after its type",
                self.line_span(line),
            );
        }
        Ok(())
    }

    fn validate_attribute(&self, line: &[usize]) -> Result<(), Vec<Diagnostic>> {
        if line.len() < 2 || self.kind_at(line, 1) != Some(TokenKind::Identifier) {
            return self.fail(
                "E1103",
                "expected an attribute name after '@'",
                self.line_span(line),
            );
        }
        Ok(())
    }

    fn validate_declaration_header(
        &self,
        line: &[usize],
        kind: SyntaxKind,
    ) -> Result<(), Vec<Diagnostic>> {
        let keyword = line
            .iter()
            .position(|index| {
                matches!(
                    self.token_at(*index).kind,
                    TokenKind::Keyword(
                        Keyword::Def
                            | Keyword::Struct
                            | Keyword::Enum
                            | Keyword::Trait
                            | Keyword::Const
                            | Keyword::Type
                    )
                )
            })
            .expect("declaration classification requires a declaration keyword");
        let Some(name) = line.get(keyword + 1).map(|index| self.token_at(*index)) else {
            return self.fail("E1107", "expected a declaration name", self.line_span(line));
        };
        if name.kind != TokenKind::Identifier {
            return self.fail("E1107", "expected a declaration name", name.span);
        }
        if kind == SyntaxKind::Function
            && !line[keyword + 2..]
                .iter()
                .any(|index| self.token_at(*index).kind == TokenKind::LeftParen)
        {
            return self.fail(
                "E1107",
                "function declaration requires a parameter list",
                self.line_span(line),
            );
        }
        if matches!(kind, SyntaxKind::Const | SyntaxKind::TypeAlias)
            && !line
                .iter()
                .any(|index| self.token_at(*index).kind == TokenKind::Equal)
        {
            return self.fail(
                "E1107",
                "constant and type declarations require '='",
                self.line_span(line),
            );
        }
        Ok(())
    }

    fn validate_variadic(&self, line: &[usize]) -> Result<(), Vec<Diagnostic>> {
        if let Some(position) = line
            .iter()
            .position(|index| self.token_at(*index).kind == TokenKind::Ellipsis)
            && line[position + 1..]
                .iter()
                .take_while(|index| self.token_at(**index).kind != TokenKind::RightParen)
                .any(|index| self.token_at(*index).kind == TokenKind::Comma)
        {
            return self.fail(
                "E1111",
                "variadic rest parameter must be the final parameter",
                self.token_at(line[position]).span,
            );
        }
        Ok(())
    }

    fn validate_tuple_arity(&self, line: &[usize]) -> Result<(), Vec<Diagnostic>> {
        let mut stack: Vec<(usize, usize, bool)> = Vec::new();
        for (position, index) in line.iter().enumerate() {
            match self.token_at(*index).kind {
                TokenKind::LeftParen => {
                    let previous = position
                        .checked_sub(1)
                        .and_then(|previous| self.kind_at(line, previous));
                    let call = matches!(
                        previous,
                        Some(
                            TokenKind::Identifier
                                | TokenKind::RightParen
                                | TokenKind::RightBracket
                                | TokenKind::Keyword(Keyword::Fn)
                        )
                    );
                    stack.push((position, 0, call));
                }
                TokenKind::Comma => {
                    if let Some((_, commas, _)) = stack.last_mut() {
                        *commas += 1;
                    }
                }
                TokenKind::RightParen => {
                    if let Some((open, commas, call)) = stack.pop()
                        && !call
                        && commas >= 4
                    {
                        return self.fail(
                            "E1110",
                            "tuple expressions require between two and four elements",
                            Span {
                                file: self.file,
                                start: self.token_at(line[open]).span.start,
                                end: self.token_at(*index).span.end,
                            },
                        );
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn validate_delimiters(&self) -> Result<(), Vec<Diagnostic>> {
        let mut stack: Vec<&Token> = Vec::new();
        for token in &self.tokens {
            match token.kind {
                TokenKind::LeftParen | TokenKind::LeftBracket => stack.push(token),
                TokenKind::RightParen | TokenKind::RightBracket => {
                    let matches = stack.pop().is_some_and(|open| {
                        matches!(
                            (open.kind, token.kind),
                            (TokenKind::LeftParen, TokenKind::RightParen)
                                | (TokenKind::LeftBracket, TokenKind::RightBracket)
                        )
                    });
                    if !matches {
                        return self.fail("E1101", "mismatched closing delimiter", token.span);
                    }
                }
                _ => {}
            }
        }
        if let Some(open) = stack.last() {
            return self.fail("E1101", "unclosed delimiter", open.span);
        }
        Ok(())
    }

    fn parse_attribute(&self, node: &SyntaxNode) -> Result<Attribute, Vec<Diagnostic>> {
        let line = self.line_tokens(node);
        let name = self.token_text(line[1]).to_owned();
        Ok(Attribute {
            name,
            span: node.span,
        })
    }

    fn parse_import(&self, node: &SyntaxNode) -> Result<Import, Vec<Diagnostic>> {
        let line = self.line_tokens(node);
        let from = node.kind == SyntaxKind::FromImport;
        let mut position = 1;
        let mut components = Vec::new();
        loop {
            let Some(token) = line.get(position) else {
                return self.fail("E1120", "expected a module name", node.span);
            };
            if token.kind != TokenKind::Identifier {
                return self.fail("E1120", "expected a module path component", token.span);
            }
            components.push(self.token_text(token).to_owned());
            position += 1;
            if line.get(position).map(|token| token.kind) != Some(TokenKind::Dot) {
                break;
            }
            position += 1;
        }
        let path = ModulePath::new(components).expect("parser collected a nonempty module path");
        let kind = if from {
            if line.get(position).map(|token| token.kind)
                != Some(TokenKind::Keyword(Keyword::Import))
            {
                return self.fail("E1120", "expected 'import' after module path", node.span);
            }
            position += 1;
            let mut names = Vec::new();
            loop {
                let Some(token) = line.get(position) else {
                    return self.fail("E1120", "expected a name in import list", node.span);
                };
                if token.kind != TokenKind::Identifier {
                    return self.fail("E1120", "expected a name in import list", token.span);
                }
                names.push(self.token_text(token).to_owned());
                position += 1;
                if line.get(position).map(|token| token.kind) != Some(TokenKind::Comma) {
                    break;
                }
                position += 1;
            }
            ImportKind::Selective { names }
        } else {
            let alias = if line.get(position).map(|token| token.kind)
                == Some(TokenKind::Keyword(Keyword::As))
            {
                position += 1;
                let Some(token) = line.get(position) else {
                    return self.fail("E1120", "expected an alias after 'as'", node.span);
                };
                if token.kind != TokenKind::Identifier {
                    return self.fail("E1120", "expected an alias after 'as'", token.span);
                }
                position += 1;
                Some(self.token_text(token).to_owned())
            } else {
                None
            };
            ImportKind::Module { alias }
        };
        if position != line.len() {
            return self.fail(
                "E1120",
                "unexpected token after import",
                line[position].span,
            );
        }
        Ok(Import {
            path,
            kind,
            span: node.span,
        })
    }

    fn item_name(&self, node: &SyntaxNode) -> Option<String> {
        let line = self.line_tokens(node);
        let keyword = line.iter().position(|token| {
            matches!(
                token.kind,
                TokenKind::Keyword(
                    Keyword::Def
                        | Keyword::Struct
                        | Keyword::Enum
                        | Keyword::Trait
                        | Keyword::Const
                        | Keyword::Type
                )
            )
        })?;
        line.get(keyword + 1)
            .filter(|token| token.kind == TokenKind::Identifier)
            .map(|token| self.token_text(token).to_owned())
    }

    fn line_tokens(&self, node: &SyntaxNode) -> Vec<&Token> {
        self.tokens[node.tokens.start..node.tokens.end]
            .iter()
            .filter(|token| !token.kind.is_trivia())
            .collect()
    }

    fn line_span(&self, line: &[usize]) -> Span {
        Span {
            file: self.file,
            start: self.token_at(line[0]).span.start,
            end: self.token_at(*line.last().expect("nonempty line")).span.end,
        }
    }

    fn line_ends_with(&self, line: &[usize], kind: TokenKind) -> bool {
        line.last()
            .is_some_and(|index| self.token_at(*index).kind == kind)
    }

    fn kind_at(&self, line: &[usize], offset: usize) -> Option<TokenKind> {
        line.get(offset).map(|index| self.token_at(*index).kind)
    }

    fn skip_newlines(&mut self) {
        while self.peek_kind() == Some(TokenKind::Newline) {
            self.position += 1;
        }
    }

    fn peek_kind(&self) -> Option<TokenKind> {
        self.peek_token().map(|token| token.kind)
    }

    fn peek_token(&self) -> Option<&Token> {
        self.significant
            .get(self.position)
            .map(|index| self.token_at(*index))
    }

    fn token_at(&self, index: usize) -> &Token {
        &self.tokens[index]
    }

    fn token_text(&self, token: &Token) -> &str {
        token.text(&self.source)
    }

    fn eof_span(&self) -> Span {
        self.tokens.last().map_or(
            Span {
                file: self.file,
                start: 0,
                end: 0,
            },
            |token| token.span,
        )
    }

    fn fail<T>(
        &self,
        code: &str,
        message: impl Into<String>,
        span: Span,
    ) -> Result<T, Vec<Diagnostic>> {
        Err(vec![
            Diagnostic::error(code, message)
                .expect("frontend diagnostic code is valid")
                .with_primary_span(span),
        ])
    }
}

fn item_kind(kind: SyntaxKind) -> Option<ItemKind> {
    Some(match kind {
        SyntaxKind::Function => ItemKind::Function,
        SyntaxKind::Struct => ItemKind::Struct,
        SyntaxKind::Enum => ItemKind::Enum,
        SyntaxKind::Trait => ItemKind::Trait,
        SyntaxKind::Impl => ItemKind::Impl,
        SyntaxKind::Extern => ItemKind::Extern,
        SyntaxKind::Const => ItemKind::Const,
        SyntaxKind::TypeAlias => ItemKind::TypeAlias,
        SyntaxKind::Import | SyntaxKind::FromImport => ItemKind::Import,
        _ => return None,
    })
}

fn requires_block(kind: SyntaxKind, parent: Option<SyntaxKind>) -> bool {
    if kind == SyntaxKind::Function
        && matches!(parent, Some(SyntaxKind::Trait | SyntaxKind::Extern))
    {
        return false;
    }
    matches!(
        kind,
        SyntaxKind::Function
            | SyntaxKind::Struct
            | SyntaxKind::Enum
            | SyntaxKind::Trait
            | SyntaxKind::Impl
            | SyntaxKind::Extern
            | SyntaxKind::If
            | SyntaxKind::Elif
            | SyntaxKind::Else
            | SyntaxKind::While
            | SyntaxKind::For
            | SyntaxKind::Match
            | SyntaxKind::MatchArm
    )
}

const fn is_control(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::If
            | SyntaxKind::Elif
            | SyntaxKind::Else
            | SyntaxKind::While
            | SyntaxKind::For
            | SyntaxKind::Match
            | SyntaxKind::MatchArm
    )
}
