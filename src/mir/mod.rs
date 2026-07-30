pub mod builder;
pub mod ir;
pub mod lower;
pub mod pass_arc;
pub mod pass_moveout;
pub mod pass_arc_local;
pub mod pass_branch;
pub mod pass_closure;
pub mod pass_constprop;
pub mod pass_copyprop;
pub mod pass_dce;
pub mod escape_solver;
pub mod pass_escape;
pub mod pass_strength;
pub mod pass_inline;
pub mod pass_peephole;
pub mod validate_borrows;

// MIR (Mid-level Intermediate Representation) will be defined here.
// This is the bridge between the high-level AST (after analysis) and Cranelift.
// It explicitly flattens expressions, manages ARC increments/decrements,
// and transforms closures into fat pointers.
