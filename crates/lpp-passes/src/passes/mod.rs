//! The Phase 4F optimizer passes: ownership-aware scalar and CFG
//! rewrites of validated MIR, applied in place.
//!
//! Each pass declares the invariants it requires and preserves, and
//! the manager revalidates the core invariants after every pass. The
//! passes never add, remove, or move instructions or blocks — they
//! rewrite an instruction's kind or a block's terminator in place —
//! so program shape (IDs, block and instruction order) is preserved.

mod analysis;
mod branchfold;
mod constfold;
mod constprop;

pub use analysis::{is_primitive_constant, single_def_constants};
pub use branchfold::BranchFoldPass;
pub use constfold::{ConstFoldPass, fold_binary, fold_unary};
pub use constprop::ConstPropPass;
