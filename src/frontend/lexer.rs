#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Keywords
    Def,
    Return,
    Mut,
    Struct,
    Enum,
    Match,
    Fn,
    Spawn,
    Import,
    From,
    As,
    Pub,

    If,
    Else,
    While,
    For,
    In,
    Break,
    Continue,

    // Identifiers and Literals
    Ident(String),
    Int(i64),
    StringLit(String),
    BoolLit(bool),
    FloatLit(f64),

    // Operators and Punctuation
    Assign,    // :=
    Equal,     // =
    EqEq,      // ==
    NotEq,     // !=
    Less,      // <
    Greater,   // >
    LessEq,    // <=
    GreaterEq, // >=
    And,       // &&
    Or,        // ||
    Colon,     // :
    Arrow,     // ->
    Plus,      // +
    Minus,     // -
    Star,      // *
    Slash,     // /
    Percent,   // %
    Question,  // ?
    Not,       // !
    LParen,    // (
    RParen,    // )
    LBracket,  // [
    RBracket,  // ]
    Comma,     // ,
    Dot,       // .

    // Significant Whitespace
    Newline,
    Indent,
    Dedent,

    // End of File
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpannedToken {
    pub token: Token,
    pub line: usize,
    pub col: usize,
}

use std::collections::VecDeque;

