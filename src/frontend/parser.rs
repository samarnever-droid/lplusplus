use crate::ast::*;
use crate::lexer::Token;

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<&Token> {
        let t = self.tokens.get(self.pos);
        self.pos += 1;
        t
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
            } else if self.match_token(&Token::Import) {
                declarations.push(TopLevel::Import(self.parse_import()?));
            } else {
                return Err(format!("Expected 'def', 'struct', or 'import', found {:?}", self.peek()));
            }
            self.skip_newlines();
        }

        Ok(Program { declarations })
    }

    fn parse_import(&mut self) -> Result<String, String> {
        let name = match self.advance() {
            Some(Token::Ident(n)) => n.clone(),
            _ => return Err("Expected module name after 'import'".to_string()),
        };
        if self.peek() == Some(&Token::Newline) {
            self.advance();
        }
        Ok(name)
    }

    fn parse_struct(&mut self) -> Result<StructDef, String> {
        let name = match self.advance() {
            Some(Token::Ident(n)) => n.clone(),
            _ => return Err("Expected struct name".to_string()),
        };

        if !self.match_token(&Token::Colon) {
            return Err("Expected ':' after struct name".to_string());
        }

        if !self.match_token(&Token::Newline) {
            return Err("Expected newline after ':'".to_string());
        }

        self.skip_newlines();

        if !self.match_token(&Token::Indent) {
            return Err("Expected indentation for struct fields".to_string());
        }

        let mut fields = Vec::new();
        while self.peek() != Some(&Token::Dedent) && self.peek().is_some() {
            self.skip_newlines();
            if self.peek() == Some(&Token::Dedent) {
                break;
            }

            let field_name = match self.advance() {
                Some(Token::Ident(n)) => n.clone(),
                _ => return Err("Expected field name".to_string()),
            };

            if !self.match_token(&Token::Colon) {
                return Err("Expected ':' after field name".to_string());
            }

            let ty = self.parse_type()?;
            fields.push(Param { name: field_name, ty });

            self.skip_newlines();
        }
        self.match_token(&Token::Dedent);

        Ok(StructDef { name, fields })
    }

    fn parse_function(&mut self) -> Result<Function, String> {
        let name = match self.advance() {
            Some(Token::Ident(n)) => n.clone(),
            _ => return Err("Expected function name".to_string()),
        };

        if !self.match_token(&Token::LParen) {
            return Err("Expected '(' after function name".to_string());
        }

        let mut params = Vec::new();
        if self.peek() != Some(&Token::RParen) {
            loop {
                let param_name = match self.advance() {
                    Some(Token::Ident(n)) => n.clone(),
                    _ => return Err("Expected parameter name".to_string()),
                };
                if !self.match_token(&Token::Colon) {
                    return Err("Expected ':' after parameter name".to_string());
                }
                let ty = self.parse_type()?;
                params.push(Param { name: param_name, ty });

                if !self.match_token(&Token::Comma) {
                    break;
                }
            }
        }

        if !self.match_token(&Token::RParen) {
            return Err("Expected ')' after parameters".to_string());
        }

        let mut return_type = Type::Void;
        if self.match_token(&Token::Arrow) {
            return_type = self.parse_type()?;
        }

        if !self.match_token(&Token::Colon) {
            return Err("Expected ':' before function body".to_string());
        }

        if !self.match_token(&Token::Newline) {
            return Err("Expected newline after ':'".to_string());
        }

        self.skip_newlines();

        if !self.match_token(&Token::Indent) {
            return Err("Expected indentation for function body".to_string());
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
            _ => return Err("Expected a type".to_string()),
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
                return Err("Expected ']' after generic type arguments".to_string());
            }
            return Ok(Type::Generic(base_name, type_args));
        }

        match base_name.as_str() {
            "Int" => Ok(Type::Int),
            "Float" => Ok(Type::Float),
            "String" => Ok(Type::String),
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
                return Err("Expected ':' after if condition".to_string());
            }
            if !self.match_token(&Token::Newline) {
                return Err("Expected newline after ':'".to_string());
            }
            self.skip_newlines();
            if !self.match_token(&Token::Indent) {
                return Err("Expected indentation for if block".to_string());
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
                // BUG-08: Support chained `else if <cond>:` without requiring `elif` keyword
                if self.peek() == Some(&Token::If) {
                    // Parse the `else if` branch as a full nested if statement and wrap it
                    let elif_stmt = self.parse_stmt()?;
                    else_block = Some(vec![elif_stmt]);
                } else {
                    if !self.match_token(&Token::Colon) {
                        return Err("Expected ':' or 'if' after 'else'".to_string());
                    }
                    if !self.match_token(&Token::Newline) {
                        return Err("Expected newline after ':'".to_string());
                    }
                    self.skip_newlines();
                    if !self.match_token(&Token::Indent) {
                        return Err("Expected indentation for else block".to_string());
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
            
            return Ok(Stmt::If { condition, then_block, else_block });
        }

        if self.match_token(&Token::While) {
            let condition = self.parse_expr()?;
            if !self.match_token(&Token::Colon) {
                return Err("Expected ':' after while condition".to_string());
            }
            if !self.match_token(&Token::Newline) {
                return Err("Expected newline after ':'".to_string());
            }
            self.skip_newlines();
            if !self.match_token(&Token::Indent) {
                return Err("Expected indentation for while block".to_string());
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

        if self.match_token(&Token::For) {
            let var_name = match self.advance() {
                Some(Token::Ident(n)) => n.clone(),
                _ => return Err("Expected identifier after 'for'".to_string()),
            };
            if !self.match_token(&Token::In) {
                return Err("Expected 'in' after variable in 'for' loop".to_string());
            }
            let list_expr = self.parse_expr()?;
            if !self.match_token(&Token::Colon) {
                return Err("Expected ':' after for loop list expression".to_string());
            }
            if !self.match_token(&Token::Newline) {
                return Err("Expected newline after ':' in 'for' loop".to_string());
            }
            self.skip_newlines();
            if !self.match_token(&Token::Indent) {
                return Err("Expected indentation for 'for' block".to_string());
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

            // `for i in range(end)` and `for i in range(start, end)` lower
            // directly to integer while-loop MIR. `range` never reaches name
            // resolution as a runtime function, so there is no hidden list
            // allocation or iterator object in native code.
            if let Expr::Call { callee, args } = &list_expr {
                if matches!(&**callee, Expr::Identifier(name, _) if name == "range") {
                    let (start, end) = match args.as_slice() {
                        [end] => (Expr::IntLiteral(0), end.clone()),
                        [start, end] => (start.clone(), end.clone()),
                        _ => return Err("range in a for loop expects range(end) or range(start, end)".to_string()),
                    };
                    let unique_id = self.pos;
                    let idx_var = format!("__lpp_for_range_idx_{}", unique_id);
                    let idx_decl = Stmt::LetInferred {
                        name: idx_var.clone(), is_mut: true, value: start,
                        binding_id: std::cell::Cell::new(None),
                    };
                    let condition = Expr::BinaryOp {
                        left: Box::new(Expr::Identifier(idx_var.clone(), std::cell::Cell::new(None))),
                        op: BinaryOperator::Less, right: Box::new(end),
                    };
                    let mut while_body = vec![Stmt::LetInferred {
                        name: var_name, is_mut: true,
                        value: Expr::Identifier(idx_var.clone(), std::cell::Cell::new(None)),
                        binding_id: std::cell::Cell::new(None),
                    }];
                    while_body.extend(body);
                    while_body.push(Stmt::Assign {
                        name: idx_var.clone(),
                        value: Expr::BinaryOp {
                            left: Box::new(Expr::Identifier(idx_var, std::cell::Cell::new(None))),
                            op: BinaryOperator::Add, right: Box::new(Expr::IntLiteral(1)),
                        }, binding_id: std::cell::Cell::new(None),
                    });
                    return Ok(Stmt::Block(vec![idx_decl, Stmt::While { condition, body: while_body }]));
                }
            }

            let unique_id = self.pos;
            let list_var = format!("__lpp_for_list_{}", unique_id);
            let idx_var = format!("__lpp_for_idx_{}", unique_id);

            let list_decl = Stmt::LetInferred {
                name: list_var.clone(),
                is_mut: false,
                value: list_expr,
                binding_id: std::cell::Cell::new(None),
            };

            let idx_decl = Stmt::LetInferred {
                name: idx_var.clone(),
                is_mut: true,
                value: Expr::IntLiteral(0),
                binding_id: std::cell::Cell::new(None),
            };

            let while_cond = Expr::BinaryOp {
                left: Box::new(Expr::Identifier(idx_var.clone(), std::cell::Cell::new(None))),
                op: BinaryOperator::Less,
                right: Box::new(Expr::Call {
                    callee: Box::new(Expr::Identifier("list_len".to_string(), std::cell::Cell::new(None))),
                    args: vec![Expr::Identifier(list_var.clone(), std::cell::Cell::new(None))],
                }),
            };

            let mut while_body = Vec::new();

            let var_decl = Stmt::LetInferred {
                name: var_name,
                is_mut: true,
                value: Expr::Call {
                    callee: Box::new(Expr::Identifier("list_get".to_string(), std::cell::Cell::new(None))),
                    args: vec![
                        Expr::Identifier(list_var.clone(), std::cell::Cell::new(None)),
                        Expr::Identifier(idx_var.clone(), std::cell::Cell::new(None)),
                    ],
                },
                binding_id: std::cell::Cell::new(None),
            };
            while_body.push(var_decl);
            while_body.extend(body);

            let increment = Stmt::Assign {
                name: idx_var.clone(),
                value: Expr::BinaryOp {
                    left: Box::new(Expr::Identifier(idx_var, std::cell::Cell::new(None))),
                    op: BinaryOperator::Add,
                    right: Box::new(Expr::IntLiteral(1)),
                },
                binding_id: std::cell::Cell::new(None),
            };
            while_body.push(increment);

            let while_stmt = Stmt::While {
                condition: while_cond,
                body: while_body,
            };

            return Ok(Stmt::Block(vec![list_decl, idx_decl, while_stmt]));
        }

        if self.match_token(&Token::Return) {
            if self.peek() == Some(&Token::Newline) || self.peek() == Some(&Token::Dedent) {
                return Ok(Stmt::Return(None));
            }
            let expr = self.parse_expr()?;
            return Ok(Stmt::Return(Some(expr)));
        }

        if self.match_token(&Token::Mut) {
            let name = match self.advance() {
                Some(Token::Ident(n)) => n.clone(),
                _ => return Err("Expected identifier after 'mut'".to_string()),
            };
            if !self.match_token(&Token::Assign) {
                return Err("Expected ':=' after mutable identifier".to_string());
            }
            let value = self.parse_expr()?;
            return Ok(Stmt::LetInferred { name, is_mut: true, value, binding_id: std::cell::Cell::new(None) });
        }

        if let Some(Token::Ident(name)) = self.peek().cloned() {
            let next = self.tokens.get(self.pos + 1);
            if next == Some(&Token::Assign) {
                self.advance(); // consume name
                self.advance(); // consume :=
                let value = self.parse_expr()?;
                return Ok(Stmt::LetInferred { name: name.clone(), is_mut: false, value, binding_id: std::cell::Cell::new(None) });
            }
        }

        let expr = self.parse_expr()?;
        if self.match_token(&Token::Equal) {
            let value = self.parse_expr()?;
            match expr {
                Expr::Identifier(name, _) => {
                    return Ok(Stmt::Assign { name, value, binding_id: std::cell::Cell::new(None) });
                }
                Expr::FieldAccess { base, field } => {
                    return Ok(Stmt::AssignField { base: *base, field, value });
                }
                _ => return Err("Invalid assignment target".to_string()),
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
            return Ok(Expr::Spawn { closure: Box::new(closure) });
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
            return Err("Expected '(' after 'fn'".to_string());
        }

        let mut params = Vec::new();
        if self.peek() != Some(&Token::RParen) {
            loop {
                let param_name = match self.advance() {
                    Some(Token::Ident(n)) => n.clone(),
                    _ => return Err("Expected parameter name in closure".to_string()),
                };
                
                let mut ty = None;
                if self.match_token(&Token::Colon) {
                    ty = Some(self.parse_type()?);
                }
                
                params.push(ClosureParam { name: param_name, ty });

                if !self.match_token(&Token::Comma) {
                    break;
                }
            }
        }

        if !self.match_token(&Token::RParen) {
            return Err("Expected ')' after closure parameters".to_string());
        }

        let mut return_type = None;
        if self.match_token(&Token::Arrow) {
            return_type = Some(self.parse_type()?);
        }

        if !self.match_token(&Token::Colon) {
            return Err("Expected ':' before closure body".to_string());
        }

        let mut body = Vec::new();
        if self.match_token(&Token::Newline) {
            // Block body
            if !self.match_token(&Token::Indent) {
                return Err("Expected indentation for closure body".to_string());
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
            // Inline body
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
                    _ => return Err("Expected field name after '.'".to_string()),
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
                    return Err("Expected ')' after arguments".to_string());
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
        let t = self.advance().ok_or_else(|| "Unexpected EOF".to_string())?;
        match t {
            Token::Int(v) => Ok(Expr::IntLiteral(*v)),
            Token::FloatLit(v) => Ok(Expr::FloatLiteral(*v)),
            Token::StringLit(s) => Ok(Expr::StringLiteral(s.clone())),
            Token::BoolLit(b) => Ok(Expr::BoolLiteral(*b)),
            Token::Ident(n) => Ok(Expr::Identifier(n.clone(), std::cell::Cell::new(None))),
            Token::LParen => {
                let expr = self.parse_expr()?;
                if !self.match_token(&Token::RParen) {
                    return Err("Expected ')'".to_string());
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
                    return Err("Expected ']'".to_string());
                }
                Ok(Expr::ListLiteral(elements))
            }
            _ => Err(format!("Unexpected token in expression: {:?}", t)),
        }
    }
}
