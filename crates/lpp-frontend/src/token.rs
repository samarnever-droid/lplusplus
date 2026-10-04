use lpp_common::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Keyword {
    As,
    Async,
    Await,
    Break,
    Const,
    Continue,
    Def,
    Elif,
    Else,
    Enum,
    Extern,
    Fn,
    For,
    From,
    If,
    Impl,
    Import,
    In,
    Match,
    Mut,
    Pub,
    Return,
    Spawn,
    Struct,
    Trait,
    Type,
    While,
}

impl Keyword {
    #[must_use]
    pub fn from_ident(value: &str) -> Option<Self> {
        Some(match value {
            "as" => Self::As,
            "async" => Self::Async,
            "await" => Self::Await,
            "break" => Self::Break,
            "const" => Self::Const,
            "continue" => Self::Continue,
            "def" => Self::Def,
            "elif" => Self::Elif,
            "else" => Self::Else,
            "enum" => Self::Enum,
            "extern" => Self::Extern,
            "fn" => Self::Fn,
            "for" => Self::For,
            "from" => Self::From,
            "if" => Self::If,
            "impl" => Self::Impl,
            "import" => Self::Import,
            "in" => Self::In,
            "match" => Self::Match,
            "mut" => Self::Mut,
            "pub" => Self::Pub,
            "return" => Self::Return,
            "spawn" => Self::Spawn,
            "struct" => Self::Struct,
            "trait" => Self::Trait,
            "type" => Self::Type,
            "while" => Self::While,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Bom,
    Whitespace,
    Comment,
    Newline,
    SoftNewline,
    Indent,
    Dedent,
    Eof,
    Keyword(Keyword),
    Identifier,
    Integer,
    Float,
    String,
    FString,
    Character,
    Bool,
    Assign,
    Equal,
    EqualEqual,
    NotEqual,
    Less,
    Greater,
    LessEqual,
    GreaterEqual,
    LogicalAnd,
    LogicalOr,
    BitAnd,
    BitOr,
    BitXor,
    ShiftLeft,
    ShiftRight,
    PlusEqual,
    MinusEqual,
    StarEqual,
    SlashEqual,
    PercentEqual,
    Colon,
    Arrow,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Question,
    Not,
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    Comma,
    Dot,
    Ellipsis,
    At,
}

impl TokenKind {
    #[must_use]
    pub const fn is_trivia(self) -> bool {
        matches!(
            self,
            Self::Bom | Self::Whitespace | Self::Comment | Self::SoftNewline
        )
    }

    #[must_use]
    pub const fn is_line_end(self) -> bool {
        matches!(self, Self::Newline | Self::Eof | Self::Dedent)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    #[must_use]
    pub const fn is_synthetic(self) -> bool {
        matches!(
            self.kind,
            TokenKind::Indent | TokenKind::Dedent | TokenKind::Eof
        )
    }

    #[must_use]
    pub fn text(self, source: &str) -> &str {
        &source[self.span.start as usize..self.span.end as usize]
    }
}
