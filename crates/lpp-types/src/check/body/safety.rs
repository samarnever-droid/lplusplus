use super::*;

impl<'hir> TypeChecker<'hir> {
    pub(in crate::check) fn validate_item_safety(
        &mut self,
        _item_id: HirItemId,
        item: HirItem,
    ) -> Result<(), ShadowTypeError> {
        let HirItemKind::Function(function) = item.kind else {
            return Ok(());
        };
        let Some(body) = function.body else {
            return Ok(());
        };
        self.validate_body_safety(body, function.is_async, function.is_unsafe, None)
    }

    pub(in crate::check) fn validate_body_safety(
        &mut self,
        body: BodyId,
        in_async: bool,
        in_unsafe: bool,
        closure_scope: Option<ScopeId>,
    ) -> Result<(), ShadowTypeError> {
        let mut borrowed_sources = BTreeSet::new();
        let statements = self
            .package
            .statements(self.package.bodies[body].statements)
            .to_vec();
        for statement in statements {
            self.validate_statement_safety(
                statement,
                in_async,
                in_unsafe,
                closure_scope,
                &mut borrowed_sources,
            )?;
        }
        Ok(())
    }

    pub(in crate::check) fn validate_statement_safety(
        &mut self,
        statement_id: StmtId,
        in_async: bool,
        in_unsafe: bool,
        closure_scope: Option<ScopeId>,
        borrowed_sources: &mut BTreeSet<LocalId>,
    ) -> Result<(), ShadowTypeError> {
        let statement = self.package.statements[statement_id];
        match statement.kind {
            StatementKind::Let { value, .. } => {
                self.validate_expression_safety(value, in_async, in_unsafe, closure_scope)?;
                if let Some(source) = self.slice_source_local(value) {
                    borrowed_sources.insert(source);
                }
            }
            StatementKind::Assign { target, value } => {
                self.validate_expression_safety(target, in_async, in_unsafe, closure_scope)?;
                self.validate_expression_safety(value, in_async, in_unsafe, closure_scope)?;
                if let Some(local) = self.name_local(target)
                    && borrowed_sources.contains(&local)
                {
                    return Err(self.at(
                        statement.origin,
                        TypeError::BorrowedSliceEscape {
                            reason: "source reassigned while a slice view is live",
                        },
                    ));
                }
            }
            StatementKind::Expression(expression) => {
                self.validate_expression_safety(expression, in_async, in_unsafe, closure_scope)?;
            }
            StatementKind::Return(value) => {
                if let Some(value) = value {
                    self.validate_expression_safety(value, in_async, in_unsafe, closure_scope)?;
                    if self.expression_is_slice(value) {
                        return Err(self.at(
                            statement.origin,
                            TypeError::BorrowedSliceEscape {
                                reason: "slice view returned from its owner scope",
                            },
                        ));
                    }
                }
            }
            StatementKind::If {
                condition,
                then_body,
                else_body,
            } => {
                self.validate_expression_safety(condition, in_async, in_unsafe, closure_scope)?;
                self.validate_body_safety(then_body, in_async, in_unsafe, closure_scope)?;
                if let Some(else_body) = else_body {
                    self.validate_body_safety(else_body, in_async, in_unsafe, closure_scope)?;
                }
            }
            StatementKind::While { condition, body } => {
                self.validate_expression_safety(condition, in_async, in_unsafe, closure_scope)?;
                self.validate_body_safety(body, in_async, in_unsafe, closure_scope)?;
            }
            StatementKind::Unsafe { body } => {
                self.validate_body_safety(body, in_async, true, closure_scope)?;
            }
            StatementKind::For { iterable, body, .. } => {
                self.validate_expression_safety(iterable, in_async, in_unsafe, closure_scope)?;
                self.validate_body_safety(body, in_async, in_unsafe, closure_scope)?;
            }
            StatementKind::Match { subject, arms } => {
                self.validate_expression_safety(subject, in_async, in_unsafe, closure_scope)?;
                for arm in self.package.match_arms(arms).to_vec() {
                    self.validate_body_safety(
                        self.package.match_arms[arm].body,
                        in_async,
                        in_unsafe,
                        closure_scope,
                    )?;
                }
            }
            StatementKind::Break | StatementKind::Continue => {}
        }
        Ok(())
    }

