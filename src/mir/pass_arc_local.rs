//! Whole-program single-thread proof for ARC.
//!
//! # Why
//!
//! An atomic read-modify-write is not slow because it is atomic; it is slow
//! because it takes exclusive ownership of a cache line. When two cores
//! retain/release the same object that line ping-pongs between them and
//! throughput collapses. Measured on this machine (2 cores, 20M retain/release
//! pairs):
//!
//! | scenario                                  | throughput  |
//! |-------------------------------------------|-------------|
//! | 1 thread, non-atomic                      | 225.9 M/s   |
//! | 1 thread, atomic ACQ_REL (what L++ did)   |  78.7 M/s   |
//! | 2 threads, private objects                | 156.1 M/s   |
//! | 2 threads, **sharing one object**         |  31.3 M/s   |
//!
//! Two threads sharing an object finish *slower than one thread*. That is the
//! contention this pass exists to avoid paying for when there is no contention
//! to begin with.
//!
//! # What
//!
//! If a program can never create a second thread, every atomic refcount
//! operation in it is pure overhead — there is nothing to synchronise against.
//! This pass proves that property over the whole MIR program and, when it
//! holds, rewrites `lpp_arc_retain`/`lpp_arc_release` to non-atomic variants.
//!
//! # Why static, and not a flag on the object
//!
//! The obvious alternative is a per-object "is shared" bit checked at run time.
//! That was measured too, and it is worse where it matters: the check has to
//! load the flag from the same cache line that is already contended, taking the
//! shared 2-thread case from 0.487s to 0.770s. A compile-time decision costs
//! nothing at run time and cannot regress the contended path.
//!
//! # Soundness
//!
//! The analysis is a whole-program *proof of absence*, so it must be
//! conservative in every direction. Any of the following forces atomics:
//!
//!   * `Rvalue::SpawnThread` anywhere in the program (the `spawn` expression);
//!   * a call to the `lpp_thread_spawn` builtin by any spelling;
//!   * any `CallIndirect`, because the callee is not known here and could be a
//!     spawning closure;
//!   * any extern/FFI declaration, because foreign code can create threads
//!     behind the compiler's back.
//!
//! The last two make this pass decline to optimise rather than guess. A program
//! that uses FFI keeps exactly the behaviour it has today.

use super::ir::{MirInstr, MirProgram, Rvalue};

/// Runtime symbols that create a thread. Matched on the MIR builtin name.
const THREAD_SPAWNING_BUILTINS: &[&str] = &["lpp_thread_spawn", "thread_spawn", "spawn"];

/// True when no execution of this program can produce a second thread.
///
/// `has_extern` comes from the AST: MIR has already lowered extern calls to
/// ordinary builtin calls, so the information is not recoverable here.
pub fn is_provably_single_threaded(program: &MirProgram, has_extern: bool) -> bool {
    if has_extern {
        // Foreign code can spawn threads without any MIR evidence.
        return false;
    }

    for function in program.functions.values() {
        for block in &function.blocks {
            for instr in &block.instrs {
                let rvalue = match instr {
                    MirInstr::Assign(_, rvalue) => rvalue,
                    MirInstr::AssignField { .. }
                    | MirInstr::Retain(_)
                    | MirInstr::Release(_) => continue,
                };
                match rvalue {
                    Rvalue::SpawnThread(_) => return false,
                    Rvalue::BuiltinCall(name, _) => {
                        if THREAD_SPAWNING_BUILTINS.iter().any(|s| name == s) {
                            return false;
                        }
                    }
                    // The callee is unknown at this point; it may spawn.
                    Rvalue::CallIndirect(_, _) => return false,
                    _ => {}
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir::ir::{BlockId, FuncId, LocalId, MirBlock, MirFunction, Operand, Terminator};
    use crate::types::TypeRef;
    use std::collections::HashMap;

    fn program_with(instrs: Vec<MirInstr>) -> MirProgram {
        let mut functions = HashMap::new();
        functions.insert(
            FuncId(0),
            MirFunction {
                id: FuncId(0),
                name: "main".to_string(),
                params: Vec::new(),
                locals: Vec::new(),
                blocks: vec![MirBlock {
                    id: BlockId(0),
                    instrs,
                    terminator: Terminator::Return(None),
                }],
                start_block: BlockId(0),
                return_type: TypeRef::Void,
                is_async: false,
            },
        );
        MirProgram { functions }
    }

    #[test]
    fn plain_program_is_single_threaded() {
        let p = program_with(vec![MirInstr::Retain(LocalId(0))]);
        assert!(is_provably_single_threaded(&p, false));
    }

    #[test]
    fn spawn_forces_atomics() {
        let p = program_with(vec![MirInstr::Assign(
            LocalId(0),
            Rvalue::SpawnThread(Operand::Local(LocalId(1))),
        )]);
        assert!(!is_provably_single_threaded(&p, false));
    }

    #[test]
    fn thread_builtin_forces_atomics() {
        let p = program_with(vec![MirInstr::Assign(
            LocalId(0),
            Rvalue::BuiltinCall("lpp_thread_spawn".to_string(), Vec::new()),
        )]);
        assert!(!is_provably_single_threaded(&p, false));
    }

    #[test]
    fn indirect_call_forces_atomics() {
        // The callee is unknown, so it might spawn.
        let p = program_with(vec![MirInstr::Assign(
            LocalId(0),
            Rvalue::CallIndirect(Operand::Local(LocalId(1)), Vec::new()),
        )]);
        assert!(!is_provably_single_threaded(&p, false));
    }

    #[test]
    fn extern_forces_atomics() {
        // FFI can create threads with no MIR evidence at all.
        let p = program_with(vec![MirInstr::Retain(LocalId(0))]);
        assert!(!is_provably_single_threaded(&p, true));
    }
}
