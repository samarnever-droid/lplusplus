use super::*;

impl<'module, 'package> ModuleLowerer<'module, 'package> {
    pub(super) fn lower_item(&mut self, item: &Item, node: &SyntaxNode) -> LowerResult<HirItemId> {
        let definition = item
            .name
            .as_deref()
            .and_then(|name| self.definition_for(name));
        let name = item
            .name
            .as_deref()
            .map(|name| self.intern(name, item.span))
            .transpose()?;
        let attributes = item
            .attributes
            .iter()
            .map(|attribute| self.intern(&attribute.name, attribute.span))
            .collect::<LowerResult<Vec<_>>>()?;
        let attributes = self.symbol_list(&attributes, item.span)?;
        let origin = self.source_origin(item.span)?;
        let declaration_origin = if item.span == node.span {
            origin
        } else {
            self.source_origin(node.span)?
        };
        let kind = match item.kind {
            ItemKind::Function => {
                HirItemKind::Function(self.lower_function(node, declaration_origin, false)?)
            }
            ItemKind::Struct => HirItemKind::Struct(self.lower_struct(node)?),
            ItemKind::Enum => HirItemKind::Enum(self.lower_enum(node)?),
            ItemKind::Trait => HirItemKind::Trait(self.lower_trait(node)?),
            ItemKind::Impl => HirItemKind::Impl(self.lower_impl(node)?),
            ItemKind::Extern => HirItemKind::Extern(self.lower_extern(node)?),
            ItemKind::Const => self.lower_const(node)?,
            ItemKind::TypeAlias => self.lower_type_alias(node)?,
            ItemKind::Import => unreachable!("imports are omitted before item lowering"),
        };
        self.alloc_item(HirItem {
            module: self.module.id,
            definition,
            name,
            public: item.public,
            attributes,
            kind,
            origin,
        })
    }

    fn lower_nested_function(
        &mut self,
        node: &SyntaxNode,
        signature_only: bool,
    ) -> LowerResult<HirItemId> {
        let mut cursor = Cursor::new(node.tokens);
        self.consume_keyword(&mut cursor, Keyword::Async);
        self.expect_keyword(&mut cursor, Keyword::Def, "expected 'def' in method")?;
        let name_token = self.expect_identifier(&mut cursor, "expected method name")?;
        let name = self.intern_token(name_token)?;
        let origin = self.source_origin(node.span)?;
        let function = self.lower_function(node, origin, signature_only)?;
        self.alloc_item(HirItem {
            module: self.module.id,
            definition: None,
            name: Some(name),
            public: false,
            attributes: IdRange::empty(),
            kind: HirItemKind::Function(function),
            origin,
        })
    }

