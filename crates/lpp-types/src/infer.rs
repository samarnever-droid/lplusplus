mod model;
mod transform;

pub use model::{
    GeneralizedVariable, InferenceLevel, InferenceVariable, TypeError, TypeScheme, TypeWorkBudget,
};

use lpp_hir::OriginId;
use std::collections::HashSet;

use crate::ids::{InferVarId, TypeId};
use crate::interner::{PrimitiveType, TypeInterner, TypeKind};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InferenceTable {
    variables: Vec<InferenceVariable>,
}

impl InferenceTable {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.variables.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.variables.is_empty()
    }

    #[must_use]
    pub fn variable(&self, id: InferVarId) -> InferenceVariable {
        self.variables[id.index()]
    }

    pub fn fresh(
        &mut self,
        interner: &mut TypeInterner,
        level: InferenceLevel,
        origin: Option<OriginId>,
    ) -> Result<TypeId, TypeError> {
        let id = InferVarId::from_index(self.variables.len())
            .ok_or(TypeError::InferenceVariablesExhausted)?;
        let type_id = interner.intern(TypeKind::InferenceVariable(id))?;
        self.variables.push(InferenceVariable {
            parent: id,
            type_id,
            level,
            origin,
            binding: None,
            rank: 0,
        });
        Ok(type_id)
    }

    pub fn resolve(
        &mut self,
        interner: &TypeInterner,
        mut type_id: TypeId,
        budget: &mut TypeWorkBudget,
    ) -> Result<TypeId, TypeError> {
        loop {
            budget.consume("resolve")?;
            let TypeKind::InferenceVariable(variable) = interner.kind(type_id) else {
                return Ok(type_id);
            };
            let root = self.find_root(variable, budget)?;
            let root_variable = self.variables[root.index()];
            let Some(binding) = root_variable.binding else {
                return Ok(root_variable.type_id);
            };
            type_id = binding;
        }
    }

    pub fn unify(
        &mut self,
        interner: &mut TypeInterner,
        expected: TypeId,
        actual: TypeId,
        budget: &mut TypeWorkBudget,
    ) -> Result<TypeId, TypeError> {
        let mut pending = vec![(expected, actual)];
        let mut saw_error = false;
        while let Some((left, right)) = pending.pop() {
            budget.consume("unify")?;
            let left = self.resolve(interner, left, budget)?;
            let right = self.resolve(interner, right, budget)?;
            if left == right {
                continue;
            }
            let left_kind = interner.kind(left);
            let right_kind = interner.kind(right);
            match (left_kind, right_kind) {
                (TypeKind::Error, _) | (_, TypeKind::Error) => saw_error = true,
                (
                    TypeKind::InferenceVariable(left_variable),
                    TypeKind::InferenceVariable(right_variable),
                ) => {
                    self.union_variables(left_variable, right_variable, budget)?;
                }
                (TypeKind::InferenceVariable(variable), _) => {
                    self.bind_variable(variable, right, interner, budget)?;
                }
                (_, TypeKind::InferenceVariable(variable)) => {
                    self.bind_variable(variable, left, interner, budget)?;
                }
                (TypeKind::Tuple(left_types), TypeKind::Tuple(right_types)) => {
                    push_lists(
                        &mut pending,
                        interner.list(left_types),
                        interner.list(right_types),
                        budget,
                    )?;
                }
                (TypeKind::List(left), TypeKind::List(right))
                | (TypeKind::Slice(left), TypeKind::Slice(right))
                | (TypeKind::Task(left), TypeKind::Task(right)) => pending.push((left, right)),
                (
                    TypeKind::Map {
                        key: left_key,
                        value: left_value,
                    },
                    TypeKind::Map {
                        key: right_key,
                        value: right_value,
                    },
                ) => {
                    pending.push((left_value, right_value));
                    pending.push((left_key, right_key));
                }
                (
                    TypeKind::Function {
                        parameters: left_parameters,
                        result: left_result,
                    },
                    TypeKind::Function {
                        parameters: right_parameters,
                        result: right_result,
                    },
                ) => {
                    pending.push((left_result, right_result));
                    push_lists(
                        &mut pending,
                        interner.list(left_parameters),
                        interner.list(right_parameters),
                        budget,
                    )?;
                }
                (
                    TypeKind::Nominal {
                        definition: left_definition,
                        arguments: left_arguments,
                    },
                    TypeKind::Nominal {
                        definition: right_definition,
                        arguments: right_arguments,
                    },
                ) if left_definition == right_definition => {
                    push_lists(
                        &mut pending,
                        interner.list(left_arguments),
                        interner.list(right_arguments),
                        budget,
                    )?;
                }
                (TypeKind::Primitive(left), TypeKind::Primitive(right))
                    if integer_primitives_are_compatible(left, right) =>
                {
                    // The low-level surface (`u8`/`u16`/`u32`/`i8`/`i16`/`i32`)
                    // is represented with i64 machine values today. Accepting
                    // Int literals/temporaries where a fixed-width integer is
                    // expected keeps exact-layout tests typeable while the MIR
                    // and backends still carry the canonical primitive type on
                    // the destination place.
                }
                _ => {
                    return Err(TypeError::Mismatch {
                        expected: left,
                        actual: right,
                    });
                }
            }
        }
        if saw_error {
            Ok(interner.error())
        } else {
            self.normalize(interner, expected, budget)
        }
    }

    fn find_root(
        &mut self,
        variable: InferVarId,
        budget: &mut TypeWorkBudget,
    ) -> Result<InferVarId, TypeError> {
        let mut root = variable;
        loop {
            budget.consume("find inference representative")?;
            let parent = self.variables[root.index()].parent;
            if parent == root {
                break;
            }
            root = parent;
        }

        let mut current = variable;
        while current != root {
            budget.consume("compress inference path")?;
            let parent = self.variables[current.index()].parent;
            self.variables[current.index()].parent = root;
            current = parent;
        }
        Ok(root)
    }

    fn union_variables(
        &mut self,
        left: InferVarId,
        right: InferVarId,
        budget: &mut TypeWorkBudget,
    ) -> Result<InferVarId, TypeError> {
        let left = self.find_root(left, budget)?;
        let right = self.find_root(right, budget)?;
        if left == right {
            return Ok(left);
        }
        let left_rank = self.variables[left.index()].rank;
        let right_rank = self.variables[right.index()].rank;
        let (root, child) = if left_rank > right_rank {
            (left, right)
        } else if right_rank > left_rank {
            (right, left)
        } else if left < right {
            (left, right)
        } else {
            (right, left)
        };
        self.variables[child.index()].parent = root;
        let child_level = self.variables[child.index()].level;
        if child_level < self.variables[root.index()].level {
            self.variables[root.index()].level = child_level;
        }
        if left_rank == right_rank {
            self.variables[root.index()].rank = self.variables[root.index()].rank.saturating_add(1);
        }
        Ok(root)
    }

    fn bind_variable(
        &mut self,
        variable: InferVarId,
        type_id: TypeId,
        interner: &TypeInterner,
        budget: &mut TypeWorkBudget,
    ) -> Result<(), TypeError> {
        let root = self.find_root(variable, budget)?;
        let level = self.variables[root.index()].level;
        let mut seen_types = HashSet::new();
        let mut seen_variables = HashSet::new();
        let mut pending = vec![type_id];
        while let Some(current) = pending.pop() {
            budget.consume("occurs check")?;
            let current = self.resolve(interner, current, budget)?;
            match interner.kind(current) {
                TypeKind::InferenceVariable(other) => {
                    let other = self.find_root(other, budget)?;
                    if other == root {
                        return Err(TypeError::OccursCheck {
                            variable: root,
                            within: type_id,
                        });
                    }
                    if seen_variables.insert(other) && self.variables[other.index()].level > level {
                        self.variables[other.index()].level = level;
                    }
                }
                kind if seen_types.insert(current) => {
                    push_children(&mut pending, interner, kind, budget)?;
                }
                _ => {}
            }
        }
        self.variables[root.index()].binding = Some(type_id);
        Ok(())
    }
}

