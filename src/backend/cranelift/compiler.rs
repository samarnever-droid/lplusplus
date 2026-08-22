use super::lower::FunctionLower;
use super::types::{abi_to_cl, type_to_cl};
use crate::layout::{struct_layout, tuple_layout};
use crate::mir::ir::*;
use crate::types::{StructTypeId, TypeRef, TypeTable};
use cranelift_codegen::ir::types as cl_types;
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};
use std::collections::HashMap;
use std::str::FromStr;
use target_lexicon::Triple;

/// A function body compiled off-thread, waiting to be defined in the module.
struct CompiledFunction {
    func_id: cranelift_module::FuncId,
    alignment: u64,
    bytes: Vec<u8>,
    relocs: Vec<cranelift_codegen::FinalizedMachReloc>,
    func: cranelift_codegen::ir::Function,
}

/// How many worker threads to use for machine-code generation.
///
/// `LPP_CODEGEN_THREADS` overrides; `1` forces the serial path. Small modules
/// stay serial because thread setup costs more than it saves.
fn codegen_threads(function_count: usize) -> usize {
    if let Ok(value) = std::env::var("LPP_CODEGEN_THREADS") {
        if let Ok(n) = value.trim().parse::<usize>() {
            return n.max(1).min(function_count.max(1));
        }
    }
    if function_count < 8 {
        return 1;
    }
    let available = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    available.min(function_count).max(1)
}

/// Number of functions lowered + emitted per bounded batch.
///
/// This bounds the peak memory of the Cranelift backend. With batch size B the
/// backend holds at most B MIR clones, B Cranelift IR contexts and B compiled
/// bodies at once, so peak memory is O(B) and independent of the total number
/// of functions. Set `LPP_CODEGEN_BATCH` to tune it; 0 or an unparsable value
/// falls back to the default.
///
/// The default tries to keep per-batch memory modest (a few MB of IR + machine
/// code per function × ~256 functions) while still amortising thread-pool
/// startup. Larger values trade a higher memory ceiling for slightly lower
/// overhead; smaller values make the compiler more frugal.
fn codegen_batch_size(function_count: usize) -> usize {
    if let Ok(value) = std::env::var("LPP_CODEGEN_BATCH") {
        if let Ok(n) = value.trim().parse::<usize>() {
            return n.max(1).min(function_count.max(1));
        }
    }
    if function_count == 0 {
        return 1;
    }
    256usize.min(function_count)
}

