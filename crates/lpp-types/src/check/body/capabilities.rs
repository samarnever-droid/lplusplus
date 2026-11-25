//! Compile-time capability and effect enforcement (`@pure`, `@deny(...)`).
//!
//! Inspired by BOIL's per-function capability headers and Tamed Pict's
//! trusted/untrusted FFI boundary, this pass computes each function's
//! transitive effect set over the static call graph and verifies every
//! `@pure` or `@deny(...)` contract at compile time with zero runtime cost.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct CapabilitySet(u16);

impl CapabilitySet {
    pub(super) const EMPTY: Self = Self(0);
    pub(super) const FS: Self = Self(1 << 0);
    pub(super) const NET: Self = Self(1 << 1);
    pub(super) const EXEC: Self = Self(1 << 2);
    pub(super) const ENV: Self = Self(1 << 3);
    pub(super) const IO: Self = Self(1 << 4);
    pub(super) const FFI: Self = Self(1 << 5);
    pub(super) const UNSAFE: Self = Self(1 << 6);
    pub(super) const SPAWN: Self = Self(1 << 7);
    pub(super) const TIME: Self = Self(1 << 8);
    pub(super) const RAND: Self = Self(1 << 9);
    pub(super) const GUI: Self = Self(1 << 10);
    pub(super) const SYS: Self = Self(1 << 11);
    pub(super) const ALL: Self = Self((1 << 12) - 1);

    const fn is_empty(self) -> bool {
        self.0 == 0
    }

    const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    const fn intersect(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "fs" => Self::FS,
            "net" => Self::NET,
            "exec" => Self::EXEC,
            "env" => Self::ENV,
            "io" => Self::IO,
            "ffi" => Self::FFI,
            "unsafe" => Self::UNSAFE,
            "spawn" => Self::SPAWN,
            "time" => Self::TIME,
            "rand" => Self::RAND,
            "gui" => Self::GUI,
            "sys" => Self::SYS,
            _ => return None,
        })
    }

    const fn first_name(self) -> Option<&'static str> {
        if self.0 & Self::FS.0 != 0 {
            Some("fs")
        } else if self.0 & Self::NET.0 != 0 {
            Some("net")
        } else if self.0 & Self::EXEC.0 != 0 {
            Some("exec")
        } else if self.0 & Self::ENV.0 != 0 {
            Some("env")
        } else if self.0 & Self::IO.0 != 0 {
            Some("io")
        } else if self.0 & Self::FFI.0 != 0 {
            Some("ffi")
        } else if self.0 & Self::UNSAFE.0 != 0 {
            Some("unsafe")
        } else if self.0 & Self::SPAWN.0 != 0 {
            Some("spawn")
        } else if self.0 & Self::TIME.0 != 0 {
            Some("time")
        } else if self.0 & Self::RAND.0 != 0 {
            Some("rand")
        } else if self.0 & Self::GUI.0 != 0 {
            Some("gui")
        } else if self.0 & Self::SYS.0 != 0 {
            Some("sys")
        } else {
            None
        }
    }
}

