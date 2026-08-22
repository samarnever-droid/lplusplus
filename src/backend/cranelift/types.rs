use crate::type_facts::AbiClass;
use crate::types::TypeRef;
use cranelift_codegen::ir::types as cl_types;

pub fn abi_to_cl(abi: AbiClass) -> cranelift_codegen::ir::Type {
    match abi {
        AbiClass::Void | AbiClass::I64 | AbiClass::Pointer => cl_types::I64,
        AbiClass::I8 => cl_types::I8,
        AbiClass::I16 => cl_types::I16,
        AbiClass::I32 => cl_types::I32,
        AbiClass::F64 => cl_types::F64,
        AbiClass::VectorI64x2 => cl_types::I64X2,
    }
}

/// Maps the backend-neutral L++ ABI category to Cranelift IR.
pub fn type_to_cl(ty: &TypeRef) -> cranelift_codegen::ir::Type {
    abi_to_cl(ty.abi_class())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_types_map_to_the_expected_cranelift_abi() {
        assert_eq!(type_to_cl(&TypeRef::Bool), cl_types::I8);
        assert_eq!(type_to_cl(&TypeRef::Int), cl_types::I64);
        assert_eq!(type_to_cl(&TypeRef::Float), cl_types::F64);
        assert_eq!(type_to_cl(&TypeRef::Str), cl_types::I64);
    }
}
