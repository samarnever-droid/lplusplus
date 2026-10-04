use std::mem::size_of;
use std::sync::Arc;

use lpp_common::{OptimizationLevel, OptimizationOptions};
use lpp_hir::{DefId, HirItemId, InstanceId, TypeParamId};
use lpp_types::{
    InstanceGrowthError, InstanceKey, InstanceLimits, InstancePlanner, InstanceRequestKind,
    InstanceRequestOutcome, PrimitiveType, TraitBound, TraitCoherenceError, TraitGoal, TraitImplId,
    TraitIndex, TraitRule, TraitSolution, TraitSolverLimits, TraitSolverOverflow, TypeId,
    TypeInterner, TypeKind,
};

fn nominal(types: &mut TypeInterner, definition: u32, arguments: &[TypeId]) -> TypeId {
    let arguments = types.intern_list(arguments).unwrap();
    types
        .intern(TypeKind::Nominal {
            definition: DefId::from_raw(definition),
            arguments,
        })
        .unwrap()
}

fn parameter(types: &mut TypeInterner, raw: u32) -> TypeId {
    types
        .intern(TypeKind::GenericParameter(TypeParamId::from_raw(raw)))
        .unwrap()
}

fn rule(
    origin: u32,
    trait_definition: u32,
    self_pattern: TypeId,
    parameters: &[u32],
    bounds: &[TraitBound],
    types: &TypeInterner,
) -> TraitRule {
    TraitRule {
        origin: HirItemId::from_raw(origin),
        trait_definition: DefId::from_raw(trait_definition),
        self_pattern,
        trait_arguments: types.empty_list(),
        parameters: parameters
            .iter()
            .map(|raw| TypeParamId::from_raw(*raw))
            .collect::<Vec<_>>()
            .into(),
        bounds: Arc::from(bounds),
    }
}

#[test]
fn trait_and_instance_identities_are_compact() {
    assert_eq!(size_of::<TraitImplId>(), 4);
    assert_eq!(size_of::<Option<TraitImplId>>(), 4);
    assert_eq!(size_of::<InstanceId>(), 4);
    assert_eq!(size_of::<Option<InstanceId>>(), 4);
    assert_eq!(size_of::<InstanceKey>(), 8);
    assert_eq!(InstancePlanner::compact_key_size(), 8);
}

#[test]
fn indexed_solver_selects_most_specific_rule_and_memoizes_goals() {
    let mut types = TypeInterner::new();
    let int = types.primitive(PrimitiveType::Int);
    let string = types.primitive(PrimitiveType::String);
    let generic = parameter(&mut types, 0);
    let generic_box = nominal(&mut types, 10, &[generic]);
    let int_box = nominal(&mut types, 10, &[int]);
    let string_box = nominal(&mut types, 10, &[string]);

    let mut index = TraitIndex::new();
    let generic_id = index
        .register(&types, rule(0, 20, generic_box, &[0], &[], &types))
        .unwrap();
    let concrete_id = index
        .register(&types, rule(1, 20, int_box, &[], &[], &types))
        .unwrap();

    let int_goal = TraitGoal::new(DefId::from_raw(20), int_box, types.empty_list());
    let TraitSolution::Unique(int_selection) =
        index.solve(&mut types, int_goal, TraitSolverLimits::default())
    else {
        panic!("Box[Int] must have one selected implementation");
    };
    assert_eq!(int_selection.implementation, concrete_id);

    let string_goal = TraitGoal::new(DefId::from_raw(20), string_box, types.empty_list());
    let TraitSolution::Unique(string_selection) =
        index.solve(&mut types, string_goal, TraitSolverLimits::default())
    else {
        panic!("Box[Str] must use the generic implementation");
    };
    assert_eq!(string_selection.implementation, generic_id);
    assert_eq!(
        string_selection.substitution.get(TypeParamId::from_raw(0)),
        Some(string)
    );

    let cache_hits = index.stats().cache_hits;
    assert!(matches!(
        index.solve(&mut types, string_goal, TraitSolverLimits::default()),
        TraitSolution::Unique(_)
    ));
    assert_eq!(index.stats().cache_hits, cache_hits + 1);
    assert_eq!(index.memoized_goal_count(), 2);
}

#[test]
fn trait_candidate_index_avoids_scanning_unrelated_type_heads() {
    let mut types = TypeInterner::new();
    let mut index = TraitIndex::new();
    let trait_definition = DefId::from_raw(900);
    let mut selected_type = types.error();
    for raw in 0..512 {
        let self_type = nominal(&mut types, 1_000 + raw, &[]);
        index
            .register(
                &types,
                rule(raw, trait_definition.raw(), self_type, &[], &[], &types),
            )
            .unwrap();
        selected_type = self_type;
    }
    let empty = types.empty_list();
    assert!(matches!(
        index.solve(
            &mut types,
            TraitGoal::new(trait_definition, selected_type, empty),
            TraitSolverLimits::default(),
        ),
        TraitSolution::Unique(_)
    ));
    assert_eq!(
        index.stats().candidates,
        1,
        "the (trait, self-type head) index must avoid a 512-rule scan",
    );
}

