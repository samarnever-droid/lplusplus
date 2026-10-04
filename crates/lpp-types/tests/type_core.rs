use std::mem::size_of;

use lpp_hir::{DefId, TypeParamId};
use lpp_types::{
    InferVarId, InferenceLevel, InferenceTable, PrimitiveType, TypeError, TypeId, TypeInterner,
    TypeKind, TypeListId, TypeSubstitution, TypeWorkBudget,
};

fn budget() -> TypeWorkBudget {
    TypeWorkBudget::new(10_000)
}

fn function(interner: &mut TypeInterner, parameters: &[TypeId], result: TypeId) -> TypeId {
    let parameters = interner.intern_list(parameters).unwrap();
    interner
        .intern(TypeKind::Function { parameters, result })
        .unwrap()
}

#[test]
fn compact_ids_and_kinds_have_fixed_structural_baselines() {
    assert_eq!(size_of::<TypeId>(), 4);
    assert_eq!(size_of::<Option<TypeId>>(), 4);
    assert_eq!(size_of::<TypeListId>(), 4);
    assert_eq!(size_of::<Option<TypeListId>>(), 4);
    assert_eq!(size_of::<InferVarId>(), 4);
    assert_eq!(size_of::<Option<InferVarId>>(), 4);
    assert_eq!(size_of::<TypeKind>(), 12);
    assert_eq!(size_of::<lpp_types::InferenceVariable>(), 24);
}

#[test]
fn canonicalizes_primitive_container_tuple_function_and_nominal_shapes() {
    let mut types = TypeInterner::new();
    let int = types.primitive(PrimitiveType::Int);
    let string = types.primitive(PrimitiveType::String);
    assert_eq!(int, types.primitive(PrimitiveType::Int));

    let pair_a = types.intern_list(&[int, string]).unwrap();
    let pair_b = types.intern_list(&[int, string]).unwrap();
    assert_eq!(pair_a, pair_b);

    let tuple_a = types.intern(TypeKind::Tuple(pair_a)).unwrap();
    let tuple_b = types.intern(TypeKind::Tuple(pair_b)).unwrap();
    assert_eq!(tuple_a, tuple_b);

    let list_a = types.intern(TypeKind::List(tuple_a)).unwrap();
    let list_b = types.intern(TypeKind::List(tuple_b)).unwrap();
    assert_eq!(list_a, list_b);

    let map_a = types
        .intern(TypeKind::Map {
            key: string,
            value: list_a,
        })
        .unwrap();
    let map_b = types
        .intern(TypeKind::Map {
            key: string,
            value: list_b,
        })
        .unwrap();
    assert_eq!(map_a, map_b);

    let slice_a = types.intern(TypeKind::Slice(tuple_a)).unwrap();
    let slice_b = types.intern(TypeKind::Slice(tuple_b)).unwrap();
    assert_eq!(slice_a, slice_b);
    let task_a = types.intern(TypeKind::Task(map_a)).unwrap();
    let task_b = types.intern(TypeKind::Task(map_b)).unwrap();
    assert_eq!(task_a, task_b);

    let function_a = function(&mut types, &[task_a, slice_a], list_a);
    let function_b = function(&mut types, &[task_b, slice_b], list_b);
    assert_eq!(function_a, function_b);

    let arguments = types.intern_list(&[int]).unwrap();
    let nominal_a = types
        .intern(TypeKind::Nominal {
            definition: DefId::from_raw(7),
            arguments,
        })
        .unwrap();
    let nominal_b = types
        .intern(TypeKind::Nominal {
            definition: DefId::from_raw(7),
            arguments,
        })
        .unwrap();
    assert_eq!(nominal_a, nominal_b);
}

#[test]
fn structurally_unifies_functions_containers_tuples_and_nominals() {
    let mut types = TypeInterner::new();
    let mut inference = InferenceTable::new();
    let int = types.primitive(PrimitiveType::Int);
    let string = types.primitive(PrimitiveType::String);
    let variable = inference
        .fresh(&mut types, InferenceLevel::new(1), None)
        .unwrap();

    let left_tuple_list = types.intern_list(&[variable, string]).unwrap();
    let left_tuple = types.intern(TypeKind::Tuple(left_tuple_list)).unwrap();
    let left_list = types.intern(TypeKind::List(left_tuple)).unwrap();
    let nominal_arguments = types.intern_list(&[left_list]).unwrap();
    let left_nominal = types
        .intern(TypeKind::Nominal {
            definition: DefId::from_raw(2),
            arguments: nominal_arguments,
        })
        .unwrap();
    let left = function(&mut types, &[left_nominal], variable);

    let right_tuple_list = types.intern_list(&[int, string]).unwrap();
    let right_tuple = types.intern(TypeKind::Tuple(right_tuple_list)).unwrap();
    let right_list = types.intern(TypeKind::List(right_tuple)).unwrap();
    let nominal_arguments = types.intern_list(&[right_list]).unwrap();
    let right_nominal = types
        .intern(TypeKind::Nominal {
            definition: DefId::from_raw(2),
            arguments: nominal_arguments,
        })
        .unwrap();
    let right = function(&mut types, &[right_nominal], int);

    let normalized = inference
        .unify(&mut types, left, right, &mut budget())
        .unwrap();
    assert_eq!(normalized, right);
}

