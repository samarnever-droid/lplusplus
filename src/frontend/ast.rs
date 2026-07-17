#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Int,
    String,
    Void,
    Custom(String),
    Generic(String, Vec<Type>),
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
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    IntLiteral(i64),
    StringLiteral(String),
    Identifier(String, std::cell::Cell<Option<usize>>),
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

#[derive(Debug, Clone, PartialEq)]
pub enum TopLevel {
    Function(Function),
    Struct(StructDef),
    Import(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub declarations: Vec<TopLevel>,
}
