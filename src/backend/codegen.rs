use crate::ast::*;
use crate::escape::StorageClass;
use crate::semantic::{BindingId, ScopeKind, SymbolTable};
use crate::typecheck::{TypeRef, TypeTable};
use std::collections::HashMap;

pub struct Codegen<'a> {
    symbol_table: &'a SymbolTable,
    type_table: &'a TypeTable,
    storage: &'a HashMap<BindingId, StorageClass>,
    /// TODO-10: accumulates lifted static closure/spawn functions (emitted before user code)
    preamble: String,
    out: String,
    closure_scope_idx: usize,
    /// TODO-10: unique counter for lifted closure/spawn function names
    closure_lift_count: usize,
    /// TODO-15: current indentation level for generated C code
    indent_depth: usize,
    /// TODO-10: list literal pre-init statements flushed before the enclosing statement
    pre_stmts: Vec<String>,
    /// TODO-10: counter for unique list-temp variable names
    tmp_count: usize,
}

impl<'a> Codegen<'a> {
    pub fn new(
        symbol_table: &'a SymbolTable,
        type_table: &'a TypeTable,
        storage: &'a HashMap<BindingId, StorageClass>,
    ) -> Self {
        Self {
            symbol_table,
            type_table,
            storage,
            preamble: String::new(),
            out: String::new(),
            closure_scope_idx: 0,
            closure_lift_count: 0,
            indent_depth: 1,
            pre_stmts: Vec::new(),
            tmp_count: 0,
        }
    }

    /// TODO-15: return the current indentation string.
    fn indent(&self) -> String {
        "    ".repeat(self.indent_depth)
    }

    /// TODO-10: Capture the output of gen_expr into a String without writing to self.out.
    /// Allows callers to inspect/redirect the generated expression text.
    fn gen_expr_str(
        &mut self,
        expr: &Expr,
        current_scope: crate::semantic::ScopeId,
        target_class: Option<&StorageClass>,
    ) -> String {
        let saved = std::mem::take(&mut self.out);
        self.gen_expr(expr, current_scope, target_class);
        let result = std::mem::take(&mut self.out);
        self.out = saved;
        result
    }

    /// Flush pre_stmts (list-literal init lines) into self.out.
    fn flush_pre_stmts(&mut self) {
        let stmts: Vec<String> = std::mem::take(&mut self.pre_stmts);
        for s in stmts {
            self.out.push_str(&s);
        }
    }

    pub fn generate(&mut self, program: &Program) -> String {
        // Must be before every system header so strict C builds expose POSIX
        // networking declarations used by the embedded runtime.
        self.out.push_str("#if !defined(_WIN32) && !defined(_POSIX_C_SOURCE)\n#  define _POSIX_C_SOURCE 200112L\n#endif\n");
        self.out.push_str("#include <stdio.h>\n");
        self.out.push_str("#include <stdlib.h>\n");
        self.out.push_str("#include <stdint.h>\n");
        self.out.push_str("#include <limits.h>\n");
        self.out.push_str("#include <string.h>\n");
        self.out.push_str("#include <math.h>\n");
        // TODO-10: MSVC uses <malloc.h> for alloca; GCC/Clang use <alloca.h>
        self.out.push_str(
            "#if defined(_MSC_VER)\n#  include <malloc.h>\n#else\n#  include <alloca.h>\n#endif\n",
        );
        self.out.push_str("\n");
        self.out.push_str(crate::c_runtime_headers::C_BUILTINS_IO);
        self.out.push_str(crate::c_runtime_headers::C_BUILTINS_JSON);

        // Emitting structs
        for def in &self.type_table.definitions {
            self.out
                .push_str(&format!("typedef struct {} {}_t;\n", def.name, def.name));
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

        // Type-specific ARC destructors. C compatibility mode uses the same
        // destructor-callback ABI as Cranelift: freeing a parent releases all
        // managed child edges before the parent allocation is reclaimed.
        for def in &self.type_table.definitions {
            self.out.push_str(&format!(
                "static void lpp_drop_{}(void* raw) {{\n    {}_t* self = ({}_t*)raw;\n",
                def.name, def.name, def.name
            ));
            for (field_name, field_ty) in &def.fields {
                if matches!(field_ty, TypeRef::Custom(_) | TypeRef::Generic(_, _)) {
                    self.out
                        .push_str(&format!("    lpp_arc_release(self->{});\n", field_name));
                }
            }
            self.out.push_str("}\n\n");
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
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    self.out
                        .push_str(&format!("{} {}", self.c_type_ast(&p.ty), p.name));
                }
                if f.params.is_empty() {
                    self.out.push_str("void");
                }
                self.out.push_str(");\n");
            }
        }
        self.out.push_str("\n");

