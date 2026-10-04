use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use lpp_hir::{DefId, ModuleId, Symbol, TypeParamId};

use crate::ids::{InferVarId, TypeId, TypeListId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PrimitiveType {
    Void,
    Bool,
    Int,
    Float,
    String,
    Char,
    U8,
    U16,
    U32,
    I8,
    I16,
    I32,
    StrSlice,
    VectorI64x2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeKind {
    Error,
    Never,
    Primitive(PrimitiveType),
    Tuple(TypeListId),
    List(TypeId),
    Map {
        key: TypeId,
        value: TypeId,
    },
    Slice(TypeId),
    Task(TypeId),
    Function {
        parameters: TypeListId,
        result: TypeId,
    },
    Nominal {
        definition: DefId,
        arguments: TypeListId,
    },
    GenericParameter(TypeParamId),
    BoundVariable(u32),
    InferenceVariable(InferVarId),
    UnresolvedName {
        module: ModuleId,
        name: Symbol,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeInterner {
    kinds: Vec<TypeKind>,
    by_kind: HashMap<TypeKind, TypeId>,
    lists: Vec<Arc<[TypeId]>>,
    by_list: HashMap<Arc<[TypeId]>, TypeListId>,
    primitives: PrimitiveIds,
    empty_list: TypeListId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PrimitiveIds {
    error: TypeId,
    never: TypeId,
    void: TypeId,
    bool_: TypeId,
    int: TypeId,
    float: TypeId,
    string: TypeId,
    char_: TypeId,
    u8_: TypeId,
    u16_: TypeId,
    u32_: TypeId,
    i8_: TypeId,
    i16_: TypeId,
    i32_: TypeId,
    str_slice: TypeId,
    vector_i64x2: TypeId,
}

impl TypeInterner {
    #[must_use]
    pub fn new() -> Self {
        let placeholder = TypeId::from_raw(0);
        let list_placeholder = TypeListId::from_raw(0);
        let mut interner = Self {
            kinds: Vec::with_capacity(32),
            by_kind: HashMap::with_capacity(32),
            lists: Vec::with_capacity(16),
            by_list: HashMap::with_capacity(16),
            primitives: PrimitiveIds {
                error: placeholder,
                never: placeholder,
                void: placeholder,
                bool_: placeholder,
                int: placeholder,
                float: placeholder,
                string: placeholder,
                char_: placeholder,
                u8_: placeholder,
                u16_: placeholder,
                u32_: placeholder,
                i8_: placeholder,
                i16_: placeholder,
                i32_: placeholder,
                str_slice: placeholder,
                vector_i64x2: placeholder,
            },
            empty_list: list_placeholder,
        };
        interner.empty_list = interner
            .intern_list(&[])
            .expect("the first type list fits the compact ID range");
        interner.primitives = PrimitiveIds {
            error: interner
                .intern(TypeKind::Error)
                .expect("primitive types fit the compact ID range"),
            never: interner
                .intern(TypeKind::Never)
                .expect("primitive types fit the compact ID range"),
            void: interner
                .intern(TypeKind::Primitive(PrimitiveType::Void))
                .expect("primitive types fit the compact ID range"),
            bool_: interner
                .intern(TypeKind::Primitive(PrimitiveType::Bool))
                .expect("primitive types fit the compact ID range"),
            int: interner
                .intern(TypeKind::Primitive(PrimitiveType::Int))
                .expect("primitive types fit the compact ID range"),
            float: interner
                .intern(TypeKind::Primitive(PrimitiveType::Float))
                .expect("primitive types fit the compact ID range"),
            string: interner
                .intern(TypeKind::Primitive(PrimitiveType::String))
                .expect("primitive types fit the compact ID range"),
            char_: interner
                .intern(TypeKind::Primitive(PrimitiveType::Char))
                .expect("primitive types fit the compact ID range"),
            u8_: interner
                .intern(TypeKind::Primitive(PrimitiveType::U8))
                .expect("primitive types fit the compact ID range"),
            u16_: interner
                .intern(TypeKind::Primitive(PrimitiveType::U16))
                .expect("primitive types fit the compact ID range"),
            u32_: interner
                .intern(TypeKind::Primitive(PrimitiveType::U32))
                .expect("primitive types fit the compact ID range"),
            i8_: interner
                .intern(TypeKind::Primitive(PrimitiveType::I8))
                .expect("primitive types fit the compact ID range"),
            i16_: interner
                .intern(TypeKind::Primitive(PrimitiveType::I16))
                .expect("primitive types fit the compact ID range"),
            i32_: interner
                .intern(TypeKind::Primitive(PrimitiveType::I32))
                .expect("primitive types fit the compact ID range"),
            str_slice: interner
                .intern(TypeKind::Primitive(PrimitiveType::StrSlice))
                .expect("primitive types fit the compact ID range"),
            vector_i64x2: interner
                .intern(TypeKind::Primitive(PrimitiveType::VectorI64x2))
                .expect("primitive types fit the compact ID range"),
        };
        interner
    }

    pub fn intern(&mut self, kind: TypeKind) -> Result<TypeId, TypeInternerExhausted> {
        if let Some(id) = self.by_kind.get(&kind) {
            return Ok(*id);
        }
        let id = TypeId::from_index(self.kinds.len()).ok_or(TypeInternerExhausted)?;
        self.kinds.push(kind);
        self.by_kind.insert(kind, id);
        Ok(id)
    }

    pub fn intern_list(&mut self, types: &[TypeId]) -> Result<TypeListId, TypeInternerExhausted> {
        if let Some(id) = self.by_list.get(types) {
            return Ok(*id);
        }
        let id = TypeListId::from_index(self.lists.len()).ok_or(TypeInternerExhausted)?;
        let types: Arc<[TypeId]> = Arc::from(types);
        self.lists.push(types.clone());
        self.by_list.insert(types, id);
        Ok(id)
    }

    #[must_use]
    pub fn kind(&self, id: TypeId) -> TypeKind {
        self.kinds[id.index()]
    }

    #[must_use]
    pub fn list(&self, id: TypeListId) -> &[TypeId] {
        &self.lists[id.index()]
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.kinds.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }

    #[must_use]
    pub fn list_count(&self) -> usize {
        self.lists.len()
    }

    pub fn types(&self) -> impl ExactSizeIterator<Item = (TypeId, TypeKind)> + '_ {
        self.kinds.iter().copied().enumerate().map(|(index, kind)| {
            (
                TypeId::from_index(index).expect("interned type indices fit their ID"),
                kind,
            )
        })
    }

    pub fn type_lists(&self) -> impl ExactSizeIterator<Item = (TypeListId, &[TypeId])> + '_ {
        self.lists.iter().enumerate().map(|(index, types)| {
            (
                TypeListId::from_index(index).expect("interned type-list indices fit their ID"),
                types.as_ref(),
            )
        })
    }

    #[must_use]
    pub const fn error(&self) -> TypeId {
        self.primitives.error
    }

    #[must_use]
    pub const fn never(&self) -> TypeId {
        self.primitives.never
    }

    #[must_use]
    pub const fn primitive(&self, primitive: PrimitiveType) -> TypeId {
        match primitive {
            PrimitiveType::Void => self.primitives.void,
            PrimitiveType::Bool => self.primitives.bool_,
            PrimitiveType::Int => self.primitives.int,
            PrimitiveType::Float => self.primitives.float,
            PrimitiveType::String => self.primitives.string,
            PrimitiveType::Char => self.primitives.char_,
            PrimitiveType::U8 => self.primitives.u8_,
            PrimitiveType::U16 => self.primitives.u16_,
            PrimitiveType::U32 => self.primitives.u32_,
            PrimitiveType::I8 => self.primitives.i8_,
            PrimitiveType::I16 => self.primitives.i16_,
            PrimitiveType::I32 => self.primitives.i32_,
            PrimitiveType::StrSlice => self.primitives.str_slice,
            PrimitiveType::VectorI64x2 => self.primitives.vector_i64x2,
        }
    }

    #[must_use]
    pub const fn empty_list(&self) -> TypeListId {
        self.empty_list
    }
}

impl Default for TypeInterner {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeInternerExhausted;

impl fmt::Display for TypeInternerExhausted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("type interner exhausted its compact ID space")
    }
}

impl std::error::Error for TypeInternerExhausted {}
