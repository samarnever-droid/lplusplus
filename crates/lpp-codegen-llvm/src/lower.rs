//! The MIR -> textual LLVM IR emitter (5E slice 1: the scalar data surface).
//!
//! Non-SSA, alloca-based: every MIR local is an `alloca` at the function
//! entry, reads are `load`s and writes are `store`s. This trades some
//! efficiency for a straightforward, phi-free lowering that is correct for
//! the slice-1 (no managed types) differential.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use lpp_codegen_api::{CodegenError, CodegenErrorKind};
use lpp_mir::{
    BasicBlockId, BinaryOperator, Constant, InstructionKind, MirFunction, MirFunctionId,
    MirLocalId, MirProgram, Operand, Rvalue, Terminator, UnaryOperator,
};
use lpp_types::{BuiltinId, PrimitiveType, TypeId, TypeInterner, TypeKind};

/// A lowered value's LLVM type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Val {
    I64,
    F64,
    I8,
    I32,
    Ptr,
    /// A bare, non-capturing function value represented as an LLVM pointer.
    Function,
    /// A void function's return type (`define void`, `ret void`).
    Void,
}

impl Val {
    fn ty(self) -> &'static str {
        match self {
            Val::I64 => "i64",
            Val::F64 => "double",
            Val::I8 => "i8",
            Val::I32 => "i32",
            Val::Ptr | Val::Function => "ptr",
            Val::Void => "void",
        }
    }

    fn align(self) -> u32 {
        match self {
            Val::I64 | Val::F64 | Val::Ptr | Val::Function => 8,
            Val::I32 => 4,
            Val::I8 => 1,
            Val::Void => 1,
        }
    }
}

fn unsupported(construct: &'static str, fn_id: Option<MirFunctionId>) -> CodegenError {
    CodegenError::new(fn_id, CodegenErrorKind::UnsupportedConstruct { construct })
}

fn val_of_type(types: &TypeInterner, ty: TypeId) -> Result<Val, CodegenError> {
    Ok(match types.kind(ty) {
        TypeKind::Primitive(PrimitiveType::Int) => Val::I64,
        TypeKind::Primitive(PrimitiveType::Float) => Val::F64,
        TypeKind::Primitive(PrimitiveType::Bool) => Val::I8,
        TypeKind::Primitive(PrimitiveType::Char) => Val::I32,
        TypeKind::Primitive(PrimitiveType::String) => Val::Ptr,
        TypeKind::Primitive(PrimitiveType::Void) => Val::I64,
        TypeKind::Primitive(PrimitiveType::StrSlice)
        | TypeKind::Primitive(PrimitiveType::VectorI64x2) => {
            return Err(unsupported("SIMD/slice scalar", None));
        }
        // The remaining primitives are the six integer types; the native ABI
        // carries every integer as i64.
        TypeKind::Primitive(_) => Val::I64,
        TypeKind::Function { .. } => Val::Function,
        _ => return Err(unsupported("unsupported scalar type", None)),
    })
}

mod function;
mod module;
mod plan;
mod strings;

use function::emit_function;
pub(crate) use module::lower_module;
use plan::const_int_value;
pub(crate) use plan::{Plan, build_plan};
use strings::{llvm_arc_string, llvm_string};
