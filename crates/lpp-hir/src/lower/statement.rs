use super::*;

impl<'module, 'package> ModuleLowerer<'module, 'package> {
    pub(super) fn lower_statements(&mut self, nodes: &[SyntaxNode]) -> LowerResult<Vec<StmtId>> {
        let mut statements = Vec::with_capacity(nodes.len());
        let mut index = 0;
        while index < nodes.len() {
            match nodes[index].kind {
                SyntaxKind::If => {
                    let mut end = index + 1;
                    while end < nodes.len() && nodes[end].kind == SyntaxKind::Elif {
                        end += 1;
                    }
                    if end < nodes.len() && nodes[end].kind == SyntaxKind::Else {
                        end += 1;
                    }
                    statements.push(self.lower_if_chain(&nodes[index..end], 0)?);
                    index = end;
                }
                SyntaxKind::Match => {
                    self.lower_match_into(&nodes[index], &mut statements)?;
                    index += 1;
                }
                SyntaxKind::Elif | SyntaxKind::Else | SyntaxKind::MatchArm => {
                    return Err(LowerError::new(
                        "E3103",
                        "orphaned branch or match arm",
                        nodes[index].span,
                    ));
                }
                _ => {
                    statements.push(self.lower_statement(&nodes[index])?);
                    index += 1;
                }
            }
        }
        Ok(statements)
    }

    fn lower_statement(&mut self, node: &SyntaxNode) -> LowerResult<StmtId> {
        let origin = self.source_origin(node.span)?;
        let kind = match node.kind {
            SyntaxKind::While => self.lower_while(node)?,
            SyntaxKind::Unsafe => self.lower_unsafe(node)?,
            SyntaxKind::For => self.lower_for(node)?,
            SyntaxKind::Match => {
                let mut stmts = Vec::new();
                self.lower_match_into(node, &mut stmts)?;
                if stmts.len() == 1 {
                    return Ok(stmts[0]);
                }
                let parent = self.current_scope();
                let scope = self.begin_scope(parent, origin)?;
                let sr = self.statement_list(&stmts, node.span)?;
                let body = self.finish_body(scope, sr, origin)?;
                let true_cond = self.make_bool_expr(true, node.span)?;
                StatementKind::If {
                    condition: true_cond,
                    then_body: body,
                    else_body: None,
                }
            }
            SyntaxKind::Return => {
                let mut cursor = Cursor::new(node.tokens);
                self.expect_keyword(&mut cursor, Keyword::Return, "expected return")?;
                let value = if self.at_end(&mut cursor) {
                    None
                } else {
                    Some(self.parse_expression(&mut cursor, None)?)
                };
                self.expect_end(&mut cursor, "unexpected token after return value")?;
                StatementKind::Return(value)
            }
            SyntaxKind::Break => StatementKind::Break,
            SyntaxKind::Continue => StatementKind::Continue,
            SyntaxKind::Binding | SyntaxKind::Assignment => {
                return self.lower_binding_or_assignment(node, origin);
            }
            SyntaxKind::Expression => {
                let mut cursor = Cursor::new(node.tokens);
                let value = self.parse_expression(&mut cursor, Some(&node.children))?;
                self.expect_end(&mut cursor, "unexpected token after expression")?;
                StatementKind::Expression(value)
            }
            SyntaxKind::Function
            | SyntaxKind::Struct
            | SyntaxKind::Enum
            | SyntaxKind::Trait
            | SyntaxKind::Impl
            | SyntaxKind::Extern
            | SyntaxKind::Const
            | SyntaxKind::TypeAlias
            | SyntaxKind::Import
            | SyntaxKind::FromImport
            | SyntaxKind::Attribute => {
                return Err(LowerError::new(
                    "E3101",
                    "declaration is not valid in a function body",
                    node.span,
                ));
            }
            SyntaxKind::If | SyntaxKind::Elif | SyntaxKind::Else | SyntaxKind::MatchArm => {
                return Err(LowerError::new(
                    "E3103",
                    "branch node must be lowered with its surrounding block",
                    node.span,
                ));
            }
        };
        self.alloc_statement(Statement { kind, origin })
    }

