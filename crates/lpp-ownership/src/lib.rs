//! Phase 4D/4E: ownership graph, placement, and balance.
//!
//! A read-only ownership plan over validated MIR: per-cell
//! Frame/Owned/Shared placement, the type containment graph with cycle
//! breaking, per-function frame arenas, a plan proof (4D), and a
//! static ownership-balance proof over the MIR (4E, `E4403`) that the
//! interpreter's ARC execution (`execute_mir_arc`, pinned set from
//! `OwnershipPlan::pinned_types`) mirrors at runtime. Pass adapters
//! establish the `Ownership`, `NoOwningCycles`, and
//! `OwnershipBalance` invariants in the pass manager's bookkeeping.
//! 4D/4E never mutate MIR.

#![forbid(unsafe_code)]

mod balance;
mod pass;
mod plan;
mod snapshot;
mod verify;

pub use balance::{
    OwnershipBalanceError, OwnershipBalanceErrorKind, OwnershipBalancePass,
    verify_ownership_balance,
};
pub use pass::OwnershipPlanPass;
pub use plan::{
    ArenaPlan, CellPlan, ContainmentNode, OwnershipError, OwnershipErrorKind, OwnershipPlan,
    OwnershipStats, TypeNode, TypeNodeId, TypeStrategy, ValuePlacement, compute_ownership_plan,
};
pub use snapshot::ownership_plan_snapshot;
pub use verify::{OwnershipPlanError, OwnershipPlanErrorKind, verify_ownership_plan};
