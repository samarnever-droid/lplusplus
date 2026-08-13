use crate::ast::*;
use crate::mir::builder::MirBuilder;
use crate::mir::ir::*;
use crate::semantic::{BindingId, ScopeKind, SymbolTable};
use crate::type_facts::ListElementClass;
use crate::types::{TypeRef, TypeTable};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
pub struct LoopTargets {
    pub break_block: BlockId,
    pub continue_block: BlockId,
}

fn list_push_symbol(element: &TypeRef) -> Result<&'static str, String> {
    match element.list_element_class() {
        ListElementClass::Scalar => Ok("lpp_list_push"),
        ListElementClass::Bool => Ok("lpp_list_push_bool"),
        ListElementClass::Float => Ok("lpp_list_push_float"),
        ListElementClass::Arc => Ok("lpp_list_push_arc"),
        ListElementClass::Unsupported => Err(format!(
            "unsupported list element type {:?} reached MIR lowering",
            element
        )),
    }
}

fn list_get_symbol(element: &TypeRef) -> Result<&'static str, String> {
    match element.list_element_class() {
        ListElementClass::Scalar => Ok("lpp_list_get"),
        ListElementClass::Bool => Ok("lpp_list_get_bool"),
        ListElementClass::Float => Ok("lpp_list_get_float"),
        ListElementClass::Arc => Ok("lpp_list_get_arc"),
        ListElementClass::Unsupported => Err(format!(
            "unsupported list element type {:?} reached MIR lowering",
            element
        )),
    }
}

fn list_set_symbol(element: &TypeRef) -> Result<&'static str, String> {
    match element.list_element_class() {
        ListElementClass::Scalar => Ok("lpp_list_set"),
        ListElementClass::Bool => Ok("lpp_list_set_bool"),
        ListElementClass::Float => Ok("lpp_list_set_float"),
        ListElementClass::Arc => Ok("lpp_list_set_arc"),
        ListElementClass::Unsupported => Err(format!(
            "unsupported list element type {:?} reached MIR lowering",
            element
        )),
    }
}

pub struct MirLowerCtx<'a> {
    pub symbol_table: &'a SymbolTable,
    pub type_table: &'a mut TypeTable,
    pub functions: HashMap<String, FuncId>,
    pub func_return_types: HashMap<String, TypeRef>,
    /// Return types of closure *values*, keyed by `(function, local)`.
    /// Closure locals all share `TypeRef::Function`, which carries no
    /// signature, so calls through them would otherwise fall back to
    /// `TypeRef::Int` and emit a `call_indirect` whose result type does
    /// not match the lifted function (runtime "indirect call type
    /// mismatch" for any non-Int-returning closure).
    pub closure_returns: HashMap<(usize, usize), TypeRef>,
    pub next_func_id: usize,

    // Program reference for enum lookups
    pub program: &'a crate::ast::Program,

    // Closure compilation context
    pub lifted_functions: HashMap<FuncId, MirFunction>,
    pub closure_scope_idx: usize,
    pub current_env_ptr: Option<LocalId>,
    pub current_captures: Vec<BindingId>,

    // Loop target stack for break and continue
    pub loop_stack: Vec<LoopTargets>,

    // Match arm bindings (name → local_id for data extraction)
    pub match_bindings: HashMap<String, LocalId>,

    // Top-level constants (name → value)
    pub constants: HashMap<String, i64>,

    // Current function's type parameters (for generics)
    pub current_type_params: Vec<String>,

    // Trait definitions: trait_name → list of method names
    pub trait_defs: HashMap<String, Vec<String>>,
    // Impl registry: (trait_name, target_type) → mangled method names
    pub impl_registry: HashMap<(String, String), Vec<String>>,
    // Set of known trait names for type resolution
    pub trait_names: std::collections::HashSet<String>,
    // Current function's vtable locals: param_name → (trait_name, method_name → LocalId)
    pub current_vtable_locals: HashMap<String, (String, HashMap<String, LocalId>)>,
    // Extern (FFI) function names → C symbol names
    pub extern_symbols: HashMap<String, String>,
    /// Storage classification from `analysis::escape`.
    ///
    /// The return rule below used to be purely type-shaped: any
    /// `Custom`/`Function`/`Generic`/`Str` was returned owned, and the escape
    /// analysis had no say. This is how the analysis finally participates in
    /// that decision.
    pub current_arena: Option<LocalId>,
    /// Calls to these functions construct Task[T] instead of invoking the body.
    pub async_functions: std::collections::HashSet<String>,
}

impl<'a> MirLowerCtx<'a> {
    pub fn new(symbol_table: &'a SymbolTable, type_table: &'a mut TypeTable, program: &'a crate::ast::Program) -> Self {
        Self {
            symbol_table,
            type_table,
            functions: HashMap::new(),
            func_return_types: HashMap::new(),
            closure_returns: HashMap::new(),
            next_func_id: 0,
            program,
            lifted_functions: HashMap::new(),
            closure_scope_idx: 0,
            current_env_ptr: None,
            current_captures: Vec::new(),
            loop_stack: Vec::new(),
            match_bindings: HashMap::new(),
            constants: HashMap::new(),
            current_type_params: Vec::new(),
            trait_defs: HashMap::new(),
            impl_registry: HashMap::new(),
            trait_names: std::collections::HashSet::new(),
            current_vtable_locals: HashMap::new(),
            extern_symbols: HashMap::new(),
            current_arena: None,
            async_functions: std::collections::HashSet::new(),
        }
    }

    fn get_field_type(&self, base_ty: &TypeRef, field: &str) -> TypeRef {
        if let TypeRef::Custom(struct_id) = base_ty {
            let struct_def = &self.type_table.definitions[struct_id.0];
            if let Some((_, ty)) = struct_def.fields.iter().find(|(name, _)| name == field) {
                return ty.clone();
            }
        }
        TypeRef::Void
    }

    fn resolve_type(&self, ty: &Type) -> TypeRef {
        match ty {
            Type::Int => TypeRef::Int,
            Type::Float => TypeRef::Float,
            Type::String => TypeRef::Str,
            Type::Bool => TypeRef::Bool,
            Type::Char => TypeRef::Char,
            Type::Void => TypeRef::Void,
            Type::Custom(name) => {
                // Check if it's a type parameter first
                if self.current_type_params.iter().any(|tp| tp == name) {
                    return TypeRef::TypeParam(name.clone());
                }
                // Trait names resolve to Int (trait objects are opaque i64 pointers)
                if self.trait_names.contains(name) {
                    return TypeRef::Int;
                }
                self.type_table
                    .structs_by_name
                    .get(name)
                    .copied()
                    .map(TypeRef::Custom)
                    .unwrap_or_else(|| TypeRef::Unresolved(name.clone()))
            }
            Type::Generic(name, args) => TypeRef::Generic(
                name.clone(),
                args.iter().map(|arg| self.resolve_type(arg)).collect(),
            ),
            Type::Tuple(elements) =>
                TypeRef::Tuple(elements.iter().map(|ty| self.resolve_type(ty)).collect()),
            Type::StrSlice => TypeRef::StrSlice,
            Type::Slice(element) => TypeRef::Slice(Box::new(self.resolve_type(element))),
            Type::Task(result) => TypeRef::Task(Box::new(self.resolve_type(result))),
        }
    }

    fn expr_type_hint(
        &self,
        expr: &Expr,
        builder: &MirBuilder,
        binding_map: &HashMap<BindingId, LocalId>,
    ) -> TypeRef {
        match expr {
            Expr::IntLiteral(_) => TypeRef::Int,
            Expr::FloatLiteral(_) => TypeRef::Float,
            Expr::StringLiteral(_) => TypeRef::Str,
            Expr::CharLiteral(_) => TypeRef::Char,
            Expr::BoolLiteral(_) => TypeRef::Bool,
            Expr::Tuple(elements) => TypeRef::Tuple(
                elements
                    .iter()
                    .map(|element| self.expr_type_hint(element, builder, binding_map))
                    .collect(),
            ),
            Expr::Await(inner) => match self.expr_type_hint(inner, builder, binding_map) {
                TypeRef::Task(result) => *result,
                _ => TypeRef::Void,
            },
            Expr::Identifier(_, cell) => {
                if let Some(ast_id) = cell.get() {
                    if let Some(local_id) = binding_map.get(&BindingId(ast_id)) {
                        return builder.function.locals[local_id.0].ty.clone();
                    }
                }
                TypeRef::Int
            }
            Expr::FieldAccess { base, field } => {
                // Check for enum variant
                if let Expr::Identifier(name, _) = base.as_ref() {
                    if let Some(id) = self.type_table.lookup_struct(name) {
                        // Check if it's an enum
                        for decl in &self.program.declarations {
                            if let crate::ast::TopLevel::Enum(e) = decl {
                                if e.name == *name {
                                    return TypeRef::Custom(id);
                                }
                            }
                        }
                    }
                }
                let base_ty = self.expr_type_hint(base, builder, binding_map);
                self.get_field_type(&base_ty, field)
            }
            Expr::ListLiteral(items) => TypeRef::Generic(
                "List".to_string(),
                vec![
                    items
                        .first()
                        .map(|item| self.expr_type_hint(item, builder, binding_map))
                        .unwrap_or(TypeRef::Int),
                ],
            ),
            Expr::Call { callee, args } => {
                if let Expr::Identifier(name, _) = &**callee {
                    if let Some(ty) = self.func_return_types.get(name) {
                        return if self.async_functions.contains(name) {
                            TypeRef::Task(Box::new(ty.clone()))
                        } else {
                            ty.clone()
                        };
                    }
                    if let Some(&struct_id) = self.type_table.structs_by_name.get(name) {
                        return TypeRef::Custom(struct_id);
                    }
                    match name.as_str() {
                        "str_slice" => return TypeRef::StrSlice,
                        "slice" => {
                            if let Some(TypeRef::Generic(list, args)) =
                                args.first().map(|arg| self.expr_type_hint(arg, builder, binding_map))
                            {
                                if list == "List" && args.len() == 1 {
                                    return TypeRef::Slice(Box::new(args[0].clone()));
                                }
                            }
                            return TypeRef::Slice(Box::new(TypeRef::Int));
                        }
                        "slice_len" => return TypeRef::Int,
                        "slice_get" => {
                            return match args.first().map(|arg| self.expr_type_hint(arg, builder, binding_map)) {
                                Some(TypeRef::Slice(element)) => *element,
                                Some(TypeRef::StrSlice) => TypeRef::Str,
                                _ => TypeRef::Int,
                            };
                        }
                        "slice_to_str" | "str_slice_to_str" => return TypeRef::Str,
                        _ => {}
                    }
                    if let Some(builtin) = crate::builtins::get_builtins()
                        .iter()
                        .find(|b| b.name == name)
                    {
                        // list_new is special-cased because of generic parameter type inference
                        if name != "list_new" {
                            return builtin.return_type.clone();
                        }
                    }
                    return match name.as_str() {
                        "input" | "read_file" | "json_get_str" | "net_recv" | "net_recv_udp"
                        | "net_resolve" | "http_get" | "http_post" | "command_output"
                        | "env_get" | "str_concat" | "str_replace" | "str_substr" | "str_trim"
                        | "path_join" => TypeRef::Str,
                        "parse_int" | "json_parse" | "json_get_int" | "json_get_obj"
                        | "list_get" | "list_len" | "len" | "get" | "net_connect"
                        | "net_listen" | "net_listen_udp" | "net_accept" | "net_accept_timeout"
                        | "net_send" | "net_send_all" | "net_dial" | "net_dial_udp"
                        | "net_set_timeout" | "net_set_deadline" | "net_set_keepalive" => {
                            TypeRef::Int
                        }
                        "map_get" | "lpp_map_get" => {
                            let map_ty = args.first().map(|arg| self.expr_type_hint(arg, builder, binding_map));
                            if let Some(TypeRef::Generic(_, params)) = map_ty {
                                if params.len() >= 2 {
                                    return params[1].clone();
                                }
                            }
                            TypeRef::Int
                        }
                        "map_new" => TypeRef::Generic("Map".to_string(), vec![TypeRef::Int, TypeRef::Int]),
                        "map_has" => TypeRef::Bool,
                        "map_len" => TypeRef::Int,
                        "map_put" | "map_remove" => TypeRef::Void,
                        "print" | "print_str" | "json_free" | "list_push" | "list_free"
                        | "net_close" => TypeRef::Void,
                        _ => TypeRef::Int,
                    };
                }
                TypeRef::Int
            }
            Expr::UnaryOp { op, operand } => {
                match op {
                    UnaryOperator::Not => TypeRef::Bool,
                    UnaryOperator::Negate => self.expr_type_hint(operand, builder, binding_map),
                }
            }
            Expr::BinaryOp { left, .. } => {
                let left_ty = self.expr_type_hint(left, builder, binding_map);
                left_ty
            }
            Expr::Closure { .. } => TypeRef::Function,
            Expr::Spawn { .. } => TypeRef::Void,
            Expr::EnumVariantConstruct { enum_name, .. } => {
                // Look up the enum type ID from the type table
                if let Some(id) = self.type_table.lookup_struct(enum_name) {
                    TypeRef::Custom(id)
                } else {
                    TypeRef::Int // fallback
                }
            }
            Expr::Match { .. } => TypeRef::Int,
            // Monomorphization rewrites every GenericCall into a plain Call,
            // so reaching here means the pass did not run.
            Expr::GenericCall { .. } => TypeRef::Int,
            Expr::Try(_) => TypeRef::Int,
            Expr::Index { base, .. } => {
                let base_ty = self.expr_type_hint(base, builder, binding_map);
                if base_ty == TypeRef::Str { TypeRef::Str } else { TypeRef::Int }
            }
        }
    }

