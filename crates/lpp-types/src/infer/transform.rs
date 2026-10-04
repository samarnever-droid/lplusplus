use std::collections::{BTreeMap, HashMap, HashSet};

use lpp_hir::OriginId;

use crate::{InferVarId, TypeId, TypeInterner, TypeKind};

use super::{
    GeneralizedVariable, InferenceLevel, InferenceTable, TypeError, TypeScheme, TypeWorkBudget,
    push_children,
};

impl InferenceTable {
    pub fn normalize(
        &mut self,
        interner: &mut TypeInterner,
        type_id: TypeId,
        budget: &mut TypeWorkBudget,
    ) -> Result<TypeId, TypeError> {
        let mut memo = HashMap::new();
        self.normalize_inner(interner, type_id, budget, &mut memo, 0)
    }

    pub fn generalize(
        &mut self,
        interner: &mut TypeInterner,
        type_id: TypeId,
        environment_level: InferenceLevel,
        budget: &mut TypeWorkBudget,
    ) -> Result<TypeScheme, TypeError> {
        let normalized = self.normalize(interner, type_id, budget)?;
        let mut roots = BTreeMap::<InferVarId, GeneralizedVariable>::new();
        let mut seen = HashSet::new();
        let mut pending = vec![normalized];
        while let Some(current) = pending.pop() {
            budget.consume("generalize")?;
            let current = self.resolve(interner, current, budget)?;
            if !seen.insert(current) {
                continue;
            }
            if let TypeKind::InferenceVariable(variable) = interner.kind(current) {
                let root = self.find_root(variable, budget)?;
                let metadata = self.variables[root.index()];
                if metadata.level > environment_level {
                    roots.entry(root).or_insert(GeneralizedVariable {
                        source: root,
                        level: metadata.level,
                        origin: metadata.origin,
                    });
                }
                continue;
            }
            push_children(&mut pending, interner, interner.kind(current), budget)?;
        }

        let replacements = roots
            .keys()
            .enumerate()
            .map(|(index, variable)| (*variable, index as u32))
            .collect::<BTreeMap<_, _>>();
        let body = self.replace_generalized(
            interner,
            normalized,
            &replacements,
            budget,
            &mut HashMap::new(),
            0,
        )?;
        Ok(TypeScheme {
            body,
            variables: roots.into_values().collect::<Vec<_>>().into(),
        })
    }

    pub fn instantiate(
        &mut self,
        interner: &mut TypeInterner,
        scheme: &TypeScheme,
        level: InferenceLevel,
        origin: Option<OriginId>,
        budget: &mut TypeWorkBudget,
    ) -> Result<TypeId, TypeError> {
        let mut replacements = Vec::with_capacity(scheme.variables.len());
        for variable in scheme.variables.iter() {
            budget.consume("instantiate")?;
            replacements.push(self.fresh(interner, level, origin.or(variable.origin))?);
        }
        self.replace_bound(
            interner,
            scheme.body,
            &replacements,
            budget,
            &mut HashMap::new(),
            0,
        )
    }