    pub(in crate::check) fn validate_expression_safety(
        &mut self,
        expression_id: ExprId,
        in_async: bool,
        in_unsafe: bool,
        closure_scope: Option<ScopeId>,
    ) -> Result<(), ShadowTypeError> {
        let expression = self.package.expressions[expression_id];
        if let Some(scope) = closure_scope
            && let Some(local) = self.name_local(expression_id)
            && !self.scope_is_inside(self.package.locals[local].scope, scope)
        {
            if self.local_is_slice(local) {
                return Err(self.at(
                    expression.origin,
                    TypeError::BorrowedSliceEscape {
                        reason: "slice view captured by a closure",
                    },
                ));
            }
            if self.local_is_task(local) {
                return Err(self.at(expression.origin, TypeError::UnsafeAsyncTaskCapture));
            }
        }

        match expression.kind {
            ExpressionKind::Unary { operand, .. } | ExpressionKind::Try(operand) => {
                self.validate_expression_safety(operand, in_async, in_unsafe, closure_scope)?;
            }
            ExpressionKind::Binary { left, right, .. } => {
                self.validate_expression_safety(left, in_async, in_unsafe, closure_scope)?;
                self.validate_expression_safety(right, in_async, in_unsafe, closure_scope)?;
            }
            ExpressionKind::Tuple(elements) | ExpressionKind::List(elements) => {
                for element in self.package.expressions(elements).to_vec() {
                    self.validate_expression_safety(element, in_async, in_unsafe, closure_scope)?;
                }
            }
            ExpressionKind::Call { callee, arguments }
            | ExpressionKind::GenericCall {
                callee, arguments, ..
            } => {
                if let Some(name) = self.expression_name(callee) {
                    if in_async && let Some(builtin) = blocking_builtin_label(name) {
                        return Err(self.at(
                            expression.origin,
                            TypeError::UnsafeAsyncBlocking { builtin },
                        ));
                    }
                    if !in_unsafe && let Some(operation) = Self::unsafe_intrinsic_label(name) {
                        return Err(
                            self.at(expression.origin, TypeError::UnsafeRequired { operation })
                        );
                    }
                }
                if !in_unsafe && self.callee_is_unsafe_function(callee) {
                    return Err(self.at(
                        expression.origin,
                        TypeError::UnsafeRequired {
                            operation: "call to unsafe function",
                        },
                    ));
                }
                self.validate_expression_safety(callee, in_async, in_unsafe, closure_scope)?;
                for argument in self.package.expressions(arguments).to_vec() {
                    self.validate_expression_safety(argument, in_async, in_unsafe, closure_scope)?;
                }
            }
            ExpressionKind::Field { base, .. } => {
                self.validate_expression_safety(base, in_async, in_unsafe, closure_scope)?;
            }
            ExpressionKind::Index { base, index } => {
                self.validate_expression_safety(base, in_async, in_unsafe, closure_scope)?;
                self.validate_expression_safety(index, in_async, in_unsafe, closure_scope)?;
            }
            ExpressionKind::Cast { value, .. } => {
                self.validate_expression_safety(value, in_async, in_unsafe, closure_scope)?;
                if !in_unsafe
                    && let Some(target_ty) = self.assignments.expression(expression_id)
                    && self.type_is_raw_pointer(target_ty)
                {
                    return Err(self.at(
                        expression.origin,
                        TypeError::UnsafeRequired {
                            operation: "cast to raw pointer",
                        },
                    ));
                }
            }
            ExpressionKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.validate_expression_safety(condition, in_async, in_unsafe, closure_scope)?;
                self.validate_expression_safety(then_expr, in_async, in_unsafe, closure_scope)?;
                self.validate_expression_safety(else_expr, in_async, in_unsafe, closure_scope)?;
            }
            ExpressionKind::Await(inner) | ExpressionKind::Spawn(inner) => {
                self.validate_expression_safety(inner, in_async, in_unsafe, closure_scope)?;
            }
            ExpressionKind::Closure { body, .. } => {
                let scope = self.package.bodies[body].scope;
                self.validate_body_safety(body, in_async, in_unsafe, Some(scope))?;
            }
            ExpressionKind::Literal(_) | ExpressionKind::Name { .. } => {}
        }
        Ok(())
    }

    pub(in crate::check) fn expression_name(&self, expression_id: ExprId) -> Option<&str> {
        let ExpressionKind::Name { symbol, .. } = self.package.expressions[expression_id].kind
        else {
            return None;
        };
        self.package.names.symbols.resolve(symbol)
    }

    pub(in crate::check) fn name_local(&self, expression_id: ExprId) -> Option<LocalId> {
        let ExpressionKind::Name {
            binding: NameBinding::Local(local),
            ..
        } = self.package.expressions[expression_id].kind
        else {
            return None;
        };
        Some(local)
    }

    pub(in crate::check) fn slice_source_local(&self, expression_id: ExprId) -> Option<LocalId> {
        let (ExpressionKind::Call { callee, arguments }
        | ExpressionKind::GenericCall {
            callee, arguments, ..
        }) = self.package.expressions[expression_id].kind
        else {
            return None;
        };
        match self.expression_name(callee) {
            Some("slice" | "str_slice") => {}
            _ => return None,
        }
        let source = *self.package.expressions(arguments).first()?;
        self.name_local(source)
    }

    pub(in crate::check) fn expression_is_slice(&mut self, expression_id: ExprId) -> bool {
        let Some(type_id) = self.assignments.expressions[expression_id.index()] else {
            return false;
        };
        self.type_is_slice(type_id)
    }

    pub(in crate::check) fn local_is_slice(&mut self, local: LocalId) -> bool {
        let Some(type_id) = self.assignments.locals[local.index()] else {
            return false;
        };
        self.type_is_slice(type_id)
    }

    pub(in crate::check) fn local_is_task(&mut self, local: LocalId) -> bool {
        let Some(type_id) = self.assignments.locals[local.index()] else {
            return false;
        };
        let Ok(type_id) = self
            .inference
            .resolve(&self.interner, type_id, &mut self.budget)
        else {
            return false;
        };
        matches!(self.interner.kind(type_id), TypeKind::Task(_))
    }

    pub(in crate::check) fn type_is_slice(&mut self, type_id: TypeId) -> bool {
        let Ok(type_id) = self
            .inference
            .resolve(&self.interner, type_id, &mut self.budget)
        else {
            return false;
        };
        matches!(
            self.interner.kind(type_id),
            TypeKind::Slice(_) | TypeKind::Primitive(PrimitiveType::StrSlice)
        )
    }

    pub(in crate::check) fn scope_is_inside(&self, scope: ScopeId, ancestor: ScopeId) -> bool {
        let mut current = Some(scope);
        while let Some(candidate) = current {
            if candidate == ancestor {
                return true;
            }
            current = self.package.scopes[candidate].parent;
        }
        false
    }

    fn callee_is_unsafe_function(&self, callee: ExprId) -> bool {
        let Some(def) = self.package.expression_definition(callee) else {
            return false;
        };
        let Some(item) = self.definition_items.get(def.index()).and_then(|i| *i) else {
            return false;
        };
        matches!(
            self.package.items[item].kind,
            HirItemKind::Function(function) if function.is_unsafe
        )
    }

    pub(super) fn type_is_raw_pointer(&mut self, type_id: TypeId) -> bool {
        let Ok(resolved) = self
            .inference
            .resolve(&self.interner, type_id, &mut self.budget)
        else {
            return false;
        };
        matches!(
            self.interner.kind(resolved),
            TypeKind::Ptr(_) | TypeKind::Primitive(PrimitiveType::RawPtr)
        )
    }

    pub(in crate::check) fn check_spawn_capture_target(
        &self,
        target: ExprId,
        origin: OriginId,
    ) -> Result<(), ShadowTypeError> {
        let Some(ancestor) = self.spawn_closure_scope else {
            return Ok(());
        };
        let ExpressionKind::Name {
            binding: NameBinding::Local(local),
            ..
        } = self.package.expressions[target].kind
        else {
            return Ok(());
        };
        if !self.scope_is_inside(self.package.locals[local].scope, ancestor) {
            return Err(self.at(origin, TypeError::SpawnCaptureMutation { local }));
        }
        Ok(())
    }
}