    pub fn lower_program(&mut self, program: &Program) -> Result<MirProgram, String> {
        let mut mir_functions = HashMap::new();

        // Register trait definitions and impl blocks for dynamic dispatch
        for decl in &program.declarations {
            if let TopLevel::Trait(t) = decl {
                let method_names: Vec<String> = t.methods.iter().map(|m| m.name.clone()).collect();
                self.trait_defs.insert(t.name.clone(), method_names);
                self.trait_names.insert(t.name.clone());
            }
            if let TopLevel::Impl(ib) = decl {
                let mangled_names: Vec<String> = ib.methods.iter().map(|m| m.name.clone()).collect();
                self.impl_registry.insert(
                    (ib.trait_name.clone(), ib.target_type.clone()),
                    mangled_names,
                );
            }
        }

        // Register extern (FFI) function symbols
        for decl in &program.declarations {
            if let TopLevel::Extern(ext) = decl {
                for ef in &ext.functions {
                    self.extern_symbols.insert(ef.name.clone(), ef.symbol.clone());
                    // Also register return types
                    self.func_return_types.insert(ef.name.clone(), self.resolve_type(&ef.return_type));
                }
            }
        }

        // Register top-level constants
        for decl in &program.declarations {
            if let TopLevel::Const { name, value } = decl {
                if let Expr::IntLiteral(v) = value {
                    self.constants.insert(name.clone(), *v);
                }
            }
        }

        // Collect all functions: top-level + impl methods
        let mut all_functions: Vec<&Function> = Vec::new();
        for decl in &program.declarations {
            if let TopLevel::Function(f) = decl {
                all_functions.push(f);
            }
            if let TopLevel::Impl(impl_block) = decl {
                for method in &impl_block.methods {
                    all_functions.push(method);
                }
            }
        }

        for f in &all_functions {
            let id = FuncId(self.next_func_id);
            self.next_func_id += 1;
            self.functions.insert(f.name.clone(), id);
            if f.is_async {
                self.async_functions.insert(f.name.clone());
            }
            let prev = std::mem::replace(&mut self.current_type_params, f.type_params.iter().map(|tp| tp.name.clone()).collect());
            self.func_return_types
                .insert(f.name.clone(), self.resolve_type(&f.return_type));
            self.current_type_params = prev;
        }

        for function in &all_functions {
            let mir_fn = self.lower_function(function)?;
            mir_functions.insert(mir_fn.id, mir_fn);
        }

        for (id, func) in self.lifted_functions.drain() {
            mir_functions.insert(id, func);
        }

        Ok(MirProgram {
            functions: mir_functions,
        })
    }

    fn ensure_arena(&mut self, builder: &mut MirBuilder) -> Result<LocalId, String> {
        if let Some(arena) = self.current_arena {
            return Ok(arena);
        }
        let arena = builder.new_local(
            TypeRef::Int,
            false,
            Some("__arena".to_string()),
            None,
        );
        builder.push_instr(MirInstr::Assign(
            arena,
            Rvalue::BuiltinCall("lpp_arena_begin".to_string(), vec![]),
        ))?;
        self.current_arena = Some(arena);
        Ok(arena)
    }

    fn struct_allocation(
        &mut self,
        builder: &mut MirBuilder,
        ty: TypeRef,
    ) -> Result<Rvalue, String> {
        if let TypeRef::Custom(id) = ty {
            if self
                .type_table
                .definitions
                .get(id.0)
                .map(|definition| definition.is_self_referential)
                .unwrap_or(false)
            {
                let arena = self.ensure_arena(builder)?;
                return Ok(Rvalue::AllocateArenaStruct(
                    TypeRef::Custom(id),
                    Operand::Local(arena),
                ));
            }
            return Ok(Rvalue::AllocateArcStruct(TypeRef::Custom(id)));
        }
        Ok(Rvalue::AllocateArcStruct(ty))
    }

    fn release_arena_if_live(&self, builder: &mut MirBuilder) -> Result<(), String> {
        if let Some(arena) = self.current_arena {
            let discard = builder.new_local(TypeRef::Void, false, None, None);
            builder.push_instr(MirInstr::Assign(
                discard,
                Rvalue::BuiltinCall(
                    "lpp_arena_release".to_string(),
                    vec![Operand::Local(arena)],
                ),
            ))?;
        }
        Ok(())
    }

    fn lower_function(&mut self, func: &Function) -> Result<MirFunction, String> {
        // Set current type parameters for this function's generics
        let prev_type_params = std::mem::replace(&mut self.current_type_params, func.type_params.iter().map(|tp| tp.name.clone()).collect());
        let prev_arena = self.current_arena.take();

        let func_id = *self.functions.get(&func.name).ok_or_else(|| {
            format!(
                "Internal error: missing MIR function id for '{}'",
                func.name
            )
        })?;
        let return_type = self.resolve_type(&func.return_type);
        let mut builder = MirBuilder::new(func_id, func.name.clone(), return_type);
        builder.function.is_async = func.is_async;
        let mut binding_map = HashMap::new();

        // Track which params are trait-typed and their vtable locals
        // vtable_locals: param_name → (trait_name, HashMap<method_name, LocalId>)
        let mut vtable_locals: HashMap<String, (String, HashMap<String, LocalId>)> = HashMap::new();

        for param in &func.params {
            let binding_id = self.symbol_table.scopes.iter().find_map(|scope| {
                if let ScopeKind::Function { name } = &scope.kind {
                    if name == &func.name {
                        return scope.bindings.get(&param.name).copied();
                    }
                }
                None
            });
            let element_ty = self.resolve_type(&param.ty);
            let ty = if param.variadic {
                TypeRef::Generic("List".to_string(), vec![element_ty])
            } else {
                element_ty
            };

            // Check if this param's type is a trait name
            let trait_name_for_param = if let Type::Custom(ref tname) = param.ty {
                if self.trait_names.contains(tname) { Some(tname.clone()) } else { None }
            } else { None };

            let local = builder.new_local(ty, false, Some(param.name.clone()), binding_id);
            builder.set_local_ownership(local, Ownership::Borrowed);
            builder.function.params.push(local);
            if let Some(binding_id) = binding_id {
                binding_map.insert(binding_id, local);
            }

            // For trait-typed params, add hidden vtable params (one per trait method)
            if let Some(ref tname) = trait_name_for_param {
                if let Some(methods) = self.trait_defs.get(tname) {
                    let mut method_locals = HashMap::new();
                    for method_name in methods {
                        let vtable_param_name = format!("_vtable_{}_{}", param.name, method_name);
                        let vtable_local = builder.new_local(
                            TypeRef::Int, false,
                            Some(vtable_param_name), None,
                        );
                        builder.set_local_ownership(vtable_local, Ownership::Borrowed);
                        builder.function.params.push(vtable_local);
                        method_locals.insert(method_name.clone(), vtable_local);
                    }
                    vtable_locals.insert(param.name.clone(), (tname.clone(), method_locals));
                }
            }
        }

        // Store vtable locals for use during method call lowering
        let prev_vtable_locals = std::mem::replace(
            &mut self.current_vtable_locals,
            vtable_locals,
        );

        for stmt in &func.body {
            self.lower_stmt(&mut builder, stmt, &mut binding_map)?;
        }

        if let Ok(current_block) = builder.current_block() {
            if current_block.0 < builder.function.blocks.len() {
                self.release_arena_if_live(&mut builder)?;
                builder.set_terminator(current_block, Terminator::Return(None))?;
            }
        }

        // Restore previous type parameters and vtable locals
        self.current_type_params = prev_type_params;
        self.current_vtable_locals = prev_vtable_locals;
        self.current_arena = prev_arena;

        Ok(builder.finish())
    }

    /// Select an explicit ownership operation for an assignment. A direct
    /// `Local` read of an owned temporary is a move; identifiers of owned
    /// variables lower to `Borrowed` and therefore stay usable after assignment.
    /// True when a local holds an ARC-managed reference.
    fn is_arc_managed(builder: &MirBuilder, local: LocalId) -> bool {
        builder.function.locals[local.0].ty.is_managed()
    }

    /// Aliasing a *borrowed* value into an owned local creates a second owner,
    /// so it needs its own reference: `c := p` where `p` is a parameter.
    ///
    /// Without this the ARC pass sees an owned local and emits `release(c)` at
    /// scope exit, but nothing ever retained it -- the callee frees an object
    /// the caller still owns, and the caller then reads freed memory. The
    /// symptom is a SIGSEGV after the function returns, with printed values all
    /// correct because the corruption only bites at teardown.
    fn retain_if_aliasing_borrow(
        builder: &mut MirBuilder,
        destination: LocalId,
        rvalue: &Rvalue,
    ) -> Result<(), String> {
        let source = match rvalue {
            Rvalue::Use(Operand::Local(source)) | Rvalue::Use(Operand::Borrowed(source)) => *source,
            _ => return Ok(()),
        };
        if builder.function.locals[destination.0].ownership != Ownership::Owned {
            return Ok(());
        }
        if !builder.function.locals[source.0].ownership.is_borrowed() {
            return Ok(());
        }
        if !Self::is_arc_managed(builder, source) {
            return Ok(());
        }
        builder.push_instr(MirInstr::Retain(destination))
    }

    /// A closure value copied into another local (`speak := fn() -> Void …`
    /// binds a fresh MIR local) keeps its recorded return type so calls
    /// through the alias emit `call_indirect` with the matching signature
    /// instead of the `Int` fallback.
    fn propagate_closure_return(
        &mut self,
        function_id: usize,
        destination: LocalId,
        rvalue: &Rvalue,
    ) {
        let source = match rvalue {
            Rvalue::Use(Operand::Local(source))
            | Rvalue::Use(Operand::Borrowed(source))
            | Rvalue::Move(source) => *source,
            _ => return,
        };
        if let Some(rt) = self.closure_returns.get(&(function_id, source.0)).cloned() {
            self.closure_returns.insert((function_id, destination.0), rt);
        }
    }

    fn assignment_rvalue(builder: &MirBuilder, destination: LocalId, operand: Operand) -> Rvalue {
        if let Operand::Local(source) = operand {
            let destination_managed = builder.function.locals[destination.0]
                .ownership
                .is_managed();
            let source_managed = builder.function.locals[source.0]
                .ownership
                .is_managed();
            if destination_managed && source_managed {
                return Rvalue::Move(source);
            }
            return Rvalue::Use(Operand::Local(source));
        }
        Rvalue::Use(operand)
    }

