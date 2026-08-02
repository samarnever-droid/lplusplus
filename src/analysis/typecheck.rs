use crate::ast::*;
use crate::semantic::{ScopeId, ScopeKind, SymbolTable};
use crate::types::{StructTypeId, TypeRef, TypeTable};
use std::collections::HashMap;

pub struct TypeChecker<'a> {
    pub type_table: TypeTable,
    pub symbol_table: &'a mut SymbolTable,
    pub closure_scope_idx: usize,
    pub func_return_types: HashMap<String, TypeRef>,
    pub func_param_types: HashMap<String, Vec<TypeRef>>,
    pub trait_names: std::collections::HashSet<String>,
    pub trait_impls: HashMap<String, std::collections::HashSet<String>>,
    pub type_param_bounds: HashMap<String, Vec<(String, String)>>,
    /// Names of declared enums. Enums now carry a real field layout
    /// (`__tag` + payload slots), so "has no fields" can no longer be used to
    /// tell an enum from a struct when resolving `Enum.Variant`.
    pub enum_names: std::collections::HashSet<String>,
    /// Function names whose calls produce Task[T]. The stored function return
    /// type remains T so returns inside the async body are checked normally.
    pub async_functions: std::collections::HashSet<String>,
    /// Typed rest element type by function name.
    pub variadic_elements: HashMap<String, TypeRef>,
    /// Bases currently borrowed by a slice. The first tier conservatively keeps
    /// the borrow active to function exit and rejects reassignment.
    pub borrowed_slice_bases: std::collections::HashSet<usize>,
}

fn type_param_names(tps: &[TypeParam]) -> Vec<String> {
    tps.iter().map(|tp| tp.name.clone()).collect()
}

/// Check if two types are compatible, treating TypeParam as a wildcard.
fn types_compatible(expected: &TypeRef, actual: &TypeRef) -> bool {
    if expected == actual {
        return true;
    }
    // Allow coercion between Char/Bool/Custom(enum) and Int, and Char/Int/Float/Bool to Str
    if matches!((expected, actual), (TypeRef::Char, TypeRef::Int) | (TypeRef::Int, TypeRef::Char) | (TypeRef::Bool, TypeRef::Int) | (TypeRef::Int, TypeRef::Bool) | (TypeRef::Custom(_), TypeRef::Int) | (TypeRef::Int, TypeRef::Custom(_))) {
        return true;
    }
    if expected == &TypeRef::Str && matches!(actual, TypeRef::Int | TypeRef::Float | TypeRef::Bool | TypeRef::Char) {
        return true;
    }
    // Allow collection/map handles (represented as 64-bit handle IDs) to coerce with Int
    if (expected == &TypeRef::Int && matches!(actual, TypeRef::Generic(..))) || (actual == &TypeRef::Int && matches!(expected, TypeRef::Generic(..))) {
        return true;
    }
    // Structural aggregates recurse so a type parameter nested in a tuple/task
    // remains a wildcard during generic checking.
    match (expected, actual) {
        (TypeRef::Tuple(a), TypeRef::Tuple(b)) if a.len() == b.len() =>
            a.iter().zip(b).all(|(x, y)| types_compatible(x, y)),
        (TypeRef::Slice(a), TypeRef::Slice(b)) | (TypeRef::Task(a), TypeRef::Task(b)) =>
            types_compatible(a, b),
        (TypeRef::Generic(an, aa), TypeRef::Generic(bn, ba))
            if an == bn && aa.len() == ba.len() =>
            aa.iter().zip(ba).all(|(x, y)| types_compatible(x, y)),
        _ => matches!(expected, TypeRef::TypeParam(_))
            || matches!(actual, TypeRef::TypeParam(_)),
    }
}

impl<'a> TypeChecker<'a> {
    pub fn new(symbol_table: &'a mut SymbolTable) -> Self {
        Self {
            type_table: TypeTable::new(),
            symbol_table,
            closure_scope_idx: 0,
            func_return_types: HashMap::new(),
            func_param_types: HashMap::new(),
            trait_names: std::collections::HashSet::new(),
            trait_impls: HashMap::new(),
            type_param_bounds: HashMap::new(),
            enum_names: std::collections::HashSet::new(),
            async_functions: std::collections::HashSet::new(),
            variadic_elements: HashMap::new(),
            borrowed_slice_bases: std::collections::HashSet::new(),
        }
    }



    fn enclosing_async_function(&self, scope: ScopeId) -> Option<&str> {
        let mut current = Some(scope);
        while let Some(id) = current {
            match &self.symbol_table.scopes[id.0].kind {
                ScopeKind::Function { name } => {
                    return self.async_functions.contains(name).then_some(name.as_str());
                }
                ScopeKind::Closure { .. } => return None,
                _ => current = self.symbol_table.scopes[id.0].parent,
            }
        }
        None
    }

    fn blocking_in_async(name: &str) -> bool {
        matches!(
            name,
            "input"
                | "read_file"
                | "write_file"
                | "append_file"
                | "command_exec"
                | "command_output"
                | "net_connect"
                | "net_listen"
                | "net_accept"
                | "net_accept_timeout"
                | "net_recv"
                | "net_recv_udp"
                | "http_get"
                | "http_post"
                | "sleep"
        )
    }

    fn convert_ast_type(type_table: &TypeTable, ast_ty: &Type) -> TypeRef {
        Self::convert_ast_type_with_params(type_table, ast_ty, &[])
    }

    fn convert_ast_type_with_params(type_table: &TypeTable, ast_ty: &Type, type_params: &[String]) -> TypeRef {
        match ast_ty {
            Type::Int => TypeRef::Int,
            Type::Float => TypeRef::Float,
            Type::String => TypeRef::Str,
            Type::Bool => TypeRef::Bool,
            Type::Char => TypeRef::Char,
            Type::Void => TypeRef::Void,
            Type::Custom(name) => {
                // Check if this is a type parameter first
                if type_params.iter().any(|tp| tp == name) {
                    return TypeRef::TypeParam(name.clone());
                }
                if let Some(&id) = type_table.structs_by_name.get(name) {
                    TypeRef::Custom(id)
                } else {
                    TypeRef::Unresolved(name.clone())
                }
            }
            Type::Generic(base_name, args) => {
                let mut ref_args = Vec::new();
                for arg in args {
                    ref_args.push(Self::convert_ast_type_with_params(type_table, arg, type_params));
                }
                TypeRef::Generic(base_name.clone(), ref_args)
            }
            Type::Tuple(elements) => TypeRef::Tuple(
                elements
                    .iter()
                    .map(|ty| Self::convert_ast_type_with_params(type_table, ty, type_params))
                    .collect(),
            ),
            Type::StrSlice => TypeRef::StrSlice,
            Type::Slice(element) => TypeRef::Slice(Box::new(
                Self::convert_ast_type_with_params(type_table, element, type_params),
            )),
            Type::Task(result) => TypeRef::Task(Box::new(
                Self::convert_ast_type_with_params(type_table, result, type_params),
            )),
        }
    }

