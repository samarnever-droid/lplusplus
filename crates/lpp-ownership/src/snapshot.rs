//! Deterministic serialization of an ownership plan.
//!
//! The snapshot uses stable IDs in fixed order — cells by function id
//! then local declaration order, nodes in the contract's emission
//! order, arenas by function id — and never hash-map iteration. It is
//! the observable form of the plan for snapshot tests and for the
//! determinism proof in `verify.rs`.

use crate::plan::{ContainmentNode, OwnershipPlan, TypeStrategy, ValuePlacement};

fn placement_name(placement: ValuePlacement) -> &'static str {
    match placement {
        ValuePlacement::Frame => "frame",
        ValuePlacement::Owned => "owned",
        ValuePlacement::Shared => "shared",
    }
}

fn strategy_name(strategy: TypeStrategy) -> &'static str {
    match strategy {
        TypeStrategy::Owned => "owned",
        TypeStrategy::Shared => "shared",
    }
}

fn node_name(node: &ContainmentNode) -> String {
    match node {
        ContainmentNode::Callable(id) => format!("callable({id:?})"),
        ContainmentNode::Aggregate(id) => format!("aggregate({id:?})"),
        ContainmentNode::List(element) => format!("list[{element:?}]"),
        ContainmentNode::Slice(element) => format!("slice[{element:?}]"),
        ContainmentNode::Task(element) => format!("task[{element:?}]"),
        ContainmentNode::Tuple(list) => format!("tuple[{list:?}]"),
        ContainmentNode::Map(key, value) => format!("map[{key:?}, {value:?}]"),
    }
}

/// Render the plan in its stable observable form.
pub fn ownership_plan_snapshot(plan: &OwnershipPlan) -> String {
    let mut out = String::from("ownership-plan v1\n");
    let stats = &plan.stats;
    out.push_str(&format!(
        "stats: cells={} frame={} owned={} shared={} nodes={} shared-nodes={} cycles={} arenas={}\n",
        stats.cell_count,
        stats.frame_cell_count,
        stats.owned_cell_count,
        stats.shared_cell_count,
        stats.node_count,
        stats.shared_node_count,
        stats.cycle_count,
        stats.arena_count,
    ));

    out.push_str("cells:\n");
    for cell in &plan.cells {
        out.push_str(&format!(
            "  fn({:?}) local({:?}) {}\n",
            cell.function,
            cell.local,
            placement_name(cell.placement)
        ));
    }

    out.push_str("nodes:\n");
    for node in &plan.nodes {
        let contains: Vec<String> = node.contains.iter().map(|id| id.0.to_string()).collect();
        out.push_str(&format!(
            "  node({}) {} ty({:?}) {} [{}]\n",
            node.id.0,
            node_name(&node.node),
            node.ty,
            strategy_name(node.strategy),
            contains.join(", "),
        ));
    }

    out.push_str("arenas:\n");
    for arena in &plan.arenas {
        let cells: Vec<String> = arena
            .cells
            .iter()
            .map(|local| format!("{local:?}"))
            .collect();
        out.push_str(&format!(
            "  arena fn({:?}) [{}]\n",
            arena.function,
            cells.join(", ")
        ));
    }

    out
}
