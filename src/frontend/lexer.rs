#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Keywords
    Def,
    Return,
    Mut,
    Struct,
    Fn,
    Spawn,
    Import,

    If,
    Else,
    While,

    // Identifiers and Literals
    Ident(String),
    Int(i64),
    StringLit(String),

    // Operators and Punctuation
    Assign, // :=
    Equal,  // =
    EqEq,   // ==
    NotEq,  // !=
    Less,   // <
    Greater,// >
    LessEq, // <=
    GreaterEq,// >=
    Colon,  // :
    Arrow,  // ->
    Plus,   // +
    Minus,  // -
    Star,   // *
    Slash,  // /
    Percent,// %
    LParen, // (
    RParen, // )
    LBracket, // [
    RBracket, // ]
    Comma,  // ,
    Dot,    // .

    // Significant Whitespace
    Newline,
    Indent,
    Dedent,

    // End of File
    Eof,
}

pub struct Lexer<'a> {
    input: std::iter::Peekable<std::str::Chars<'a>>,
    indent_stack: Vec<usize>,
    pending_tokens: Vec<Token>,
    at_line_start: bool,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            input: input.chars().peekable(),
            indent_stack: vec![0],
            pending_tokens: Vec::new(),
            at_line_start: true,
        }
    }

    pub fn tokenize(&mut self) -> Result<Vec<Token>, String> {
        let mut tokens = Vec::new();

        loop {
            if !self.pending_tokens.is_empty() {
                tokens.push(self.pending_tokens.remove(0));
                continue;
            }

            if self.at_line_start {
                self.at_line_start = false;
                let mut spaces = 0;
                
                while let Some(&c) = self.input.peek() {
                    if c == ' ' {
                        spaces += 1;
                        self.input.next();
                    } else if c == '\t' {
                        return Err("Tabs are not allowed for indentation. Use spaces.".to_string());
                    } else if c == '\n' || c == '\r' {
                        // Empty line, ignore indentation
                        break;
                    } else {
                        break;
                    }
                }

                if let Some(&c) = self.input.peek() {
                    if c != '\n' && c != '\r' {
                        let current_indent = *self.indent_stack.last().unwrap();
                        if spaces > current_indent {
                            self.indent_stack.push(spaces);
                            tokens.push(Token::Indent);
                        } else if spaces < current_indent {
                            while let Some(&top) = self.indent_stack.last() {
                                if top > spaces {
                                    self.indent_stack.pop();
                                    tokens.push(Token::Dedent);
                                } else if top == spaces {
                                    break;
                                } else {
                                    return Err("Inconsistent indentation level.".to_string());
                                }
                            }
                        }
                    }
                }
            }

            let c = match self.input.next() {
                Some(c) => c,
                None => {
                    while self.indent_stack.len() > 1 {
                        self.indent_stack.pop();
                        tokens.push(Token::Dedent);
                    }
                    tokens.push(Token::Eof);
                    break;
                }
            };

            match c {
                ' ' => continue,
                '\r' => continue,
                '\n' => {
                    tokens.push(Token::Newline);
                    self.at_line_start = true;
                }
                ':' => {
                    if self.input.peek() == Some(&'=') {
                        self.input.next();
                        tokens.push(Token::Assign);
                    } else {
                        tokens.push(Token::Colon);
                    }
                }
                '-' => {
                    if self.input.peek() == Some(&'>') {
                        self.input.next();
                        tokens.push(Token::Arrow);
                    } else {
                        tokens.push(Token::Minus);
                    }
                }
                '=' => {
                    if self.input.peek() == Some(&'=') {
                        self.input.next();
                        tokens.push(Token::EqEq);
                    } else {
                        tokens.push(Token::Equal);
                    }
                }
                '<' => {
                    if self.input.peek() == Some(&'=') {
                        self.input.next();
                        tokens.push(Token::LessEq);
                    } else {
                        tokens.push(Token::Less);
                    }
                }
                '>' => {
                    if self.input.peek() == Some(&'=') {
                        self.input.next();
                        tokens.push(Token::GreaterEq);
                    } else {
                        tokens.push(Token::Greater);
                    }
                }
                '!' => {
                    if self.input.peek() == Some(&'=') {
                        self.input.next();
                        tokens.push(Token::NotEq);
                    } else {
                        return Err("Unexpected character: !".to_string());
                    }
                }
                '+' => tokens.push(Token::Plus),
                '*' => tokens.push(Token::Star),
                '/' => tokens.push(Token::Slash),
                '%' => tokens.push(Token::Percent),
                '#' => {
                    while let Some(&next_c) = self.input.peek() {
                        if next_c == '\n' || next_c == '\r' {
                            break;
                        }
                        self.input.next();
                    }
                }
                '(' => tokens.push(Token::LParen),
                ')' => tokens.push(Token::RParen),
                '[' => tokens.push(Token::LBracket),
                ']' => tokens.push(Token::RBracket),
                ',' => tokens.push(Token::Comma),
                '.' => tokens.push(Token::Dot),
                '"' => {
                    let mut s = String::new();
                    while let Some(ch) = self.input.next() {
                        if ch == '"' {
                            break;
                        }
                        if ch == '\\' {
                            if let Some(escaped) = self.input.next() {
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
                                return Err("Unterminated string escape".to_string());
                            }
                        } else {
                            s.push(ch);
                        }
                    }
                    tokens.push(Token::StringLit(s));
                }
                _ if c.is_ascii_digit() => {
                    let mut num = String::from(c);
                    while let Some(&next_c) = self.input.peek() {
                        if next_c.is_ascii_digit() {
                            num.push(next_c);
                            self.input.next();
                        } else {
                            break;
                        }
                    }
                    tokens.push(Token::Int(num.parse().unwrap()));
                }
                _ if c.is_alphabetic() || c == '_' => {
                    let mut ident = String::from(c);
                    while let Some(&next_c) = self.input.peek() {
                        if next_c.is_alphanumeric() || next_c == '_' {
                            ident.push(next_c);
                            self.input.next();
                        } else {
                            break;
                        }
                    }
                    match ident.as_str() {
                        "def" => tokens.push(Token::Def),
                        "return" => tokens.push(Token::Return),
                        "mut" => tokens.push(Token::Mut),
                        "struct" => tokens.push(Token::Struct),
                        "fn" => tokens.push(Token::Fn),
                        "spawn" => tokens.push(Token::Spawn),
                        "import" => tokens.push(Token::Import),
                        "if" => tokens.push(Token::If),
                        "else" => tokens.push(Token::Else),
                        "while" => tokens.push(Token::While),
                        _ => tokens.push(Token::Ident(ident)),
                    }
                }
                _ => return Err(format!("Unexpected character: {}", c)),
            }
        }
        Ok(tokens)
    }
}
