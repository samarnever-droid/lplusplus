//! Core resolved type model shared by semantic analysis, MIR, and backends.
//!
//! These definitions intentionally do not live inside the type checker. The
//! checker produces them; ownership facts, layout, optimization and codegen
//! consume them without depending on checker implementation details.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StructTypeId(pub usize);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeRef {
    Int,
    Float,
    Str,
    Char,
    Void,
    Bool,
    U8,
    U16,
    U32,
    I8,
    I16,
    I32,
    Custom(StructTypeId),
    Generic(String, Vec<TypeRef>),
    Unresolved(String),
    Function,
    TypeParam(String),
    VectorI64x2,
    Tuple(Vec<TypeRef>),
    StrSlice,
    Slice(Box<TypeRef>),
    Task(Box<TypeRef>),
}

#[derive(Debug, Clone)]
pub struct StructTypeDef {
    pub name: String,
    pub fields: Vec<(String, TypeRef)>,
    pub is_self_referential: bool,
    pub repr_exact: bool,
    pub align: Option<usize>,
}

#[derive(Debug)]
pub struct TypeTable {
    pub structs_by_name: HashMap<String, StructTypeId>,
    pub definitions: Vec<StructTypeDef>,
}

impl TypeTable {
    pub fn new() -> Self {
        Self {
            structs_by_name: HashMap::new(),
            definitions: Vec::new(),
        }
    }

    pub fn lookup_struct(&self, name: &str) -> Option<StructTypeId> {
        self.structs_by_name.get(name).copied()
    }

    pub fn register_struct(&mut self, name: String) -> StructTypeId {
        let id = StructTypeId(self.definitions.len());
        self.structs_by_name.insert(name.clone(), id);
        self.definitions.push(StructTypeDef {
            name,
            fields: Vec::new(),
            is_self_referential: false,
            repr_exact: false,
            align: None,
        });
        id
    }
}
