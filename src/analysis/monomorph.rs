use crate::ast::*;
use std::collections::{HashMap, HashSet};

pub struct Monomorphizer {
    specialized_funcs: HashMap<String, Function>,
    specialized_structs: HashMap<String, StructDef>,
    instantiated_keys: HashSet<String>,
}

impl Monomorphizer {
    pub fn new() -> Self {
        Self {
            specialized_funcs: HashMap::new(),
            specialized_structs: HashMap::new(),
            instantiated_keys: HashSet::new(),
        }
    }

    pub fn process_program(program: &mut Program) -> Result<(), String> {
        let mut mono = Monomorphizer::new();
        mono.run(program)?;
        Ok(())
    }

    fn run(&mut self, program: &mut Program) -> Result<(), String> {
        let mut generic_funcs: HashMap<String, Function> = HashMap::new();
        let mut generic_structs: HashMap<String, StructDef> = HashMap::new();

        for decl in &program.declarations {
            if let TopLevel::Function(func) = decl {
                if !func.type_params.is_empty() {
                    generic_funcs.insert(func.name.clone(), func.clone());
                }
            } else if let TopLevel::Struct(s) = decl {
                if !s.type_params.is_empty() {
                    generic_structs.insert(s.name.clone(), s.clone());
                }
            }
        }

        // Collect trait impls for bound checking
        let mut trait_impls: HashMap<String, HashSet<String>> = HashMap::new();
        for decl in &program.declarations {
            if let TopLevel::Impl(impl_block) = decl {
                trait_impls
                    .entry(impl_block.target_type.clone())
                    .or_insert_with(HashSet::new)
                    .insert(impl_block.trait_name.clone());
            }
        }

        if generic_funcs.is_empty() && generic_structs.is_empty() {
            return Ok(());
        }

        // Walk function bodies to discover and rewrite generic call/struct sites
        for decl in &mut program.declarations {
            if let TopLevel::Function(func) = decl {
                self.walk_statements(&mut func.body, &generic_funcs, &generic_structs, &trait_impls);
            } else if let TopLevel::Impl(impl_block) = decl {
                for method in &mut impl_block.methods {
                    self.walk_statements(&mut method.body, &generic_funcs, &generic_structs, &trait_impls);
                }
            }
        }

        // Append generated monomorphized functions & structs to top-level declarations
        for (_, func) in self.specialized_funcs.drain() {
            program.declarations.push(TopLevel::Function(func));
        }
        for (_, struct_def) in self.specialized_structs.drain() {
            program.declarations.push(TopLevel::Struct(struct_def));
        }

        Ok(())
    }

    fn walk_statements(
        &mut self,
        stmts: &mut [Stmt],
        generic_funcs: &HashMap<String, Function>,
        generic_structs: &HashMap<String, StructDef>,
        trait_impls: &HashMap<String, HashSet<String>>,
    ) {
        for stmt in stmts {
            match stmt {
                Stmt::LetInferred { value, .. } => {
                    self.walk_expr(value, generic_funcs, generic_structs, trait_impls);
                }
                Stmt::Assign { value, .. } => {
                    self.walk_expr(value, generic_funcs, generic_structs, trait_impls);
                }
                Stmt::AssignField { base, value, .. } => {
                    self.walk_expr(base, generic_funcs, generic_structs, trait_impls);
                    self.walk_expr(value, generic_funcs, generic_structs, trait_impls);
                }
                Stmt::If { condition, then_block, else_block } => {
                    self.walk_expr(condition, generic_funcs, generic_structs, trait_impls);
                    self.walk_statements(then_block, generic_funcs, generic_structs, trait_impls);
                    if let Some(eb) = else_block {
                        self.walk_statements(eb, generic_funcs, generic_structs, trait_impls);
                    }
                }
                Stmt::While { condition, body } => {
                    self.walk_expr(condition, generic_funcs, generic_structs, trait_impls);
                    self.walk_statements(body, generic_funcs, generic_structs, trait_impls);
                }
                Stmt::ForRange { start, end, body, .. } => {
                    self.walk_expr(start, generic_funcs, generic_structs, trait_impls);
                    self.walk_expr(end, generic_funcs, generic_structs, trait_impls);
                    self.walk_statements(body, generic_funcs, generic_structs, trait_impls);
                }
                Stmt::Return(Some(expr)) => {
                    self.walk_expr(expr, generic_funcs, generic_structs, trait_impls);
                }
                Stmt::Expr(expr) => {
                    self.walk_expr(expr, generic_funcs, generic_structs, trait_impls);
                }
                _ => {}
            }
        }
    }

