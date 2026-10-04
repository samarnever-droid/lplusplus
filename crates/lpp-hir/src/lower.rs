use std::borrow::Cow;
use std::collections::BTreeMap;

use lpp_common::{Diagnostic, Span};
use lpp_frontend::{Item, ItemKind, Keyword, SyntaxKind, SyntaxNode, Token, TokenKind, TokenRange};

use crate::arena::Arena;
use crate::graph::{Module, PackageGraph};
use crate::ids::{
    BodyId, DefId, ExprId, FieldId, HirItemId, LocalId, MatchArmId, OriginId, ScopeId, StmtId,
    Symbol, TypeParamId, TypeRefId, VariantId,
};
use crate::ir::{
    BinaryOperator, Body, DesugaringKind, Enum, Expression, ExpressionKind, ExternBlock, Field,
    Function, HirItem, HirItemKind, HirLists, HirModule, HirPackage, Impl, Literal, Local,
    LocalKind, MatchArm, NameBinding, Origin, OriginKind, Scope, Statement, StatementKind, Struct,
    Trait, TypeParameter, TypeRef, TypeRefKind, UnaryOperator, Variant,
};
use crate::list::IdRange;
use crate::resolve::{BindingTarget, DefinitionKind, ResolutionMode, build_name_index};

mod declaration;
mod expression;
mod statement;

pub fn lower_package(
    graph: &PackageGraph,
    mode: ResolutionMode,
) -> Result<HirPackage, Vec<Diagnostic>> {
    let names = build_name_index(graph, mode)?;
    let token_count = graph
        .modules
        .iter()
        .map(|module| module.syntax.tokens.len())
        .sum::<usize>();
    let item_count = graph
        .modules
        .iter()
        .map(|module| module.syntax.items.len())
        .sum::<usize>();
    let mut package = HirPackage {
        names,
        modules: Vec::with_capacity(graph.modules.len()),
        origins: Arena::with_capacity(token_count),
        scopes: Arena::with_capacity(item_count),
        locals: Arena::with_capacity(token_count / 8),
        type_refs: Arena::with_capacity(token_count / 8),
        type_parameters: Arena::with_capacity(item_count),
        fields: Arena::with_capacity(item_count),
        variants: Arena::with_capacity(item_count),
        expressions: Arena::with_capacity(token_count / 3),
        statements: Arena::with_capacity(token_count / 8),
        bodies: Arena::with_capacity(item_count),
        match_arms: Arena::with_capacity(item_count),
        items: Arena::with_capacity(item_count),
        lists: HirLists::with_capacity(token_count, item_count),
    };

    for module in &graph.modules {
        let mut lowerer = ModuleLowerer::new(module, &mut package);
        let items = lowerer
            .lower_module()
            .map_err(|error| vec![error.into_diagnostic()])?;
        let items =
            package.lists.items.extend(&items).map_err(|error| {
                vec![capacity_diagnostic(error.to_string(), module_span(module))]
            })?;
        package.modules.push(HirModule {
            module: module.id,
            items,
        });
    }

    Ok(package)
}

struct ActiveScope {
    id: ScopeId,
    latest: BTreeMap<Symbol, LocalId>,
    bindings: Vec<LocalId>,
}

struct ModuleLowerer<'module, 'package> {
    module: &'module Module,
    package: &'package mut HirPackage,
    scopes: Vec<ActiveScope>,
}

impl<'module, 'package> ModuleLowerer<'module, 'package> {
    fn new(module: &'module Module, package: &'package mut HirPackage) -> Self {
        Self {
            module,
            package,
            scopes: Vec::new(),
        }
    }

