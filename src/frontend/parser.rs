use crate::ast::*;
use crate::lexer::{SpannedToken, Token};

pub struct Parser {
    tokens: Vec<SpannedToken>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<SpannedToken>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn current_span(&self) -> (usize, usize) {
        self.tokens
            .get(self.pos)
            .or_else(|| self.tokens.last())
            .map(|st| (st.line, st.col))
            .unwrap_or((1, 1))
    }

    fn error<T>(&self, msg: impl std::fmt::Display) -> Result<T, String> {
        let (line, col) = self.current_span();
        Err(format!("[line {}:col {}] Syntax Error: {}", line, col, msg))
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos).map(|st| &st.token)
    }

    fn advance(&mut self) -> Option<&Token> {
        if self.pos < self.tokens.len() {
            let t = &self.tokens[self.pos].token;
            self.pos += 1;
            Some(t)
        } else {
            None
        }
    }

    fn match_token(&mut self, expected: &Token) -> bool {
        if let Some(t) = self.peek() {
            if t == expected {
                self.advance();
                return true;
            }
        }
        false
    }

    fn skip_newlines(&mut self) {
        while self.match_token(&Token::Newline) {}
    }

    pub fn parse(&mut self) -> Result<Program, String> {
        let mut declarations = Vec::new();

        self.skip_newlines();
        while self.peek() != Some(&Token::Eof) && self.peek().is_some() {
            if self.match_token(&Token::Def) {
                declarations.push(TopLevel::Function(self.parse_function()?));
            } else if self.match_token(&Token::Struct) {
                declarations.push(TopLevel::Struct(self.parse_struct()?));
            } else if self.match_token(&Token::Enum) {
                declarations.push(TopLevel::Enum(self.parse_enum()?));
            } else if self.match_token(&Token::Import) {
                declarations.push(TopLevel::Import(self.parse_import()?));
            } else if self.match_token(&Token::From) {
                declarations.push(TopLevel::Import(self.parse_from_import()?));
            } else {
                let found = self.peek().cloned();
                return self.error(format!(
                    "Expected 'def', 'struct', 'enum', 'import', or 'from', found {:?}",
                    found
                ));
            }
            self.skip_newlines();
        }

        Ok(Program { declarations })
    }

    /// Parse `import math`, `import utils.math`, `import math as m`
    fn parse_import(&mut self) -> Result<ImportKind, String> {
        let mut path = Vec::new();
        let first = match self.advance() {
            Some(Token::Ident(n)) => n.clone(),
            _ => return self.error("Expected module name after 'import'"),
        };
        path.push(first);
        // Parse dotted path: import utils.math.helpers
        while self.peek() == Some(&Token::Dot) {
            self.advance(); // consume '.'
            match self.advance() {
                Some(Token::Ident(n)) => path.push(n.clone()),
                _ => return self.error("Expected identifier after '.' in import path"),
            }
        }
        // Check for 'as' alias
        let alias = if self.peek() == Some(&Token::As) {
            self.advance(); // consume 'as'
            match self.advance() {
                Some(Token::Ident(n)) => Some(n.clone()),
                _ => return self.error("Expected identifier after 'as'"),
            }
        } else {
            None
        };
        if self.peek() == Some(&Token::Newline) {
            self.advance();
        }
        Ok(ImportKind::Module { path, alias })
    }

    /// Parse `from math import sqrt, PI`
    fn parse_from_import(&mut self) -> Result<ImportKind, String> {
        let mut path = Vec::new();
        let first = match self.advance() {
            Some(Token::Ident(n)) => n.clone(),
            _ => return self.error("Expected module name after 'from'"),
        };
        path.push(first);
        // Parse dotted path
        while self.peek() == Some(&Token::Dot) {
            self.advance();
            match self.advance() {
                Some(Token::Ident(n)) => path.push(n.clone()),
                _ => return self.error("Expected identifier after '.' in module path"),
            }
        }
        // Expect 'import'
        if !self.match_token(&Token::Import) {
            return self.error("Expected 'import' after module path in 'from ... import'");
        }
        // Parse item list: sqrt, PI, calculate
        let mut items = Vec::new();
        loop {
            match self.advance() {
                Some(Token::Ident(n)) => items.push(n.clone()),
                _ => return self.error("Expected identifier in import list"),
            }
            if !self.match_token(&Token::Comma) {
                break;
            }
        }
        if items.is_empty() {
            return self.error("Expected at least one item in 'from ... import' list");
        }
        if self.peek() == Some(&Token::Newline) {
            self.advance();
        }
        Ok(ImportKind::Selective { path, items })
    }

    /// Parse: `enum Color: Red, Green, Blue(intensity: Int)`
    fn parse_enum(&mut self) -> Result<EnumDef, String> {
        let name = match self.advance() {
            Some(Token::Ident(n)) => n.clone(),
            _ => return self.error("Expected enum name"),
        };

        if !self.match_token(&Token::Colon) {
            return self.error("Expected ':' after enum name");
        }

        if !self.match_token(&Token::Newline) {
            return self.error("Expected newline after ':'");
        }

        self.skip_newlines();

        if !self.match_token(&Token::Indent) {
            return self.error("Expected indentation for enum variants");
        }

        let mut variants = Vec::new();
        while self.peek() != Some(&Token::Dedent) && self.peek().is_some() {
            self.skip_newlines();
            if self.peek() == Some(&Token::Dedent) {
                break;
            }

            let variant_name = match self.advance() {
                Some(Token::Ident(n)) => n.clone(),
                _ => return self.error("Expected variant name"),
            };

            // Optional fields: `Ok(value: Int)` or `Blue(r: Int, g: Int, b: Int)`
            let mut fields = Vec::new();
            if self.match_token(&Token::LParen) {
                if self.peek() != Some(&Token::RParen) {
                    loop {
                        let field_name = match self.advance() {
                            Some(Token::Ident(n)) => n.clone(),
                            _ => return self.error("Expected field name in variant"),
                        };
                        if !self.match_token(&Token::Colon) {
                            return self.error("Expected ':' after variant field name");
                        }
                        let ty = self.parse_type()?;
                        fields.push(Param { name: field_name, ty });
                        if !self.match_token(&Token::Comma) {
                            break;
                        }
                    }
                }
                if !self.match_token(&Token::RParen) {
                    return self.error("Expected ')' after variant fields");
                }
            }

            variants.push(EnumVariant { name: variant_name, fields });
            self.skip_newlines();
        }
        self.match_token(&Token::Dedent);

        if variants.is_empty() {
            return self.error("Enum must have at least one variant");
        }

        Ok(EnumDef { name, variants })
    }

    fn parse_struct(&mut self) -> Result<StructDef, String> {
        let name = match self.advance() {
            Some(Token::Ident(n)) => n.clone(),
            _ => return self.error("Expected struct name"),
        };

        if !self.match_token(&Token::Colon) {
            return self.error("Expected ':' after struct name");
        }

        if !self.match_token(&Token::Newline) {
            return self.error("Expected newline after ':'");
        }

        self.skip_newlines();

        if !self.match_token(&Token::Indent) {
            return self.error("Expected indentation for struct fields");
        }

        let mut fields = Vec::new();
        while self.peek() != Some(&Token::Dedent) && self.peek().is_some() {
            self.skip_newlines();
            if self.peek() == Some(&Token::Dedent) {
                break;
            }

            let field_name = match self.advance() {
                Some(Token::Ident(n)) => n.clone(),
                _ => return self.error("Expected field name"),
            };

            if !self.match_token(&Token::Colon) {
                return self.error("Expected ':' after field name");
            }

            let ty = self.parse_type()?;
            fields.push(Param {
                name: field_name,
                ty,
            });

            self.skip_newlines();
        }
        self.match_token(&Token::Dedent);

        Ok(StructDef { name, fields })
    }

    fn parse_function(&mut self) -> Result<Function, String> {
        let name = match self.advance() {
            Some(Token::Ident(n)) => n.clone(),
            _ => return self.error("Expected function name"),
        };

        if !self.match_token(&Token::LParen) {
            return self.error("Expected '(' after function name");
        }

        let mut params = Vec::new();
        if self.peek() != Some(&Token::RParen) {
            loop {
                let param_name = match self.advance() {
                    Some(Token::Ident(n)) => n.clone(),
                    _ => return self.error("Expected parameter name"),
                };
                if !self.match_token(&Token::Colon) {
                    return self.error("Expected ':' after parameter name");
                }
                let ty = self.parse_type()?;
                params.push(Param {
                    name: param_name,
                    ty,
                });

                if !self.match_token(&Token::Comma) {
                    break;
                }
            }
        }

        if !self.match_token(&Token::RParen) {
            return self.error("Expected ')' after parameters");
        }

        let mut return_type = Type::Void;
        if self.match_token(&Token::Arrow) {
            return_type = self.parse_type()?;
        }

        if !self.match_token(&Token::Colon) {
            return self.error("Expected ':' before function body");
        }

        if !self.match_token(&Token::Newline) {
            return self.error("Expected newline after ':'");
        }

        self.skip_newlines();

        if !self.match_token(&Token::Indent) {
            return self.error("Expected indentation for function body");
        }

        let mut body = Vec::new();
        while self.peek() != Some(&Token::Dedent) && self.peek().is_some() {
            self.skip_newlines();
            if self.peek() == Some(&Token::Dedent) {
                break;
            }
            body.push(self.parse_stmt()?);
            self.skip_newlines();
        }
        self.match_token(&Token::Dedent);

        Ok(Function {
            name,
            params,
            return_type,
            body,
        })
    }

    fn parse_type(&mut self) -> Result<Type, String> {
        let base_name = match self.advance() {
            Some(Token::Ident(n)) => n.clone(),
            _ => return self.error("Expected a type name"),
        };

        if self.match_token(&Token::LBracket) {
            let mut type_args = Vec::new();
            if self.peek() != Some(&Token::RBracket) {
                loop {
                    type_args.push(self.parse_type()?);
                    if !self.match_token(&Token::Comma) {
                        break;
                    }
                }
            }
            if !self.match_token(&Token::RBracket) {
                return self.error("Expected ']' after generic type arguments");
            }
            return Ok(Type::Generic(base_name, type_args));
        }

        match base_name.as_str() {
            "Int" => Ok(Type::Int),
            "Float" => Ok(Type::Float),
            "String" | "Str" => Ok(Type::String),
            "Bool" => Ok(Type::Bool),
            "Void" => Ok(Type::Void),
            _ => Ok(Type::Custom(base_name)),
        }
    }

    fn parse_stmt(&mut self) -> Result<Stmt, String> {
        self.skip_newlines(); // Ensure we don't trip over blank lines before a statement

        if self.match_token(&Token::If) {
            let condition = self.parse_expr()?;
            if !self.match_token(&Token::Colon) {
                return self.error("Expected ':' after if condition");
            }
            if !self.match_token(&Token::Newline) {
                return self.error("Expected newline after ':'");
            }
            self.skip_newlines();
            if !self.match_token(&Token::Indent) {
                return self.error("Expected indentation for if block");
            }
            let mut then_block = Vec::new();
            while self.peek() != Some(&Token::Dedent) && self.peek().is_some() {
                self.skip_newlines();
                if self.peek() == Some(&Token::Dedent) {
                    break;
                }
                then_block.push(self.parse_stmt()?);
                self.skip_newlines();
            }
            self.match_token(&Token::Dedent);

            let mut else_block = None;
            self.skip_newlines();
            if self.match_token(&Token::Else) {
                if self.peek() == Some(&Token::If) {
                    let elif_stmt = self.parse_stmt()?;
                    else_block = Some(vec![elif_stmt]);
                } else {
                    if !self.match_token(&Token::Colon) {
                        return self.error("Expected ':' or 'if' after 'else'");
                    }
                    if !self.match_token(&Token::Newline) {
                        return self.error("Expected newline after ':'");
                    }
                    self.skip_newlines();
                    if !self.match_token(&Token::Indent) {
                        return self.error("Expected indentation for else block");
                    }
                    let mut e_block = Vec::new();
                    while self.peek() != Some(&Token::Dedent) && self.peek().is_some() {
                        self.skip_newlines();
                        if self.peek() == Some(&Token::Dedent) {
                            break;
                        }
                        e_block.push(self.parse_stmt()?);
                        self.skip_newlines();
                    }
                    self.match_token(&Token::Dedent);
                    else_block = Some(e_block);
                }
            }

            return Ok(Stmt::If {
                condition,
                then_block,
                else_block,
            });
        }

        if self.match_token(&Token::While) {
            let condition = self.parse_expr()?;
            if !self.match_token(&Token::Colon) {
                return self.error("Expected ':' after while condition");
            }
            if !self.match_token(&Token::Newline) {
                return self.error("Expected newline after ':'");
            }
            self.skip_newlines();
            if !self.match_token(&Token::Indent) {
                return self.error("Expected indentation for while block");
            }
            let mut body = Vec::new();
            while self.peek() != Some(&Token::Dedent) && self.peek().is_some() {
                self.skip_newlines();
                if self.peek() == Some(&Token::Dedent) {
                    break;
                }
                body.push(self.parse_stmt()?);
                self.skip_newlines();
            }
            self.match_token(&Token::Dedent);
            return Ok(Stmt::While { condition, body });
        }

        // match subject:
        //     Variant(binding):
        //         body
        if self.match_token(&Token::Match) {
            let subject = self.parse_expr()?;
            if !self.match_token(&Token::Colon) {
                return self.error("Expected ':' after match subject");
            }
            if !self.match_token(&Token::Newline) {
                return self.error("Expected newline after ':'");
            }
            self.skip_newlines();
            if !self.match_token(&Token::Indent) {
                return self.error("Expected indentation for match arms");
            }

            let mut arms = Vec::new();
            while self.peek() != Some(&Token::Dedent) && self.peek().is_some() {
                self.skip_newlines();
                if self.peek() == Some(&Token::Dedent) {
                    break;
                }

                // Parse variant name
                let variant = match self.advance() {
                    Some(Token::Ident(n)) => n.clone(),
                    _ => return self.error("Expected variant name in match arm"),
                };

                // Optional bindings: Ok(value) or Ok(v, msg)
                let mut bindings = Vec::new();
                if self.match_token(&Token::LParen) {
                    if self.peek() != Some(&Token::RParen) {
                        loop {
                            match self.advance() {
                                Some(Token::Ident(n)) => bindings.push(n.clone()),
                                _ => return self.error("Expected binding name"),
                            }
                            if !self.match_token(&Token::Comma) {
                                break;
                            }
                        }
                    }
                    if !self.match_token(&Token::RParen) {
                        return self.error("Expected ')' after match bindings");
                    }
                }

                if !self.match_token(&Token::Colon) {
                    return self.error("Expected ':' after match arm");
                }
                if !self.match_token(&Token::Newline) {
                    return self.error("Expected newline after match arm ':'");
                }
                self.skip_newlines();
                if !self.match_token(&Token::Indent) {
                    return self.error("Expected indentation for match arm body");
                }

                let mut body = Vec::new();
                while self.peek() != Some(&Token::Dedent) && self.peek().is_some() {
                    self.skip_newlines();
                    if self.peek() == Some(&Token::Dedent) {
                        break;
                    }
                    body.push(self.parse_stmt()?);
                    self.skip_newlines();
                }
                self.match_token(&Token::Dedent);

                arms.push(MatchArm { variant, bindings, body });
                self.skip_newlines();
            }
            self.match_token(&Token::Dedent);

            if arms.is_empty() {
                return self.error("match must have at least one arm");
            }

            return Ok(Stmt::Match { subject, arms });
        }

        if self.match_token(&Token::For) {
            let var_name = match self.advance() {
                Some(Token::Ident(n)) => n.clone(),
                _ => return self.error("Expected identifier after 'for'"),
            };
            if !self.match_token(&Token::In) {
                return self.error("Expected 'in' after variable in 'for' loop");
            }
            let list_expr = self.parse_expr()?;
            if !self.match_token(&Token::Colon) {
                return self.error("Expected ':' after for loop list expression");
            }
            if !self.match_token(&Token::Newline) {
                return self.error("Expected newline after ':' in 'for' loop");
            }
            self.skip_newlines();
            if !self.match_token(&Token::Indent) {
                return self.error("Expected indentation for 'for' block");
            }
            let mut body = Vec::new();
            while self.peek() != Some(&Token::Dedent) && self.peek().is_some() {
                self.skip_newlines();
                if self.peek() == Some(&Token::Dedent) {
                    break;
                }
                body.push(self.parse_stmt()?);
                self.skip_newlines();
            }
            self.match_token(&Token::Dedent);

            if let Expr::Call { callee, args } = &list_expr {
                if let Expr::Identifier(fn_name, _) = callee.as_ref() {
                    if fn_name == "range" {
                        let (start, end) = match args.len() {
                            1 => (Expr::IntLiteral(0), args[0].clone()),
                            2 => (args[0].clone(), args[1].clone()),
                            _ => return self.error(
                                "range in a for loop expects range(end) or range(start, end)",
                            ),
                        };
                        return Ok(Stmt::ForRange {
                            var_name,
                            start,
                            end,
                            body,
                            binding_id: std::cell::Cell::new(None),
                        });
                    }
                }
            }

            return Ok(Stmt::ForIn {
                var_name,
                list: list_expr,
                body,
                binding_id: std::cell::Cell::new(None),
            });
        }

        if self.match_token(&Token::Return) {
            if self.peek() == Some(&Token::Newline) || self.peek() == Some(&Token::Dedent) {
                return Ok(Stmt::Return(None));
            }
            let expr = self.parse_expr()?;
            return Ok(Stmt::Return(Some(expr)));
        }

        if self.match_token(&Token::Break) {
            return Ok(Stmt::Break);
        }

        if self.match_token(&Token::Continue) {
            return Ok(Stmt::Continue);
        }

        if self.match_token(&Token::Mut) {
            let name = match self.advance() {
                Some(Token::Ident(n)) => n.clone(),
                _ => return self.error("Expected identifier after 'mut'"),
            };
            if !self.match_token(&Token::Assign) {
                return self.error("Expected ':=' after mutable identifier");
            }
            let value = self.parse_expr()?;
            return Ok(Stmt::LetInferred {
                name,
                is_mut: true,
                value,
                binding_id: std::cell::Cell::new(None),
            });
        }

        if let Some(Token::Ident(name)) = self.peek().cloned() {
            let next = self.tokens.get(self.pos + 1).map(|st| &st.token);
            if next == Some(&Token::Assign) {
                self.advance(); // consume name
                self.advance(); // consume :=
                let value = self.parse_expr()?;
                return Ok(Stmt::LetInferred {
                    name: name.clone(),
                    is_mut: false,
                    value,
                    binding_id: std::cell::Cell::new(None),
                });
            }
        }

        let expr = self.parse_expr()?;
        if self.match_token(&Token::Equal) {
            let value = self.parse_expr()?;
            match expr {
                Expr::Identifier(name, _) => {
                    return Ok(Stmt::Assign {
                        name,
                        value,
                        binding_id: std::cell::Cell::new(None),
                    });
                }
                Expr::FieldAccess { base, field } => {
                    return Ok(Stmt::AssignField {
                        base: *base,
                        field,
                        value,
                    });
                }
                _ => return self.error("Invalid assignment target"),
            }
        }
        Ok(Stmt::Expr(expr))
    }

    fn parse_expr(&mut self) -> Result<Expr, String> {
        if self.match_token(&Token::Fn) {
            return self.parse_closure();
        }
        if self.match_token(&Token::Spawn) {
            let closure = self.parse_expr()?;
            return Ok(Expr::Spawn {
                closure: Box::new(closure),
            });
        }
        self.parse_relational()
    }

    fn parse_relational(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_add_sub()?;

        while let Some(t) = self.peek() {
            let op = match t {
                Token::EqEq => BinaryOperator::Eq,
                Token::NotEq => BinaryOperator::NotEq,
                Token::Less => BinaryOperator::Less,
                Token::LessEq => BinaryOperator::LessEq,
                Token::Greater => BinaryOperator::Greater,
                Token::GreaterEq => BinaryOperator::GreaterEq,
                _ => break,
            };
            self.advance();
            let right = self.parse_add_sub()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_closure(&mut self) -> Result<Expr, String> {
        if !self.match_token(&Token::LParen) {
            return self.error("Expected '(' after 'fn'");
        }

        let mut params = Vec::new();
        if self.peek() != Some(&Token::RParen) {
            loop {
                let param_name = match self.advance() {
                    Some(Token::Ident(n)) => n.clone(),
                    _ => return self.error("Expected parameter name in closure"),
                };

                let mut ty = None;
                if self.match_token(&Token::Colon) {
                    ty = Some(self.parse_type()?);
                }

                params.push(ClosureParam {
                    name: param_name,
                    ty,
                });

                if !self.match_token(&Token::Comma) {
                    break;
                }
            }
        }

        if !self.match_token(&Token::RParen) {
            return self.error("Expected ')' after closure parameters");
        }

        let mut return_type = None;
        if self.match_token(&Token::Arrow) {
            return_type = Some(self.parse_type()?);
        }

        if !self.match_token(&Token::Colon) {
            return self.error("Expected ':' before closure body");
        }

        let mut body = Vec::new();
        if self.match_token(&Token::Newline) {
            if !self.match_token(&Token::Indent) {
                return self.error("Expected indentation for closure body");
            }
            while self.peek() != Some(&Token::Dedent) && self.peek().is_some() {
                self.skip_newlines();
                if self.peek() == Some(&Token::Dedent) {
                    break;
                }
                body.push(self.parse_stmt()?);
                self.skip_newlines();
            }
            self.match_token(&Token::Dedent);
        } else {
            let expr = self.parse_expr()?;
            body.push(Stmt::Return(Some(expr)));
        }

        Ok(Expr::Closure {
            params,
            return_type,
            body,
        })
    }

    fn parse_add_sub(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_mul_div()?;

        while let Some(t) = self.peek() {
            let op = match t {
                Token::Plus => BinaryOperator::Add,
                Token::Minus => BinaryOperator::Subtract,
                _ => break,
            };
            self.advance();
            let right = self.parse_mul_div()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_mul_div(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_postfix()?;

        while let Some(t) = self.peek() {
            let op = match t {
                Token::Star => BinaryOperator::Multiply,
                Token::Slash => BinaryOperator::Divide,
                Token::Percent => BinaryOperator::Modulo,
                _ => break,
            };
            self.advance();
            let right = self.parse_postfix()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_postfix(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_primary()?;

        loop {
            if self.match_token(&Token::Dot) {
                let field = match self.advance() {
                    Some(Token::Ident(n)) => n.clone(),
                    _ => return self.error("Expected field name after '.'"),
                };
                expr = Expr::FieldAccess {
                    base: Box::new(expr),
                    field,
                };
            } else if self.match_token(&Token::LParen) {
                let mut args = Vec::new();
                if self.peek() != Some(&Token::RParen) {
                    loop {
                        args.push(self.parse_expr()?);
                        if !self.match_token(&Token::Comma) {
                            break;
                        }
                    }
                }
                if !self.match_token(&Token::RParen) {
                    return self.error("Expected ')' after arguments");
                }
                // Check for EnumName.Variant(args) pattern
                if let Expr::FieldAccess { base, field } = &expr {
                    if let Expr::Identifier(enum_name, _) = base.as_ref() {
                        expr = Expr::EnumVariantConstruct {
                            enum_name: enum_name.clone(),
                            variant: field.clone(),
                            args,
                        };
                        continue;
                    }
                }
                expr = Expr::Call {
                    callee: Box::new(expr),
                    args,
                };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        let t = match self.advance() {
            Some(t) => t.clone(),
            None => return self.error("Unexpected EOF"),
        };
        match t {
            Token::Int(v) => Ok(Expr::IntLiteral(v)),
            Token::FloatLit(v) => Ok(Expr::FloatLiteral(v)),
            Token::StringLit(s) => Ok(Expr::StringLiteral(s)),
            Token::BoolLit(b) => Ok(Expr::BoolLiteral(b)),
            Token::Ident(n) => Ok(Expr::Identifier(n, std::cell::Cell::new(None))),
            Token::LParen => {
                let expr = self.parse_expr()?;
                if !self.match_token(&Token::RParen) {
                    return self.error("Expected ')'");
                }
                Ok(expr)
            }
            Token::LBracket => {
                let mut elements = Vec::new();
                if self.peek() != Some(&Token::RBracket) {
                    loop {
                        elements.push(self.parse_expr()?);
                        if !self.match_token(&Token::Comma) {
                            break;
                        }
                    }
                }
                if !self.match_token(&Token::RBracket) {
                    return self.error("Expected ']'");
                }
                Ok(Expr::ListLiteral(elements))
            }
            other => self.error(format!("Unexpected token in expression: {:?}", other)),
        }
    }
}
