use std::sync::Arc;

use lpp_common::{Diagnostic, FileId, Span};

use crate::literal::{LiteralError, scan_character, scan_string};
use crate::token::{Keyword, Token, TokenKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexedFile {
    pub file: FileId,
    pub source: Arc<str>,
    pub tokens: Vec<Token>,
}

impl LexedFile {
    #[must_use]
    pub fn reconstruct(&self) -> &str {
        &self.source
    }
}

pub fn lex(file: FileId, input: &str) -> Result<LexedFile, Vec<Diagnostic>> {
    lex_shared(file, Arc::from(input))
}

pub fn lex_shared(file: FileId, source: Arc<str>) -> Result<LexedFile, Vec<Diagnostic>> {
    if u32::try_from(source.len()).is_err() {
        return Err(vec![
            Diagnostic::error("E1000", "source file exceeds the 4 GiB span limit")
                .expect("frontend diagnostic code is valid")
                .with_primary_span(Span {
                    file,
                    start: 0,
                    end: 0,
                }),
        ]);
    }
    let tokens = Lexer::new(file, &source).run()?;
    Ok(LexedFile {
        file,
        source,
        tokens,
    })
}

struct Lexer<'a> {
    file: FileId,
    input: &'a str,
    offset: usize,
    at_line_start: bool,
    delimiter_depth: usize,
    indent_stack: Vec<usize>,
    tokens: Vec<Token>,
}

impl<'a> Lexer<'a> {
    fn new(file: FileId, input: &'a str) -> Self {
        Self {
            file,
            input,
            offset: 0,
            at_line_start: true,
            delimiter_depth: 0,
            indent_stack: vec![0],
            tokens: Vec::new(),
        }
    }

    fn run(mut self) -> Result<Vec<Token>, Vec<Diagnostic>> {
        if self.input.starts_with('\u{feff}') {
            let end = '\u{feff}'.len_utf8();
            self.emit(TokenKind::Bom, 0, end);
            self.offset = end;
        }

        while self.offset < self.input.len() {
            if self.at_line_start {
                self.scan_indentation()?;
                if self.offset >= self.input.len() {
                    break;
                }
            }
            self.scan_token()?;
        }

        while self.indent_stack.len() > 1 {
            self.indent_stack.pop();
            self.synthetic(TokenKind::Dedent, self.offset);
        }
        self.synthetic(TokenKind::Eof, self.offset);
        Ok(self.tokens)
    }

    fn scan_indentation(&mut self) -> Result<(), Vec<Diagnostic>> {
        self.at_line_start = false;
        let start = self.offset;
        while self.peek() == Some(' ') {
            self.bump();
        }
        if self.peek() == Some('\t') {
            return self.fail(
                "E1002",
                "tabs are not allowed for indentation; use spaces",
                self.offset,
                self.offset + 1,
            );
        }
        if self.offset > start {
            self.emit(TokenKind::Whitespace, start, self.offset);
        }

        let blank_or_comment = matches!(self.peek(), None | Some('\r' | '\n' | '#'))
            || self.input[self.offset..].starts_with("//");
        if self.delimiter_depth > 0 || blank_or_comment {
            return Ok(());
        }

        let width = self.offset - start;
        let current = *self
            .indent_stack
            .last()
            .expect("indent stack is never empty");
        if width > current {
            self.indent_stack.push(width);
            self.synthetic(TokenKind::Indent, self.offset);
        } else if width < current {
            while self.indent_stack.last().is_some_and(|level| *level > width) {
                self.indent_stack.pop();
                self.synthetic(TokenKind::Dedent, self.offset);
            }
            if self.indent_stack.last().copied() != Some(width) {
                return self.fail(
                    "E1003",
                    "inconsistent indentation level",
                    start,
                    self.offset,
                );
            }
        }
        Ok(())
    }

