use crate::ast::*;
use std::collections::{HashMap, HashSet};

pub struct Monomorphizer {
    specialized_funcs: HashMap<String, Function>,
    specialized_structs: HashMap<String, StructDef>,
    instantiated_keys: HashSet<String>,
    /// Types of locals in the function currently being walked. Without this a
    /// call like `f := 1.5; identity(f)` cannot see that the argument is a
    /// Float and silently falls back to Int, producing a wrongly specialised
    /// copy (and a confusing type error at the use site).
    locals: HashMap<String, Type>,
    /// Every struct name in the program, so a constructor call like `Dog(3)`
    /// is typed as `Dog` and can be matched against a trait bound.
    struct_names: HashSet<String>,
    /// Unsatisfied trait bounds discovered while instantiating.
    bound_errors: Vec<String>,
    /// Every generic-struct instantiation made so far:
    /// mangled name -> (base name, type arguments in declared order).
    /// Generic impls are resolved against this, so an impl is specialised
    /// exactly when its target type is.
    struct_instantiations: HashMap<String, (String, Vec<Type>)>,
    /// Specialised impl blocks, keyed by `{mangled_target}:{trait}`.
    specialized_impls: HashMap<String, ImplBlock>,
}

impl Monomorphizer {
    pub fn new() -> Self {
        Self {
            specialized_funcs: HashMap::new(),
            specialized_structs: HashMap::new(),
            instantiated_keys: HashSet::new(),
            locals: HashMap::new(),
            struct_names: HashSet::new(),
            bound_errors: Vec::new(),
            struct_instantiations: HashMap::new(),
            specialized_impls: HashMap::new(),
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
        let mut generic_impls: Vec<ImplBlock> = Vec::new();
        for decl in &program.declarations {
            if let TopLevel::Struct(s) = decl {
                self.struct_names.insert(s.name.clone());
            }
        }

        for decl in &program.declarations {
            if let TopLevel::Function(func) = decl {
                if !func.type_params.is_empty() {
                    generic_funcs.insert(func.name.clone(), func.clone());
                }
            } else if let TopLevel::Struct(s) = decl {
                if !s.type_params.is_empty() {
                    generic_structs.insert(s.name.clone(), s.clone());
                }
            } else if let TopLevel::Impl(ib) = decl {
                if !ib.type_params.is_empty() {
                    generic_impls.push(ib.clone());
                }
                // A generic method inside an impl block is, after the parser's
                // `Target_method` mangling, just a free function. Registering it
                // here is what makes `h.pick(2.5)` specialise instead of
                // reaching the backend still generic — which crashed Cranelift
                // with "arg 1 has type f64, expected i64".
                for m in &ib.methods {
                    if !m.type_params.is_empty() {
                        generic_funcs.insert(m.name.clone(), m.clone());
                    }
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
                self.locals.clear();
                for p in &func.params {
                    self.locals.insert(p.name.clone(), p.ty.clone());
                }
                self.walk_statements(&mut func.body, &generic_funcs, &generic_structs, &trait_impls);
            } else if let TopLevel::Impl(impl_block) = decl {
                for method in &mut impl_block.methods {
                    self.walk_statements(&mut method.body, &generic_funcs, &generic_structs, &trait_impls);
                }
            }
        }

        // Specialised bodies can themselves contain generic calls
        // (`def twice[T](x: T): return identity(x)`), so keep resolving until
        // no new instantiations appear. Without this the nested call still
        // names the template, which no longer exists after pruning.
        let mut guard = 0;
        loop {
            guard += 1;
            if guard > 16 {
                break;
            }
            let pending: Vec<String> = self.specialized_funcs.keys().cloned().collect();
            let before = self.instantiated_keys.len();
            for key in pending {
                if let Some(mut f) = self.specialized_funcs.remove(&key) {
                    let saved = std::mem::take(&mut self.locals);
                    for p in &f.params {
                        self.locals.insert(p.name.clone(), p.ty.clone());
                    }
                    self.walk_statements(
                        &mut f.body, &generic_funcs, &generic_structs, &trait_impls,
                    );
                    self.locals = saved;
                    self.specialized_funcs.insert(key, f);
                }
            }
            // Impl specialisation joins the loop: a body specialised this
            // round may have instantiated a struct that needs its impl too.
            let made_impls = self.specialize_impls(&generic_impls, &generic_structs, &trait_impls);
            if self.instantiated_keys.len() == before && !made_impls {
                break;
            }
        }
        // A program that exhausts the round cap would otherwise emit a
        // half-specialised AST; fail loudly instead.
        if guard > 16 {
            self.bound_errors.push(
                "generic specialisation did not reach a fixed point within 16 rounds".to_string(),
            );
        }

        // Drop the generic templates. They are not compilable on their own -
        // a body like `return b.value` where `b: Box[T]` still mentions the
        // unbound parameter, so the type checker rejects it with
        // "Cannot access field 'value' on non-struct type Generic(Box,[T])".
        // Only the specialised copies below are real code.
        program.declarations.retain(|decl| match decl {
            TopLevel::Function(f) => f.type_params.is_empty(),
            TopLevel::Struct(s) => s.type_params.is_empty(),
            _ => true,
        });
        // Same for generic methods: the template mentions an unbound `T`, so
        // only the specialised copies (appended below as free functions) are
        // real code.
        // Generic impl templates mention an unbound T in `self` and in method
        // bodies; only the specialised copies are real code.
        program.declarations.retain(|decl| match decl {
            TopLevel::Impl(ib) => ib.type_params.is_empty(),
            _ => true,
        });
        for decl in program.declarations.iter_mut() {
            if let TopLevel::Impl(ib) = decl {
                ib.methods.retain(|m| m.type_params.is_empty());
            }
        }

        // Append generated monomorphized functions & structs to top-level declarations
        for (_, func) in self.specialized_funcs.drain() {
            program.declarations.push(TopLevel::Function(func));
        }
        for (_, struct_def) in self.specialized_structs.drain() {
            program.declarations.push(TopLevel::Struct(struct_def));
        }
        let mut impls: Vec<(String, ImplBlock)> = self.specialized_impls.drain().collect();
        impls.sort_by(|a, b| a.0.cmp(&b.0)); // deterministic output order
        for (_, ib) in impls {
            program.declarations.push(TopLevel::Impl(ib));
        }

        if let Some(first) = self.bound_errors.first() {
            return Err(first.clone());
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
                Stmt::LetInferred { name, value, .. } => {
                    self.walk_expr(value, generic_funcs, generic_structs, trait_impls);
                    let ty = self.infer_expr_type(value, generic_structs);
                    self.locals.insert(name.clone(), ty);
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
            // Turbofish: `identity[Int](x)`. The bindings come straight from
            // the written type arguments instead of being inferred, then the
            // node collapses to an ordinary call on the specialised name so no
            // later stage has to know about it.
            Expr::GenericCall { callee, type_args, args } => {
                for arg in args.iter_mut() {
                    self.walk_expr(arg, generic_funcs, generic_structs, trait_impls);
                }
                let name = match &**callee {
                    Expr::Identifier(n, _) => n.clone(),
                    _ => return,
                };
                let (tmpl_params, is_struct) = if let Some(f) = generic_funcs.get(&name) {
                    (f.type_params.clone(), false)
                } else if let Some(s) = generic_structs.get(&name) {
                    (s.type_params.clone(), true)
                } else {
                    self.bound_errors.push(format!(
                        "'{}' is not generic, so it cannot take type arguments",
                        name
                    ));
                    return;
                };
                if type_args.len() != tmpl_params.len() {
                    self.bound_errors.push(format!(
                        "'{}' expects {} type argument(s) but {} were given",
                        name,
                        tmpl_params.len(),
                        type_args.len()
                    ));
                    return;
                }
                let mut subst_map = HashMap::new();
                for (tp, ty) in tmpl_params.iter().zip(type_args.iter()) {
                    subst_map.insert(tp.name.clone(), ty.clone());
                }
                for tp in &tmpl_params {
                    if let Some(ref bound) = tp.bound {
                        if let Some(concrete) = subst_map.get(&tp.name) {
                            let cname = type_to_name(concrete);
                            let ok = trait_impls
                                .get(&cname)
                                .map_or(false, |impls| impls.contains(bound));
                            if !ok {
                                self.bound_errors.push(format!(
                                    "type '{}' does not implement trait '{}' required by generic parameter '{}' of '{}'",
                                    cname, bound, tp.name, name
                                ));
                                return;
                            }
                        }
                    }
                }
                let order: Vec<String> = tmpl_params.iter().map(|tp| tp.name.clone()).collect();
                let mangled = format!("{}__{}", name, mangle_types(&subst_map, &order));
                if is_struct {
                    self.instantiate_struct(&name, &subst_map, generic_structs);
                } else if !self.instantiated_keys.contains(&mangled) {
                    self.instantiated_keys.insert(mangled.clone());
                    let tmpl = generic_funcs.get(&name).cloned().unwrap();
                    let mut specialized = tmpl;
                    specialized.name = mangled.clone();
                    specialized.type_params.clear();
                    let mut pending: Vec<(String, HashMap<String, Type>)> = Vec::new();
                    for p in &mut specialized.params {
                        substitute_ast_type(&mut p.ty, &subst_map);
                        concretize_generic_struct(&mut p.ty, generic_structs, &mut pending);
                    }
                    substitute_ast_type(&mut specialized.return_type, &subst_map);
                    concretize_generic_struct(
                        &mut specialized.return_type,
                        generic_structs,
                        &mut pending,
                    );
                    for (sname, smap) in pending {
                        self.instantiate_struct(&sname, &smap, generic_structs);
                    }
                    self.substitute_stmts(&mut specialized.body, &subst_map);
                    self.specialized_funcs.insert(mangled.clone(), specialized);
                }
                *expr = Expr::Call {
                    callee: Box::new(Expr::Identifier(mangled, std::cell::Cell::new(None))),
                    args: std::mem::take(args),
                };
            }
            Expr::Call { callee, args } => {
                for arg in args.iter_mut() {
                    self.walk_expr(arg, generic_funcs, generic_structs, trait_impls);
                }

                // Method call `recv.m(a)`. The parser mangles impl methods to
                // `Target_m`, so if that name is generic, specialise it here and
                // rewrite the call to the specialised free function with the
                // receiver passed as the explicit first argument.
                if let Expr::FieldAccess { base, field } = &mut **callee {
                    // Walk the receiver in place. Walking a clone discarded the
                    // rewrite, so an inline constructor receiver -- `Box(5).show()`
                    // -- kept naming the pruned generic template and failed with
                    // "Undeclared identifier 'Box'".
                    self.walk_expr(base, generic_funcs, generic_structs, trait_impls);
                    let recv_ty = self.infer_expr_type(base, generic_structs);
                    if let Type::Custom(ref tname) = recv_ty {
                        let mangled_tmpl = format!("{}_{}", tname, field.clone());
                        if let Some(tmpl) = generic_funcs.get(&mangled_tmpl).cloned() {
                            let tp_names: HashSet<String> =
                                tmpl.type_params.iter().map(|tp| tp.name.clone()).collect();
                            let mut subst_map = HashMap::new();
                            // params[0] is `self`; user args line up from 1.
                            for (i, param) in tmpl.params.iter().skip(1).enumerate() {
                                if i < args.len() {
                                    let aty = self.infer_expr_type(&args[i], generic_structs);
                                    unify_type(&param.ty, &aty, &tp_names, &mut subst_map);
                                }
                            }
                            if !subst_map.is_empty() && bindings_are_concrete(&subst_map) {
                                let order: Vec<String> =
                                    tmpl.type_params.iter().map(|tp| tp.name.clone()).collect();
                                let mangled =
                                    format!("{}__{}", mangled_tmpl, mangle_types(&subst_map, &order));
                                if !self.instantiated_keys.contains(&mangled) {
                                    self.instantiated_keys.insert(mangled.clone());
                                    let mut sp = tmpl.clone();
                                    sp.name = mangled.clone();
                                    sp.type_params.clear();
                                    let mut pending: Vec<(String, HashMap<String, Type>)> = Vec::new();
                                    for p in &mut sp.params {
                                        substitute_ast_type(&mut p.ty, &subst_map);
                                        concretize_generic_struct(
                                            &mut p.ty, generic_structs, &mut pending,
                                        );
                                    }
                                    substitute_ast_type(&mut sp.return_type, &subst_map);
                                    concretize_generic_struct(
                                        &mut sp.return_type, generic_structs, &mut pending,
                                    );
                                    for (sn, sm) in pending {
                                        self.instantiate_struct(&sn, &sm, generic_structs);
                                    }
                                    self.substitute_stmts(&mut sp.body, &subst_map);
                                    self.specialized_funcs.insert(mangled.clone(), sp);
                                }
                                let mut new_args = vec![(**base).clone()];
                                new_args.append(args);
                                *expr = Expr::Call {
                                    callee: Box::new(Expr::Identifier(
                                        mangled,
                                        std::cell::Cell::new(None),
                                    )),
                                    args: new_args,
                                };
                                return;
                            }
                        }
                    }
                }

                if let Expr::Identifier(ref name, _) = **callee {
                    // Check if calling generic function
                    if let Some(tmpl) = generic_funcs.get(name) {
                        let mut subst_map = HashMap::new();

                        // Infer type parameter bindings by unifying each
                        // declared parameter type against the argument's type.
                        // Structural unification is what lets `Box[T]` bind
                        // T from a `Box__Int` argument; matching only bare
                        // `Type::Custom("T")` misses every nested position.
                        let tp_names: HashSet<String> =
                            tmpl.type_params.iter().map(|tp| tp.name.clone()).collect();
                        for (i, param) in tmpl.params.iter().enumerate() {
                            if i < args.len() {
                                let arg_ty = self.infer_expr_type(&args[i], generic_structs);
                                unify_type(&param.ty, &arg_ty, &tp_names, &mut subst_map);
                            }
                        }

                        if !subst_map.is_empty() && bindings_are_concrete(&subst_map) {
                            // Check trait bounds before monomorphizing
                            for tp in &tmpl.type_params {
                                if let Some(ref bound) = tp.bound {
                                    if let Some(concrete) = subst_map.get(&tp.name) {
                                        let concrete_name = type_to_name(concrete);
                                        let has_impl = trait_impls
                                            .get(&concrete_name)
                                            .map_or(false, |impls| impls.contains(bound));
                                        if !has_impl {
                                            // Record the violation. Silently
                                            // skipping leaves the call naming a
                                            // template that gets pruned, which
                                            // surfaces as a confusing
                                            // "Undeclared identifier" later.
                                            self.bound_errors.push(format!(
                                                "type '{}' does not implement trait '{}' required by generic parameter '{}' of '{}'",
                                                concrete_name, bound, tp.name, name
                                            ));
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

                                // Substitute params & return type. A generic
                                // struct used as a parameter (e.g. `Box[T]`)
                                // also has to be instantiated and renamed to
                                // its specialised form, otherwise the type
                                // checker sees `Generic("Box",[TypeParam T])`
                                // and rejects every field access on it.
                                let mut pending: Vec<(String, HashMap<String, Type>)> = Vec::new();
                                for p in &mut specialized.params {
                                    substitute_ast_type(&mut p.ty, &subst_map);
                                    concretize_generic_struct(
                                        &mut p.ty, generic_structs, &mut pending,
                                    );
                                }
                                substitute_ast_type(&mut specialized.return_type, &subst_map);
                                concretize_generic_struct(
                                    &mut specialized.return_type, generic_structs, &mut pending,
                                );
                                for (sname, smap) in pending {
                                    self.instantiate_struct(&sname, &smap, generic_structs);
                                }

                                // Substitute statements body
                                self.substitute_stmts(&mut specialized.body, &subst_map);

                                self.specialized_funcs.insert(key, specialized);
                            }

                            *callee = Box::new(Expr::Identifier(mangled_name, std::cell::Cell::new(None)));
                        }
                    } else if let Some(tmpl) = generic_structs.get(name) {
                        // Struct constructor call: e.g. Box(42)
                        let mut subst_map = HashMap::new();
                        let tp_names: HashSet<String> =
                            tmpl.type_params.iter().map(|tp| tp.name.clone()).collect();
                        for (i, field) in tmpl.fields.iter().enumerate() {
                            if i < args.len() {
                                let arg_ty = self.infer_expr_type(&args[i], generic_structs);
                                unify_type(&field.ty, &arg_ty, &tp_names, &mut subst_map);
                            }
                        }

                        // Refuse to specialise on a binding that is itself a
                        // type parameter (produces nonsense like `Box__T`).
                        if !subst_map.is_empty() && bindings_are_concrete(&subst_map) {
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

                                let ordered: Vec<Type> = tp_names
                                    .iter()
                                    .map(|n| subst_map.get(n).cloned().unwrap_or(Type::Int))
                                    .collect();
                                self.struct_instantiations
                                    .insert(key.clone(), (name.clone(), ordered));
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
            Expr::Try(inner) => {
                self.walk_expr(inner, generic_funcs, generic_structs, trait_impls);
            }
            Expr::Index { base, index } => {
                self.walk_expr(base, generic_funcs, generic_structs, trait_impls);
                self.walk_expr(index, generic_funcs, generic_structs, trait_impls);
            }
            Expr::EnumVariantConstruct { args, .. } => {
                for arg in args {
                    self.walk_expr(arg, generic_funcs, generic_structs, trait_impls);
                }
            }
            _ => {}
        }
    }

    /// Walk an expression that is only being inspected (a method receiver),
    /// so nested generic calls inside it still get specialised.
    fn walk_expr_ro(
        &mut self,
        expr: &Expr,
        generic_funcs: &HashMap<String, Function>,
        generic_structs: &HashMap<String, StructDef>,
        trait_impls: &HashMap<String, HashSet<String>>,
    ) {
        let mut clone = expr.clone();
        self.walk_expr(&mut clone, generic_funcs, generic_structs, trait_impls);
    }

    /// Specialise generic impls for every generic-struct instantiation seen.
    ///
    /// Runs inside the fixed-point loop, not before or after it: specialisation
    /// is transitive (a specialised body can instantiate another generic
    /// struct), so an impl pass placed outside the loop would silently miss
    /// impls for anything instantiated in a later round.
    ///
    /// Returns true if it produced anything new, so the loop knows to continue.
    fn specialize_impls(
        &mut self,
        generic_impls: &[ImplBlock],
        generic_structs: &HashMap<String, StructDef>,
        trait_impls: &HashMap<String, HashSet<String>>,
    ) -> bool {
        if generic_impls.is_empty() {
            return false;
        }
        let mut produced = false;
        let targets: Vec<(String, String, Vec<Type>)> = self
            .struct_instantiations
            .iter()
            .map(|(m, (base, args))| (m.clone(), base.clone(), args.clone()))
            .collect();

        for (mangled_target, base, args) in targets {
            // Candidate impls for this base type, grouped by trait.
            let mut by_trait: HashMap<String, Vec<&ImplBlock>> = HashMap::new();
            for ib in generic_impls.iter().filter(|i| i.target_type == base) {
                by_trait.entry(ib.trait_name.clone()).or_default().push(ib);
            }

            for (trait_name, candidates) in by_trait {
                let key = format!("{}:{}", mangled_target, trait_name);
                if self.specialized_impls.contains_key(&key) {
                    continue;
                }

                // Keep only impls whose target pattern unifies with this
                // instantiation, recording the bindings each one implies.
                let mut applicable: Vec<(&ImplBlock, HashMap<String, Type>, usize)> = Vec::new();
                for ib in candidates {
                    let tp_names: HashSet<String> =
                        ib.type_params.iter().map(|tp| tp.name.clone()).collect();
                    let mut binds = HashMap::new();
                    let mut ok = ib.target_args.len() == args.len();
                    if ok {
                        for (pat, actual) in ib.target_args.iter().zip(args.iter()) {
                            match pat {
                                Type::Custom(n) if tp_names.contains(n) => {
                                    binds.insert(n.clone(), actual.clone());
                                }
                                // A concrete position must match exactly.
                                other => {
                                    if type_to_name(other) != type_to_name(actual) {
                                        ok = false;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    if !ok {
                        continue;
                    }
                    // Specificity: how many positions are concrete. Sound only
                    // for a single type parameter, which the parser enforces.
                    let concrete = ib
                        .target_args
                        .iter()
                        .filter(|t| !matches!(t, Type::Custom(n) if tp_names.contains(n)))
                        .count();
                    applicable.push((ib, binds, concrete));
                }

                if applicable.is_empty() {
                    continue;
                }
                let best = applicable.iter().map(|(_, _, c)| *c).max().unwrap_or(0);
                let winners: Vec<_> = applicable.iter().filter(|(_, _, c)| *c == best).collect();
                if winners.len() > 1 {
                    self.bound_errors.push(format!(
                        "conflicting implementations of trait '{}' for '{}'",
                        trait_name, mangled_target
                    ));
                    continue;
                }
                let (tmpl, binds, _) = winners[0];

                // Bounds on the impl's own parameters are checked here, using
                // the same map that backs generic-function bound errors.
                let mut bound_ok = true;
                for tp in &tmpl.type_params {
                    if let Some(ref bound) = tp.bound {
                        if let Some(concrete) = binds.get(&tp.name) {
                            let cname = type_to_name(concrete);
                            let has = trait_impls
                                .get(&cname)
                                .map_or(false, |set| set.contains(bound));
                            if !has {
                                self.bound_errors.push(format!(
                                    "type '{}' does not implement trait '{}' required by generic parameter '{}' of 'impl {} for {}'",
                                    cname, bound, tp.name, trait_name, tmpl.target_type
                                ));
                                bound_ok = false;
                            }
                        }
                    }
                }
                if !bound_ok {
                    continue;
                }

                // Emit the specialised block: methods renamed onto the
                // specialised target so the existing name-based MIR dispatch
                // (`{struct}_{method}`) finds them with no changes.
                let mut sp: ImplBlock = (*tmpl).clone();
                sp.target_type = mangled_target.clone();
                sp.type_params.clear();
                sp.target_args.clear();
                for m in &mut sp.methods {
                    let short = m
                        .name
                        .strip_prefix(&format!("{}_", base))
                        .unwrap_or(&m.name)
                        .to_string();
                    m.name = format!("{}_{}", mangled_target, short);
                    m.type_params.clear();
                    let mut pending: Vec<(String, HashMap<String, Type>)> = Vec::new();
                    for prm in &mut m.params {
                        if prm.name == "self" {
                            prm.ty = Type::Custom(mangled_target.clone());
                        } else {
                            substitute_ast_type(&mut prm.ty, binds);
                            concretize_generic_struct(&mut prm.ty, generic_structs, &mut pending);
                        }
                    }
                    substitute_ast_type(&mut m.return_type, binds);
                    concretize_generic_struct(&mut m.return_type, generic_structs, &mut pending);
                    for (sn, sm) in pending {
                        self.instantiate_struct(&sn, &sm, generic_structs);
                    }
                    self.substitute_stmts(&mut m.body, binds);
                }
                self.specialized_impls.insert(key, sp);
                produced = true;
            }
        }
        produced
    }

    /// Create the specialised copy of a generic struct if it does not exist.
    fn instantiate_struct(
        &mut self,
        name: &str,
        subst_map: &HashMap<String, Type>,
        generic_structs: &HashMap<String, StructDef>,
    ) {
        let tmpl = match generic_structs.get(name) {
            Some(t) => t.clone(),
            None => return,
        };
        let tp_names: Vec<String> = tmpl.type_params.iter().map(|tp| tp.name.clone()).collect();
        let mangled = format!("{}__{}", name, mangle_types(subst_map, &tp_names));
        if self.instantiated_keys.contains(&mangled) {
            return;
        }
        self.instantiated_keys.insert(mangled.clone());
        let mut specialized = tmpl;
        specialized.name = mangled.clone();
        specialized.type_params.clear();
        for f in &mut specialized.fields {
            substitute_ast_type(&mut f.ty, subst_map);
        }
        let ordered: Vec<Type> = tp_names
            .iter()
            .map(|n| subst_map.get(n).cloned().unwrap_or(Type::Int))
            .collect();
        self.struct_instantiations
            .insert(mangled.clone(), (name.to_string(), ordered));
        self.specialized_structs.insert(mangled, specialized);
    }

    /// Best-effort type of an expression, using literals, known locals and
    /// specialised struct constructors.
    fn infer_expr_type(&self, expr: &Expr, generic_structs: &HashMap<String, StructDef>) -> Type {
        match expr {
            Expr::IntLiteral(_) => Type::Int,
            Expr::FloatLiteral(_) => Type::Float,
            Expr::StringLiteral(_) => Type::String,
            Expr::CharLiteral(_) => Type::Char,
            Expr::BoolLiteral(_) => Type::Bool,
            Expr::Identifier(name, _) => self
                .locals
                .get(name)
                .cloned()
                .unwrap_or(Type::Int),
            Expr::Call { callee, .. } => {
                if let Expr::Identifier(ref name, _) = **callee {
                    // A rewritten constructor call (`Box__Int(...)`) names the
                    // specialised struct directly.
                    if self.specialized_structs.contains_key(name) {
                        return Type::Custom(name.clone());
                    }
                    if self.struct_names.contains(name) {
                        return Type::Custom(name.clone());
                    }
                    if generic_structs.contains_key(name) {
                        return Type::Custom(name.clone());
                    }
                    if let Some(f) = self.specialized_funcs.get(name) {
                        return f.return_type.clone();
                    }
                }
                Type::Int
            }
            Expr::BinaryOp { left, .. } => self.infer_expr_type(left, generic_structs),
            _ => Type::Int,
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
            Expr::GenericCall { callee, type_args, args } => {
                self.substitute_expr(callee, map);
                // `def outer[T](x: T): return identity[T](x)` — the explicit
                // argument is itself a type parameter and must be replaced by
                // the binding for this instantiation.
                for ty in type_args.iter_mut() {
                    substitute_ast_type(ty, map);
                }
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

/// True when no binding is still an unresolved single-letter type parameter.
/// Specialising on `T` would emit `Box__T`, which the type checker rejects.
fn bindings_are_concrete(map: &HashMap<String, Type>) -> bool {
    map.values().all(|t| match t {
        Type::Custom(n) => !(n.len() == 1 && n.chars().all(|c| c.is_ascii_uppercase())),
        _ => true,
    })
}

/// Bind type parameters by structurally matching a declared type against a
/// concrete one.
///
///   declared `T`            vs `Int`        -> T = Int
///   declared `Box[T]`       vs `Box__Int`   -> T = Int   (via the mangled name)
///   declared `Pair[A, B]`   vs `Pair__Int_Str` -> A = Int, B = Str
fn unify_type(
    declared: &Type,
    concrete: &Type,
    tp_names: &HashSet<String>,
    out: &mut HashMap<String, Type>,
) {
    match declared {
        Type::Custom(name) => {
            if tp_names.contains(name) {
                out.entry(name.clone()).or_insert_with(|| concrete.clone());
            }
        }
        Type::Generic(_base, args) => {
            // The argument arrives already specialised, e.g. `Box__Int`, so
            // recover the type arguments from the mangled suffix.
            if let Type::Custom(cname) = concrete {
                if let Some((_, suffix)) = cname.split_once("__") {
                    let parts: Vec<&str> = suffix.split('_').collect();
                    for (i, arg) in args.iter().enumerate() {
                        if let Some(part) = parts.get(i) {
                            unify_type(arg, &name_to_type(part), tp_names, out);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// Inverse of `type_to_name` for the primitive labels used in mangling.
fn name_to_type(name: &str) -> Type {
    match name {
        "Int" => Type::Int,
        "Float" => Type::Float,
        "Str" => Type::String,
        "Char" => Type::Char,
        "Bool" => Type::Bool,
        "Void" => Type::Void,
        other => Type::Custom(other.to_string()),
    }
}

/// Rewrite a fully-concrete generic struct type (`Box[Int]`) into its
/// specialised nominal form (`Box__Int`), recording the instantiation needed.
fn concretize_generic_struct(
    ty: &mut Type,
    generic_structs: &HashMap<String, StructDef>,
    pending: &mut Vec<(String, HashMap<String, Type>)>,
) {
    if let Type::Generic(base, args) = ty.clone() {
        if let Some(tmpl) = generic_structs.get(&base) {
            // Only rewrite once every argument is concrete.
            if args.iter().all(|a| !matches!(a, Type::Custom(n) if n.len() == 1)) {
                let mut map = HashMap::new();
                for (tp, arg) in tmpl.type_params.iter().zip(args.iter()) {
                    map.insert(tp.name.clone(), arg.clone());
                }
                let tp_names: Vec<String> =
                    tmpl.type_params.iter().map(|tp| tp.name.clone()).collect();
                let mangled = format!("{}__{}", base, mangle_types(&map, &tp_names));
                pending.push((base.clone(), map));
                *ty = Type::Custom(mangled);
            }
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
    fn unifies_generic_struct_parameter() {
        // `Box[T]` as a parameter type must bind T and be rewritten to the
        // specialised struct; matching only bare `Custom("T")` missed this.
        let tp = |n: &str| TypeParam { name: n.to_string(), bound: None };
        let mut program = Program {
            declarations: vec![
                TopLevel::Struct(StructDef {
                    name: "Box".to_string(),
                    type_params: vec![tp("T")],
                    fields: vec![Param { name: "value".to_string(), ty: Type::Custom("T".to_string()), default: None }],
                }),
                TopLevel::Function(Function {
                    name: "unwrap".to_string(),
                    type_params: vec![tp("T")],
                    params: vec![Param {
                        name: "b".to_string(),
                        ty: Type::Generic("Box".to_string(), vec![Type::Custom("T".to_string())]),
                        default: None,
                    }],
                    return_type: Type::Custom("T".to_string()),
                    body: vec![],
                }),
                TopLevel::Function(Function {
                    name: "main".to_string(),
                    type_params: vec![],
                    params: vec![],
                    return_type: Type::Void,
                    body: vec![
                        Stmt::LetInferred {
                            name: "b".to_string(),
                            value: Expr::Call {
                                callee: Box::new(Expr::Identifier("Box".to_string(), std::cell::Cell::new(None))),
                                args: vec![Expr::IntLiteral(42)],
                            },
                            binding_id: std::cell::Cell::new(None),
                            is_mut: false,
                        },
                        Stmt::Expr(Expr::Call {
                            callee: Box::new(Expr::Identifier("unwrap".to_string(), std::cell::Cell::new(None))),
                            args: vec![Expr::Identifier("b".to_string(), std::cell::Cell::new(None))],
                        }),
                    ],
                }),
            ],
        };

        Monomorphizer::process_program(&mut program).expect("should monomorphize");

        let fns: Vec<String> = program.declarations.iter()
            .filter_map(|d| if let TopLevel::Function(f) = d { Some(f.name.clone()) } else { None })
            .collect();
        let structs: Vec<String> = program.declarations.iter()
            .filter_map(|d| if let TopLevel::Struct(s) = d { Some(s.name.clone()) } else { None })
            .collect();

        assert!(fns.contains(&"unwrap__Int".to_string()), "fns = {:?}", fns);
        assert!(structs.contains(&"Box__Int".to_string()), "structs = {:?}", structs);
        // Templates must be gone: their bodies still mention the unbound `T`.
        assert!(!fns.contains(&"unwrap".to_string()), "generic template not pruned");
        assert!(!structs.contains(&"Box".to_string()), "generic struct not pruned");

        // The specialised parameter must be the nominal struct, not Generic.
        let unwrap_int = program.declarations.iter().find_map(|d| match d {
            TopLevel::Function(f) if f.name == "unwrap__Int" => Some(f),
            _ => None,
        }).unwrap();
        assert_eq!(unwrap_int.params[0].ty, Type::Custom("Box__Int".to_string()));
    }

    #[test]
    fn infers_from_local_variable_type() {
        // `f := 1.5; identity(f)` must specialise on Float. Previously the
        // argument was not a literal so inference fell back to Int.
        let tp = |n: &str| TypeParam { name: n.to_string(), bound: None };
        let mut program = Program {
            declarations: vec![
                TopLevel::Function(Function {
                    name: "identity".to_string(),
                    type_params: vec![tp("T")],
                    params: vec![Param { name: "x".to_string(), ty: Type::Custom("T".to_string()), default: None }],
                    return_type: Type::Custom("T".to_string()),
                    body: vec![],
                }),
                TopLevel::Function(Function {
                    name: "main".to_string(),
                    type_params: vec![],
                    params: vec![],
                    return_type: Type::Void,
                    body: vec![
                        Stmt::LetInferred {
                            name: "f".to_string(),
                            value: Expr::FloatLiteral(1.5),
                            binding_id: std::cell::Cell::new(None),
                            is_mut: false,
                        },
                        Stmt::Expr(Expr::Call {
                            callee: Box::new(Expr::Identifier("identity".to_string(), std::cell::Cell::new(None))),
                            args: vec![Expr::Identifier("f".to_string(), std::cell::Cell::new(None))],
                        }),
                    ],
                }),
            ],
        };

        Monomorphizer::process_program(&mut program).expect("should monomorphize");
        let fns: Vec<String> = program.declarations.iter()
            .filter_map(|d| if let TopLevel::Function(f) = d { Some(f.name.clone()) } else { None })
            .collect();
        assert!(fns.contains(&"identity__Float".to_string()), "fns = {:?}", fns);
        assert!(!fns.contains(&"identity__Int".to_string()), "wrong specialisation: {:?}", fns);
    }

    #[test]
    fn specializes_nested_generic_calls() {
        // A specialised body may itself call a generic function; that call has
        // to be resolved too, or it names a template that gets pruned.
        let tp = |n: &str| TypeParam { name: n.to_string(), bound: None };
        let mut program = Program {
            declarations: vec![
                TopLevel::Function(Function {
                    name: "identity".to_string(),
                    type_params: vec![tp("T")],
                    params: vec![Param { name: "x".to_string(), ty: Type::Custom("T".to_string()), default: None }],
                    return_type: Type::Custom("T".to_string()),
                    body: vec![],
                }),
                TopLevel::Function(Function {
                    name: "twice".to_string(),
                    type_params: vec![tp("T")],
                    params: vec![Param { name: "x".to_string(), ty: Type::Custom("T".to_string()), default: None }],
                    return_type: Type::Custom("T".to_string()),
                    body: vec![Stmt::Return(Some(Expr::Call {
                        callee: Box::new(Expr::Identifier("identity".to_string(), std::cell::Cell::new(None))),
                        args: vec![Expr::Identifier("x".to_string(), std::cell::Cell::new(None))],
                    }))],
                }),
                TopLevel::Function(Function {
                    name: "main".to_string(),
                    type_params: vec![],
                    params: vec![],
                    return_type: Type::Void,
                    body: vec![Stmt::Expr(Expr::Call {
                        callee: Box::new(Expr::Identifier("twice".to_string(), std::cell::Cell::new(None))),
                        args: vec![Expr::IntLiteral(7)],
                    })],
                }),
            ],
        };

        Monomorphizer::process_program(&mut program).expect("should monomorphize");
        let fns: Vec<String> = program.declarations.iter()
            .filter_map(|d| if let TopLevel::Function(f) = d { Some(f.name.clone()) } else { None })
            .collect();
        assert!(fns.contains(&"twice__Int".to_string()), "fns = {:?}", fns);
        assert!(fns.contains(&"identity__Int".to_string()),
            "nested generic call was not specialised: {:?}", fns);
    }

    #[test]
    fn turbofish_uses_explicit_type_arguments() {
        // `identity[Str](x)` must specialise on Str even though the argument
        // expression alone would infer Int.
        let tp = |n: &str| TypeParam { name: n.to_string(), bound: None };
        let mut program = Program {
            declarations: vec![
                TopLevel::Function(Function {
                    name: "identity".to_string(),
                    type_params: vec![tp("T")],
                    params: vec![Param { name: "x".to_string(), ty: Type::Custom("T".to_string()), default: None }],
                    return_type: Type::Custom("T".to_string()),
                    body: vec![],
                }),
                TopLevel::Function(Function {
                    name: "main".to_string(),
                    type_params: vec![],
                    params: vec![],
                    return_type: Type::Void,
                    body: vec![Stmt::Expr(Expr::GenericCall {
                        callee: Box::new(Expr::Identifier("identity".to_string(), std::cell::Cell::new(None))),
                        type_args: vec![Type::String],
                        args: vec![Expr::IntLiteral(0)],
                    })],
                }),
            ],
        };

        Monomorphizer::process_program(&mut program).expect("should monomorphize");
        let fns: Vec<String> = program.declarations.iter()
            .filter_map(|d| if let TopLevel::Function(f) = d { Some(f.name.clone()) } else { None })
            .collect();
        assert!(fns.contains(&"identity__Str".to_string()), "fns = {:?}", fns);
        assert!(!fns.contains(&"identity__Int".to_string()),
            "explicit type argument was ignored: {:?}", fns);

        // The node must be rewritten to a plain call; later stages reject
        // GenericCall outright.
        let main = program.declarations.iter().find_map(|d| match d {
            TopLevel::Function(f) if f.name == "main" => Some(f),
            _ => None,
        }).unwrap();
        match &main.body[0] {
            Stmt::Expr(Expr::Call { callee, .. }) => match &**callee {
                Expr::Identifier(n, _) => assert_eq!(n, "identity__Str"),
                other => panic!("callee not rewritten: {:?}", other),
            },
            other => panic!("turbofish left in AST: {:?}", other),
        }
    }

    #[test]
    fn turbofish_arity_mismatch_is_reported() {
        let tp = |n: &str| TypeParam { name: n.to_string(), bound: None };
        let mut program = Program {
            declarations: vec![
                TopLevel::Function(Function {
                    name: "identity".to_string(),
                    type_params: vec![tp("T")],
                    params: vec![Param { name: "x".to_string(), ty: Type::Custom("T".to_string()), default: None }],
                    return_type: Type::Custom("T".to_string()),
                    body: vec![],
                }),
                TopLevel::Function(Function {
                    name: "main".to_string(),
                    type_params: vec![],
                    params: vec![],
                    return_type: Type::Void,
                    body: vec![Stmt::Expr(Expr::GenericCall {
                        callee: Box::new(Expr::Identifier("identity".to_string(), std::cell::Cell::new(None))),
                        type_args: vec![Type::Int, Type::String],
                        args: vec![Expr::IntLiteral(0)],
                    })],
                }),
            ],
        };
        let err = Monomorphizer::process_program(&mut program)
            .expect_err("wrong type-argument count must be reported");
        assert!(err.contains("1 type argument"), "unhelpful message: {}", err);
    }

    #[test]
    fn specializes_generic_method_in_impl_block() {
        // The parser mangles impl methods to `Target_method`. A generic one
        // must still be specialised, or it reaches the backend generic and
        // Cranelift fails its verifier instead of producing a diagnostic.
        let tp = |n: &str| TypeParam { name: n.to_string(), bound: None };
        let mut program = Program {
            declarations: vec![
                TopLevel::Struct(StructDef {
                    name: "Holder".to_string(),
                    type_params: vec![],
                    fields: vec![Param { name: "n".to_string(), ty: Type::Int, default: None }],
                }),
                TopLevel::Impl(ImplBlock {
                    trait_name: "Getter".to_string(),
                    target_type: "Holder".to_string(),
                    type_params: vec![],
                    target_args: vec![],
                    methods: vec![Function {
                        name: "Holder_pick".to_string(),
                        type_params: vec![tp("T")],
                        params: vec![
                            Param { name: "self".to_string(), ty: Type::Custom("Holder".to_string()), default: None },
                            Param { name: "x".to_string(), ty: Type::Custom("T".to_string()), default: None },
                        ],
                        return_type: Type::Custom("T".to_string()),
                        body: vec![],
                    }],
                }),
                TopLevel::Function(Function {
                    name: "main".to_string(),
                    type_params: vec![],
                    params: vec![],
                    return_type: Type::Void,
                    body: vec![
                        Stmt::LetInferred {
                            name: "h".to_string(),
                            value: Expr::Call {
                                callee: Box::new(Expr::Identifier("Holder".to_string(), std::cell::Cell::new(None))),
                                args: vec![Expr::IntLiteral(1)],
                            },
                            binding_id: std::cell::Cell::new(None),
                            is_mut: false,
                        },
                        Stmt::Expr(Expr::Call {
                            callee: Box::new(Expr::FieldAccess {
                                base: Box::new(Expr::Identifier("h".to_string(), std::cell::Cell::new(None))),
                                field: "pick".to_string(),
                            }),
                            args: vec![Expr::FloatLiteral(2.5)],
                        }),
                    ],
                }),
            ],
        };

        Monomorphizer::process_program(&mut program).expect("should monomorphize");
        let fns: Vec<String> = program.declarations.iter()
            .filter_map(|d| if let TopLevel::Function(f) = d { Some(f.name.clone()) } else { None })
            .collect();
        assert!(fns.contains(&"Holder_pick__Float".to_string()),
            "generic method not specialised: {:?}", fns);
        // The template must be gone from the impl block.
        let leftover = program.declarations.iter().any(|d| match d {
            TopLevel::Impl(ib) => ib.methods.iter().any(|m| !m.type_params.is_empty()),
            _ => false,
        });
        assert!(!leftover, "generic method template was not pruned");
    }

    #[test]
    fn specializes_generic_trait_impl_per_instantiation() {
        // impl[T] Show for Box[T] must produce one impl per Box instantiation,
        // named so the existing `{struct}_{method}` MIR lookup finds it.
        let tp = |n: &str| TypeParam { name: n.to_string(), bound: None };
        let mk_main = |args: Vec<Expr>| Function {
            name: "main".to_string(),
            type_params: vec![],
            params: vec![],
            return_type: Type::Void,
            body: args
                .into_iter()
                .map(|a| {
                    Stmt::Expr(Expr::Call {
                        callee: Box::new(Expr::Identifier("Box".to_string(), std::cell::Cell::new(None))),
                        args: vec![a],
                    })
                })
                .collect(),
        };
        let mut program = Program {
            declarations: vec![
                TopLevel::Struct(StructDef {
                    name: "Box".to_string(),
                    type_params: vec![tp("T")],
                    fields: vec![Param { name: "value".to_string(), ty: Type::Custom("T".to_string()), default: None }],
                }),
                TopLevel::Impl(ImplBlock {
                    trait_name: "Show".to_string(),
                    target_type: "Box".to_string(),
                    type_params: vec![tp("T")],
                    target_args: vec![Type::Custom("T".to_string())],
                    methods: vec![Function {
                        name: "Box_show".to_string(),
                        type_params: vec![tp("T")],
                        params: vec![Param {
                            name: "self".to_string(),
                            ty: Type::Generic("Box".to_string(), vec![Type::Custom("T".to_string())]),
                            default: None,
                        }],
                        return_type: Type::Int,
                        body: vec![],
                    }],
                }),
                TopLevel::Function(mk_main(vec![Expr::IntLiteral(1), Expr::StringLiteral("s".to_string())])),
            ],
        };

        Monomorphizer::process_program(&mut program).expect("should monomorphize");
        let impls: Vec<(String, Vec<String>)> = program
            .declarations
            .iter()
            .filter_map(|d| match d {
                TopLevel::Impl(ib) => {
                    Some((ib.target_type.clone(), ib.methods.iter().map(|m| m.name.clone()).collect()))
                }
                _ => None,
            })
            .collect();
        let targets: Vec<&String> = impls.iter().map(|(t, _)| t).collect();
        assert!(targets.iter().any(|t| *t == "Box__Int"), "impls = {:?}", impls);
        assert!(targets.iter().any(|t| *t == "Box__Str"), "impls = {:?}", impls);
        // The generic template must be pruned; its `self` mentions an unbound T.
        assert!(!targets.iter().any(|t| *t == "Box"), "template not pruned: {:?}", impls);
        // Methods must be renamed onto the specialised target.
        let all: Vec<String> = impls.iter().flat_map(|(_, m)| m.clone()).collect();
        assert!(all.contains(&"Box__Int_show".to_string()), "methods = {:?}", all);
        assert!(all.contains(&"Box__Str_show".to_string()), "methods = {:?}", all);
    }

    #[test]
    fn concrete_impl_beats_generic_one() {
        // Most-specific-wins: impl[T] Show for Box[Int] outranks Box[T].
        let tp = |n: &str| TypeParam { name: n.to_string(), bound: None };
        let mk_impl = |arg: Type, ret: i64| TopLevel::Impl(ImplBlock {
            trait_name: "Show".to_string(),
            target_type: "Box".to_string(),
            type_params: vec![tp("T")],
            target_args: vec![arg],
            methods: vec![Function {
                name: "Box_show".to_string(),
                type_params: vec![tp("T")],
                params: vec![Param {
                    name: "self".to_string(),
                    ty: Type::Custom("Box".to_string()),
                    default: None,
                }],
                return_type: Type::Int,
                body: vec![Stmt::Return(Some(Expr::IntLiteral(ret)))],
            }],
        });
        let mut program = Program {
            declarations: vec![
                TopLevel::Struct(StructDef {
                    name: "Box".to_string(),
                    type_params: vec![tp("T")],
                    fields: vec![Param { name: "value".to_string(), ty: Type::Custom("T".to_string()), default: None }],
                }),
                mk_impl(Type::Custom("T".to_string()), 1),
                mk_impl(Type::Int, 99),
                TopLevel::Function(Function {
                    name: "main".to_string(),
                    type_params: vec![],
                    params: vec![],
                    return_type: Type::Void,
                    body: vec![Stmt::Expr(Expr::Call {
                        callee: Box::new(Expr::Identifier("Box".to_string(), std::cell::Cell::new(None))),
                        args: vec![Expr::IntLiteral(5)],
                    })],
                }),
            ],
        };

        Monomorphizer::process_program(&mut program).expect("should monomorphize");
        let body = program.declarations.iter().find_map(|d| match d {
            TopLevel::Impl(ib) if ib.target_type == "Box__Int" => Some(ib.methods[0].body.clone()),
            _ => None,
        }).expect("Box__Int impl must exist");
        assert_eq!(
            body,
            vec![Stmt::Return(Some(Expr::IntLiteral(99)))],
            "the more specific Box[Int] impl should win"
        );
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

        // An unsatisfied bound is now a hard error rather than a silent skip:
        // skipping left the call site naming a template that later gets pruned,
        // which surfaced as a confusing "Undeclared identifier".
        let result = Monomorphizer::process_program(&mut program);
        assert!(result.is_err(), "unsatisfied trait bound must be reported");
        let msg = result.unwrap_err();
        assert!(msg.contains("Display"), "error should name the trait: {}", msg);
        assert!(msg.contains("Int"), "error should name the offending type: {}", msg);

        let func_names: Vec<String> = program
            .declarations
            .iter()
            .filter_map(|d| if let TopLevel::Function(f) = d { Some(f.name.clone()) } else { None })
            .collect();

        assert!(!func_names.contains(&"print_it__Int".to_string()),
            "should NOT create print_it__Int because Int does not impl Display");
    }
}