    fn lower_function(
        &mut self,
        node: &SyntaxNode,
        origin: OriginId,
        signature_only: bool,
    ) -> LowerResult<Function> {
        let mut cursor = Cursor::new(node.tokens);
        self.consume_keyword(&mut cursor, Keyword::Pub);
        let is_async = self.consume_keyword(&mut cursor, Keyword::Async);
        self.expect_keyword(&mut cursor, Keyword::Def, "expected function declaration")?;
        self.expect_identifier(&mut cursor, "expected function name")?;
        let explicit_type_parameters = self.parse_type_parameters(&mut cursor)?;
        // Trait-typed parameters (`def f(x: SomeTrait)`) are desugared into
        // implicit trait-bounded generics (`def f[$impl0: SomeTrait](x: $impl0)`),
        // so the existing monomorphization machinery produces a concrete copy per
        // implementing type. Synthetic type parameters are appended to the
        // explicit ones below.
        let mut type_parameter_ids: Vec<TypeParamId> = self
            .package
            .type_parameters(explicit_type_parameters)
            .to_vec();
        let mut synthetic_trait_params: u32 = 0;

        self.expect(
            &mut cursor,
            TokenKind::LeftParen,
            "expected '(' after function name",
        )?;
        let scope = self.begin_scope(None, origin)?;
        let mut parameters = Vec::new();
        let mut variadic = false;
        if self.peek_kind(&mut cursor) != Some(TokenKind::RightParen) {
            loop {
                let parameter_origin_span = self.peek_span(&mut cursor).unwrap_or(node.span);
                let is_variadic = self.consume(&mut cursor, TokenKind::Ellipsis);
                variadic |= is_variadic;
                let name_token = self.expect_identifier(&mut cursor, "expected parameter name")?;
                let is_self = self.token_text(name_token) == "self";
                let name = self.intern_token(name_token)?;
                let type_ref = if self.consume(&mut cursor, TokenKind::Colon) {
                    Some(self.parse_type_ref(&mut cursor)?)
                } else if is_self && !is_variadic {
                    Some(self.named_type("Self", name_token.span)?)
                } else {
                    return Err(LowerError::new(
                        "E3100",
                        "expected ':' and a parameter type",
                        name_token.span,
                    ));
                };
                // If the (non-self) parameter's annotation names a trait, rewrite
                // it into a fresh trait-bounded generic type parameter.
                let type_ref = match type_ref {
                    Some(annotation) if !is_self => {
                        match self
                            .desugar_trait_parameter(annotation, &mut synthetic_trait_params)?
                        {
                            Some((param_id, param_ref)) => {
                                type_parameter_ids.push(param_id);
                                Some(param_ref)
                            }
                            None => Some(annotation),
                        }
                    }
                    other => other,
                };
                let default = if self.consume(&mut cursor, TokenKind::Equal) {
                    if is_variadic {
                        return Err(LowerError::new(
                            "E3100",
                            "variadic parameters cannot have defaults",
                            name_token.span,
                        ));
                    }
                    Some(self.parse_expression(&mut cursor, None)?)
                } else {
                    None
                };
                let local_origin = self.source_origin(Span {
                    file: node.span.file,
                    start: parameter_origin_span.start,
                    end: self
                        .previous_span(&cursor)
                        .map_or(name_token.span.end, |span| span.end),
                })?;
                parameters.push(self.declare_local(
                    name,
                    false,
                    LocalKind::Parameter,
                    type_ref,
                    default,
                    local_origin,
                )?);
                if !self.consume(&mut cursor, TokenKind::Comma) {
                    break;
                }
                if is_variadic {
                    return Err(LowerError::new(
                        "E3100",
                        "variadic parameter must be final",
                        name_token.span,
                    ));
                }
            }
        }
        self.expect(
            &mut cursor,
            TokenKind::RightParen,
            "expected ')' after parameters",
        )?;
        let return_type = if self.consume(&mut cursor, TokenKind::Arrow) {
            Some(self.parse_type_ref(&mut cursor)?)
        } else {
            None
        };
        let has_colon = self.consume(&mut cursor, TokenKind::Colon);
        self.expect_end(&mut cursor, "unexpected token after function signature")?;

        let body = if signature_only {
            if !node.children.is_empty() {
                return Err(LowerError::new(
                    "E3101",
                    "signature-only function cannot have a body",
                    node.span,
                ));
            }
            None
        } else {
            if !has_colon || node.children.is_empty() {
                return Err(LowerError::new(
                    "E3100",
                    "function requires an indented body",
                    node.span,
                ));
            }
            let statements = self.lower_statements(&node.children)?;
            let statements = self.statement_list(&statements, node.span)?;
            Some(self.finish_body(scope, statements, origin)?)
        };
        if body.is_none() {
            self.finish_scope(scope, node.span)?;
        }
        let parameters = self.local_list(&parameters, node.span)?;
        let type_parameters = if synthetic_trait_params == 0 {
            explicit_type_parameters
        } else {
            self.type_parameter_list(&type_parameter_ids, node.span)?
        };
        Ok(Function {
            type_parameters,
            parameters,
            return_type,
            body,
            is_async,
            variadic,
        })
    }