    fn normalize_inner(
        &mut self,
        interner: &mut TypeInterner,
        type_id: TypeId,
        budget: &mut TypeWorkBudget,
        memo: &mut HashMap<TypeId, TypeId>,
        depth: usize,
    ) -> Result<TypeId, TypeError> {
        budget.check_depth(depth, "normalize")?;
        budget.consume("normalize")?;
        let type_id = self.resolve(interner, type_id, budget)?;
        if let Some(result) = memo.get(&type_id) {
            return Ok(*result);
        }
        let result = match interner.kind(type_id) {
            TypeKind::Tuple(types) => {
                let types = interner.list(types).to_vec();
                let types = self.normalize_list(interner, &types, budget, memo, depth + 1)?;
                let types = interner.intern_list(&types)?;
                interner.intern(TypeKind::Tuple(types))?
            }
            TypeKind::List(element) => {
                let element = self.normalize_inner(interner, element, budget, memo, depth + 1)?;
                interner.intern(TypeKind::List(element))?
            }
            TypeKind::Map { key, value } => {
                let key = self.normalize_inner(interner, key, budget, memo, depth + 1)?;
                let value = self.normalize_inner(interner, value, budget, memo, depth + 1)?;
                interner.intern(TypeKind::Map { key, value })?
            }
            TypeKind::Slice(element) => {
                let element = self.normalize_inner(interner, element, budget, memo, depth + 1)?;
                interner.intern(TypeKind::Slice(element))?
            }
            TypeKind::Task(output) => {
                let output = self.normalize_inner(interner, output, budget, memo, depth + 1)?;
                interner.intern(TypeKind::Task(output))?
            }
            TypeKind::Function { parameters, result } => {
                let parameters = interner.list(parameters).to_vec();
                let parameters =
                    self.normalize_list(interner, &parameters, budget, memo, depth + 1)?;
                let parameters = interner.intern_list(&parameters)?;
                let result = self.normalize_inner(interner, result, budget, memo, depth + 1)?;
                interner.intern(TypeKind::Function { parameters, result })?
            }
            TypeKind::Nominal {
                definition,
                arguments,
            } => {
                let arguments = interner.list(arguments).to_vec();
                let arguments =
                    self.normalize_list(interner, &arguments, budget, memo, depth + 1)?;
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

    fn normalize_list(
        &mut self,
        interner: &mut TypeInterner,
        types: &[TypeId],
        budget: &mut TypeWorkBudget,
        memo: &mut HashMap<TypeId, TypeId>,
        depth: usize,
    ) -> Result<Vec<TypeId>, TypeError> {
        types
            .iter()
            .map(|type_id| self.normalize_inner(interner, *type_id, budget, memo, depth))
            .collect()
    }

    fn replace_generalized(
        &mut self,
        interner: &mut TypeInterner,
        type_id: TypeId,
        replacements: &BTreeMap<InferVarId, u32>,
        budget: &mut TypeWorkBudget,
        memo: &mut HashMap<TypeId, TypeId>,
        depth: usize,
    ) -> Result<TypeId, TypeError> {
        budget.check_depth(depth, "replace generalized variables")?;
        budget.consume("replace generalized variables")?;
        let type_id = self.resolve(interner, type_id, budget)?;
        if let Some(result) = memo.get(&type_id) {
            return Ok(*result);
        }
        if let TypeKind::InferenceVariable(variable) = interner.kind(type_id) {
            let root = self.find_root(variable, budget)?;
            let result = if let Some(index) = replacements.get(&root) {
                interner.intern(TypeKind::BoundVariable(*index))?
            } else {
                self.variables[root.index()].type_id
            };
            memo.insert(type_id, result);
            return Ok(result);
        }
        self.rebuild(
            interner,
            type_id,
            budget,
            memo,
            depth,
            |table, interner, child, budget, memo, depth| {
                table.replace_generalized(interner, child, replacements, budget, memo, depth)
            },
        )
    }

    fn replace_bound(
        &mut self,
        interner: &mut TypeInterner,
        type_id: TypeId,
        replacements: &[TypeId],
        budget: &mut TypeWorkBudget,
        memo: &mut HashMap<TypeId, TypeId>,
        depth: usize,
    ) -> Result<TypeId, TypeError> {
        budget.check_depth(depth, "replace bound variables")?;
        budget.consume("replace bound variables")?;
        if let Some(result) = memo.get(&type_id) {
            return Ok(*result);
        }
        if let TypeKind::BoundVariable(index) = interner.kind(type_id) {
            let result = replacements.get(index as usize).copied().ok_or(
                TypeError::InvalidBoundVariable {
                    index,
                    available: replacements.len(),
                },
            )?;
            memo.insert(type_id, result);
            return Ok(result);
        }
        self.rebuild(
            interner,
            type_id,
            budget,
            memo,
            depth,
            |table, interner, child, budget, memo, depth| {
                table.replace_bound(interner, child, replacements, budget, memo, depth)
            },
        )
    }

    fn rebuild<F>(
        &mut self,
        interner: &mut TypeInterner,
        type_id: TypeId,
        budget: &mut TypeWorkBudget,
        memo: &mut HashMap<TypeId, TypeId>,
        depth: usize,
        mut replace: F,
    ) -> Result<TypeId, TypeError>
    where
        F: FnMut(
            &mut Self,
            &mut TypeInterner,
            TypeId,
            &mut TypeWorkBudget,
            &mut HashMap<TypeId, TypeId>,
            usize,
        ) -> Result<TypeId, TypeError>,
    {
        let result = match interner.kind(type_id) {
            TypeKind::Tuple(types) => {
                let source = interner.list(types).to_vec();
                let mut output = Vec::with_capacity(source.len());
                for child in source {
                    output.push(replace(self, interner, child, budget, memo, depth + 1)?);
                }
                let output = interner.intern_list(&output)?;
                interner.intern(TypeKind::Tuple(output))?
            }
            TypeKind::List(element) => {
                let element = replace(self, interner, element, budget, memo, depth + 1)?;
                interner.intern(TypeKind::List(element))?
            }
            TypeKind::Map { key, value } => {
                let key = replace(self, interner, key, budget, memo, depth + 1)?;
                let value = replace(self, interner, value, budget, memo, depth + 1)?;
                interner.intern(TypeKind::Map { key, value })?
            }
            TypeKind::Slice(element) => {
                let element = replace(self, interner, element, budget, memo, depth + 1)?;
                interner.intern(TypeKind::Slice(element))?
            }
            TypeKind::Task(output) => {
                let output = replace(self, interner, output, budget, memo, depth + 1)?;
                interner.intern(TypeKind::Task(output))?
            }
            TypeKind::Function { parameters, result } => {
                let source = interner.list(parameters).to_vec();
                let mut output = Vec::with_capacity(source.len());
                for child in source {
                    output.push(replace(self, interner, child, budget, memo, depth + 1)?);
                }
                let parameters = interner.intern_list(&output)?;
                let result = replace(self, interner, result, budget, memo, depth + 1)?;
                interner.intern(TypeKind::Function { parameters, result })?
            }
            TypeKind::Nominal {
                definition,
                arguments,
            } => {
                let source = interner.list(arguments).to_vec();
                let mut output = Vec::with_capacity(source.len());
                for child in source {
                    output.push(replace(self, interner, child, budget, memo, depth + 1)?);
                }
                let arguments = interner.intern_list(&output)?;
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
}
