use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::ir::AbiParam;
use cranelift_codegen::ir::types as cl_types;
use cranelift_module::{Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};
use target_lexicon::Triple;
use crate::mir::ir::*;
use crate::typecheck::{TypeRef, TypeTable};
use super::lower::FunctionLower;
use super::types::type_to_cl;
use std::collections::HashMap;


fn decode_ty(tag: u8) -> cranelift_codegen::ir::Type {
    match tag { 
        0 => cl_types::I64, 
        1 => cl_types::I8, 
        2 => cl_types::I32, 
        3 => cl_types::F64,
        _ => cl_types::I64 
    }
}

// ── AotCompiler ──────────────────────────────────────────────────────────────

pub struct AotCompiler {
    pub module: ObjectModule,
    pub func_ids:    HashMap<FuncId, cranelift_module::FuncId>,
    pub builtin_ids: HashMap<String, cranelift_module::FuncId>,
}

impl AotCompiler {
    pub fn new_for_host() -> Result<Self, String> {
        let mut flag_builder = settings::builder();
        flag_builder
            .set("use_colocated_libcalls", "false")
            .map_err(|e| format!("set use_colocated_libcalls: {}", e))?;
        flag_builder
            .set("is_pic", "false")
            .map_err(|e| format!("set is_pic: {}", e))?;
        let opt_level = if std::env::var("LPP_RELEASE").is_ok() {
            "speed"
        } else {
            "none"
        };
        flag_builder
            .set("opt_level", opt_level)
            .map_err(|e| format!("set opt_level '{}': {}", opt_level, e))?;

        let isa = cranelift_codegen::isa::lookup(Triple::host())
            .map_err(|e| format!("ISA lookup: {}", e))?
            .finish(settings::Flags::new(flag_builder))
            .map_err(|e| format!("ISA finish: {}", e))?;

        let module = ObjectModule::new(
            ObjectBuilder::new(isa, "lpp_module", cranelift_module::default_libcall_names())
                .map_err(|e| format!("ObjectBuilder: {}", e))?,
        );

        Ok(Self { module, func_ids: HashMap::new(), builtin_ids: HashMap::new() })
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
            let id = self.module
                .declare_function(builtin.symbol, Linkage::Import, &sig)
                .map_err(|e| format!("declare builtin '{}': {:?}", builtin.symbol, e))?;
            self.builtin_ids.insert(builtin.symbol.to_string(), id);
        }
        Ok(())
    }

    /// Declare all user functions so they can call each other.
    pub fn declare_functions(&mut self, program: &MirProgram) -> Result<(), String> {
        for (mir_id, mir_fn) in &program.functions {
            let mut sig = self.module.make_signature();
            for param_id in &mir_fn.params {
                sig.params.push(AbiParam::new(type_to_cl(&mir_fn.locals[param_id.0].ty)));
            }
            if mir_fn.return_type != TypeRef::Void {
                sig.returns.push(AbiParam::new(type_to_cl(&mir_fn.return_type)));
            }
            let linkage = if mir_fn.name == "main" { Linkage::Export } else { Linkage::Local };
            let id = self.module
                .declare_function(&mir_fn.name, linkage, &sig)
                .map_err(|e| format!("declare '{}': {:?}", mir_fn.name, e))?;
            self.func_ids.insert(*mir_id, id);
        }
        Ok(())
    }

    /// Lower all function bodies.
    pub fn lower_functions(&mut self, program: &MirProgram, type_table: &TypeTable) -> Result<(), String> {
        let mir_fns: Vec<MirFunction> = program.functions.values().cloned().collect();
        for mir_fn in &mir_fns {
            if mir_fn.blocks.is_empty() { continue; }
            let mut lower = FunctionLower {
                module:      &mut self.module,
                func_ids:    &self.func_ids,
                builtin_ids: &self.builtin_ids,
                type_table,
                fn_name:     mir_fn.name.clone(),
                next_str_idx: 0,
            };
            lower.lower_function(mir_fn)?;
        }
        Ok(())
    }

    pub fn finish(self) -> Result<Vec<u8>, String> {
        self.module.finish().emit().map_err(|e| format!("emit: {:?}", e))
    }

    /// Full pipeline: builtins → declare → lower → emit.
    pub fn compile(program: &MirProgram, type_table: &TypeTable) -> Result<Vec<u8>, String> {
        let mut c = Self::new_for_host()?;
        c.declare_builtins()?;
        c.declare_functions(program)?;
        c.lower_functions(program, type_table)?;
        c.finish()
    }
}