#[test]
fn unifies_map_slice_and_task_children() {
    let mut types = TypeInterner::new();
    let mut inference = InferenceTable::new();
    let int = types.primitive(PrimitiveType::Int);
    let string = types.primitive(PrimitiveType::String);
    let key = inference
        .fresh(&mut types, InferenceLevel::new(1), None)
        .unwrap();
    let value = inference
        .fresh(&mut types, InferenceLevel::new(1), None)
        .unwrap();
    let inferred_map = types.intern(TypeKind::Map { key, value }).unwrap();
    let inferred_slice = types.intern(TypeKind::Slice(inferred_map)).unwrap();
    let inferred_task = types.intern(TypeKind::Task(inferred_slice)).unwrap();

    let concrete_map = types
        .intern(TypeKind::Map {
            key: string,
            value: int,
        })
        .unwrap();
    let concrete_slice = types.intern(TypeKind::Slice(concrete_map)).unwrap();
    let concrete_task = types.intern(TypeKind::Task(concrete_slice)).unwrap();
    assert_eq!(
        inference
            .unify(&mut types, inferred_task, concrete_task, &mut budget())
            .unwrap(),
        concrete_task
    );
}

#[test]
fn error_type_absorbs_without_binding_unrelated_inference_state() {
    let mut types = TypeInterner::new();
    let mut inference = InferenceTable::new();
    let variable = inference
        .fresh(&mut types, InferenceLevel::new(1), None)
        .unwrap();
    let error = types.error();
    assert_eq!(
        inference
            .unify(&mut types, variable, error, &mut budget())
            .unwrap(),
        error
    );
    assert_eq!(
        inference.resolve(&types, variable, &mut budget()).unwrap(),
        variable
    );
}

#[test]
fn reports_stable_arity_nominal_and_primitive_mismatches() {
    let mut types = TypeInterner::new();
    let mut inference = InferenceTable::new();
    let int = types.primitive(PrimitiveType::Int);
    let string = types.primitive(PrimitiveType::String);

    assert!(matches!(
        inference.unify(&mut types, int, string, &mut budget()),
        Err(TypeError::Mismatch { expected, actual }) if expected == int && actual == string
    ));

    let unary = function(&mut types, &[int], int);
    let binary = function(&mut types, &[int, int], int);
    assert_eq!(
        inference.unify(&mut types, unary, binary, &mut budget()),
        Err(TypeError::ArityMismatch {
            expected: 1,
            actual: 2,
        })
    );

    let empty = types.empty_list();
    let left = types
        .intern(TypeKind::Nominal {
            definition: DefId::from_raw(1),
            arguments: empty,
        })
        .unwrap();
    let right = types
        .intern(TypeKind::Nominal {
            definition: DefId::from_raw(2),
            arguments: empty,
        })
        .unwrap();
    assert!(matches!(
        inference.unify(&mut types, left, right, &mut budget()),
        Err(TypeError::Mismatch { .. })
    ));
}

#[test]
fn rejects_direct_and_nested_infinite_types_with_a_bounded_occurs_check() {
    let mut types = TypeInterner::new();
    let mut inference = InferenceTable::new();
    let variable = inference
        .fresh(&mut types, InferenceLevel::new(1), None)
        .unwrap();
    let list = types.intern(TypeKind::List(variable)).unwrap();
    let error = inference
        .unify(&mut types, variable, list, &mut budget())
        .unwrap_err();
    assert!(matches!(error, TypeError::OccursCheck { .. }));

    let variable = inference
        .fresh(&mut types, InferenceLevel::new(1), None)
        .unwrap();
    let parameters = types
        .intern_list(&[types.primitive(PrimitiveType::Int)])
        .unwrap();
    let recursive = types
        .intern(TypeKind::Function {
            parameters,
            result: variable,
        })
        .unwrap();
    let error = inference
        .unify(&mut types, variable, recursive, &mut budget())
        .unwrap_err();
    assert!(matches!(error, TypeError::OccursCheck { .. }));

    let variable = inference
        .fresh(&mut types, InferenceLevel::new(1), None)
        .unwrap();
    let list = types.intern(TypeKind::List(variable)).unwrap();
    let mut exhausted = TypeWorkBudget::new(1);
    assert!(matches!(
        inference.unify(&mut types, variable, list, &mut exhausted),
        Err(TypeError::WorkLimitExceeded { .. })
    ));
}