    fn lower_struct(&mut self, node: &SyntaxNode) -> LowerResult<Struct> {
        let mut cursor = Cursor::new(node.tokens);
        self.consume_keyword(&mut cursor, Keyword::Pub);
        self.expect_keyword(&mut cursor, Keyword::Struct, "expected struct declaration")?;
        self.expect_identifier(&mut cursor, "expected struct name")?;
        let type_parameters = self.parse_type_parameters(&mut cursor)?;
        self.expect(
            &mut cursor,
            TokenKind::Colon,
            "expected ':' after struct name",
        )?;
        self.expect_end(&mut cursor, "unexpected token after struct header")?;

        let mut fields = Vec::with_capacity(node.children.len());
        for child in &node.children {
            fields.push(self.lower_field(child)?);
        }
        Ok(Struct {
            type_parameters,
            fields: self.field_list(&fields, node.span)?,
        })
    }

    fn lower_enum(&mut self, node: &SyntaxNode) -> LowerResult<Enum> {
        let mut cursor = Cursor::new(node.tokens);
        self.consume_keyword(&mut cursor, Keyword::Pub);
        self.expect_keyword(&mut cursor, Keyword::Enum, "expected enum declaration")?;
        self.expect_identifier(&mut cursor, "expected enum name")?;
        let type_parameters = self.parse_type_parameters(&mut cursor)?;
        self.expect(
            &mut cursor,
            TokenKind::Colon,
            "expected ':' after enum name",
        )?;
        self.expect_end(&mut cursor, "unexpected token after enum header")?;

        let mut variants = Vec::with_capacity(node.children.len());
        for child in &node.children {
            let origin = self.source_origin(child.span)?;
            let mut child_cursor = Cursor::new(child.tokens);
            let name = self.intern_token(
                self.expect_identifier(&mut child_cursor, "expected enum variant name")?,
            )?;
            let mut fields = Vec::new();
            if self.consume(&mut child_cursor, TokenKind::LeftParen) {
                if self.peek_kind(&mut child_cursor) != Some(TokenKind::RightParen) {
                    loop {
                        let field_start = self.peek_span(&mut child_cursor).unwrap_or(child.span);
                        let field_name_token = self
                            .expect_identifier(&mut child_cursor, "expected variant field name")?;
                        let field_name = self.intern_token(field_name_token)?;
                        self.expect(
                            &mut child_cursor,
                            TokenKind::Colon,
                            "expected ':' after variant field name",
                        )?;
                        let type_ref = self.parse_type_ref(&mut child_cursor)?;
                        let field_origin = self.source_origin(Span {
                            file: child.span.file,
                            start: field_start.start,
                            end: self
                                .previous_span(&child_cursor)
                                .map_or(field_name_token.span.end, |span| span.end),
                        })?;
                        fields.push(self.alloc_field(Field {
                            name: field_name,
                            type_ref,
                            default: None,
                            origin: field_origin,
                        })?);
                        if !self.consume(&mut child_cursor, TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.expect(
                    &mut child_cursor,
                    TokenKind::RightParen,
                    "expected ')' after variant fields",
                )?;
            }
            self.expect_end(&mut child_cursor, "unexpected token after enum variant")?;
            let fields = self.field_list(&fields, child.span)?;
            variants.push(self.alloc_variant(Variant {
                name,
                fields,
                origin,
            })?);
        }
        Ok(Enum {
            type_parameters,
            variants: self.variant_list(&variants, node.span)?,
        })
    }

    fn lower_trait(&mut self, node: &SyntaxNode) -> LowerResult<Trait> {
        let mut cursor = Cursor::new(node.tokens);
        self.consume_keyword(&mut cursor, Keyword::Pub);
        self.expect_keyword(&mut cursor, Keyword::Trait, "expected trait declaration")?;
        self.expect_identifier(&mut cursor, "expected trait name")?;
        let type_parameters = self.parse_type_parameters(&mut cursor)?;
        self.expect(
            &mut cursor,
            TokenKind::Colon,
            "expected ':' after trait name",
        )?;
        self.expect_end(&mut cursor, "unexpected token after trait header")?;
        let methods = node
            .children
            .iter()
            .map(|method| self.lower_nested_function(method, true))
            .collect::<LowerResult<Vec<_>>>()?;
        Ok(Trait {
            type_parameters,
            methods: self.item_list(&methods, node.span)?,
        })
    }

    fn lower_impl(&mut self, node: &SyntaxNode) -> LowerResult<Impl> {
        let mut cursor = Cursor::new(node.tokens);
        self.consume_keyword(&mut cursor, Keyword::Pub);
        self.expect_keyword(&mut cursor, Keyword::Impl, "expected impl declaration")?;
        let type_parameters = self.parse_type_parameters(&mut cursor)?;
        let first = self.parse_type_ref(&mut cursor)?;
        let (trait_ref, target) = if self.consume_keyword(&mut cursor, Keyword::For) {
            (Some(first), self.parse_type_ref(&mut cursor)?)
        } else {
            (None, first)
        };
        self.expect(
            &mut cursor,
            TokenKind::Colon,
            "expected ':' after impl target",
        )?;
        self.expect_end(&mut cursor, "unexpected token after impl header")?;
        let methods = node
            .children
            .iter()
            .map(|method| self.lower_nested_function(method, false))
            .collect::<LowerResult<Vec<_>>>()?;
        let impl_parameters = self
            .package
            .type_parameters(type_parameters)
            .iter()
            .copied()
            .filter(|parameter| {
                let name = self.package.type_parameters[*parameter].name;
                self.type_ref_mentions_symbol(target, name)
                    || trait_ref
                        .is_some_and(|type_ref| self.type_ref_mentions_symbol(type_ref, name))
            })
            .collect::<Vec<_>>();
        if !impl_parameters.is_empty() {
            for method in &methods {
                let HirItemKind::Function(mut function) = self.package.items[*method].kind else {
                    continue;
                };
                let mut combined = impl_parameters.clone();
                combined.extend_from_slice(self.package.type_parameters(function.type_parameters));
                function.type_parameters = self.type_parameter_list(&combined, node.span)?;
                self.package.items[*method].kind = HirItemKind::Function(function);
            }
        }
        Ok(Impl {
            type_parameters,
            trait_ref,
            target,
            methods: self.item_list(&methods, node.span)?,
        })
    }

    fn type_ref_mentions_symbol(&self, type_ref: TypeRefId, symbol: Symbol) -> bool {
        match self.package.type_refs[type_ref].kind {
            TypeRefKind::Named(name) => name == symbol,
            TypeRefKind::Applied { base, arguments } => {
                base == symbol
                    || self
                        .package
                        .type_refs(arguments)
                        .iter()
                        .any(|argument| self.type_ref_mentions_symbol(*argument, symbol))
            }
            TypeRefKind::Tuple(elements) => self
                .package
                .type_refs(elements)
                .iter()
                .any(|element| self.type_ref_mentions_symbol(*element, symbol)),
        }
    }

    fn lower_extern(&mut self, node: &SyntaxNode) -> LowerResult<ExternBlock> {
        let mut cursor = Cursor::new(node.tokens);
        self.consume_keyword(&mut cursor, Keyword::Pub);
        self.expect_keyword(&mut cursor, Keyword::Extern, "expected extern declaration")?;
        let abi = self.expect(&mut cursor, TokenKind::String, "expected extern ABI string")?;
        let link_library = if self.peek(&mut cursor).is_some_and(|token| {
            token.kind == TokenKind::Identifier && self.token_text(token) == "link"
        }) {
            self.advance(&mut cursor);
            Some(
                self.expect(
                    &mut cursor,
                    TokenKind::String,
                    "expected library string after 'link'",
                )?
                .span,
            )
        } else {
            None
        };
        self.expect(
            &mut cursor,
            TokenKind::Colon,
            "expected ':' after extern ABI",
        )?;
        self.expect_end(&mut cursor, "unexpected token after extern header")?;
        let functions = node
            .children
            .iter()
            .map(|function| self.lower_nested_function(function, true))
            .collect::<LowerResult<Vec<_>>>()?;
        Ok(ExternBlock {
            abi: abi.span,
            link_library,
            functions: self.item_list(&functions, node.span)?,
        })
    }

    fn lower_const(&mut self, node: &SyntaxNode) -> LowerResult<HirItemKind> {
        let mut cursor = Cursor::new(node.tokens);
        self.consume_keyword(&mut cursor, Keyword::Pub);
        self.expect_keyword(&mut cursor, Keyword::Const, "expected const declaration")?;
        self.expect_identifier(&mut cursor, "expected constant name")?;
        self.expect(&mut cursor, TokenKind::Equal, "expected '=' in constant")?;
        let value = self.parse_expression(&mut cursor, None)?;
        self.expect_end(&mut cursor, "unexpected token after constant value")?;
        Ok(HirItemKind::Const { value })
    }

    fn lower_type_alias(&mut self, node: &SyntaxNode) -> LowerResult<HirItemKind> {
        let mut cursor = Cursor::new(node.tokens);
        self.consume_keyword(&mut cursor, Keyword::Pub);
        self.expect_keyword(&mut cursor, Keyword::Type, "expected type alias")?;
        self.expect_identifier(&mut cursor, "expected type alias name")?;
        self.expect(&mut cursor, TokenKind::Equal, "expected '=' in type alias")?;
        let target = self.parse_type_ref(&mut cursor)?;
        self.expect_end(&mut cursor, "unexpected token after type alias")?;
        Ok(HirItemKind::TypeAlias { target })
    }

    fn lower_field(&mut self, node: &SyntaxNode) -> LowerResult<FieldId> {
        let mut cursor = Cursor::new(node.tokens);
        let name_token = self.expect_identifier(&mut cursor, "expected field name")?;
        let name = self.intern_token(name_token)?;
        self.expect(
            &mut cursor,
            TokenKind::Colon,
            "expected ':' after field name",
        )?;
        let type_ref = self.parse_type_ref(&mut cursor)?;
        let default = if self.consume(&mut cursor, TokenKind::Equal) {
            Some(self.parse_expression(&mut cursor, None)?)
        } else {
            None
        };
        self.expect_end(&mut cursor, "unexpected token after field")?;
        let origin = self.source_origin(node.span)?;
        self.alloc_field(Field {
            name,
            type_ref,
            default,
            origin,
        })
    }

    fn parse_type_parameters(&mut self, cursor: &mut Cursor) -> LowerResult<IdRange<TypeParamId>> {
        if !self.consume(cursor, TokenKind::LeftBracket) {
            return Ok(IdRange::empty());
        }
        let mut parameters = Vec::new();
        if self.peek_kind(cursor) != Some(TokenKind::RightBracket) {
            loop {
                let name_token = self.expect_identifier(cursor, "expected type parameter name")?;
                let name = self.intern_token(name_token)?;
                let bound = if self.consume(cursor, TokenKind::Colon) {
                    Some(self.parse_type_ref(cursor)?)
                } else {
                    None
                };
                let origin = self.source_origin(Span {
                    file: name_token.span.file,
                    start: name_token.span.start,
                    end: self
                        .previous_span(cursor)
                        .map_or(name_token.span.end, |span| span.end),
                })?;
                parameters.push(self.alloc_type_parameter(TypeParameter {
                    name,
                    bound,
                    origin,
                })?);
                if !self.consume(cursor, TokenKind::Comma) {
                    break;
                }
            }
        }
        self.expect(
            cursor,
            TokenKind::RightBracket,
            "expected ']' after type parameters",
        )?;
        self.type_parameter_list(&parameters, self.cursor_span(cursor))
    }

    /// Returns `true` when `symbol` resolves (in the current module, including
    /// imports) to a trait definition.
    fn symbol_is_trait(&self, symbol: Symbol) -> bool {
        matches!(
            self.package.names.resolve_symbol(self.module.id, symbol),
            Some(BindingTarget::Definition(def))
                if self.package.names.definitions[def].kind == DefinitionKind::Trait
        )
    }

    /// If `annotation` is a bare name that resolves to a trait, synthesize an
    /// implicit trait-bounded type parameter and return `(param, new_type_ref)`
    /// where `new_type_ref` names that parameter. Otherwise returns `None`.
    fn desugar_trait_parameter(
        &mut self,
        annotation: TypeRefId,
        counter: &mut u32,
    ) -> LowerResult<Option<(TypeParamId, TypeRefId)>> {
        let TypeRefKind::Named(trait_name) = self.package.type_refs[annotation].kind else {
            return Ok(None);
        };
        if !self.symbol_is_trait(trait_name) {
            return Ok(None);
        }
        let origin = self.package.type_refs[annotation].origin;
        let span = self.package.origins[origin].span;
        let param_name = self.intern(&format!("$impl{counter}"), span)?;
        *counter += 1;
        // Reuse the original annotation (which names the trait) as the bound.
        let type_param = self.alloc_type_parameter(TypeParameter {
            name: param_name,
            bound: Some(annotation),
            origin,
        })?;
        let param_ref = self.alloc_type_ref(TypeRef {
            kind: TypeRefKind::Named(param_name),
            origin,
        })?;
        Ok(Some((type_param, param_ref)))
    }

    pub(super) fn parse_type_ref(&mut self, cursor: &mut Cursor) -> LowerResult<TypeRefId> {
        let start = self
            .peek_span(cursor)
            .ok_or_else(|| LowerError::new("E3100", "expected a type", self.cursor_span(cursor)))?;
        let kind = if self.consume(cursor, TokenKind::LeftParen) {
            let mut elements = vec![self.parse_type_ref(cursor)?];
            self.expect(
                cursor,
                TokenKind::Comma,
                "tuple types require at least two elements",
            )?;
            loop {
                elements.push(self.parse_type_ref(cursor)?);
                if !self.consume(cursor, TokenKind::Comma) {
                    break;
                }
            }
            self.expect(
                cursor,
                TokenKind::RightParen,
                "expected ')' after tuple type",
            )?;
            if !(2..=4).contains(&elements.len()) {
                return Err(LowerError::new(
                    "E3100",
                    "tuple types require between two and four elements",
                    start,
                ));
            }
            TypeRefKind::Tuple(self.type_ref_list(&elements, start)?)
        } else {
            let name_token = self.expect_identifier(cursor, "expected type name")?;
            let base = self.intern_token(name_token)?;
            if self.consume(cursor, TokenKind::LeftBracket) {
                let mut arguments = Vec::new();
                if self.peek_kind(cursor) != Some(TokenKind::RightBracket) {
                    loop {
                        arguments.push(self.parse_type_ref(cursor)?);
                        if !self.consume(cursor, TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.expect(
                    cursor,
                    TokenKind::RightBracket,
                    "expected ']' after type arguments",
                )?;
                TypeRefKind::Applied {
                    base,
                    arguments: self.type_ref_list(&arguments, name_token.span)?,
                }
            } else {
                TypeRefKind::Named(base)
            }
        };
        let span = Span {
            file: start.file,
            start: start.start,
            end: self
                .previous_span(cursor)
                .map_or(start.end, |span| span.end),
        };
        let origin = self.source_origin(span)?;
        self.alloc_type_ref(TypeRef { kind, origin })
    }

    fn named_type(&mut self, name: &str, span: Span) -> LowerResult<TypeRefId> {
        let symbol = self.intern(name, span)?;
        let origin = self.source_origin(span)?;
        self.alloc_type_ref(TypeRef {
            kind: TypeRefKind::Named(symbol),
            origin,
        })
    }
}