pub struct Lexer<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    line: usize,
    col: usize,
    indent_stack: Vec<usize>,
    pending_tokens: VecDeque<SpannedToken>,
    at_line_start: bool,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            chars: input.chars().peekable(),
            line: 1,
            col: 1,
            indent_stack: vec![0],
            pending_tokens: VecDeque::new(),
            at_line_start: true,
        }
    }

    fn peek_c(&mut self) -> Option<char> {
        self.chars.peek().copied()
    }

    fn next_c(&mut self) -> Option<char> {
        let ch = self.chars.next()?;
        if ch == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(ch)
    }

    pub fn tokenize(&mut self) -> Result<Vec<SpannedToken>, String> {
        let mut tokens = Vec::new();

        loop {
            if let Some(tok) = self.pending_tokens.pop_front() {
                tokens.push(tok);
                continue;
            }

            let tok_line = self.line;
            let tok_col = self.col;

            if self.at_line_start {
                self.at_line_start = false;
                let mut spaces = 0;

                while let Some(c) = self.peek_c() {
                    if c == ' ' {
                        spaces += 1;
                        self.next_c();
                    } else if c == '\t' {
                        return Err(format!(
                            "[line {}:col {}] Lexer error: Tabs are not allowed for indentation. Use spaces.",
                            self.line, self.col
                        ));
                    } else if c == '\n' || c == '\r' {
                        // Empty line, ignore indentation
                        break;
                    } else {
                        break;
                    }
                }

                if let Some(c) = self.peek_c() {
                    if c != '\n' && c != '\r' {
                        let current_indent = *self.indent_stack.last().unwrap_or(&0);
                        if spaces > current_indent {
                            self.indent_stack.push(spaces);
                            tokens.push(SpannedToken {
                                token: Token::Indent,
                                line: tok_line,
                                col: tok_col,
                            });
                        } else if spaces < current_indent {
                            while let Some(&top) = self.indent_stack.last() {
                                if top > spaces {
                                    self.indent_stack.pop();
                                    tokens.push(SpannedToken {
                                        token: Token::Dedent,
                                        line: tok_line,
                                        col: tok_col,
                                    });
                                } else if top == spaces {
                                    break;
                                } else {
                                    return Err(format!(
                                        "[line {}:col {}] Lexer error: Inconsistent indentation level.",
                                        self.line, self.col
                                    ));
                                }
                            }
                        }
                    }
                }
            }

            let start_line = self.line;
            let start_col = self.col;
            let c = match self.next_c() {
                Some(c) => c,
                None => {
                    while self.indent_stack.len() > 1 {
                        self.indent_stack.pop();
                        tokens.push(SpannedToken {
                            token: Token::Dedent,
                            line: start_line,
                            col: start_col,
                        });
                    }
                    tokens.push(SpannedToken {
                        token: Token::Eof,
                        line: start_line,
                        col: start_col,
                    });
                    break;
                }
            };

            let mk_token = |t: Token| SpannedToken {
                token: t,
                line: start_line,
                col: start_col,
            };

            match c {
                ' ' | '\r' => continue,
                '\n' => {
                    tokens.push(mk_token(Token::Newline));
                    self.at_line_start = true;
                }
                ':' => {
                    if self.peek_c() == Some('=') {
                        self.next_c();
                        tokens.push(mk_token(Token::Assign));
                    } else {
                        tokens.push(mk_token(Token::Colon));
                    }
                }
                '-' => {
                    if self.peek_c() == Some('>') {
                        self.next_c();
                        tokens.push(mk_token(Token::Arrow));
                    } else {
                        tokens.push(mk_token(Token::Minus));
                    }
                }
                '=' => {
                    if self.peek_c() == Some('=') {
                        self.next_c();
                        tokens.push(mk_token(Token::EqEq));
                    } else {
                        tokens.push(mk_token(Token::Equal));
                    }
                }
                '<' => {
                    if self.peek_c() == Some('=') {
                        self.next_c();
                        tokens.push(mk_token(Token::LessEq));
                    } else {
                        tokens.push(mk_token(Token::Less));
                    }
                }
                '>' => {
                    if self.peek_c() == Some('=') {
                        self.next_c();
                        tokens.push(mk_token(Token::GreaterEq));
                    } else {
                        tokens.push(mk_token(Token::Greater));
                    }
                }
                '!' => {
                    if self.peek_c() == Some('=') {
                        self.next_c();
                        tokens.push(mk_token(Token::NotEq));
                    } else {
                        tokens.push(mk_token(Token::Not));
                    }
                }
                '+' => tokens.push(mk_token(Token::Plus)),
                '*' => tokens.push(mk_token(Token::Star)),
                '/' => tokens.push(mk_token(Token::Slash)),
                '%' => tokens.push(mk_token(Token::Percent)),
                '?' => tokens.push(mk_token(Token::Question)),
                '&' => {
                    if self.peek_c() == Some('&') {
                        self.next_c();
                        tokens.push(mk_token(Token::And));
                    }
                }
                '|' => {
                    if self.peek_c() == Some('|') {
                        self.next_c();
                        tokens.push(mk_token(Token::Or));
                    }
                }
                '#' => {
                    while let Some(next_c) = self.peek_c() {
                        if next_c == '\n' || next_c == '\r' {
                            break;
                        }
                        self.next_c();
                    }
                }
                '(' => tokens.push(mk_token(Token::LParen)),
                ')' => tokens.push(mk_token(Token::RParen)),
                '[' => tokens.push(mk_token(Token::LBracket)),
                ']' => tokens.push(mk_token(Token::RBracket)),
                ',' => tokens.push(mk_token(Token::Comma)),
                '.' => tokens.push(mk_token(Token::Dot)),
                '"' => {
                    let mut s = String::new();
                    let mut terminated = false;
                    while let Some(ch) = self.next_c() {
                        if ch == '"' {
                            terminated = true;
                            break;
                        }
                        if ch == '\\' {
                            if let Some(escaped) = self.next_c() {
                                match escaped {
                                    'n' => s.push('\n'),
                                    'r' => s.push('\r'),
                                    't' => s.push('\t'),
                                    '"' => s.push('"'),
                                    '\\' => s.push('\\'),
                                    other => {
                                        s.push('\\');
                                        s.push(other);
                                    }
                                }
                            } else {
                                return Err(format!(
                                    "[line {}:col {}] Lexer error: Unterminated string escape",
                                    self.line, self.col
                                ));
                            }
                        } else {
                            s.push(ch);
                        }
                    }
                    if !terminated {
                        return Err(format!(
                            "[line {}:col {}] Lexer error: Unterminated string literal",
                            start_line, start_col
                        ));
                    }
                    tokens.push(mk_token(Token::StringLit(s)));
                }
                _ if c.is_ascii_digit() => {
                    let mut num = String::from(c);
                    let mut is_float = false;
                    while let Some(next_c) = self.peek_c() {
                        if next_c.is_ascii_digit() {
                            num.push(next_c);
                            self.next_c();
                        } else if next_c == '.' {
                            is_float = true;
                            num.push(next_c);
                            self.next_c();
                        } else {
                            break;
                        }
                    }
                    if is_float {
                        let value = num.parse().map_err(|_| {
                            format!(
                                "[line {}:col {}] Lexer error: Float literal '{}' is invalid",
                                start_line, start_col, num
                            )
                        })?;
                        tokens.push(mk_token(Token::FloatLit(value)));
                    } else {
                        let value = num
                            .parse()
                            .map_err(|_| format!("[line {}:col {}] Lexer error: Integer literal '{}' is out of range for Int", start_line, start_col, num))?;
                        tokens.push(mk_token(Token::Int(value)));
                    }
                }
                _ if c.is_alphabetic() || c == '_' => {
                    let mut ident = String::from(c);
                    while let Some(next_c) = self.peek_c() {
                        if next_c.is_alphanumeric() || next_c == '_' {
                            ident.push(next_c);
                            self.next_c();
                        } else {
                            break;
                        }
                    }
                    match ident.as_str() {
                        "def" => tokens.push(mk_token(Token::Def)),
                        "return" => tokens.push(mk_token(Token::Return)),
                        "mut" => tokens.push(mk_token(Token::Mut)),
                        "struct" => tokens.push(mk_token(Token::Struct)),
                        "enum" => tokens.push(mk_token(Token::Enum)),
                        "match" => tokens.push(mk_token(Token::Match)),
                        "fn" => tokens.push(mk_token(Token::Fn)),
                        "spawn" => tokens.push(mk_token(Token::Spawn)),
                        "import" => tokens.push(mk_token(Token::Import)),
                        "from" => tokens.push(mk_token(Token::From)),
                        "as" => tokens.push(mk_token(Token::As)),
                        "pub" => tokens.push(mk_token(Token::Pub)),
                        "if" => tokens.push(mk_token(Token::If)),
                        "else" => tokens.push(mk_token(Token::Else)),
                        "while" => tokens.push(mk_token(Token::While)),
                        "for" => tokens.push(mk_token(Token::For)),
                        "in" => tokens.push(mk_token(Token::In)),
                        "break" => tokens.push(mk_token(Token::Break)),
                        "continue" => tokens.push(mk_token(Token::Continue)),
                        "true" => tokens.push(mk_token(Token::BoolLit(true))),
                        "false" => tokens.push(mk_token(Token::BoolLit(false))),
                        _ => tokens.push(mk_token(Token::Ident(ident))),
                    }
                }
                _ => {
                    return Err(format!(
                        "[line {}:col {}] Lexer error: Unexpected character: {}",
                        start_line, start_col, c
                    ));
                }
            }
        }
        Ok(tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::{Lexer, Token};

    #[test]
    fn rejects_out_of_range_integer_literals() {
        let mut lexer = Lexer::new("9223372036854775808");
        let err = lexer
            .tokenize()
            .expect_err("lexer should reject overflowing Int literals");
        assert!(err.contains("out of range"));
        assert!(err.contains("line 1"));
    }

    #[test]
    fn emits_distinct_bindings_tokens_for_shadowing_source() {
        let mut lexer = Lexer::new("def main():\n    x := 1\n    x := 2\n");
        let tokens = lexer
            .tokenize()
            .expect("lexer should accept valid shadowing syntax");
        let raw_tokens: Vec<Token> = tokens.into_iter().map(|st| st.token).collect();
        assert!(raw_tokens.contains(&Token::Assign));
    }

    #[test]
    fn lexes_boolean_literals() {
        let mut lexer = Lexer::new("true false");
        let tokens = lexer
            .tokenize()
            .expect("lexer should parse boolean literals");
        let raw_tokens: Vec<Token> = tokens.into_iter().map(|st| st.token).collect();
        assert_eq!(
            raw_tokens,
            vec![Token::BoolLit(true), Token::BoolLit(false), Token::Eof]
        );
    }

    #[test]
    fn lexes_break_and_continue_keywords() {
        let mut lexer = Lexer::new("break continue");
        let tokens = lexer
            .tokenize()
            .expect("lexer should parse break and continue keywords");
        let raw_tokens: Vec<Token> = tokens.into_iter().map(|st| st.token).collect();
        assert_eq!(
            raw_tokens,
            vec![Token::Break, Token::Continue, Token::Eof]
        );
    }
}