    fn scan_token(&mut self) -> Result<(), Vec<Diagnostic>> {
        let start = self.offset;
        let character = self.bump().expect("lexer offset is inside source text");
        match character {
            ' ' => {
                while self.peek() == Some(' ') {
                    self.bump();
                }
                self.emit(TokenKind::Whitespace, start, self.offset);
            }
            '\t' => {
                return self.fail("E1001", "unexpected tab character", start, self.offset);
            }
            '\r' | '\n' => self.scan_newline(start, character),
            '#' => self.scan_comment(start),
            '/' if self.peek() == Some('/') => {
                self.bump();
                self.scan_comment(start);
            }
            '\'' => self.scan_character_token(start)?,
            '"' => self.scan_string_token(start, false)?,
            'f' if self.peek() == Some('"') => self.scan_string_token(start, true)?,
            value if value.is_ascii_digit() => self.scan_number(start, value)?,
            value if value.is_alphabetic() || value == '_' => self.scan_identifier(start),
            '(' => self.open(TokenKind::LeftParen, start),
            '[' => self.open(TokenKind::LeftBracket, start),
            ')' => self.close(TokenKind::RightParen, start),
            ']' => self.close(TokenKind::RightBracket, start),
            ':' => self.either(start, '=', TokenKind::Assign, TokenKind::Colon),
            '=' => self.either(start, '=', TokenKind::EqualEqual, TokenKind::Equal),
            '!' => self.either(start, '=', TokenKind::NotEqual, TokenKind::Not),
            '+' => self.either(start, '=', TokenKind::PlusEqual, TokenKind::Plus),
            '%' => self.either(start, '=', TokenKind::PercentEqual, TokenKind::Percent),
            '&' => self.either(start, '&', TokenKind::LogicalAnd, TokenKind::BitAnd),
            '|' => self.either(start, '|', TokenKind::LogicalOr, TokenKind::BitOr),
            '-' => self.multi(
                start,
                '>',
                '=',
                TokenKind::Arrow,
                TokenKind::MinusEqual,
                TokenKind::Minus,
            ),
            '*' => self.either(start, '=', TokenKind::StarEqual, TokenKind::Star),
            '/' => self.either(start, '=', TokenKind::SlashEqual, TokenKind::Slash),
            '<' => self.multi(
                start,
                '=',
                '<',
                TokenKind::LessEqual,
                TokenKind::ShiftLeft,
                TokenKind::Less,
            ),
            '>' => self.multi(
                start,
                '=',
                '>',
                TokenKind::GreaterEqual,
                TokenKind::ShiftRight,
                TokenKind::Greater,
            ),
            '^' => self.emit(TokenKind::BitXor, start, self.offset),
            '?' => self.emit(TokenKind::Question, start, self.offset),
            ',' => self.emit(TokenKind::Comma, start, self.offset),
            '@' => self.emit(TokenKind::At, start, self.offset),
            '.' => self.scan_dot(start)?,
            _ => {
                return self.fail(
                    "E1001",
                    format!("unexpected character: {character}"),
                    start,
                    self.offset,
                );
            }
        }
        Ok(())
    }

    fn scan_newline(&mut self, start: usize, first: char) {
        if first == '\r' && self.peek() == Some('\n') {
            self.bump();
        }
        let kind = if self.delimiter_depth == 0 {
            self.at_line_start = true;
            TokenKind::Newline
        } else {
            TokenKind::SoftNewline
        };
        self.emit(kind, start, self.offset);
    }

    fn scan_comment(&mut self, start: usize) {
        while !matches!(self.peek(), None | Some('\r' | '\n')) {
            self.bump();
        }
        self.emit(TokenKind::Comment, start, self.offset);
    }

    fn scan_character_token(&mut self, start: usize) -> Result<(), Vec<Diagnostic>> {
        match scan_character(self.input, start) {
            Ok(end) => {
                self.offset = end;
                self.emit(TokenKind::Character, start, end);
                Ok(())
            }
            Err(error) => self.literal_error(error),
        }
    }

    fn scan_string_token(&mut self, start: usize, formatted: bool) -> Result<(), Vec<Diagnostic>> {
        match scan_string(self.input, start, formatted) {
            Ok(end) => {
                self.offset = end;
                self.emit(
                    if formatted {
                        TokenKind::FString
                    } else {
                        TokenKind::String
                    },
                    start,
                    end,
                );
                Ok(())
            }
            Err(error) => self.literal_error(error),
        }
    }

