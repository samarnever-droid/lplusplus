use super::lower::FunctionLower;
use super::types::{struct_layout, type_to_cl};
use crate::mir::ir::*;
use crate::typecheck::{StructTypeId, TypeRef, TypeTable};
use cranelift_codegen::ir::types as cl_types;
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};
use std::collections::{HashMap, HashSet};
use target_lexicon::Triple;

fn decode_ty(tag: u8) -> cranelift_codegen::ir::Type {
    match tag {
        0 => cl_types::I64,
        1 => cl_types::I8,
        2 => cl_types::I32,
        3 => cl_types::F64,
        _ => cl_types::I64,
    }
}

/// Find structs whose owned custom fields form a cycle. ARC cannot reclaim a
/// strongly connected ownership graph, so the AOT backend refuses to allocate
/// one until L++ has explicit `Weak`/arena ownership syntax or a cycle collector.
fn arc_cycle_structs(type_table: &TypeTable) -> HashSet<StructTypeId> {
    fn reaches(
        type_table: &TypeTable,
        target: StructTypeId,
        current: StructTypeId,
        visited: &mut HashSet<StructTypeId>,
    ) -> bool {
        for (_, field_ty) in &type_table.definitions[current.0].fields {
            let next = match field_ty {
                TypeRef::Custom(next) => Some(*next),
                TypeRef::Generic(name, args) if name == "List" && args.len() == 1 => {
                    match args[0] {
                        TypeRef::Custom(next) => Some(next),
                        _ => None,
                    }
                }
                _ => None,
            };
            if let Some(next) = next {
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

    let mut cycles = HashSet::new();
    for index in 0..type_table.definitions.len() {
        let id = StructTypeId(index);
        let mut visited = HashSet::new();
        if reaches(type_table, id, id, &mut visited) {
            cycles.insert(id);
        }
    }
    cycles
}

/// Validate the subset whose runtime representation is defined for AOT.  This
/// deliberately sits at the backend boundary as defence in depth: frontend
/// checks can evolve without accidentally making Cranelift emit a binary for
/// a type that its ABI cannot represent.
fn validate_aot_program(program: &MirProgram, type_table: &TypeTable) -> Result<(), String> {
    fn validate_type(ty: &TypeRef, where_: &str) -> Result<(), String> {
        match ty {
            TypeRef::Generic(name, args)
                if (name == "List" && args.len() == 1) || (name == "Map" && args.len() == 2) =>
            {
                for arg in args {
                    validate_type(arg, where_)?;
                }
                Ok(())
            }
            TypeRef::Generic(name, args) => Err(format!(
                "AOT does not yet support {}[{}] in {}",
                name,
                args.iter()
                    .map(|arg| format!("{:?}", arg))
                    .collect::<Vec<_>>()
                    .join(", "),
                where_
            )),
            TypeRef::Unresolved(name) => Err(format!(
                "unresolved type '{}' reached the AOT backend in {}",
                name, where_
            )),
            _ => Ok(()),
        }
    }

    for def in &type_table.definitions {
        for (field_name, field_ty) in &def.fields {
            validate_type(field_ty, &format!("field '{}.{}'", def.name, field_name))?;
        }
    }

    let cyclic_structs = arc_cycle_structs(type_table);

    for function in program.functions.values() {
        validate_type(
            &function.return_type,
            &format!("return type of '{}'", function.name),
        )?;
        for local in &function.locals {
            validate_type(
                &local.ty,
                &format!("local {:?} in '{}'", local.debug_name, function.name),
            )?;
        }
        for block in &function.blocks {
            for instruction in &block.instrs {
                match instruction {
                    MirInstr::Assign(_, Rvalue::AllocateArcStruct(TypeRef::Custom(struct_id)))
                        if cyclic_structs.contains(struct_id) =>
                    {
                        let name = &type_table.definitions[struct_id.0].name;
                        return Err(format!(
                            "AOT rejects cyclic owned struct '{}': ARC cannot reclaim ownership cycles. Use a future Weak/arena annotation or remove the cycle.",
                            name
                        ));
                    }
                    MirInstr::Assign(_, Rvalue::AllocateStruct(_)) => {
                        return Err(format!(
                            "raw struct allocation reached AOT in '{}'; ownership lowering is required",
                            function.name
                        ));
                    }
                    MirInstr::Assign(_, Rvalue::AllocateList(element_ty))
                        if !matches!(
                            element_ty,
                            TypeRef::Int | TypeRef::Float | TypeRef::Custom(_) | TypeRef::Str | TypeRef::Bool
                        ) =>
                    {
                        return Err(format!(
                            "AOT supports List[Int/Float/Bool/Str/Custom], but '{}' allocates List[{:?}]",
                            function.name, element_ty
                        ));
                    }
                    MirInstr::Assign(_, Rvalue::BuiltinCall(symbol, _))
                        if symbol == "lpp_list_free" =>
                    {
                        return Err(format!(
                            "AOT List[Int] uses automatic ARC cleanup; remove manual list_free in '{}'",
                            function.name
                        ));
                    }
                    MirInstr::Assign(_, Rvalue::MakeClosure(_, captures))
                        if captures.len() != 1 =>
                    {
                        return Err(format!(
                            "invalid closure environment in '{}': expected exactly one environment pointer, got {}",
                            function.name,
                            captures.len()
                        ));
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

// ── AotCompiler ──────────────────────────────────────────────────────────────

pub struct AotCompiler {
    pub module: ObjectModule,
    pub func_ids: HashMap<FuncId, cranelift_module::FuncId>,
    pub builtin_ids: HashMap<String, cranelift_module::FuncId>,
    pub drop_ids: HashMap<StructTypeId, cranelift_module::FuncId>,
    /// C ABI `main` wrapper around the L++ user-level `main` function.
    pub entrypoint_id: Option<cranelift_module::FuncId>,
}

impl AotCompiler {
    pub fn new_for_host() -> Result<Self, String> {
        let mut flag_builder = settings::builder();
        flag_builder
            .set("use_colocated_libcalls", "false")
            .map_err(|e| format!("set use_colocated_libcalls: {}", e))?;
        // Emit relocations suitable for modern Linux/macOS PIE executables and
        // shared-library style linking. This removes the need for a non-PIE
        // linker workaround in the normal Cranelift AOT path.
        flag_builder
            .set("is_pic", "true")
            .map_err(|e| format!("set is_pic: {}", e))?;
        // Compilation latency is a first-class L++ pillar. Keep the existing
        // release default, but make Cranelift's trade-off explicit and
        // benchmarkable instead of forcing contributors to edit compiler code.
        // Valid values are Cranelift's stable levels: none, speed, speed_and_size.
        let opt_level = match std::env::var("LPP_AOT_OPT") {
            Ok(value) if matches!(value.as_str(), "none" | "speed" | "speed_and_size") => value,
            Ok(value) => {
                return Err(format!(
                    "invalid LPP_AOT_OPT='{}'; expected none, speed, or speed_and_size",
                    value
                ));
            }
            Err(_) if std::env::var("LPP_RELEASE").is_ok() => "speed".to_string(),
            Err(_) => "speed".to_string(), // Always optimize — Cranelift speed mode is fast enough
        };
        flag_builder
            .set("opt_level", &opt_level)
            .map_err(|e| format!("set opt_level '{}': {}", opt_level, e))?;

        let isa = cranelift_codegen::isa::lookup(Triple::host())
            .map_err(|e| format!("ISA lookup: {}", e))?
            .finish(settings::Flags::new(flag_builder))
            .map_err(|e| format!("ISA finish: {}", e))?;

        let module = ObjectModule::new(
            ObjectBuilder::new(isa, "lpp_module", cranelift_module::default_libcall_names())
                .map_err(|e| format!("ObjectBuilder: {}", e))?,
        );

        Ok(Self {
            module,
            func_ids: HashMap::new(),
            builtin_ids: HashMap::new(),
            drop_ids: HashMap::new(),
            entrypoint_id: None,
        })
    }

    /// Declare all L++ runtime symbols as external imports.
    pub fn declare_builtins(&mut self) -> Result<(), String> {
        for builtin in crate::builtins::get_builtins() {
            if builtin.symbol.is_empty() {
                continue;
            }
            if self.builtin_ids.contains_key(builtin.symbol) {
                continue;
            }
            let mut sig = self.module.make_signature();
            for &p in builtin.cl_params {
                sig.params.push(AbiParam::new(decode_ty(p)));
            }
            if let Some(r) = builtin.cl_return {
                sig.returns.push(AbiParam::new(decode_ty(r)));
            }
            let id = self
                .module
                .declare_function(builtin.symbol, Linkage::Import, &sig)
                .map_err(|e| format!("declare builtin '{}': {:?}", builtin.symbol, e))?;
            self.builtin_ids.insert(builtin.symbol.to_string(), id);
        }
        Ok(())
    }

    /// Declare one internal destructor per custom struct. The runtime stores a
    /// pointer to this function in the ARC header and calls it exactly when the
    /// object's reference count reaches zero.
    pub fn declare_drop_functions(&mut self, type_table: &TypeTable) -> Result<(), String> {
        for (index, definition) in type_table.definitions.iter().enumerate() {
            let mut sig = self.module.make_signature();
            sig.params.push(AbiParam::new(cl_types::I64));
            let id = self
                .module
                .declare_function(
                    &format!("__lpp_drop_{}", definition.name),
                    Linkage::Local,
                    &sig,
                )
                .map_err(|e| format!("declare ARC destructor '{}': {:?}", definition.name, e))?;
            self.drop_ids.insert(StructTypeId(index), id);
        }
        Ok(())
    }

    /// Define destructors after all IDs exist, allowing recursive struct graphs.
    /// A child release invokes its own registered destructor only when that
    /// child's last reference is released.
    pub fn lower_drop_functions(&mut self, type_table: &TypeTable) -> Result<(), String> {
        let release_id = *self
            .builtin_ids
            .get("lpp_arc_release")
            .ok_or_else(|| "Builtin 'lpp_arc_release' was not declared".to_string())?;

        for (index, definition) in type_table.definitions.iter().enumerate() {
            let struct_id = StructTypeId(index);
            let drop_id = *self.drop_ids.get(&struct_id).ok_or_else(|| {
                format!("missing declared ARC destructor for '{}'", definition.name)
            })?;
            let mut ctx = self.module.make_context();
            ctx.func.signature.params.push(AbiParam::new(cl_types::I64));
            ctx.func.name = cranelift_codegen::ir::UserFuncName::user(0, drop_id.as_u32());
            let mut fn_ctx = FunctionBuilderContext::new();
            {
                let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fn_ctx);
                let entry = builder.create_block();
                builder.switch_to_block(entry);
                builder.append_block_params_for_function_params(entry);
                let payload = builder.block_params(entry)[0];
                let release_ref = self.module.declare_func_in_func(release_id, builder.func);
                let (layout, _) = struct_layout(type_table, struct_id);

                for ((_, field_type), field_layout) in definition.fields.iter().zip(layout.iter()) {
                    if matches!(field_type, TypeRef::Custom(_) | TypeRef::Generic(_, _)) {
                        let child = builder.ins().load(
                            cl_types::I64,
                            MemFlags::new(),
                            payload,
                            field_layout.offset as i32,
                        );
                        builder.ins().call(release_ref, &[child]);
                    }
                }
                builder.ins().return_(&[]);
                builder.seal_all_blocks();
                builder.finalize();
            }
            self.module
                .define_function(drop_id, &mut ctx)
                .map_err(|e| format!("define ARC destructor '{}': {:?}", definition.name, e))?;
        }
        Ok(())
    }

    /// Declare all user functions so they can call each other.
    pub fn declare_functions(&mut self, program: &MirProgram) -> Result<(), String> {
        for (mir_id, mir_fn) in &program.functions {
            let mut sig = self.module.make_signature();
            for param_id in &mir_fn.params {
                sig.params
                    .push(AbiParam::new(type_to_cl(&mir_fn.locals[param_id.0].ty)));
            }
            if mir_fn.return_type != TypeRef::Void {
                sig.returns
                    .push(AbiParam::new(type_to_cl(&mir_fn.return_type)));
            }
            // Keep the user function internal as `lpp_main`; a generated C ABI
            // `main` wrapper returns a defined process status of zero.
            let symbol_name = if mir_fn.name == "main" {
                "lpp_main"
            } else {
                &mir_fn.name
            };
            let id = self
                .module
                .declare_function(symbol_name, Linkage::Local, &sig)
                .map_err(|e| format!("declare '{}': {:?}", mir_fn.name, e))?;
            self.func_ids.insert(*mir_id, id);
        }
        Ok(())
    }

    /// Declare a conventional `int main(void)` entry point for the system
    /// linker. The L++ source-level `main` may be `Void`, which is not itself a
    /// valid C process-entry ABI.
    pub fn declare_entrypoint_wrapper(&mut self, program: &MirProgram) -> Result<(), String> {
        if !program
            .functions
            .values()
            .any(|function| function.name == "main")
        {
            return Ok(());
        }
        let mut signature = self.module.make_signature();
        signature.returns.push(AbiParam::new(cl_types::I32));
        let id = self
            .module
            .declare_function("main", Linkage::Export, &signature)
            .map_err(|error| format!("declare C ABI main wrapper: {:?}", error))?;
        self.entrypoint_id = Some(id);
        Ok(())
    }

    /// Lower the C ABI entry point after the source-level functions are defined.
    pub fn lower_entrypoint_wrapper(&mut self, program: &MirProgram) -> Result<(), String> {
        let Some(wrapper_id) = self.entrypoint_id else {
            return Ok(());
        };
        let (user_main_id, _) = program
            .functions
            .iter()
            .find(|(_, function)| function.name == "main")
            .ok_or_else(|| "entrypoint wrapper declared without L++ main".to_string())?;
        let main_id = *self
            .func_ids
            .get(user_main_id)
            .ok_or_else(|| "missing declared L++ main function".to_string())?;

        let mut ctx = self.module.make_context();
        ctx.func
            .signature
            .returns
            .push(AbiParam::new(cl_types::I32));
        let mut fn_ctx = FunctionBuilderContext::new();
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fn_ctx);
            let entry = builder.create_block();
            builder.switch_to_block(entry);
            let main_ref = self.module.declare_func_in_func(main_id, builder.func);
            builder.ins().call(main_ref, &[]);
            let status = builder.ins().iconst(cl_types::I32, 0);
            builder.ins().return_(&[status]);
            builder.seal_all_blocks();
            builder.finalize();
        }
        self.module
            .define_function(wrapper_id, &mut ctx)
            .map_err(|error| format!("define C ABI main wrapper: {:?}", error))?;
        Ok(())
    }

    /// Lower all function bodies.
    pub fn lower_functions(
        &mut self,
        program: &MirProgram,
        type_table: &TypeTable,
    ) -> Result<(), String> {
        let mir_fns: Vec<MirFunction> = program.functions.values().cloned().collect();
        for mir_fn in &mir_fns {
            if mir_fn.blocks.is_empty() {
                continue;
            }
            let mut lower = FunctionLower {
                module: &mut self.module,
                func_ids: &self.func_ids,
                builtin_ids: &mut self.builtin_ids,
                drop_ids: &self.drop_ids,
                type_table,
                fn_name: mir_fn.name.clone(),
                next_str_idx: 0,
            };
            lower.lower_function(mir_fn)?;
        }
        Ok(())
    }

    pub fn finish(self) -> Result<Vec<u8>, String> {
        self.module
            .finish()
            .emit()
            .map_err(|e| format!("emit: {:?}", e))
    }

    /// Full pipeline: builtins → declare → lower → emit.
    pub fn compile(program: &MirProgram, type_table: &TypeTable) -> Result<Vec<u8>, String> {
        validate_aot_program(program, type_table)?;
        let mut c = Self::new_for_host()?;
        c.declare_builtins()?;
        c.declare_drop_functions(type_table)?;
        c.declare_functions(program)?;
        c.declare_entrypoint_wrapper(program)?;
        c.lower_drop_functions(type_table)?;
        c.lower_functions(program, type_table)?;
        c.lower_entrypoint_wrapper(program)?;
        c.finish()
    }
}
