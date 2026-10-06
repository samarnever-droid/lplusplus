//! `lpp-codegen-cranelift` — the Phase 5B/5C Cranelift backend.
//!
//! Lowers the 5B/5C MIR slice (scalars, CFG, direct calls, string
//! constants, `print_str`, plus the 5C aggregate data surface:
//! structs, enums with dense `SwitchEnum` dispatch, lists, projected
//! load/store, and the ARC traffic that keeps them honest) to
//! cranelift IR and emits a host-format ELF, Mach-O, or COFF relocatable
//! through cranelift-object. The lowering matches the v1 cranelift backend's
//! conventions: function order by `MirFunctionId`, `lpp_main` plus a
//! generated C-ABI `main` wrapper, the 24-byte string-constant header
//! (whose two magic words are simultaneously the host-runtime magic
//! and the freestanding immortal sentinel), and cranelift settings
//! with `opt_level = "speed"`.
//!
//! Anything outside the slice is a typed `CodegenError` (the 5A
//! contract), never a partial object and never a miscompile.

// The shared aggregate layout (5C/5D2a) lives in `lpp-codegen-api` so
// the native and wasm backends use the identical field-offset
// computation by construction.
mod layout {
    pub(crate) use lpp_codegen_api::layout::*;
}
mod lower;

use cranelift_codegen::settings::{self, Configurable};
use cranelift_object::{ObjectBuilder, ObjectModule};
use lpp_codegen_api::{
    Backend, CodegenError, CodegenErrorKind, CodegenOptions, CompiledModule, Target,
};
use lpp_mir::MirProgram;
use lpp_types::{BuiltinId, TypeInterner};

use lower::{Lowering, pre_scan};

/// The Cranelift backend for the native Phase 5 targets.
#[derive(Debug, Default)]
pub struct CraneliftBackend;

impl Backend for CraneliftBackend {
    fn name(&self) -> &'static str {
        "cranelift"
    }

    fn targets(&self) -> &'static [Target] {
        &[Target::X86_64, Target::Aarch64]
    }

    fn compile_module(
        &self,
        program: &MirProgram,
        types: &TypeInterner,
        options: &CodegenOptions<'_>,
    ) -> Result<CompiledModule, CodegenError> {
        assert_table_shape();

        if !self.targets().contains(&options.target) {
            return Err(CodegenError::new(
                None,
                CodegenErrorKind::UnsupportedTarget(options.target),
            ));
        }

        let mut flags = settings::builder();
        // The v1 AOT settings: no colocated libcalls, PIC, speed.
        flags.set("use_colocated_libcalls", "false").map_err(|e| {
            CodegenError::new(None, CodegenErrorKind::ObjectEmissionFailed(e.to_string()))
        })?;
        flags.set("is_pic", "true").map_err(|e| {
            CodegenError::new(None, CodegenErrorKind::ObjectEmissionFailed(e.to_string()))
        })?;
        flags.set("opt_level", "speed").map_err(|e| {
            CodegenError::new(None, CodegenErrorKind::ObjectEmissionFailed(e.to_string()))
        })?;

        let triple = options
            .target
            .triple()
            .parse::<target_lexicon::Triple>()
            .map_err(|e| {
                CodegenError::new(None, CodegenErrorKind::ObjectEmissionFailed(e.to_string()))
            })?;
        let isa_builder = cranelift_codegen::isa::lookup(triple).map_err(|e| {
            CodegenError::new(None, CodegenErrorKind::ObjectEmissionFailed(e.to_string()))
        })?;
        let isa = isa_builder
            .finish(settings::Flags::new(flags))
            .map_err(|e| {
                CodegenError::new(None, CodegenErrorKind::ObjectEmissionFailed(e.to_string()))
            })?;

        let builder =
            ObjectBuilder::new(isa, "lpp_module", cranelift_module::default_libcall_names())
                .map_err(|e| {
                    CodegenError::new(None, CodegenErrorKind::ObjectEmissionFailed(e.to_string()))
                })?;
        let mut module = ObjectModule::new(builder);

        // Typed rejection happens up front: the module never holds a
        // partial lowering.
        let plan = pre_scan(program, types, options.names)?;

        let mut lowering = Lowering::new(
            &mut module,
            plan.aggregates.clone(),
            plan.layouts.clone(),
            plan.value_classes.clone(),
        );

        lowering.declare(&plan, program, types)?;
        lowering.lower_destructors(types, &plan)?;
        lowering.lower_functions(program, types, &plan)?;
        lowering.lower_main_wrapper(program, types)?;

        // The census is the module's own declaration record, read
        // before the module moves into the emitter.
        let exported_symbols = lowering.exported_symbols();
        let imported_symbols = lowering.imported_symbols();
        let entry = lowering.entry();
        drop(lowering);

        let object = module.finish().emit().map_err(|e| {
            CodegenError::new(None, CodegenErrorKind::ObjectEmissionFailed(e.to_string()))
        })?;

        Ok(CompiledModule {
            target: options.target,
            object,
            exported_symbols,
            imported_symbols,
            entry,
        })
    }
}

/// The `BuiltinId`s of the 5C builtin subset (identity, not
/// per-occurrence string matching). `assert_table_shape` at the top of
/// every `compile_module` keeps each index in lockstep with the
/// checked-in table: any drift fails the compilation loudly.
pub(crate) const PRINT_STR: BuiltinId = BuiltinId::from_raw(14);
pub(crate) const LIST_NEW: BuiltinId = BuiltinId::from_raw(46);

fn assert_table_shape() {
    let descriptor = PRINT_STR.descriptor();
    assert!(
        descriptor.name == "print_str" && descriptor.symbol == "lpp_print_str",
        "generated table drift at index 14: {:?} / {:?}",
        descriptor.name,
        descriptor.symbol,
    );
    let descriptor = LIST_NEW.descriptor();
    assert!(
        descriptor.name == "list_new" && descriptor.symbol == "lpp_list_new",
        "generated table drift at index 46: {:?} / {:?}",
        descriptor.name,
        descriptor.symbol,
    );
}
