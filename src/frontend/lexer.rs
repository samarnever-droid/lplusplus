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
    Const,
    TypeKw,
    Trait,
    ImplKw,
    Extern,
    Async,
    Await,

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
    CharLit(char),
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
    BitAnd,    // &
    BitOr,     // |
    BitXor,    // ^
    Shl,       // <<
    Shr,       // >>
    PlusEq,    // +=
    MinusEq,   // -=
    StarEq,    // *=
    SlashEq,   // /=
    PercentEq, // %=
    Colon,     // :
    Arrow,     // ->
    Plus,      // +
    Minus,     // -
    Star,      // *
    Slash,     // /
    Percent,   // %
    Question,  // ?
    Not,       // !
    /// f"hello {name}" — interpolated string (stores parts + expressions)
    FStringLit(Vec<FStringPart>),
    LParen,    // (
    RParen,    // )
    LBracket,  // [
    RBracket,  // ]
    Comma,     // ,
    Dot,       // .
    Ellipsis,  // ...

    // Significant Whitespace
    Newline,
    Indent,
    Dedent,

    // End of File
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FStringPart {
    Literal(String),
    Expr(String),
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
    paren_depth: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        let clean_input = input.strip_prefix('\u{FEFF}').unwrap_or(input);
        Self {
            chars: clean_input.chars().peekable(),
            line: 1,
            col: 1,
            indent_stack: vec![0],
            pending_tokens: VecDeque::new(),
            at_line_start: true,
            paren_depth: 0,
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

                if self.paren_depth == 0 {
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
                    if self.paren_depth == 0 {
                        tokens.push(mk_token(Token::Newline));
                        self.at_line_start = true;
                    }
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
                    } else if self.peek_c() == Some('=') {
                        self.next_c();
                        tokens.push(mk_token(Token::MinusEq));
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
                    } else if self.peek_c() == Some('<') {
                        self.next_c();
                        tokens.push(mk_token(Token::Shl));
                    } else {
                        tokens.push(mk_token(Token::Less));
                    }
                }
                '>' => {
                    if self.peek_c() == Some('=') {
                        self.next_c();
                        tokens.push(mk_token(Token::GreaterEq));
                    } else if self.peek_c() == Some('>') {
                        self.next_c();
                        tokens.push(mk_token(Token::Shr));
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
                '+' => {
                    if self.peek_c() == Some('=') { self.next_c(); tokens.push(mk_token(Token::PlusEq)); }
                    else { tokens.push(mk_token(Token::Plus)); }
                }
                '*' => {
                    if self.peek_c() == Some('=') { self.next_c(); tokens.push(mk_token(Token::StarEq)); }
                    else { tokens.push(mk_token(Token::Star)); }
                }
                '/' => {
                    if self.peek_c() == Some('/') {
                        self.next_c();
                        while let Some(next_c) = self.peek_c() {
                            if next_c == '\n' {
                                break;
                            }
                            self.next_c();
                        }
                        continue;
                    } else if self.peek_c() == Some('=') {
                        self.next_c();
                        tokens.push(mk_token(Token::SlashEq));
                    } else {
                        tokens.push(mk_token(Token::Slash));
                    }
                }
                '%' => {
                    if self.peek_c() == Some('=') { self.next_c(); tokens.push(mk_token(Token::PercentEq)); }
                    else { tokens.push(mk_token(Token::Percent)); }
                }
                '?' => tokens.push(mk_token(Token::Question)),
                '&' => {
                    if self.peek_c() == Some('&') {
                        self.next_c();
                        tokens.push(mk_token(Token::And));
                    } else {
                        tokens.push(mk_token(Token::BitAnd));
                    }
                }
                '|' => {
                    if self.peek_c() == Some('|') {
                        self.next_c();
                        tokens.push(mk_token(Token::Or));
                    } else {
                        tokens.push(mk_token(Token::BitOr));
                    }
                }
                '^' => tokens.push(mk_token(Token::BitXor)),
                '#' => {
                    while let Some(next_c) = self.peek_c() {
                        if next_c == '\n' || next_c == '\r' {
                            break;
                        }
                        self.next_c();
                    }
                }
                '(' => {
                    self.paren_depth += 1;
                    tokens.push(mk_token(Token::LParen));
                }
                ')' => {
                    if self.paren_depth > 0 { self.paren_depth -= 1; }
                    tokens.push(mk_token(Token::RParen));
                }
                '[' => {
                    self.paren_depth += 1;
                    tokens.push(mk_token(Token::LBracket));
                }
                ']' => {
                    if self.paren_depth > 0 { self.paren_depth -= 1; }
                    tokens.push(mk_token(Token::RBracket));
                }
                ',' => tokens.push(mk_token(Token::Comma)),
                '.' => {
                    if self.peek_c() == Some('.') {
                        self.next_c();
                        if self.peek_c() == Some('.') {
                            self.next_c();
                            tokens.push(mk_token(Token::Ellipsis));
                        } else {
                            return Err(format!("[line {}:col {}] Lexer error: Expected third '.' in variadic marker", start_line, start_col));
                        }
                    } else {
                        tokens.push(mk_token(Token::Dot));
                    }
                },
                '\'' => {
                    let ch = match self.next_c() {
                        Some('\\') => match self.next_c() {
                            Some('n') => '\n',
                            Some('r') => '\r',
                            Some('t') => '\t',
                            Some('0') => '\0',
                            Some('\'') => '\'',
                            Some('\\') => '\\',
                            Some('x') => {
                                let h1 = self.next_c().ok_or_else(|| format!("[line {}:col {}] Unterminated hex escape", start_line, start_col))?;
                                let h2 = self.next_c().ok_or_else(|| format!("[line {}:col {}] Unterminated hex escape", start_line, start_col))?;
                                let hex_str = format!("{}{}", h1, h2);
                                let val = u8::from_str_radix(&hex_str, 16)
                                    .map_err(|_| format!("[line {}:col {}] Invalid hex escape '\\x{}'", start_line, start_col, hex_str))?;
                                val as char
                            }
                            Some(c) => {
                                return Err(format!(
                                    "[line {}:col {}] Lexer error: Unknown escape sequence '\\{}'",
                                    start_line, start_col, c
                                ));
                            }
                            None => return Err(format!("[line {}:col {}] Unterminated char escape", start_line, start_col)),
                        },
                        Some(c) => c,
                        None => return Err(format!("[line {}:col {}] Unterminated char literal", start_line, start_col)),
                    };
                    if self.next_c() != Some('\'') {
                        return Err(format!("[line {}:col {}] Unclosed char literal", start_line, start_col));
                    }
                    tokens.push(mk_token(Token::CharLit(ch)));
                }
                '"' => {
                    // Check for triple-quote multiline string
                    if self.peek_c() == Some('"') {
                        self.next_c(); // second "
                        if self.peek_c() == Some('"') {
                            self.next_c(); // third "
                            let mut s = String::new();
                            let mut terminated = false;
                            loop {
                                match self.next_c() {
                                    Some('"') if self.peek_c() == Some('"') => {
                                        self.next_c();
                                        if self.peek_c() == Some('"') {
                                            self.next_c();
                                            terminated = true;
                                            break;
                                        }
                                        s.push('"'); s.push('"');
                                    }
                                    Some(ch) => s.push(ch),
                                    None => break,
                                }
                            }
                            if !terminated {
                                return Err(format!("[line {}:col {}] Lexer error: Unterminated triple-quote string", start_line, start_col));
                            }
                            tokens.push(mk_token(Token::StringLit(s)));
                            continue;
                        }
                        // Just two quotes: empty string ""
                        tokens.push(mk_token(Token::StringLit(String::new())));
                        continue;
                    }
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
                                    '0' => s.push('\0'),
                                    '"' => s.push('"'),
                                    '\\' => s.push('\\'),
                                    'x' => {
                                        let h1 = self.next_c().ok_or_else(|| format!("[line {}:col {}] Unterminated hex escape in string", self.line, self.col))?;
                                        let h2 = self.next_c().ok_or_else(|| format!("[line {}:col {}] Unterminated hex escape in string", self.line, self.col))?;
                                        let hex_str = format!("{}{}", h1, h2);
                                        let val = u8::from_str_radix(&hex_str, 16)
                                            .map_err(|_| format!("[line {}:col {}] Invalid hex escape '\\x{}' in string", self.line, self.col, hex_str))?;
                                        s.push(val as char);
                                    }
                                    other => {
                                        return Err(format!(
                                            "[line {}:col {}] Lexer error: Unknown escape sequence '\\{}'",
                                            self.line, self.col, other
                                        ));
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
                    // Check for hex (0x) or binary (0b) literals
                    if c == '0' {
                        if self.peek_c() == Some('x') || self.peek_c() == Some('X') {
                            self.next_c(); // consume x
                            let mut hex = String::new();
                            while let Some(hc) = self.peek_c() {
                                if hc.is_ascii_hexdigit() { hex.push(hc); self.next_c(); }
                                else { break; }
                            }
                            let value = i64::from_str_radix(&hex, 16).map_err(|_|
                                format!("[line {}:col {}] Lexer error: Invalid hex literal '0x{}'", start_line, start_col, hex))?;
                            tokens.push(mk_token(Token::Int(value)));
                            continue;
                        }
                        if self.peek_c() == Some('b') || self.peek_c() == Some('B') {
                            self.next_c(); // consume b
                            let mut bin = String::new();
                            while let Some(bc) = self.peek_c() {
                                if bc == '0' || bc == '1' { bin.push(bc); self.next_c(); }
                                else { break; }
                            }
                            let value = i64::from_str_radix(&bin, 2).map_err(|_|
                                format!("[line {}:col {}] Lexer error: Invalid binary literal '0b{}'", start_line, start_col, bin))?;
                            tokens.push(mk_token(Token::Int(value)));
                            continue;
                        }
                    }
                    let mut num = String::from(c);
                    let mut is_float = false;
                    while let Some(next_c) = self.peek_c() {
                        if next_c.is_ascii_digit() {
                            num.push(next_c);
                            self.next_c();
                        } else if next_c == '.' && !is_float {
                            let mut clone_iter = self.chars.clone();
                            clone_iter.next();
                            if let Some(&after_dot) = clone_iter.peek() {
                                if after_dot.is_ascii_digit() {
                                    is_float = true;
                                    num.push(next_c);
                                    self.next_c();
                                } else {
                                    break;
                                }
                            } else {
                                break;
                            }
                        } else if next_c == '_' {
                            self.next_c(); // skip underscores in numbers (1_000_000)
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
                        "const" => tokens.push(mk_token(Token::Const)),
                        "type" => tokens.push(mk_token(Token::TypeKw)),
                        "trait" => tokens.push(mk_token(Token::Trait)),
                        "impl" => tokens.push(mk_token(Token::ImplKw)),
                        "extern" => tokens.push(mk_token(Token::Extern)),
                        "async" => tokens.push(mk_token(Token::Async)),
                        "await" => tokens.push(mk_token(Token::Await)),
                        "if" => tokens.push(mk_token(Token::If)),
                        "else" => tokens.push(mk_token(Token::Else)),
                        "elif" => {
                            // elif → Else + If (synthetic two tokens)
                            tokens.push(mk_token(Token::Else));
                            tokens.push(mk_token(Token::If));
                        }
                        "while" => tokens.push(mk_token(Token::While)),
                        "for" => tokens.push(mk_token(Token::For)),
                        "in" => tokens.push(mk_token(Token::In)),
                        "break" => tokens.push(mk_token(Token::Break)),
                        "continue" => tokens.push(mk_token(Token::Continue)),
                        "true" => tokens.push(mk_token(Token::BoolLit(true))),
                        "false" => tokens.push(mk_token(Token::BoolLit(false))),
                        "and" => tokens.push(mk_token(Token::And)),
                        "or" => tokens.push(mk_token(Token::Or)),
                        "not" => tokens.push(mk_token(Token::Not)),
                        _ => {
                            // Check for f"..." string interpolation
                            if ident == "f" && self.peek_c() == Some('"') {
                                self.next_c(); // consume opening "
                                let mut parts = Vec::new();
                                let mut current = String::new();
                                let mut terminated = false;
                                while let Some(ch) = self.next_c() {
                                    if ch == '"' {
                                        terminated = true;
                                        break;
                                    }
                                    if ch == '{' {
                                        if !current.is_empty() {
                                            parts.push(FStringPart::Literal(current.clone()));
                                            current.clear();
                                        }
                                        let mut expr_str = String::new();
                                        while let Some(ec) = self.next_c() {
                                            if ec == '}' { break; }
                                            expr_str.push(ec);
                                        }
                                        parts.push(FStringPart::Expr(expr_str));
                                    } else if ch == '\\' {
                                        if let Some(esc) = self.next_c() {
                                            match esc {
                                                'n' => current.push('\n'),
                                                'r' => current.push('\r'),
                                                't' => current.push('\t'),
                                                '0' => current.push('\0'),
                                                '"' => current.push('"'),
                                                '\\' => current.push('\\'),
                                                '{' => current.push('{'),
                                                '}' => current.push('}'),
                                                other => {
                                                    return Err(format!(
                                                        "[line {}:col {}] Lexer error: Unknown escape sequence '\\{}'",
                                                        self.line, self.col, other
                                                    ));
                                                }
                                            }
                                        }
                                    } else {
                                        current.push(ch);
                                    }
                                }
                                if !current.is_empty() {
                                    parts.push(FStringPart::Literal(current));
                                }
                                if !terminated {
                                    return Err(format!(
                                        "[line {}:col {}] Lexer error: Unterminated f-string",
                                        self.line, self.col
                                    ));
                                }
                                tokens.push(mk_token(Token::FStringLit(parts)));
                            } else {
                                tokens.push(mk_token(Token::Ident(ident)));
                            }
                        }
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
    fn lexes_char_literals() {
        let mut lexer = Lexer::new("'a' '\\n' '\\t'");
        let tokens = lexer
            .tokenize()
            .expect("lexer should parse char literals");
        let raw_tokens: Vec<Token> = tokens.into_iter().map(|st| st.token).collect();
        assert_eq!(
            raw_tokens,
            vec![Token::CharLit('a'), Token::CharLit('\n'), Token::CharLit('\t'), Token::Eof]
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

    #[test]
    fn rejects_unknown_escape_sequences() {
        let mut lexer = Lexer::new("\"hello \\q world\"");
        let err = lexer.tokenize().expect_err("should reject \\q escape");
        assert!(err.contains("Unknown escape sequence '\\q'"));

        let mut char_lexer = Lexer::new("'\\q'");
        let char_err = char_lexer.tokenize().expect_err("should reject \\q char escape");
        assert!(char_err.contains("Unknown escape sequence '\\q'"));
    }
}