#[test]
fn coherence_rejects_equal_specificity_overlap_but_allows_specialization() {
    let mut types = TypeInterner::new();
    let int = types.primitive(PrimitiveType::Int);
    let string = types.primitive(PrimitiveType::String);
    let left_parameter = parameter(&mut types, 0);
    let right_parameter = parameter(&mut types, 1);
    let left = nominal(&mut types, 10, &[left_parameter, int]);
    let right = nominal(&mut types, 10, &[string, right_parameter]);

    let mut index = TraitIndex::new();
    index
        .register(&types, rule(3, 20, left, &[0], &[], &types))
        .unwrap();
    let error = index
        .register(&types, rule(4, 20, right, &[1], &[], &types))
        .unwrap_err();
    assert!(matches!(
        error,
        TraitCoherenceError::Overlap {
            first,
            second,
            ..
        } if first == HirItemId::from_raw(3) && second == HirItemId::from_raw(4)
    ));

    let generic_box = nominal(&mut types, 11, &[left_parameter]);
    let int_box = nominal(&mut types, 11, &[int]);
    let mut specialized = TraitIndex::new();
    specialized
        .register(&types, rule(5, 20, generic_box, &[0], &[], &types))
        .unwrap();
    specialized
        .register(&types, rule(6, 20, int_box, &[], &[], &types))
        .unwrap();
}

#[test]
fn solver_proves_bounds_and_rejects_unsatisfied_candidates() {
    let mut types = TypeInterner::new();
    let int = types.primitive(PrimitiveType::Int);
    let dog = nominal(&mut types, 30, &[]);
    let generic = parameter(&mut types, 0);
    let generic_kennel = nominal(&mut types, 31, &[generic]);
    let dog_kennel = nominal(&mut types, 31, &[dog]);
    let int_kennel = nominal(&mut types, 31, &[int]);
    let speak = DefId::from_raw(40);
    let housed = DefId::from_raw(41);
    let empty = types.empty_list();

    let mut index = TraitIndex::new();
    index
        .register(&types, rule(0, speak.raw(), dog, &[], &[], &types))
        .unwrap();
    let bound = TraitBound {
        parameter: TypeParamId::from_raw(0),
        trait_definition: speak,
        arguments: types.empty_list(),
    };
    index
        .register(
            &types,
            rule(1, housed.raw(), generic_kennel, &[0], &[bound], &types),
        )
        .unwrap();

    assert!(matches!(
        index.solve(
            &mut types,
            TraitGoal::new(housed, dog_kennel, empty),
            TraitSolverLimits::default(),
        ),
        TraitSolution::Unique(_)
    ));
    assert_eq!(
        index.solve(
            &mut types,
            TraitGoal::new(housed, int_kennel, empty),
            TraitSolverLimits::default(),
        ),
        TraitSolution::NoSolution
    );
}

#[test]
fn solver_reports_candidate_and_recursive_cycle_limits_deterministically() {
    let mut types = TypeInterner::new();
    let int = types.primitive(PrimitiveType::Int);
    let generic = parameter(&mut types, 0);
    let list_generic = types.intern(TypeKind::List(generic)).unwrap();
    let list_int = types.intern(TypeKind::List(int)).unwrap();
    let trait_definition = DefId::from_raw(50);
    let empty = types.empty_list();
    let mut index = TraitIndex::new();
    index
        .register(
            &types,
            rule(0, trait_definition.raw(), generic, &[0], &[], &types),
        )
        .unwrap();
    index
        .register(
            &types,
            rule(1, trait_definition.raw(), list_generic, &[0], &[], &types),
        )
        .unwrap();
    let limits = TraitSolverLimits {
        max_candidates_per_goal: 1,
        ..TraitSolverLimits::default()
    };
    assert_eq!(
        index.solve(
            &mut types,
            TraitGoal::new(trait_definition, list_int, empty),
            limits,
        ),
        TraitSolution::Overflow(TraitSolverOverflow::CandidateLimit)
    );

    let recursive_trait = DefId::from_raw(51);
    let recursive_bound = TraitBound {
        parameter: TypeParamId::from_raw(0),
        trait_definition: recursive_trait,
        arguments: types.empty_list(),
    };
    let mut recursive = TraitIndex::new();
    recursive
        .register(
            &types,
            rule(
                2,
                recursive_trait.raw(),
                generic,
                &[0],
                &[recursive_bound],
                &types,
            ),
        )
        .unwrap();
    let recursive_goal = TraitGoal::new(recursive_trait, int, empty);
    assert_eq!(
        recursive.solve(&mut types, recursive_goal, TraitSolverLimits::default(),),
        TraitSolution::Overflow(TraitSolverOverflow::Cycle)
    );
    assert_eq!(
        recursive.solve(
            &mut types,
            recursive_goal,
            TraitSolverLimits {
                max_goals: 0,
                ..TraitSolverLimits::default()
            },
        ),
        TraitSolution::Overflow(TraitSolverOverflow::GoalLimit)
    );
    assert_eq!(
        recursive.solve(
            &mut types,
            recursive_goal,
            TraitSolverLimits {
                max_work: 0,
                ..TraitSolverLimits::default()
            },
        ),
        TraitSolution::Overflow(TraitSolverOverflow::WorkLimit)
    );
    assert_eq!(
        recursive.solve(
            &mut types,
            recursive_goal,
            TraitSolverLimits {
                max_depth: 0,
                ..TraitSolverLimits::default()
            },
        ),
        TraitSolution::Overflow(TraitSolverOverflow::DepthLimit)
    );
}