fn integer_primitives_are_compatible(left: PrimitiveType, right: PrimitiveType) -> bool {
    fn is_int_like(value: PrimitiveType) -> bool {
        matches!(
            value,
            PrimitiveType::Int
                | PrimitiveType::U8
                | PrimitiveType::U16
                | PrimitiveType::U32
                | PrimitiveType::I8
                | PrimitiveType::I16
                | PrimitiveType::I32
        )
    }

    left == right || (is_int_like(left) && is_int_like(right))
}

fn push_lists(
    pending: &mut Vec<(TypeId, TypeId)>,
    left: &[TypeId],
    right: &[TypeId],
    budget: &mut TypeWorkBudget,
) -> Result<(), TypeError> {
    if left.len() != right.len() {
        return Err(TypeError::ArityMismatch {
            expected: left.len(),
            actual: right.len(),
        });
    }
    for (left, right) in left.iter().zip(right).rev() {
        budget.consume("queue unification edge")?;
        pending.push((*left, *right));
    }
    Ok(())
}

fn push_children(
    pending: &mut Vec<TypeId>,
    interner: &TypeInterner,
    kind: TypeKind,
    budget: &mut TypeWorkBudget,
) -> Result<(), TypeError> {
    let mut push = |child: TypeId| -> Result<(), TypeError> {
        budget.consume("queue type child")?;
        pending.push(child);
        Ok(())
    };
    match kind {
        TypeKind::Tuple(types) => {
            for child in interner.list(types).iter().rev() {
                push(*child)?;
            }
        }
        TypeKind::List(element) | TypeKind::Slice(element) | TypeKind::Task(element) => {
            push(element)?;
        }
        TypeKind::Map { key, value } => {
            push(value)?;
            push(key)?;
        }
        TypeKind::Function { parameters, result } => {
            push(result)?;
            for child in interner.list(parameters).iter().rev() {
                push(*child)?;
            }
        }
        TypeKind::Nominal { arguments, .. } => {
            for child in interner.list(arguments).iter().rev() {
                push(*child)?;
            }
        }
        _ => {}
    }
    Ok(())
}