    fn lower_module(&mut self) -> LowerResult<Vec<HirItemId>> {
        let mut lowered = Vec::with_capacity(self.module.syntax.items.len());
        for item in &self.module.syntax.items {
            if item.kind == ItemKind::Import {
                continue;
            }
            let node = self
                .module
                .syntax
                .syntax
                .get(item.syntax_index as usize)
                .ok_or_else(|| {
                    LowerError::new("E3100", "item references a missing syntax node", item.span)
                })?;
            lowered.push(self.lower_item(item, node)?);
        }
        Ok(lowered)
    }

    fn begin_scope(&mut self, parent: Option<ScopeId>, origin: OriginId) -> LowerResult<ScopeId> {
        let id = self.alloc_scope(Scope {
            parent,
            bindings: IdRange::empty(),
            origin,
        })?;
        self.scopes.push(ActiveScope {
            id,
            latest: BTreeMap::new(),
            bindings: Vec::new(),
        });
        Ok(id)
    }

    fn finish_scope(&mut self, scope: ScopeId, span: Span) -> LowerResult<()> {
        let active = self
            .scopes
            .pop()
            .ok_or_else(|| LowerError::new("E3199", "lowering scope stack underflow", span))?;
        if active.id != scope {
            return Err(LowerError::new(
                "E3199",
                "lowering scope stack became inconsistent",
                span,
            ));
        }
        let bindings = self.local_list(&active.bindings, span)?;
        self.package.scopes[scope].bindings = bindings;
        Ok(())
    }

    fn finish_body(
        &mut self,
        scope: ScopeId,
        statements: IdRange<StmtId>,
        origin: OriginId,
    ) -> LowerResult<BodyId> {
        let span = self.package.origins[origin].span;
        self.finish_scope(scope, span)?;
        self.alloc_body(Body {
            scope,
            statements,
            origin,
        })
    }

    fn current_scope(&self) -> Option<ScopeId> {
        self.scopes.last().map(|scope| scope.id)
    }

    fn declare_local(
        &mut self,
        name: Symbol,
        mutable: bool,
        kind: LocalKind,
        type_ref: Option<TypeRefId>,
        default: Option<ExprId>,
        origin: OriginId,
    ) -> LowerResult<LocalId> {
        let scope = self.current_scope().ok_or_else(|| {
            LowerError::new(
                "E3199",
                "local declaration has no lexical scope",
                self.package.origins[origin].span,
            )
        })?;
        let local = self.alloc_local(Local {
            name,
            mutable,
            kind,
            type_ref,
            default,
            scope,
            origin,
        })?;
        let active = self
            .scopes
            .last_mut()
            .expect("current scope exists after local allocation");
        active.latest.insert(name, local);
        active.bindings.push(local);
        Ok(local)
    }

    fn resolve_name(&self, symbol: Symbol) -> NameBinding {
        for scope in self.scopes.iter().rev() {
            if let Some(local) = scope.latest.get(&symbol) {
                return NameBinding::Local(*local);
            }
        }
        self.package
            .names
            .resolve_symbol(self.module.id, symbol)
            .map_or(NameBinding::Unresolved, NameBinding::Item)
    }

    fn definition_for(&self, name: &str) -> Option<DefId> {
        match self.package.names.resolve(self.module.id, name) {
            Some(BindingTarget::Definition(definition))
                if self.package.names.definitions[definition].module == self.module.id =>
            {
                Some(definition)
            }
            _ => None,
        }
    }

    fn intern_token(&mut self, token: Token) -> LowerResult<Symbol> {
        let text = token.text(&self.module.syntax.source);
        self.package
            .names
            .symbols
            .intern(text)
            .map_err(|error| capacity_error(error, token.span))
    }

    fn intern(&mut self, text: &str, span: Span) -> LowerResult<Symbol> {
        self.package
            .names
            .symbols
            .intern(text)
            .map_err(|error| capacity_error(error, span))
    }

    fn source_origin(&mut self, span: Span) -> LowerResult<OriginId> {
        self.alloc_origin(Origin {
            span,
            parent: None,
            kind: OriginKind::Source,
        })
    }

