use cranelift_codegen::ir::types as cl_types;
use crate::typecheck::TypeRef;

/// Maps a L++ TypeRef to a Cranelift IR type.
pub fn type_to_cl(ty: &TypeRef) -> cranelift_codegen::ir::Type {
    match ty {
        TypeRef::Int              => cl_types::I64,
        TypeRef::Bool             => cl_types::I8,
        TypeRef::Str              => cl_types::I64, // pointer (MVP: null for now)
        TypeRef::Void             => cl_types::I64, // dummy; callers check != Void
        TypeRef::Custom(_)        => cl_types::I64, // opaque struct pointer
        TypeRef::Generic(_, _)    => cl_types::I64, // opaque container pointer
        TypeRef::Unresolved(_)    => cl_types::I64, // not yet resolved; treat as ptr
        TypeRef::Function         => cl_types::I64, // function pointer placeholder
    }
}