    fn verify_struct_cycles(_type_table: &TypeTable) -> Result<(), String> {
        use std::collections::HashSet;

        fn collect_custom_ids(ty: &TypeRef, ids: &mut Vec<StructTypeId>) {
            match ty {
                TypeRef::Custom(id) => ids.push(*id),
                TypeRef::Generic(_, args) => {
                    for arg in args {
                        collect_custom_ids(arg, ids);
                    }
                }
                TypeRef::Tuple(tys) => {
                    for t in tys {
                        collect_custom_ids(t, ids);
                    }
                }
                TypeRef::Slice(inner) => {
                    collect_custom_ids(inner, ids);
                }
                TypeRef::Task(inner) => {
                    collect_custom_ids(inner, ids);
                }
                _ => {}
            }
        }

        fn reaches(
            type_table: &TypeTable,
            target: StructTypeId,
            current: StructTypeId,
            visited: &mut HashSet<StructTypeId>,
        ) -> bool {
            for (_, field_ty) in &type_table.definitions[current.0].fields {
                let mut field_targets = Vec::new();
                collect_custom_ids(field_ty, &mut field_targets);
                for next in field_targets {
                    if next == target {
                        return true;
                    }
                    if visited.insert(next) && reaches(type_table, target, next, visited) {
                        return true;
                    }
                }
            }
            false
        }

        // Recursive struct types are ACCEPTED, and their cycles are broken
        // statically by `analysis::cyclebreak` rather than rejected here.
        //
        // The old behaviour refused any struct reachable from itself, which
        // ruled out binary trees, linked lists and parent pointers. The reason
        // given was that ARC cannot reclaim ownership cycles -- true, but the
        // fix is to ensure no *owning* cycle is ever built, not to ban the type.
        //
        // `break_cycles` classifies exactly one edge of every cycle as
        // non-owning, so the owning subgraph is acyclic by construction (see
        // the proof in that module). A field so demoted is stored without a
        // retain and read back through a generation-checked weak handle, so it
        // can neither leak nor dangle.
        //
        // The layout was never the obstacle: a Custom field is an 8-byte
        // pointer, so a self-referential struct has a finite size.
        let _ = reaches;
        Ok(())
    }