    fn lower_binding_or_assignment(
        &mut self,
        node: &SyntaxNode,
        origin: OriginId,
    ) -> LowerResult<StmtId> {
        let mut cursor = Cursor::new(node.tokens);
        let mutable = self.consume_keyword(&mut cursor, Keyword::Mut);

        if self.peek_kind(&mut cursor) == Some(TokenKind::LeftParen) {
            let saved = cursor;
            self.advance(&mut cursor);
            let mut names = Vec::new();
            while let Some(token) = self.peek(&mut cursor) {
                if token.kind != TokenKind::Identifier {
                    break;
                }
                self.advance(&mut cursor);
                names.push(token);
                if !self.consume(&mut cursor, TokenKind::Comma) {
                    break;
                }
            }
            if self.consume(&mut cursor, TokenKind::RightParen)
                && self.consume(&mut cursor, TokenKind::Assign)
            {
                if !(2..=16).contains(&names.len()) {
                    return Err(LowerError::new(
                        "E3100",
                        "tuple destructuring requires between two and sixteen names",
                        node.span,
                    ));
                }
                let value = self.parse_expression(&mut cursor, Some(&node.children))?;
                self.expect_end(&mut cursor, "unexpected token after destructuring value")?;
                let mut bindings = Vec::with_capacity(names.len());
                for token in names {
                    let local_origin = self.source_origin(token.span)?;
                    let symbol = self.intern_token(token)?;
                    bindings.push(self.declare_local(
                        symbol,
                        mutable,
                        LocalKind::Binding,
                        None,
                        None,
                        local_origin,
                    )?);
                }
                let bindings = self.local_list(&bindings, node.span)?;
                return self.alloc_statement(Statement {
                    kind: StatementKind::Let { bindings, value },
                    origin,
                });
            }
            cursor = saved;
        }

        if self.peek_kind(&mut cursor) == Some(TokenKind::Identifier) {
            let saved = cursor;
            let name_token = self.advance(&mut cursor).expect("peeked identifier");
            let type_ref = if self.consume(&mut cursor, TokenKind::Colon) {
                Some(self.parse_type_ref(&mut cursor)?)
            } else {
                None
            };
            if self.consume(&mut cursor, TokenKind::Assign)
                || (type_ref.is_some() && self.consume(&mut cursor, TokenKind::Equal))
            {
                let value = self.parse_expression(&mut cursor, Some(&node.children))?;
                self.expect_end(&mut cursor, "unexpected token after binding initializer")?;
                let local_origin = self.source_origin(name_token.span)?;
                let symbol = self.intern_token(name_token)?;
                let local = self.declare_local(
                    symbol,
                    mutable,
                    LocalKind::Binding,
                    type_ref,
                    None,
                    local_origin,
                )?;
                let bindings = self.local_list(&[local], node.span)?;
                return self.alloc_statement(Statement {
                    kind: StatementKind::Let { bindings, value },
                    origin,
                });
            }
            cursor = saved;
        }

        if mutable {
            return Err(LowerError::new(
                "E3100",
                "'mut' must introduce a local binding",
                node.span,
            ));
        }
        let target = self.parse_expression(&mut cursor, None)?;
        let operator_token = self
            .advance(&mut cursor)
            .ok_or_else(|| LowerError::new("E3100", "expected assignment operator", node.span))?;
        let augmented = match operator_token.kind {
            TokenKind::PlusEqual => Some(BinaryOperator::Add),
            TokenKind::MinusEqual => Some(BinaryOperator::Subtract),
            TokenKind::StarEqual => Some(BinaryOperator::Multiply),
            TokenKind::SlashEqual => Some(BinaryOperator::Divide),
            TokenKind::PercentEqual => Some(BinaryOperator::Modulo),
            TokenKind::Equal => None,
            _ => {
                return Err(LowerError::new(
                    "E3100",
                    "expected assignment operator",
                    operator_token.span,
                ));
            }
        };
        if !self.is_assignment_target(target) {
            return Err(LowerError::new(
                "E3102",
                "assignment target must be a local, field, or index",
                self.package.origins[self.package.expressions[target].origin].span,
            ));
        }
        let right = self.parse_expression(&mut cursor, Some(&node.children))?;
        self.expect_end(&mut cursor, "unexpected token after assignment value")?;
        let value = if let Some(operator) = augmented {
            let right_span = self.package.origins[self.package.expressions[right].origin].span;
            let desugared = self.desugared_origin(
                Span {
                    file: node.span.file,
                    start: self.package.origins[self.package.expressions[target].origin]
                        .span
                        .start,
                    end: right_span.end,
                },
                origin,
                DesugaringKind::AugmentedAssignment,
            )?;
            self.alloc_expression(Expression {
                kind: ExpressionKind::Binary {
                    left: target,
                    operator,
                    right,
                },
                origin: desugared,
            })?
        } else {
            right
        };
        self.alloc_statement(Statement {
            kind: StatementKind::Assign { target, value },
            origin,
        })
    }

    fn lower_if_chain(&mut self, nodes: &[SyntaxNode], index: usize) -> LowerResult<StmtId> {
        let node = &nodes[index];
        let source_origin = self.source_origin(node.span)?;
        let statement_origin = if node.kind == SyntaxKind::Elif {
            self.desugared_origin(node.span, source_origin, DesugaringKind::Elif)?
        } else {
            source_origin
        };
        let mut cursor = Cursor::new(node.tokens);
        match node.kind {
            SyntaxKind::If => {
                self.expect_keyword(&mut cursor, Keyword::If, "expected if")?;
            }
            SyntaxKind::Elif => {
                self.expect_keyword(&mut cursor, Keyword::Elif, "expected elif")?;
            }
            _ => unreachable!("if-chain entry is if or elif"),
        }
        let condition = self.parse_expression(&mut cursor, None)?;
        self.expect(
            &mut cursor,
            TokenKind::Colon,
            "expected ':' after condition",
        )?;
        self.expect_end(&mut cursor, "unexpected token after condition")?;
        let then_body = self.lower_child_body(&node.children, source_origin)?;

        let else_body = if index + 1 < nodes.len() {
            let next = &nodes[index + 1];
            if next.kind == SyntaxKind::Else {
                let mut else_cursor = Cursor::new(next.tokens);
                self.expect_keyword(&mut else_cursor, Keyword::Else, "expected else")?;
                self.expect(
                    &mut else_cursor,
                    TokenKind::Colon,
                    "expected ':' after else",
                )?;
                self.expect_end(&mut else_cursor, "unexpected token after else")?;
                let else_origin = self.source_origin(next.span)?;
                Some(self.lower_child_body(&next.children, else_origin)?)
            } else {
                let wrapper_origin = self.source_origin(next.span)?;
                let parent = self.current_scope();
                let scope = self.begin_scope(parent, wrapper_origin)?;
                let nested = self.lower_if_chain(nodes, index + 1)?;
                let statements = self.statement_list(&[nested], next.span)?;
                Some(self.finish_body(scope, statements, wrapper_origin)?)
            }
        } else {
            None
        };
        self.alloc_statement(Statement {
            kind: StatementKind::If {
                condition,
                then_body,
                else_body,
            },
            origin: statement_origin,
        })
    }

    fn lower_while(&mut self, node: &SyntaxNode) -> LowerResult<StatementKind> {
        let mut cursor = Cursor::new(node.tokens);
        self.expect_keyword(&mut cursor, Keyword::While, "expected while")?;
        let condition = self.parse_expression(&mut cursor, None)?;
        self.expect(
            &mut cursor,
            TokenKind::Colon,
            "expected ':' after while condition",
        )?;
        self.expect_end(&mut cursor, "unexpected token after while condition")?;
        let origin = self.source_origin(node.span)?;
        let body = self.lower_child_body(&node.children, origin)?;
        Ok(StatementKind::While { condition, body })
    }

