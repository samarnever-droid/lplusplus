use std::collections::{BTreeMap, BTreeSet, HashMap};

use lpp_hir::TypeParamId;

use crate::{TypeId, TypeInterner, TypeKind, TypeListId, TypeSubstitution};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PatternLimit {
    Work,
    Depth,
}

pub(crate) struct PatternBudget {
    initial: usize,
    remaining: usize,
    max_depth: usize,
}

impl PatternBudget {
    pub(crate) const fn new(work: usize, max_depth: usize) -> Self {
        Self {
            initial: work,
            remaining: work,
            max_depth,
        }
    }

    pub(crate) const fn consumed(&self) -> usize {
        self.initial - self.remaining
    }

    fn enter(&mut self, depth: usize) -> Result<(), PatternLimit> {
        if depth > self.max_depth {
            return Err(PatternLimit::Depth);
        }
        self.remaining = self.remaining.checked_sub(1).ok_or(PatternLimit::Work)?;
        Ok(())
    }
}

pub(crate) fn match_type_pattern(
    interner: &TypeInterner,
    pattern: TypeId,
    actual: TypeId,
    substitution: &mut TypeSubstitution,
    budget: &mut PatternBudget,
) -> Result<bool, PatternLimit> {
    match_inner(interner, pattern, actual, substitution, budget, 0)
}

fn match_inner(
    interner: &TypeInterner,
    pattern: TypeId,
    actual: TypeId,
    substitution: &mut TypeSubstitution,
    budget: &mut PatternBudget,
    depth: usize,
) -> Result<bool, PatternLimit> {
    budget.enter(depth)?;
    if pattern == actual {
        return Ok(true);
    }
    if let TypeKind::GenericParameter(parameter) = interner.kind(pattern) {
        return Ok(match substitution.get(parameter) {
            Some(bound) => bound == actual,
            None => {
                substitution.insert(parameter, actual);
                true
            }
        });
    }
    match (interner.kind(pattern), interner.kind(actual)) {
        (TypeKind::Tuple(left), TypeKind::Tuple(right)) => {
            match_lists(interner, left, right, substitution, budget, depth + 1)
        }
        (TypeKind::List(left), TypeKind::List(right))
        | (TypeKind::Slice(left), TypeKind::Slice(right))
        | (TypeKind::Task(left), TypeKind::Task(right)) => {
            match_inner(interner, left, right, substitution, budget, depth + 1)
        }
        (
            TypeKind::Map {
                key: left_key,
                value: left_value,
            },
            TypeKind::Map {
                key: right_key,
                value: right_value,
            },
        ) => Ok(match_inner(
            interner,
            left_key,
            right_key,
            substitution,
            budget,
            depth + 1,
        )? && match_inner(
            interner,
            left_value,
            right_value,
            substitution,
            budget,
            depth + 1,
        )?),
        (
            TypeKind::Function {
                parameters: left_parameters,
                result: left_result,
            },
            TypeKind::Function {
                parameters: right_parameters,
                result: right_result,
            },
        ) => Ok(match_lists(
            interner,
            left_parameters,
            right_parameters,
            substitution,
            budget,
            depth + 1,
        )? && match_inner(
            interner,
            left_result,
            right_result,
            substitution,
            budget,
            depth + 1,
        )?),
        (
            TypeKind::Nominal {
                definition: left_definition,
                arguments: left_arguments,
            },
            TypeKind::Nominal {
                definition: right_definition,
                arguments: right_arguments,
            },
        ) if left_definition == right_definition => match_lists(
            interner,
            left_arguments,
            right_arguments,
            substitution,
            budget,
            depth + 1,
        ),
        _ => Ok(false),
    }
}