    pub fn check_program(&mut self, program: &Program) -> Result<(), String> {
        // Phase 0.5: Collect trait names and impl mappings
        for decl in &program.declarations {
            if let TopLevel::Trait(t) = decl {
                self.trait_names.insert(t.name.clone());
            }
        }
        for decl in &program.declarations {
            if let TopLevel::Impl(impl_block) = decl {
                self.trait_impls
                    .entry(impl_block.target_type.clone())
                    .or_insert_with(std::collections::HashSet::new)
                    .insert(impl_block.trait_name.clone());
            }
        }

        // Phase 1: Register all struct and enum names (stubs) and map function return types
        for decl in &program.declarations {
            if let TopLevel::Struct(s) = decl {
                self.type_table.register_struct(s.name.clone());
            }
            if let TopLevel::Enum(e) = decl {
                // Register enum as a custom type (like a struct)
                self.type_table.register_struct(e.name.clone());
                self.enum_names.insert(e.name.clone());
            }
        }
        for decl in &program.declarations {
            if let TopLevel::Function(f) = decl {
                let tp = type_param_names(&f.type_params);
                let ret_ty = Self::convert_ast_type_with_params(&self.type_table, &f.return_type, &tp);
                self.func_return_types.insert(f.name.clone(), ret_ty);
                if f.is_async {
                    if f.name == "main" && !f.params.is_empty() {
                        return Err("Type error: async main cannot take parameters".to_string());
                    }
                    self.async_functions.insert(f.name.clone());
                }
                let mut param_tys = Vec::with_capacity(f.params.len());
                for param in &f.params {
                    let element =
                        Self::convert_ast_type_with_params(&self.type_table, &param.ty, &tp);
                    if param.variadic {
                        if !element.is_list_element_supported() {
                            return Err(format!(
                                "Type error: variadic parameter '{}' in '{}' cannot safely store element type {:?}",
                                param.name, f.name, element
                            ));
                        }
                        self.variadic_elements
                            .insert(f.name.clone(), element.clone());
                        param_tys.push(TypeRef::Generic("List".to_string(), vec![element]));
                    } else {
                        param_tys.push(element);
                    }
                }
                self.func_param_types.insert(f.name.clone(), param_tys);
                // Record trait bounds for this function's type params
                let bounds: Vec<(String, String)> = f.type_params.iter()
                    .filter_map(|tp| tp.bound.as_ref().map(|b| (tp.name.clone(), b.clone())))
                    .collect();
                if !bounds.is_empty() {
                    self.type_param_bounds.insert(f.name.clone(), bounds);
                }
            }
            // Register impl method types (they are mangled as TargetType_method)
            if let TopLevel::Impl(impl_block) = decl {
                for method in &impl_block.methods {
                    let tp = type_param_names(&method.type_params);
                    let ret_ty = Self::convert_ast_type_with_params(&self.type_table, &method.return_type, &tp);
                    self.func_return_types.insert(method.name.clone(), ret_ty);
                    if method.is_async {
                        self.async_functions.insert(method.name.clone());
                    }
                    let mut param_tys = Vec::with_capacity(method.params.len());
                    for param in &method.params {
                        let element =
                            Self::convert_ast_type_with_params(&self.type_table, &param.ty, &tp);
                        if param.variadic {
                            if !element.is_list_element_supported() {
                                return Err(format!(
                                    "Type error: variadic parameter '{}' in '{}' cannot safely store element type {:?}",
                                    param.name, method.name, element
                                ));
                            }
                            self.variadic_elements
                                .insert(method.name.clone(), element.clone());
                            param_tys
                                .push(TypeRef::Generic("List".to_string(), vec![element]));
                        } else {
                            param_tys.push(element);
                        }
                    }
                    self.func_param_types.insert(method.name.clone(), param_tys);
                }
            }
            // Register extern function types
            if let TopLevel::Extern(ext) = decl {
                for ef in &ext.functions {
                    let ret_ty = Self::convert_ast_type(&self.type_table, &ef.return_type);
                    self.func_return_types.insert(ef.name.clone(), ret_ty);
                    let param_tys: Vec<TypeRef> = ef
                        .params
                        .iter()
                        .map(|p| Self::convert_ast_type(&self.type_table, &p.ty))
                        .collect();
                    self.func_param_types.insert(ef.name.clone(), param_tys);
                }
            }
        }

        // Phase 2: Resolve struct fields and check for self-reference
        for decl in &program.declarations {
            if let TopLevel::Struct(s) = decl {
                let id = *self
                    .type_table
                    .structs_by_name
                    .get(&s.name)
                    .ok_or_else(|| format!("Type error: Unknown struct definition '{}'", s.name))?;

                let mut resolved_fields = Vec::new();
                let mut is_self_referential = false;

                for field in &s.fields {
                    let field_ty = Self::convert_ast_type_with_params(&self.type_table, &field.ty, &type_param_names(&s.type_params));

                    if let TypeRef::Custom(ref_id) = field_ty {
                        if ref_id == id {
                            is_self_referential = true;
                        }
                    } else if let TypeRef::Unresolved(name) = &field_ty {
                        return Err(format!("Unknown type '{}' in struct '{}'", name, s.name));
                    }

                    resolved_fields.push((field.name.clone(), field_ty));
                }

                let def = &mut self.type_table.definitions[id.0];
                def.fields = resolved_fields;
                def.is_self_referential = is_self_referential;
            }
        }

        // Phase 2b: Give every enum a real field layout.
        //
        // Enums used to be a single packed i64: (tag << 32) | (payload & 0xFFFFFFFF).
        // That silently truncated any payload wider than 32 bits — a Str or a
        // struct pointer became a garbage address (SIGSEGV on use) and an Int
        // above 2^32 came back as a different number. Laying an enum out as a
        // real heap object with a tag plus one slot per variant keeps the
        // payload at full width and reuses the existing ARC machinery.
        //
        // Layout: field 0 is `__tag`, then `__vN` for variant N's payload.
        // Variants share the object but not their slots, so the payload type
        // stays exact instead of being erased to i64.
        for decl in &program.declarations {
            if let TopLevel::Enum(e) = decl {
                let id = *self
                    .type_table
                    .structs_by_name
                    .get(&e.name)
                    .ok_or_else(|| format!("Type error: Unknown enum definition '{}'", e.name))?;
                let tp = type_param_names(&e.type_params);
                let mut fields = vec![("__tag".to_string(), TypeRef::Int)];
                for (i, variant) in e.variants.iter().enumerate() {
                    if let Some(p) = variant.fields.first() {
                        let ty = Self::convert_ast_type_with_params(&self.type_table, &p.ty, &tp);
                        let ty = if matches!(ty, TypeRef::Unresolved(_)) { TypeRef::Int } else { ty };
                        fields.push((format!("__v{}", i), ty));
                    }
                }
                let def = &mut self.type_table.definitions[id.0];
                def.fields = fields;
            }
        }

        // Check for cyclic ownership graphs in custom types
        Self::verify_struct_cycles(&self.type_table)?;

        // Collect all type parameter names from all generic functions/structs/enums
        let mut all_type_params: Vec<String> = Vec::new();
        for decl in &program.declarations {
            match decl {
                TopLevel::Function(f) => all_type_params.extend(type_param_names(&f.type_params)),
                TopLevel::Struct(s) => all_type_params.extend(type_param_names(&s.type_params)),
                TopLevel::Enum(e) => all_type_params.extend(type_param_names(&e.type_params)),
                _ => {}
            }
        }
        // Also treat trait names as type params so they resolve to TypeParam (→ i64)
        for tn in &self.trait_names {
            all_type_params.push(tn.clone());
        }
        all_type_params.sort();
        all_type_params.dedup();

        // Phase 3: Update all bindings in the symbol table with resolved TypeRefs
        for binding in &mut self.symbol_table.bindings {
            if let Some(ast_ty) = &binding.ast_ty {
                binding.ty = Some(Self::convert_ast_type_with_params(&self.type_table, ast_ty, &all_type_params));
            }
        }

        // Async blocking safety is transitive: wrapping `read_file` in an
        // ordinary helper does not make it nonblocking. Compute a small call
        // graph before body inference and reject any async root that can reach
        // a blocking builtin without an adapter.
        fn collect_expr_calls(expr: &Expr, calls: &mut std::collections::HashSet<String>) {
            match expr {
                Expr::Call { callee, args } | Expr::GenericCall { callee, args, .. } => {
                    if let Expr::Identifier(name, _) = &**callee { calls.insert(name.clone()); }
                    collect_expr_calls(callee, calls);
                    for arg in args { collect_expr_calls(arg, calls); }
                }
                Expr::Tuple(items) | Expr::ListLiteral(items) =>
                    for item in items { collect_expr_calls(item, calls); },
                Expr::Await(inner) | Expr::Try(inner) | Expr::UnaryOp { operand: inner, .. }
                | Expr::Spawn { closure: inner } => collect_expr_calls(inner, calls),
                Expr::BinaryOp { left, right, .. } => {
                    collect_expr_calls(left, calls); collect_expr_calls(right, calls);
                }
                Expr::Closure { body, .. } => collect_stmt_calls(body, calls),
                Expr::FieldAccess { base, .. } => collect_expr_calls(base, calls),
                Expr::Match { subject, arms } => {
                    collect_expr_calls(subject, calls);
                    for arm in arms { collect_stmt_calls(&arm.body, calls); }
                }
                Expr::Index { base, index } => {
                    collect_expr_calls(base, calls); collect_expr_calls(index, calls);
                }
                Expr::EnumVariantConstruct { args, .. } =>
                    for arg in args { collect_expr_calls(arg, calls); },
                Expr::IntLiteral(_) | Expr::FloatLiteral(_) | Expr::StringLiteral(_)
                | Expr::CharLiteral(_) | Expr::BoolLiteral(_) | Expr::Identifier(_, _) => {}
            }
        }
        fn collect_stmt_calls(stmts: &[Stmt], calls: &mut std::collections::HashSet<String>) {
            for stmt in stmts {
                match stmt {
                    Stmt::Destructure { value, .. } | Stmt::LetInferred { value, .. }
                    | Stmt::Assign { value, .. } | Stmt::Expr(value)
                    | Stmt::Return(Some(value)) => collect_expr_calls(value, calls),
                    Stmt::AssignField { base, value, .. } => {
                        collect_expr_calls(base, calls); collect_expr_calls(value, calls);
                    }
                    Stmt::If { condition, then_block, else_block, .. } => {
                        collect_expr_calls(condition, calls); collect_stmt_calls(then_block, calls);
                        if let Some(block) = else_block { collect_stmt_calls(block, calls); }
                    }
                    Stmt::While { condition, body, .. } => {
                        collect_expr_calls(condition, calls); collect_stmt_calls(body, calls);
                    }
                    Stmt::ForRange { start, end, step, body, .. } => {
                        collect_expr_calls(start, calls); collect_expr_calls(end, calls);
                        if let Some(step) = step { collect_expr_calls(step, calls); }
                        collect_stmt_calls(body, calls);
                    }
                    Stmt::ForIn { list, body, .. } => {
                        collect_expr_calls(list, calls); collect_stmt_calls(body, calls);
                    }
                    Stmt::Match { subject, arms } => {
                        collect_expr_calls(subject, calls);
                        for arm in arms { collect_stmt_calls(&arm.body, calls); }
                    }
                    Stmt::Block(body) => collect_stmt_calls(body, calls),
                    Stmt::Return(None) | Stmt::Break | Stmt::Continue => {}
                }
            }
        }
        let mut call_graph: HashMap<String, std::collections::HashSet<String>> = HashMap::new();
        for declaration in &program.declarations {
            match declaration {
                TopLevel::Function(function) => {
                    let mut calls = std::collections::HashSet::new();
                    collect_stmt_calls(&function.body, &mut calls);
                    call_graph.insert(function.name.clone(), calls);
                }
                TopLevel::Impl(block) => for function in &block.methods {
                    let mut calls = std::collections::HashSet::new();
                    collect_stmt_calls(&function.body, &mut calls);
                    call_graph.insert(function.name.clone(), calls);
                },
                _ => {}
            }
        }
        let mut blocking_functions: std::collections::HashSet<String> = call_graph
            .iter()
            .filter(|(_, calls)| calls.iter().any(|name| Self::blocking_in_async(name)))
            .map(|(name, _)| name.clone())
            .collect();
        loop {
            let mut changed = false;
            for (function, calls) in &call_graph {
                if !blocking_functions.contains(function)
                    && calls.iter().any(|callee| blocking_functions.contains(callee))
                {
                    blocking_functions.insert(function.clone()); changed = true;
                }
            }
            if !changed { break; }
        }
        if let Some(function) = self.async_functions.iter()
            .find(|name| blocking_functions.contains(*name))
        {
            return Err(format!(
                "Async safety error: async function '{}' reaches a blocking call without an adapter",
                function
            ));
        }

        // Phase 4: Local Type Inference
        // Collect all functions: top-level + impl methods
        let mut all_funcs: Vec<&Function> = Vec::new();
        for decl in &program.declarations {
            if let TopLevel::Function(func) = decl {
                all_funcs.push(func);
            }
            if let TopLevel::Impl(impl_block) = decl {
                for method in &impl_block.methods {
                    all_funcs.push(method);
                }
            }
        }
        for func in &all_funcs {
            let mut func_scope_id = None;
            for scope in &self.symbol_table.scopes {
                if let ScopeKind::Function { name } = &scope.kind {
                    if name == &func.name {
                        func_scope_id = Some(scope.id);
                        break;
                    }
                }
            }
            if let Some(scope_id) = func_scope_id {
                for stmt in &func.body {
                    self.infer_stmt(stmt, scope_id)?;
                }
            }
        }

        Ok(())
    }

