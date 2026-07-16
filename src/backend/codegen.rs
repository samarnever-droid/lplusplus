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
                self.out.push_str(&format!("    __auto_type {} = ", unique_name));
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
                            for (i, _) in args.iter().enumerate() {
                                if i > 0 { self.out.push_str(" "); }
                                self.out.push_str("%d");
                            }
                            self.out.push_str("\\n\"");
                            for arg in args {
                                self.out.push_str(", (int)(");
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
                        self.out.push_str("({ char buffer[1024]; if(fgets(buffer, sizeof(buffer), stdin)){ buffer[strcspn(buffer, \"\\n\")] = 0; } else { buffer[0]=0; } char* res = malloc(strlen(buffer)+1); strcpy(res, buffer); res; })");
                        return;
                    }
                    if n == "read_file" {
                        self.out.push_str("({ char* res = NULL; FILE* f = fopen(");
                        self.gen_expr(&args[0], current_scope, None);
                        self.out.push_str(", \"rb\"); if(f){ fseek(f, 0, SEEK_END); long fsize = ftell(f); fseek(f, 0, SEEK_SET); res = malloc(fsize + 1); fread(res, fsize, 1, f); fclose(f); res[fsize] = 0; } else { res = malloc(1); res[0] = 0; } res; })");
                        return;
                    }
                    if n == "write_file" {
                        self.out.push_str("({ FILE* f = fopen(");
                        self.gen_expr(&args[0], current_scope, None);
                        self.out.push_str(", \"wb\"); if(f){ const char* data = ");
                        self.gen_expr(&args[1], current_scope, None);
                        self.out.push_str("; fwrite(data, 1, strlen(data), f); fclose(f); } 0; })");
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
                self.out.push_str("({ void** __list = (void**)calloc(");
                self.out.push_str(&elements.len().to_string());
                self.out.push_str(", sizeof(void*)); ");
                for (i, el) in elements.iter().enumerate() {
                    self.out.push_str(&format!("__list[{}] = (void*)(intptr_t)", i));
                    self.gen_expr(el, current_scope, None);
                    self.out.push_str("; ");
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
}
