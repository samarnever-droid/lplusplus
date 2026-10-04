//! The `lpp-codegen-llvm` backend (Phase 5E, slice 1).
//!
//! `LlvmBackend` implements `Backend` for `Target::X86_64`, emitting
//! **textual LLVM IR** and compiling it to an ELF relocatable object via a
//! shell-out to `clang -c`. The object links against the 5B/5C `c_shim.c`
//! runtime (the gate does the link, mirroring the 5B gate). The scalar
//! data surface (scalars, CFG, direct calls, the scalar/str/bool/char
//! builtin subset) is supported, plus float printing (`print_float`, the
//! 5E2 float-output slice); the managed data surface, the function-value
//! surface, slices, and SIMD remain typed rejections (5E2).

use std::collections::BTreeSet;
use std::process::Command;

use lpp_codegen_api::{
    Backend, CodegenError, CodegenErrorKind, CodegenOptions, CompiledModule, Target,
};
use lpp_mir::MirProgram;
use lpp_types::TypeInterner;

mod lower;

/// The LLVM backend.
pub struct LlvmBackend;

const TARGETS: [Target; 1] = [Target::X86_64];

impl Backend for LlvmBackend {
    fn name(&self) -> &'static str {
        "llvm"
    }

    fn targets(&self) -> &'static [Target] {
        &TARGETS
    }

    fn compile_module(
        &self,
        program: &MirProgram,
        types: &TypeInterner,
        options: &CodegenOptions<'_>,
    ) -> Result<CompiledModule, CodegenError> {
        if options.target != Target::X86_64 {
            return Err(CodegenError::new(
                None,
                CodegenErrorKind::UnsupportedTarget(options.target),
            ));
        }

        // Reject non-slice-1 constructs (typed) and collect the module plan.
        let plan = lower::build_plan(program, types, options.names)?;
        let ir = lower::lower_module(program, types, &plan);

        let object = clang_object(&ir)?;

        let exported_symbols = plan.symbols.values().cloned().collect();
        let imported_symbols: BTreeSet<String> =
            plan.builtins.iter().map(|s| (*s).to_owned()).collect();
        let entry = plan
            .symbols
            .values()
            .any(|s| s == "main")
            .then(|| "main".to_owned());

        Ok(CompiledModule {
            target: Target::X86_64,
            object,
            exported_symbols,
            imported_symbols,
            entry,
        })
    }
}

/// Write the IR to a temp `.ll` and compile it to an ELF object with
/// `clang -c`, returning the object bytes.
fn clang_object(ir: &str) -> Result<Vec<u8>, CodegenError> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static INVOKE: AtomicU64 = AtomicU64::new(0);
    let emit = |message: String| CodegenError::new(None, CodegenErrorKind::ObjectEmissionFailed(message));
    // One workspace per *invocation*: cargo runs the gate's tests on
    // parallel threads of one process, and a shared `module.ll` would
    // let concurrent compiles clobber each other's IR.
    let seq = INVOKE.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("lpp5e_{}_{}", std::process::id(), seq));
    std::fs::create_dir_all(&dir).map_err(|e| emit(format!("clang workspace: {e}")))?;
    let ll = dir.join("module.ll");
    let object = dir.join("module.o");
    std::fs::write(&ll, ir).map_err(|e| emit(format!("write IR: {e}")))?;

    // `-O0`: the slice-1 lowering is a plain non-SSA alloca emitter. At `-O1`
    // and above the LLVM optimizer miscompiles it (reordering the load/store
    // pairs that carry the program state), so correctness first; optimization
    // is the 5F concern once the lowered IR is proven correct.
    let run = Command::new("clang")
        .args(["-c", "-w", "-O0"])
        .arg(&ll)
        .arg("-o")
        .arg(&object)
        .output()
        .map_err(|e| emit(format!("clang spawn: {e}")))?;
    if !run.status.success() {
        return Err(emit(format!(
            "clang -c failed:\n{}",
            String::from_utf8_lossy(&run.stderr)
        )));
    }
    std::fs::read(&object).map_err(|e| emit(format!("read object: {e}")))
}