fn decode_ty(tag: u8) -> cranelift_codegen::ir::Type {
    match tag {
        0 => cl_types::I64,
        1 => cl_types::I8,
        2 => cl_types::I32,
        3 => cl_types::F64,
        4 => cl_types::I64X2,
        _ => cl_types::I64,
    }
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
            TypeRef::Tuple(elements) => {
                if !(2..=4).contains(&elements.len()) {
                    return Err(format!(
                        "tuple arity {} is invalid in {}",
                        elements.len(),
                        where_
                    ));
                }
                for element in elements {
                    validate_type(element, where_)?;
                }
                Ok(())
            }
            TypeRef::Slice(element) | TypeRef::Task(element) => validate_type(element, where_),
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
                    // Recursive struct types are accepted; `analysis::cyclebreak`
                    // has already demoted one edge of every cycle to non-owning,
                    // so no owning cycle can reach here to leak.
                    _ if false => unreachable!(),
                    // `AllocateStruct` is the legacy raw form and stays rejected:
                    // it has no header and no proof that it does not need one.
                    // `AllocateStackStruct` is different -- it is only ever
                    // produced by `pass_escape`, which proves the local cannot
                    // outlive the frame before emitting it.
                    MirInstr::Assign(_, Rvalue::AllocateStruct(_)) => {
                        return Err(format!(
                            "raw struct allocation reached AOT in '{}'; ownership lowering is required",
                            function.name
                        ));
                    }
                    MirInstr::Assign(_, Rvalue::AllocateList(element_ty))
                        if !element_ty.is_list_element_supported() =>
                    {
                        return Err(format!(
                            "AOT supports scalar or ARC-managed one-slot list elements, but '{}' allocates List[{:?}]",
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
                    MirInstr::Assign(
                        _,
                        Rvalue::MakeClosure(_, captures) | Rvalue::MakeStackClosure(_, captures),
                    ) if captures.len() != 1 => {
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
    pub task_thunk_ids: HashMap<FuncId, cranelift_module::FuncId>,
    /// C ABI `main` wrapper around the L++ user-level `main` function.
    pub entrypoint_id: Option<cranelift_module::FuncId>,
    /// When the whole program is proven single-threaded, ARC uses the
    /// non-atomic runtime entry points. See `mir::pass_arc_local`.
    pub arc_non_atomic: bool,
    /// Fields demoted to non-owning by `analysis::cyclebreak`. Their
    /// destructors must not release, and their stores must not retain.
    pub weak_fields: std::collections::HashSet<(StructTypeId, String)>,
    /// Per function, the locals the escape solver classified `Shared`. Only
    /// these need atomic refcount updates; everything else can use the
    /// non-atomic entry points even in a program that spawns threads.
    /// An absent function means "no information", which is treated as shared.
    pub shared_locals: HashMap<FuncId, std::collections::HashSet<LocalId>>,
}

impl AotCompiler {
    pub fn new_for_host() -> Result<Self, String> {
        Self::new_for_target(None)
    }

    /// Construct a codegen engine for an optional target triple. When `target`
    /// is None the host triple is used (normal build). A `--target` triple such
    /// as `aarch64-linux-android` selects the matching Cranelift backend (e.g.
    /// the aarch64 ISA, available when the compiler is built with the
    /// `all-arch`/aarch64 feature).
    pub fn new_for_target(target: Option<&str>) -> Result<Self, String> {
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
        let isa_triple: Triple = match target {
            Some(t) => {
                Triple::from_str(t).map_err(|e| format!("invalid target triple '{}': {}", t, e))?
            }
            None => Triple::host(),
        };
        // AVX2/AVX are x86_64-only CPU features. Gate them on the *selected*
        // target architecture, not the host the compiler was compiled on, so a
        // cross-target (e.g. aarch64-linux-android) does not try to enable x86
        // features on an aarch64 ISA builder.
        let target_is_x86_64 = isa_triple.architecture.to_string().starts_with("x86_64");
        let mut isa_builder = cranelift_codegen::isa::lookup(isa_triple).map_err(|e| {
            format!(
                "ISA lookup for target '{}': {}",
                target.unwrap_or("host"),
                e
            )
        })?;
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        fn is_avx2_detected() -> bool {
            std::is_x86_feature_detected!("avx2")
        }

        #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
        fn is_avx2_detected() -> bool {
            false
        }

        if std::env::var("LPP_CRANELIFT_SIMD").as_deref() != Ok("0")
            && target_is_x86_64
            && is_avx2_detected()
        {
            isa_builder
                .enable("has_avx")
                .map_err(|e| format!("enable Cranelift AVX: {}", e))?;
            isa_builder
                .enable("has_avx2")
                .map_err(|e| format!("enable Cranelift AVX2: {}", e))?;
        }
        let isa = isa_builder
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
            arc_non_atomic: false,
            weak_fields: std::collections::HashSet::new(),
            shared_locals: HashMap::new(),
            drop_ids: HashMap::new(),
            task_thunk_ids: HashMap::new(),
            entrypoint_id: None,
        })
    }

    /// Declare referenced L++ runtime symbols as external imports.
    pub fn declare_builtins(&mut self, program: &MirProgram) -> Result<(), String> {
        let mut used_symbols = std::collections::HashSet::new();
        // Core runtime builtins that codegen unconditionally references
        for sym in &[
            "lpp_arc_alloc_with_destructor",
            "lpp_arc_alloc",
            "lpp_arc_release",
            "lpp_arena_release_node",
            "lpp_closure_destroy",
            "lpp_tuple_alloc",
            "lpp_task_new",
            "lpp_task_await",
            "lpp_panic",
            "fmod",
            "lpp_str_slice_to_str",
            "lpp_slice_init",
            "lpp_slice_len",
            "lpp_thread_spawn",
            "lpp_arc_retain_local",
            "lpp_arc_release_local",
            "lpp_arc_retain",
            "lpp_list_new",
            "lpp_list_new_arc",
            "lpp_list_push",
            "lpp_list_push_arc",
            "lpp_list_push_float",
            "lpp_list_push_bool",
            "lpp_list_get",
            "lpp_list_get_float",
            "lpp_list_get_bool",
            "lpp_list_get_arc",
            "lpp_list_set",
            "lpp_list_set_float",
            "lpp_list_set_bool",
            "lpp_list_set_arc",
            "lpp_list_len",
        ] {
            used_symbols.insert(sym.to_string());
        }
        for function in program.functions.values() {
            for block in &function.blocks {
                for instr in &block.instrs {
                    if let MirInstr::Assign(_, Rvalue::BuiltinCall(sym, _)) = instr {
                        used_symbols.insert(sym.clone());
                    }
                }
            }
        }
        for builtin in crate::builtins::get_builtins() {
            if builtin.symbol.is_empty() {
                continue;
            }
            // VectorI64x2 builtins are lowered to inline SIMD instructions by
            // the Cranelift backend — they never emit an actual call and must
            // NOT be declared as external imports, because that would cause
            // every object file (even non-SIMD programs) to carry unresolved
            // references that break the host linker (cl.exe / cc).
            if builtin.symbol.starts_with("lpp_vec_i64x2")
                || builtin.symbol.starts_with("lpp_vec_u8x16")
            {
                continue;
            }
            if !used_symbols.contains(builtin.symbol) {
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
        let arena_release_id = *self
            .builtin_ids
            .get("lpp_arena_release_node")
            .ok_or_else(|| "Builtin 'lpp_arena_release_node' was not declared".to_string())?;

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
                let arena_release_ref = self
                    .module
                    .declare_func_in_func(arena_release_id, builder.func);
                let (layout, _) = struct_layout(type_table, struct_id);

                for ((field_name, field_type), field_layout) in
                    definition.fields.iter().zip(layout.iter())
                {
                    // A field demoted by the static cycle breaker is NOT an
                    // owning edge: it was stored without a retain, so releasing
                    // it here would over-release. Skipping it is precisely what
                    // makes the cycle collectable -- the remaining owning
                    // subgraph is acyclic, so refcounts reach zero.
                    if self.weak_fields.contains(&(struct_id, field_name.clone())) {
                        continue;
                    }
                    if field_type.is_managed() {
                        let child = builder.ins().load(
                            cl_types::I64,
                            MemFlags::new(),
                            payload,
                            field_layout.offset as i32,
                        );
                        let release = match field_type {
                            TypeRef::Custom(child_id)
                                if type_table
                                    .definitions
                                    .get(child_id.0)
                                    .map(|child| child.is_self_referential)
                                    .unwrap_or(false) =>
                            {
                                arena_release_ref
                            }
                            _ => release_ref,
                        };
                        builder.ins().call(release, &[child]);
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
        // Deterministic order: `program.functions` is a HashMap, so iterating it
        // directly makes symbol layout (and therefore the object file) differ
        // between runs of the *same* compiler on the *same* input.
        let mut ordered: Vec<(&FuncId, &MirFunction)> = program.functions.iter().collect();
        ordered.sort_by_key(|(id, _)| id.0);
        for (mir_id, mir_fn) in ordered {
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
                .declare_function(symbol_name, Linkage::Export, &sig)
                .map_err(|e| format!("declare '{}': {:?}", mir_fn.name, e))?;
            self.func_ids.insert(*mir_id, id);
        }
        Ok(())
    }

    pub fn declare_task_thunks(&mut self, program: &MirProgram) -> Result<(), String> {
        let mut functions: Vec<_> = program.functions.values().filter(|f| f.is_async).collect();
        functions.sort_by_key(|f| f.id.0);
        for function in functions {
            let mut signature = self.module.make_signature();
            signature.params.push(AbiParam::new(cl_types::I64));
            signature.returns.push(AbiParam::new(cl_types::I64));
            let id = self
                .module
                .declare_function(
                    &format!("__lpp_task_thunk_{}", function.id.0),
                    Linkage::Export,
                    &signature,
                )
                .map_err(|error| format!("declare task thunk '{}': {:?}", function.name, error))?;
            self.task_thunk_ids.insert(function.id, id);
        }
        Ok(())
    }

    pub fn lower_task_thunks(&mut self, program: &MirProgram) -> Result<(), String> {
        let mut functions: Vec<_> = program.functions.values().filter(|f| f.is_async).collect();
        functions.sort_by_key(|f| f.id.0);
        for function in functions {
            let thunk_id = self.task_thunk_ids[&function.id];
            let target_id = self.func_ids[&function.id];
            let mut context = self.module.make_context();
            context
                .func
                .signature
                .params
                .push(AbiParam::new(cl_types::I64));
            context
                .func
                .signature
                .returns
                .push(AbiParam::new(cl_types::I64));
            context.func.name = cranelift_codegen::ir::UserFuncName::user(0, thunk_id.as_u32());
            let mut builder_context = FunctionBuilderContext::new();
            {
                let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_context);
                let entry = builder.create_block();
                builder.switch_to_block(entry);
                builder.append_block_params_for_function_params(entry);
                let environment = builder.block_params(entry)[0];
                let parameter_types: Vec<TypeRef> = function
                    .params
                    .iter()
                    .map(|id| function.locals[id.0].ty.clone())
                    .collect();
                let (layout, _) = tuple_layout(&parameter_types);
                let mut arguments = Vec::with_capacity(layout.len());
                for field in &layout {
                    arguments.push(builder.ins().load(
                        abi_to_cl(field.abi),
                        MemFlags::new(),
                        environment,
                        field.offset as i32,
                    ));
                }
                let target = self.module.declare_func_in_func(target_id, builder.func);
                let call = builder.ins().call(target, &arguments);
                let raw = if function.return_type == TypeRef::Void {
                    builder.ins().iconst(cl_types::I64, 0)
                } else {
                    let value = builder.inst_results(call)[0];
                    match &function.return_type {
                        TypeRef::Bool => builder.ins().uextend(cl_types::I64, value),
                        TypeRef::Float => {
                            let slot = builder.create_sized_stack_slot(
                                cranelift_codegen::ir::StackSlotData::new(
                                    cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                                    8,
                                    3,
                                ),
                            );
                            let address = builder.ins().stack_addr(cl_types::I64, slot, 0);
                            builder.ins().store(MemFlags::trusted(), value, address, 0);
                            builder
                                .ins()
                                .load(cl_types::I64, MemFlags::trusted(), address, 0)
                        }
                        _ => value,
                    }
                };
                builder.ins().return_(&[raw]);
                builder.seal_all_blocks();
                builder.finalize();
            }
            self.module
                .define_function(thunk_id, &mut context)
                .map_err(|error| format!("define task thunk '{}': {:?}", function.name, error))?;
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
        let (user_main_id, user_main) = program
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
            if user_main.is_async {
                let tuple_alloc = self
                    .module
                    .declare_func_in_func(self.builtin_ids["lpp_tuple_alloc"], builder.func);
                let size = builder.ins().iconst(cl_types::I64, 16);
                let zero = builder.ins().iconst(cl_types::I64, 0);
                let allocation = builder.ins().call(tuple_alloc, &[size, zero, zero]);
                let environment = builder.inst_results(allocation)[0];
                let thunk = self
                    .module
                    .declare_func_in_func(self.task_thunk_ids[user_main_id], builder.func);
                let thunk_address = builder.ins().func_addr(cl_types::I64, thunk);
                let task_new = self
                    .module
                    .declare_func_in_func(self.builtin_ids["lpp_task_new"], builder.func);
                let created = builder
                    .ins()
                    .call(task_new, &[thunk_address, environment, zero]);
                let task = builder.inst_results(created)[0];
                let await_id = self
                    .module
                    .declare_func_in_func(self.builtin_ids["lpp_task_await"], builder.func);
                builder.ins().call(await_id, &[task]);
                let release = self
                    .module
                    .declare_func_in_func(self.builtin_ids["lpp_arc_release"], builder.func);
                builder.ins().call(release, &[task]);
            } else {
                let main_ref = self.module.declare_func_in_func(main_id, builder.func);
                builder.ins().call(main_ref, &[]);
            }
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
        // Same deterministic ordering as declare_functions.
        let mut ordered: Vec<(&FuncId, &MirFunction)> = program.functions.iter().collect();
        ordered.sort_by_key(|(id, _)| id.0);
        // Do not clone the whole MIR up front: that would hold every function's
        // locals+blocks in memory alongside the IR contexts, defeating the
        // bounded-memory goal below. We keep only sorted (id, ref) pairs and
        // clone one batch of functions at a time.
        let refs: Vec<(&FuncId, &MirFunction)> = ordered;

        // Bounded-batch codegen.
        //
        // The classic pipeline builds Cranelift IR for *every* function and
        // holds all of it in a single `pending` Vec before the (parallel)
        // regalloc/instruction-selection phase. Peak memory is therefore
        // O(whole program): every IR context + every compiled body lives at
        // once. That does not scale to very large programs.
        //
        // Instead we process functions in bounded batches: lower a batch, emit
        // it (parallel within the batch), define the bodies into the module,
        // then drop the batch contexts before the next batch. Peak memory
        // becomes O(batch_size) regardless of program size. Batch size is
        // configurable via `LPP_CODEGEN_BATCH`; the default is chosen to keep
        // per-batch IR + machine code modest while still amortising the
        // per-batch thread-pool spin-up.
        let batch = codegen_batch_size(refs.len());
        let arc_non_atomic = self.arc_non_atomic;
        // Per-local escape facts are small and are consulted for every batch,
        // so take a cheap clone once. This avoids holding an immutable borrow
        // of `self.shared_locals` across the `&mut self` calls that define each
        // batch's machine code back into the module.
        let shared_by_fn = std::mem::take(&mut self.shared_locals);

        for chunk in refs.chunks(batch) {
            // Phase 1 (serial per batch): build Cranelift IR.
            // IR construction must stay serial because it mutates the module:
            // `declare_func_in_func` interns callee references and string
            // literals are declared as new data objects on demand.
            let mut pending: Vec<(cranelift_module::FuncId, cranelift_codegen::Context)> =
                Vec::with_capacity(chunk.len());
            for (_, mir_fn) in chunk {
                if mir_fn.blocks.is_empty() {
                    continue;
                }
                let mut lower = FunctionLower {
                    module: &mut self.module,
                    func_ids: &self.func_ids,
                    builtin_ids: &mut self.builtin_ids,
                    drop_ids: &self.drop_ids,
                    task_thunk_ids: &self.task_thunk_ids,
                    type_table,
                    fn_name: mir_fn.name.clone(),
                    next_str_idx: 0,
                    arc_non_atomic,
                    shared_locals: shared_by_fn.get(&mir_fn.id),
                };
                let (func_id, ctx) = lower.build_function_ir(mir_fn)?;
                pending.push((func_id, ctx));
            }
            if pending.is_empty() {
                continue;
            }

            // Phase 2 (parallel within the batch): optimise + emit machine code.
            // This is the expensive half — regalloc and instruction selection —
            // and each function is independent, so it scales across cores.
            // Defining the results back into the module (phase 3) stays serial
            // and is cheap.
            let threads = codegen_threads(pending.len());
            if threads > 1 {
                self.define_functions_parallel(pending, threads)?;
            } else {
                for (func_id, mut ctx) in pending {
                    self.module
                        .define_function(func_id, &mut ctx)
                        .map_err(|e| format!("define_function: {:?}", e))?;
                }
            }
            // `pending` (and each batch's contexts) drops here, bounding peak
            // memory to roughly one batch of IR + machine code.
        }
        Ok(())
    }

    /// Compile function bodies on a worker pool, then define them in order.
    ///
    /// Results are collected keyed by their original index so the object file
    /// is byte-for-byte identical to a serial build; only wall time changes.
    fn define_functions_parallel(
        &mut self,
        pending: Vec<(cranelift_module::FuncId, cranelift_codegen::Context)>,
        threads: usize,
    ) -> Result<(), String> {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Mutex};

        let isa = self.module.isa();
        let total = pending.len();
        let next = AtomicUsize::new(0);
        let inputs: Vec<Mutex<Option<(cranelift_module::FuncId, cranelift_codegen::Context)>>> =
            pending
                .into_iter()
                .map(|item| Mutex::new(Some(item)))
                .collect();
        let inputs = Arc::new(inputs);
        let results: Vec<Mutex<Option<CompiledFunction>>> =
            (0..total).map(|_| Mutex::new(None)).collect();
        let results = Arc::new(results);
        let failure: Mutex<Option<String>> = Mutex::new(None);

        std::thread::scope(|scope| {
            for _ in 0..threads {
                let inputs = Arc::clone(&inputs);
                let results = Arc::clone(&results);
                let next = &next;
                let failure = &failure;
                scope.spawn(move || {
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        if index >= total {
                            break;
                        }
                        if failure.lock().map(|g| g.is_some()).unwrap_or(true) {
                            break;
                        }
                        let taken = inputs[index].lock().ok().and_then(|mut slot| slot.take());
                        let (func_id, mut ctx) = match taken {
                            Some(item) => item,
                            None => continue,
                        };
                        let mut ctrl = cranelift_codegen::control::ControlPlane::default();
                        let fn_label = ctx.func.name.to_string();
                        match ctx.compile(isa, &mut ctrl) {
                            Ok(code) => {
                                let compiled = CompiledFunction {
                                    func_id,
                                    alignment: code.buffer.alignment as u64,
                                    bytes: code.code_buffer().to_vec(),
                                    relocs: code.buffer.relocs().to_vec(),
                                    func: ctx.func.clone(),
                                };
                                if let Ok(mut slot) = results[index].lock() {
                                    *slot = Some(compiled);
                                }
                            }
                            Err(error) => {
                                if let Ok(mut slot) = failure.lock() {
                                    if slot.is_none() {
                                        *slot = Some(format!(
                                            "define_function '{}': {:?}",
                                            fn_label, error.inner
                                        ));
                                    }
                                }
                                break;
                            }
                        }
                    }
                });
            }
        });

        if let Some(message) = failure
            .into_inner()
            .map_err(|_| "codegen thread panicked")?
        {
            return Err(message);
        }

        // Phase 3 (serial): register the finished bodies in source order.
        let results = Arc::try_unwrap(results).map_err(|_| "codegen results still shared")?;
        for slot in results {
            let compiled = slot
                .into_inner()
                .map_err(|_| "codegen result poisoned")?
                .ok_or_else(|| "codegen produced no result for a function".to_string())?;
            self.module
                .define_function_bytes(
                    compiled.func_id,
                    &compiled.func,
                    compiled.alignment,
                    &compiled.bytes,
                    &compiled.relocs,
                )
                .map_err(|e| format!("define_function_bytes: {:?}", e))?;
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
        Self::compile_with_options(
            program,
            type_table,
            false,
            &std::collections::HashSet::new(),
        )
    }

    /// `has_extern` reports whether the source declared any FFI block. Foreign
    /// code can spawn threads with no MIR evidence, so it forces atomic ARC.
    pub fn compile_with_options(
        program: &MirProgram,
        type_table: &TypeTable,
        has_extern: bool,
        weak_fields: &std::collections::HashSet<(StructTypeId, String)>,
    ) -> Result<Vec<u8>, String> {
        Self::compile_with_options_target(program, type_table, has_extern, weak_fields, None)
    }

    /// Like [`compile_with_options`] but accepts an optional target triple so a
    /// `--target` flag can select a non-host ISA (Android/Termux aarch64, etc).
    pub fn compile_with_options_target(
        program: &MirProgram,
        type_table: &TypeTable,
        has_extern: bool,
        weak_fields: &std::collections::HashSet<(StructTypeId, String)>,
        target: Option<&str>,
    ) -> Result<Vec<u8>, String> {
        validate_aot_program(program, type_table)?;
        let mut c = Self::new_for_target(target)?;
        c.weak_fields = weak_fields.clone();
        c.arc_non_atomic =
            crate::mir::pass_arc_local::is_provably_single_threaded(program, has_extern);
        // Per-object refinement of the same question. The whole-program proof
        // above is all-or-nothing: one `spawn` anywhere makes every refcount in
        // the program atomic. The escape solver already knows which individual
        // locals can reach a second thread, so the rest can keep the cheap
        // entry points even when the program does spawn.
        //
        // Skipped entirely when FFI is present: foreign code can share an
        // object without any MIR evidence, so no per-local claim is safe.
        if !has_extern {
            let facts = crate::mir::escape_solver::solve(program);
            for (fid, function) in &program.functions {
                let mut shared = std::collections::HashSet::new();
                if let Some(f) = facts.functions.get(fid) {
                    for local in &function.locals {
                        if f.locals.get(local.id.0).copied()
                            == Some(crate::mir::escape_solver::Storage::Shared)
                        {
                            shared.insert(local.id);
                        }
                    }
                }
                c.shared_locals.insert(*fid, shared);
            }
        }
        c.declare_builtins(program)?;
        c.declare_drop_functions(type_table)?;
        c.declare_functions(program)?;
        c.declare_task_thunks(program)?;
        c.declare_entrypoint_wrapper(program)?;
        c.lower_drop_functions(type_table)?;
        c.lower_task_thunks(program)?;
        c.lower_functions(program, type_table)?;
        c.lower_entrypoint_wrapper(program)?;
        c.finish()
    }
}