    fn lower_stmt(
        &mut self,
        builder: &mut MirBuilder,
        stmt: &Stmt,
        binding_map: &mut HashMap<BindingId, LocalId>,
    ) -> Result<(), String> {
        match stmt {
            Stmt::Destructure {
                names,
                value,
                binding_ids,
            } => {
                let tuple_operand = self.lower_expr(builder, value, binding_map)?;
                let tuple_ty = match &tuple_operand {
                    Operand::Local(id) | Operand::Borrowed(id) =>
                        builder.function.locals[id.0].ty.clone(),
                    _ => TypeRef::Void,
                };
                let elements = match tuple_ty {
                    TypeRef::Tuple(elements) => elements,
                    other => return Err(format!("destructuring non-tuple MIR type {:?}", other)),
                };
                if elements.len() != names.len() {
                    return Err("tuple destructuring arity changed after type checking".to_string());
                }
                for (index, (name, ty)) in names.iter().zip(elements.into_iter()).enumerate() {
                    let ast_id = binding_ids
                        .get(index)
                        .and_then(|cell| cell.get())
                        .ok_or_else(|| format!("Missing binding id for destructured name '{}'", name))?;
                    let binding_id = BindingId(ast_id);
                    let field_local = builder.new_local(
                        ty.clone(),
                        false,
                        Some(format!("__tuple_field_{}", index)),
                        None,
                    );
                    if ty.is_managed() || ty.is_borrowed_view() {
                        builder.set_local_ownership(field_local, Ownership::Borrowed);
                    }
                    builder.push_instr(MirInstr::Assign(
                        field_local,
                        Rvalue::TupleField(tuple_operand.clone(), index),
                    ))?;

                    let destination = builder.new_local(
                        ty,
                        false,
                        Some(name.clone()),
                        Some(binding_id),
                    );
                    binding_map.insert(binding_id, destination);
                    let source = if builder.function.locals[field_local.0].ownership.is_borrowed() {
                        Operand::Borrowed(field_local)
                    } else {
                        Operand::Local(field_local)
                    };
                    let rvalue = Self::assignment_rvalue(builder, destination, source);
                    builder.push_instr(MirInstr::Assign(destination, rvalue.clone()))?;
                    Self::retain_if_aliasing_borrow(builder, destination, &rvalue)?;
                }
            }
            Stmt::LetInferred {
                name,
                value,
                binding_id,
                ..
            } => {
                let ast_id = binding_id.get().ok_or_else(|| {
                    format!("Missing binding id while lowering declaration '{}'", name)
                })?;
                let binding_id = BindingId(ast_id);
                let ty = self
                    .symbol_table
                    .bindings
                    .get(ast_id)
                    .and_then(|binding| binding.ty.clone())
                    .ok_or_else(|| format!("Missing inferred type for binding '{}'", name))?;

                let local_id = builder.new_local(ty, true, Some(name.clone()), Some(binding_id));
                binding_map.insert(binding_id, local_id);

                let operand = self.lower_expr(builder, value, binding_map)?;
                let rvalue = Self::assignment_rvalue(builder, local_id, operand);
                builder.push_instr(MirInstr::Assign(local_id, rvalue.clone()))?;
                Self::retain_if_aliasing_borrow(builder, local_id, &rvalue)?;
                let fid = builder.function.id.0;
                self.propagate_closure_return(fid, local_id, &rvalue);
            }
            Stmt::Assign {
                value, binding_id, ..
            } => {
                let ast_id = binding_id
                    .get()
                    .ok_or_else(|| "Missing binding id while lowering assignment".to_string())?;
                let binding_id = BindingId(ast_id);
                let operand = self.lower_expr(builder, value, binding_map)?;
                if let Some(local_id) = binding_map.get(&binding_id) {
                    let local_id = *local_id;
                    let rvalue = Self::assignment_rvalue(builder, local_id, operand);
                    builder.push_instr(MirInstr::Assign(local_id, rvalue.clone()))?;
                    Self::retain_if_aliasing_borrow(builder, local_id, &rvalue)?;
                    let fid = builder.function.id.0;
                    self.propagate_closure_return(fid, local_id, &rvalue);
                } else if let Some(env_ptr) = self.current_env_ptr {
                    if let Some(idx) = self
                        .current_captures
                        .iter()
                        .position(|&cid| cid == binding_id)
                    {
                        builder.push_instr(MirInstr::AssignField {
                            base: env_ptr,
                            field: format!("cap_{}", idx),
                            value: operand,
                        })?;
                    } else {
                        return Err(format!(
                            "Missing MIR local or capture for binding {}",
                            ast_id
                        ));
                    }
                } else {
                    return Err(format!("Missing MIR local for binding {}", ast_id));
                }
            }
            Stmt::AssignField { base, field, value } => {
                let base_op = self.lower_expr(builder, base, binding_map)?;
                let value_op = self.lower_expr(builder, value, binding_map)?;
                if let Operand::Local(base_id) | Operand::Borrowed(base_id) = base_op {
                    builder.push_instr(MirInstr::AssignField {
                        base: base_id,
                        field: field.clone(),
                        value: value_op,
                    })?;
                } else {
                    return Err("Field assignment base is not a local variable".to_string());
                }
            }
            Stmt::Expr(expr) => {
                self.lower_expr(builder, expr, binding_map)?;
            }
            Stmt::Return(expr) => {
                let op = match expr {
                    Some(expr) => Some(self.lower_expr(builder, expr, binding_map)?),
                    None => None,
                };
                // Function ownership contract: custom structs and closure
                // capsules are returned *owned*. Returning an owned local moves
                // its reference. Returning a borrowed parameter/field first
                // retains it, thereby creating the caller's return reference.
                // `Str` is in this set now that string locals are owned. Without
                // it, `return str_substr(s, b, e - b)` lowered to a plain
                // `Return`, so the ARC pass released the string it was about to
                // hand back and the caller read freed memory -- ASan caught this
                // as a use-after-free in lppsqlite's `trim`.
                // The decision is the UNION of two inputs, not the type shape
                // alone:
                //
                //   owned = type_is_managed(local) || escape_analysis_says_arc(local)
                //
                // Escape analysis is a real participant here now -- Rule 1 in
                // `analysis::escape` promotes exactly the bindings that are
                // returned, and this is where that classification finally
                // reaches codegen.
                //
                // It is a union rather than a replacement on purpose. The type
                // check is conservative in the safe direction: it can only
                // over-approximate ownership, which costs a refcount, never
                // correctness. The escape walker is a recursive descent over
                // ~40 AST forms and I cannot prove it exhaustive, so deleting
                // the type check would make a missed match arm into a dangling
                // pointer. Both are compile-time predicates, so the union is
                // free at run time.
                let managed_return = match &op {
                    Some(Operand::Local(local)) | Some(Operand::Borrowed(local)) => {
                        let decl = &builder.function.locals[local.0];
                        let type_says_managed = decl.ty.is_managed();
                        type_says_managed.then_some(*local)
                    }
                    _ => None,
                };
                let terminator = if let Some(local) = managed_return {
                    if builder.function.locals[local.0].ownership.is_borrowed() {
                        builder.push_instr(MirInstr::Retain(local))?;
                    }
                    self.release_arena_if_live(builder)?;
                    Terminator::ReturnOwned(Operand::Local(local))
                } else {
                    self.release_arena_if_live(builder)?;
                    Terminator::Return(op)
                };
                builder.terminate_current_block(terminator)?;
                let next = builder.new_block();
                builder.switch_to_block(next);
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                let cond_op = self.lower_expr(builder, condition, binding_map)?;
                let then_block_id = builder.new_block();
                let else_block_id = builder.new_block();
                let merge_block_id = builder.new_block();

                builder.terminate_current_block(Terminator::If {
                    cond: cond_op,
                    then_block: then_block_id,
                    else_block: if else_block.is_some() {
                        else_block_id
                    } else {
                        merge_block_id
                    },
                })?;

                builder.switch_to_block(then_block_id);
                for stmt in then_block {
                    self.lower_stmt(builder, stmt, binding_map)?;
                }
                if builder.current_block().is_ok() {
                    builder.terminate_current_block(Terminator::Goto(merge_block_id))?;
                }

                if let Some(else_block) = else_block {
                    builder.switch_to_block(else_block_id);
                    for stmt in else_block {
                        self.lower_stmt(builder, stmt, binding_map)?;
                    }
                    if builder.current_block().is_ok() {
                        builder.terminate_current_block(Terminator::Goto(merge_block_id))?;
                    }
                }

                builder.switch_to_block(merge_block_id);
            }
            Stmt::While { condition, body, .. } => {
                let cond_block_id = builder.new_block();
                let body_block_id = builder.new_block();
                let end_block_id = builder.new_block();

                builder.terminate_current_block(Terminator::Goto(cond_block_id))?;

                builder.switch_to_block(cond_block_id);
                let cond_op = self.lower_expr(builder, condition, binding_map)?;
                builder.terminate_current_block(Terminator::If {
                    cond: cond_op,
                    then_block: body_block_id,
                    else_block: end_block_id,
                })?;

                self.loop_stack.push(LoopTargets {
                    break_block: end_block_id,
                    continue_block: cond_block_id,
                });

                builder.switch_to_block(body_block_id);
                for stmt in body {
                    self.lower_stmt(builder, stmt, binding_map)?;
                }
                if builder.current_block().is_ok() {
                    builder.terminate_current_block(Terminator::Goto(cond_block_id))?;
                }

                self.loop_stack.pop();

                builder.switch_to_block(end_block_id);
            }
            Stmt::ForRange {
                var_name,
                start,
                end,
                step,
                body,
                binding_id,
                ..
            } => {
                let start_op = self.lower_expr(builder, start, binding_map)?;
                let end_op = self.lower_expr(builder, end, binding_map)?;

                let var_binding_id = binding_id.get().map(BindingId);
                let var_local = builder.new_local(TypeRef::Int, true, Some(var_name.clone()), var_binding_id);
                if let Some(bid) = var_binding_id {
                    binding_map.insert(bid, var_local);
                }
                builder.push_instr(MirInstr::Assign(var_local, Rvalue::Use(start_op)))?;

                let cond_block_id = builder.new_block();
                let body_block_id = builder.new_block();
                let step_block_id = builder.new_block();
                let end_block_id = builder.new_block();

                builder.terminate_current_block(Terminator::Goto(cond_block_id))?;

                builder.switch_to_block(cond_block_id);
                let cmp_temp = builder.new_local(TypeRef::Bool, false, None, None);
                builder.push_instr(MirInstr::Assign(
                    cmp_temp,
                    Rvalue::BinaryOp(BinaryOperator::Less, Operand::Local(var_local), end_op),
                ))?;
                builder.terminate_current_block(Terminator::If {
                    cond: Operand::Local(cmp_temp),
                    then_block: body_block_id,
                    else_block: end_block_id,
                })?;

                self.loop_stack.push(LoopTargets {
                    break_block: end_block_id,
                    continue_block: step_block_id,
                });

                builder.switch_to_block(body_block_id);
                for stmt in body {
                    self.lower_stmt(builder, stmt, binding_map)?;
                }
                if builder.current_block().is_ok() {
                    builder.terminate_current_block(Terminator::Goto(step_block_id))?;
                }

                builder.switch_to_block(step_block_id);
                let step_val = if let Some(step_expr) = step {
                    self.lower_expr(builder, step_expr, binding_map)?
                } else {
                    Operand::Int(1)
                };
                let add_temp = builder.new_local(TypeRef::Int, false, None, None);
                builder.push_instr(MirInstr::Assign(
                    add_temp,
                    Rvalue::BinaryOp(
                        BinaryOperator::Add,
                        Operand::Local(var_local),
                        step_val,
                    ),
                ))?;
                builder.push_instr(MirInstr::Assign(var_local, Rvalue::Use(Operand::Local(add_temp))))?;
                builder.terminate_current_block(Terminator::Goto(cond_block_id))?;

                self.loop_stack.pop();
                builder.switch_to_block(end_block_id);
            }
            Stmt::ForIn {
                var_name,
                list,
                body,
                binding_id,
                ..
            } => {
                let list_op = self.lower_expr(builder, list, binding_map)?;
                let list_ty = self.expr_type_hint(list, builder, binding_map);
                let elem_ty = match &list_ty {
                    TypeRef::Generic(name, params) if name == "List" && !params.is_empty() => {
                        params[0].clone()
                    }
                    _ => TypeRef::Int,
                };

                let list_local = builder.new_local(list_ty, false, Some("__for_list".to_string()), None);
                let list_rvalue = Self::assignment_rvalue(builder, list_local, list_op);
                builder.push_instr(MirInstr::Assign(list_local, list_rvalue.clone()))?;
                Self::retain_if_aliasing_borrow(builder, list_local, &list_rvalue)?;

                let idx_local = builder.new_local(TypeRef::Int, true, Some("__for_idx".to_string()), None);
                builder.push_instr(MirInstr::Assign(idx_local, Rvalue::Use(Operand::Int(0))))?;

                let cond_block_id = builder.new_block();
                let body_block_id = builder.new_block();
                let step_block_id = builder.new_block();
                let end_block_id = builder.new_block();

                builder.terminate_current_block(Terminator::Goto(cond_block_id))?;

                builder.switch_to_block(cond_block_id);
                let len_temp = builder.new_local(TypeRef::Int, false, None, None);
                builder.push_instr(MirInstr::Assign(
                    len_temp,
                    Rvalue::BuiltinCall("lpp_list_len".to_string(), vec![Operand::Local(list_local)]),
                ))?;
                let cmp_temp = builder.new_local(TypeRef::Bool, false, None, None);
                builder.push_instr(MirInstr::Assign(
                    cmp_temp,
                    Rvalue::BinaryOp(
                        BinaryOperator::Less,
                        Operand::Local(idx_local),
                        Operand::Local(len_temp),
                    ),
                ))?;
                builder.terminate_current_block(Terminator::If {
                    cond: Operand::Local(cmp_temp),
                    then_block: body_block_id,
                    else_block: end_block_id,
                })?;

                self.loop_stack.push(LoopTargets {
                    break_block: end_block_id,
                    continue_block: step_block_id,
                });

                builder.switch_to_block(body_block_id);
                let element_class = elem_ty.list_element_class();
                let get_symbol = list_get_symbol(&elem_ty)?;
                let elem_temp = builder.new_local(elem_ty.clone(), false, None, None);
                if element_class == ListElementClass::Arc {
                    builder.set_local_ownership(elem_temp, Ownership::Borrowed);
                }
                builder.push_instr(MirInstr::Assign(
                    elem_temp,
                    Rvalue::BuiltinCall(
                        get_symbol.to_string(),
                        vec![Operand::Local(list_local), Operand::Local(idx_local)],
                    ),
                ))?;

                let var_binding_id = binding_id.get().map(BindingId);
                let var_local = builder.new_local(elem_ty, false, Some(var_name.clone()), var_binding_id);
                if element_class == ListElementClass::Arc {
                    // The loop variable only borrows the list's element edge
                    // (same model as `list_get` on List[ARC]); assignment or
                    // `return` out of it retains explicitly. Marking it Owned
                    // here released every element at each iteration's end and
                    // the list's destructor released the dead pieces again.
                    builder.set_local_ownership(var_local, Ownership::Borrowed);
                }
                if let Some(bid) = var_binding_id {
                    binding_map.insert(bid, var_local);
                }
                builder.push_instr(MirInstr::Assign(var_local, Rvalue::Use(Operand::Local(elem_temp))))?;

                for stmt in body {
                    self.lower_stmt(builder, stmt, binding_map)?;
                }
                if builder.current_block().is_ok() {
                    builder.terminate_current_block(Terminator::Goto(step_block_id))?;
                }

                builder.switch_to_block(step_block_id);
                let add_temp = builder.new_local(TypeRef::Int, false, None, None);
                builder.push_instr(MirInstr::Assign(
                    add_temp,
                    Rvalue::BinaryOp(
                        BinaryOperator::Add,
                        Operand::Local(idx_local),
                        Operand::Int(1),
                    ),
                ))?;
                builder.push_instr(MirInstr::Assign(idx_local, Rvalue::Use(Operand::Local(add_temp))))?;
                builder.terminate_current_block(Terminator::Goto(cond_block_id))?;

                self.loop_stack.pop();
                builder.switch_to_block(end_block_id);
            }
            Stmt::Break => {
                let target = self
                    .loop_stack
                    .last()
                    .ok_or_else(|| "break statement outside of loop".to_string())?
                    .break_block;
                builder.terminate_current_block(Terminator::Goto(target))?;
                let dead_block = builder.new_block();
                builder.switch_to_block(dead_block);
            }
            Stmt::Continue => {
                let target = self
                    .loop_stack
                    .last()
                    .ok_or_else(|| "continue statement outside of loop".to_string())?
                    .continue_block;
                builder.terminate_current_block(Terminator::Goto(target))?;
                let dead_block = builder.new_block();
                builder.switch_to_block(dead_block);
            }
            Stmt::Block(stmts) => {
                for stmt in stmts {
                    self.lower_stmt(builder, stmt, binding_map)?;
                }
            }
            Stmt::Match { subject, arms } => {
                // Enums are heap objects: `__tag` plus one payload slot per
                // variant. Each arm reads its own slot, so the payload keeps
                // its real type and full width.
                let subject_val = self.lower_expr(builder, subject, binding_map)?;
                let subject_ty = match &subject_val {
                    Operand::Local(id) | Operand::Borrowed(id) => {
                        builder.function.locals[id.0].ty.clone()
                    }
                    _ => TypeRef::Int,
                };
                let enum_struct_id = match subject_ty {
                    TypeRef::Custom(id) => Some(id),
                    _ => None,
                };

                let tag_val = builder.new_local(TypeRef::Int, false, None, None);
                if enum_struct_id.is_some() {
                    builder.push_instr(MirInstr::Assign(
                        tag_val,
                        Rvalue::FieldAccess(subject_val.clone(), "__tag".to_string()),
                    ))?;
                } else {
                    // Legacy scalar form, still used when the subject's type is
                    // not a known enum object.
                    let shift_const = builder.new_local(TypeRef::Int, false, None, None);
                    builder.push_instr(MirInstr::Assign(
                        shift_const,
                        Rvalue::Use(Operand::Int(4294967296)),
                    ))?;
                    builder.push_instr(MirInstr::Assign(
                        tag_val,
                        Rvalue::BinaryOp(
                            BinaryOperator::Divide,
                            subject_val.clone(),
                            Operand::Local(shift_const),
                        ),
                    ))?;
                }

                let end_block = builder.new_block();

                for (i, arm) in arms.iter().enumerate() {
                    let arm_block = builder.new_block();
                    let next_block = if i + 1 < arms.len() {
                        builder.new_block()
                    } else {
                        end_block
                    };

                    // Compare tag
                    let expected_tag = builder.new_local(TypeRef::Int, false, None, None);
                    builder.push_instr(MirInstr::Assign(expected_tag, Rvalue::Use(Operand::Int(i as i64))))?;
                    let cmp_local = builder.new_local(TypeRef::Bool, false, None, None);
                    builder.push_instr(MirInstr::Assign(
                        cmp_local,
                        Rvalue::BinaryOp(BinaryOperator::Eq, Operand::Local(tag_val), Operand::Local(expected_tag)),
                    ))?;
                    builder.terminate_current_block(Terminator::If {
                        cond: Operand::Local(cmp_local),
                        then_block: arm_block,
                        else_block: next_block,
                    })?;

                    // Arm body — bind this variant's payload slot, at its real
                    // type, so a Str stays a Str instead of a truncated int.
                    builder.switch_to_block(arm_block);
                    if !arm.bindings.is_empty() {
                        let binding_name = &arm.bindings[0];
                        let field = format!("__v{}", i);
                        let payload_ty = enum_struct_id
                            .and_then(|sid| {
                                self.type_table.definitions[sid.0]
                                    .fields
                                    .iter()
                                    .find(|(n, _)| *n == field)
                                    .map(|(_, t)| t.clone())
                            })
                            .unwrap_or(TypeRef::Int);
                        let bound_local = builder.new_local(payload_ty.clone(), true, None, None);
                        if enum_struct_id.is_some() {
                            // Reading a field borrows the container's ARC edge.
                            if payload_ty.is_managed() {
                                builder.set_local_ownership(bound_local, Ownership::Borrowed);
                            }
                            builder.push_instr(MirInstr::Assign(
                                bound_local,
                                Rvalue::FieldAccess(subject_val.clone(), field),
                            ))?;
                        } else {
                            let shift_const = builder.new_local(TypeRef::Int, false, None, None);
                            builder.push_instr(MirInstr::Assign(
                                shift_const,
                                Rvalue::Use(Operand::Int(4294967296)),
                            ))?;
                            builder.push_instr(MirInstr::Assign(
                                bound_local,
                                Rvalue::BinaryOp(
                                    BinaryOperator::Modulo,
                                    subject_val.clone(),
                                    Operand::Local(shift_const),
                                ),
                            ))?;
                        }
                        self.match_bindings.insert(binding_name.clone(), bound_local);
                    }
                    for stmt in &arm.body {
                        self.lower_stmt(builder, stmt, binding_map)?;
                    }
                    if !arm.bindings.is_empty() {
                        self.match_bindings.remove(&arm.bindings[0]);
                    }
                    builder.terminate_current_block(Terminator::Goto(end_block))?;

                    if i + 1 < arms.len() {
                        builder.switch_to_block(next_block);
                    }
                }

                builder.switch_to_block(end_block);
            }
        }
        Ok(())
    }