    fn literal_error(&self, error: LiteralError) -> Result<(), Vec<Diagnostic>> {
        self.fail(
            "E1004",
            error.message,
            error.offset,
            (error.offset + 1).min(self.input.len()),
        )
    }

    fn scan_number(&mut self, start: usize, first: char) -> Result<(), Vec<Diagnostic>> {
        if first == '0' && matches!(self.peek(), Some('x' | 'X' | 'b' | 'B')) {
            let radix_marker = self.bump().expect("checked radix marker");
            let radix = if matches!(radix_marker, 'x' | 'X') {
                16
            } else {
                2
            };
            let digits_start = self.offset;
            while self.peek().is_some_and(|value| {
                value == '_'
                    || (radix == 16 && value.is_ascii_hexdigit())
                    || (radix == 2 && matches!(value, '0' | '1'))
            }) {
                self.bump();
            }
            let digits = self.input[digits_start..self.offset].replace('_', "");
            if digits.is_empty() || i64::from_str_radix(&digits, radix).is_err() {
                return self.fail(
                    "E1005",
                    "integer literal is out of range",
                    start,
                    self.offset,
                );
            }
            self.emit(TokenKind::Integer, start, self.offset);
            return Ok(());
        }

        let mut float = false;
        while let Some(value) = self.peek() {
            if value.is_ascii_digit() || value == '_' {
                self.bump();
            } else if value == '.'
                && !float
                && self.peek_second().is_some_and(|next| next.is_ascii_digit())
            {
                float = true;
                self.bump();
            } else {
                break;
            }
        }
        let number = self.input[start..self.offset].replace('_', "");
        let valid = if float {
            number.parse::<f64>().is_ok()
        } else {
            number.parse::<i64>().is_ok()
        };
        if !valid {
            return self.fail(
                "E1005",
                "numeric literal is out of range",
                start,
                self.offset,
            );
        }
        self.emit(
            if float {
                TokenKind::Float
            } else {
                TokenKind::Integer
            },
            start,
            self.offset,
        );
        Ok(())
    }

    fn scan_identifier(&mut self, start: usize) {
        while self
            .peek()
            .is_some_and(|value| value.is_alphanumeric() || value == '_')
        {
            self.bump();
        }
        let text = &self.input[start..self.offset];
        let kind = match text {
            "true" | "false" => TokenKind::Bool,
            "and" => TokenKind::LogicalAnd,
            "or" => TokenKind::LogicalOr,
            "not" => TokenKind::Not,
            _ => Keyword::from_ident(text).map_or(TokenKind::Identifier, TokenKind::Keyword),
        };
        self.emit(kind, start, self.offset);
    }

    fn scan_dot(&mut self, start: usize) -> Result<(), Vec<Diagnostic>> {
        if self.input[self.offset..].starts_with("..") {
            self.offset += 2;
            self.emit(TokenKind::Ellipsis, start, self.offset);
            Ok(())
        } else if self.peek() == Some('.') {
            self.fail(
                "E1001",
                "expected a third '.' in variadic marker",
                start,
                self.offset + 1,
            )
        } else {
            self.emit(TokenKind::Dot, start, self.offset);
            Ok(())
        }
    }

    fn open(&mut self, kind: TokenKind, start: usize) {
        self.delimiter_depth += 1;
        self.emit(kind, start, self.offset);
    }

    fn close(&mut self, kind: TokenKind, start: usize) {
        self.delimiter_depth = self.delimiter_depth.saturating_sub(1);
        self.emit(kind, start, self.offset);
    }

    fn either(&mut self, start: usize, next: char, combined: TokenKind, single: TokenKind) {
        if self.peek() == Some(next) {
            self.bump();
            self.emit(combined, start, self.offset);
        } else {
            self.emit(single, start, self.offset);
        }
    }