#[test]
fn union_find_uses_deterministic_representatives_and_lowers_levels() {
    let mut types = TypeInterner::new();
    let mut inference = InferenceTable::new();
    let first = inference
        .fresh(&mut types, InferenceLevel::new(4), None)
        .unwrap();
    let second = inference
        .fresh(&mut types, InferenceLevel::new(2), None)
        .unwrap();
    let third = inference
        .fresh(&mut types, InferenceLevel::new(6), None)
        .unwrap();

    inference
        .unify(&mut types, second, first, &mut budget())
        .unwrap();
    inference
        .unify(&mut types, third, second, &mut budget())
        .unwrap();
    let resolved = inference.resolve(&types, third, &mut budget()).unwrap();
    let TypeKind::InferenceVariable(root) = types.kind(resolved) else {
        panic!("unbound variables retain an inference representative");
    };
    assert_eq!(root.raw(), 0, "equal-rank ties choose the lower stable ID");
    assert_eq!(inference.variable(root).level, InferenceLevel::new(2));
    assert_eq!(inference.variable(InferVarId::from_raw(2)).parent, root);
}

#[test]
fn generalization_records_levels_and_instantiation_is_fresh_and_structural() {
    let mut types = TypeInterner::new();
    let mut inference = InferenceTable::new();
    let variable = inference
        .fresh(&mut types, InferenceLevel::new(3), None)
        .unwrap();
    let polymorphic_identity = function(&mut types, &[variable], variable);
    let scheme = inference
        .generalize(
            &mut types,
            polymorphic_identity,
            InferenceLevel::ROOT,
            &mut budget(),
        )
        .unwrap();
    assert_eq!(scheme.quantified_count(), 1);
    assert_eq!(scheme.variables()[0].level, InferenceLevel::new(3));

    let first = inference
        .instantiate(
            &mut types,
            &scheme,
            InferenceLevel::new(1),
            None,
            &mut budget(),
        )
        .unwrap();
    let second = inference
        .instantiate(
            &mut types,
            &scheme,
            InferenceLevel::new(1),
            None,
            &mut budget(),
        )
        .unwrap();
    assert_ne!(first, second);

    for instance in [first, second] {
        let TypeKind::Function { parameters, result } = types.kind(instance) else {
            panic!("scheme body remains a function");
        };
        assert_eq!(types.list(parameters), &[result]);
    }
}

#[test]
fn generalization_leaves_environment_level_variables_monomorphic() {
    let mut types = TypeInterner::new();
    let mut inference = InferenceTable::new();
    let environment_variable = inference
        .fresh(&mut types, InferenceLevel::ROOT, None)
        .unwrap();
    let scheme = inference
        .generalize(
            &mut types,
            environment_variable,
            InferenceLevel::ROOT,
            &mut budget(),
        )
        .unwrap();
    assert_eq!(scheme.quantified_count(), 0);
    assert_eq!(scheme.body, environment_variable);
}

#[test]
fn structural_rewrites_reject_adversarial_depth_before_exhausting_the_stack() {
    let mut types = TypeInterner::new();
    let mut nested = types.primitive(PrimitiveType::Int);
    for _ in 0..8 {
        nested = types.intern(TypeKind::List(nested)).unwrap();
    }
    let substitution = TypeSubstitution::new();
    let mut shallow = TypeWorkBudget::with_max_depth(1_000, 3);
    assert!(matches!(
        substitution.apply(&mut types, nested, &mut shallow),
        Err(TypeError::DepthLimitExceeded { limit: 3, .. })
    ));
}

#[test]
fn substitutions_iterate_in_stable_parameter_order_and_rebuild_canonically() {
    let mut types = TypeInterner::new();
    let first_parameter = TypeParamId::from_raw(1);
    let second_parameter = TypeParamId::from_raw(9);
    let first = types
        .intern(TypeKind::GenericParameter(first_parameter))
        .unwrap();
    let second = types
        .intern(TypeKind::GenericParameter(second_parameter))
        .unwrap();
    let source = function(&mut types, &[second, first], second);

    let int = types.primitive(PrimitiveType::Int);
    let string = types.primitive(PrimitiveType::String);
    let mut substitution = TypeSubstitution::new();
    substitution.insert(second_parameter, string);
    substitution.insert(first_parameter, int);
    assert_eq!(
        substitution.iter().collect::<Vec<_>>(),
        [(first_parameter, int), (second_parameter, string)]
    );

    let output = substitution
        .apply(&mut types, source, &mut budget())
        .unwrap();
    let expected = function(&mut types, &[string, int], string);
    assert_eq!(output, expected);
    assert_eq!(
        substitution
            .apply(&mut types, source, &mut budget())
            .unwrap(),
        expected
    );
}
