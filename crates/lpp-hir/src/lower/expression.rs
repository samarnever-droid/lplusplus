use super::*;

impl<'module, 'package> ModuleLowerer<'module, 'package> {
    pub(super) fn parse_expression(
        &mut self,
        cursor: &mut Cursor,
        closure_body: Option<&[SyntaxNode]>,
    ) -> LowerResult<ExprId> {
        if self.consume_keyword(cursor, Keyword::Fn) {
            return self.parse_closure(cursor, closure_body);
        }
        if let Some(token) = self.peek(cursor)
            && token.kind == TokenKind::Keyword(Keyword::Spawn)
        {
            let start = self.advance(cursor).expect("peeked spawn").span;
            let operand = self.parse_expression(cursor, closure_body)?;
            let end = self.package.origins[self.package.expressions[operand].origin]
                .span
                .end;
            let origin = self.source_origin(Span {
                file: start.file,
                start: start.start,
                end,
            })?;
            return self.alloc_expression(Expression {
                kind: ExpressionKind::Spawn(operand),
                origin,
            });
        }
        let then_expr = self.parse_binary(cursor, 1, closure_body)?;
        if self.consume_keyword(cursor, Keyword::If) {
            let condition = self.parse_expression(cursor, closure_body)?;
            self.expect_keyword(
                cursor,
                Keyword::Else,
                "expected 'else' in conditional expression",
            )?;
            let else_expr = self.parse_expression(cursor, closure_body)?;
            let then_span = self.package.origins[self.package.expressions[then_expr].origin].span;
            let else_span = self.package.origins[self.package.expressions[else_expr].origin].span;
            let origin = self.source_origin(Span {
                file: then_span.file,
                start: then_span.start,
                end: else_span.end,
            })?;
            return self.alloc_expression(Expression {
                kind: ExpressionKind::Conditional {
                    condition,
                    then_expr,
                    else_expr,
                },
                origin,
            });
        }
        Ok(then_expr)
    }

    fn parse_binary(
        &mut self,
        cursor: &mut Cursor,
        minimum_precedence: u8,
        closure_body: Option<&[SyntaxNode]>,
    ) -> LowerResult<ExprId> {
        let mut left = self.parse_cast(cursor, closure_body)?;
        while let Some((operator, precedence)) = self.peek_kind(cursor).and_then(binary_operator) {
            if precedence < minimum_precedence {
                break;
            }
            self.advance(cursor);
            let right = self.parse_binary(cursor, precedence + 1, closure_body)?;
            let left_span = self.package.origins[self.package.expressions[left].origin].span;
            let right_span = self.package.origins[self.package.expressions[right].origin].span;
            let origin = self.source_origin(Span {
                file: left_span.file,
                start: left_span.start,
                end: right_span.end,
            })?;
            left = self.alloc_expression(Expression {
                kind: ExpressionKind::Binary {
                    left,
                    operator,
                    right,
                },
                origin,
            })?;
        }
        Ok(left)
    }

    fn parse_cast(
        &mut self,
        cursor: &mut Cursor,
        closure_body: Option<&[SyntaxNode]>,
    ) -> LowerResult<ExprId> {
        let mut value = self.parse_unary(cursor, closure_body)?;
        while self.consume_keyword(cursor, Keyword::As) {
            let target = self.parse_type_ref(cursor)?;
            let value_span = self.package.origins[self.package.expressions[value].origin].span;
            let target_span = self.package.origins[self.package.type_refs[target].origin].span;
            let origin = self.source_origin(Span {
                file: value_span.file,
                start: value_span.start,
                end: target_span.end,
            })?;
            value = self.alloc_expression(Expression {
                kind: ExpressionKind::Cast { value, target },
                origin,
            })?;
        }
        Ok(value)
    }