    fn multi(
        &mut self,
        start: usize,
        first: char,
        second: char,
        first_kind: TokenKind,
        second_kind: TokenKind,
        single: TokenKind,
    ) {
        if self.peek() == Some(first) {
            self.bump();
            self.emit(first_kind, start, self.offset);
        } else if self.peek() == Some(second) {
            self.bump();
            self.emit(second_kind, start, self.offset);
        } else {
            self.emit(single, start, self.offset);
        }
    }

    fn emit(&mut self, kind: TokenKind, start: usize, end: usize) {
        self.tokens.push(Token {
            kind,
            span: Span {
                file: self.file,
                start: start as u32,
                end: end as u32,
            },
        });
    }

    fn synthetic(&mut self, kind: TokenKind, offset: usize) {
        self.tokens.push(Token {
            kind,
            span: Span {
                file: self.file,
                start: offset as u32,
                end: offset as u32,
            },
        });
    }

    fn fail<T>(
        &self,
        code: &str,
        message: impl Into<String>,
        start: usize,
        end: usize,
    ) -> Result<T, Vec<Diagnostic>> {
        let diagnostic = Diagnostic::error(code, message)
            .expect("frontend diagnostic code is valid")
            .with_primary_span(Span {
                file: self.file,
                start: start as u32,
                end: end as u32,
            });
        Err(vec![diagnostic])
    }

    fn peek(&self) -> Option<char> {
        self.input[self.offset..].chars().next()
    }

    fn peek_second(&self) -> Option<char> {
        self.input[self.offset..].chars().nth(1)
    }

    fn bump(&mut self) -> Option<char> {
        let value = self.peek()?;
        self.offset += value.len_utf8();
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use lpp_common::{FileId, SourceMap};

    use super::*;

    #[test]
    fn token_stream_is_lossless_and_span_complete() {
        let input = "\u{feff}def main():\r\n    # note\r\n    print(f\"hi {name}\")\r\n";
        let lexed = lex(FileId::from_raw(0), input).unwrap();
        assert_eq!(lexed.reconstruct(), input);
        let rebuilt = lexed
            .tokens
            .iter()
            .filter(|token| !token.is_synthetic())
            .map(|token| token.text(&lexed.source))
            .collect::<String>();
        assert_eq!(rebuilt, input);
        for token in lexed.tokens.iter().filter(|token| !token.is_synthetic()) {
            assert_eq!(
                &input[token.span.start as usize..token.span.end as usize],
                token.text(&lexed.source)
            );
        }
    }

    #[test]
    fn reports_structured_unicode_locations() {
        let input = "def main():\n    π := {\n";
        let mut sources = SourceMap::new();
        let file = sources.add_file("main.lpp", input).unwrap();
        let diagnostic = &lex(file, input).unwrap_err()[0];
        assert_eq!(diagnostic.code.as_str(), "E1001");
        assert!(diagnostic.render_human(&sources).contains("main.lpp:2:10"));
    }

    #[test]
    fn emits_balanced_indentation() {
        let input = "def main():\n    if true:\n        print(1)\n    print(2)\n";
        let tokens = lex(FileId::from_raw(0), input).unwrap().tokens;
        assert_eq!(
            tokens
                .iter()
                .filter(|token| token.kind == TokenKind::Indent)
                .count(),
            2
        );
        assert_eq!(
            tokens
                .iter()
                .filter(|token| token.kind == TokenKind::Dedent)
                .count(),
            2
        );
    }

    #[test]
    fn maps_word_and_symbol_boolean_operators_to_the_same_kinds() {
        let tokens = lex(FileId::from_raw(0), "a and b || not c && !d\n")
            .unwrap()
            .tokens;
        let operators = tokens
            .iter()
            .filter_map(|token| {
                matches!(
                    token.kind,
                    TokenKind::LogicalAnd | TokenKind::LogicalOr | TokenKind::Not
                )
                .then_some(token.kind)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            operators,
            [
                TokenKind::LogicalAnd,
                TokenKind::LogicalOr,
                TokenKind::Not,
                TokenKind::LogicalAnd,
                TokenKind::Not,
            ]
        );
    }
}
