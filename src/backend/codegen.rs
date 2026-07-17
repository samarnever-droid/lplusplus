use std::collections::HashMap;
use crate::ast::*;
use crate::semantic::{SymbolTable, ScopeKind, BindingId};
use crate::typecheck::{TypeTable, TypeRef};
use crate::escape::StorageClass;

pub struct Codegen<'a> {
    symbol_table: &'a SymbolTable,
    type_table: &'a TypeTable,
    storage: &'a HashMap<BindingId, StorageClass>,
    out: String,
    closure_scope_idx: usize,
}

impl<'a> Codegen<'a> {
    pub fn new(
        symbol_table: &'a SymbolTable,
        type_table: &'a TypeTable,
        storage: &'a HashMap<BindingId, StorageClass>
    ) -> Self {
        Self {
            symbol_table,
            type_table,
            storage,
            out: String::new(),
            closure_scope_idx: 0,
        }
    }

    pub fn generate(&mut self, program: &Program) -> String {
        self.out.push_str("#include <stdio.h>\n");
        self.out.push_str("#include <stdlib.h>\n");
        self.out.push_str("#include <stdint.h>\n");
        self.out.push_str("#include <string.h>\n");
        self.out.push_str("#include <malloc.h>\n"); // for alloca
        self.out.push_str("\n");
        self.out.push_str(crate::c_runtime_headers::C_BUILTINS_IO);
        self.out.push_str(crate::c_runtime_headers::C_BUILTINS_JSON);

        // Emitting structs
        for def in &self.type_table.definitions {
            self.out.push_str(&format!("typedef struct {} {}_t;\n", def.name, def.name));
            self.out.push_str(&format!("struct {} {{\n", def.name));
            if def.is_self_referential {
                self.out.push_str("    int _ref_count;\n");
            }
            for (name, ty) in &def.fields {
                let c_type = self.c_type(ty);
                self.out.push_str(&format!("    {} {};\n", c_type, name));
            }
            self.out.push_str("};\n\n");
        }
        
        // Emitting function prototypes
        for decl in &program.declarations {
            if let TopLevel::Function(f) = decl {
                let ret_ty = self.c_type_ast(&f.return_type);
                if f.name == "main" {
                    self.out.push_str(&format!("{} lpp_main(", ret_ty));
                } else {
                    self.out.push_str(&format!("{} {}(", ret_ty, f.name));
                }
                for (i, p) in f.params.iter().enumerate() {
                    if i > 0 { self.out.push_str(", "); }
                    self.out.push_str(&format!("{} {}", self.c_type_ast(&p.ty), p.name));
                }
                if f.params.is_empty() {
                    self.out.push_str("void");
                }
                self.out.push_str(");\n");
            }
        }
        self.out.push_str("\n");
        
        // Emitting functions
        for decl in &program.declarations {
            if let TopLevel::Function(f) = decl {
                self.gen_function(f);
            }
        }
        
        // Adding C main
        self.out.push_str("int main() {\n");
        // find if main exists in lpp
        let mut has_main = false;
        for decl in &program.declarations {
            if let TopLevel::Function(f) = decl {
                if f.name == "main" {
                    has_main = true;
                }
            }
        }
        if has_main {
            self.out.push_str("    lpp_main();\n");
        }
        self.out.push_str("    return 0;\n}\n");
        
        self.out.clone()
    }

    fn gen_function(&mut self, f: &Function) {
        let ret_ty = self.c_type_ast(&f.return_type);
        if f.name == "main" {
            self.out.push_str(&format!("{} lpp_main(", ret_ty));
        } else {
            self.out.push_str(&format!("{} {}(", ret_ty, f.name));
        }
        let mut func_scope = None;
        for scope in &self.symbol_table.scopes {
            if let ScopeKind::Function { name } = &scope.kind {
                if name == &f.name {
                    func_scope = Some(scope.id);
                    break;
                }
            }
        }
        
        for (i, p) in f.params.iter().enumerate() {
            if i > 0 { self.out.push_str(", "); }
            let mut unique_name = p.name.clone();
            if let Some(scope_id) = func_scope {
                if let Some(binding_id) = self.symbol_table.resolve_name_immutable(scope_id, &p.name) {
                    unique_name = format!("{}_{}", p.name, binding_id.0);
                }
            }
            self.out.push_str(&format!("{} {}", self.c_type_ast(&p.ty), unique_name));
        }
        if f.params.is_empty() {
            self.out.push_str("void");
        }
        self.out.push_str(") {\n");
        
        if let Some(scope_id) = func_scope {
            for stmt in &f.body {
                self.gen_stmt(stmt, scope_id);
            }
        }
        self.out.push_str("}\n\n");
    }

