pub mod alloc;
pub mod arc;
pub mod io;
pub mod string;
pub mod list;
pub mod thread;

// L++ Runtime Library
// 
// This module provides the native runtime implementations for L++ built-ins.
// When Cranelift AOT compiles an L++ program, it will link against these
// Rust implementations for memory management (ARC), I/O (print), and collections.
