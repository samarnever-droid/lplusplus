pub mod compiler;
pub mod lower;
pub mod types;

// Cranelift AOT Backend
//
// This module translates L++ MIR into Cranelift IR (CLIF).
// It leverages cranelift_object to emit standard native object files (.o).