/// Classify a builtin name into its required capability set.
fn builtin_capabilities(raw_name: &str) -> CapabilitySet {
    let name = raw_name.strip_prefix("lpp_").unwrap_or(raw_name);
    if matches!(
        name,
        "read_file" | "write_file" | "append_file" | "delete_file" | "is_file" | "is_dir" | "chdir"
    ) || name.starts_with("file_")
        || name.starts_with("dir_")
        || name.starts_with("fd_")
        || name.starts_with("path_")
    {
        return CapabilitySet::FS;
    }
    if name.starts_with("net_") || name.starts_with("http_") {
        return CapabilitySet::NET;
    }
    if name == "exit" || name.starts_with("command_") {
        return CapabilitySet::EXEC;
    }
    if name == "args" || name == "cwd" || name.starts_with("args_") || name.starts_with("env_") {
        return CapabilitySet::ENV;
    }
    if matches!(
        name,
        "print"
            | "input"
            | "read_line"
            | "flush"
            | "write_str"
            | "write_int"
            | "write_uint"
            | "write_float"
            | "write_bool"
            | "write_char"
            | "write_nl"
    ) || name.starts_with("print_")
        || name.starts_with("eprint")
        || name.starts_with("ewrite")
    {
        return CapabilitySet::IO;
    }
    if matches!(name, "dlopen" | "dlsym" | "dlclose" | "dlerror") {
        return CapabilitySet::FFI;
    }
    if matches!(name, "alloc" | "free" | "c_malloc" | "c_free")
        || name.starts_with("c_load_")
        || name.starts_with("c_store_")
    {
        return CapabilitySet::UNSAFE;
    }
    if name.starts_with("thread_") {
        return CapabilitySet::SPAWN;
    }
    if name == "sleep"
        || name == "sleep_ms"
        || name.starts_with("time_")
        || name.starts_with("clock_")
    {
        return CapabilitySet::TIME;
    }
    if name == "random" || name.starts_with("random_") || name.starts_with("rng_") {
        return CapabilitySet::RAND;
    }
    if name.starts_with("gui_") || name.starts_with("webview_") {
        return CapabilitySet::GUI;
    }
    if name.starts_with("sys_") || name.starts_with("cpu_") {
        return CapabilitySet::SYS;
    }
    CapabilitySet::EMPTY
}

impl<'hir> TypeChecker<'hir> {
    /// Enforce `@pure` and `@deny(...)` capability annotations across all
    /// functions using transitive call-graph propagation.
    pub(in crate::check) fn validate_package_capabilities(
        &mut self,
    ) -> Result<(), ShadowTypeError> {
        let mut denied_by_item: BTreeMap<HirItemId, CapabilitySet> = BTreeMap::new();
        let mut extern_functions: BTreeSet<HirItemId> = BTreeSet::new();

        for (item_id, item) in self.package.items.enumerate() {
            match item.kind {
                HirItemKind::Extern(ext) => {
                    for &func_id in self.package.items(ext.functions) {
                        extern_functions.insert(func_id);
                    }
                }
                HirItemKind::Function(_) => {
                    let denied = self.parse_denied_capabilities(*item)?;
                    if !denied.is_empty() {
                        denied_by_item.insert(item_id, denied);
                    }
                }
                _ => {}
            }
        }

        if denied_by_item.is_empty() {
            return Ok(());
        }

        // 1. Collect direct effects and static call edges for every function.
        let mut effects: BTreeMap<HirItemId, CapabilitySet> = BTreeMap::new();
        let mut call_edges: BTreeMap<HirItemId, BTreeSet<HirItemId>> = BTreeMap::new();

        for (item_id, item) in self.package.items.enumerate() {
            let HirItemKind::Function(function) = item.kind else {
                continue;
            };
            let mut direct = CapabilitySet::EMPTY;
            if extern_functions.contains(&item_id) {
                direct = direct.union(CapabilitySet::FFI);
            }
            if function.is_unsafe {
                direct = direct.union(CapabilitySet::UNSAFE);
            }
            let mut callees = BTreeSet::new();
            if let Some(body) = function.body {
                self.collect_body_capabilities(body, &extern_functions, &mut direct, &mut callees);
            }
            effects.insert(item_id, direct);
            call_edges.insert(item_id, callees);
        }

        // 2. Fixed-point transitive propagation over the static call graph.
        let mut changed = true;
        while changed {
            changed = false;
            for (&caller, callees) in &call_edges {
                let current = effects.get(&caller).copied().unwrap_or_default();
                let mut updated = current;
                for callee in callees {
                    if let Some(&callee_effects) = effects.get(callee) {
                        updated = updated.union(callee_effects);
                    }
                }
                if updated != current {
                    effects.insert(caller, updated);
                    changed = true;
                }
            }
        }

        // 3. Validate every annotated function and pinpoint the offending site.
        for (&item_id, &denied) in &denied_by_item {
            let item = self.package.items[item_id];
            let HirItemKind::Function(function) = item.kind else {
                continue;
            };
            if function.is_unsafe && !denied.intersect(CapabilitySet::UNSAFE).is_empty() {
                return Err(self.at(
                    item.origin,
                    TypeError::CapabilityDenied {
                        capability: "unsafe",
                        detail: "function is declared `unsafe def`".to_owned(),
                    },
                ));
            }
            let Some(body) = function.body else {
                continue;
            };
            let total = effects.get(&item_id).copied().unwrap_or_default();
            if total.intersect(denied).is_empty() {
                continue;
            }
            self.check_body_capabilities(body, denied, &extern_functions, &effects)?;
        }

        Ok(())
    }

