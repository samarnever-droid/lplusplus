use super::*;

impl<'input> MirBuilder<'input> {
    pub(super) fn compute_captures(&self, body: BodyId, scope: ScopeId) -> Vec<LocalId> {
        let mut referenced: BTreeSet<LocalId> = BTreeSet::new();
        let mut visit = |expression: ExprId| {
            if let ExpressionKind::Name {
                binding: NameBinding::Local(local),
                ..
            } = self.package.expressions[expression].kind
            {
                referenced.insert(local);
            }
        };
        self.walk_body(body, &mut visit);
        let mut captures = Vec::new();
        for local in referenced {
            if !Self::scope_is_inside(self.package, self.package.locals[local].scope, scope) {
                captures.push(local);
            }
        }
        captures
    }

    pub(super) fn walk_body(&self, body: BodyId, visit: &mut dyn FnMut(ExprId)) {
        for statement in self
            .package
            .statements(self.package.bodies[body].statements)
        {
            self.walk_statement(*statement, visit);
        }
    }

    pub(super) fn walk_statement(&self, statement: StmtId, visit: &mut dyn FnMut(ExprId)) {
        match self.package.statements[statement].kind {
            StatementKind::Let { bindings: _, value } => self.walk_expression(value, visit),
            StatementKind::Assign { target, value } => {
                self.walk_expression(target, visit);
                self.walk_expression(value, visit);
            }
            StatementKind::Expression(expression) => self.walk_expression(expression, visit),
            StatementKind::Return(value) => {
                if let Some(value) = value {
                    self.walk_expression(value, visit);
                }
            }
            StatementKind::If {
                condition,
                then_body,
                else_body,
            } => {
                self.walk_expression(condition, visit);
                self.walk_body(then_body, visit);
                if let Some(else_body) = else_body {
                    self.walk_body(else_body, visit);
                }
            }
            StatementKind::While { condition, body } => {
                self.walk_expression(condition, visit);
                self.walk_body(body, visit);
            }
            StatementKind::For {
                binding: _,
                iterable,
                body,
            } => {
                self.walk_expression(iterable, visit);
                self.walk_body(body, visit);
            }
            StatementKind::Match { subject, arms } => {
                self.walk_expression(subject, visit);
                for arm in self.package.match_arms(arms) {
                    self.walk_body(self.package.match_arms[*arm].body, visit);
                }
            }
            StatementKind::Unsafe { body } => self.walk_body(body, visit),
            StatementKind::Break | StatementKind::Continue => {}
        }
    }