        // Lifted closures are discovered while generating function bodies, but
        // their declarations must appear after headers/prototypes and before
        // any user function that refers to them. Keep this completed prefix
        // separate so it can be assembled in that safe C order at the end.
        let prefix = std::mem::take(&mut self.out);

        // Emitting functions
        for decl in &program.declarations {
            if let TopLevel::Function(f) = decl {
                self.gen_function(f);
            }
        }

        // Adding C main
        self.out.push_str("int main() {\n");
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

        // Lifted closures/spawn wrappers need the standard headers and user
        // prototypes above, and must be declared before user function bodies.
        let preamble = std::mem::take(&mut self.preamble);
        let body = std::mem::take(&mut self.out);
        if preamble.is_empty() {
            prefix + &body
        } else {
            prefix + "\n" + &preamble + "\n" + &body
        }
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
            if i > 0 {
                self.out.push_str(", ");
            }
            let mut unique_name = p.name.clone();
            if let Some(scope_id) = func_scope {
                if let Some(binding_id) =
                    self.symbol_table.resolve_name_immutable(scope_id, &p.name)
                {
                    unique_name = format!("{}_{}", p.name, binding_id.0);
                }
            }
            self.out
                .push_str(&format!("{} {}", self.c_type_ast(&p.ty), unique_name));
        }
        if f.params.is_empty() {
            self.out.push_str("void");
        }
        self.out.push_str(") {\n");

        self.indent_depth = 1; // TODO-15: reset depth for each function
        if let Some(scope_id) = func_scope {
            for stmt in &f.body {
                self.gen_stmt(stmt, scope_id);
            }
        }
        self.out.push_str("}\n\n");
    }

    fn gen_stmt(&mut self, stmt: &Stmt, current_scope: crate::semantic::ScopeId) {
        let ind = self.indent();
        match stmt {
            Stmt::LetInferred {
                name,
                value,
                binding_id,
                ..
            } => {
                let id = match binding_id.get() {
                    Some(id) => id,
                    None => return,
                };
                let class = self
                    .storage
                    .get(&crate::semantic::BindingId(id))
                    .unwrap_or(&StorageClass::Value)
                    .clone();
                let unique_name = format!("{}_{}", name, id);
                let binding = &self.symbol_table.bindings[id];
                let ty_str = if let Some(ty) = &binding.ty {
                    self.c_type(ty)
                } else {
                    "int64_t".to_string()
                };
                // TODO-15: use depth-aware indent
                self.out
                    .push_str(&format!("{}/* Storage: {:?} */\n", ind, class));
                let val_str = self.gen_expr_str(value, current_scope, Some(&class));
                // TODO-10: flush any list-literal pre-init before the declaration
                self.flush_pre_stmts();
                if matches!(value, Expr::Closure { .. })
                    || matches!(binding.ty, Some(TypeRef::Function))
                {
                    // The current type table infers a closure expression's
                    // return type, not its callable signature. Detect the AST
                    // closure at its binding site so generated C stores a real
                    // function pointer rather than an integer/void pointer.
                    // The supported C closure subset returns Int; the old-style
                    // pointer declaration accepts its inferred parameter list.
                    self.out.push_str(&format!(
                        "{}int64_t (*{})() = {};\n",
                        ind, unique_name, val_str
                    ));
                } else {
                    self.out.push_str(&format!(
                        "{}{} {} = {};\n",
                        ind, ty_str, unique_name, val_str
                    ));
                }
            }
            Stmt::Assign {
                name,
                value,
                binding_id,
                ..
            } => {
                let id = match binding_id.get() {
                    Some(id) => id,
                    None => return,
                };
                let unique_name = format!("{}_{}", name, id);
                let val_str = self.gen_expr_str(value, current_scope, None);
                self.flush_pre_stmts();
                self.out
                    .push_str(&format!("{}{} = {};\n", ind, unique_name, val_str));
            }
            Stmt::AssignField { base, field, value } => {
                let base_str = self.gen_expr_str(base, current_scope, None);
                let val_str = self.gen_expr_str(value, current_scope, None);
                self.flush_pre_stmts();
                self.out
                    .push_str(&format!("{}{}->{} = {};\n", ind, base_str, field, val_str));
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
            } => {
                let cond_str = self.gen_expr_str(condition, current_scope, None);
                self.flush_pre_stmts();
                // TODO-15: proper nested indentation for if/else bodies
                self.out.push_str(&format!("{}if ({}) {{\n", ind, cond_str));
                self.indent_depth += 1;
                for stmt in then_block {
                    self.gen_stmt(stmt, current_scope);
                }
                self.indent_depth -= 1;
                self.out.push_str(&format!("{}}}", ind));
                if let Some(else_b) = else_block {
                    self.out.push_str(" else {\n");
                    self.indent_depth += 1;
                    for stmt in else_b {
                        self.gen_stmt(stmt, current_scope);
                    }
                    self.indent_depth -= 1;
                    self.out.push_str(&format!("{}}}\n", ind));
                } else {
                    self.out.push_str("\n");
                }
            }
            Stmt::While { condition, body } => {
                let cond_str = self.gen_expr_str(condition, current_scope, None);
                self.flush_pre_stmts();
                // TODO-15: proper nested indentation for while body
                self.out
                    .push_str(&format!("{}while ({}) {{\n", ind, cond_str));
                self.indent_depth += 1;
                for stmt in body {
                    self.gen_stmt(stmt, current_scope);
                }
                self.indent_depth -= 1;
                self.out.push_str(&format!("{}}}\n", ind));
            }
            Stmt::ForRange {
                var_name,
                start,
                end,
                body,
                binding_id,
            } => {
                let unique_name = if let Some(id) = binding_id.get() {
                    format!("{}_{}", var_name, id)
                } else {
                    var_name.clone()
                };
                let start_str = self.gen_expr_str(start, current_scope, None);
                let end_str = self.gen_expr_str(end, current_scope, None);
                self.flush_pre_stmts();
                self.out.push_str(&format!(
                    "{}for (int64_t {} = {}; {} < {}; {}++) {{\n",
                    ind, unique_name, start_str, unique_name, end_str, unique_name
                ));
                self.indent_depth += 1;
                for s in body {
                    self.gen_stmt(s, current_scope);
                }
                self.indent_depth -= 1;
                self.out.push_str(&format!("{}}}\n", ind));
            }
            Stmt::ForIn {
                var_name,
                list,
                body,
                binding_id,
            } => {
                let unique_name = if let Some(id) = binding_id.get() {
                    format!("{}_{}", var_name, id)
                } else {
                    var_name.clone()
                };
                let list_str = self.gen_expr_str(list, current_scope, None);
                let tmp_list = format!("__lpp_for_list_{}", self.tmp_count);
                let tmp_len = format!("__lpp_for_len_{}", self.tmp_count);
                let tmp_i = format!("__lpp_for_i_{}", self.tmp_count);
                self.tmp_count += 1;
                self.flush_pre_stmts();

                let list_ty = self.expr_type(list, current_scope);
                let elem_ty = match &list_ty {
                    TypeRef::Generic(name, params) if name == "List" && !params.is_empty() => {
                        params[0].clone()
                    }
                    _ => TypeRef::Int,
                };
                let c_elem_type = self.c_type(&elem_ty);
                let is_arc_elem = matches!(elem_ty, TypeRef::Custom(_) | TypeRef::Str | TypeRef::Bool);

                self.out.push_str(&format!("{}void* {} = {};\n", ind, tmp_list, list_str));
                self.out.push_str(&format!("{}int64_t {} = lpp_list_len({});\n", ind, tmp_len, tmp_list));
                self.out.push_str(&format!(
                    "{}for (int64_t {} = 0; {} < {}; {}++) {{\n",
                    ind, tmp_i, tmp_i, tmp_len, tmp_i
                ));
                self.indent_depth += 1;
                let ind_inner = self.indent();
                let get_fn = if is_arc_elem { "(void*)lpp_list_get_arc" } else { "lpp_list_get" };
                self.out.push_str(&format!(
                    "{}{} {} = ({}){}({}, {});\n",
                    ind_inner, c_elem_type, unique_name, c_elem_type, get_fn, tmp_list, tmp_i
                ));
                for s in body {
                    self.gen_stmt(s, current_scope);
                }
                self.indent_depth -= 1;
                self.out.push_str(&format!("{}}}\n", ind));
            }
            Stmt::Break => {
                self.out.push_str(&format!("{}break;\n", ind));
            }
            Stmt::Continue => {
                self.out.push_str(&format!("{}continue;\n", ind));
            }
            Stmt::Block(stmts) => {
                for stmt in stmts {
                    self.gen_stmt(stmt, current_scope);
                }
            }
            Stmt::Expr(expr) => {
                let expr_str = self.gen_expr_str(expr, current_scope, None);
                self.flush_pre_stmts();
                self.out.push_str(&format!("{}{};\n", ind, expr_str));
            }
            Stmt::Return(Some(e)) => {
                let e_str = self.gen_expr_str(e, current_scope, None);
                self.flush_pre_stmts();
                self.out.push_str(&format!("{}return {};\n", ind, e_str));
            }
            Stmt::Return(None) => {
                self.out.push_str(&format!("{}return;\n", ind));
            }
        }
    }

    fn gen_expr(
        &mut self,
        expr: &Expr,
        current_scope: crate::semantic::ScopeId,
        target_class: Option<&StorageClass>,
    ) {
        match expr {
            Expr::IntLiteral(i) => self.out.push_str(&i.to_string()),
            Expr::FloatLiteral(v) => self.out.push_str(&v.to_string()),
            Expr::StringLiteral(s) => self.out.push_str(&format!("\"{}\"", s)),
            Expr::BoolLiteral(b) => self.out.push_str(if *b { "1" } else { "0" }),
            Expr::Identifier(n, binding_cell) => {
                if let Some(id) = binding_cell.get() {
                    // Top-level function names have stable C linkage. Local
                    // variables are uniquely mangled to preserve shadowing.
                    if matches!(
                        self.symbol_table.bindings[id].kind,
                        crate::semantic::BindingKind::FunctionName
                    ) {
                        self.out.push_str(n);
                    } else {
                        self.out.push_str(&format!("{}_{}", n, id));
                    }
                } else {
                    self.out.push_str(n);
                }
            }
            Expr::BinaryOp { left, op, right } => {
                let left_ty = self.expr_type(left, current_scope);
                if *op == crate::ast::BinaryOperator::Modulo && left_ty == TypeRef::Float {
                    self.out.push_str("fmod(");
                    self.gen_expr(left, current_scope, target_class);
                    self.out.push_str(", ");
                    self.gen_expr(right, current_scope, target_class);
                    self.out.push_str(")");
                    return;
                }
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
                                if i > 0 {
                                    self.out.push_str(" ");
                                }
                                let ty = self.expr_type(arg, current_scope);
                                if ty == TypeRef::Str {
                                    self.out.push_str("%s");
                                } else if ty == TypeRef::Float {
                                    self.out.push_str("%f");
                                } else {
                                    self.out.push_str("%lld");
                                }
                            }
                            self.out.push_str("\\n\"");
                            for arg in args {
                                let ty = self.expr_type(arg, current_scope);
                                if ty == TypeRef::Str {
                                    self.out.push_str(", (char*)(");
                                } else if ty == TypeRef::Float {
                                    self.out.push_str(", (double)(");
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
                    if (n == "list_push" || n == "list_get" || n == "push" || n == "get") && !args.is_empty() {
                        let list_ty = self.expr_type(&args[0], current_scope);
                        let is_arc_list = matches!(
                            list_ty,
                            TypeRef::Generic(_, ref params)
                                if matches!(
                                    params.first(),
                                    Some(TypeRef::Custom(_) | TypeRef::Str | TypeRef::Bool)
                                )
                        );
                        if is_arc_list {
                            let symbol = if n == "list_push" || n == "push" {
                                "lpp_list_push_arc"
                            } else {
                                "lpp_list_get_arc"
                            };
                            if n == "list_get" || n == "get" {
                                self.out.push_str("((void*)");
                            }
                            self.out.push_str(symbol);
                            self.out.push_str("(");
                            for (i, arg) in args.iter().enumerate() {
                                if i > 0 {
                                    self.out.push_str(", ");
                                }
                                self.gen_expr(arg, current_scope, None);
                            }
                            self.out.push_str(")");
                            if n == "list_get" || n == "get" {
                                self.out.push_str(")");
                            }
                            return;
                        }
                    }
                    if n != "print" {
                        if let Some(builtin) =
                            crate::builtins::get_builtins().iter().find(|b| b.name == n)
                        {
                            if !builtin.symbol.is_empty() {
                                self.out.push_str(&format!("{}(", builtin.symbol));
                                for (i, arg) in args.iter().enumerate() {
                                    if i > 0 {
                                        self.out.push_str(", ");
                                    }
                                    self.gen_expr(arg, current_scope, None);
                                }
                                self.out.push_str(")");
                                return;
                            }
                        }
                    }
                    if let Some(&id) = self.type_table.structs_by_name.get(n) {
                        let def = &self.type_table.definitions[id.0];
                        let alloc_str = match target_class {
                            Some(StorageClass::Value) => {
                                format!("({}_t*)alloca(sizeof({}_t))", def.name, def.name)
                            }
                            Some(StorageClass::Arc) => format!(
                                "({}_t*)lpp_arc_alloc_with_destructor(sizeof({}_t), lpp_drop_{})",
                                def.name, def.name, def.name
                            ),
                            Some(StorageClass::Arena { .. }) => format!(
                                "({}_t*)malloc(sizeof({}_t)) /* arena */",
                                def.name, def.name
                            ),
                            None => {
                                format!("({}_t*)lpp_arc_alloc(sizeof({}_t))", def.name, def.name)
                            }
                        };
                        if args.is_empty() {
                            self.out.push_str(&alloc_str);
                        } else {
                            let tmp = format!("__lpp_struct_{}", self.tmp_count);
                            self.tmp_count += 1;
                            let ind = self.indent();
                            let c_type = format!("{}_t*", def.name);
                            self.pre_stmts.push(format!(
                                "{}{} {} = {};\n",
                                ind, c_type, tmp, alloc_str
                            ));
                            for (i, arg) in args.iter().enumerate() {
                                if i < def.fields.len() {
                                    let arg_str = self.gen_expr_str(arg, current_scope, None);
                                    let field_name = &def.fields[i].0;
                                    self.pre_stmts.push(format!(
                                        "{}{}->{} = {};\n",
                                        ind, tmp, field_name, arg_str
                                    ));
                                }
                            }
                            self.out.push_str(&tmp);
                        }
                        return;
                    }
                }
                // Generic call (user-defined function or closure variable)
                self.gen_expr(callee, current_scope, None);
                self.out.push_str("(");
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    self.gen_expr(arg, current_scope, None);
                }
                self.out.push_str(")");
            }
            Expr::FieldAccess { base, field } => {
                self.gen_expr(base, current_scope, None);
                self.out.push_str(&format!("->{}", field));
            }
            Expr::ListLiteral(elements) => {
                // TODO-10: MSVC-safe — use a named temp variable via pre_stmts
                // instead of the GCC-only ({ ... }) statement expression.
                let tmp = format!("__lpp_list_{}", self.tmp_count);
                self.tmp_count += 1;
                let ind = self.indent();
                // Capture element expressions before pushing to pre_stmts
                let mut elem_strs: Vec<String> = Vec::new();
                for el in elements {
                    elem_strs.push(self.gen_expr_str(el, current_scope, None));
                }
                let element_ty = elements
                    .first()
                    .map(|element| self.expr_type(element, current_scope))
                    .unwrap_or(TypeRef::Int);
                let is_arc_element =
                    matches!(element_ty, TypeRef::Custom(_) | TypeRef::Str | TypeRef::Bool);
                self.pre_stmts.push(format!(
                    "{}void* {} = {}();\n",
                    ind,
                    tmp,
                    if is_arc_element {
                        "lpp_list_new_arc"
                    } else {
                        "lpp_list_new"
                    }
                ));
                for el_str in elem_strs {
                    if is_arc_element {
                        self.pre_stmts.push(format!(
                            "{}lpp_list_push_arc({}, (void*)({}));\n",
                            ind, tmp, el_str
                        ));
                    } else {
                        self.pre_stmts.push(format!(
                            "{}lpp_list_push({}, (int64_t)({}));\n",
                            ind, tmp, el_str
                        ));
                    }
                }
                self.out.push_str(&tmp);
            }
            Expr::Spawn { closure } => {
                // TODO-12: actually call lpp_thread_spawn with a proper wrapper + env struct
                self.gen_spawn(closure, current_scope);
            }
            Expr::Closure {
                params,
                body,
                return_type,
            } => {
                // TODO-10: lift to a static top-level function — MSVC compatible
                self.gen_lifted_closure(params, body, return_type, current_scope);
            }
        }
    }

    // ── TODO-10: Closure lifting ───────────────────────────────────────────────

    /// Lift a closure to a static top-level function in `preamble`.
    /// Emits a function pointer (or trampoline for closures with captures) into self.out.
    fn gen_lifted_closure(
        &mut self,
        params: &[ClosureParam],
        body: &[Stmt],
        return_type: &Option<Type>,
        _current_scope: crate::semantic::ScopeId,
    ) {
        // Find the closure scope
        let scope_id = self.next_closure_scope();

        let captures: Vec<BindingId> =
            if let ScopeKind::Closure { captures } = &self.symbol_table.scopes[scope_id.0].kind {
                captures.clone()
            } else {
                Vec::new()
            };

        let idx = self.closure_lift_count;
        self.closure_lift_count += 1;
        let fn_name = format!("lpp__fn_{}", idx);

        let ret_ty = if let Some(t) = return_type {
            self.c_type_ast(t)
        } else {
            "int64_t".to_string()
        };
        let has_env = !captures.is_empty();
        let env_type = format!("lpp__env_{}_t", idx);

        if has_env {
            // Emit the capture-environment struct
            self.preamble.push_str(&format!("typedef struct {{\n"));
            for &cap_id in &captures {
                let b = &self.symbol_table.bindings[cap_id.0];
                let ty = if let Some(ty) = &b.ty {
                    self.c_type(ty)
                } else {
                    "int64_t".to_string()
                };
                self.preamble
                    .push_str(&format!("    {} {}_{};\n", ty, b.name, cap_id.0));
            }
            self.preamble.push_str(&format!("}} {};\n", env_type));
            // Static current-env pointer (trampoline; single-threaded call without explicit env)
            self.preamble.push_str(&format!(
                "static {}* lpp__cur_env_{} = NULL;\n\n",
                env_type, idx
            ));
        }

        // Emit the static function
        self.preamble
            .push_str(&format!("static {} {}(", ret_ty, fn_name));
        if has_env {
            self.preamble.push_str(&format!("{}* __env", env_type));
            if !params.is_empty() {
                self.preamble.push_str(", ");
            }
        }
        if params.is_empty() && !has_env {
            self.preamble.push_str("void");
        }
        for (i, p) in params.iter().enumerate() {
            if i > 0 {
                self.preamble.push_str(", ");
            }
            let uname = self.param_unique_name(scope_id, p);
            let pty = if let Some(t) = &p.ty {
                self.c_type_ast(t)
            } else {
                "int64_t".to_string()
            };
            self.preamble.push_str(&format!("{} {}", pty, uname));
        }
        self.preamble.push_str(") {\n");

        // Unpack captures inside the function body
        if has_env {
            for &cap_id in &captures {
                let b = &self.symbol_table.bindings[cap_id.0];
                let ty = if let Some(ty) = &b.ty {
                    self.c_type(ty)
                } else {
                    "int64_t".to_string()
                };
                self.preamble.push_str(&format!(
                    "    {} {}_{} = __env->{}_{};\n",
                    ty, b.name, cap_id.0, b.name, cap_id.0
                ));
            }
        }

        // Generate body into preamble
        let saved_out = std::mem::take(&mut self.out);
        let saved_depth = self.indent_depth;
        self.indent_depth = 1;
        for stmt in body {
            self.gen_stmt(stmt, scope_id);
        }
        let body_code = std::mem::take(&mut self.out);
        self.out = saved_out;
        self.indent_depth = saved_depth;
        self.preamble.push_str(&body_code);
        self.preamble.push_str("}\n\n");

        if has_env {
            // Emit a trampoline that reads from the static env pointer.
            // This allows calling the closure as a plain function pointer
            // (single-threaded use only — for spawn, see gen_spawn).
            let tramp = format!("lpp__trampoline_{}", idx);
            self.preamble
                .push_str(&format!("static {} {}(", ret_ty, tramp));
            if params.is_empty() {
                self.preamble.push_str("void");
            }
            for (i, p) in params.iter().enumerate() {
                if i > 0 {
                    self.preamble.push_str(", ");
                }
                let uname = self.param_unique_name(scope_id, p);
                let pty = if let Some(t) = &p.ty {
                    self.c_type_ast(t)
                } else {
                    "int64_t".to_string()
                };
                self.preamble.push_str(&format!("{} {}", pty, uname));
            }
            self.preamble.push_str(") {\n");
            self.preamble
                .push_str(&format!("    return {}(lpp__cur_env_{}", fn_name, idx));
            for p in params {
                let uname = self.param_unique_name(scope_id, p);
                self.preamble.push_str(&format!(", {}", uname));
            }
            self.preamble.push_str(");\n}\n\n");

            // At the closure site: allocate env, fill captures, set trampoline env, emit trampoline ptr
            let ind = self.indent();
            let env_var = format!("__lpp_env_{}", idx);
            self.pre_stmts.push(format!(
                "{}{}* {} = ({}*)malloc(sizeof({}));\n",
                ind, env_type, env_var, env_type, env_type
            ));
            for &cap_id in &captures {
                let b = &self.symbol_table.bindings[cap_id.0];
                self.pre_stmts.push(format!(
                    "{}{}->{}_{}  = {}_{};\n",
                    ind, env_var, b.name, cap_id.0, b.name, cap_id.0
                ));
            }
            self.pre_stmts
                .push(format!("{}lpp__cur_env_{} = {};\n", ind, idx, env_var));
            self.out.push_str(&tramp);
        } else {
            // No captures — emit plain function pointer
            self.out.push_str(&fn_name);
        }
    }

    // ── TODO-12: spawn → lpp_thread_spawn ─────────────────────────────────────

    /// Generate a real `lpp_thread_spawn` call for a `spawn` expression.
    fn gen_spawn(&mut self, closure: &Expr, _current_scope: crate::semantic::ScopeId) {
        if let Expr::Closure {
            params,
            body,
            return_type,
        } = closure
        {
            let scope_id = self.next_closure_scope();

            let captures: Vec<BindingId> = if let ScopeKind::Closure { captures } =
                &self.symbol_table.scopes[scope_id.0].kind
            {
                captures.clone()
            } else {
                Vec::new()
            };

            let idx = self.closure_lift_count;
            self.closure_lift_count += 1;
            let fn_name = format!("lpp__spawn_fn_{}", idx);
            let wrapper_name = format!("lpp__spawn_wrapper_{}", idx);
            let env_type = format!("lpp__spawn_env_{}_t", idx);

            let _ret_ty = if let Some(t) = return_type {
                self.c_type_ast(t)
            } else {
                "void".to_string()
            };

            // Env struct (empty structs get a dummy member to satisfy C89/MSVC)
            self.preamble.push_str("typedef struct {\n");
            if captures.is_empty() {
                self.preamble.push_str("    char _dummy;\n");
            }
            for &cap_id in &captures {
                let b = &self.symbol_table.bindings[cap_id.0];
                let ty = if let Some(ty) = &b.ty {
                    self.c_type(ty)
                } else {
                    "int64_t".to_string()
                };
                self.preamble
                    .push_str(&format!("    {} {}_{};\n", ty, b.name, cap_id.0));
            }
            self.preamble.push_str(&format!("}} {};\n\n", env_type));

            // Thread function (takes env* and no user params; spawn closures rarely have params)
            self.preamble
                .push_str(&format!("static void {}({}* __env", fn_name, env_type));
            for p in params {
                let uname = self.param_unique_name(scope_id, p);
                let pty = if let Some(t) = &p.ty {
                    self.c_type_ast(t)
                } else {
                    "int64_t".to_string()
                };
                self.preamble.push_str(&format!(", {} {}", pty, uname));
            }
            self.preamble.push_str(") {\n");
            // Unpack captures
            for &cap_id in &captures {
                let b = &self.symbol_table.bindings[cap_id.0];
                let ty = if let Some(ty) = &b.ty {
                    self.c_type(ty)
                } else {
                    "int64_t".to_string()
                };
                self.preamble.push_str(&format!(
                    "    {} {}_{} = __env->{}_{};\n",
                    ty, b.name, cap_id.0, b.name, cap_id.0
                ));
            }
            // Body
            let saved_out = std::mem::take(&mut self.out);
            let saved_depth = self.indent_depth;
            self.indent_depth = 1;
            for stmt in body {
                self.gen_stmt(stmt, scope_id);
            }
            let body_code = std::mem::take(&mut self.out);
            self.out = saved_out;
            self.indent_depth = saved_depth;
            self.preamble.push_str(&body_code);
            self.preamble.push_str("}\n\n");

            // void(*)(void*) wrapper for lpp_thread_spawn
            self.preamble
                .push_str(&format!("static void {}(void* __raw) {{\n", wrapper_name));
            self.preamble.push_str(&format!(
                "    {}* __env = ({}*)__raw;\n",
                env_type, env_type
            ));
            self.preamble
                .push_str(&format!("    {}(__env);\n", fn_name));
            self.preamble.push_str("    free(__raw);\n}\n\n");

            // At the spawn site: allocate env, fill captures, call lpp_thread_spawn
            let ind = self.indent();
            let env_var = format!("__lpp_senv_{}", idx);
            self.pre_stmts.push(format!(
                "{}{}* {} = ({}*)malloc(sizeof({}));\n",
                ind, env_type, env_var, env_type, env_type
            ));
            for &cap_id in &captures {
                let b = &self.symbol_table.bindings[cap_id.0];
                self.pre_stmts.push(format!(
                    "{}{}->{}_{}  = {}_{};\n",
                    ind, env_var, b.name, cap_id.0, b.name, cap_id.0
                ));
            }
            // The spawn call goes into self.out (will be emitted as the statement body)
            self.out
                .push_str(&format!("lpp_thread_spawn({}, {})", wrapper_name, env_var));
        } else {
            // Non-literal closure — best-effort: just run it inline (same thread)
            self.out.push_str("/* spawn(non-literal) */ ");
            self.gen_expr(closure, _current_scope, None);
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn next_closure_scope(&mut self) -> crate::semantic::ScopeId {
        for i in self.closure_scope_idx..self.symbol_table.scopes.len() {
            if let ScopeKind::Closure { .. } = self.symbol_table.scopes[i].kind {
                self.closure_scope_idx = i + 1;
                return crate::semantic::ScopeId(i);
            }
        }
        panic!("Closure scope not found");
    }

    fn param_unique_name(&self, scope_id: crate::semantic::ScopeId, p: &ClosureParam) -> String {
        if let Some(bid) = self.symbol_table.resolve_name_immutable(scope_id, &p.name) {
            format!("{}_{}", p.name, bid.0)
        } else {
            p.name.clone()
        }
    }

    fn c_type(&self, ty: &TypeRef) -> String {
        match ty {
            TypeRef::Int => "int64_t".to_string(),
            TypeRef::Float => "double".to_string(),
            TypeRef::Str => "char*".to_string(),
            TypeRef::Void => "void".to_string(),
            TypeRef::Bool => "int".to_string(),
            TypeRef::Custom(id) => format!("{}_t*", self.type_table.definitions[id.0].name),
            TypeRef::Generic(_, _) => "void*".to_string(),
            TypeRef::Unresolved(n) => n.clone(),
            TypeRef::Function => "void*".to_string(),
        }
    }

    fn c_type_ast(&self, ty: &Type) -> String {
        match ty {
            Type::Int => "int64_t".to_string(),
            Type::Float => "double".to_string(),
            Type::String => "char*".to_string(),
            Type::Bool => "int".to_string(),
            Type::Void => "void".to_string(),
            Type::Custom(n) => format!("{}_t*", n),
            Type::Generic(_, _) => "void*".to_string(),
        }
    }

    fn expr_type(&self, expr: &Expr, scope: crate::semantic::ScopeId) -> TypeRef {
        match expr {
            Expr::IntLiteral(_) => TypeRef::Int,
            Expr::FloatLiteral(_) => TypeRef::Float,
            Expr::StringLiteral(_) => TypeRef::Str,
            Expr::BoolLiteral(_) => TypeRef::Bool,
            Expr::Identifier(name, binding_id_cell) => {
                if let Some(id) = binding_id_cell.get() {
                    if let Some(ref ty) = self.symbol_table.bindings[id].ty {
                        return ty.clone();
                    }
                }
                if let Some(binding_id) = self.symbol_table.resolve_name_immutable(scope, name) {
                    if let Some(ref ty) = self.symbol_table.bindings[binding_id.0].ty {
                        return ty.clone();
                    }
                }
                match name.as_str() {
                    "input" | "read_file" | "json_get_str" | "net_recv" => TypeRef::Str,
                    _ => TypeRef::Int,
                }
            }
            Expr::BinaryOp { op, left, .. } => match op {
                BinaryOperator::Add
                | BinaryOperator::Subtract
                | BinaryOperator::Multiply
                | BinaryOperator::Divide
                | BinaryOperator::Modulo => self.expr_type(left, scope),
                _ => TypeRef::Bool,
            },
            Expr::Call { callee, args } => {
                if let Expr::Identifier(name, _) = &**callee {
                    if name == "list_get" {
                        if let Some(first) = args.first() {
                            if let TypeRef::Generic(_, params) = self.expr_type(first, scope) {
                                if let Some(element_ty) = params.first() {
                                    return element_ty.clone();
                                }
                            }
                        }
                    }
                    match name.as_str() {
                        "input" | "read_file" | "json_get_str" | "net_recv" => return TypeRef::Str,
                        "print" | "print_str" | "write_file" | "json_free" | "list_push"
                        | "list_free" | "net_close" => return TypeRef::Void,
                        "parse_int" | "json_parse" | "json_get_int" | "json_get_obj"
                        | "list_get" | "list_len" | "file_size" | "file_copy" | "file_move"
                        | "net_connect" | "net_listen" | "net_accept" | "net_send"
                        | "net_send_all" | "net_set_timeout" => return TypeRef::Int,
                        "list_new" => return TypeRef::Generic("List".into(), vec![TypeRef::Int]),
                        _ => {}
                    }
                    if let Some(&id) = self.type_table.structs_by_name.get(name) {
                        return TypeRef::Custom(id);
                    }
                    if let Some(bid) = self
                        .symbol_table
                        .resolve_name_immutable(crate::semantic::ScopeId(0), name)
                    {
                        if let Some(ref ty) = self.symbol_table.bindings[bid.0].ty {
                            return ty.clone();
                        }
                    }
                }
                TypeRef::Int
            }
            Expr::FieldAccess { base, field } => {
                let base_ty = self.expr_type(base, scope);
                if let TypeRef::Custom(struct_id) = base_ty {
                    let def = &self.type_table.definitions[struct_id.0];
                    if let Some(fe) = def.fields.iter().find(|(n, _)| n == field) {
                        return fe.1.clone();
                    }
                }
                TypeRef::Int
            }
            Expr::ListLiteral(elements) => {
                let elem_ty = elements
                    .first()
                    .map(|e| self.expr_type(e, scope))
                    .unwrap_or(TypeRef::Int);
                TypeRef::Generic("List".to_string(), vec![elem_ty])
            }
            Expr::Closure { .. } => TypeRef::Function,
            Expr::Spawn { .. } => TypeRef::Void,
        }
    }
}