#[test]
fn instance_worklist_is_sorted_deduplicated_and_transitive() {
    let mut types = TypeInterner::new();
    let int = types.primitive(PrimitiveType::Int);
    let string = types.primitive(PrimitiveType::String);
    let int_arguments = types.intern_list(&[int]).unwrap();
    let string_arguments = types.intern_list(&[string]).unwrap();
    let int_key = InstanceKey::new(HirItemId::from_raw(2), int_arguments);
    let string_key = InstanceKey::new(HirItemId::from_raw(1), string_arguments);
    let mut planner = InstancePlanner::new(InstanceLimits::default());

    assert_eq!(
        planner
            .request_root(int_key, None, InstanceRequestKind::Required)
            .unwrap(),
        InstanceRequestOutcome::Queued
    );
    assert_eq!(
        planner
            .request_root(string_key, None, InstanceRequestKind::Required)
            .unwrap(),
        InstanceRequestOutcome::Queued
    );
    assert_eq!(
        planner
            .request_root(int_key, None, InstanceRequestKind::Required)
            .unwrap(),
        InstanceRequestOutcome::AlreadyQueued
    );

    let first = planner.pop_next().unwrap().unwrap();
    assert_eq!(first.id, InstanceId::from_raw(0));
    assert_eq!(first.key, string_key, "BTree worklist defines stable order");
    assert_eq!(
        planner
            .request_from(first.id, int_key, None, InstanceRequestKind::Required,)
            .unwrap(),
        InstanceRequestOutcome::AlreadyQueued
    );
    planner.complete(first.id).unwrap();

    let second = planner.pop_next().unwrap().unwrap();
    assert_eq!(second.id, InstanceId::from_raw(1));
    assert_eq!(second.key, int_key);
    assert_eq!(
        planner
            .request_from(second.id, int_key, None, InstanceRequestKind::Required,)
            .unwrap(),
        InstanceRequestOutcome::Existing(second.id)
    );
    planner.complete(second.id).unwrap();
    planner.finish().unwrap();
    assert_eq!(planner.stats().unique_requests, 2);
    assert_eq!(planner.stats().duplicate_requests, 3);
}

#[test]
fn instance_growth_and_optimization_caps_are_explicit() {
    let mut types = TypeInterner::new();
    let int = types.primitive(PrimitiveType::Int);
    let arguments = types.intern_list(&[int]).unwrap();
    let first = InstanceKey::new(HirItemId::from_raw(1), arguments);
    let second = InstanceKey::new(HirItemId::from_raw(2), arguments);
    let limits = InstanceLimits {
        max_instances_global: 1,
        max_instances_per_item: 1,
        max_depth: 1,
        max_fixed_point_rounds: 1,
        max_optional_instances: 0,
        max_optional_rounds: 0,
    };
    let mut planner = InstancePlanner::new(limits);
    planner
        .request_root(first, None, InstanceRequestKind::Required)
        .unwrap();
    assert_eq!(
        planner.request_root(second, None, InstanceRequestKind::Required),
        Err(InstanceGrowthError::GlobalLimit { limit: 1 })
    );
    let work = planner.pop_next().unwrap().unwrap();
    assert_eq!(
        planner.request_from(work.id, second, None, InstanceRequestKind::Required,),
        Err(InstanceGrowthError::FixedPointLimit { limit: 1 })
    );

    let o0 = InstanceLimits::for_optimization(OptimizationOptions {
        level: OptimizationLevel::O0,
        verify_each_pass: false,
    });
    let mut planner = InstancePlanner::new(o0);
    assert_eq!(
        planner.request_root(first, None, InstanceRequestKind::OptionalSpecialization),
        Err(InstanceGrowthError::OptionalInstanceLimit { limit: 0 })
    );
    assert!(
        planner
            .request_root(first, None, InstanceRequestKind::Required)
            .is_ok()
    );
}