    fn gen_stmt(&mut self, stmt: &Stmt, current_scope: crate::semantic::ScopeId) {
        match stmt {
            Stmt::LetInferred { name, value, binding_id, .. } => {
                let id = binding_id.get().unwrap();
                let class = self.storage.get(&crate::semantic::BindingId(id)).unwrap_or(&StorageClass::Value);
                self.out.push_str(&format!("    /* Storage: {:?} */\n", class));
                let unique_name = format!("{}_{}", name, id);
                let binding = &self.symbol_table.bindings[id];
                let ty_str = if let Some(ty) = &binding.ty {
                    self.c_type(ty)
                } else {
                    "int64_t".to_string()
                };
                self.out.push_str(&format!("    {} {} = ", ty_str, unique_name));
                self.gen_expr(value, current_scope, Some(class));
                self.out.push_str(";\n");
            }
            Stmt::Assign { name, value, binding_id, .. } => {
                let id = binding_id.get().unwrap();
                let unique_name = format!("{}_{}", name, id);
                self.out.push_str(&format!("    {} = ", unique_name));
                self.gen_expr(value, current_scope, None);
                self.out.push_str(";\n");
            }
            Stmt::AssignField { base, field, value } => {
                self.out.push_str("    ");
                self.gen_expr(base, current_scope, None);
                self.out.push_str(&format!("->{} = ", field));
                self.gen_expr(value, current_scope, None);
                self.out.push_str(";\n");
            }
            Stmt::If { condition, then_block, else_block } => {
                self.out.push_str("if (");
                self.gen_expr(condition, current_scope, None);
                self.out.push_str(") {\n");
                for stmt in then_block {
                    self.gen_stmt(stmt, current_scope);
                }
                self.out.push_str("}");
                if let Some(else_b) = else_block {
                    self.out.push_str(" else {\n");
                    for stmt in else_b {
                        self.gen_stmt(stmt, current_scope);
                    }
                    self.out.push_str("}\n");
                } else {
                    self.out.push_str("\n");
                }
            }
            Stmt::While { condition, body } => {
                self.out.push_str("while (");
                self.gen_expr(condition, current_scope, None);
                self.out.push_str(") {\n");
                for stmt in body {
                    self.gen_stmt(stmt, current_scope);
                }
                self.out.push_str("}\n");
            }
            Stmt::Expr(expr) => {
                self.out.push_str("    ");
                self.gen_expr(expr, current_scope, None);
                self.out.push_str(";\n");
            }
            Stmt::Return(Some(e)) => {
                self.out.push_str("    return ");
                self.gen_expr(e, current_scope, None);
                self.out.push_str(";\n");
            }
            Stmt::Return(None) => {
                self.out.push_str("    return;\n");
            }
        }
    }

