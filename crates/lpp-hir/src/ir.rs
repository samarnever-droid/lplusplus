use lpp_common::Span;

use crate::arena::Arena;
use crate::graph::ModuleId;
use crate::ids::{
    BodyId, DefId, ExprId, FieldId, HirItemId, LocalId, MatchArmId, OriginId, ScopeId, StmtId,
    Symbol, TypeParamId, TypeRefId, VariantId,
};
use crate::list::{IdList, IdRange};
use crate::resolve::{BindingTarget, NameIndex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesugaringKind {
    AugmentedAssignment,
    Elif,
    ImplicitClosureReturn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginKind {
    Source,
    Desugared(DesugaringKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Origin {
    pub span: Span,
    pub parent: Option<OriginId>,
    pub kind: OriginKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scope {
    pub parent: Option<ScopeId>,
    pub bindings: IdRange<LocalId>,
    pub origin: OriginId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalKind {
    Parameter,
    Binding,
    Pattern,
    Loop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Local {
    pub name: Symbol,
    pub mutable: bool,
    pub kind: LocalKind,
    pub type_ref: Option<TypeRefId>,
    pub default: Option<ExprId>,
    pub scope: ScopeId,
    pub origin: OriginId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeRefKind {
    Named(Symbol),
    Applied {
        base: Symbol,
        arguments: IdRange<TypeRefId>,
    },
    Tuple(IdRange<TypeRefId>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeRef {
    pub kind: TypeRefKind,
    pub origin: OriginId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeParameter {
    pub name: Symbol,
    pub bound: Option<TypeRefId>,
    pub origin: OriginId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field {
    pub name: Symbol,
    pub type_ref: TypeRefId,
    pub default: Option<ExprId>,
    pub origin: OriginId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Variant {
    pub name: Symbol,
    pub fields: IdRange<FieldId>,
    pub origin: OriginId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Literal {
    Integer(i64),
    FloatBits(u64),
    String { span: Span, formatted: bool },
    Character(Span),
    Bool(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameBinding {
    Local(LocalId),
    Item(BindingTarget),
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOperator {
    Negate,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    Equal,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpressionKind {
    Literal(Literal),
    Name {
        symbol: Symbol,
        binding: NameBinding,
    },
    Unary {
        operator: UnaryOperator,
        operand: ExprId,
    },
    Binary {
        left: ExprId,
        operator: BinaryOperator,
        right: ExprId,
    },
    Tuple(IdRange<ExprId>),
    List(IdRange<ExprId>),
    Call {
        callee: ExprId,
        arguments: IdRange<ExprId>,
    },
    GenericCall {
        callee: ExprId,
        type_arguments: IdRange<TypeRefId>,
        arguments: IdRange<ExprId>,
    },
    Field {
        base: ExprId,
        name: Symbol,
    },
    Index {
        base: ExprId,
        index: ExprId,
    },
    Try(ExprId),
    Await(ExprId),
    Spawn(ExprId),
    Closure {
        parameters: IdRange<LocalId>,
        return_type: Option<TypeRefId>,
        body: BodyId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Expression {
    pub kind: ExpressionKind,
    pub origin: OriginId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatementKind {
    Let {
        bindings: IdRange<LocalId>,
        value: ExprId,
    },
    Assign {
        target: ExprId,
        value: ExprId,
    },
    Expression(ExprId),
    Return(Option<ExprId>),
    If {
        condition: ExprId,
        then_body: BodyId,
        else_body: Option<BodyId>,
    },
    While {
        condition: ExprId,
        body: BodyId,
    },
    For {
        binding: LocalId,
        iterable: ExprId,
        body: BodyId,
    },
    Match {
        subject: ExprId,
        arms: IdRange<MatchArmId>,
    },
    Break,
    Continue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Statement {
    pub kind: StatementKind,
    pub origin: OriginId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Body {
    pub scope: ScopeId,
    pub statements: IdRange<StmtId>,
    pub origin: OriginId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchArm {
    pub path: IdRange<Symbol>,
    pub bindings: IdRange<LocalId>,
    pub body: BodyId,
    pub wildcard: bool,
    pub origin: OriginId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Function {
    pub type_parameters: IdRange<TypeParamId>,
    pub parameters: IdRange<LocalId>,
    pub return_type: Option<TypeRefId>,
    pub body: Option<BodyId>,
    pub is_async: bool,
    pub variadic: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Struct {
    pub type_parameters: IdRange<TypeParamId>,
    pub fields: IdRange<FieldId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Enum {
    pub type_parameters: IdRange<TypeParamId>,
    pub variants: IdRange<VariantId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Trait {
    pub type_parameters: IdRange<TypeParamId>,
    pub methods: IdRange<HirItemId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Impl {
    pub type_parameters: IdRange<TypeParamId>,
    pub trait_ref: Option<TypeRefId>,
    pub target: TypeRefId,
    pub methods: IdRange<HirItemId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExternBlock {
    pub abi: Span,
    pub link_library: Option<Span>,
    pub functions: IdRange<HirItemId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HirItemKind {
    Function(Function),
    Struct(Struct),
    Enum(Enum),
    Trait(Trait),
    Impl(Impl),
    Extern(ExternBlock),
    Const { value: ExprId },
    TypeAlias { target: TypeRefId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HirItem {
    pub module: ModuleId,
    pub definition: Option<DefId>,
    pub name: Option<Symbol>,
    pub public: bool,
    pub attributes: IdRange<Symbol>,
    pub kind: HirItemKind,
    pub origin: OriginId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HirModule {
    pub module: ModuleId,
    pub items: IdRange<HirItemId>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct HirLists {
    pub symbols: IdList<Symbol>,
    pub type_refs: IdList<TypeRefId>,
    pub type_parameters: IdList<TypeParamId>,
    pub locals: IdList<LocalId>,
    pub expressions: IdList<ExprId>,
    pub statements: IdList<StmtId>,
    pub fields: IdList<FieldId>,
    pub variants: IdList<VariantId>,
    pub items: IdList<HirItemId>,
    pub match_arms: IdList<MatchArmId>,
}

impl HirLists {
    pub fn with_capacity(token_count: usize, item_count: usize) -> Self {
        Self {
            symbols: IdList::with_capacity(token_count / 16),
            type_refs: IdList::with_capacity(token_count / 8),
            type_parameters: IdList::with_capacity(item_count),
            locals: IdList::with_capacity(token_count / 8),
            expressions: IdList::with_capacity(token_count / 3),
            statements: IdList::with_capacity(token_count / 8),
            fields: IdList::with_capacity(item_count),
            variants: IdList::with_capacity(item_count),
            items: IdList::with_capacity(item_count.saturating_mul(2)),
            match_arms: IdList::with_capacity(item_count),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HirPackage {
    pub names: NameIndex,
    pub modules: Vec<HirModule>,
    pub origins: Arena<OriginId, Origin>,
    pub scopes: Arena<ScopeId, Scope>,
    pub locals: Arena<LocalId, Local>,
    pub type_refs: Arena<TypeRefId, TypeRef>,
    pub type_parameters: Arena<TypeParamId, TypeParameter>,
    pub fields: Arena<FieldId, Field>,
    pub variants: Arena<VariantId, Variant>,
    pub expressions: Arena<ExprId, Expression>,
    pub statements: Arena<StmtId, Statement>,
    pub bodies: Arena<BodyId, Body>,
    pub match_arms: Arena<MatchArmId, MatchArm>,
    pub items: Arena<HirItemId, HirItem>,
    pub(crate) lists: HirLists,
}

impl HirPackage {
    #[must_use]
    pub fn symbols(&self, range: IdRange<Symbol>) -> &[Symbol] {
        self.lists.symbols.get(range)
    }

    #[must_use]
    pub fn type_refs(&self, range: IdRange<TypeRefId>) -> &[TypeRefId] {
        self.lists.type_refs.get(range)
    }

    #[must_use]
    pub fn type_parameters(&self, range: IdRange<TypeParamId>) -> &[TypeParamId] {
        self.lists.type_parameters.get(range)
    }

    #[must_use]
    pub fn locals(&self, range: IdRange<LocalId>) -> &[LocalId] {
        self.lists.locals.get(range)
    }

    #[must_use]
    pub fn expressions(&self, range: IdRange<ExprId>) -> &[ExprId] {
        self.lists.expressions.get(range)
    }

    #[must_use]
    pub fn statements(&self, range: IdRange<StmtId>) -> &[StmtId] {
        self.lists.statements.get(range)
    }

    #[must_use]
    pub fn fields(&self, range: IdRange<FieldId>) -> &[FieldId] {
        self.lists.fields.get(range)
    }

    #[must_use]
    pub fn variants(&self, range: IdRange<VariantId>) -> &[VariantId] {
        self.lists.variants.get(range)
    }

    #[must_use]
    pub fn items(&self, range: IdRange<HirItemId>) -> &[HirItemId] {
        self.lists.items.get(range)
    }

    #[must_use]
    pub fn match_arms(&self, range: IdRange<MatchArmId>) -> &[MatchArmId] {
        self.lists.match_arms.get(range)
    }
}
