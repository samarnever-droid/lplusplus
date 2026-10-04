use std::collections::{BTreeMap, HashMap};

use lpp_hir::TypeParamId;

use crate::{TypeError, TypeId, TypeInterner, TypeKind, TypeWorkBudget};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypeSubstitution {
    bindings: BTreeMap<TypeParamId, TypeId>,
}

impl TypeSubstitution {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, parameter: TypeParamId, type_id: TypeId) -> Option<TypeId> {
        self.bindings.insert(parameter, type_id)
    }

    #[must_use]
    pub fn get(&self, parameter: TypeParamId) -> Option<TypeId> {
        self.bindings.get(&parameter).copied()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = (TypeParamId, TypeId)> + '_ {
        self.bindings
            .iter()
            .map(|(parameter, type_id)| (*parameter, *type_id))
    }

    pub fn apply(
        &self,
        interner: &mut TypeInterner,
        type_id: TypeId,
        budget: &mut TypeWorkBudget,
    ) -> Result<TypeId, TypeError> {
        self.apply_inner(interner, type_id, budget, &mut HashMap::new(), 0)
    }

    fn apply_inner(
        &self,
        interner: &mut TypeInterner,
        type_id: TypeId,
        budget: &mut TypeWorkBudget,
        memo: &mut HashMap<TypeId, TypeId>,
        depth: usize,
    ) -> Result<TypeId, TypeError> {
        budget.check_depth(depth, "apply type substitution")?;
        budget.consume("apply type substitution")?;
        if let Some(result) = memo.get(&type_id) {
            return Ok(*result);
        }
        if let TypeKind::GenericParameter(parameter) = interner.kind(type_id) {
            let result = self.get(parameter).unwrap_or(type_id);
            memo.insert(type_id, result);
            return Ok(result);
        }
        let result = match interner.kind(type_id) {
            TypeKind::Tuple(types) => {
                let source = interner.list(types).to_vec();
                let output = self.apply_list(interner, &source, budget, memo, depth + 1)?;
                let output = interner.intern_list(&output)?;
                interner.intern(TypeKind::Tuple(output))?
            }
            TypeKind::List(element) => {
                let element = self.apply_inner(interner, element, budget, memo, depth + 1)?;
                interner.intern(TypeKind::List(element))?
            }
            TypeKind::Map { key, value } => {
                let key = self.apply_inner(interner, key, budget, memo, depth + 1)?;
                let value = self.apply_inner(interner, value, budget, memo, depth + 1)?;
                interner.intern(TypeKind::Map { key, value })?
            }
            TypeKind::Slice(element) => {
                let element = self.apply_inner(interner, element, budget, memo, depth + 1)?;
                interner.intern(TypeKind::Slice(element))?
            }
            TypeKind::Task(output) => {
                let output = self.apply_inner(interner, output, budget, memo, depth + 1)?;
                interner.intern(TypeKind::Task(output))?
            }
            TypeKind::Function { parameters, result } => {
                let source = interner.list(parameters).to_vec();
                let parameters = self.apply_list(interner, &source, budget, memo, depth + 1)?;
                let parameters = interner.intern_list(&parameters)?;
                let result = self.apply_inner(interner, result, budget, memo, depth + 1)?;
                interner.intern(TypeKind::Function { parameters, result })?
            }
            TypeKind::Nominal {
                definition,
                arguments,
            } => {
                let source = interner.list(arguments).to_vec();
                let arguments = self.apply_list(interner, &source, budget, memo, depth + 1)?;
                let arguments = interner.intern_list(&arguments)?;
                interner.intern(TypeKind::Nominal {
                    definition,
                    arguments,
                })?
            }
            _ => type_id,
        };
        memo.insert(type_id, result);
        Ok(result)
    }

    fn apply_list(
        &self,
        interner: &mut TypeInterner,
        types: &[TypeId],
        budget: &mut TypeWorkBudget,
        memo: &mut HashMap<TypeId, TypeId>,
        depth: usize,
    ) -> Result<Vec<TypeId>, TypeError> {
        types
            .iter()
            .map(|type_id| self.apply_inner(interner, *type_id, budget, memo, depth))
            .collect()
    }
}
