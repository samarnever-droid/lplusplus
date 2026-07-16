use crate::ast::*;
use crate::semantic::{SymbolTable, BindingId, ScopeKind};
use crate::typecheck::{TypeTable, TypeRef};
use crate::mir::ir::*;
use crate::mir::builder::MirBuilder;
use std::collections::HashMap;

pub struct MirLowerCtx<'a> {
    pub symbol_table: &'a SymbolTable,
    pub type_table: &'a TypeTable,
    
    // Mapping from global AST function name to MIR FuncId
    pub functions: HashMap<String, FuncId>,
    
    pub next_func_id: usize,
}

impl<'a> MirLowerCtx<'a> {
    pub fn new(symbol_table: &'a SymbolTable, type_table: &'a TypeTable) -> Self {
        Self {
            symbol_table,
            type_table,
            functions: HashMap::new(),
            next_func_id: 0,
        }
    }

    /// Convert an AST `Type` annotation to the MIR `TypeRef`.
    fn ast_type_to_mir_type(ty: &Type) -> TypeRef {
        match ty {
            Type::Int        => TypeRef::Int,
            Type::String     => TypeRef::Str,
            Type::Void       => TypeRef::Void,
            Type::Custom(s)  => TypeRef::Unresolved(s.clone()),
            Type::Generic(n, args) => {
                TypeRef::Generic(n.clone(), args.iter().map(Self::ast_type_to_mir_type).collect())
            }
        }
    }
    
    pub fn lower_program(&mut self, program: &Program) -> MirProgram {
        let mut mir_functions = HashMap::new();
        
        // First pass: assign FuncId to all functions
        for decl in &program.declarations {
            if let TopLevel::Function(f) = decl {
                let id = FuncId(self.next_func_id);
                self.next_func_id += 1;
                self.functions.insert(f.name.clone(), id);
            }
        }
        
        // Second pass: lower bodies
        let ast_functions: Vec<_> = program.declarations.iter().filter_map(|d| {
            if let TopLevel::Function(f) = d { Some(f) } else { None }
        }).collect();
        
        for f in ast_functions {
            let mir_fn = self.lower_function(f);
            mir_functions.insert(mir_fn.id, mir_fn);
        }
        
        MirProgram {
            functions: mir_functions,
        }
    }
    
    fn lower_function(&mut self, func: &Function) -> MirFunction {
        let func_id = self.functions[&func.name];
        // Resolve return type from the AST annotation (exact mapping).
        let return_type = Self::ast_type_to_mir_type(&func.return_type);
        let mut builder = MirBuilder::new(func_id, func.name.clone(), return_type);
        let mut binding_map = HashMap::new();
        
        // Lower parameters
        // The type table stores the function's signature. For the MVP, we just use TypeRef::Int for everything if not mapped.
        // We really should use TypeTable or SymbolTable. Since AST parameters don't have BindingId yet, 
        // wait, do AST parameters have BindingId in the current tree? No, but semantic pass inserted them into scopes.
        // We can just iterate the params, they aren't uniquely identified by ID in AST. Let's just create locals for them.
        for param in &func.params {
            let b_id = self.symbol_table.scopes.iter().find_map(|s| {
                if let ScopeKind::Function { name } = &s.kind {
                    if name == &func.name {
                        return s.bindings.get(&param.name).copied();
                    }
                }
                None
            });
            let ty = TypeRef::Int; // Default fallback for now
            let local = builder.new_local(ty, false, Some(param.name.clone()), b_id);
            builder.function.params.push(local);
            if let Some(id) = b_id {
                binding_map.insert(id, local);
            }
        }
        
        for stmt in &func.body {
            self.lower_stmt(&mut builder, stmt, &mut binding_map);
        }
        
        // Add a default return if the block isn't terminated
        // (This would be handled more cleanly in a complete compiler)
        if builder.current_block().0 < builder.function.blocks.len() {
            let cur = builder.current_block();
            builder.set_terminator(cur, Terminator::Return(None));
        }
        
        builder.finish()
    }
    