    fn lower_unsafe(&mut self, node: &SyntaxNode) -> LowerResult<StatementKind> {
        let mut cursor = Cursor::new(node.tokens);
        self.expect_keyword(&mut cursor, Keyword::Unsafe, "expected unsafe")?;
        self.expect(&mut cursor, TokenKind::Colon, "expected ':' after unsafe")?;
        self.expect_end(&mut cursor, "unexpected token after 'unsafe:'")?;
        let origin = self.source_origin(node.span)?;
        let body = self.lower_child_body(&node.children, origin)?;
        Ok(StatementKind::Unsafe { body })
    }

    fn lower_for(&mut self, node: &SyntaxNode) -> LowerResult<StatementKind> {
        let mut cursor = Cursor::new(node.tokens);
        self.expect_keyword(&mut cursor, Keyword::For, "expected for")?;
        let binding_token = self.expect_identifier(&mut cursor, "expected loop binding")?;
        let binding_name = self.intern_token(binding_token)?;
        self.expect_keyword(&mut cursor, Keyword::In, "expected 'in' in for loop")?;
        let mut iterable = self.parse_expression(&mut cursor, None)?;
        let inclusive = if self.consume(&mut cursor, TokenKind::DotDotEqual) {
            Some(true)
        } else if self.consume(&mut cursor, TokenKind::DotDot) {
            Some(false)
        } else {
            None
        };
        if let Some(inc) = inclusive {
            let hi = self.parse_expression(&mut cursor, None)?;
            iterable = self.desugar_for_range(iterable, hi, inc, node.span)?;
        }
        self.expect(
            &mut cursor,
            TokenKind::Colon,
            "expected ':' after for iterable",
        )?;
        self.expect_end(&mut cursor, "unexpected token after for iterable")?;

        let origin = self.source_origin(node.span)?;
        let parent = self.current_scope();
        let scope = self.begin_scope(parent, origin)?;
        let binding_origin = self.source_origin(binding_token.span)?;
        let binding = self.declare_local(
            binding_name,
            false,
            LocalKind::Loop,
            None,
            None,
            binding_origin,
        )?;
        let statements = self.lower_statements(&node.children)?;
        let statements = self.statement_list(&statements, node.span)?;
        let body = self.finish_body(scope, statements, origin)?;
        Ok(StatementKind::For {
            binding,
            iterable,
            body,
        })
    }

    fn lower_match_into(&mut self, node: &SyntaxNode, out: &mut Vec<StmtId>) -> LowerResult<()> {
        let origin = self.source_origin(node.span)?;
        let mut cursor = Cursor::new(node.tokens);
        self.expect_keyword(&mut cursor, Keyword::Match, "expected match")?;
        let subject = self.parse_expression(&mut cursor, None)?;
        self.expect(
            &mut cursor,
            TokenKind::Colon,
            "expected ':' after match subject",
        )?;
        self.expect_end(&mut cursor, "unexpected token after match subject")?;
        if node.children.is_empty() {
            return Err(LowerError::new(
                "E3100",
                "match requires at least one arm",
                node.span,
            ));
        }

        let mut parsed_arms = Vec::with_capacity(node.children.len());
        let mut any_or = false;
        let mut any_guard = false;
        let mut any_value_pat = false;
        let mut any_enum_shape = false;

        for child in &node.children {
            if child.kind != SyntaxKind::MatchArm {
                return Err(LowerError::new("E3103", "expected a match arm", child.span));
            }
            let arm = self.parse_match_arm_header(child)?;
            if arm.alts.len() > 1 {
                any_or = true;
            }
            if arm.guard_start.is_some() {
                any_guard = true;
            }
            for alt in &arm.alts {
                match alt {
                    MatchPat::Literal(_) | MatchPat::Range { .. } => any_value_pat = true,
                    MatchPat::VariantOrIdent { path, bindings, .. } => {
                        if path.len() > 1 || !bindings.is_empty() {
                            any_enum_shape = true;
                        }
                    }
                    MatchPat::Wildcard => {}
                }
            }
            parsed_arms.push((child, arm));
        }

        // Pure enum match (or enum match with OR-patterns and no `if` guard):
        // emit `StatementKind::Match` directly so exhaustiveness and SwitchEnum stay intact.
        if !any_value_pat && !any_guard {
            let mut arms = Vec::with_capacity(parsed_arms.len());
            if !any_or {
                for (child, _) in &parsed_arms {
                    arms.push(self.lower_match_arm(child)?);
                }
            } else {
                for (child, arm) in &parsed_arms {
                    for alt in &arm.alts {
                        arms.push(self.build_enum_match_arm(child, alt)?);
                    }
                }
            }
            let arms_range = self.match_arm_list(&arms, node.span)?;
            let stmt = self.alloc_statement(Statement {
                kind: StatementKind::Match {
                    subject,
                    arms: arms_range,
                },
                origin,
            })?;
            out.push(stmt);
            return Ok(());
        }

        // Value match or guarded enum match:
        let treat_as_enum = any_enum_shape && !any_value_pat;
        if treat_as_enum {
            let match_stmt = self.build_guarded_enum_match(subject, &parsed_arms, node.span)?;
            out.push(match_stmt);
            return Ok(());
        }

        // Value match: bind subject once in current scope and build an explicit if/else
        // decision tree so MIR return-path analysis recognizes exhaustive tails.
        let subj_name = format!("__match_subj_{}", self.package.locals.len());
        let (subj_sym, subj_local, subj_bind) =
            self.make_let_local(&subj_name, false, LocalKind::Binding, subject, node.span)?;
        out.push(subj_bind);
        let tail_stmts =
            self.build_value_match_stmts(&parsed_arms, 0, subj_sym, subj_local, node.span)?;
        out.extend(tail_stmts);
        Ok(())
    }