    fn walk_expr(
        &mut self,
        expr: &mut Expr,
        generic_funcs: &HashMap<String, Function>,
        generic_structs: &HashMap<String, StructDef>,
        trait_impls: &HashMap<String, HashSet<String>>,
    ) {
        match expr {
            Expr::Call { callee, args } => {
                for arg in args.iter_mut() {
                    self.walk_expr(arg, generic_funcs, generic_structs, trait_impls);
                }

                if let Expr::Identifier(ref name, _) = **callee {
                    // Check if calling generic function
                    if let Some(tmpl) = generic_funcs.get(name) {
                        let mut subst_map = HashMap::new();

                        // Infer type parameter bindings from argument types
                        for (i, param) in tmpl.params.iter().enumerate() {
                            if i < args.len() {
                                if let Type::Custom(ref tp_name) = param.ty {
                                    if tmpl.type_params.iter().any(|tp| &tp.name == tp_name) {
                                        let arg_ty = infer_simple_expr_type(&args[i]);
                                        subst_map.insert(tp_name.clone(), arg_ty);
                                    }
                                }
                            }
                        }

                        if !subst_map.is_empty() {
                            // Check trait bounds before monomorphizing
                            for tp in &tmpl.type_params {
                                if let Some(ref bound) = tp.bound {
                                    if let Some(concrete) = subst_map.get(&tp.name) {
                                        let concrete_name = type_to_name(concrete);
                                        let has_impl = trait_impls
                                            .get(&concrete_name)
                                            .map_or(false, |impls| impls.contains(bound));
                                        if !has_impl {
                                            // Skip this instantiation — bound not satisfied
                                            return;
                                        }
                                    }
                                }
                            }

                            let tp_names: Vec<String> = tmpl.type_params.iter().map(|tp| tp.name.clone()).collect();
                            let mangled_name = format!("{}__{}",  name, mangle_types(&subst_map, &tp_names));
                            let key = mangled_name.clone();

                            if !self.instantiated_keys.contains(&key) {
                                self.instantiated_keys.insert(key.clone());

                                let mut specialized = tmpl.clone();
                                specialized.name = mangled_name.clone();
                                specialized.type_params.clear();

                                // Substitute params & return type
                                for p in &mut specialized.params {
                                    substitute_ast_type(&mut p.ty, &subst_map);
                                }
                                substitute_ast_type(&mut specialized.return_type, &subst_map);

                                // Substitute statements body
                                self.substitute_stmts(&mut specialized.body, &subst_map);

                                self.specialized_funcs.insert(key, specialized);
                            }

                            *callee = Box::new(Expr::Identifier(mangled_name, std::cell::Cell::new(None)));
                        }
                    } else if let Some(tmpl) = generic_structs.get(name) {
                        // Struct constructor call: e.g. Box(42)
                        let mut subst_map = HashMap::new();
                        for (i, field) in tmpl.fields.iter().enumerate() {
                            if i < args.len() {
                                if let Type::Custom(ref tp_name) = field.ty {
                                    if tmpl.type_params.iter().any(|tp| &tp.name == tp_name) {
                                        let arg_ty = infer_simple_expr_type(&args[i]);
                                        subst_map.insert(tp_name.clone(), arg_ty);
                                    }
                                }
                            }
                        }

                        if !subst_map.is_empty() {
                            let tp_names: Vec<String> = tmpl.type_params.iter().map(|tp| tp.name.clone()).collect();
                            let mangled_name = format!("{}__{}",  name, mangle_types(&subst_map, &tp_names));
                            let key = mangled_name.clone();

                            if !self.instantiated_keys.contains(&key) {
                                self.instantiated_keys.insert(key.clone());

                                let mut specialized = tmpl.clone();
                                specialized.name = mangled_name.clone();
                                specialized.type_params.clear();

                                for f in &mut specialized.fields {
                                    substitute_ast_type(&mut f.ty, &subst_map);
                                }

                                self.specialized_structs.insert(key, specialized);
                            }

                            *callee = Box::new(Expr::Identifier(mangled_name, std::cell::Cell::new(None)));
                        }
                    }
                }
            }
            Expr::UnaryOp { operand, .. } => {
                self.walk_expr(operand, generic_funcs, generic_structs, trait_impls);
            }
            Expr::BinaryOp { left, right, .. } => {
                self.walk_expr(left, generic_funcs, generic_structs, trait_impls);
                self.walk_expr(right, generic_funcs, generic_structs, trait_impls);
            }
            Expr::FieldAccess { base, .. } => {
                self.walk_expr(base, generic_funcs, generic_structs, trait_impls);
            }
            Expr::ListLiteral(items) => {
                for item in items {
                    self.walk_expr(item, generic_funcs, generic_structs, trait_impls);
                }
            }
            _ => {}
        }
    }

