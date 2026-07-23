#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Int,
    Float,
    String,
    Bool,
    Void,
    Custom(String),
    Generic(String, Vec<Type>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOperator {
    Negate,   // -x
    Not,      // !x
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinaryOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    Eq,
    NotEq,
    Less,
    Greater,
    LessEq,
    GreaterEq,
    And,       // &&
    Or,        // ||
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    IntLiteral(i64),
    FloatLiteral(f64),
    StringLiteral(String),
    BoolLiteral(bool),
    Identifier(String, std::cell::Cell<Option<usize>>),
    /// `-x` or `!b`
    UnaryOp {
        op: UnaryOperator,
        operand: Box<Expr>,
    },
    BinaryOp {
        left: Box<Expr>,
        op: BinaryOperator,
        right: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    Closure {
        params: Vec<ClosureParam>,
        return_type: Option<Type>,
        body: Vec<Stmt>,
    },
    FieldAccess {
        base: Box<Expr>,
        field: String,
    },
    Spawn {
        closure: Box<Expr>,
    },
    ListLiteral(Vec<Expr>),
    /// `match expr: Ok(v): ..., Err(e): ...`
    Match {
        subject: Box<Expr>,
        arms: Vec<MatchArm>,
    },
    /// `expr?` — try operator: unwrap Ok or return Err early
    Try(Box<Expr>),
    /// `expr[index]` — subscript/index access
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
    },
    /// `Result.Ok(42)` — enum variant constructor
    EnumVariantConstruct {
        enum_name: String,
        variant: String,
        args: Vec<Expr>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    LetInferred {
        name: String,
        is_mut: bool,
        value: Expr,
        binding_id: std::cell::Cell<Option<usize>>,
    },
    Assign {
        name: String,
        value: Expr,
        binding_id: std::cell::Cell<Option<usize>>,
    },
    AssignField {
        base: Expr,
        field: String,
        value: Expr,
    },
    Expr(Expr),
    Return(Option<Expr>),
    If {
        condition: Expr,
        then_block: Vec<Stmt>,
        else_block: Option<Vec<Stmt>>,
    },
    While {
        condition: Expr,
        body: Vec<Stmt>,
    },
    ForRange {
        var_name: String,
        start: Expr,
        end: Expr,
        body: Vec<Stmt>,
        binding_id: std::cell::Cell<Option<usize>>,
    },
    ForIn {
        var_name: String,
        list: Expr,
        body: Vec<Stmt>,
        binding_id: std::cell::Cell<Option<usize>>,
    },
    Break,
    Continue,
    Block(Vec<Stmt>),
    /// `match expr: Ok(v): ..., Err(e): ...`
    Match {
        subject: Expr,
        arms: Vec<MatchArm>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: Type,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClosureParam {
    pub name: String,
    pub ty: Option<Type>, // Optional for type inference
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: Type,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<Param>,
}

/// Enum variant: `Ok(value: Int)` or `None` (no data)
#[derive(Debug, Clone, PartialEq)]
pub struct EnumVariant {
    pub name: String,
    pub fields: Vec<Param>, // empty for unit variants like `None`
}

/// Enum definition: `enum Result: Ok(value: Int), Err(msg: Str)`
#[derive(Debug, Clone, PartialEq)]
pub struct EnumDef {
    pub name: String,
    pub variants: Vec<EnumVariant>,
}

/// A single arm in a match expression
#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub variant: String,                    // "Ok", "Err", "None"
    pub bindings: Vec<String>,              // ["value"], ["msg"], []
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImportKind {
    /// `import math` — imports module, access via `math.func()`
    Module {
        path: Vec<String>,     // ["math"] or ["utils", "math"]
        alias: Option<String>, // `import math as m` → alias = Some("m")
    },
    /// `from math import sqrt, PI` — imports specific items into scope
    Selective {
        path: Vec<String>,   // ["math"] or ["utils", "math"]
        items: Vec<String>,  // ["sqrt", "PI"]
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum TopLevel {
    Function(Function),
    Struct(StructDef),
    Enum(EnumDef),
    Import(ImportKind),
    /// `const MAX_SIZE = 1024`
    Const {
        name: String,
        value: Expr,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub declarations: Vec<TopLevel>,
}