    fn parse_denied_capabilities(&self, item: HirItem) -> Result<CapabilitySet, ShadowTypeError> {
        let mut denied = CapabilitySet::EMPTY;
        for &symbol in self.package.symbols(item.attributes) {
            let Some(text) = self.package.names.symbols.resolve(symbol) else {
                continue;
            };
            if text == "pure" {
                denied = denied.union(CapabilitySet::ALL);
            } else if text.starts_with("pure(") {
                return Err(self.at(
                    item.origin,
                    TypeError::InvalidCapabilityAttribute {
                        attribute: text.to_owned(),
                    },
                ));
            } else if text == "deny" {
                return Err(self.at(
                    item.origin,
                    TypeError::InvalidCapabilityAttribute {
                        attribute: text.to_owned(),
                    },
                ));
            } else if let Some(inner) = text.strip_prefix("deny(").and_then(|s| s.strip_suffix(')'))
            {
                let mut parsed_any = false;
                for raw in inner.split(',') {
                    let cap_name = raw.trim();
                    if cap_name.is_empty() {
                        continue;
                    }
                    let Some(cap) = CapabilitySet::from_name(cap_name) else {
                        return Err(self.at(
                            item.origin,
                            TypeError::InvalidCapabilityAttribute {
                                attribute: text.to_owned(),
                            },
                        ));
                    };
                    denied = denied.union(cap);
                    parsed_any = true;
                }
                if !parsed_any {
                    return Err(self.at(
                        item.origin,
                        TypeError::InvalidCapabilityAttribute {
                            attribute: text.to_owned(),
                        },
                    ));
                }
            }
        }
        Ok(denied)
    }

    fn resolve_callee_item(&self, callee: ExprId) -> Option<HirItemId> {
        let def = self.package.expression_definition(callee)?;
        self.definition_items.get(def.index()).copied().flatten()
    }

    fn collect_body_capabilities(
        &mut self,
        body: BodyId,
        extern_functions: &BTreeSet<HirItemId>,
        direct: &mut CapabilitySet,
        callees: &mut BTreeSet<HirItemId>,
    ) {
        let statements = self
            .package
            .statements(self.package.bodies[body].statements)
            .to_vec();
        for stmt_id in statements {
            self.collect_statement_capabilities(stmt_id, extern_functions, direct, callees);
        }
    }

    fn collect_statement_capabilities(
        &mut self,
        stmt_id: StmtId,
        extern_functions: &BTreeSet<HirItemId>,
        direct: &mut CapabilitySet,
        callees: &mut BTreeSet<HirItemId>,
    ) {
        match self.package.statements[stmt_id].kind {
            StatementKind::Let { value, .. } => {
                self.collect_expression_capabilities(value, extern_functions, direct, callees);
            }
            StatementKind::Assign { target, value } => {
                self.collect_expression_capabilities(target, extern_functions, direct, callees);
                self.collect_expression_capabilities(value, extern_functions, direct, callees);
            }
            StatementKind::Expression(expr) => {
                self.collect_expression_capabilities(expr, extern_functions, direct, callees);
            }
            StatementKind::Return(Some(expr)) => {
                self.collect_expression_capabilities(expr, extern_functions, direct, callees);
            }
            StatementKind::Return(None) | StatementKind::Break | StatementKind::Continue => {}
            StatementKind::If {
                condition,
                then_body,
                else_body,
            } => {
                self.collect_expression_capabilities(condition, extern_functions, direct, callees);
                self.collect_body_capabilities(then_body, extern_functions, direct, callees);
                if let Some(else_body) = else_body {
                    self.collect_body_capabilities(else_body, extern_functions, direct, callees);
                }
            }
            StatementKind::While { condition, body } => {
                self.collect_expression_capabilities(condition, extern_functions, direct, callees);
                self.collect_body_capabilities(body, extern_functions, direct, callees);
            }
            StatementKind::Unsafe { body } => {
                *direct = direct.union(CapabilitySet::UNSAFE);
                self.collect_body_capabilities(body, extern_functions, direct, callees);
            }
            StatementKind::For { iterable, body, .. } => {
                self.collect_expression_capabilities(iterable, extern_functions, direct, callees);
                self.collect_body_capabilities(body, extern_functions, direct, callees);
            }
            StatementKind::Match { subject, arms } => {
                self.collect_expression_capabilities(subject, extern_functions, direct, callees);
                for arm in self.package.match_arms(arms).to_vec() {
                    let body = self.package.match_arms[arm].body;
                    self.collect_body_capabilities(body, extern_functions, direct, callees);
                }
            }
        }
    }