pub(crate) fn match_lists(
    interner: &TypeInterner,
    patterns: TypeListId,
    actuals: TypeListId,
    substitution: &mut TypeSubstitution,
    budget: &mut PatternBudget,
    depth: usize,
) -> Result<bool, PatternLimit> {
    budget.enter(depth)?;
    let patterns = interner.list(patterns);
    let actuals = interner.list(actuals);
    if patterns.len() != actuals.len() {
        return Ok(false);
    }
    for (pattern, actual) in patterns.iter().zip(actuals) {
        if !match_inner(interner, *pattern, *actual, substitution, budget, depth + 1)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) fn patterns_overlap(
    interner: &TypeInterner,
    left: TypeId,
    right: TypeId,
    budget: &mut PatternBudget,
) -> Result<bool, PatternLimit> {
    overlap_inner(interner, left, right, &mut BTreeMap::new(), budget, 0)
}

pub(crate) fn pattern_lists_overlap(
    interner: &TypeInterner,
    left: TypeListId,
    right: TypeListId,
    budget: &mut PatternBudget,
) -> Result<bool, PatternLimit> {
    let left = interner.list(left);
    let right = interner.list(right);
    if left.len() != right.len() {
        return Ok(false);
    }
    let mut bindings = BTreeMap::new();
    for (left, right) in left.iter().zip(right) {
        if !overlap_inner(interner, *left, *right, &mut bindings, budget, 0)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn overlap_inner(
    interner: &TypeInterner,
    left: TypeId,
    right: TypeId,
    bindings: &mut BTreeMap<TypeParamId, TypeId>,
    budget: &mut PatternBudget,
    depth: usize,
) -> Result<bool, PatternLimit> {
    budget.enter(depth)?;
    let left = resolve_pattern_binding(interner, left, bindings);
    let right = resolve_pattern_binding(interner, right, bindings);
    if left == right {
        return Ok(true);
    }
    if let TypeKind::GenericParameter(left_parameter) = interner.kind(left) {
        return bind_overlap_variable(
            interner,
            left,
            left_parameter,
            right,
            bindings,
            budget,
            depth,
        );
    }
    if let TypeKind::GenericParameter(right_parameter) = interner.kind(right) {
        return bind_overlap_variable(
            interner,
            right,
            right_parameter,
            left,
            bindings,
            budget,
            depth,
        );
    }
    match (interner.kind(left), interner.kind(right)) {
        (TypeKind::Tuple(left), TypeKind::Tuple(right)) => {
            overlap_lists(interner, left, right, bindings, budget, depth + 1)
        }
        (TypeKind::List(left), TypeKind::List(right))
        | (TypeKind::Slice(left), TypeKind::Slice(right))
        | (TypeKind::Task(left), TypeKind::Task(right)) => {
            overlap_inner(interner, left, right, bindings, budget, depth + 1)
        }
        (
            TypeKind::Map {
                key: left_key,
                value: left_value,
            },
            TypeKind::Map {
                key: right_key,
                value: right_value,
            },
        ) => Ok(
            overlap_inner(interner, left_key, right_key, bindings, budget, depth + 1)?
                && overlap_inner(
                    interner,
                    left_value,
                    right_value,
                    bindings,
                    budget,
                    depth + 1,
                )?,
        ),
        (
            TypeKind::Function {
                parameters: left_parameters,
                result: left_result,
            },
            TypeKind::Function {
                parameters: right_parameters,
                result: right_result,
            },
        ) => Ok(overlap_lists(
            interner,
            left_parameters,
            right_parameters,
            bindings,
            budget,
            depth + 1,
        )? && overlap_inner(
            interner,
            left_result,
            right_result,
            bindings,
            budget,
            depth + 1,
        )?),
        (
            TypeKind::Nominal {
                definition: left_definition,
                arguments: left_arguments,
            },
            TypeKind::Nominal {
                definition: right_definition,
                arguments: right_arguments,
            },
        ) if left_definition == right_definition => overlap_lists(
            interner,
            left_arguments,
            right_arguments,
            bindings,
            budget,
            depth + 1,
        ),
        _ => Ok(false),
    }
}

fn overlap_lists(
    interner: &TypeInterner,
    left: TypeListId,
    right: TypeListId,
    bindings: &mut BTreeMap<TypeParamId, TypeId>,
    budget: &mut PatternBudget,
    depth: usize,
) -> Result<bool, PatternLimit> {
    budget.enter(depth)?;
    let left = interner.list(left);
    let right = interner.list(right);
    if left.len() != right.len() {
        return Ok(false);
    }
    for (left, right) in left.iter().zip(right) {
        if !overlap_inner(interner, *left, *right, bindings, budget, depth + 1)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn bind_overlap_variable(
    interner: &TypeInterner,
    variable: TypeId,
    parameter: TypeParamId,
    value: TypeId,
    bindings: &mut BTreeMap<TypeParamId, TypeId>,
    budget: &mut PatternBudget,
    depth: usize,
) -> Result<bool, PatternLimit> {
    if let TypeKind::GenericParameter(other) = interner.kind(value) {
        let (bound, target) = if parameter < other {
            (other, variable)
        } else {
            (parameter, value)
        };
        bindings.insert(bound, target);
        return Ok(true);
    }
    if contains_parameter(interner, value, parameter, bindings, budget, depth + 1)? {
        return Ok(false);
    }
    bindings.insert(parameter, value);
    Ok(true)
}

fn contains_parameter(
    interner: &TypeInterner,
    type_id: TypeId,
    needle: TypeParamId,
    bindings: &BTreeMap<TypeParamId, TypeId>,
    budget: &mut PatternBudget,
    depth: usize,
) -> Result<bool, PatternLimit> {
    budget.enter(depth)?;
    let mut pending = vec![type_id];
    let mut visited = BTreeSet::new();
    while let Some(type_id) = pending.pop() {
        budget.enter(depth)?;
        let type_id = resolve_pattern_binding(interner, type_id, bindings);
        if !visited.insert(type_id) {
            continue;
        }
        match interner.kind(type_id) {
            TypeKind::GenericParameter(parameter) if parameter == needle => return Ok(true),
            TypeKind::Tuple(types) => pending.extend(interner.list(types).iter().rev().copied()),
            TypeKind::List(element) | TypeKind::Slice(element) | TypeKind::Task(element) => {
                pending.push(element);
            }
            TypeKind::Map { key, value } => {
                pending.push(value);
                pending.push(key);
            }
            TypeKind::Function { parameters, result } => {
                pending.push(result);
                pending.extend(interner.list(parameters).iter().rev().copied());
            }
            TypeKind::Nominal { arguments, .. } => {
                pending.extend(interner.list(arguments).iter().rev().copied());
            }
            _ => {}
        }
    }
    Ok(false)
}

fn resolve_pattern_binding(
    interner: &TypeInterner,
    mut type_id: TypeId,
    bindings: &BTreeMap<TypeParamId, TypeId>,
) -> TypeId {
    for _ in 0..=bindings.len() {
        let TypeKind::GenericParameter(parameter) = interner.kind(type_id) else {
            return type_id;
        };
        let Some(next) = bindings.get(&parameter) else {
            return type_id;
        };
        if *next == type_id {
            return type_id;
        }
        type_id = *next;
    }
    type_id
}

pub(crate) fn substitute_pattern_list(
    interner: &mut TypeInterner,
    list: TypeListId,
    substitution: &TypeSubstitution,
    budget: &mut PatternBudget,
) -> Result<TypeListId, PatternLimit> {
    let source = interner.list(list).to_vec();
    let mut output = Vec::with_capacity(source.len());
    let mut memo = HashMap::new();
    for type_id in source {
        output.push(substitute_inner(
            interner,
            type_id,
            substitution,
            budget,
            &mut memo,
            0,
        )?);
    }
    interner
        .intern_list(&output)
        .map_err(|_| PatternLimit::Work)
}

fn substitute_inner(
    interner: &mut TypeInterner,
    type_id: TypeId,
    substitution: &TypeSubstitution,
    budget: &mut PatternBudget,
    memo: &mut HashMap<TypeId, TypeId>,
    depth: usize,
) -> Result<TypeId, PatternLimit> {
    budget.enter(depth)?;
    if let Some(output) = memo.get(&type_id) {
        return Ok(*output);
    }
    if let TypeKind::GenericParameter(parameter) = interner.kind(type_id) {
        let output = substitution.get(parameter).unwrap_or(type_id);
        memo.insert(type_id, output);
        return Ok(output);
    }
    let output = match interner.kind(type_id) {
        TypeKind::Tuple(types) => {
            let types =
                substitute_list_inner(interner, types, substitution, budget, memo, depth + 1)?;
            interner
                .intern(TypeKind::Tuple(types))
                .map_err(|_| PatternLimit::Work)?
        }
        TypeKind::List(element) => {
            let element =
                substitute_inner(interner, element, substitution, budget, memo, depth + 1)?;
            interner
                .intern(TypeKind::List(element))
                .map_err(|_| PatternLimit::Work)?
        }
        TypeKind::Map { key, value } => {
            let key = substitute_inner(interner, key, substitution, budget, memo, depth + 1)?;
            let value = substitute_inner(interner, value, substitution, budget, memo, depth + 1)?;
            interner
                .intern(TypeKind::Map { key, value })
                .map_err(|_| PatternLimit::Work)?
        }
        TypeKind::Slice(element) => {
            let element =
                substitute_inner(interner, element, substitution, budget, memo, depth + 1)?;
            interner
                .intern(TypeKind::Slice(element))
                .map_err(|_| PatternLimit::Work)?
        }
        TypeKind::Task(output) => {
            let output = substitute_inner(interner, output, substitution, budget, memo, depth + 1)?;
            interner
                .intern(TypeKind::Task(output))
                .map_err(|_| PatternLimit::Work)?
        }
        TypeKind::Function { parameters, result } => {
            let parameters =
                substitute_list_inner(interner, parameters, substitution, budget, memo, depth + 1)?;
            let result = substitute_inner(interner, result, substitution, budget, memo, depth + 1)?;
            interner
                .intern(TypeKind::Function { parameters, result })
                .map_err(|_| PatternLimit::Work)?
        }
        TypeKind::Nominal {
            definition,
            arguments,
        } => {
            let arguments =
                substitute_list_inner(interner, arguments, substitution, budget, memo, depth + 1)?;
            interner
                .intern(TypeKind::Nominal {
                    definition,
                    arguments,
                })
                .map_err(|_| PatternLimit::Work)?
        }
        _ => type_id,
    };
    memo.insert(type_id, output);
    Ok(output)
}

fn substitute_list_inner(
    interner: &mut TypeInterner,
    list: TypeListId,
    substitution: &TypeSubstitution,
    budget: &mut PatternBudget,
    memo: &mut HashMap<TypeId, TypeId>,
    depth: usize,
) -> Result<TypeListId, PatternLimit> {
    budget.enter(depth)?;
    let source = interner.list(list).to_vec();
    let mut output = Vec::with_capacity(source.len());
    for type_id in source {
        output.push(substitute_inner(
            interner,
            type_id,
            substitution,
            budget,
            memo,
            depth + 1,
        )?);
    }
    interner
        .intern_list(&output)
        .map_err(|_| PatternLimit::Work)
}

pub(crate) fn pattern_specificity(
    interner: &TypeInterner,
    type_id: TypeId,
    budget: &mut PatternBudget,
) -> Result<u32, PatternLimit> {
    specificity_inner(interner, type_id, budget, 0)
}

pub(crate) fn list_specificity(
    interner: &TypeInterner,
    list: TypeListId,
    budget: &mut PatternBudget,
) -> Result<u32, PatternLimit> {
    let mut specificity = 0_u32;
    for type_id in interner.list(list) {
        specificity = specificity.saturating_add(specificity_inner(interner, *type_id, budget, 0)?);
    }
    Ok(specificity)
}

fn specificity_inner(
    interner: &TypeInterner,
    type_id: TypeId,
    budget: &mut PatternBudget,
    depth: usize,
) -> Result<u32, PatternLimit> {
    budget.enter(depth)?;
    let children = match interner.kind(type_id) {
        TypeKind::GenericParameter(_) => return Ok(0),
        TypeKind::Tuple(types) => interner.list(types).to_vec(),
        TypeKind::List(element) | TypeKind::Slice(element) | TypeKind::Task(element) => {
            vec![element]
        }
        TypeKind::Map { key, value } => vec![key, value],
        TypeKind::Function { parameters, result } => {
            let mut children = interner.list(parameters).to_vec();
            children.push(result);
            children
        }
        TypeKind::Nominal { arguments, .. } => interner.list(arguments).to_vec(),
        _ => Vec::new(),
    };
    let mut specificity = 1_u32;
    for child in children {
        specificity =
            specificity.saturating_add(specificity_inner(interner, child, budget, depth + 1)?);
    }
    Ok(specificity)
}

pub(crate) fn is_concrete_type(
    interner: &TypeInterner,
    type_id: TypeId,
    budget: &mut PatternBudget,
) -> Result<bool, PatternLimit> {
    budget.enter(0)?;
    let mut pending = vec![type_id];
    let mut visited = BTreeSet::new();
    while let Some(type_id) = pending.pop() {
        budget.enter(0)?;
        if !visited.insert(type_id) {
            continue;
        }
        match interner.kind(type_id) {
            TypeKind::GenericParameter(_) | TypeKind::InferenceVariable(_) => return Ok(false),
            TypeKind::Tuple(types) => pending.extend(interner.list(types).iter().rev().copied()),
            TypeKind::List(element) | TypeKind::Slice(element) | TypeKind::Task(element) => {
                pending.push(element);
            }
            TypeKind::Map { key, value } => {
                pending.push(value);
                pending.push(key);
            }
            TypeKind::Function { parameters, result } => {
                pending.push(result);
                pending.extend(interner.list(parameters).iter().rev().copied());
            }
            TypeKind::Nominal { arguments, .. } => {
                pending.extend(interner.list(arguments).iter().rev().copied());
            }
            _ => {}
        }
    }
    Ok(true)
}
