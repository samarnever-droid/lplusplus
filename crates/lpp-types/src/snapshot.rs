use std::fmt::Write as _;
use std::mem::size_of;

use lpp_hir::{HirPackage, hir_snapshot};

use crate::{ShadowTypeOutput, TraitImplId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticMetrics {
    pub hir_nodes: usize,
    pub interned_types: usize,
    pub interned_type_lists: usize,
    pub type_list_entries: usize,
    pub inference_variables: usize,
    pub aggregate_facts: usize,
    pub place_facts: usize,
    pub enum_flow_facts: usize,
    pub builtin_facts: usize,
    pub trait_rules: usize,
    pub generic_instances: usize,
    pub minimum_payload_bytes: usize,
}

#[must_use]
pub fn semantic_metrics(package: &HirPackage, output: &ShadowTypeOutput) -> SemanticMetrics {
    let hir_nodes = package.origins.len()
        + package.scopes.len()
        + package.locals.len()
        + package.type_refs.len()
        + package.type_parameters.len()
        + package.fields.len()
        + package.variants.len()
        + package.expressions.len()
        + package.statements.len()
        + package.bodies.len()
        + package.match_arms.len()
        + package.items.len();
    let type_list_entries = output
        .interner
        .type_lists()
        .map(|(_, types)| types.len())
        .sum::<usize>();
    let assignment_slots = package.expressions.len()
        + package.locals.len()
        + package.type_refs.len()
        + package.items.len();
    let minimum_payload_bytes = package.origins.len() * size_of::<lpp_hir::Origin>()
        + package.scopes.len() * size_of::<lpp_hir::Scope>()
        + package.locals.len() * size_of::<lpp_hir::Local>()
        + package.type_refs.len() * size_of::<lpp_hir::TypeRef>()
        + package.type_parameters.len() * size_of::<lpp_hir::TypeParameter>()
        + package.fields.len() * size_of::<lpp_hir::Field>()
        + package.variants.len() * size_of::<lpp_hir::Variant>()
        + package.expressions.len() * size_of::<lpp_hir::Expression>()
        + package.statements.len() * size_of::<lpp_hir::Statement>()
        + package.bodies.len() * size_of::<lpp_hir::Body>()
        + package.match_arms.len() * size_of::<lpp_hir::MatchArm>()
        + package.items.len() * size_of::<lpp_hir::HirItem>()
        + output.interner.len() * size_of::<crate::TypeKind>()
        + output.interner.list_count() * size_of::<std::sync::Arc<[crate::TypeId]>>()
        + type_list_entries * size_of::<crate::TypeId>()
        + output.inference.len() * size_of::<crate::InferenceVariable>()
        + assignment_slots * size_of::<Option<crate::TypeId>>()
        + output.aggregates.expression_slot_count()
            * size_of::<Option<crate::AggregateExpressionFact>>()
        + output.places.expression_slot_count() * size_of::<Option<crate::PlaceExpressionFact>>()
        + output.places.statement_slot_count() * size_of::<Option<crate::PlaceStatementFact>>()
        + output.enum_flow.match_slot_count() * size_of::<Option<crate::EnumMatchFact>>()
        + output.enum_flow.arm_slot_count() * size_of::<Option<crate::EnumMatchArmFact>>()
        + output.enum_flow.try_slot_count() * size_of::<Option<crate::EnumTryFact>>()
        + package.expressions.len() * size_of::<Option<crate::BuiltinFact>>()
        + output.trait_index.len() * size_of::<crate::TraitRule>()
        + size_of_val(output.instances.records());
    SemanticMetrics {
        hir_nodes,
        interned_types: output.interner.len(),
        interned_type_lists: output.interner.list_count(),
        type_list_entries,
        inference_variables: output.inference.len(),
        aggregate_facts: output.aggregates.expressions().count(),
        place_facts: output
            .places
            .expressions()
            .count()
            .saturating_add(output.places.statements().count()),
        enum_flow_facts: output.enum_flow.fact_count(),
        builtin_facts: output.builtins.len(),
        trait_rules: output.trait_index.len(),
        generic_instances: output.instances.records().len(),
        minimum_payload_bytes,
    }
}

#[must_use]
pub fn semantic_snapshot(package: &HirPackage, output: &ShadowTypeOutput) -> String {
    let mut snapshot = hir_snapshot(package);
    writeln!(snapshot, "semantic-v4").expect("writing to a String cannot fail");
    writeln!(snapshot, "metrics {:?}", semantic_metrics(package, output))
        .expect("writing to a String cannot fail");
    writeln!(snapshot, "types").expect("writing to a String cannot fail");
    for (id, kind) in output.interner.types() {
        writeln!(snapshot, "  {id:?} {kind:?}").expect("writing to a String cannot fail");
    }
    writeln!(snapshot, "type_lists").expect("writing to a String cannot fail");
    for (id, types) in output.interner.type_lists() {
        writeln!(snapshot, "  {id:?} {types:?}").expect("writing to a String cannot fail");
    }
    writeln!(snapshot, "inference").expect("writing to a String cannot fail");
    for raw in 0..output.inference.len() {
        let id = crate::InferVarId::from_raw(
            u32::try_from(raw).expect("inference-variable count fits its ID"),
        );
        writeln!(snapshot, "  {id:?} {:?}", output.inference.variable(id))
            .expect("writing to a String cannot fail");
    }
    writeln!(snapshot, "assignments").expect("writing to a String cannot fail");
    for (id, _) in package.expressions.enumerate() {
        writeln!(
            snapshot,
            "  expression {id:?} {:?}",
            output.assignments.expression(id)
        )
        .expect("writing to a String cannot fail");
    }
    for (id, _) in package.locals.enumerate() {
        writeln!(
            snapshot,
            "  local {id:?} {:?}",
            output.assignments.local(id)
        )
        .expect("writing to a String cannot fail");
        if let Some(scheme) = output.assignments.local_scheme(id) {
            writeln!(snapshot, "  scheme {id:?} {scheme:?}")
                .expect("writing to a String cannot fail");
        }
    }
    for (id, _) in package.type_refs.enumerate() {
        writeln!(
            snapshot,
            "  type_ref {id:?} {:?}",
            output.assignments.type_ref(id)
        )
        .expect("writing to a String cannot fail");
    }
    for (id, _) in package.items.enumerate() {
        writeln!(snapshot, "  item {id:?} {:?}", output.assignments.item(id))
            .expect("writing to a String cannot fail");
    }
    writeln!(snapshot, "aggregate_facts").expect("writing to a String cannot fail");
    for (expression, fact) in output.aggregates.expressions() {
        writeln!(snapshot, "  {expression:?} {fact:?}").expect("writing to a String cannot fail");
    }
    writeln!(snapshot, "place_facts").expect("writing to a String cannot fail");
    for (expression, fact) in output.places.expressions() {
        writeln!(snapshot, "  expression {expression:?} {fact:?}")
            .expect("writing to a String cannot fail");
    }
    for (statement, fact) in output.places.statements() {
        writeln!(snapshot, "  statement {statement:?} {fact:?}")
            .expect("writing to a String cannot fail");
    }
    writeln!(snapshot, "enum_flow_facts").expect("writing to a String cannot fail");
    for (statement, fact) in output.enum_flow.matches() {
        writeln!(snapshot, "  match {statement:?} {fact:?}")
            .expect("writing to a String cannot fail");
    }
    for (arm, fact) in output.enum_flow.arms() {
        writeln!(snapshot, "  arm {arm:?} {fact:?}").expect("writing to a String cannot fail");
    }
    for (expression, fact) in output.enum_flow.tries() {
        writeln!(snapshot, "  try {expression:?} {fact:?}")
            .expect("writing to a String cannot fail");
    }
    writeln!(snapshot, "builtin_facts").expect("writing to a String cannot fail");
    for (expression, fact) in output.builtins.expressions() {
        writeln!(snapshot, "  {expression:?} {fact:?}").expect("writing to a String cannot fail");
    }

    writeln!(snapshot, "trait_rules").expect("writing to a String cannot fail");
    for raw in 0..output.trait_index.len() {
        let id = TraitImplId::from_raw(u32::try_from(raw).expect("trait-rule count fits its ID"));
        writeln!(
            snapshot,
            "  {id:?} specificity={} {:?}",
            output
                .trait_index
                .specificity(id)
                .expect("snapshot trait rule exists"),
            output
                .trait_index
                .rule(id)
                .expect("snapshot trait rule exists"),
        )
        .expect("writing to a String cannot fail");
    }
    writeln!(snapshot, "trait_diagnostics {:?}", output.trait_diagnostics)
        .expect("writing to a String cannot fail");
    writeln!(snapshot, "instances").expect("writing to a String cannot fail");
    for record in output.instances.records() {
        writeln!(snapshot, "  {record:?}").expect("writing to a String cannot fail");
    }
    writeln!(
        snapshot,
        "instance_diagnostics {:?}",
        output.instance_diagnostics
    )
    .expect("writing to a String cannot fail");
    snapshot
}
