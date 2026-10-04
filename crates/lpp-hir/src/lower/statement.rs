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
            SyntaxKind::For => self.lower_for(node)?,
            SyntaxKind::Match => self.lower_match(node)?,
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
                if !(2..=4).contains(&names.len()) {
                    return Err(LowerError::new(
                        "E3100",
                        "tuple destructuring requires between two and four names",
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

    fn lower_for(&mut self, node: &SyntaxNode) -> LowerResult<StatementKind> {
        let mut cursor = Cursor::new(node.tokens);
        self.expect_keyword(&mut cursor, Keyword::For, "expected for")?;
        let binding_token = self.expect_identifier(&mut cursor, "expected loop binding")?;
        let binding_name = self.intern_token(binding_token)?;
        self.expect_keyword(&mut cursor, Keyword::In, "expected 'in' in for loop")?;
        let iterable = self.parse_expression(&mut cursor, None)?;
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

    fn lower_match(&mut self, node: &SyntaxNode) -> LowerResult<StatementKind> {
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
        let mut arms = Vec::with_capacity(node.children.len());
        for child in &node.children {
            if child.kind != SyntaxKind::MatchArm {
                return Err(LowerError::new("E3103", "expected a match arm", child.span));
            }
            arms.push(self.lower_match_arm(child)?);
        }
        Ok(StatementKind::Match {
            subject,
            arms: self.match_arm_list(&arms, node.span)?,
        })
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