    fn desugared_origin(
        &mut self,
        span: Span,
        parent: OriginId,
        kind: DesugaringKind,
    ) -> LowerResult<OriginId> {
        self.alloc_origin(Origin {
            span,
            parent: Some(parent),
            kind: OriginKind::Desugared(kind),
        })
    }

    fn alloc_origin(&mut self, value: Origin) -> LowerResult<OriginId> {
        self.package
            .origins
            .alloc(value)
            .map_err(|error| capacity_error(error, value.span))
    }

    fn alloc_scope(&mut self, value: Scope) -> LowerResult<ScopeId> {
        let span = self.package.origins[value.origin].span;
        self.package
            .scopes
            .alloc(value)
            .map_err(|error| capacity_error(error, span))
    }

    fn alloc_local(&mut self, value: Local) -> LowerResult<LocalId> {
        let span = self.package.origins[value.origin].span;
        self.package
            .locals
            .alloc(value)
            .map_err(|error| capacity_error(error, span))
    }

    fn alloc_type_ref(&mut self, value: TypeRef) -> LowerResult<TypeRefId> {
        let span = self.package.origins[value.origin].span;
        self.package
            .type_refs
            .alloc(value)
            .map_err(|error| capacity_error(error, span))
    }

    fn alloc_type_parameter(&mut self, value: TypeParameter) -> LowerResult<TypeParamId> {
        let span = self.package.origins[value.origin].span;
        self.package
            .type_parameters
            .alloc(value)
            .map_err(|error| capacity_error(error, span))
    }

    fn alloc_field(&mut self, value: Field) -> LowerResult<FieldId> {
        let span = self.package.origins[value.origin].span;
        self.package
            .fields
            .alloc(value)
            .map_err(|error| capacity_error(error, span))
    }

    fn alloc_variant(&mut self, value: Variant) -> LowerResult<VariantId> {
        let span = self.package.origins[value.origin].span;
        self.package
            .variants
            .alloc(value)
            .map_err(|error| capacity_error(error, span))
    }

    fn alloc_expression(&mut self, value: Expression) -> LowerResult<ExprId> {
        let span = self.package.origins[value.origin].span;
        self.package
            .expressions
            .alloc(value)
            .map_err(|error| capacity_error(error, span))
    }

    fn alloc_statement(&mut self, value: Statement) -> LowerResult<StmtId> {
        let span = self.package.origins[value.origin].span;
        self.package
            .statements
            .alloc(value)
            .map_err(|error| capacity_error(error, span))
    }

    fn alloc_body(&mut self, value: Body) -> LowerResult<BodyId> {
        let span = self.package.origins[value.origin].span;
        self.package
            .bodies
            .alloc(value)
            .map_err(|error| capacity_error(error, span))
    }

    fn alloc_match_arm(&mut self, value: MatchArm) -> LowerResult<MatchArmId> {
        let span = self.package.origins[value.origin].span;
        self.package
            .match_arms
            .alloc(value)
            .map_err(|error| capacity_error(error, span))
    }

    fn alloc_item(&mut self, value: HirItem) -> LowerResult<HirItemId> {
        let span = self.package.origins[value.origin].span;
        self.package
            .items
            .alloc(value)
            .map_err(|error| capacity_error(error, span))
    }

    fn symbol_list(&mut self, values: &[Symbol], span: Span) -> LowerResult<IdRange<Symbol>> {
        self.package
            .lists
            .symbols
            .extend(values)
            .map_err(|error| capacity_error(error, span))
    }

    fn type_ref_list(
        &mut self,
        values: &[TypeRefId],
        span: Span,
    ) -> LowerResult<IdRange<TypeRefId>> {
        self.package
            .lists
            .type_refs
            .extend(values)
            .map_err(|error| capacity_error(error, span))
    }

    fn local_list(&mut self, values: &[LocalId], span: Span) -> LowerResult<IdRange<LocalId>> {
        self.package
            .lists
            .locals
            .extend(values)
            .map_err(|error| capacity_error(error, span))
    }