    fn lower_expr(
        &mut self,
        builder: &mut MirBuilder,
        expr: &Expr,
        binding_map: &mut HashMap<BindingId, LocalId>,
    ) -> Result<Operand, String> {
        match expr {
            Expr::IntLiteral(value) => Ok(Operand::Int(*value)),
            Expr::FloatLiteral(value) => Ok(Operand::Float(*value)),
            Expr::StringLiteral(value) => Ok(Operand::String(value.clone())),
            Expr::CharLiteral(ch) => Ok(Operand::Int(*ch as i64)),
            Expr::BoolLiteral(value) => Ok(Operand::Bool(*value)),
            Expr::Tuple(elements) => {
                let types: Vec<TypeRef> = elements
                    .iter()
                    .map(|element| self.expr_type_hint(element, builder, binding_map))
                    .collect();
                let mut values = Vec::with_capacity(elements.len());
                for element in elements {
                    let value = self.lower_expr(builder, element, binding_map)?;
                    if let Operand::Borrowed(source) = value {
                        if builder.function.locals[source.0].ty.is_managed() {
                            // The tuple creates an owning edge from a borrow.
                            builder.push_instr(MirInstr::Retain(source))?;
                        }
                        values.push(Operand::Borrowed(source));
                    } else {
                        values.push(value);
                    }
                }
                let tuple_ty = TypeRef::Tuple(types.clone());
                let result = builder.new_local(tuple_ty, false, None, None);
                builder.push_instr(MirInstr::Assign(
                    result,
                    Rvalue::AllocateTuple(types, values),
                ))?;
                Ok(Operand::Local(result))
            }
            Expr::Await(task) => {
                let task_operand = self.lower_expr(builder, task, binding_map)?;
                let result_ty = match &task_operand {
                    Operand::Local(id) | Operand::Borrowed(id) =>
                        match &builder.function.locals[id.0].ty {
                            TypeRef::Task(result) => (**result).clone(),
                            other => return Err(format!("await reached MIR with non-task type {:?}", other)),
                        },
                    _ => return Err("await requires a task local".to_string()),
                };
                let result = builder.new_local(result_ty, false, None, None);
                builder.push_instr(MirInstr::Assign(result, Rvalue::Await(task_operand)))?;
                Ok(Operand::Local(result))
            }
            Expr::Identifier(name, cell) => {
                // Check constants first
                if let Some(&val) = self.constants.get(name) {
                    return Ok(Operand::Int(val));
                }
                // Check match arm bindings
                if let Some(&local_id) = self.match_bindings.get(name) {
                    return Ok(Operand::Local(local_id));
                }
                let ast_id = match cell.get() {
                    Some(id) => id,
                    None => {
                        if crate::builtins::get_builtins().iter().any(|b| b.name == name)
                            || self.functions.contains_key(name)
                            || self.extern_symbols.contains_key(name)
                        {
                            return Ok(Operand::Int(0));
                        }
                        return Err(format!("Missing binding id for identifier '{}'", name));
                    }
                };
                let binding_id = BindingId(ast_id);
                if let Some(local_id) = binding_map.get(&binding_id) {
                    let local = &builder.function.locals[local_id.0];
                    if local.ty.is_managed() {
                        // Identifier reads borrow managed objects, including
                        // caller-owned parameters whose MIR ownership is
                        // explicitly `Borrowed`. A later ownership operation
                        // decides whether to retain or move the value.
                        Ok(Operand::Borrowed(*local_id))
                    } else {
                        Ok(Operand::Local(*local_id))
                    }
                } else if let Some(env_ptr) = self.current_env_ptr {
                    if let Some(idx) = self
                        .current_captures
                        .iter()
                        .position(|&cid| cid == binding_id)
                    {
                        let cap_ty = self.symbol_table.bindings[binding_id.0]
                            .ty
                            .clone()
                            .unwrap_or(TypeRef::Int);
                        let temp = builder.new_local(
                            cap_ty.clone(),
                            false,
                            Some(format!("cap_val_{}", name)),
                            None,
                        );
                        // A captured custom value is borrowed from the closure
                        // environment; the environment owns the ARC edge.
                        if cap_ty.is_managed() {
                            builder.set_local_ownership(temp, Ownership::Borrowed);
                        }
                        builder.push_instr(MirInstr::Assign(
                            temp,
                            Rvalue::FieldAccess(Operand::Local(env_ptr), format!("cap_{}", idx)),
                        ))?;
                        if builder.function.locals[temp.0].ownership.is_borrowed() {
                            Ok(Operand::Borrowed(temp))
                        } else {
                            Ok(Operand::Local(temp))
                        }
                    } else {
                        Err(format!(
                            "Identifier '{}' (binding {}) was not mapped in locals or captures of '{}'",
                            name, ast_id, builder.function.name
                        ))
                    }
                } else {
                    Err(format!(
                        "Identifier '{}' (binding {}) was not mapped into MIR locals for '{}'",
                        name, ast_id, builder.function.name
                    ))
                }
            }
            Expr::UnaryOp { op, operand } => {
                let val = self.lower_expr(builder, operand, binding_map)?;
                match op {
                    UnaryOperator::Negate => {
                        // -x = 0 - x
                        let zero = builder.new_local(TypeRef::Int, false, None, None);
                        builder.push_instr(MirInstr::Assign(zero, Rvalue::Use(Operand::Int(0))))?;
                        let result = builder.new_local(TypeRef::Int, false, None, None);
                        builder.push_instr(MirInstr::Assign(
                            result,
                            Rvalue::BinaryOp(BinaryOperator::Subtract, Operand::Local(zero), val),
                        ))?;
                        Ok(Operand::Local(result))
                    }
                    UnaryOperator::Not => {
                        // !b = b == false
                        let false_val = builder.new_local(TypeRef::Bool, false, None, None);
                        builder.push_instr(MirInstr::Assign(false_val, Rvalue::Use(Operand::Bool(false))))?;
                        let result = builder.new_local(TypeRef::Bool, false, None, None);
                        builder.push_instr(MirInstr::Assign(
                            result,
                            Rvalue::BinaryOp(BinaryOperator::Eq, val, Operand::Local(false_val)),
                        ))?;
                        Ok(Operand::Local(result))
                    }
                }
            }
            Expr::BinaryOp { left, op, right } => {
                // `a + b` on two Str values means concatenation. The type
                // checker allows Add on any two matchingng types, so this reached
                // the backend and emitted `iadd` on two pointers: it compiled
                // clean, then segfaulted with no diagnostic at all.
                if *op == BinaryOperator::Add {
                    let lt = self.expr_type_hint(left, builder, binding_map);
                    let rt = self.expr_type_hint(right, builder, binding_map);
                    if lt == TypeRef::Str && rt == TypeRef::Str {
                        let desugared = Expr::Call {
                            callee: Box::new(Expr::Identifier(
                                "str_concat".to_string(),
                                std::cell::Cell::new(None),
                            )),
                            args: vec![(**left).clone(), (**right).clone()],
                        };
                        return self.lower_expr(builder, &desugared, binding_map);
                    }
                }
                // Short-circuit && and ||
                if *op == BinaryOperator::And {
                    let result = builder.new_local(TypeRef::Bool, true, None, None);
                    let left_val = self.lower_expr(builder, left, binding_map)?;
                    let eval_right = builder.new_block();
                    let end_block = builder.new_block();
                    // If left is false, result = false, skip right
                    builder.push_instr(MirInstr::Assign(result, Rvalue::Use(Operand::Bool(false))))?;
                    builder.terminate_current_block(Terminator::If {
                        cond: left_val,
                        then_block: eval_right,
                        else_block: end_block,
                    })?;
                    // Left was true — evaluate right
                    builder.switch_to_block(eval_right);
                    let right_val = self.lower_expr(builder, right, binding_map)?;
                    builder.push_instr(MirInstr::Assign(result, Rvalue::Use(right_val)))?;
                    builder.terminate_current_block(Terminator::Goto(end_block))?;
                    builder.switch_to_block(end_block);
                    return Ok(Operand::Local(result));
                }
                if *op == BinaryOperator::Or {
                    let result = builder.new_local(TypeRef::Bool, true, None, None);
                    let left_val = self.lower_expr(builder, left, binding_map)?;
                    let eval_right = builder.new_block();
                    let end_block = builder.new_block();
                    // If left is true, result = true, skip right
                    builder.push_instr(MirInstr::Assign(result, Rvalue::Use(Operand::Bool(true))))?;
                    builder.terminate_current_block(Terminator::If {
                        cond: left_val,
                        then_block: end_block,
                        else_block: eval_right,
                    })?;
                    // Left was false — evaluate right
                    builder.switch_to_block(eval_right);
                    let right_val = self.lower_expr(builder, right, binding_map)?;
                    builder.push_instr(MirInstr::Assign(result, Rvalue::Use(right_val)))?;
                    builder.terminate_current_block(Terminator::Goto(end_block))?;
                    builder.switch_to_block(end_block);
                    return Ok(Operand::Local(result));
                }

                let left_ty = self.expr_type_hint(left, builder, binding_map);
                let left = self.lower_expr(builder, left, binding_map)?;
                let right = self.lower_expr(builder, right, binding_map)?;
                let res_ty = match op {
                    BinaryOperator::Eq
                    | BinaryOperator::NotEq
                    | BinaryOperator::Less
                    | BinaryOperator::LessEq
                    | BinaryOperator::Greater
                    | BinaryOperator::GreaterEq => TypeRef::Bool,
                    _ => left_ty,
                };
                let temp = builder.new_local(res_ty, false, None, None);
                builder.push_instr(MirInstr::Assign(
                    temp,
                    Rvalue::BinaryOp(op.clone(), left, right),
                ))?;
                Ok(Operand::Local(temp))
            }
            Expr::Call { callee, args } => {
                // Borrowed slice operations have explicit MIR forms so escape
                // validation and both backends see the lifetime boundary.
                if let Expr::Identifier(name, _) = &**callee {
                    match name.as_str() {
                        "str_slice" | "slice" => {
                            if args.len() != 3 {
                                return Err(format!("{} expects exactly three arguments", name));
                            }
                            let base = self.lower_expr(builder, &args[0], binding_map)?;
                            let start = self.lower_expr(builder, &args[1], binding_map)?;
                            let length = self.lower_expr(builder, &args[2], binding_map)?;
                            let view_ty = if name == "str_slice" {
                                TypeRef::StrSlice
                            } else {
                                match self.expr_type_hint(&args[0], builder, binding_map) {
                                    TypeRef::Generic(list, elements)
                                        if list == "List" && elements.len() == 1 =>
                                        TypeRef::Slice(Box::new(elements[0].clone())),
                                    _ => TypeRef::Slice(Box::new(TypeRef::Int)),
                                }
                            };
                            let view = builder.new_local(view_ty, false, None, None);
                            builder.set_local_ownership(view, Ownership::Borrowed);
                            builder.push_instr(MirInstr::Assign(
                                view,
                                Rvalue::MakeSlice {
                                    base,
                                    start,
                                    length,
                                    kind: if name == "str_slice" { 0 } else { 1 },
                                },
                            ))?;
                            return Ok(Operand::Borrowed(view));
                        }
                        "slice_len" => {
                            let view_op = self.lower_expr(builder, &args[0], binding_map)?;
                            let result = builder.new_local(TypeRef::Int, false, None, None);
                            builder.push_instr(MirInstr::Assign(result, Rvalue::SliceLen(view_op)))?;
                            return Ok(Operand::Local(result));
                        }
                        "slice_get" => {
                            let view_ty = self.expr_type_hint(&args[0], builder, binding_map);
                            let result_ty = match &view_ty {
                                TypeRef::Slice(element) => (**element).clone(),
                                TypeRef::StrSlice => TypeRef::Str,
                                _ => TypeRef::Int,
                            };
                            let view_op = self.lower_expr(builder, &args[0], binding_map)?;
                            let index_op = self.lower_expr(builder, &args[1], binding_map)?;
                            let result = builder.new_local(result_ty.clone(), false, None, None);
                            if matches!(view_ty, TypeRef::Slice(element) if element.is_managed()) {
                                builder.set_local_ownership(result, Ownership::Borrowed);
                            }
                            builder.push_instr(MirInstr::Assign(
                                result,
                                Rvalue::SliceGet(view_op, index_op),
                            ))?;
                            return Ok(if builder.function.locals[result.0].ownership.is_borrowed() {
                                Operand::Borrowed(result)
                            } else {
                                Operand::Local(result)
                            });
                        }
                        "slice_to_str" | "str_slice_to_str" => {
                            let view_op = self.lower_expr(builder, &args[0], binding_map)?;
                            let result = builder.new_local(TypeRef::Str, false, None, None);
                            builder.push_instr(MirInstr::Assign(result, Rvalue::SliceToStr(view_op)))?;
                            return Ok(Operand::Local(result));
                        }
                        _ => {}
                    }
                }

                // Fill in default parameter values if fewer args than params
                let mut effective_args: Vec<&Expr> = args.iter().collect();
                if let Expr::Identifier(name, _) = &**callee {
                    let func_def = self.program.declarations.iter().find_map(|d| {
                        if let TopLevel::Function(f) = d {
                            if &f.name == name { Some(f) } else { None }
                        } else { None }
                    });
                    if let Some(f) = func_def {
                        if args.len() < f.params.len() {
                            // Use references directly instead of cloning
                            for i in args.len()..f.params.len() {
                                if let Some(ref default_expr) = f.params[i].default {
                                    effective_args.push(default_expr);
                                }
                            }
                        }
                    }
                }

                let mut lowered_args = Vec::new();
                let variadic_spec = if let Expr::Identifier(name, _) = &**callee {
                    self.program.declarations.iter().find_map(|decl| match decl {
                        TopLevel::Function(function) if &function.name == name =>
                            function.params.last().and_then(|param| {
                                param.variadic.then(|| (
                                    function.params.len() - 1,
                                    self.resolve_type(&param.ty),
                                ))
                            }),
                        TopLevel::Impl(block) => block.methods.iter().find(|function| &function.name == name)
                            .and_then(|function| function.params.last().and_then(|param| {
                                param.variadic.then(|| (
                                    function.params.len() - 1,
                                    self.resolve_type(&param.ty),
                                ))
                            })),
                        _ => None,
                    })
                } else {
                    None
                };

                if let Some((fixed_count, element_ty)) = variadic_spec {
                    if args.len() < fixed_count {
                        return Err(format!("variadic call requires at least {} fixed arguments", fixed_count));
                    }
                    for arg in args.iter().take(fixed_count) {
                        lowered_args.push(self.lower_expr(builder, arg, binding_map)?);
                    }
                    let rest = builder.new_local(
                        TypeRef::Generic("List".to_string(), vec![element_ty.clone()]),
                        false,
                        Some("__rest".to_string()),
                        None,
                    );
                    builder.push_instr(MirInstr::Assign(
                        rest,
                        Rvalue::AllocateList(element_ty.clone()),
                    ))?;
                    let push_symbol = list_push_symbol(&element_ty)?;
                    for arg in args.iter().skip(fixed_count) {
                        let value = self.lower_expr(builder, arg, binding_map)?;
                        let discard = builder.new_local(TypeRef::Void, false, None, None);
                        builder.push_instr(MirInstr::Assign(
                            discard,
                            Rvalue::BuiltinCall(
                                push_symbol.to_string(),
                                vec![Operand::Borrowed(rest), value],
                            ),
                        ))?;
                    }
                    lowered_args.push(Operand::Local(rest));
                } else {
                    for arg in &effective_args {
                        lowered_args.push(self.lower_expr(builder, arg, binding_map)?);
                    }
                }

                let mut return_type = TypeRef::Void;
                if let Expr::Identifier(name, callee_cell) = &**callee {
                    if (name == "map_get" || name == "lpp_map_get") && !args.is_empty() {
                        let map_ty = self.expr_type_hint(&args[0], builder, binding_map);
                        if let TypeRef::Generic(_, ref params) = map_ty {
                            if params.len() >= 2 {
                                return_type = params[1].clone();
                            }
                        }
                    }
                    if return_type == TypeRef::Void {
                        if let Some(ty) = self.func_return_types.get(name) {
                            return_type = ty.clone();
                            // Generic type inference at MIR level: if return type is TypeParam,
                            // infer from the argument types
                            if let TypeRef::TypeParam(ref tp_name) = return_type {
                                // Find the function AST to get param type info
                                for decl in &self.program.declarations {
                                    if let TopLevel::Function(f) = decl {
                                        if &f.name == name {
                                            let prev = std::mem::replace(&mut self.current_type_params, f.type_params.iter().map(|tp| tp.name.clone()).collect());
                                            for (i, param) in f.params.iter().enumerate() {
                                                let param_resolved = self.resolve_type(&param.ty);
                                                if let TypeRef::TypeParam(ref pn) = param_resolved {
                                                    if pn == tp_name && i < args.len() {
                                                        return_type = self.expr_type_hint(&args[i], builder, binding_map);
                                                        break;
                                                    }
                                                }
                                            }
                                            self.current_type_params = prev;
                                            break;
                                        }
                                    }
                                }
                            }
                        } else if let Some(&struct_id) = self.type_table.structs_by_name.get(name) {
                            return_type = TypeRef::Custom(struct_id);
                        } else if let Some(builtin) = crate::builtins::get_builtins()
                            .iter()
                            .find(|b| b.name == name)
                        {
                            if name == "list_get" || name == "lpp_list_get" || name == "get" {
                                let list_ty = args
                                    .first()
                                    .map(|arg| self.expr_type_hint(arg, builder, binding_map))
                                    .unwrap_or(TypeRef::Int);
                                if let TypeRef::Generic(_, params) = list_ty {
                                    if let Some(element_ty) = params.first() {
                                        return_type = element_ty.clone();
                                    }
                                }
                            } else if name != "list_new" {
                                return_type = builtin.return_type.clone();
                            } else {
                                // list_new is special-cased because of generic list type inference
                                return_type = TypeRef::Generic("List".to_string(), vec![TypeRef::Int]);
                            }
                        } else {
                            return_type = match name.as_str() {
                                "input" | "read_file" | "json_get_str" | "net_recv"
                                | "net_recv_udp" | "net_resolve" | "http_get" | "http_post"
                                | "command_output" | "env_get" | "str_concat" | "str_replace"
                                | "str_substr" | "str_trim" | "path_join" => TypeRef::Str,
                                "parse_int" | "json_parse" | "json_get_int" | "json_get_obj"
                                | "list_get" | "list_len" | "get" | "net_connect" | "net_listen"
                                | "net_listen_udp" | "net_accept" | "net_accept_timeout"
                                | "net_send" | "net_send_all" | "net_dial" | "net_dial_udp"
                                | "net_set_timeout" | "net_set_deadline" | "net_set_keepalive"
                                | "net_set_nonblocking" | "net_poll"
                                | "command_exec" | "str_find" | "str_split" | "dir_create"
                                | "dir_remove" | "path_exists" | "file_copy" | "file_move"
                                | "delete_file" | "append_file" | "file_size" | "file_exists"
                                | "env_set" => TypeRef::Int,
                                "list_new" => TypeRef::Generic("List".to_string(), vec![TypeRef::Int]),
                                "print" | "print_str" | "json_free" | "list_push" | "list_free"
                                | "net_close" => TypeRef::Void,
                                _ => {
                                    // Closure through a variable: the local carries
                                    // `TypeRef::Function` with no signature, so use the
                                    // return type recorded when that closure value was
                                    // constructed instead of the Int fallback below.
                                    let mut closure_ret: Option<TypeRef> = None;
                                    if let Some(bid) = callee_cell.get() {
                                        if let Some(&lid) = binding_map.get(&BindingId(bid)) {
                                            closure_ret = self
                                                .closure_returns
                                                .get(&(builder.function.id.0, lid.0))
                                                .cloned();
                                        }
                                    }
                                    if closure_ret.is_none() {
                                        if let Expr::Identifier(var_name, _) = &**callee {
                                            if let Some(&lid) = self.symbol_table.scopes.iter().rev().find_map(|s| s.bindings.get(var_name)) {
                                                closure_ret = self
                                                    .closure_returns
                                                    .get(&(builder.function.id.0, lid.0))
                                                    .cloned();
                                            }
                                        }
                                    }
                                    match closure_ret {
                                        Some(rt) => rt,
                                        None => {
                                            // Try trait method: infer receiver type, look up StructName_method
                                            let mut trait_ret = TypeRef::Int;
                                            if !effective_args.is_empty() {
                                                let recv_ty = self.expr_type_hint(&effective_args[0], builder, binding_map);
                                                if let TypeRef::Custom(sid) = &recv_ty {
                                                    let sname = self.type_table.definitions[sid.0].name.clone();
                                                    let mangled = format!("{}_{}", sname, name);
                                                    if let Some(rt) = self.func_return_types.get(&mangled) {
                                                        trait_ret = rt.clone();
                                                    }
                                                }
                                            }
                                            trait_ret
                                        }
                                    }
                                },
                            };
                        }
                    }
                } else {
                    return_type = TypeRef::Int;
                }

                let is_async_call = matches!(
                    &**callee,
                    Expr::Identifier(name, _) if self.async_functions.contains(name)
                );
                if is_async_call {
                    return_type = TypeRef::Task(Box::new(return_type));
                }
                let list_get_borrows_element = matches!(
                    &**callee,
                    Expr::Identifier(name, _) if name == "list_get" || name == "lpp_list_get" || name == "get"
                ) && return_type.is_managed();
                let temp = builder.new_local(return_type, false, None, None);
                if list_get_borrows_element {
                    // List[ARC] owns the element edge; get returns only a
                    // borrow. Assignment/return will retain explicitly.
                    builder.set_local_ownership(temp, Ownership::Borrowed);
                }

                if let Expr::Identifier(name, _) = &**callee {
                    // Try direct function lookup first
                    let mut resolved_func_id = self.functions.get(name).copied();

                    // Dynamic dispatch: if the first arg has a vtable for this method,
                    // use CallIndirect through the function pointer
                    if resolved_func_id.is_none() && !effective_args.is_empty() {
                        if let Expr::Identifier(recv_name, _) = &effective_args[0] {
                            if let Some((_tname, method_map)) = self.current_vtable_locals.get(recv_name) {
                                if let Some(&vtable_local) = method_map.get(name) {
                                    // Dynamic dispatch: call through function pointer
                                    let fptr = Operand::Local(vtable_local);
                                    builder.push_instr(MirInstr::Assign(
                                        temp,
                                        Rvalue::CallIndirect(fptr, lowered_args),
                                    ))?;
                                    return Ok(Operand::Local(temp));
                                }
                            }
                        }
                    }

                    // Static trait method dispatch: if not found, try StructName_method
                    if resolved_func_id.is_none() && !effective_args.is_empty() {
                        // Infer the type of the first argument (the receiver)
                        let receiver_type_name = self.expr_type_hint(&effective_args[0], builder, binding_map);

                        if let TypeRef::Custom(sid) = &receiver_type_name {
                            let struct_name = &self.type_table.definitions[sid.0].name;
                            let mangled = format!("{}_{}", struct_name, name);
                            if let Some(&fid) = self.functions.get(&mangled) {
                                resolved_func_id = Some(fid);
                                // Also fix return type
                                if let Some(rt) = self.func_return_types.get(&mangled) {
                                    builder.function.locals[temp.0].ty = rt.clone();
                                }
                            }
                        }
                    }

                    if let Some(func_id) = resolved_func_id {
                        // Dynamic dispatch: if the callee has trait-typed params,
                        // append function pointer args for each trait method
                        let callee_func = self.program.declarations.iter().find_map(|d| {
                            match d {
                                TopLevel::Function(f) if &f.name == name => Some(f),
                                TopLevel::Impl(ib) => ib.methods.iter().find(|m| &m.name == name),
                                _ => None,
                            }
                        });
                        if let Some(cf) = callee_func {
                            for (pi, param) in cf.params.iter().enumerate() {
                                if let Type::Custom(ref tname) = param.ty {
                                    if self.trait_names.contains(tname) {
                                        // This param is trait-typed. Find the concrete type of the arg.
                                        if pi < effective_args.len() {
                                            let concrete_ty = self.expr_type_hint(&effective_args[pi], builder, binding_map);
                                            if let TypeRef::Custom(sid) = &concrete_ty {
                                                let struct_name = self.type_table.definitions[sid.0].name.clone();
                                                // For each trait method, pass the FuncId as an i64
                                                if let Some(methods) = self.trait_defs.get(tname) {
                                                    for method_name in methods {
                                                        let mangled = format!("{}_{}", struct_name, method_name);
                                                        if let Some(&mfid) = self.functions.get(&mangled) {
                                                            let fptr_local = builder.new_local(TypeRef::Int, false, None, None);
                                                            builder.push_instr(MirInstr::Assign(
                                                                fptr_local,
                                                                Rvalue::FuncRef(mfid),
                                                            ))?;
                                                            lowered_args.push(Operand::Local(fptr_local));
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        if self.async_functions.contains(name) {
                            let result_ty = match &builder.function.locals[temp.0].ty {
                                TypeRef::Task(result) => (**result).clone(),
                                _ => return Err("async call lost its Task result type".to_string()),
                            };
                            let mut argument_types = Vec::with_capacity(lowered_args.len());
                            for argument in &lowered_args {
                                let ty = match argument {
                                    Operand::Local(id) | Operand::Borrowed(id) =>
                                        builder.function.locals[id.0].ty.clone(),
                                    Operand::Int(_) => TypeRef::Int,
                                    Operand::Float(_) => TypeRef::Float,
                                    Operand::String(_) => TypeRef::Str,
                                    Operand::Bool(_) => TypeRef::Bool,
                                };
                                argument_types.push(ty);
                            }
                            for argument in &lowered_args {
                                if let Operand::Borrowed(id) = argument {
                                    if builder.function.locals[id.0].ty.is_managed() {
                                        builder.push_instr(MirInstr::Retain(*id))?;
                                    }
                                }
                            }
                            builder.push_instr(MirInstr::Assign(
                                temp,
                                Rvalue::MakeTask(func_id, argument_types, lowered_args, result_ty),
                            ))?;
                        } else {
                            builder.push_instr(MirInstr::Assign(
                                temp,
                                Rvalue::CallDirect(func_id, lowered_args),
                            ))?;
                        }
                        return Ok(Operand::Local(temp));
                    }

                    if let Some(&struct_id) = self.type_table.structs_by_name.get(name) {
                        let allocation = self.struct_allocation(
                            builder,
                            TypeRef::Custom(struct_id),
                        )?;
                        builder.push_instr(MirInstr::Assign(temp, allocation))?;
                        if !lowered_args.is_empty() {
                            let struct_def = &self.type_table.definitions[struct_id.0];
                            for (i, val_op) in lowered_args.into_iter().enumerate() {
                                if i < struct_def.fields.len() {
                                    let field_name = &struct_def.fields[i].0;
                                    builder.push_instr(MirInstr::AssignField {
                                        base: temp,
                                        field: field_name.clone(),
                                        value: val_op,
                                    })?;
                                }
                            }
                        }
                        return Ok(Operand::Local(temp));
                    }

                    let builtin_symbol =
                        if (name == "map_put" || name == "lpp_map_put" || name == "map_get" || name == "lpp_map_get" || name == "map_has" || name == "lpp_map_has" || name == "map_remove" || name == "lpp_map_remove") && args.len() >= 2 {
                            let key_ty = self.expr_type_hint(&args[1], builder, binding_map);
                            let val_ty = if args.len() >= 3 {
                                self.expr_type_hint(&args[2], builder, binding_map)
                            } else if name == "map_get" || name == "lpp_map_get" {
                                let map_ty = self.expr_type_hint(&args[0], builder, binding_map);
                                if let TypeRef::Generic(_, ref params) = map_ty {
                                    params.get(1).cloned().unwrap_or(TypeRef::Int)
                                } else {
                                    TypeRef::Int
                                }
                            } else {
                                TypeRef::Int
                            };
                            let is_str_key = key_ty == TypeRef::Str;
                            let is_float_val = val_ty == TypeRef::Float;
                            Some(match name.as_str() {
                                "map_put" | "lpp_map_put" => {
                                    if is_str_key && is_float_val {
                                        "lpp_map_put_str_float".to_string()
                                    } else if is_str_key {
                                        "lpp_map_put_str".to_string()
                                    } else if is_float_val {
                                        "lpp_map_put_float".to_string()
                                    } else {
                                        "lpp_map_put".to_string()
                                    }
                                }
                                "map_get" | "lpp_map_get" => {
                                    if is_str_key && is_float_val {
                                        "lpp_map_get_str_float".to_string()
                                    } else if is_str_key {
                                        "lpp_map_get_str".to_string()
                                    } else if is_float_val {
                                        "lpp_map_get_float".to_string()
                                    } else {
                                        "lpp_map_get".to_string()
                                    }
                                }
                                "map_has" | "lpp_map_has" => {
                                    if is_str_key {
                                        "lpp_map_has_str".to_string()
                                    } else {
                                        "lpp_map_has".to_string()
                                    }
                                }
                                "map_remove" | "lpp_map_remove" => {
                                    if is_str_key {
                                        "lpp_map_remove_str".to_string()
                                    } else {
                                        "lpp_map_remove".to_string()
                                    }
                                }
                                _ => "lpp_map_get".to_string(),
                            })
                        } else if (name == "map_new" || name == "lpp_map_new") && args.is_empty() {
                            // Select arc variant when the map's value type is a managed object.
                            // The local's inferred type tells us the Map[K,V] shape; if V is
                            // managed (struct, str, generic collection, tuple, task) we need the
                            // arc-tracking constructor so the runtime calls retain on insert and
                            // release on overwrite/remove/destroy.
                            let is_arc_val = match &builder.function.locals[temp.0].ty {
                                TypeRef::Generic(n, params) if n == "Map" && params.len() == 2 => {
                                    params[1].is_managed()
                                }
                                _ => false,
                            };
                            Some(if is_arc_val { "lpp_map_new_arc".to_string() } else { "lpp_map_new".to_string() })
                        } else if matches!(
                            name.as_str(),
                            "list_push" | "lpp_list_push"
                                | "list_get" | "lpp_list_get"
                                | "list_set" | "lpp_list_set"
                                | "push" | "get"
                        ) {
                            let list_ty = args
                                .first()
                                .map(|arg| self.expr_type_hint(arg, builder, binding_map));
                            if let Some(TypeRef::Generic(_, ref params)) = list_ty {
                                if let Some(elem_ty) = params.first() {
                                    Some(match name.as_str() {
                                        "list_push" | "lpp_list_push" | "push" => {
                                            list_push_symbol(elem_ty)?
                                        }
                                        "list_set" | "lpp_list_set" => list_set_symbol(elem_ty)?,
                                        _ => list_get_symbol(elem_ty)?,
                                    }
                                    .to_string())
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else if name == "print" {
                        let (is_string, is_float, is_bool) = match lowered_args.first() {
                            Some(Operand::String(_)) => (true, false, false),
                            Some(Operand::Float(_)) => (false, true, false),
                            Some(Operand::Bool(_)) => (false, false, true),
                            Some(Operand::Local(local_id)) | Some(Operand::Borrowed(local_id)) => {
                                let ty = &builder.function.locals[local_id.0].ty;
                                (
                                    *ty == TypeRef::Str,
                                    *ty == TypeRef::Float,
                                    *ty == TypeRef::Bool,
                                )
                            }
                            _ => (false, false, false),
                        };
                        Some(
                            if is_string {
                                "lpp_print_str"
                            } else if is_float {
                                "lpp_print_float"
                            } else if is_bool {
                                "lpp_print_bool"
                            } else {
                                "lpp_print_int"
                            }
                            .to_string(),
                        )
                    } else {
                        crate::builtins::get_builtins()
                            .iter()
                            .find(|b| b.name == name)
                            .map(|b| b.symbol.to_string())
                    };

                    if let Some(symbol) = builtin_symbol {
                        if !symbol.is_empty() {
                            builder.push_instr(MirInstr::Assign(
                                temp,
                                Rvalue::BuiltinCall(symbol, lowered_args),
                            ))?;
                            return Ok(
                                if builder.function.locals[temp.0].ownership.is_borrowed()
                                {
                                    Operand::Borrowed(temp)
                                } else {
                                    Operand::Local(temp)
                                },
                            );
                        }
                    }
                }

                // Check for extern (FFI) function call
                if let Expr::Identifier(name, _) = &**callee {
                    if let Some(symbol) = self.extern_symbols.get(name) {
                        builder.push_instr(MirInstr::Assign(
                            temp,
                            Rvalue::BuiltinCall(symbol.clone(), lowered_args),
                        ))?;
                        return Ok(Operand::Local(temp));
                    }
                }

                let callee = self.lower_expr(builder, callee, binding_map)?;
                builder.push_instr(MirInstr::Assign(
                    temp,
                    Rvalue::CallIndirect(callee, lowered_args),
                ))?;
                Ok(Operand::Local(temp))
            }
            Expr::FieldAccess { base, field } => {
                // Check if this is an enum variant access (e.g., Color.Red)
                if let Expr::Identifier(name, _) = base.as_ref() {
                    for decl in &self.program.declarations {
                        if let crate::ast::TopLevel::Enum(e) = decl {
                            if e.name == *name {
                                // Unit enum variant (`Color.Red`) — same heap
                                // layout as a data variant, just no payload.
                                let tag = e.variants.iter().position(|v| v.name == *field).unwrap_or(0);
                                if let Some(id) = self.type_table.lookup_struct(name) {
                                    let ty = TypeRef::Custom(id);
                                    let temp = builder.new_local(ty.clone(), false, None, None);
                                    let allocation = self.struct_allocation(builder, ty)?;
                                    builder.push_instr(MirInstr::Assign(temp, allocation))?;
                                    builder.push_instr(MirInstr::AssignField {
                                        base: temp,
                                        field: "__tag".to_string(),
                                        value: Operand::Int(tag as i64),
                                    })?;
                                    return Ok(Operand::Local(temp));
                                }
                                let temp = builder.new_local(TypeRef::Int, false, None, None);
                                builder.push_instr(MirInstr::Assign(
                                    temp,
                                    Rvalue::Use(Operand::Int((tag as i64) << 32)),
                                ))?;
                                return Ok(Operand::Local(temp));
                            }
                        }
                    }
                }
                let base_op = self.lower_expr(builder, base, binding_map)?;
                let base_ty = match &base_op {
                    Operand::Local(local_id) | Operand::Borrowed(local_id) => {
                        builder.function.locals[local_id.0].ty.clone()
                    }
                    _ => TypeRef::Void,
                };
                let field_ty = self.get_field_type(&base_ty, field);
                let temp = builder.new_local(field_ty.clone(), false, None, None);
                // Reading a custom-struct field borrows the field's ARC edge;
                // it does not transfer ownership out of the containing object.
                if field_ty.is_managed() {
                    builder.set_local_ownership(temp, Ownership::Borrowed);
                }
                builder.push_instr(MirInstr::Assign(
                    temp,
                    Rvalue::FieldAccess(base_op, field.clone()),
                ))?;
                if builder.function.locals[temp.0].ownership.is_borrowed() {
                    Ok(Operand::Borrowed(temp))
                } else {
                    Ok(Operand::Local(temp))
                }
            }
            Expr::ListLiteral(items) => {
                let elem_ty = items
                    .first()
                    .map(|item| self.expr_type_hint(item, builder, binding_map))
                    .unwrap_or(TypeRef::Int);
                let temp = builder.new_local(
                    TypeRef::Generic("List".to_string(), vec![elem_ty.clone()]),
                    false,
                    None,
                    None,
                );
                builder.push_instr(MirInstr::Assign(
                    temp,
                    Rvalue::AllocateList(elem_ty.clone()),
                ))?;
                let push_symbol = list_push_symbol(&elem_ty)?;
                for item in items {
                    let item_op = self.lower_expr(builder, item, binding_map)?;
                    let discard_local = builder.new_local(TypeRef::Void, false, None, None);
                    builder.push_instr(MirInstr::Assign(
                        discard_local,
                        Rvalue::BuiltinCall(
                            push_symbol.to_string(),
                            vec![Operand::Local(temp), item_op],
                        ),
                    ))?;
                }
                Ok(Operand::Local(temp))
            }
            Expr::Spawn { closure } => {
                let closure_op = self.lower_expr(builder, closure, binding_map)?;
                let temp = builder.new_local(TypeRef::Void, false, None, None);
                builder.push_instr(MirInstr::Assign(temp, Rvalue::SpawnThread(closure_op)))?;
                Ok(Operand::Local(temp))
            }
            Expr::Closure {
                params,
                return_type: opt_return_type,
                body,
            } => {
                let closure_scope = {
                    let mut scope = None;
                    for i in self.closure_scope_idx..self.symbol_table.scopes.len() {
                        if let ScopeKind::Closure { .. } = self.symbol_table.scopes[i].kind {
                            scope = Some(self.symbol_table.scopes[i].id);
                            self.closure_scope_idx = i + 1;
                            break;
                        }
                    }
                    scope.ok_or_else(|| "Closure scope not found".to_string())?
                };

                let captures = match &self.symbol_table.scopes[closure_scope.0].kind {
                    ScopeKind::Closure { captures } => captures.clone(),
                    _ => Vec::new(),
                };

                // Mutable captures need a defined shared-cell / move ownership model.
                // Copying them into an environment makes `x = ...` inside the closure
                // silently diverge from the outer variable, which is not memory-safe or
                // unsurprising language semantics. Reject this case until that model is
                // implemented rather than compiling an incorrect program.
                if let Some(capture) = captures
                    .iter()
                    .find(|id| self.symbol_table.bindings[id.0].is_mut)
                {
                    let binding = &self.symbol_table.bindings[capture.0];
                    return Err(format!(
                        "mutable capture '{}' is not supported safely yet",
                        binding.name
                    ));
                }

                // Register environment struct
                let env_struct_name = format!("__lpp_closure_env_{}", closure_scope.0);
                let env_struct_id = self.type_table.register_struct(env_struct_name);

                let mut fields = Vec::new();
                for (i, &cap_id) in captures.iter().enumerate() {
                    let binding = &self.symbol_table.bindings[cap_id.0];
                    let ty = binding.ty.as_ref().cloned().unwrap_or(TypeRef::Int);
                    fields.push((format!("cap_{}", i), ty));
                }
                self.type_table.definitions[env_struct_id.0].fields = fields;

                // Allocate environment struct at definition site
                let env_local = builder.new_local(
                    TypeRef::Custom(env_struct_id),
                    false,
                    Some("__env_alloc".to_string()),
                    None,
                );
                builder.push_instr(MirInstr::Assign(
                    env_local,
                    Rvalue::AllocateArcStruct(TypeRef::Custom(env_struct_id)),
                ))?;

                // Populate captures
                for (i, &cap_id) in captures.iter().enumerate() {
                    let binding = &self.symbol_table.bindings[cap_id.0];
                    let val_op = self.lower_expr(
                        builder,
                        &Expr::Identifier(
                            binding.name.clone(),
                            std::cell::Cell::new(Some(cap_id.0)),
                        ),
                        binding_map,
                    )?;
                    builder.push_instr(MirInstr::AssignField {
                        base: env_local,
                        field: format!("cap_{}", i),
                        value: val_op,
                    })?;
                }

                // Allocate func ID for lifted closure function
                let closure_func_id = FuncId(self.next_func_id);
                self.next_func_id += 1;
                let closure_name = format!("__lpp_closure_fn_{}", closure_func_id.0);

                // Lower closure function
                let return_type = if let Some(t) = opt_return_type {
                    self.resolve_type(t)
                } else {
                    let mut inferred_rt = TypeRef::Void;
                    for stmt in body {
                        if let Stmt::Return(Some(expr)) = stmt {
                            // Best-effort typehint:
                            if let Ok(ty) = self.lower_expr(builder, expr, binding_map) {
                                if let Operand::Local(lid) | Operand::Borrowed(lid) = ty {
                                    inferred_rt = builder.function.locals[lid.0].ty.clone();
                                }
                            }
                            break;
                        }
                    }
                    inferred_rt
                };

                let mut closure_builder =
                    MirBuilder::new(closure_func_id, closure_name.clone(), return_type.clone());
                let mut closure_binding_map = HashMap::new();

                let env_ptr_local = closure_builder.new_local(
                    TypeRef::Custom(env_struct_id),
                    false,
                    Some("__env".to_string()),
                    None,
                );
                closure_builder.set_local_ownership(env_ptr_local, Ownership::Borrowed);
                closure_builder.function.params.push(env_ptr_local);

                for param in params {
                    let param_binding_id = self.symbol_table.scopes[closure_scope.0]
                        .bindings
                        .get(&param.name)
                        .copied();
                    let ty = if let Some(ref t) = param.ty {
                        self.resolve_type(t)
                    } else if let Some(bid) = param_binding_id {
                        self.symbol_table.bindings[bid.0]
                            .ty
                            .clone()
                            .unwrap_or(TypeRef::Int)
                    } else {
                        TypeRef::Int
                    };
                    let local = closure_builder.new_local(
                        ty,
                        false,
                        Some(param.name.clone()),
                        param_binding_id,
                    );
                    closure_builder.set_local_ownership(local, Ownership::Borrowed);
                    closure_builder.function.params.push(local);
                    if let Some(bid) = param_binding_id {
                        closure_binding_map.insert(bid, local);
                    }
                }

                // Set closure lowering context
                let saved_env_ptr = self.current_env_ptr;
                let saved_captures = std::mem::take(&mut self.current_captures);
                let saved_arena = self.current_arena.take();

                self.current_env_ptr = Some(env_ptr_local);
                self.current_captures = captures;

                for stmt in body {
                    self.lower_stmt(&mut closure_builder, stmt, &mut closure_binding_map)?;
                }

                if let Ok(current_block) = closure_builder.current_block() {
                    if current_block.0 < closure_builder.function.blocks.len() {
                        closure_builder.set_terminator(current_block, Terminator::Return(None))?;
                    }
                }

                let mir_fn = closure_builder.finish();
                self.lifted_functions.insert(closure_func_id, mir_fn);

                // Restore context
                self.current_env_ptr = saved_env_ptr;
                self.current_captures = saved_captures;
                self.current_arena = saved_arena;

                // Return closure fat pointer
                let closure_local = builder.new_local(
                    TypeRef::Function,
                    false,
                    Some("__closure".to_string()),
                    None,
                );
                self.closure_returns
                    .insert((builder.function.id.0, closure_local.0), return_type.clone());
                builder.push_instr(MirInstr::Assign(
                    closure_local,
                    Rvalue::MakeClosure(closure_func_id, vec![Operand::Local(env_local)]),
                ))?;

                Ok(Operand::Local(closure_local))
            }
            Expr::EnumVariantConstruct { enum_name, variant, args } => {
                // Allocate a real ARC object: `__tag` plus the payload slot for
                // this variant. The old packed-i64 form truncated the payload
                // to 32 bits, which corrupted Str/Float/large-Int values.
                let tag = self.get_enum_variant_tag(enum_name, variant);
                let struct_id = match self.type_table.lookup_struct(enum_name) {
                    Some(id) => id,
                    None => {
                        // Not a known enum — preserve the old scalar behaviour.
                        let temp = builder.new_local(TypeRef::Int, false, None, None);
                        builder.push_instr(MirInstr::Assign(
                            temp,
                            Rvalue::Use(Operand::Int((tag as i64) << 32)),
                        ))?;
                        return Ok(Operand::Local(temp));
                    }
                };
                let ty = TypeRef::Custom(struct_id);
                let temp = builder.new_local(ty.clone(), false, None, None);
                let allocation = self.struct_allocation(builder, ty)?;
                builder.push_instr(MirInstr::Assign(temp, allocation))?;
                builder.push_instr(MirInstr::AssignField {
                    base: temp,
                    field: "__tag".to_string(),
                    value: Operand::Int(tag as i64),
                })?;
                if !args.is_empty() {
                    let data_op = self.lower_expr(builder, &args[0], binding_map)?;
                    builder.push_instr(MirInstr::AssignField {
                        base: temp,
                        field: format!("__v{}", tag),
                        value: data_op,
                    })?;
                }
                Ok(Operand::Local(temp))
            }
            Expr::Match { subject, arms } => {
                // Match as expression — same as statement match for now.
                // The tag lives in a field now, so compare against that rather
                // than against the object handle itself.
                let subject_val = self.lower_expr(builder, subject, binding_map)?;
                let subject_is_enum = matches!(
                    match &subject_val {
                        Operand::Local(id) | Operand::Borrowed(id) =>
                            builder.function.locals[id.0].ty.clone(),
                        _ => TypeRef::Int,
                    },
                    TypeRef::Custom(_)
                );
                let subject_tag = builder.new_local(TypeRef::Int, false, None, None);
                if subject_is_enum {
                    builder.push_instr(MirInstr::Assign(
                        subject_tag,
                        Rvalue::FieldAccess(subject_val.clone(), "__tag".to_string()),
                    ))?;
                } else {
                    builder.push_instr(MirInstr::Assign(
                        subject_tag,
                        Rvalue::Use(subject_val.clone()),
                    ))?;
                }
                let result = builder.new_local(TypeRef::Int, false, None, None);
                let end_block = builder.new_block();

                for (i, arm) in arms.iter().enumerate() {
                    let arm_block = builder.new_block();
                    let next_block = if i + 1 < arms.len() { builder.new_block() } else { end_block };
                    let tag_local = builder.new_local(TypeRef::Int, false, None, None);
                    builder.push_instr(MirInstr::Assign(tag_local, Rvalue::Use(Operand::Int(i as i64))))?;
                    let cmp_local = builder.new_local(TypeRef::Bool, false, None, None);
                    builder.push_instr(MirInstr::Assign(
                        cmp_local,
                        Rvalue::BinaryOp(BinaryOperator::Eq, Operand::Local(subject_tag), Operand::Local(tag_local)),
                    ))?;
                    builder.terminate_current_block(Terminator::If {
                        cond: Operand::Local(cmp_local),
                        then_block: arm_block,
                        else_block: next_block,
                    })?;
                    builder.switch_to_block(arm_block);
                    for stmt in &arm.body {
                        self.lower_stmt(builder, stmt, binding_map)?;
                    }
                    builder.terminate_current_block(Terminator::Goto(end_block))?;
                    if i + 1 < arms.len() {
                        builder.switch_to_block(next_block);
                    }
                }
                builder.switch_to_block(end_block);
                Ok(Operand::Local(result))
            }
            Expr::Try(inner) => {
                // expr? — unwrap Ok (variant 0) or return the whole value early.
                // Enums are heap objects, so the tag and payload are fields.
                let val = self.lower_expr(builder, inner, binding_map)?;
                let val_ty = match &val {
                    Operand::Local(id) | Operand::Borrowed(id) => {
                        builder.function.locals[id.0].ty.clone()
                    }
                    _ => TypeRef::Int,
                };
                let enum_struct_id = match val_ty {
                    TypeRef::Custom(id) => Some(id),
                    _ => None,
                };

                let shift = builder.new_local(TypeRef::Int, false, None, None);
                builder.push_instr(MirInstr::Assign(shift, Rvalue::Use(Operand::Int(4294967296))))?;
                let tag = builder.new_local(TypeRef::Int, false, None, None);
                if enum_struct_id.is_some() {
                    builder.push_instr(MirInstr::Assign(
                        tag,
                        Rvalue::FieldAccess(val.clone(), "__tag".to_string()),
                    ))?;
                } else {
                    builder.push_instr(MirInstr::Assign(
                        tag,
                        Rvalue::BinaryOp(BinaryOperator::Divide, val.clone(), Operand::Local(shift)),
                    ))?;
                }

                // 3. If tag != 0 (is Err), return the whole value (propagate error)
                let zero = builder.new_local(TypeRef::Int, false, None, None);
                builder.push_instr(MirInstr::Assign(zero, Rvalue::Use(Operand::Int(0))))?;
                let is_ok = builder.new_local(TypeRef::Bool, false, None, None);
                builder.push_instr(MirInstr::Assign(
                    is_ok,
                    Rvalue::BinaryOp(BinaryOperator::Eq, Operand::Local(tag), Operand::Local(zero)),
                ))?;

                let ok_block = builder.new_block();
                let err_block = builder.new_block();
                builder.terminate_current_block(Terminator::If {
                    cond: Operand::Local(is_ok),
                    then_block: ok_block,
                    else_block: err_block,
                })?;

                // Err path: propagate the value immediately. It must leave as
                // ReturnOwned, otherwise the ARC pass emits a release for the
                // very object being returned and the caller reads freed memory.
                builder.switch_to_block(err_block);
                if enum_struct_id.is_some() {
                    builder.terminate_current_block(Terminator::ReturnOwned(val.clone()))?;
                } else {
                    builder.terminate_current_block(Terminator::Return(Some(val.clone())))?;
                }

                // Ok path: read variant 0's payload slot at its real type.
                builder.switch_to_block(ok_block);
                let payload_ty = enum_struct_id
                    .and_then(|sid| {
                        self.type_table.definitions[sid.0]
                            .fields
                            .iter()
                            .find(|(n, _)| n == "__v0")
                            .map(|(_, t)| t.clone())
                    })
                    .unwrap_or(TypeRef::Int);
                let data = builder.new_local(payload_ty.clone(), false, None, None);
                if enum_struct_id.is_some() {
                    if payload_ty.is_managed() {
                        builder.set_local_ownership(data, Ownership::Borrowed);
                    }
                    builder.push_instr(MirInstr::Assign(
                        data,
                        Rvalue::FieldAccess(val, "__v0".to_string()),
                    ))?;
                } else {
                    builder.push_instr(MirInstr::Assign(
                        data,
                        Rvalue::BinaryOp(BinaryOperator::Modulo, val, Operand::Local(shift)),
                    ))?;
                }
                Ok(Operand::Local(data))
            }
            Expr::GenericCall { .. } => Err(
                "internal error: generic call with explicit type arguments reached MIR lowering; monomorphization should have resolved it"
                    .to_string(),
            ),
            Expr::Index { base, index } => {
                // Desugar subscript:
                //   str[i]  → str_substr(str, i, 1)
                //   lst[i]  → list_get(lst, i)
                let base_ty = self.expr_type_hint(base, builder, binding_map);
                if base_ty == TypeRef::Str {
                    // str_substr(base, index, 1)
                    let desugared = Expr::Call {
                        callee: Box::new(Expr::Identifier("str_substr".to_string(), std::cell::Cell::new(None))),
                        args: vec![*base.clone(), *index.clone(), Expr::IntLiteral(1)],
                    };
                    self.lower_expr(builder, &desugared, binding_map)
                } else {
                    // list_get(base, index)
                    let desugared = Expr::Call {
                        callee: Box::new(Expr::Identifier("list_get".to_string(), std::cell::Cell::new(None))),
                        args: vec![*base.clone(), *index.clone()],
                    };
                    self.lower_expr(builder, &desugared, binding_map)
                }
            }
        }
    }

    fn get_enum_variant_tag(&self, enum_name: &str, variant: &str) -> usize {
        // Search through the program's enum definitions for the variant index
        for decl in &self.program.declarations {
            if let crate::ast::TopLevel::Enum(e) = decl {
                if e.name == enum_name {
                    for (i, v) in e.variants.iter().enumerate() {
                        if v.name == variant {
                            return i;
                        }
                    }
                }
            }
        }
        0 // fallback
    }
}