    fn substitute_stmts(&mut self, stmts: &mut [Stmt], map: &HashMap<String, Type>) {
        for stmt in stmts {
            match stmt {
                Stmt::LetInferred { value, .. } => {
                    self.substitute_expr(value, map);
                }
                Stmt::Assign { value, .. } => {
                    self.substitute_expr(value, map);
                }
                Stmt::AssignField { base, value, .. } => {
                    self.substitute_expr(base, map);
                    self.substitute_expr(value, map);
                }
                Stmt::If { condition, then_block, else_block } => {
                    self.substitute_expr(condition, map);
                    self.substitute_stmts(then_block, map);
                    if let Some(eb) = else_block {
                        self.substitute_stmts(eb, map);
                    }
                }
                Stmt::While { condition, body } => {
                    self.substitute_expr(condition, map);
                    self.substitute_stmts(body, map);
                }
                Stmt::ForRange { start, end, body, .. } => {
                    self.substitute_expr(start, map);
                    self.substitute_expr(end, map);
                    self.substitute_stmts(body, map);
                }
                Stmt::Return(Some(expr)) => {
                    self.substitute_expr(expr, map);
                }
                Stmt::Expr(expr) => {
                    self.substitute_expr(expr, map);
                }
                _ => {}
            }
        }
    }

    fn substitute_expr(&mut self, expr: &mut Expr, map: &HashMap<String, Type>) {
        match expr {
            Expr::Call { callee, args } => {
                self.substitute_expr(callee, map);
                for arg in args {
                    self.substitute_expr(arg, map);
                }
            }
            Expr::UnaryOp { operand, .. } => {
                self.substitute_expr(operand, map);
            }
            Expr::BinaryOp { left, right, .. } => {
                self.substitute_expr(left, map);
                self.substitute_expr(right, map);
            }
            Expr::FieldAccess { base, .. } => {
                self.substitute_expr(base, map);
            }
            Expr::ListLiteral(items) => {
                for item in items {
                    self.substitute_expr(item, map);
                }
            }
            _ => {}
        }
    }
}

fn substitute_ast_type(ty: &mut Type, map: &HashMap<String, Type>) {
    match ty {
        Type::Custom(name) => {
            if let Some(subst) = map.get(name) {
                *ty = subst.clone();
            }
        }
        Type::Generic(base, args) => {
            if let Some(subst) = map.get(base) {
                *ty = subst.clone();
            } else {
                for arg in args {
                    substitute_ast_type(arg, map);
                }
            }
        }
        _ => {}
    }
}

fn type_to_name(ty: &Type) -> String {
    match ty {
        Type::Int => "Int".to_string(),
        Type::Float => "Float".to_string(),
        Type::String => "Str".to_string(),
        Type::Char => "Char".to_string(),
        Type::Bool => "Bool".to_string(),
        Type::Custom(s) => s.clone(),
        Type::Generic(g, _) => g.clone(),
        Type::Void => "Void".to_string(),
    }
}