    fn expression_list(&mut self, values: &[ExprId], span: Span) -> LowerResult<IdRange<ExprId>> {
        self.package
            .lists
            .expressions
            .extend(values)
            .map_err(|error| capacity_error(error, span))
    }

    fn statement_list(&mut self, values: &[StmtId], span: Span) -> LowerResult<IdRange<StmtId>> {
        self.package
            .lists
            .statements
            .extend(values)
            .map_err(|error| capacity_error(error, span))
    }

    fn field_list(&mut self, values: &[FieldId], span: Span) -> LowerResult<IdRange<FieldId>> {
        self.package
            .lists
            .fields
            .extend(values)
            .map_err(|error| capacity_error(error, span))
    }

    fn variant_list(
        &mut self,
        values: &[VariantId],
        span: Span,
    ) -> LowerResult<IdRange<VariantId>> {
        self.package
            .lists
            .variants
            .extend(values)
            .map_err(|error| capacity_error(error, span))
    }

    fn item_list(&mut self, values: &[HirItemId], span: Span) -> LowerResult<IdRange<HirItemId>> {
        self.package
            .lists
            .items
            .extend(values)
            .map_err(|error| capacity_error(error, span))
    }

    fn match_arm_list(
        &mut self,
        values: &[MatchArmId],
        span: Span,
    ) -> LowerResult<IdRange<MatchArmId>> {
        self.package
            .lists
            .match_arms
            .extend(values)
            .map_err(|error| capacity_error(error, span))
    }

    fn type_parameter_list(
        &mut self,
        values: &[TypeParamId],
        span: Span,
    ) -> LowerResult<IdRange<TypeParamId>> {
        self.package
            .lists
            .type_parameters
            .extend(values)
            .map_err(|error| capacity_error(error, span))
    }

    fn peek(&self, cursor: &mut Cursor) -> Option<Token> {
        while cursor.position < cursor.end
            && self.module.syntax.tokens[cursor.position].kind.is_trivia()
        {
            cursor.position += 1;
        }
        (cursor.position < cursor.end).then(|| self.module.syntax.tokens[cursor.position])
    }

    fn peek_kind(&self, cursor: &mut Cursor) -> Option<TokenKind> {
        self.peek(cursor).map(|token| token.kind)
    }

    fn peek_span(&self, cursor: &mut Cursor) -> Option<Span> {
        self.peek(cursor).map(|token| token.span)
    }

    fn advance(&self, cursor: &mut Cursor) -> Option<Token> {
        let token = self.peek(cursor)?;
        cursor.position += 1;
        Some(token)
    }

    fn consume(&self, cursor: &mut Cursor, kind: TokenKind) -> bool {
        if self.peek_kind(cursor) == Some(kind) {
            cursor.position += 1;
            true
        } else {
            false
        }
    }

    fn consume_keyword(&self, cursor: &mut Cursor, keyword: Keyword) -> bool {
        self.consume(cursor, TokenKind::Keyword(keyword))
    }

    fn expect(
        &self,
        cursor: &mut Cursor,
        kind: TokenKind,
        message: &'static str,
    ) -> LowerResult<Token> {
        let token = self
            .advance(cursor)
            .ok_or_else(|| LowerError::new("E3100", message, self.cursor_span(cursor)))?;
        if token.kind == kind {
            Ok(token)
        } else {
            Err(LowerError::new("E3100", message, token.span))
        }
    }

    fn expect_keyword(
        &self,
        cursor: &mut Cursor,
        keyword: Keyword,
        message: &'static str,
    ) -> LowerResult<Token> {
        self.expect(cursor, TokenKind::Keyword(keyword), message)
    }

    fn expect_identifier(&self, cursor: &mut Cursor, message: &'static str) -> LowerResult<Token> {
        self.expect(cursor, TokenKind::Identifier, message)
    }