    fn collect_expression_capabilities(
        &mut self,
        expr_id: ExprId,
        extern_functions: &BTreeSet<HirItemId>,
        direct: &mut CapabilitySet,
        callees: &mut BTreeSet<HirItemId>,
    ) {
        match self.package.expressions[expr_id].kind {
            ExpressionKind::Unary { operand, .. }
            | ExpressionKind::Try(operand)
            | ExpressionKind::Await(operand) => {
                self.collect_expression_capabilities(operand, extern_functions, direct, callees);
            }
            ExpressionKind::Spawn(inner) => {
                *direct = direct.union(CapabilitySet::SPAWN);
                self.collect_expression_capabilities(inner, extern_functions, direct, callees);
            }
            ExpressionKind::Binary { left, right, .. } => {
                self.collect_expression_capabilities(left, extern_functions, direct, callees);
                self.collect_expression_capabilities(right, extern_functions, direct, callees);
            }
            ExpressionKind::Tuple(elements) | ExpressionKind::List(elements) => {
                for element in self.package.expressions(elements).to_vec() {
                    self.collect_expression_capabilities(
                        element,
                        extern_functions,
                        direct,
                        callees,
                    );
                }
            }
            ExpressionKind::Call { callee, arguments }
            | ExpressionKind::GenericCall {
                callee, arguments, ..
            } => {
                if let Some(name) = self.expression_name(callee) {
                    *direct = direct.union(builtin_capabilities(name));
                    if Self::unsafe_intrinsic_label(name).is_some() {
                        *direct = direct.union(CapabilitySet::UNSAFE);
                    }
                }
                if let Some(callee_item) = self.resolve_callee_item(callee) {
                    if extern_functions.contains(&callee_item) {
                        *direct = direct.union(CapabilitySet::FFI);
                    } else {
                        callees.insert(callee_item);
                    }
                }
                self.collect_expression_capabilities(callee, extern_functions, direct, callees);
                for arg in self.package.expressions(arguments).to_vec() {
                    self.collect_expression_capabilities(arg, extern_functions, direct, callees);
                }
            }
            ExpressionKind::Field { base, .. } => {
                self.collect_expression_capabilities(base, extern_functions, direct, callees);
            }
            ExpressionKind::Index { base, index } => {
                self.collect_expression_capabilities(base, extern_functions, direct, callees);
                self.collect_expression_capabilities(index, extern_functions, direct, callees);
            }
            ExpressionKind::Cast { value, .. } => {
                self.collect_expression_capabilities(value, extern_functions, direct, callees);
                if let Some(target_ty) = self.assignments.expression(expr_id)
                    && self.type_is_raw_pointer(target_ty)
                {
                    *direct = direct.union(CapabilitySet::UNSAFE);
                }
            }
            ExpressionKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.collect_expression_capabilities(condition, extern_functions, direct, callees);
                self.collect_expression_capabilities(then_expr, extern_functions, direct, callees);
                self.collect_expression_capabilities(else_expr, extern_functions, direct, callees);
            }
            ExpressionKind::Closure { body, .. } => {
                self.collect_body_capabilities(body, extern_functions, direct, callees);
            }
            ExpressionKind::Literal(_) | ExpressionKind::Name { .. } => {}
        }
    }

    fn check_body_capabilities(
        &mut self,
        body: BodyId,
        denied: CapabilitySet,
        extern_functions: &BTreeSet<HirItemId>,
        effects: &BTreeMap<HirItemId, CapabilitySet>,
    ) -> Result<(), ShadowTypeError> {
        let statements = self
            .package
            .statements(self.package.bodies[body].statements)
            .to_vec();
        for stmt_id in statements {
            self.check_statement_capabilities(stmt_id, denied, extern_functions, effects)?;
        }
        Ok(())
    }

    fn check_statement_capabilities(
        &mut self,
        stmt_id: StmtId,
        denied: CapabilitySet,
        extern_functions: &BTreeSet<HirItemId>,
        effects: &BTreeMap<HirItemId, CapabilitySet>,
    ) -> Result<(), ShadowTypeError> {
        let statement = self.package.statements[stmt_id];
        match statement.kind {
            StatementKind::Let { value, .. } => {
                self.check_expression_capabilities(value, denied, extern_functions, effects)?;
            }
            StatementKind::Assign { target, value } => {
                self.check_expression_capabilities(target, denied, extern_functions, effects)?;
                self.check_expression_capabilities(value, denied, extern_functions, effects)?;
            }
            StatementKind::Expression(expr) => {
                self.check_expression_capabilities(expr, denied, extern_functions, effects)?;
            }
            StatementKind::Return(Some(expr)) => {
                self.check_expression_capabilities(expr, denied, extern_functions, effects)?;
            }
            StatementKind::Return(None) | StatementKind::Break | StatementKind::Continue => {}
            StatementKind::If {
                condition,
                then_body,
                else_body,
            } => {
                self.check_expression_capabilities(condition, denied, extern_functions, effects)?;
                self.check_body_capabilities(then_body, denied, extern_functions, effects)?;
                if let Some(else_body) = else_body {
                    self.check_body_capabilities(else_body, denied, extern_functions, effects)?;
                }
            }
            StatementKind::While { condition, body } => {
                self.check_expression_capabilities(condition, denied, extern_functions, effects)?;
                self.check_body_capabilities(body, denied, extern_functions, effects)?;
            }
            StatementKind::Unsafe { body } => {
                if !denied.intersect(CapabilitySet::UNSAFE).is_empty() {
                    return Err(self.at(
                        statement.origin,
                        TypeError::CapabilityDenied {
                            capability: "unsafe",
                            detail: "enters an `unsafe:` block".to_owned(),
                        },
                    ));
                }
                self.check_body_capabilities(body, denied, extern_functions, effects)?;
            }
            StatementKind::For { iterable, body, .. } => {
                self.check_expression_capabilities(iterable, denied, extern_functions, effects)?;
                self.check_body_capabilities(body, denied, extern_functions, effects)?;
            }
            StatementKind::Match { subject, arms } => {
                self.check_expression_capabilities(subject, denied, extern_functions, effects)?;
                for arm in self.package.match_arms(arms).to_vec() {
                    let body = self.package.match_arms[arm].body;
                    self.check_body_capabilities(body, denied, extern_functions, effects)?;
                }
            }
        }
        Ok(())
    }

    fn check_expression_capabilities(
        &mut self,
        expr_id: ExprId,
        denied: CapabilitySet,
        extern_functions: &BTreeSet<HirItemId>,
        effects: &BTreeMap<HirItemId, CapabilitySet>,
    ) -> Result<(), ShadowTypeError> {
        let expression = self.package.expressions[expr_id];
        match expression.kind {
            ExpressionKind::Unary { operand, .. }
            | ExpressionKind::Try(operand)
            | ExpressionKind::Await(operand) => {
                self.check_expression_capabilities(operand, denied, extern_functions, effects)?;
            }
            ExpressionKind::Spawn(inner) => {
                if !denied.intersect(CapabilitySet::SPAWN).is_empty() {
                    return Err(self.at(
                        expression.origin,
                        TypeError::CapabilityDenied {
                            capability: "spawn",
                            detail: "spawns an asynchronous task".to_owned(),
                        },
                    ));
                }
                self.check_expression_capabilities(inner, denied, extern_functions, effects)?;
            }
            ExpressionKind::Binary { left, right, .. } => {
                self.check_expression_capabilities(left, denied, extern_functions, effects)?;
                self.check_expression_capabilities(right, denied, extern_functions, effects)?;
            }
            ExpressionKind::Tuple(elements) | ExpressionKind::List(elements) => {
                for element in self.package.expressions(elements).to_vec() {
                    self.check_expression_capabilities(element, denied, extern_functions, effects)?;
                }
            }
            ExpressionKind::Call { callee, arguments }
            | ExpressionKind::GenericCall {
                callee, arguments, ..
            } => {
                if let Some(name) = self.expression_name(callee) {
                    let req = builtin_capabilities(name);
                    if let Some(cap) = req.intersect(denied).first_name() {
                        return Err(self.at(
                            expression.origin,
                            TypeError::CapabilityDenied {
                                capability: cap,
                                detail: format!("calls builtin `{name}`"),
                            },
                        ));
                    }
                    if Self::unsafe_intrinsic_label(name).is_some()
                        && !denied.intersect(CapabilitySet::UNSAFE).is_empty()
                    {
                        return Err(self.at(
                            expression.origin,
                            TypeError::CapabilityDenied {
                                capability: "unsafe",
                                detail: format!("calls unsafe intrinsic `{name}`"),
                            },
                        ));
                    }
                }
                if let Some(callee_item) = self.resolve_callee_item(callee) {
                    let callee_name = self.package.items[callee_item]
                        .name
                        .and_then(|s| self.package.names.symbols.resolve(s))
                        .unwrap_or("<anonymous>")
                        .to_owned();
                    if extern_functions.contains(&callee_item) {
                        if !denied.intersect(CapabilitySet::FFI).is_empty() {
                            return Err(self.at(
                                expression.origin,
                                TypeError::CapabilityDenied {
                                    capability: "ffi",
                                    detail: format!("calls extern function `{callee_name}`"),
                                },
                            ));
                        }
                    } else if let Some(&callee_effects) = effects.get(&callee_item)
                        && let Some(cap) = callee_effects.intersect(denied).first_name()
                    {
                        return Err(self.at(
                            expression.origin,
                            TypeError::CapabilityDenied {
                                capability: cap,
                                detail: format!(
                                    "calls `{callee_name}` which transitively requires `{cap}`"
                                ),
                            },
                        ));
                    }
                }
                self.check_expression_capabilities(callee, denied, extern_functions, effects)?;
                for arg in self.package.expressions(arguments).to_vec() {
                    self.check_expression_capabilities(arg, denied, extern_functions, effects)?;
                }
            }
            ExpressionKind::Field { base, .. } => {
                self.check_expression_capabilities(base, denied, extern_functions, effects)?;
            }
            ExpressionKind::Index { base, index } => {
                self.check_expression_capabilities(base, denied, extern_functions, effects)?;
                self.check_expression_capabilities(index, denied, extern_functions, effects)?;
            }
            ExpressionKind::Cast { value, .. } => {
                self.check_expression_capabilities(value, denied, extern_functions, effects)?;
                if !denied.intersect(CapabilitySet::UNSAFE).is_empty()
                    && let Some(target_ty) = self.assignments.expression(expr_id)
                    && self.type_is_raw_pointer(target_ty)
                {
                    return Err(self.at(
                        expression.origin,
                        TypeError::CapabilityDenied {
                            capability: "unsafe",
                            detail: "casts to a raw pointer".to_owned(),
                        },
                    ));
                }
            }
            ExpressionKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.check_expression_capabilities(condition, denied, extern_functions, effects)?;
                self.check_expression_capabilities(then_expr, denied, extern_functions, effects)?;
                self.check_expression_capabilities(else_expr, denied, extern_functions, effects)?;
            }
            ExpressionKind::Closure { body, .. } => {
                self.check_body_capabilities(body, denied, extern_functions, effects)?;
            }
            ExpressionKind::Literal(_) | ExpressionKind::Name { .. } => {}
        }
        Ok(())
    }
}