fn infer_simple_expr_type(expr: &Expr) -> Type {
    match expr {
        Expr::IntLiteral(_) => Type::Int,
        Expr::FloatLiteral(_) => Type::Float,
        Expr::StringLiteral(_) => Type::String,
        Expr::CharLiteral(_) => Type::Char,
        Expr::BoolLiteral(_) => Type::Bool,
        _ => Type::Int, // default fallback for monomorphization key
    }
}

fn mangle_types(map: &HashMap<String, Type>, order: &[String]) -> String {
    let mut parts = Vec::new();
    for param in order {
        if let Some(ty) = map.get(param) {
            let label = match ty {
                Type::Int => "Int".to_string(),
                Type::Float => "Float".to_string(),
                Type::String => "Str".to_string(),
                Type::Char => "Char".to_string(),
                Type::Bool => "Bool".to_string(),
                Type::Custom(s) => s.clone(),
                Type::Generic(g, _) => g.clone(),
                Type::Void => "Void".to_string(),
            };
            parts.push(label);
        } else {
            parts.push("Int".to_string());
        }
    }
    parts.join("_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monomorphizes_generic_function() {
        let mut program = Program {
            declarations: vec![
                TopLevel::Function(Function {
                    name: "identity".to_string(),
                    type_params: vec![TypeParam::plain("T".to_string())],
                    params: vec![Param {
                        name: "x".to_string(),
                        ty: Type::Custom("T".to_string()),
                        default: None,
                    }],
                    return_type: Type::Custom("T".to_string()),
                    body: vec![Stmt::Return(Some(Expr::Identifier("x".to_string(), std::cell::Cell::new(None))))],
                }),
                TopLevel::Function(Function {
                    name: "main".to_string(),
                    type_params: vec![],
                    params: vec![],
                    return_type: Type::Void,
                    body: vec![
                        Stmt::Expr(Expr::Call {
                            callee: Box::new(Expr::Identifier("identity".to_string(), std::cell::Cell::new(None))),
                            args: vec![Expr::IntLiteral(42)],
                        }),
                    ],
                }),
            ],
        };

        Monomorphizer::process_program(&mut program).expect("monomorphization should succeed");

        let func_names: Vec<String> = program
            .declarations
            .iter()
            .filter_map(|d| if let TopLevel::Function(f) = d { Some(f.name.clone()) } else { None })
            .collect();

        assert!(func_names.contains(&"identity__Int".to_string()), "should create specialized identity__Int function");
    }

    #[test]
    fn trait_bound_blocks_invalid_instantiation() {
        let mut program = Program {
            declarations: vec![
                TopLevel::Trait(TraitDef {
                    name: "Display".to_string(),
                    methods: vec![TraitMethod {
                        name: "show".to_string(),
                        params: vec![Param { name: "self".to_string(), ty: Type::Custom("Self".to_string()), default: None }],
                        return_type: Type::String,
                    }],
                }),
                TopLevel::Function(Function {
                    name: "print_it".to_string(),
                    type_params: vec![TypeParam { name: "T".to_string(), bound: Some("Display".to_string()) }],
                    params: vec![Param {
                        name: "x".to_string(),
                        ty: Type::Custom("T".to_string()),
                        default: None,
                    }],
                    return_type: Type::Void,
                    body: vec![],
                }),
                TopLevel::Function(Function {
                    name: "main".to_string(),
                    type_params: vec![],
                    params: vec![],
                    return_type: Type::Void,
                    body: vec![
                        Stmt::Expr(Expr::Call {
                            callee: Box::new(Expr::Identifier("print_it".to_string(), std::cell::Cell::new(None))),
                            args: vec![Expr::IntLiteral(42)],
                        }),
                    ],
                }),
            ],
        };

        Monomorphizer::process_program(&mut program).expect("monomorphization should succeed");

        let func_names: Vec<String> = program
            .declarations
            .iter()
            .filter_map(|d| if let TopLevel::Function(f) = d { Some(f.name.clone()) } else { None })
            .collect();

        assert!(!func_names.contains(&"print_it__Int".to_string()),
            "should NOT create print_it__Int because Int does not impl Display");
    }
}