    fn lower_stmt(&mut self, builder: &mut MirBuilder, stmt: &Stmt, binding_map: &mut HashMap<BindingId, LocalId>) {
        match stmt {
            Stmt::LetInferred { name, value, binding_id, .. } => {
                let ast_id = binding_id.get().unwrap();
                let b_id = BindingId(ast_id);
                let ty = self.symbol_table.bindings[ast_id].ty.clone().unwrap();
                
                let local_id = builder.new_local(ty, true, Some(name.clone()), Some(b_id));
                binding_map.insert(b_id, local_id);
                
                let operand = self.lower_expr(builder, value, binding_map);
                builder.push_instr(MirInstr::Assign(local_id, Rvalue::Use(operand)));
            }
            Stmt::Assign { name: _, value, binding_id } => {
                let ast_id = binding_id.get().unwrap();
                let b_id = BindingId(ast_id);
                let local_id = binding_map[&b_id];
                
                let operand = self.lower_expr(builder, value, binding_map);
                builder.push_instr(MirInstr::Assign(local_id, Rvalue::Use(operand)));
            }
            Stmt::AssignField { base, field, value } => {
                let base_op = self.lower_expr(builder, base, binding_map);
                let val_op = self.lower_expr(builder, value, binding_map);
                if let Operand::Local(base_id) = base_op {
                    builder.push_instr(MirInstr::AssignField {
                        base: base_id,
                        field: field.clone(),
                        value: val_op,
                    });
                } else {
                    panic!("Field assignment base is not a local variable");
                }
            }
            Stmt::Expr(expr) => {
                self.lower_expr(builder, expr, binding_map);
            }
            Stmt::Return(expr) => {
                let op = expr.as_ref().map(|e| self.lower_expr(builder, e, binding_map));
                builder.terminate_current_block(Terminator::Return(op));
                let next = builder.new_block();
                builder.switch_to_block(next);
            }
            Stmt::If { condition, then_block, else_block } => {
                let cond_op = self.lower_expr(builder, condition, binding_map);
                
                let then_b = builder.new_block();
                let else_b = builder.new_block();
                let merge_b = builder.new_block();
                
                builder.terminate_current_block(Terminator::If {
                    cond: cond_op,
                    then_block: then_b,
                    else_block: if else_block.is_some() { else_b } else { merge_b },
                });
                
                builder.switch_to_block(then_b);
                for s in then_block {
                    self.lower_stmt(builder, s, binding_map);
                }
                builder.terminate_current_block(Terminator::Goto(merge_b));
                
                if let Some(else_stmts) = else_block {
                    builder.switch_to_block(else_b);
                    for s in else_stmts {
                        self.lower_stmt(builder, s, binding_map);
                    }
                    builder.terminate_current_block(Terminator::Goto(merge_b));
                }
                
                builder.switch_to_block(merge_b);
            }
            Stmt::While { condition, body } => {
                let cond_b = builder.new_block();
                let body_b = builder.new_block();
                let end_b = builder.new_block();
                
                builder.terminate_current_block(Terminator::Goto(cond_b));
                
                builder.switch_to_block(cond_b);
                let cond_op = self.lower_expr(builder, condition, binding_map);
                builder.terminate_current_block(Terminator::If {
                    cond: cond_op,
                    then_block: body_b,
                    else_block: end_b,
                });
                
                builder.switch_to_block(body_b);
                for s in body {
                    self.lower_stmt(builder, s, binding_map);
                }
                builder.terminate_current_block(Terminator::Goto(cond_b));
                
                builder.switch_to_block(end_b);
            }
        }
    }
    