    fn parse_unary(
        &mut self,
        cursor: &mut Cursor,
        closure_body: Option<&[SyntaxNode]>,
    ) -> LowerResult<ExprId> {
        let operator = match self.peek_kind(cursor) {
            Some(TokenKind::Minus) => Some(UnaryOperator::Negate),
            Some(TokenKind::Plus) => Some(UnaryOperator::Plus),
            Some(TokenKind::Not) => Some(UnaryOperator::Not),
            Some(TokenKind::BitNot) => Some(UnaryOperator::BitNot),
            _ => None,
        };
        if let Some(operator) = operator {
            let token = self.advance(cursor).expect("peeked unary operator");
            let operand = self.parse_unary(cursor, closure_body)?;
            let end = self.package.origins[self.package.expressions[operand].origin]
                .span
                .end;
            let origin = self.source_origin(Span {
                file: token.span.file,
                start: token.span.start,
                end,
            })?;
            return self.alloc_expression(Expression {
                kind: ExpressionKind::Unary { operator, operand },
                origin,
            });
        }
        let primary = self.parse_primary(cursor, closure_body)?;
        self.parse_postfix(cursor, primary, closure_body)
    }

    fn parse_primary(
        &mut self,
        cursor: &mut Cursor,
        closure_body: Option<&[SyntaxNode]>,
    ) -> LowerResult<ExprId> {
        if self.consume_keyword(cursor, Keyword::Fn) {
            return self.parse_closure(cursor, closure_body);
        }
        let token = self.advance(cursor).ok_or_else(|| {
            LowerError::new("E3100", "expected an expression", self.cursor_span(cursor))
        })?;
        // A formatted string (`f"...{expr}..."`) is desugared here into a
        // concatenation of its literal segments and interpolated expressions,
        // so nothing downstream needs a dedicated f-string node.
        if token.kind == TokenKind::FString {
            return self.lower_formatted_string(token);
        }
        let kind = match token.kind {
            TokenKind::Integer => {
                let raw = self.token_text(token);
                let (digits, suffix_type) = split_int_suffix(raw);
                let text = if digits.contains('_') {
                    Cow::Owned(digits.replace('_', ""))
                } else {
                    Cow::Borrowed(digits)
                };
                let value = if let Some(hex) =
                    text.strip_prefix("0x").or_else(|| text.strip_prefix("0X"))
                {
                    i64::from_str_radix(hex, 16)
                } else if let Some(octal) =
                    text.strip_prefix("0o").or_else(|| text.strip_prefix("0O"))
                {
                    i64::from_str_radix(octal, 8)
                } else if let Some(binary) =
                    text.strip_prefix("0b").or_else(|| text.strip_prefix("0B"))
                {
                    i64::from_str_radix(binary, 2)
                } else {
                    text.parse::<i64>()
                }
                .map_err(|_| LowerError::new("E3104", "invalid integer literal", token.span))?;
                if let Some(target_name) = suffix_type {
                    let origin = self.source_origin(token.span)?;
                    let lit = self.alloc_expression(Expression {
                        kind: ExpressionKind::Literal(Literal::Integer(value)),
                        origin,
                    })?;
                    let target = self.named_type(target_name, token.span)?;
                    return self.alloc_expression(Expression {
                        kind: ExpressionKind::Cast { value: lit, target },
                        origin,
                    });
                }
                ExpressionKind::Literal(Literal::Integer(value))
            }
            TokenKind::Float => {
                let raw = self.token_text(token);
                let digits = raw.strip_suffix("f64").unwrap_or(raw);
                let text = if digits.contains('_') {
                    Cow::Owned(digits.replace('_', ""))
                } else {
                    Cow::Borrowed(digits)
                };
                let value = text
                    .parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| LowerError::new("E3104", "invalid float literal", token.span))?;
                ExpressionKind::Literal(Literal::FloatBits(value.to_bits()))
            }
            TokenKind::String => ExpressionKind::Literal(Literal::String {
                span: token.span,
                formatted: false,
            }),
            TokenKind::RawString => ExpressionKind::Literal(Literal::RawString(token.span)),
            TokenKind::Character => ExpressionKind::Literal(Literal::Character(token.span)),
            TokenKind::Bool => {
                ExpressionKind::Literal(Literal::Bool(self.token_text(token) == "true"))
            }
            TokenKind::Identifier => {
                let symbol = self.intern_token(token)?;
                ExpressionKind::Name {
                    symbol,
                    binding: self.resolve_name(symbol),
                }
            }
            TokenKind::LeftParen => {
                if self.peek_kind(cursor) == Some(TokenKind::RightParen) {
                    return Err(LowerError::new(
                        "E3100",
                        "empty tuple expressions are not supported",
                        token.span,
                    ));
                }
                let first = self.parse_expression(cursor, closure_body)?;
                if self.consume(cursor, TokenKind::Comma) {
                    let mut elements = vec![first];
                    while self.peek_kind(cursor) != Some(TokenKind::RightParen) {
                        elements.push(self.parse_expression(cursor, closure_body)?);
                        if !self.consume(cursor, TokenKind::Comma) {
                            break;
                        }
                    }
                    let close =
                        self.expect(cursor, TokenKind::RightParen, "expected ')' after tuple")?;
                    if !(2..=16).contains(&elements.len()) {
                        return Err(LowerError::new(
                            "E3100",
                            "tuple expressions require between two and sixteen elements",
                            token.span,
                        ));
                    }
                    let origin = self.source_origin(Span {
                        file: token.span.file,
                        start: token.span.start,
                        end: close.span.end,
                    })?;
                    let elements = self.expression_list(&elements, token.span)?;
                    return self.alloc_expression(Expression {
                        kind: ExpressionKind::Tuple(elements),
                        origin,
                    });
                }
                let close = self.expect(
                    cursor,
                    TokenKind::RightParen,
                    "expected ')' after expression",
                )?;
                let origin = self.package.expressions[first].origin;
                self.package.origins[origin].span = Span {
                    file: token.span.file,
                    start: token.span.start,
                    end: close.span.end,
                };
                return Ok(first);
            }
            TokenKind::LeftBracket => {
                let mut elements = Vec::new();
                while self.peek_kind(cursor) != Some(TokenKind::RightBracket) {
                    elements.push(self.parse_expression(cursor, closure_body)?);
                    if !self.consume(cursor, TokenKind::Comma) {
                        break;
                    }
                }
                let close = self.expect(
                    cursor,
                    TokenKind::RightBracket,
                    "expected ']' after list literal",
                )?;
                let origin = self.source_origin(Span {
                    file: token.span.file,
                    start: token.span.start,
                    end: close.span.end,
                })?;
                let elements = self.expression_list(&elements, token.span)?;
                return self.alloc_expression(Expression {
                    kind: ExpressionKind::List(elements),
                    origin,
                });
            }
            _ => {
                return Err(LowerError::new(
                    "E3100",
                    "expected an expression",
                    token.span,
                ));
            }
        };
        let origin = self.source_origin(token.span)?;
        self.alloc_expression(Expression { kind, origin })
    }

    /// Desugar `f"...{expr}..."` into left-to-right string concatenation.
    ///
    /// Each literal run between interpolations becomes a `Literal::String`
    /// carrying the *inner* segment span (no surrounding quotes) with
    /// `formatted = true`, which tells the MIR builder to unescape the raw
    /// segment text directly. Each `{expr}` is lexed and parsed as a full L++
    /// expression in the active scope, and the pieces are folded with `+`.
    fn lower_formatted_string(&mut self, token: Token) -> LowerResult<ExprId> {
        let full = self.token_text(token).to_string();
        let base = token.span.start;
        let file = token.span.file;
        // The token text is `f"..."`; the interior lives at byte range
        // [2, len-1) — after the `f"` prefix and before the closing quote.
        let triple = full.starts_with("f\"\"\"") && full.ends_with("\"\"\"") && full.len() >= 7;
        let (content_start, end_idx) = if triple {
            (4usize, full.len() - 3)
        } else {
            if full.len() < 3 {
                return Err(LowerError::new(
                    "E3104",
                    "invalid formatted string",
                    token.span,
                ));
            }
            (2usize, full.len() - 1)
        };
        let bytes = full.as_bytes();
        let seg_span = |start: usize, end: usize| Span {
            file,
            start: base + start as u32,
            end: base + end as u32,
        };
        let mut parts: Vec<ExprId> = Vec::new();
        let mut seg_start = content_start;
        let mut i = content_start;
        while i < end_idx {
            match bytes[i] {
                // Keep an escape and its escaped sequence inside the literal run
                // (`\{`, `\n`, `\u{...}`, …) so neither `\{` nor `\u{...}` is
                // misread as an interpolation.
                b'\\' => {
                    if bytes.get(i + 1) == Some(&b'u') && bytes.get(i + 2) == Some(&b'{') {
                        let mut k = i + 3;
                        while k < end_idx && bytes[k] != b'}' {
                            k += 1;
                        }
                        i = if k < end_idx { k + 1 } else { i + 2 };
                    } else {
                        i += 2;
                    }
                }
                b'{' => {
                    if i > seg_start {
                        let span = seg_span(seg_start, i);
                        let origin = self.source_origin(span)?;
                        parts.push(self.alloc_expression(Expression {
                            kind: ExpressionKind::Literal(Literal::String {
                                span,
                                formatted: true,
                            }),
                            origin,
                        })?);
                    }
                    let interp_start = i + 1;
                    let mut j = interp_start;
                    let mut depth = 1usize;
                    let mut in_quote: Option<u8> = None;
                    while j < end_idx && depth > 0 {
                        let b = bytes[j];
                        if let Some(q) = in_quote {
                            if b == b'\\' {
                                j += 2;
                                continue;
                            }
                            if b == q {
                                in_quote = None;
                            }
                        } else {
                            match b {
                                b'"' | b'\'' => in_quote = Some(b),
                                b'{' => depth += 1,
                                b'}' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                _ => {}
                            }
                        }
                        j += 1;
                    }
                    if depth != 0 || j >= end_idx {
                        return Err(LowerError::new(
                            "E3104",
                            "unterminated interpolation in formatted string",
                            token.span,
                        ));
                    }
                    let expr_text = &full[interp_start..j];
                    let expr_span = seg_span(interp_start, j);
                    if expr_text.trim().is_empty() {
                        return Err(LowerError::new(
                            "E3105",
                            "empty interpolation in formatted string",
                            expr_span,
                        ));
                    }
                    let offset = base + interp_start as u32;
                    let lexed = lpp_frontend::lex(file, expr_text).map_err(|diags| {
                        let msg = diags
                            .into_iter()
                            .next()
                            .map(|d| d.message)
                            .unwrap_or_else(|| "invalid interpolation expression".to_string());
                        LowerError::new("E3105", msg, expr_span)
                    })?;
                    let sub_tokens: Vec<Token> = lexed
                        .tokens
                        .into_iter()
                        .filter(|t| {
                            !matches!(
                                t.kind,
                                TokenKind::Newline
                                    | TokenKind::Indent
                                    | TokenKind::Dedent
                                    | TokenKind::Eof
                            )
                        })
                        .map(|mut t| {
                            t.span = Span {
                                file,
                                start: offset + t.span.start,
                                end: offset + t.span.end,
                            };
                            t
                        })
                        .collect();
                    let prev_tokens = self.override_tokens.take();
                    self.override_tokens = Some(sub_tokens);
                    let mut sub_cursor = Cursor {
                        start: 0,
                        position: 0,
                        end: self.active_tokens().len(),
                    };
                    let parsed = self.parse_expression(&mut sub_cursor, None).and_then(|e| {
                        self.expect_end(
                            &mut sub_cursor,
                            "unexpected token in formatted-string interpolation",
                        )?;
                        Ok(e)
                    });
                    self.override_tokens = prev_tokens;
                    parts.push(parsed?);
                    i = j + 1;
                    seg_start = i;
                }
                _ => i += 1,
            }
        }
        if end_idx > seg_start {
            let span = seg_span(seg_start, end_idx);
            let origin = self.source_origin(span)?;
            parts.push(self.alloc_expression(Expression {
                kind: ExpressionKind::Literal(Literal::String {
                    span,
                    formatted: true,
                }),
                origin,
            })?);
        }
        // Fold every part onto a leading empty string so the left operand of
        // each concatenation is always a String. Codegen dispatches `+` to
        // `lpp_str_concat` on the LEFT operand and coerces a scalar RIGHT
        // operand (int/float/bool) to a string, so a leading interpolation
        // (`f"{n} left"`) and an empty f-string both concatenate correctly.
        let empty_span = seg_span(content_start, content_start);
        let empty_origin = self.source_origin(empty_span)?;
        let mut accumulator = self.alloc_expression(Expression {
            kind: ExpressionKind::Literal(Literal::String {
                span: empty_span,
                formatted: true,
            }),
            origin: empty_origin,
        })?;
        for &right in &parts {
            let left_span = self.package.origins[self.package.expressions[accumulator].origin].span;
            let right_span = self.package.origins[self.package.expressions[right].origin].span;
            let origin = self.source_origin(Span {
                file,
                start: left_span.start,
                end: right_span.end,
            })?;
            accumulator = self.alloc_expression(Expression {
                kind: ExpressionKind::Binary {
                    left: accumulator,
                    operator: BinaryOperator::Add,
                    right,
                },
                origin,
            })?;
        }
        Ok(accumulator)
    }

    fn parse_postfix(
        &mut self,
        cursor: &mut Cursor,
        mut expression: ExprId,
        closure_body: Option<&[SyntaxNode]>,
    ) -> LowerResult<ExprId> {
        loop {
            match self.peek_kind(cursor) {
                Some(TokenKind::Dot) => {
                    self.advance(cursor);
                    if self.consume_keyword(cursor, Keyword::Await) {
                        expression = self.wrap_postfix(
                            expression,
                            ExpressionKind::Await(expression),
                            cursor,
                        )?;
                    } else if self.peek_kind(cursor) == Some(TokenKind::Integer) {
                        let index = self.advance(cursor).expect("peeked tuple index");
                        let raw = self.token_text(index);
                        let text = if raw.contains('_') {
                            Cow::Owned(raw.replace('_', ""))
                        } else {
                            Cow::Borrowed(raw)
                        };
                        let value = text.parse::<i64>().map_err(|_| {
                            LowerError::new("E3104", "invalid tuple index", index.span)
                        })?;
                        let index_origin = self.source_origin(index.span)?;
                        let index = self.alloc_expression(Expression {
                            kind: ExpressionKind::Literal(Literal::Integer(value)),
                            origin: index_origin,
                        })?;
                        expression = self.wrap_postfix(
                            expression,
                            ExpressionKind::Index {
                                base: expression,
                                index,
                            },
                            cursor,
                        )?;
                    } else {
                        let field =
                            self.expect_identifier(cursor, "expected field name after '.'")?;
                        let name = self.intern_token(field)?;
                        expression = self.wrap_postfix(
                            expression,
                            ExpressionKind::Field {
                                base: expression,
                                name,
                            },
                            cursor,
                        )?;
                    }
                }
                Some(TokenKind::LeftParen) => {
                    self.advance(cursor);
                    let arguments = self.parse_expression_arguments(cursor, closure_body)?;
                    expression = self.wrap_postfix(
                        expression,
                        ExpressionKind::Call {
                            callee: expression,
                            arguments,
                        },
                        cursor,
                    )?;
                }
                Some(TokenKind::Question) => {
                    self.advance(cursor);
                    expression =
                        self.wrap_postfix(expression, ExpressionKind::Try(expression), cursor)?;
                }
                Some(TokenKind::Colon) if self.explicit_turbofish_ahead(cursor) => {
                    self.advance(cursor);
                    self.expect(cursor, TokenKind::Colon, "expected second ':' in turbofish")?;
                    let close = if self.consume(cursor, TokenKind::Less) {
                        TokenKind::Greater
                    } else {
                        self.expect(cursor, TokenKind::LeftBracket, "expected '<' or '['")?;
                        TokenKind::RightBracket
                    };
                    let types = self.parse_type_arguments(cursor, close)?;
                    self.expect(
                        cursor,
                        TokenKind::LeftParen,
                        "expected '(' after type arguments",
                    )?;
                    let arguments = self.parse_expression_arguments(cursor, closure_body)?;
                    expression = self.wrap_postfix(
                        expression,
                        ExpressionKind::GenericCall {
                            callee: expression,
                            type_arguments: types,
                            arguments,
                        },
                        cursor,
                    )?;
                }
                Some(TokenKind::LeftBracket) if self.bracket_turbofish_ahead(cursor) => {
                    self.advance(cursor);
                    let types = self.parse_type_arguments(cursor, TokenKind::RightBracket)?;
                    self.expect(
                        cursor,
                        TokenKind::LeftParen,
                        "expected '(' after type arguments",
                    )?;
                    let arguments = self.parse_expression_arguments(cursor, closure_body)?;
                    expression = self.wrap_postfix(
                        expression,
                        ExpressionKind::GenericCall {
                            callee: expression,
                            type_arguments: types,
                            arguments,
                        },
                        cursor,
                    )?;
                }
                Some(TokenKind::LeftBracket) => {
                    self.advance(cursor);
                    let index = self.parse_expression(cursor, closure_body)?;
                    self.expect(cursor, TokenKind::RightBracket, "expected ']' after index")?;
                    expression = self.wrap_postfix(
                        expression,
                        ExpressionKind::Index {
                            base: expression,
                            index,
                        },
                        cursor,
                    )?;
                }
                _ => break,
            }
        }
        Ok(expression)
    }

    fn parse_expression_arguments(
        &mut self,
        cursor: &mut Cursor,
        closure_body: Option<&[SyntaxNode]>,
    ) -> LowerResult<IdRange<ExprId>> {
        let mut arguments = Vec::new();
        while self.peek_kind(cursor) != Some(TokenKind::RightParen) {
            arguments.push(self.parse_expression(cursor, closure_body)?);
            if !self.consume(cursor, TokenKind::Comma) {
                break;
            }
        }
        let span = self.cursor_span(cursor);
        self.expect(
            cursor,
            TokenKind::RightParen,
            "expected ')' after arguments",
        )?;
        self.expression_list(&arguments, span)
    }

    fn parse_type_arguments(
        &mut self,
        cursor: &mut Cursor,
        close: TokenKind,
    ) -> LowerResult<IdRange<TypeRefId>> {
        let mut arguments = Vec::new();
        while self.peek_kind(cursor) != Some(close) {
            arguments.push(self.parse_type_ref(cursor)?);
            if !self.consume(cursor, TokenKind::Comma) {
                break;
            }
        }
        let span = self.cursor_span(cursor);
        self.expect(cursor, close, "expected closing generic delimiter")?;
        self.type_ref_list(&arguments, span)
    }

    fn parse_closure(
        &mut self,
        cursor: &mut Cursor,
        closure_body: Option<&[SyntaxNode]>,
    ) -> LowerResult<ExprId> {
        let start = self
            .previous_span(cursor)
            .unwrap_or_else(|| self.cursor_span(cursor));
        self.expect(cursor, TokenKind::LeftParen, "expected '(' after 'fn'")?;
        let placeholder_origin = self.source_origin(start)?;
        let parent = self.current_scope();
        let scope = self.begin_scope(parent, placeholder_origin)?;
        let mut parameters = Vec::new();
        while self.peek_kind(cursor) != Some(TokenKind::RightParen) {
            let name_token = self.expect_identifier(cursor, "expected closure parameter")?;
            let name = self.intern_token(name_token)?;
            let type_ref = if self.consume(cursor, TokenKind::Colon) {
                Some(self.parse_type_ref(cursor)?)
            } else {
                None
            };
            let origin = self.source_origin(name_token.span)?;
            parameters.push(self.declare_local(
                name,
                false,
                LocalKind::Parameter,
                type_ref,
                None,
                origin,
            )?);
            if !self.consume(cursor, TokenKind::Comma) {
                break;
            }
        }
        self.expect(
            cursor,
            TokenKind::RightParen,
            "expected ')' after closure parameters",
        )?;
        let return_type = if self.consume(cursor, TokenKind::Arrow) {
            Some(self.parse_type_ref(cursor)?)
        } else {
            None
        };
        self.expect(cursor, TokenKind::Colon, "expected ':' before closure body")?;

        let statements = if self.at_end(cursor) {
            let children = closure_body
                .ok_or_else(|| LowerError::new("E3100", "closure requires a body", start))?;
            if children.is_empty() {
                return Err(LowerError::new("E3100", "closure requires a body", start));
            }
            self.lower_statements(children)?
        } else {
            let value = self.parse_expression(cursor, None)?;
            let value_origin = self.package.expressions[value].origin;
            let value_span = self.package.origins[value_origin].span;
            let return_origin = self.desugared_origin(
                value_span,
                value_origin,
                DesugaringKind::ImplicitClosureReturn,
            )?;
            vec![self.alloc_statement(Statement {
                kind: StatementKind::Return(Some(value)),
                origin: return_origin,
            })?]
        };
        let end = self
            .previous_span(cursor)
            .map_or(start.end, |span| span.end.max(start.end));
        let span = Span {
            file: start.file,
            start: start.start,
            end: if let Some(children) = closure_body {
                children.last().map_or(end, |node| node.span.end.max(end))
            } else {
                end
            },
        };
        self.package.origins[placeholder_origin].span = span;
        let closure_origin = placeholder_origin;
        let statements = self.statement_list(&statements, span)?;
        let body = self.finish_body(scope, statements, closure_origin)?;
        let parameters = self.local_list(&parameters, span)?;
        self.alloc_expression(Expression {
            kind: ExpressionKind::Closure {
                parameters,
                return_type,
                body,
            },
            origin: closure_origin,
        })
    }

    fn wrap_postfix(
        &mut self,
        base: ExprId,
        kind: ExpressionKind,
        cursor: &Cursor,
    ) -> LowerResult<ExprId> {
        let base_span = self.package.origins[self.package.expressions[base].origin].span;
        let end = self
            .previous_span(cursor)
            .map_or(base_span.end, |span| span.end);
        let origin = self.source_origin(Span {
            file: base_span.file,
            start: base_span.start,
            end,
        })?;
        self.alloc_expression(Expression { kind, origin })
    }

    pub(super) fn is_assignment_target(&self, expression: ExprId) -> bool {
        matches!(
            self.package.expressions[expression].kind,
            ExpressionKind::Name {
                binding: NameBinding::Local(_),
                ..
            } | ExpressionKind::Field { .. }
                | ExpressionKind::Index { .. }
        )
    }

    fn explicit_turbofish_ahead(&self, cursor: &mut Cursor) -> bool {
        let mut probe = *cursor;
        self.consume(&mut probe, TokenKind::Colon)
            && self.consume(&mut probe, TokenKind::Colon)
            && matches!(
                self.peek_kind(&mut probe),
                Some(TokenKind::Less | TokenKind::LeftBracket)
            )
    }

    fn bracket_turbofish_ahead(&self, cursor: &mut Cursor) -> bool {
        let mut probe = *cursor;
        if !self.consume(&mut probe, TokenKind::LeftBracket) {
            return false;
        }
        let mut depth = 1usize;
        while let Some(token) = self.advance(&mut probe) {
            match token.kind {
                TokenKind::LeftBracket => depth += 1,
                TokenKind::RightBracket => {
                    depth -= 1;
                    if depth == 0 {
                        return self.peek_kind(&mut probe) == Some(TokenKind::LeftParen);
                    }
                }
                TokenKind::Identifier
                | TokenKind::Comma
                | TokenKind::LeftParen
                | TokenKind::RightParen => {}
                _ => return false,
            }
        }
        false
    }
}

pub(super) fn split_int_suffix(raw: &str) -> (&str, Option<&'static str>) {
    const SUFFIXES: &[(&str, Option<&'static str>)] = &[
        ("usize", Some("USize")),
        ("isize", Some("ISize")),
        ("u64", Some("U64")),
        ("u32", Some("U32")),
        ("u16", Some("U16")),
        ("i64", None),
        ("i32", Some("I32")),
        ("i16", Some("I16")),
        ("u8", Some("U8")),
        ("i8", Some("I8")),
    ];
    for &(suffix, type_name) in SUFFIXES {
        if let Some(prefix) = raw.strip_suffix(suffix) {
            return (prefix, type_name);
        }
    }
    (raw, None)
}