    fn at_end(&self, cursor: &mut Cursor) -> bool {
        self.peek(cursor).is_none()
    }

    fn expect_end(&self, cursor: &mut Cursor, message: &'static str) -> LowerResult<()> {
        if let Some(token) = self.peek(cursor) {
            Err(LowerError::new("E3100", message, token.span))
        } else {
            Ok(())
        }
    }

    fn previous_span(&self, cursor: &Cursor) -> Option<Span> {
        cursor.position.checked_sub(1).and_then(|index| {
            (index >= cursor.start).then(|| self.module.syntax.tokens[index].span)
        })
    }

    fn cursor_span(&self, cursor: &Cursor) -> Span {
        self.previous_span(cursor).unwrap_or(Span {
            file: self.module.file,
            start: 0,
            end: 0,
        })
    }

    fn token_text(&self, token: Token) -> &str {
        token.text(&self.module.syntax.source)
    }
}

#[derive(Clone, Copy)]
struct Cursor {
    start: usize,
    end: usize,
    position: usize,
}

impl Cursor {
    const fn new(range: TokenRange) -> Self {
        Self {
            start: range.start,
            end: range.end,
            position: range.start,
        }
    }
}

type LowerResult<T> = Result<T, LowerError>;

struct LowerError {
    code: &'static str,
    message: String,
    span: Span,
}

impl LowerError {
    fn new(code: &'static str, message: impl Into<String>, span: Span) -> Self {
        Self {
            code,
            message: message.into(),
            span,
        }
    }

    fn into_diagnostic(self) -> Diagnostic {
        Diagnostic::error(self.code, self.message)
            .expect("HIR lowering diagnostic codes are valid")
            .with_primary_span(self.span)
    }
}

fn binary_operator(kind: TokenKind) -> Option<(BinaryOperator, u8)> {
    Some(match kind {
        TokenKind::LogicalAnd => (BinaryOperator::LogicalAnd, 1),
        TokenKind::LogicalOr => (BinaryOperator::LogicalOr, 1),
        TokenKind::EqualEqual => (BinaryOperator::Equal, 2),
        TokenKind::NotEqual => (BinaryOperator::NotEqual, 2),
        TokenKind::Less => (BinaryOperator::Less, 2),
        TokenKind::Greater => (BinaryOperator::Greater, 2),
        TokenKind::LessEqual => (BinaryOperator::LessEqual, 2),
        TokenKind::GreaterEqual => (BinaryOperator::GreaterEqual, 2),
        TokenKind::Plus => (BinaryOperator::Add, 3),
        TokenKind::Minus => (BinaryOperator::Subtract, 3),
        TokenKind::BitAnd => (BinaryOperator::BitAnd, 3),
        TokenKind::BitOr => (BinaryOperator::BitOr, 3),
        TokenKind::BitXor => (BinaryOperator::BitXor, 3),
        TokenKind::ShiftLeft => (BinaryOperator::ShiftLeft, 3),
        TokenKind::ShiftRight => (BinaryOperator::ShiftRight, 3),
        TokenKind::Star => (BinaryOperator::Multiply, 4),
        TokenKind::Slash => (BinaryOperator::Divide, 4),
        TokenKind::Percent => (BinaryOperator::Modulo, 4),
        _ => return None,
    })
}

fn capacity_error(error: impl std::fmt::Display, span: Span) -> LowerError {
    LowerError::new("E3199", error.to_string(), span)
}

fn capacity_diagnostic(message: String, span: Span) -> Diagnostic {
    Diagnostic::error("E3199", message)
        .expect("HIR lowering diagnostic code is valid")
        .with_primary_span(span)
}

fn module_span(module: &Module) -> Span {
    Span {
        file: module.file,
        start: 0,
        end: module.syntax.source.len().min(u32::MAX as usize) as u32,
    }
}