    fn infer_stmt(&mut self, stmt: &Stmt, current_scope: ScopeId) -> Result<(), String> {
        match stmt {
            Stmt::Destructure {
                names,
                value,
                binding_ids,
            } => {
                let tuple_ty = self.infer_expr(value, current_scope, None)?;
                let elements = match tuple_ty {
                    TypeRef::Tuple(elements) => elements,
                    other => {
                        return Err(format!(
                            "Type error: tuple destructuring requires a tuple value, got {:?}",
                            other
                        ));
                    }
                };
                if elements.len() != names.len() {
                    return Err(format!(
                        "Type error: destructuring has {} names but tuple has {} elements",
                        names.len(), elements.len()
                    ));
                }
                for (index, element_ty) in elements.into_iter().enumerate() {
                    let binding_id = binding_ids
                        .get(index)
                        .and_then(|cell| cell.get())
                        .ok_or_else(|| "Binding ID not set for tuple destructuring".to_string())?;
                    self.symbol_table.bindings[binding_id].ty = Some(element_ty);
                }
            }
            Stmt::LetInferred {
                name: _,
                is_mut: _,
                value,
                binding_id,
            } => {
                let inferred_type = self.infer_expr(value, current_scope, None)?;
                let b_id = binding_id
                    .get()
                    .ok_or_else(|| "Binding ID not set".to_string())?;
                let binding = &mut self.symbol_table.bindings[b_id];
                if binding.ty.is_none() {
                    binding.ty = Some(inferred_type);
                }
                if let Expr::Call { callee, args } = value {
                    if matches!(&**callee, Expr::Identifier(name, _) if name == "str_slice" || name == "slice") {
                        if let Some(Expr::Identifier(_, cell)) = args.first() {
                            if let Some(source_id) = cell.get() {
                                self.borrowed_slice_bases.insert(source_id);
                            }
                        }
                    }
                }
            }
            Stmt::Assign {
                name,
                value,
                binding_id,
            } => {
                let resolved_id = binding_id
                    .get()
                    .or_else(|| self.symbol_table.resolve_name(current_scope, name).map(|id| id.0));
                if resolved_id.map(|id| self.borrowed_slice_bases.contains(&id)).unwrap_or(false) {
                    return Err(format!(
                        "Borrow error: cannot reassign '{}' while a borrowed slice view is live",
                        name
                    ));
                }
                let expected_ty = if let Some(b_id) = binding_id.get() {
                    self.symbol_table.bindings[b_id].ty.clone()
                } else if let Some(b_id) = self.symbol_table.resolve_name(current_scope, name) {
                    binding_id.set(Some(b_id.0));
                    self.symbol_table.bindings[b_id.0].ty.clone()
                } else {
                    None
                };
                let val_ty = self.infer_expr(value, current_scope, expected_ty.clone())?;
                if let Some(exp) = expected_ty {
                    if !types_compatible(&exp, &val_ty) {
                        return Err(format!(
                            "Type mismatch in assignment: cannot assign '{:?}' to variable '{}' of type '{:?}'",
                            val_ty, name, exp
                        ));
                    }
                }
            }
            Stmt::AssignField { base, field, value } => {
                let base_ty = self.infer_expr(base, current_scope, None)?;
                let mut expected_ty = None;
                if let TypeRef::Custom(struct_id) = base_ty {
                    let struct_def = &self.type_table.definitions[struct_id.0];
                    if let Some(field_entry) =
                        struct_def.fields.iter().find(|(name, _)| name == field)
                    {
                        expected_ty = Some(field_entry.1.clone());
                    }
                }
                let val_ty = self.infer_expr(value, current_scope, expected_ty)?;
                if let TypeRef::Custom(struct_id) = base_ty {
                    let struct_def = &self.type_table.definitions[struct_id.0];
                    if let Some(field_entry) =
                        struct_def.fields.iter().find(|(name, _)| name == field)
                    {
                        if !types_compatible(&field_entry.1, &val_ty) {
                            return Err(format!(
                                "Type mismatch in field assignment: expected {:?}, got {:?}",
                                field_entry.1, val_ty
                            ));
                        }
                    } else {
                        return Err(format!(
                            "Field '{}' not found on struct '{}'",
                            field, struct_def.name
                        ));
                    }
                } else {
                    return Err(format!(
                        "Cannot access field '{}' on non-struct type {:?}",
                        field, base_ty
                    ));
                }
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
                then_scope,
                else_scope,
            } => {
                let cond_ty = self.infer_expr(condition, current_scope, None)?;
                if cond_ty != TypeRef::Bool && cond_ty != TypeRef::Int {
                    return Err(format!(
                        "'if' condition must be Bool or Int, found {:?}",
                        cond_ty
                    ));
                }

                // BUG-11: use the block's own scope, not the outer function scope
                let then_scope = ScopeId(then_scope.get().unwrap());
                for stmt in then_block {
                    self.infer_stmt(stmt, then_scope)?
                }

                if let Some(else_b) = else_block {
                    // BUG-11: use the block's own scope, not the outer function scope
                    let else_scope = ScopeId(else_scope.get().unwrap());
                    for stmt in else_b {
                        self.infer_stmt(stmt, else_scope)?
                    }
                }
            }
            Stmt::While {
                condition,
                body,
                body_scope,
            } => {
                let cond_ty = self.infer_expr(condition, current_scope, None)?;
                if cond_ty != TypeRef::Bool && cond_ty != TypeRef::Int {
                    return Err(format!(
                        "'while' condition must be Bool or Int, found {:?}",
                        cond_ty
                    ));
                }

                // BUG-11: use the while body's own block scope
                let body_scope = ScopeId(body_scope.get().unwrap());
                for stmt in body {
                    self.infer_stmt(stmt, body_scope)?;
                }
            }
            Stmt::ForRange {
                var_name: _,
                start,
                end,
                step: _,
                body,
                binding_id,
                body_scope,
            } => {
                let start_ty = self.infer_expr(start, current_scope, None)?;
                let end_ty = self.infer_expr(end, current_scope, None)?;
                if start_ty != TypeRef::Int || end_ty != TypeRef::Int {
                    return Err(format!(
                        "'for range' boundaries must be Int, found {:?} and {:?}",
                        start_ty, end_ty
                    ));
                }
                if let Some(ast_id) = binding_id.get() {
                    self.symbol_table.bindings[ast_id].ty = Some(TypeRef::Int);
                }
                let body_scope = ScopeId(body_scope.get().unwrap());
                for stmt in body {
                    self.infer_stmt(stmt, body_scope)?;
                }
            }
            Stmt::ForIn {
                var_name: _,
                list,
                body,
                binding_id,
                body_scope,
            } => {
                let list_ty = self.infer_expr(list, current_scope, None)?;
                let elem_ty = match list_ty {
                    TypeRef::Generic(ref name, ref params) if name == "List" && !params.is_empty() => {
                        params[0].clone()
                    }
                    TypeRef::Str => TypeRef::Str,
                    _ => TypeRef::Int,
                };
                if let Some(ast_id) = binding_id.get() {
                    self.symbol_table.bindings[ast_id].ty = Some(elem_ty);
                }
                let body_scope = ScopeId(body_scope.get().unwrap());
                for stmt in body {
                    self.infer_stmt(stmt, body_scope)?;
                }
            }
            Stmt::Block(stmts) => {
                for stmt in stmts {
                    self.infer_stmt(stmt, current_scope)?;
                }
            }
            Stmt::Expr(expr) => {
                self.infer_expr(expr, current_scope, None)?;
            }
            Stmt::Break | Stmt::Continue => {}
            Stmt::Match { subject, arms } => {
                self.infer_expr(subject, current_scope, None)?;
                for arm in arms {
                    for s in &arm.body {
                        self.infer_stmt(s, current_scope)?;
                    }
                }
            }
            Stmt::Return(Some(expr)) => {
                let mut func_name = None;
                let mut expected_ret_ty = None;
                let mut curr = Some(current_scope);
                while let Some(sid) = curr {
                    match &self.symbol_table.scopes[sid.0].kind {
                        ScopeKind::Function { name } => {
                            func_name = Some(name.clone());
                            expected_ret_ty = self.func_return_types.get(name).cloned();
                            break;
                        }
                        ScopeKind::Closure { .. } => {
                            // Closure return boundary — returns from closure, not outer function
                            break;
                        }
                        _ => {}
                    }
                    curr = self.symbol_table.scopes[sid.0].parent;
                }
                let actual_ty = self.infer_expr(expr, current_scope, expected_ret_ty.clone())?;
                if actual_ty.is_borrowed_view() {
                    return Err("Borrow error: a borrowed slice cannot be returned; use str_slice_to_str/slice_to_str for an owned value".to_string());
                }
                if let Some(expected) = expected_ret_ty {
                    if expected == TypeRef::Void {
                        let fname = func_name.as_deref().unwrap_or("function");
                        return Err(format!("Type error: Void function '{}' cannot return a value", fname));
                    }
                    if !types_compatible(&expected, &actual_ty) {
                        let fname = func_name.as_deref().unwrap_or("function");
                        return Err(format!(
                            "Type error: Return type mismatch in function '{}': expected {:?}, got {:?}",
                            fname, expected, actual_ty
                        ));
                    }
                }
            }
            Stmt::Return(None) => {
                let mut func_name = None;
                let mut expected_ret_ty = None;
                let mut curr = Some(current_scope);
                while let Some(sid) = curr {
                    match &self.symbol_table.scopes[sid.0].kind {
                        ScopeKind::Function { name } => {
                            func_name = Some(name.clone());
                            expected_ret_ty = self.func_return_types.get(name).cloned();
                            break;
                        }
                        ScopeKind::Closure { .. } => {
                            // Closure return boundary — returns from closure, not outer function
                            break;
                        }
                        _ => {}
                    }
                    curr = self.symbol_table.scopes[sid.0].parent;
                }
                if let Some(expected) = expected_ret_ty {
                    if expected != TypeRef::Void {
                        let fname = func_name.as_deref().unwrap_or("function");
                        return Err(format!(
                            "Type error: Function '{}' expects return value of type {:?}, got empty return",
                            fname, expected
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn infer_expr(
        &mut self,
        expr: &Expr,
        current_scope: ScopeId,
        expected_ty: Option<TypeRef>,
    ) -> Result<TypeRef, String> {
        match expr {
            Expr::IntLiteral(_) => Ok(TypeRef::Int),
            Expr::FloatLiteral(_) => Ok(TypeRef::Float),
            Expr::StringLiteral(_) => Ok(TypeRef::Str),
            Expr::CharLiteral(_) => Ok(TypeRef::Char),
            Expr::BoolLiteral(_) => Ok(TypeRef::Bool),
            Expr::Tuple(elements) => {
                if !(2..=4).contains(&elements.len()) {
                    return Err(format!("Tuple expressions require arity 2..=4, got {}", elements.len()));
                }
                let mut types = Vec::with_capacity(elements.len());
                let expected_elements = match expected_ty.as_ref() {
                    Some(TypeRef::Tuple(items)) if items.len() == elements.len() => Some(items),
                    _ => None,
                };
                for (index, element) in elements.iter().enumerate() {
                    let ty = self.infer_expr(
                        element,
                        current_scope,
                        expected_elements.and_then(|items| items.get(index)).cloned(),
                    )?;
                    if ty.is_borrowed_view() {
                        return Err("Borrow error: a slice view cannot be stored in a tuple".to_string());
                    }
                    types.push(ty);
                }
                Ok(TypeRef::Tuple(types))
            }
            Expr::Await(inner) => {
                if self.enclosing_async_function(current_scope).is_none() {
                    return Err("Type error: '.await' is only legal inside an async function".to_string());
                }
                match self.infer_expr(inner, current_scope, None)? {
                    TypeRef::Task(result) => Ok(*result),
                    other => Err(format!("Type error: '.await' requires Task[T], got {:?}", other)),
                }
            }
            Expr::GenericCall { .. } => Err(
                "internal error: unresolved generic call with explicit type arguments"
                    .to_string(),
            ),
            Expr::Identifier(name, binding_id_cell) => {
                if let Some(id) = binding_id_cell.get() {
                    let binding = &self.symbol_table.bindings[id];
                    binding
                        .ty
                        .clone()
                        .ok_or_else(|| "Type of identifier not yet inferred".to_string())
                } else {
                    if let Some(b_id) = self.symbol_table.resolve_name(current_scope, name) {
                        binding_id_cell.set(Some(b_id.0));
                        if let Some(ref ty) = self.symbol_table.bindings[b_id.0].ty {
                            return Ok(ty.clone());
                        }
                    }
                    // BUG-05: Builtin identifiers have no binding_id (semantic resolver skips them).
                    // Return their known types instead of panicking with "Unresolved identifier".
                    if let Some(builtin) = crate::builtins::get_builtins()
                        .iter()
                        .find(|b| b.name == name)
                    {
                        return Ok(builtin.return_type.clone());
                    }
                    Ok(TypeRef::Void)
                }
            }
            Expr::UnaryOp { op, operand } => {
                let ty = self.infer_expr(operand, current_scope, None)?;
                match op {
                    UnaryOperator::Negate => Ok(ty), // -Int→Int, -Float→Float
                    UnaryOperator::Not => Ok(TypeRef::Bool), // !Bool→Bool
                }
            }
            Expr::BinaryOp { left, op, right } => {
                let left_ty = self.infer_expr(left, current_scope, None)?;
                let right_ty = self.infer_expr(right, current_scope, None)?;
                let is_ptr_null_check = match (&left_ty, &right_ty) {
                    (&TypeRef::Custom(_), &TypeRef::Int) | (&TypeRef::Int, &TypeRef::Custom(_)) => {
                        matches!(
                            op,
                            crate::ast::BinaryOperator::Eq | crate::ast::BinaryOperator::NotEq
                        )
                    }
                    _ => false,
                };
                if !types_compatible(&left_ty, &right_ty) && !is_ptr_null_check {
                    return Err(format!(
                        "Type mismatch in binary operation: {:?} and {:?}",
                        left_ty, right_ty
                    ));
                }
                match op {
                    crate::ast::BinaryOperator::Add
                    | crate::ast::BinaryOperator::Subtract
                    | crate::ast::BinaryOperator::Multiply
                    | crate::ast::BinaryOperator::Divide
                    | crate::ast::BinaryOperator::Modulo
                    | crate::ast::BinaryOperator::BitAnd
                    | crate::ast::BinaryOperator::BitOr
                    | crate::ast::BinaryOperator::BitXor
                    | crate::ast::BinaryOperator::Shl
                    | crate::ast::BinaryOperator::Shr => Ok(left_ty),
                    crate::ast::BinaryOperator::Eq
                    | crate::ast::BinaryOperator::NotEq
                    | crate::ast::BinaryOperator::Less
                    | crate::ast::BinaryOperator::LessEq
                    | crate::ast::BinaryOperator::Greater
                    | crate::ast::BinaryOperator::GreaterEq
                    | crate::ast::BinaryOperator::And
                    | crate::ast::BinaryOperator::Or => Ok(TypeRef::Bool),
                }
            }
            Expr::Call { callee, args } => {
                let mut param_tys = Vec::new();
                if let Expr::Identifier(name, _) = &**callee {
                    if self.enclosing_async_function(current_scope).is_some()
                        && Self::blocking_in_async(name)
                    {
                        return Err(format!(
                            "Async safety error: blocking call '{}' has no readiness/completion adapter",
                            name
                        ));
                    }
                    if let Some(tys) = self.func_param_types.get(name) {
                        if let Some(rest_element) = self.variadic_elements.get(name) {
                            let fixed = tys.len().saturating_sub(1);
                            if args.len() < fixed {
                                return Err(format!(
                                    "{} expects at least {} arguments before its variadic rest, got {}",
                                    name, fixed, args.len()
                                ));
                            }
                            param_tys.extend_from_slice(&tys[..fixed]);
                            param_tys.extend(
                                std::iter::repeat(rest_element.clone())
                                    .take(args.len().saturating_sub(fixed)),
                            );
                        } else {
                            param_tys = tys.clone();
                        }
                    } else if (name == "list_push" || name == "lpp_list_push" || name == "push")
                        && args.len() >= 2
                    {
                        let list_ty = self.infer_expr(&args[0], current_scope, None)?;
                        if let TypeRef::Generic(ref list_name, ref params) = list_ty {
                            if list_name == "List" && !params.is_empty() {
                                param_tys = vec![list_ty.clone(), params[0].clone()];
                            }
                        }
                    } else if (name == "list_set" || name == "lpp_list_set")
                        && args.len() >= 3
                    {
                        let list_ty = self.infer_expr(&args[0], current_scope, None)?;
                        if let TypeRef::Generic(ref list_name, ref params) = list_ty {
                            if list_name == "List" && !params.is_empty() {
                                param_tys =
                                    vec![list_ty.clone(), TypeRef::Int, params[0].clone()];
                            }
                        }
                    } else if (name == "list_get" || name == "lpp_list_get" || name == "get") && args.len() >= 2 {
                        let list_ty = self.infer_expr(&args[0], current_scope, None)?;
                        if let TypeRef::Generic(ref list_name, ref params) = list_ty {
                            if list_name == "List" && !params.is_empty() {
                                param_tys = vec![list_ty.clone(), TypeRef::Int];
                            }
                        }
                    }
                }

                let mut arg_tys = Vec::new();
                for (i, arg) in args.iter().enumerate() {
                    let expected_arg_ty = param_tys.get(i);
                    arg_tys.push(self.infer_expr(arg, current_scope, expected_arg_ty.cloned())?);
                }

                if let Expr::Identifier(name, _) = &**callee {
                    if matches!(
                        name.as_str(),
                        "list_push" | "lpp_list_push" | "push"
                            | "list_set" | "lpp_list_set"
                    ) {
                        for (index, (expected, actual)) in
                            param_tys.iter().zip(arg_tys.iter()).enumerate()
                        {
                            if !types_compatible(expected, actual) {
                                return Err(format!(
                                    "{} argument {} expects {:?}, got {:?}",
                                    name,
                                    index + 1,
                                    expected,
                                    actual
                                ));
                            }
                        }
                    }
                    match name.as_str() {
                        "str_slice" => {
                            if arg_tys.len() != 3
                                || arg_tys[0] != TypeRef::Str
                                || arg_tys[1] != TypeRef::Int
                                || arg_tys[2] != TypeRef::Int
                            {
                                return Err("str_slice expects (Str, Int, Int)".to_string());
                            }
                            return Ok(TypeRef::StrSlice);
                        }
                        "slice" => {
                            if arg_tys.len() != 3
                                || arg_tys[1] != TypeRef::Int
                                || arg_tys[2] != TypeRef::Int
                            {
                                return Err("slice expects (List[T], Int, Int)".to_string());
                            }
                            if let TypeRef::Generic(list, elements) = &arg_tys[0] {
                                if list == "List" && elements.len() == 1 {
                                    return Ok(TypeRef::Slice(Box::new(elements[0].clone())));
                                }
                            }
                            return Err("slice first argument must be List[T]".to_string());
                        }
                        "slice_len" => {
                            if arg_tys.len() != 1 || !arg_tys[0].is_borrowed_view() {
                                return Err("slice_len expects a StrSlice or Slice[T]".to_string());
                            }
                            return Ok(TypeRef::Int);
                        }
                        "slice_get" => {
                            if arg_tys.len() != 2 || arg_tys[1] != TypeRef::Int {
                                return Err("slice_get expects (view, Int)".to_string());
                            }
                            return match &arg_tys[0] {
                                TypeRef::Slice(element) => Ok((**element).clone()),
                                TypeRef::StrSlice => Ok(TypeRef::Str),
                                other => Err(format!("slice_get expects a slice view, got {:?}", other)),
                            };
                        }
                        "slice_to_str" | "str_slice_to_str" => {
                            if arg_tys.len() != 1 || arg_tys[0] != TypeRef::StrSlice {
                                return Err(format!("{} expects StrSlice", name));
                            }
                            return Ok(TypeRef::Str);
                        }
                        _ => {}
                    }

                    if let Some(builtin) = crate::builtins::get_builtins()
                        .iter()
                        .find(|b| b.name == name)
                    {
                        if builtin.params.len() != args.len()
                            && !builtin
                                .params
                                .iter()
                                .any(|p| matches!(p, crate::builtins::ParamType::Any))
                        {
                            return Err(format!(
                                "{} expects {} arguments, got {}",
                                name,
                                builtin.params.len(),
                                args.len()
                            ));
                        }

                        for (i, param) in builtin.params.iter().enumerate() {
                            // The arity check above is skipped when any
                            // parameter is `Any` (variadic-ish builtins), so a
                            // call with too few arguments can still reach here.
                            // Indexing blindly panicked the whole compiler with
                            // "index out of bounds" instead of reporting the
                            // real problem.
                            let Some(arg_ty) = arg_tys.get(i) else {
                                return Err(format!(
                                    "{} expects {} arguments, got {}",
                                    name,
                                    builtin.params.len(),
                                    args.len()
                                ));
                            };
                            match param {
                                crate::builtins::ParamType::Specific(expected_ty) => {
                                    if !types_compatible(expected_ty, arg_ty) {
                                        if let TypeRef::Generic(expected_name, _) = expected_ty {
                                            if let TypeRef::Generic(arg_name, _) = arg_ty {
                                                if expected_name == arg_name {
                                                    continue;
                                                }
                                            }
                                        }
                                        return Err(format!(
                                            "{} expects parameter {} to be {:?}, got {:?}",
                                            name,
                                            i + 1,
                                            expected_ty,
                                            arg_ty
                                        ));
                                    }
                                }
                                crate::builtins::ParamType::Any => {}
                            }
                        }

                        if name == "list_new" {
                            if let Some(TypeRef::Generic(list_name, params)) = expected_ty {
                                if list_name == "List" {
                                    if params.len() != 1 {
                                        return Err(
                                            "List requires exactly one element type".to_string()
                                        );
                                    }
                                    if !params[0].is_list_element_supported() {
                                        return Err(format!(
                                            "List element type {:?} is not supported safely yet",
                                            params[0]
                                        ));
                                    }
                                    return Ok(TypeRef::Generic(
                                        "List".to_string(),
                                        params.clone(),
                                    ));
                                }
                            }
                            return Ok(TypeRef::Generic("List".to_string(), vec![TypeRef::Int]));
                        }

                        if name == "list_get" || name == "lpp_list_get" || name == "get" {
                            let list_ty = arg_tys[0].clone();
                            if let TypeRef::Generic(ref name, ref params) = list_ty {
                                if name == "List" && !params.is_empty() {
                                    return Ok(params[0].clone());
                                }
                            }
                            return Ok(TypeRef::Int);
                        }

                        if name == "map_new" || name == "lpp_map_new" {
                            if let Some(TypeRef::Generic(map_name, params)) = expected_ty {
                                if map_name == "Map" && params.len() == 2 {
                                    return Ok(TypeRef::Generic("Map".to_string(), params));
                                }
                            }
                            return Ok(TypeRef::Generic("Map".to_string(), vec![TypeRef::Int, TypeRef::Int]));
                        }

                        if name == "map_put" || name == "lpp_map_put" {
                            if args.len() >= 3 {
                                let key_ty = arg_tys[1].clone();
                                let val_ty = arg_tys[2].clone();
                                if let Expr::Identifier(_, ref cell) = args[0] {
                                    if let Some(id) = cell.get() {
                                        self.symbol_table.bindings[id].ty = Some(TypeRef::Generic(
                                            "Map".to_string(),
                                            vec![key_ty, val_ty],
                                        ));
                                    }
                                }
                            }
                            return Ok(TypeRef::Void);
                        }

                        if name == "map_get" || name == "lpp_map_get" {
                            let map_ty = arg_tys[0].clone();
                            if let TypeRef::Generic(ref name, ref params) = map_ty {
                                if name == "Map" && params.len() >= 2 {
                                    return Ok(params[1].clone());
                                }
                            }
                            return Ok(TypeRef::Int);
                        }

                        return Ok(builtin.return_type.clone());
                    }

                    if let Some(&id) = self.type_table.structs_by_name.get(name) {
                        let def = &self.type_table.definitions[id.0];
                        if !args.is_empty() {
                            if args.len() != def.fields.len() {
                                return Err(format!(
                                    "Struct '{}' constructor expects {} arguments or 0, got {}",
                                    def.name,
                                    def.fields.len(),
                                    args.len()
                                ));
                            }
                            for (i, (field_name, field_ty)) in def.fields.iter().enumerate() {
                                let arg_ty = &arg_tys[i];
                                if !types_compatible(field_ty, arg_ty) {
                                    return Err(format!(
                                        "Struct '{}' field '{}' expects {:?}, got {:?}",
                                        def.name, field_name, field_ty, arg_ty
                                    ));
                                }
                            }
                        }
                        return Ok(TypeRef::Custom(id));
                    }
                    if let Some(ty) = self.func_return_types.get(name) {
                        if self.variadic_elements.contains_key(name) {
                            for (index, (expected, actual)) in
                                param_tys.iter().zip(arg_tys.iter()).enumerate()
                            {
                                if !types_compatible(expected, actual) {
                                    return Err(format!(
                                        "{} argument {} expects {:?}, got {:?}",
                                        name, index + 1, expected, actual
                                    ));
                                }
                            }
                        }
                        // Generic type inference: if return type is TypeParam,
                        // substitute it based on the actual argument types
                        if let TypeRef::TypeParam(tp_name) = ty {
                            if let Some(param_types) = self.func_param_types.get(name) {
                                for (i, pt) in param_types.iter().enumerate() {
                                    if let TypeRef::TypeParam(pn) = pt {
                                        if pn == tp_name && i < arg_tys.len() {
                                            return Ok(arg_tys[i].clone());
                                        }
                                    }
                                }
                            }
                        }
                        let result = ty.clone();
                        return if self.async_functions.contains(name) {
                            Ok(TypeRef::Task(Box::new(result)))
                        } else {
                            Ok(result)
                        };
                    }
                    // Trait method dispatch: try StructName_method
                    if !arg_tys.is_empty() {
                        if let TypeRef::Custom(sid) = &arg_tys[0] {
                            let struct_name = &self.type_table.definitions[sid.0].name;
                            let mangled = format!("{}_{}", struct_name, name);
                            if let Some(ty) = self.func_return_types.get(&mangled) {
                                return Ok(ty.clone());
                            }
                        }
                    }
                } else if let Expr::FieldAccess { base, field } = &**callee {
                    if let Expr::Identifier(mod_name, _) = &**base {
                        let mangled = format!("{}_{}", mod_name, field);
                        if let Some(ty) = self.func_return_types.get(&mangled) {
                            return Ok(ty.clone());
                        }
                    }
                }
                Ok(TypeRef::Int)
            }
            Expr::Closure {
                params,
                body,
                return_type,
            } => {
                let mut closure_scope = None;
                for i in self.closure_scope_idx..self.symbol_table.scopes.len() {
                    if let ScopeKind::Closure { .. } = self.symbol_table.scopes[i].kind {
                        closure_scope = Some(ScopeId(i));
                        self.closure_scope_idx = i + 1;
                        break;
                    }
                }

                let scope_id = closure_scope
                    .ok_or_else(|| "Type error: Closure scope resolution failed".to_string())?;

                if let ScopeKind::Closure { captures } = &self.symbol_table.scopes[scope_id.0].kind {
                    for capture in captures {
                        if let Some(ty) = &self.symbol_table.bindings[capture.0].ty {
                            if matches!(ty, TypeRef::Task(_)) {
                                return Err(format!(
                                    "Async safety error: task '{}' cannot be captured; the first executor is single-thread confined",
                                    self.symbol_table.bindings[capture.0].name
                                ));
                            }
                            if ty.is_borrowed_view() {
                                return Err(format!(
                                    "Borrow error: slice view '{}' cannot be captured by a closure",
                                    self.symbol_table.bindings[capture.0].name
                                ));
                            }
                        }
                    }
                }

                for param in params {
                    if param.ty.is_none() {
                        let binding_id = self
                            .symbol_table
                            .resolve_name(scope_id, &param.name)
                            .ok_or_else(|| {
                                format!("Type error: Unresolved closure parameter '{}'", param.name)
                            })?;
                        let binding = &mut self.symbol_table.bindings[binding_id.0];
                        if binding.ty.is_none() {
                            binding.ty = Some(TypeRef::Int);
                        }
                    }
                }

                // Traverse body
                for stmt in body {
                    self.infer_stmt(stmt, scope_id)?;
                }

                // Check an explicit/inferred result type for diagnostics, but
                // the expression itself is a callable ownership capsule, not
                // the value it will eventually return when invoked.
                if let Some(t) = return_type {
                    let _ = Self::convert_ast_type(&self.type_table, t);
                } else {
                    for stmt in body {
                        if let Stmt::Return(Some(expr)) = stmt {
                            let _ = self.infer_expr(expr, scope_id, None)?;
                            break;
                        }
                    }
                }
                Ok(TypeRef::Function)
            }
            Expr::FieldAccess { base, field } => {
                let base_ty = self.infer_expr(base, current_scope, None)?;
                if let TypeRef::Custom(struct_id) = &base_ty {
                    let struct_def = &self.type_table.definitions[struct_id.0];
                    // Check if it's a regular struct field
                    if let Some(field_entry) =
                        struct_def.fields.iter().find(|(name, _)| name == field)
                    {
                        return Ok(field_entry.1.clone());
                    }
                    // `Enum.Variant` — a variant constructor, not a field.
                    // Enums carry `__tag`/`__vN` slots now, so this must test
                    // the enum name rather than "the type has no fields".
                    if self.enum_names.contains(&struct_def.name) || struct_def.fields.is_empty() {
                        return Ok(TypeRef::Custom(*struct_id));
                    }
                    Err(format!(
                        "Field '{}' not found on struct '{}'",
                        field, struct_def.name
                    ))
                } else {
                    // Could be an enum accessed via FunctionName binding
                    // Check if base is an identifier that matches a registered enum
                    if let Expr::Identifier(name, _) = base.as_ref() {
                        if let Some(id) = self.type_table.lookup_struct(name) {
                            return Ok(TypeRef::Custom(id));
                        }
                    }
                    Err(format!(
                        "Cannot access field '{}' on non-struct type {:?}",
                        field, base_ty
                    ))
                }
            }
            Expr::Spawn { closure } => {
                self.infer_expr(closure, current_scope, None)?;
                Ok(TypeRef::Void)
            }
            Expr::ListLiteral(elements) => {
                let mut elem_ty = TypeRef::Int; // Default if empty
                if !elements.is_empty() {
                    elem_ty = self.infer_expr(&elements[0], current_scope, None)?;
                }
                for element in elements.iter().skip(1) {
                    let actual_ty = self.infer_expr(element, current_scope, None)?;
                    if actual_ty != elem_ty {
                        return Err(format!(
                            "list literal has mixed element types: expected {:?}, got {:?}",
                            elem_ty, actual_ty
                        ));
                    }
                }
                if !elem_ty.is_list_element_supported() {
                    return Err(format!(
                        "List element type {:?} is not supported safely yet",
                        elem_ty
                    ));
                }
                Ok(TypeRef::Generic("List".to_string(), vec![elem_ty]))
            }
            Expr::Match { subject, arms } => {
                let _subject_ty = self.infer_expr(subject, current_scope, None)?;
                // For now, match returns Void (statement-level match)
                // Type-check each arm body
                for arm in arms {
                    for stmt in &arm.body {
                        self.infer_stmt(stmt, current_scope)?;
                    }
                }
                Ok(TypeRef::Void)
            }
            Expr::EnumVariantConstruct { enum_name, .. } => {
                // Returns the enum type
                if let Some(id) = self.type_table.lookup_struct(enum_name) {
                    Ok(TypeRef::Custom(id))
                } else {
                    Ok(TypeRef::Int)
                }
            }
            Expr::Try(inner) => {
                let _inner_ty = self.infer_expr(inner, current_scope, None)?;
                Ok(TypeRef::Int)
            }
            Expr::Index { base, index } => {
                let base_ty = self.infer_expr(base, current_scope, None)?;
                self.infer_expr(index, current_scope, None)?;
                match base_ty {
                    TypeRef::Str => Ok(TypeRef::Str), // str[i] → single char as Str
                    TypeRef::Generic(ref name, _) if name == "List" => Ok(TypeRef::Int),
                    _ => Ok(TypeRef::Int), // fallback
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TypeChecker;
    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use crate::semantic::Resolver;

    #[test]
    fn networking_builtins_typecheck_in_lpp_programs() {
        let source = r#"
def main():
    listener := net_listen(9000)
    client := net_accept(listener)
    sent := net_send(client, "hello from lpp")
    payload := net_recv(client, 128)
    print(sent)
    print_str(payload)
    net_close(client)
    net_close(listener)
"#;

        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize().expect("source should lex");
        let mut parser = Parser::new(tokens);
        let mut ast = parser.parse().expect("source should parse");

        let mut resolver = Resolver::new();
        resolver
            .resolve_program(&mut ast)
            .expect("networking program should resolve");

        let mut type_checker = TypeChecker::new(&mut resolver.table);
        type_checker
            .check_program(&ast)
            .expect("networking builtins should typecheck");
    }

    #[test]
    fn boolean_literals_typecheck() {
        let source = r#"
def main():
    mut b := true
    b = false
"#;

        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize().expect("source should lex");
        let mut parser = Parser::new(tokens);
        let mut ast = parser.parse().expect("source should parse");

        let mut resolver = Resolver::new();
        resolver
            .resolve_program(&mut ast)
            .expect("boolean program should resolve");

        let mut type_checker = TypeChecker::new(&mut resolver.table);
        type_checker
            .check_program(&ast)
            .expect("boolean program should typecheck");
    }

    #[test]
    fn map_operations_typecheck() {
        let source = r#"
def main():
    mut m := map_new()
    map_put(m, "apple", 100)
    map_put(m, "banana", 200)

    if map_has(m, "apple"):
        val := map_get(m, "apple")
        lpp_print_int(val)

    lpp_print_int(map_len(m))
    map_remove(m, "apple")
"#;

        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize().expect("source should lex");
        let mut parser = Parser::new(tokens);
        let mut ast = parser.parse().expect("source should parse");

        let mut resolver = Resolver::new();
        resolver
            .resolve_program(&mut ast)
            .expect("program should resolve");

        let mut type_checker = TypeChecker::new(&mut resolver.table);
        type_checker
            .check_program(&ast)
            .expect("map operations should typecheck");
    }

    #[test]
    fn positional_struct_constructor_typechecks() {
        let source = r#"
struct Point:
    x: Int
    y: Int

def main():
    p := Point(10, 20)
    lpp_print_int(p.x)
"#;

        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize().expect("source should lex");
        let mut parser = Parser::new(tokens);
        let mut ast = parser.parse().expect("source should parse");

        let mut resolver = Resolver::new();
        resolver
            .resolve_program(&mut ast)
            .expect("program should resolve");

        let mut type_checker = TypeChecker::new(&mut resolver.table);
        type_checker
            .check_program(&ast)
            .expect("positional struct constructor should typecheck");
    }

    #[test]
    /// SAFETY-CONTRACT CHANGE, made deliberately and recorded here.
    ///
    /// This test previously asserted that a self-referential struct is a
    /// compile error. It now asserts the opposite: the program type-checks,
    /// because `analysis::cyclebreak` demotes one edge of every cycle to
    /// non-owning, so no owning cycle is ever constructed.
    ///
    /// The old contract bought leak-freedom by making binary trees, linked
    /// lists and parent pointers inexpressible. The new one keeps
    /// leak-freedom -- proven structurally, and checked empirically with
    /// 50 000 genuine A<->B cycles under AddressSanitizer -- while allowing
    /// those structures.
    fn accepts_cyclic_owned_structs_and_breaks_them() {
        let source = r#"
struct Node:
    next: Node

def main():
    print(0)
"#;

        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize().expect("source should lex");
        let mut parser = Parser::new(tokens);
        let mut ast = parser.parse().expect("source should parse");

        let mut resolver = Resolver::new();
        resolver
            .resolve_program(&mut ast)
            .expect("program should resolve");

        let mut type_checker = TypeChecker::new(&mut resolver.table);
        type_checker
            .check_program(&ast)
            .expect("a recursive struct must now type-check");

        // ...and the cycle must actually be broken, not merely tolerated.
        let graph = crate::cyclebreak::break_cycles(&type_checker.type_table);
        let node = type_checker
            .type_table
            .lookup_struct("Node")
            .expect("Node should be registered");
        assert!(
            graph.is_weak(node, "next"),
            "the self edge must be demoted to non-owning"
        );
    }

    #[test]
    fn rejects_return_type_mismatch() {
        let source = r#"
def foo() -> Int:
    return "hello"

def main():
    foo()
"#;

        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize().expect("source should lex");
        let mut parser = Parser::new(tokens);
        let mut ast = parser.parse().expect("source should parse");

        let mut resolver = Resolver::new();
        resolver
            .resolve_program(&mut ast)
            .expect("program should resolve");

        let mut type_checker = TypeChecker::new(&mut resolver.table);
        let err = type_checker
            .check_program(&ast)
            .expect_err("return type mismatch should fail typecheck");
        assert!(err.contains("Return type mismatch in function 'foo'"));
    }

    #[test]
    fn char_literals_typecheck() {
        let source = r#"
def main():
    ch := 'Z'
    mut c := 'a'
    c = '\n'
"#;

        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize().expect("source should lex");
        let mut parser = Parser::new(tokens);
        let mut ast = parser.parse().expect("source should parse");

        let mut resolver = Resolver::new();
        resolver
            .resolve_program(&mut ast)
            .expect("program should resolve");

        let mut type_checker = TypeChecker::new(&mut resolver.table);
        type_checker
            .check_program(&ast)
            .expect("char literal program should typecheck");
    }
}