    fn desugar_for_range(
        &mut self,
        lo: ExprId,
        hi: ExprId,
        inclusive: bool,
        span: Span,
    ) -> LowerResult<ExprId> {
        let origin = self.source_origin(span)?;
        let end = if inclusive {
            let one = self.alloc_expression(Expression {
                kind: ExpressionKind::Literal(Literal::Integer(1)),
                origin,
            })?;
            self.alloc_expression(Expression {
                kind: ExpressionKind::Binary {
                    left: hi,
                    operator: BinaryOperator::Add,
                    right: one,
                },
                origin,
            })?
        } else {
            hi
        };
        let range_sym = self.intern("range", span)?;
        let callee = self.alloc_expression(Expression {
            kind: ExpressionKind::Name {
                symbol: range_sym,
                binding: NameBinding::Unresolved,
            },
            origin,
        })?;
        let arguments = self.expression_list(&[lo, end], span)?;
        self.alloc_expression(Expression {
            kind: ExpressionKind::Call { callee, arguments },
            origin,
        })
    }

    fn parse_match_arm_header(&mut self, node: &SyntaxNode) -> LowerResult<ParsedArmHeader> {
        let mut cursor = Cursor::new(node.tokens);
        let mut alts = Vec::new();
        loop {
            alts.push(self.parse_single_pattern(&mut cursor)?);
            if !self.consume(&mut cursor, TokenKind::BitOr) {
                break;
            }
        }
        let guard_start = if self.consume_keyword(&mut cursor, Keyword::If) {
            let start_pos = cursor.position;
            let mut end_pos = cursor.end;
            // Find the trailing ':' token of this arm header without consuming it.
            while end_pos > start_pos && self.active_tokens()[end_pos - 1].kind.is_trivia() {
                end_pos -= 1;
            }
            if end_pos == 0 || self.active_tokens()[end_pos - 1].kind != TokenKind::Colon {
                return Err(LowerError::new(
                    "E3100",
                    "expected ':' after match guard",
                    node.span,
                ));
            }
            let colon_pos = end_pos - 1;
            if start_pos >= colon_pos {
                return Err(LowerError::new(
                    "E3100",
                    "expected boolean expression after 'if' in match guard",
                    node.span,
                ));
            }
            Some(Cursor {
                start: start_pos,
                position: start_pos,
                end: colon_pos,
            })
        } else {
            self.expect(
                &mut cursor,
                TokenKind::Colon,
                "expected ':' after match pattern",
            )?;
            self.expect_end(&mut cursor, "unexpected token after match pattern")?;
            None
        };
        Ok(ParsedArmHeader { alts, guard_start })
    }

    fn parse_single_pattern(&mut self, cursor: &mut Cursor) -> LowerResult<MatchPat> {
        if let Some(tok) = self.peek(cursor) {
            match tok.kind {
                TokenKind::Integer | TokenKind::Minus | TokenKind::Character => {
                    let lo = self.parse_pattern_atom(cursor)?;
                    if self.consume(cursor, TokenKind::DotDotEqual) {
                        let hi = self.parse_pattern_atom(cursor)?;
                        return Ok(MatchPat::Range {
                            lo,
                            hi,
                            inclusive: true,
                        });
                    }
                    if self.consume(cursor, TokenKind::DotDot) {
                        let hi = self.parse_pattern_atom(cursor)?;
                        return Ok(MatchPat::Range {
                            lo,
                            hi,
                            inclusive: false,
                        });
                    }
                    return Ok(MatchPat::Literal(lo));
                }
                TokenKind::Bool | TokenKind::String => {
                    let lit = self.parse_pattern_atom(cursor)?;
                    return Ok(MatchPat::Literal(lit));
                }
                _ => {}
            }
        }
        let first = self.expect_identifier(cursor, "expected match pattern")?;
        if self.token_text(first) == "_"
            && self.peek_kind(cursor) != Some(TokenKind::Dot)
            && self.peek_kind(cursor) != Some(TokenKind::LeftParen)
        {
            return Ok(MatchPat::Wildcard);
        }
        let mut path = vec![self.intern_token(first)?];
        let mut path_spans = vec![first.span];
        while self.consume(cursor, TokenKind::Dot) {
            let component = self.expect_identifier(cursor, "expected pattern component")?;
            path.push(self.intern_token(component)?);
            path_spans.push(component.span);
        }
        let mut bindings = Vec::new();
        if self.consume(cursor, TokenKind::LeftParen) {
            if self.peek_kind(cursor) != Some(TokenKind::RightParen) {
                loop {
                    bindings.push(self.expect_identifier(cursor, "expected pattern binding")?);
                    if !self.consume(cursor, TokenKind::Comma) {
                        break;
                    }
                }
            }
            self.expect(
                cursor,
                TokenKind::RightParen,
                "expected ')' after pattern bindings",
            )?;
        }
        Ok(MatchPat::VariantOrIdent {
            path,
            path_spans,
            bindings,
        })
    }