    fn lower_expr(&mut self, builder: &mut MirBuilder, expr: &Expr, binding_map: &mut HashMap<BindingId, LocalId>) -> Operand {
        match expr {
            Expr::IntLiteral(i) => Operand::Int(*i),
            Expr::StringLiteral(s) => Operand::String(s.clone()),
            Expr::Identifier(name, cell) => {
                let ast_id = cell.get().unwrap();
                let local = match binding_map.get(&BindingId(ast_id)) {
                    Some(l) => *l,
                    None => {
                        panic!(
                            "Identifier '{}' (BindingId {}) not found in binding_map while lowering function '{}'!",
                            name, ast_id, builder.function.name
                        );
                    }
                };
                Operand::Local(local)
            }
            Expr::BinaryOp { left, op, right } => {
                let l = self.lower_expr(builder, left, binding_map);
                let r = self.lower_expr(builder, right, binding_map);
                
                // create a temporary
                let ty = TypeRef::Int; // hardcoded for now, normally get from typecheck
                let temp = builder.new_local(ty, false, None, None);
                
                builder.push_instr(MirInstr::Assign(temp, Rvalue::BinaryOp(op.clone(), l, r)));
                Operand::Local(temp)
            }
            Expr::Call { callee, args } => {
                let mut ops = Vec::new();
                for arg in args {
                    ops.push(self.lower_expr(builder, arg, binding_map));
                }

                let ty = TypeRef::Void; // return type resolved properly in future
                let temp = builder.new_local(ty, false, None, None);

                if let Expr::Identifier(name, _) = &**callee {
                    // 1. Known top-level user function → CallDirect
                    if let Some(&func_id) = self.functions.get(name) {
                        builder.push_instr(MirInstr::Assign(temp, Rvalue::CallDirect(func_id, ops)));
                        return Operand::Local(temp);
                    }

                    // 2. Custom struct constructor call → AllocateStruct
                    if let Some(&struct_id) = self.type_table.structs_by_name.get(name) {
                        builder.push_instr(MirInstr::Assign(temp, Rvalue::AllocateStruct(TypeRef::Custom(struct_id))));
                        return Operand::Local(temp);
                    }

                    // 2. Known runtime builtin → BuiltinCall (name-mangled to C ABI symbol)
                    let runtime_sym = match name.as_str() {
                        "print"      => {
                            // Dispatch based on the first arg's inferred type:
                            // Operand::String → lpp_print_str, else → lpp_print_int
                            let sym = if matches!(ops.first(), Some(Operand::String(_))) {
                                "lpp_print_str"
                            } else {
                                "lpp_print_int"
                            };
                            Some(sym)
                        }
                        "print_str"  => Some("lpp_print_str"),
                        "input"      => Some("lpp_input"),
                        "read_file"  => Some("lpp_read_file"),
                        "write_file" => Some("lpp_write_file"),
                        _            => None,
                    };

                    if let Some(sym) = runtime_sym {
                        builder.push_instr(MirInstr::Assign(
                            temp,
                            Rvalue::BuiltinCall(sym.to_string(), ops),
                        ));
                        return Operand::Local(temp);
                    }
                }

                // 3. Anything else → CallIndirect (closure or unknown)
                let c_op = self.lower_expr(builder, callee, binding_map);
                builder.push_instr(MirInstr::Assign(temp, Rvalue::CallIndirect(c_op, ops)));
                Operand::Local(temp)
            }
            Expr::FieldAccess { base, field } => {
                let base_op = self.lower_expr(builder, base, binding_map);
                let ty = TypeRef::Void; // hardcoded
                let temp = builder.new_local(ty, false, None, None);
                builder.push_instr(MirInstr::Assign(temp, Rvalue::FieldAccess(base_op, field.clone())));
                Operand::Local(temp)
            }
            Expr::ListLiteral(_items) => {
                let ty = TypeRef::Generic("List".to_string(), vec![TypeRef::Int]);
                let temp = builder.new_local(ty, false, None, None);
                builder.push_instr(MirInstr::Assign(temp, Rvalue::AllocateList(TypeRef::Int)));
                Operand::Local(temp)
                // TODO: emit assignments for items
            }
            Expr::Spawn { closure } => {
                // For MVP, just lower the closure expr
                self.lower_expr(builder, closure, binding_map)
            }
            Expr::Closure { params: _, return_type: _, body: _ } => {
                // Closure lowering requires extracting the body into a new MIR function.
                // We'll stub this by returning a dummy operand for now, as full
                // capture analysis is needed.
                Operand::Int(0)
            }
            // Other expressions omitted for the MVP
        }
    }
}