    fn gen_expr(&mut self, expr: &Expr, current_scope: crate::semantic::ScopeId, target_class: Option<&StorageClass>) {
        match expr {
            Expr::IntLiteral(i) => self.out.push_str(&i.to_string()),
            Expr::StringLiteral(s) => self.out.push_str(&format!("\"{}\"", s)),
            Expr::Identifier(n, binding_cell) => {
                if let Some(id) = binding_cell.get() {
                    self.out.push_str(&format!("{}_{}", n, id));
                } else {
                    self.out.push_str(n);
                }
            },
            Expr::BinaryOp { left, op, right } => {
                self.out.push_str("(");
                self.gen_expr(left, current_scope, target_class);
                let op_str = match op {
                    crate::ast::BinaryOperator::Add => " + ",
                    crate::ast::BinaryOperator::Subtract => " - ",
                    crate::ast::BinaryOperator::Multiply => " * ",
                    crate::ast::BinaryOperator::Divide => " / ",
                    crate::ast::BinaryOperator::Modulo => " % ",
                    crate::ast::BinaryOperator::Eq => " == ",
                    crate::ast::BinaryOperator::NotEq => " != ",
                    crate::ast::BinaryOperator::Less => " < ",
                    crate::ast::BinaryOperator::LessEq => " <= ",
                    crate::ast::BinaryOperator::Greater => " > ",
                    crate::ast::BinaryOperator::GreaterEq => " >= ",
                };
                self.out.push_str(op_str);
                self.gen_expr(right, current_scope, target_class);
                self.out.push_str(")");
            }
            Expr::Call { callee, args } => {
                if let Expr::Identifier(n, _) = &**callee {
                    if n == "print" {
                        if args.is_empty() {
                            self.out.push_str("printf(\"\\n\")");
                        } else {
                            self.out.push_str("printf(\"");
                            for (i, arg) in args.iter().enumerate() {
                                if i > 0 { self.out.push_str(" "); }
                                let arg_ty = self.expr_type(arg, current_scope);
                                if arg_ty == TypeRef::Str {
                                    self.out.push_str("%s");
                                } else {
                                    self.out.push_str("%lld");
                                }
                            }
                            self.out.push_str("\\n\"");
                            for arg in args {
                                let arg_ty = self.expr_type(arg, current_scope);
                                if arg_ty == TypeRef::Str {
                                    self.out.push_str(", (char*)(");
                                } else {
                                    self.out.push_str(", (long long)(");
                                }
                                self.gen_expr(arg, current_scope, None);
                                self.out.push_str(")");
                            }
                            self.out.push_str(")");
                        }
                        return;
                    }
                    if n == "print_str" {
                        self.out.push_str("printf(\"%s\\n\", (char*)(");
                        if !args.is_empty() {
                            self.gen_expr(&args[0], current_scope, None);
                        } else {
                            self.out.push_str("\"\"");
                        }
                        self.out.push_str("))");
                        return;
                    }
                    if n == "input" {
                        self.out.push_str("lpp_input()");
                        return;
                    }
                    if n == "read_file" {
                        self.out.push_str("lpp_read_file(");
                        self.gen_expr(&args[0], current_scope, None);
                        self.out.push_str(")");
                        return;
                    }
                    if n == "write_file" {
                        self.out.push_str("lpp_write_file(");
                        self.gen_expr(&args[0], current_scope, None);
                        self.out.push_str(", ");
                        self.gen_expr(&args[1], current_scope, None);
                        self.out.push_str(")");
                        return;
                    }
                    if n == "parse_int" {
                        self.out.push_str("(int64_t)atoll(");
                        self.gen_expr(&args[0], current_scope, None);
                        self.out.push_str(")");
                        return;
                    }
                    if n == "list_new" {
                        self.out.push_str("lpp_list_new()");
                        return;
                    }
                    if n == "list_push" {
                        self.out.push_str("lpp_list_push(");
                        self.gen_expr(&args[0], current_scope, None);
                        self.out.push_str(", ");
                        self.gen_expr(&args[1], current_scope, None);
                        self.out.push_str(")");
                        return;
                    }
                    if n == "list_get" {
                        self.out.push_str("lpp_list_get(");
                        self.gen_expr(&args[0], current_scope, None);
                        self.out.push_str(", ");
                        self.gen_expr(&args[1], current_scope, None);
                        self.out.push_str(")");
                        return;
                    }
                    if n == "list_len" {
                        self.out.push_str("lpp_list_len(");
                        self.gen_expr(&args[0], current_scope, None);
                        self.out.push_str(")");
                        return;
                    }
                    if n == "list_free" {
                        self.out.push_str("lpp_list_free(");
                        self.gen_expr(&args[0], current_scope, None);
                        self.out.push_str(")");
                        return;
                    }
                    if let Some(&id) = self.type_table.structs_by_name.get(n) {
                        let def = &self.type_table.definitions[id.0];
                        let alloc_str = match target_class {
                            Some(StorageClass::Value) => format!("({}_t*)alloca(sizeof({}_t))", def.name, def.name),
                            Some(StorageClass::Arc) => format!("({}_t*)calloc(1, sizeof({}_t))", def.name, def.name),
                            Some(StorageClass::Arena { .. }) => format!("({}_t*)malloc(sizeof({}_t)) /* arena */", def.name, def.name),
                            None => format!("({}_t*)calloc(1, sizeof({}_t))", def.name, def.name),
                        };
                        self.out.push_str(&alloc_str);
                        return;
                    }
                }
                self.gen_expr(callee, current_scope, None);
                self.out.push_str("(");
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 { self.out.push_str(", "); }
                    self.gen_expr(arg, current_scope, None);
                }
                self.out.push_str(")");
            }
            Expr::FieldAccess { base, field } => {
                self.gen_expr(base, current_scope, None);
                self.out.push_str(&format!("->{}", field)); 
            }
            Expr::ListLiteral(elements) => {
                self.out.push_str("({ void* __list = lpp_list_new(); ");
                for el in elements {
                    self.out.push_str("lpp_list_push(__list, (int64_t)(");
                    self.gen_expr(el, current_scope, None);
                    self.out.push_str(")); ");
                }
                self.out.push_str("__list; })");
            }
            Expr::Spawn { closure } => {
                self.out.push_str("/* spawn thread */ ");
                self.gen_expr(closure, current_scope, None);
            }
            Expr::Closure { params, body, return_type } => {
                let mut closure_scope = None;
                for i in self.closure_scope_idx..self.symbol_table.scopes.len() {
                    if let ScopeKind::Closure { .. } = self.symbol_table.scopes[i].kind {
                        closure_scope = Some(crate::semantic::ScopeId(i));
                        self.closure_scope_idx = i + 1;
                        break;
                    }
                }
                let scope_id = closure_scope.expect("Closure scope not found");

                let ret_ty_str = if let Some(t) = return_type {
                    self.c_type_ast(t)
                } else {
                    "int64_t".to_string()
                };

                self.out.push_str(&format!("({{ {} __closure(", ret_ty_str));
                if params.is_empty() {
                    self.out.push_str("void");
                }
                for (i, p) in params.iter().enumerate() {
                    if i > 0 { self.out.push_str(", "); }
                    let mut unique_name = p.name.clone();
                    if let Some(binding_id) = self.symbol_table.resolve_name_immutable(scope_id, &p.name) {
                        unique_name = format!("{}_{}", p.name, binding_id.0);
                    }
                    let p_ty = if let Some(t) = &p.ty { self.c_type_ast(t) } else { "int64_t".to_string() };
                    self.out.push_str(&format!("{} {}", p_ty, unique_name));
                }
                self.out.push_str(") {\n");
                for stmt in body {
                    self.gen_stmt(stmt, scope_id);
                }
                self.out.push_str("    } __closure; })");
            }
        }
    }

    fn c_type(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Int => "int64_t".to_string(),
            TypeRef::Str => "char*".to_string(),
            TypeRef::Void => "void".to_string(),
            TypeRef::Bool => "bool".to_string(),
            TypeRef::Custom(id) => format!("{}_t*", self.type_table.definitions[id.0].name),
            TypeRef::Generic(_, _) => "void*".to_string(), // simple stub
            TypeRef::Unresolved(n) => n.clone(),
            TypeRef::Function => "void*".to_string(),
        }
    }

    fn c_type_ast(&self, ty: &Type) -> String {
        match ty {
            Type::Int => "int64_t".to_string(),
            Type::String => "char*".to_string(),
            Type::Void => "void".to_string(),
            Type::Custom(n) => format!("{}_t*", n),
            Type::Generic(_, _) => "void*".to_string(),
        }
    }

    fn expr_type(&self, expr: &Expr, scope: crate::semantic::ScopeId) -> TypeRef {
        match expr {
            Expr::IntLiteral(_) => TypeRef::Int,
            Expr::StringLiteral(_) => TypeRef::Str,
            Expr::Identifier(name, _) => {
                if let Some(binding_id) = self.symbol_table.resolve_name_immutable(scope, name) {
                    if let Some(ref ty) = self.symbol_table.bindings[binding_id.0].ty {
                        return ty.clone();
                    }
                }
                match name.as_str() {
                    "input" | "read_file" | "json_get_str" => TypeRef::Str,
                    _ => TypeRef::Int,
                }
            }
            Expr::BinaryOp { op, left, .. } => {
                match op {
                    BinaryOperator::Add | BinaryOperator::Subtract | 
                    BinaryOperator::Multiply | BinaryOperator::Divide | BinaryOperator::Modulo => {
                        self.expr_type(left, scope)
                    }
                    _ => TypeRef::Bool,
                }
            }
            Expr::Call { callee, .. } => {
                if let Expr::Identifier(name, _) = &**callee {
                    match name.as_str() {
                        "input" | "read_file" | "json_get_str" => return TypeRef::Str,
                        "print" | "print_str" | "write_file" | "json_free" | "list_push" | "list_free" => return TypeRef::Void,
                        "parse_int" | "json_parse" | "json_get_int" | "json_get_obj" | "list_get" | "list_len" => return TypeRef::Int,
                        "list_new" => return TypeRef::Generic("List".into(), vec![TypeRef::Int]),
                        _ => {}
                    }
                    if self.type_table.structs_by_name.contains_key(name) {
                        if let Some(&id) = self.type_table.structs_by_name.get(name) {
                            return TypeRef::Custom(id);
                        }
                    }
                    if let Some(binding_id) = self.symbol_table.resolve_name_immutable(crate::semantic::ScopeId(0), name) {
                        if let Some(ref ty) = self.symbol_table.bindings[binding_id.0].ty {
                            return ty.clone();
                        }
                    }
                }
                TypeRef::Int
            }
            Expr::FieldAccess { base, field } => {
                let base_ty = self.expr_type(base, scope);
                if let TypeRef::Custom(struct_id) = base_ty {
                    let struct_def = &self.type_table.definitions[struct_id.0];
                    if let Some(field_entry) = struct_def.fields.iter().find(|(name, _)| name == field) {
                        return field_entry.1.clone();
                    }
                }
                TypeRef::Int
            }
            Expr::ListLiteral(elements) => {
                let mut elem_ty = TypeRef::Int;
                if !elements.is_empty() {
                    elem_ty = self.expr_type(&elements[0], scope);
                }
                TypeRef::Generic("List".to_string(), vec![elem_ty])
            }
            Expr::Closure { .. } => TypeRef::Function,
            Expr::Spawn { .. } => TypeRef::Void,
        }
    }
}