    fn parse_pattern_atom(&mut self, cursor: &mut Cursor) -> LowerResult<ExprId> {
        let tok = self.advance(cursor).ok_or_else(|| {
            LowerError::new(
                "E3100",
                "expected literal pattern",
                self.cursor_span(cursor),
            )
        })?;
        match tok.kind {
            TokenKind::Minus => {
                let int_tok = self.expect(
                    cursor,
                    TokenKind::Integer,
                    "expected integer literal after '-' in pattern",
                )?;
                let inner_lit = self.lower_int_token(int_tok)?;
                let span = Span {
                    file: tok.span.file,
                    start: tok.span.start,
                    end: int_tok.span.end,
                };
                let origin = self.source_origin(span)?;
                self.alloc_expression(Expression {
                    kind: ExpressionKind::Unary {
                        operator: UnaryOperator::Negate,
                        operand: inner_lit,
                    },
                    origin,
                })
            }
            TokenKind::Integer => self.lower_int_token(tok),
            TokenKind::Bool => {
                let origin = self.source_origin(tok.span)?;
                self.alloc_expression(Expression {
                    kind: ExpressionKind::Literal(Literal::Bool(self.token_text(tok) == "true")),
                    origin,
                })
            }
            TokenKind::Character => {
                let origin = self.source_origin(tok.span)?;
                self.alloc_expression(Expression {
                    kind: ExpressionKind::Literal(Literal::Character(tok.span)),
                    origin,
                })
            }
            TokenKind::String => {
                let origin = self.source_origin(tok.span)?;
                self.alloc_expression(Expression {
                    kind: ExpressionKind::Literal(Literal::String {
                        span: tok.span,
                        formatted: false,
                    }),
                    origin,
                })
            }
            _ => Err(LowerError::new(
                "E3100",
                "unsupported match pattern",
                tok.span,
            )),
        }
    }