    pub(super) fn walk_expression(&self, expression: ExprId, visit: &mut dyn FnMut(ExprId)) {
        visit(expression);
        match self.package.expressions[expression].kind {
            ExpressionKind::Literal(_) | ExpressionKind::Name { .. } => {}
            ExpressionKind::Unary { operand, .. } | ExpressionKind::Cast { value: operand, .. } => {
                self.walk_expression(operand, visit)
            }
            ExpressionKind::Binary { left, right, .. } => {
                self.walk_expression(left, visit);
                self.walk_expression(right, visit);
            }
            ExpressionKind::Tuple(elements) | ExpressionKind::List(elements) => {
                for element in self.package.expressions(elements) {
                    self.walk_expression(*element, visit);
                }
            }
            ExpressionKind::Call { callee, arguments } => {
                self.walk_expression(callee, visit);
                for argument in self.package.expressions(arguments) {
                    self.walk_expression(*argument, visit);
                }
            }
            ExpressionKind::GenericCall {
                callee,
                type_arguments: _,
                arguments,
            } => {
                self.walk_expression(callee, visit);
                for argument in self.package.expressions(arguments) {
                    self.walk_expression(*argument, visit);
                }
            }
            ExpressionKind::Field { base, .. } => self.walk_expression(base, visit),
            ExpressionKind::Index { base, index } => {
                self.walk_expression(base, visit);
                self.walk_expression(index, visit);
            }
            ExpressionKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.walk_expression(condition, visit);
                self.walk_expression(then_expr, visit);
                self.walk_expression(else_expr, visit);
            }
            ExpressionKind::Try(inner)
            | ExpressionKind::Await(inner)
            | ExpressionKind::Spawn(inner) => self.walk_expression(inner, visit),
            ExpressionKind::Closure {
                parameters: _,
                return_type: _,
                body,
            } => self.walk_body(body, visit),
        }
    }

    /// By-reference capture cells: before any local of `body` is
    /// materialized, mark every local that a closure contained in `body`
    /// (at any nesting depth) assigns to. The mark makes the local lower
    /// as a single-element list shared with the closure. A local that is
    /// a parameter of any enclosing function is never marked: the call
    /// ABI passes parameters by value, so that capture stays the
    /// one-way value capture.
    pub(super) fn mark_write_captures(
        &mut self,
        body: BodyId,
        function_parameters: &BTreeSet<LocalId>,
    ) {
        self.mark_write_captures_inner(body, function_parameters);
    }

    /// The same walk restricted to a function body: `enclosing_params`
    /// holds the parameter locals of every function whose body contains
    /// this one (the item function and each enclosing closure).
    pub(super) fn mark_write_captures_inner(
        &mut self,
        body: BodyId,
        enclosing_params: &BTreeSet<LocalId>,
    ) {
        for &statement in self
            .package
            .statements(self.package.bodies[body].statements)
        {
            self.mark_write_captures_statement(statement, enclosing_params);
        }
    }

    pub(super) fn mark_write_captures_statement(
        &mut self,
        statement: StmtId,
        enclosing_params: &BTreeSet<LocalId>,
    ) {
        match self.package.statements[statement].kind {
            StatementKind::Let { value, .. } => {
                self.mark_write_captures_expression(value, enclosing_params)
            }
            StatementKind::Assign { target, value } => {
                self.mark_write_captures_expression(target, enclosing_params);
                self.mark_write_captures_expression(value, enclosing_params);
            }
            StatementKind::Expression(expression) => {
                self.mark_write_captures_expression(expression, enclosing_params)
            }
            StatementKind::Return(value) => {
                if let Some(value) = value {
                    self.mark_write_captures_expression(value, enclosing_params);
                }
            }
            StatementKind::If {
                condition,
                then_body,
                else_body,
            } => {
                self.mark_write_captures_expression(condition, enclosing_params);
                self.mark_write_captures_inner(then_body, enclosing_params);
                if let Some(else_body) = else_body {
                    self.mark_write_captures_inner(else_body, enclosing_params);
                }
            }
            StatementKind::While { condition, body } => {
                self.mark_write_captures_expression(condition, enclosing_params);
                self.mark_write_captures_inner(body, enclosing_params);
            }
            StatementKind::For { iterable, body, .. } => {
                self.mark_write_captures_expression(iterable, enclosing_params);
                self.mark_write_captures_inner(body, enclosing_params);
            }
            StatementKind::Match { subject, arms } => {
                self.mark_write_captures_expression(subject, enclosing_params);
                for &arm in self.package.match_arms(arms) {
                    self.mark_write_captures_inner(
                        self.package.match_arms[arm].body,
                        enclosing_params,
                    );
                }
            }
            StatementKind::Unsafe { body } => {
                self.mark_write_captures_inner(body, enclosing_params);
            }
            StatementKind::Break | StatementKind::Continue => {}
        }
    }

    pub(super) fn mark_write_captures_expression(
        &mut self,
        expression: ExprId,
        enclosing_params: &BTreeSet<LocalId>,
    ) {
        match self.package.expressions[expression].kind {
            ExpressionKind::Closure {
                parameters, body, ..
            } => {
                // This closure's own assignments to locals visible above
                // it become cell captures.
                let mut extended = enclosing_params.clone();
                extended.extend(self.package.locals(parameters).iter().copied());
                let scope = self.package.bodies[body].scope;
                Self::collect_capture_writes(
                    self.package,
                    body,
                    scope,
                    &extended,
                    &mut self.cell_sources,
                );
                // Nested closures live inside this body.
                self.mark_write_captures_inner(body, &extended);
            }
            ExpressionKind::Literal(_) | ExpressionKind::Name { .. } => {}
            ExpressionKind::Unary { operand, .. } | ExpressionKind::Cast { value: operand, .. } => {
                self.mark_write_captures_expression(operand, enclosing_params)
            }
            ExpressionKind::Binary { left, right, .. } => {
                self.mark_write_captures_expression(left, enclosing_params);
                self.mark_write_captures_expression(right, enclosing_params);
            }
            ExpressionKind::Tuple(elements) | ExpressionKind::List(elements) => {
                for &element in self.package.expressions(elements) {
                    self.mark_write_captures_expression(element, enclosing_params);
                }
            }
            ExpressionKind::Call { callee, arguments } => {
                self.mark_write_captures_expression(callee, enclosing_params);
                for &argument in self.package.expressions(arguments) {
                    self.mark_write_captures_expression(argument, enclosing_params);
                }
            }
            ExpressionKind::GenericCall {
                callee,
                type_arguments: _,
                arguments,
            } => {
                self.mark_write_captures_expression(callee, enclosing_params);
                for &argument in self.package.expressions(arguments) {
                    self.mark_write_captures_expression(argument, enclosing_params);
                }
            }
            ExpressionKind::Field { base, .. } => {
                self.mark_write_captures_expression(base, enclosing_params)
            }
            ExpressionKind::Index { base, index } => {
                self.mark_write_captures_expression(base, enclosing_params);
                self.mark_write_captures_expression(index, enclosing_params);
            }
            ExpressionKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.mark_write_captures_expression(condition, enclosing_params);
                self.mark_write_captures_expression(then_expr, enclosing_params);
                self.mark_write_captures_expression(else_expr, enclosing_params);
            }
            ExpressionKind::Try(inner)
            | ExpressionKind::Await(inner)
            | ExpressionKind::Spawn(inner) => {
                self.mark_write_captures_expression(inner, enclosing_params)
            }
        }
    }

    /// The closure's direct assignments (through its own control flow,
    /// excluding nested closures — each of those is marked by its own
    /// visit) that target a local outside the closure scope.
    pub(super) fn collect_capture_writes(
        package: &HirPackage,
        body: BodyId,
        scope: ScopeId,
        enclosing_params: &BTreeSet<LocalId>,
        cell_sources: &mut BTreeSet<LocalId>,
    ) {
        Self::collect_writes_statements(package, body, scope, enclosing_params, cell_sources);
    }

    pub(super) fn collect_writes_statements(
        package: &HirPackage,
        body: BodyId,
        scope: ScopeId,
        enclosing_params: &BTreeSet<LocalId>,
        cell_sources: &mut BTreeSet<LocalId>,
    ) {
        for &statement in package.statements(package.bodies[body].statements) {
            match package.statements[statement].kind {
                StatementKind::Assign { target, .. } => {
                    if let Some(local) = Self::assign_target_root_local(package, target)
                        && !Self::scope_is_inside_local(package, local, scope)
                        && !enclosing_params.contains(&local)
                    {
                        cell_sources.insert(local);
                    }
                }
                StatementKind::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    Self::collect_writes_statements(
                        package,
                        then_body,
                        scope,
                        enclosing_params,
                        cell_sources,
                    );
                    if let Some(else_body) = else_body {
                        Self::collect_writes_statements(
                            package,
                            else_body,
                            scope,
                            enclosing_params,
                            cell_sources,
                        );
                    }
                }
                StatementKind::While { body, .. }
                | StatementKind::For { body, .. }
                | StatementKind::Unsafe { body } => {
                    Self::collect_writes_statements(
                        package,
                        body,
                        scope,
                        enclosing_params,
                        cell_sources,
                    );
                }
                StatementKind::Match { arms, .. } => {
                    for &arm in package.match_arms(arms) {
                        Self::collect_writes_statements(
                            package,
                            package.match_arms[arm].body,
                            scope,
                            enclosing_params,
                            cell_sources,
                        );
                    }
                }
                _ => {}
            }
        }
    }

    /// The local an assignment target roots at, walking field and index
    /// projections to the base name; `None` for non-place targets.
    pub(super) fn assign_target_root_local(
        package: &HirPackage,
        expression: ExprId,
    ) -> Option<LocalId> {
        match package.expressions[expression].kind {
            ExpressionKind::Name {
                binding: NameBinding::Local(local),
                ..
            } => Some(local),
            ExpressionKind::Field { base, .. } | ExpressionKind::Index { base, .. } => {
                Self::assign_target_root_local(package, base)
            }
            _ => None,
        }
    }

    pub(super) fn scope_is_inside_local(
        package: &HirPackage,
        local: LocalId,
        ancestor: ScopeId,
    ) -> bool {
        Self::scope_is_inside(package, package.locals[local].scope, ancestor)
    }

    pub(super) fn scope_is_inside(package: &HirPackage, scope: ScopeId, ancestor: ScopeId) -> bool {
        let mut current = Some(scope);
        while let Some(candidate) = current {
            if candidate == ancestor {
                return true;
            }
            current = package.scopes[candidate].parent;
        }
        false
    }

    pub(super) fn reserve_function(
        &mut self,
        current: usize,
        origin: OriginId,
    ) -> Result<MirFunctionId, MirBuildError> {
        self.check_next(
            MirCapacity::Functions,
            current,
            self.options.max_functions,
            origin,
        )?;
        let id = MirFunctionId::from_index(current).ok_or_else(|| {
            self.error(origin, MirBuildErrorKind::Capacity(MirCapacity::Functions))
        })?;
        // The ID is owned from reservation time: bodies may finish in any
        // order (closures finish before their enclosing function), so the
        // count must not wait for the allocation.
        self.counts.functions = current + 1;
        Ok(id)
    }
}
