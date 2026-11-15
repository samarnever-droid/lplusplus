//! `lpp-codegen-llvm` — the LLVM IR backend for L++.
//!
//! Emits typed LLVM IR text from optimized/revalidated MIR and lowers it
//! through `clang` to relocatable native objects (`.o`) matching Cranelift's
//! native ABI, ARC lifecycle, cycle collection, closures, async tasks,
//! collections, SIMD, FFI, and freestanding kernel targets.
#![allow(clippy::all, warnings)]

use std::path::PathBuf;
use std::process::Command;

use lpp_codegen_api::{
    Backend, CodegenError, CodegenErrorKind, CodegenOptions, CompiledModule, NameResolver,
    OptLevel, Target,
};
use lpp_mir::MirProgram;
use lpp_types::TypeInterner;

mod lower;

pub use lower::emit_llvm_ir;

/// The LLVM IR native backend unit struct implementing [`Backend`].
pub struct LlvmBackend;

const TARGETS: &[Target] = &[
    Target::X86_64,
    Target::Aarch64,
    Target::X86_64None,
    Target::Aarch64None,
];

impl Backend for LlvmBackend {
    fn name(&self) -> &'static str {
        "llvm"
    }

    fn targets(&self) -> &'static [Target] {
        TARGETS
    }

    fn compile_module(
        &self,
        program: &MirProgram,
        types: &TypeInterner,
        options: &CodegenOptions<'_>,
    ) -> Result<CompiledModule, CodegenError> {
        if !TARGETS.contains(&options.target) {
            return Err(CodegenError::new(
                None,
                CodegenErrorKind::UnsupportedTarget(options.target),
            ));
        }

        let lowered = lower::emit_llvm_ir_with_options(program, types, options)?;
        let object = clang_object(
            &lowered.ir,
            options.target,
            options.opt_level,
            options.debug_info,
        )?;

        Ok(CompiledModule {
            object,
            target: options.target,
            exported_symbols: lowered.exported_symbols,
            imported_symbols: lowered.imported_symbols,
            link_libraries: lowered.link_libraries,
            entry: lowered.entry,
        })
    }
}

/// Compiler resolution: `LPP_LLVM_CC` env var or PATH lookup
/// (`clang`, then versioned `clang-18`..`clang-17`).
pub fn resolve_llvm_cc() -> Option<String> {
    if let Ok(env_cc) = std::env::var("LPP_LLVM_CC") {
        let trimmed = env_cc.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    for candidate in ["clang", "clang-18", "clang-19", "clang-17"] {
        if Command::new(candidate)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
        {
            return Some(candidate.to_string());
        }
    }
    None
}

pub(crate) fn llvm_target_triple(target: Target) -> &'static str {
    match target {
        Target::X86_64 => {
            if cfg!(target_os = "windows") {
                "x86_64-pc-windows-msvc"
            } else if cfg!(target_os = "macos") {
                "x86_64-apple-darwin"
            } else {
                "x86_64-unknown-linux-gnu"
            }
        }
        Target::Aarch64 => {
            if cfg!(target_os = "windows") {
                "aarch64-pc-windows-msvc"
            } else if cfg!(target_os = "macos") {
                "arm64-apple-darwin"
            } else {
                "aarch64-unknown-linux-gnu"
            }
        }
        Target::X86_64None => "x86_64-unknown-none-elf",
        Target::Aarch64None => "aarch64-unknown-none-elf",
        Target::Wasm32Wasi => "wasm32-wasip1",
    }
}

fn opt_flag(level: OptLevel) -> &'static str {
    match level {
        OptLevel::None => "-O0",
        OptLevel::O1 => "-O1",
        OptLevel::O2 => "-O2",
        OptLevel::O3 | OptLevel::O4 => "-O3",
        OptLevel::Os => "-Os",
        OptLevel::Oz => "-Oz",
    }
}

fn clang_object(
    ir: &str,
    target: Target,
    opt_level: OptLevel,
    debug_info: bool,
) -> Result<Vec<u8>, CodegenError> {
    let compiler = resolve_llvm_cc().ok_or_else(|| {
        CodegenError::new(
            None,
            CodegenErrorKind::ObjectEmissionFailed(
                "clang compiler not found (set LPP_LLVM_CC or install clang)".to_string(),
            ),
        )
    })?;

    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let unique = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("lpp_llvm_{}_{}", std::process::id(), unique));
    std::fs::create_dir_all(&dir).map_err(|e| {
        CodegenError::new(
            None,
            CodegenErrorKind::ObjectEmissionFailed(format!("create temp dir: {e}")),
        )
    })?;
    let ir_path: PathBuf = dir.join("lpp_module.ll");
    let obj_path: PathBuf = dir.join("lpp_module.o");

    std::fs::write(&ir_path, ir).map_err(|e| {
        let _ = std::fs::remove_dir_all(&dir);
        CodegenError::new(
            None,
            CodegenErrorKind::ObjectEmissionFailed(format!("write llvm ir: {e}")),
        )
    })?;

    let mut cmd = Command::new(&compiler);
    cmd.arg("-x")
        .arg("ir")
        .arg("-c")
        .arg(opt_flag(opt_level))
        .arg(format!("--target={}", llvm_target_triple(target)));
    if debug_info {
        cmd.arg("-g");
    }

    if target.is_freestanding() {
        cmd.arg("-ffreestanding").arg("-fno-pic");
        if target == Target::X86_64None {
            cmd.arg("-mno-red-zone");
        }
    }

    let output = cmd
        .arg("-o")
        .arg(&obj_path)
        .arg(&ir_path)
        .output()
        .map_err(|e| {
            let _ = std::fs::remove_dir_all(&dir);
            CodegenError::new(
                None,
                CodegenErrorKind::ObjectEmissionFailed(format!("spawn {compiler}: {e}")),
            )
        })?;

    if !output.status.success() {
        let _ = std::fs::remove_dir_all(&dir);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CodegenError::new(
            None,
            CodegenErrorKind::ObjectEmissionFailed(format!(
                "{compiler} rejected LLVM IR:\n{stderr}\n--- IR ---\n{ir}"
            )),
        ));
    }

    let bytes = std::fs::read(&obj_path).map_err(|e| {
        CodegenError::new(
            None,
            CodegenErrorKind::ObjectEmissionFailed(format!("read llvm object: {e}")),
        )
    });
    let _ = std::fs::remove_dir_all(&dir);
    bytes
}

pub(crate) fn _resolve_name(names: &dyn NameResolver, id: u32) -> Option<&str> {
    names.resolve(id)
}