    fn lower_int_token(&mut self, token: Token) -> LowerResult<ExprId> {
        let raw = self.token_text(token);
        let text = if raw.contains('_') {
            Cow::Owned(raw.replace('_', ""))
        } else {
            Cow::Borrowed(raw)
        };
        let value =
            if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
                i64::from_str_radix(hex, 16)
            } else if let Some(bin) = text.strip_prefix("0b").or_else(|| text.strip_prefix("0B")) {
                i64::from_str_radix(bin, 2)
            } else {
                text.parse::<i64>()
            }
            .map_err(|_| LowerError::new("E3104", "invalid integer literal", token.span))?;
        let origin = self.source_origin(token.span)?;
        self.alloc_expression(Expression {
            kind: ExpressionKind::Literal(Literal::Integer(value)),
            origin,
        })
    }

    fn build_enum_match_arm(
        &mut self,
        node: &SyntaxNode,
        alt: &MatchPat,
    ) -> LowerResult<MatchArmId> {
        let origin = self.source_origin(node.span)?;
        let (path, binding_tokens, wildcard) = match alt {
            MatchPat::Wildcard => (vec![self.intern("_", node.span)?], Vec::new(), true),
            MatchPat::VariantOrIdent { path, bindings, .. } => {
                (path.clone(), bindings.clone(), false)
            }
            _ => {
                return Err(LowerError::new(
                    "E3100",
                    "cannot mix literal patterns into an enum match without wildcards/guards",
                    node.span,
                ));
            }
        };
        let parent = self.current_scope();
        let scope = self.begin_scope(parent, origin)?;
        let mut bindings = Vec::with_capacity(binding_tokens.len());
        for token in binding_tokens {
            let symbol = self.intern_token(token)?;
            let binding_origin = self.source_origin(token.span)?;
            bindings.push(self.declare_local(
                symbol,
                false,
                LocalKind::Pattern,
                None,
                None,
                binding_origin,
            )?);
        }
        let statements = self.lower_statements(&node.children)?;
        let statements = self.statement_list(&statements, node.span)?;
        let body = self.finish_body(scope, statements, origin)?;
        let path = self.symbol_list(&path, node.span)?;
        let bindings = self.local_list(&bindings, node.span)?;
        self.alloc_match_arm(MatchArm {
            path,
            bindings,
            body,
            wildcard,
            origin,
        })
    }

    fn build_guarded_enum_match(
        &mut self,
        subject: ExprId,
        arms: &[(&SyntaxNode, ParsedArmHeader)],
        match_span: Span,
    ) -> LowerResult<StmtId> {
        let origin = self.source_origin(match_span)?;
        struct VariantBucket<'a> {
            variant_key: Symbol,
            path: Vec<Symbol>,
            canonical_tokens: Vec<Token>,
            // `(node, arm, tokens_for_this_clause)`
            clauses: Vec<(&'a SyntaxNode, &'a ParsedArmHeader, Vec<Token>)>,
        }

        let mut buckets: Vec<VariantBucket<'_>> = Vec::new();
        let mut any_wild = false;
        let mut wild_clauses: Vec<(&SyntaxNode, &ParsedArmHeader)> = Vec::new();

        for (node, arm) in arms {
            for alt in &arm.alts {
                match alt {
                    MatchPat::Wildcard => {
                        any_wild = true;
                        wild_clauses.push((node, arm));
                        for b in &mut buckets {
                            b.clauses.push((node, arm, Vec::new()));
                        }
                    }
                    MatchPat::VariantOrIdent { path, bindings, .. } => {
                        let key = *path.last().expect("variant path is non-empty");
                        if let Some(existing) = buckets.iter_mut().find(|b| b.variant_key == key) {
                            existing.clauses.push((node, arm, bindings.clone()));
                        } else {
                            // Any earlier wildcard arms that preceded this variant's first appearance apply first.
                            let mut init_clauses: Vec<(&SyntaxNode, &ParsedArmHeader, Vec<Token>)> =
                                wild_clauses
                                    .iter()
                                    .map(|&(wn, wa)| (wn, wa, Vec::new()))
                                    .collect();
                            init_clauses.push((node, arm, bindings.clone()));
                            buckets.push(VariantBucket {
                                variant_key: key,
                                path: path.clone(),
                                canonical_tokens: bindings.clone(),
                                clauses: init_clauses,
                            });
                        }
                    }
                    _ => {
                        return Err(LowerError::new(
                            "E3100",
                            "unsupported mixed literal and variant pattern",
                            node.span,
                        ));
                    }
                }
            }
        }

        let mut match_arms = Vec::with_capacity(buckets.len() + usize::from(any_wild));
        for bucket in buckets {
            let arm_origin = self.source_origin(bucket.clauses[0].0.span)?;
            let arm_scope = self.begin_scope(self.current_scope(), arm_origin)?;
            let mut canonical_locals = Vec::with_capacity(bucket.canonical_tokens.len());
            for token in &bucket.canonical_tokens {
                let sym = self.intern_token(*token)?;
                let bind_origin = self.source_origin(token.span)?;
                let local =
                    self.declare_local(sym, false, LocalKind::Pattern, None, None, bind_origin)?;
                canonical_locals.push((sym, local));
            }
            let stmts =
                self.build_variant_clause_chain(&bucket.clauses, 0, &canonical_locals, arm_origin)?;
            let sr = self.statement_list(&stmts, match_span)?;
            let body = self.finish_body(arm_scope, sr, arm_origin)?;
            let path = self.symbol_list(&bucket.path, match_span)?;
            let binding_ids: Vec<LocalId> = canonical_locals.iter().map(|&(_, id)| id).collect();
            let bindings = self.local_list(&binding_ids, match_span)?;
            match_arms.push(self.alloc_match_arm(MatchArm {
                path,
                bindings,
                body,
                wildcard: false,
                origin: arm_origin,
            })?);
        }

        if any_wild {
            let wild_origin = self.source_origin(wild_clauses[0].0.span)?;
            let wild_scope = self.begin_scope(self.current_scope(), wild_origin)?;
            let clauses: Vec<(&SyntaxNode, &ParsedArmHeader, Vec<Token>)> = wild_clauses
                .iter()
                .map(|&(wn, wa)| (wn, wa, Vec::new()))
                .collect();
            let stmts = self.build_variant_clause_chain(&clauses, 0, &[], wild_origin)?;
            let sr = self.statement_list(&stmts, match_span)?;
            let body = self.finish_body(wild_scope, sr, wild_origin)?;
            let wild_sym = self.intern("_", match_span)?;
            let path = self.symbol_list(&[wild_sym], match_span)?;
            let bindings = self.local_list(&[], match_span)?;
            match_arms.push(self.alloc_match_arm(MatchArm {
                path,
                bindings,
                body,
                wildcard: true,
                origin: wild_origin,
            })?);
        }

        let arms_range = self.match_arm_list(&match_arms, match_span)?;
        self.alloc_statement(Statement {
            kind: StatementKind::Match {
                subject,
                arms: arms_range,
            },
            origin,
        })
    }

    fn build_variant_clause_chain(
        &mut self,
        clauses: &[(&SyntaxNode, &ParsedArmHeader, Vec<Token>)],
        idx: usize,
        canonical_locals: &[(Symbol, LocalId)],
        fallback_origin: OriginId,
    ) -> LowerResult<Vec<StmtId>> {
        let (node, arm, tokens) = &clauses[idx];
        let origin = self.source_origin(node.span)?;
        // Bind clause pattern aliases (`let clause_tok = canonical_local`) inside an isolated
        // helper list when `tokens` differs from the arm's initial canonical locals.
        let mut alias_stmts = Vec::new();
        let bind_aliases = idx > 0 && !tokens.is_empty();
        if bind_aliases {
            if tokens.len() != canonical_locals.len() {
                return Err(LowerError::new(
                    "E3100",
                    "inconsistent variant pattern arity across match arms",
                    node.span,
                ));
            }
            for (tok, &(canon_sym, canon_local)) in tokens.iter().zip(canonical_locals.iter()) {
                let sym = self.intern_token(*tok)?;
                let bind_origin = self.source_origin(tok.span)?;
                let init = self.make_local_expr(canon_sym, canon_local, tok.span)?;
                let local = self.declare_local(
                    sym,
                    false,
                    LocalKind::Pattern,
                    None,
                    Some(init),
                    bind_origin,
                )?;
                let bindings = self.local_list(&[local], tok.span)?;
                alias_stmts.push(self.alloc_statement(Statement {
                    kind: StatementKind::Let {
                        bindings,
                        value: init,
                    },
                    origin: bind_origin,
                })?);
            }
        }

        if let Some(guard_cursor) = arm.guard_start {
            let mut gc = guard_cursor;
            let guard_expr = self.parse_expression(&mut gc, None)?;
            self.expect_end(&mut gc, "unexpected token in match guard")?;
            let then_scope = self.begin_scope(self.current_scope(), origin)?;
            let then_stmts = self.lower_statements(&node.children)?;
            let then_sr = self.statement_list(&then_stmts, node.span)?;
            let then_body = self.finish_body(then_scope, then_sr, origin)?;

            let else_body = if idx + 1 < clauses.len() {
                let else_scope = self.begin_scope(self.current_scope(), fallback_origin)?;
                let else_stmts = self.build_variant_clause_chain(
                    clauses,
                    idx + 1,
                    canonical_locals,
                    fallback_origin,
                )?;
                let else_sr = self.statement_list(&else_stmts, node.span)?;
                Some(self.finish_body(else_scope, else_sr, origin)?)
            } else {
                None
            };

            alias_stmts.push(self.alloc_statement(Statement {
                kind: StatementKind::If {
                    condition: guard_expr,
                    then_body,
                    else_body,
                },
                origin,
            })?);
            return Ok(alias_stmts);
        }

        alias_stmts.extend(self.lower_statements(&node.children)?);
        Ok(alias_stmts)
    }

    fn build_value_match_body(
        &mut self,
        arms: &[(&SyntaxNode, ParsedArmHeader)],
        idx: usize,
        subj_sym: Symbol,
        subj_local: LocalId,
        match_span: Span,
    ) -> LowerResult<BodyId> {
        let origin = self.source_origin(arms[idx].0.span)?;
        let scope = self.begin_scope(self.current_scope(), origin)?;
        let stmts = self.build_value_match_stmts(arms, idx, subj_sym, subj_local, match_span)?;
        let sr = self.statement_list(&stmts, match_span)?;
        self.finish_body(scope, sr, origin)
    }

    fn build_value_match_stmts(
        &mut self,
        arms: &[(&SyntaxNode, ParsedArmHeader)],
        idx: usize,
        subj_sym: Symbol,
        subj_local: LocalId,
        match_span: Span,
    ) -> LowerResult<Vec<StmtId>> {
        let (node, arm) = &arms[idx];
        let origin = self.source_origin(node.span)?;
        let mut conds = Vec::with_capacity(arm.alts.len());
        let mut catch_all_binder: Option<(Symbol, Span)> = None;
        let mut pure_wildcard = arm.alts.len() == 1 && matches!(arm.alts[0], MatchPat::Wildcard);

        for alt in &arm.alts {
            match alt {
                MatchPat::Wildcard => {
                    pure_wildcard = arm.alts.len() == 1;
                    conds.push(self.make_bool_expr(true, node.span)?);
                }
                MatchPat::Literal(lit) => {
                    let subj = self.make_local_expr(subj_sym, subj_local, node.span)?;
                    conds.push(self.alloc_expression(Expression {
                        kind: ExpressionKind::Binary {
                            left: subj,
                            operator: BinaryOperator::Equal,
                            right: *lit,
                        },
                        origin,
                    })?);
                }
                MatchPat::Range { lo, hi, inclusive } => {
                    let subj1 = self.make_local_expr(subj_sym, subj_local, node.span)?;
                    let subj2 = self.make_local_expr(subj_sym, subj_local, node.span)?;
                    let ge = self.alloc_expression(Expression {
                        kind: ExpressionKind::Binary {
                            left: subj1,
                            operator: BinaryOperator::GreaterEqual,
                            right: *lo,
                        },
                        origin,
                    })?;
                    let hi_cmp = self.alloc_expression(Expression {
                        kind: ExpressionKind::Binary {
                            left: subj2,
                            operator: if *inclusive {
                                BinaryOperator::LessEqual
                            } else {
                                BinaryOperator::Less
                            },
                            right: *hi,
                        },
                        origin,
                    })?;
                    conds.push(self.alloc_expression(Expression {
                        kind: ExpressionKind::Binary {
                            left: ge,
                            operator: BinaryOperator::LogicalAnd,
                            right: hi_cmp,
                        },
                        origin,
                    })?);
                }
                MatchPat::VariantOrIdent {
                    path,
                    path_spans,
                    bindings,
                } => {
                    if path.len() == 1 && bindings.is_empty() && arm.alts.len() == 1 {
                        catch_all_binder = Some((path[0], path_spans[0]));
                        conds.push(self.make_bool_expr(true, node.span)?);
                    } else {
                        return Err(LowerError::new(
                            "E3100",
                            "variant pattern cannot be used in a scalar/literal match",
                            node.span,
                        ));
                    }
                }
            }
        }

        // Unconditional wildcard `_:` or unguarded binder `x:`: becomes the unconditional tail.
        if (pure_wildcard || catch_all_binder.is_some()) && arm.guard_start.is_none() {
            let mut stmts = Vec::new();
            if let Some((bind_sym, bind_span)) = catch_all_binder {
                let bind_origin = self.source_origin(bind_span)?;
                let subj_val = self.make_local_expr(subj_sym, subj_local, bind_span)?;
                let local = self.declare_local(
                    bind_sym,
                    false,
                    LocalKind::Pattern,
                    None,
                    Some(subj_val),
                    bind_origin,
                )?;
                let bindings = self.local_list(&[local], bind_span)?;
                stmts.push(self.alloc_statement(Statement {
                    kind: StatementKind::Let {
                        bindings,
                        value: subj_val,
                    },
                    origin: bind_origin,
                })?);
            }
            stmts.extend(self.lower_statements(&node.children)?);
            return Ok(stmts);
        }

        // Build `else_body` BEFORE introducing any binder from this arm into scope.
        let else_body = if idx + 1 < arms.len() {
            Some(self.build_value_match_body(arms, idx + 1, subj_sym, subj_local, match_span)?)
        } else {
            None
        };

        if let Some((bind_sym, bind_span)) = catch_all_binder {
            // Binder + guard (e.g. `x if x > 100:`): bind `x` in an inner scope around the guard `if`.
            let wrap_scope = self.begin_scope(self.current_scope(), origin)?;
            let bind_origin = self.source_origin(bind_span)?;
            let subj_val = self.make_local_expr(subj_sym, subj_local, bind_span)?;
            let local = self.declare_local(
                bind_sym,
                false,
                LocalKind::Pattern,
                None,
                Some(subj_val),
                bind_origin,
            )?;
            let bindings = self.local_list(&[local], bind_span)?;
            let bind_stmt = self.alloc_statement(Statement {
                kind: StatementKind::Let {
                    bindings,
                    value: subj_val,
                },
                origin: bind_origin,
            })?;
            let guard_cursor = arm
                .guard_start
                .expect("binder fell through only when guarded");
            let mut gc = guard_cursor;
            let guard_expr = self.parse_expression(&mut gc, None)?;
            self.expect_end(&mut gc, "unexpected token in match guard")?;
            let then_scope = self.begin_scope(self.current_scope(), origin)?;
            let then_stmts = self.lower_statements(&node.children)?;
            let then_sr = self.statement_list(&then_stmts, node.span)?;
            let then_body = self.finish_body(then_scope, then_sr, origin)?;
            let if_stmt = self.alloc_statement(Statement {
                kind: StatementKind::If {
                    condition: guard_expr,
                    then_body,
                    else_body,
                },
                origin,
            })?;
            // Close the temporary wrapper scope after emitting `[bind_stmt, if_stmt]`.
            let sr = self.statement_list(&[bind_stmt, if_stmt], node.span)?;
            let _unused = sr;
            self.finish_scope(wrap_scope, node.span)?;
            return Ok(vec![bind_stmt, if_stmt]);
        }

        let mut pat_cond = conds[0];
        for &c in &conds[1..] {
            pat_cond = self.alloc_expression(Expression {
                kind: ExpressionKind::Binary {
                    left: pat_cond,
                    operator: BinaryOperator::LogicalOr,
                    right: c,
                },
                origin,
            })?;
        }

        let condition = if let Some(guard_cursor) = arm.guard_start {
            let mut gc = guard_cursor;
            let guard_expr = self.parse_expression(&mut gc, None)?;
            self.expect_end(&mut gc, "unexpected token in match guard")?;
            if pure_wildcard {
                guard_expr
            } else {
                self.alloc_expression(Expression {
                    kind: ExpressionKind::Binary {
                        left: pat_cond,
                        operator: BinaryOperator::LogicalAnd,
                        right: guard_expr,
                    },
                    origin,
                })?
            }
        } else {
            pat_cond
        };

        let then_scope = self.begin_scope(self.current_scope(), origin)?;
        let then_stmts = self.lower_statements(&node.children)?;
        let then_sr = self.statement_list(&then_stmts, node.span)?;
        let then_body = self.finish_body(then_scope, then_sr, origin)?;

        let if_stmt = self.alloc_statement(Statement {
            kind: StatementKind::If {
                condition,
                then_body,
                else_body,
            },
            origin,
        })?;
        Ok(vec![if_stmt])
    }

    fn make_bool_expr(&mut self, val: bool, span: Span) -> LowerResult<ExprId> {
        let origin = self.source_origin(span)?;
        self.alloc_expression(Expression {
            kind: ExpressionKind::Literal(Literal::Bool(val)),
            origin,
        })
    }

    fn make_local_expr(
        &mut self,
        symbol: Symbol,
        local: LocalId,
        span: Span,
    ) -> LowerResult<ExprId> {
        let origin = self.source_origin(span)?;
        self.alloc_expression(Expression {
            kind: ExpressionKind::Name {
                symbol,
                binding: NameBinding::Local(local),
            },
            origin,
        })
    }

    fn make_let_local(
        &mut self,
        name: &str,
        mutable: bool,
        kind: LocalKind,
        value: ExprId,
        span: Span,
    ) -> LowerResult<(Symbol, LocalId, StmtId)> {
        let origin = self.source_origin(span)?;
        let sym = self.intern(name, span)?;
        let local = self.declare_local(sym, mutable, kind, None, Some(value), origin)?;
        let bindings = self.local_list(&[local], span)?;
        let stmt = self.alloc_statement(Statement {
            kind: StatementKind::Let { bindings, value },
            origin,
        })?;
        Ok((sym, local, stmt))
    }

    fn lower_match_arm(&mut self, node: &SyntaxNode) -> LowerResult<MatchArmId> {
        let origin = self.source_origin(node.span)?;
        let mut cursor = Cursor::new(node.tokens);
        let mut path = Vec::new();
        let first = self.expect_identifier(&mut cursor, "expected match pattern")?;
        let wildcard = self.token_text(first) == "_";
        path.push(self.intern_token(first)?);
        while self.consume(&mut cursor, TokenKind::Dot) {
            let component = self.expect_identifier(&mut cursor, "expected pattern component")?;
            path.push(self.intern_token(component)?);
        }
        let mut binding_tokens = Vec::new();
        if self.consume(&mut cursor, TokenKind::LeftParen) {
            if self.peek_kind(&mut cursor) != Some(TokenKind::RightParen) {
                loop {
                    binding_tokens
                        .push(self.expect_identifier(&mut cursor, "expected pattern binding")?);
                    if !self.consume(&mut cursor, TokenKind::Comma) {
                        break;
                    }
                }
            }
            self.expect(
                &mut cursor,
                TokenKind::RightParen,
                "expected ')' after pattern bindings",
            )?;
        }
        self.expect(
            &mut cursor,
            TokenKind::Colon,
            "expected ':' after match pattern",
        )?;
        self.expect_end(&mut cursor, "unexpected token after match pattern")?;

        let parent = self.current_scope();
        let scope = self.begin_scope(parent, origin)?;
        let mut bindings = Vec::with_capacity(binding_tokens.len());
        for token in binding_tokens {
            let symbol = self.intern_token(token)?;
            let binding_origin = self.source_origin(token.span)?;
            bindings.push(self.declare_local(
                symbol,
                false,
                LocalKind::Pattern,
                None,
                None,
                binding_origin,
            )?);
        }
        let statements = self.lower_statements(&node.children)?;
        let statements = self.statement_list(&statements, node.span)?;
        let body = self.finish_body(scope, statements, origin)?;
        let path = self.symbol_list(&path, node.span)?;
        let bindings = self.local_list(&bindings, node.span)?;
        self.alloc_match_arm(MatchArm {
            path,
            bindings,
            body,
            wildcard,
            origin,
        })
    }

    fn lower_child_body(&mut self, nodes: &[SyntaxNode], origin: OriginId) -> LowerResult<BodyId> {
        let parent = self.current_scope();
        let scope = self.begin_scope(parent, origin)?;
        let statements = self.lower_statements(nodes)?;
        let span = self.package.origins[origin].span;
        let statements = self.statement_list(&statements, span)?;
        self.finish_body(scope, statements, origin)
    }
}

struct ParsedArmHeader {
    alts: Vec<MatchPat>,
    guard_start: Option<Cursor>,
}

enum MatchPat {
    Wildcard,
    Literal(ExprId),
    Range {
        lo: ExprId,
        hi: ExprId,
        inclusive: bool,
    },
    VariantOrIdent {
        path: Vec<Symbol>,
        path_spans: Vec<Span>,
        bindings: Vec<Token>,
    },
}
