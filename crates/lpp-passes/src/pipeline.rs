//! Level-driven entry points for the 4F optimizer.

use std::collections::BTreeSet;

use lpp_common::OptimizationLevel;
use lpp_mir::{Constant, InstructionKind, MirFunctionId, MirProgram, Operand, Rvalue, Terminator};
use lpp_types::TypeInterner;

use crate::passes::{
    AlgebraicSimplifyPass, BranchFoldPass, ConstFoldPass, ConstPropPass, DispatchThreadPass,
    InlinePass,
};
use crate::{PassManager, PassManagerError, PassManagerOutcome};

/// One canonical local optimization sweep: `const-fold`, `const-prop`,
/// `algebraic-simplify`, `branch-fold`, then `dispatch-thread`, each
/// revalidated by the manager afterwards.
#[must_use]
pub fn optimization_passes() -> PassManager {
    let mut manager = PassManager::new();
    manager.push(ConstFoldPass);
    manager.push(ConstPropPass);
    manager.push(AlgebraicSimplifyPass);
    manager.push(BranchFoldPass);
    manager.push(DispatchThreadPass);
    manager
}

/// Run the canonical sweep up to the level's fixed-point budget (O0: 1, O1:
/// 2, O2: 4, O3: 6, Os: 3, Oz: 2), stopping when a sweep changes nothing.
pub fn run_optimization(
    program: &mut MirProgram,
    types: &TypeInterner,
    level: OptimizationLevel,
) -> Result<PassManagerOutcome, PassManagerError> {
    run_optimization_internal(program, types, level, None)
}

/// Run local optimization plus a safe, whole-program MIR inlining pass.
///
/// All L++ modules are already merged into one `MirProgram`, so this lets
/// direct calls across source modules inline before object emission. The
/// current inliner only substitutes tiny pure scalar callees; this is not
/// linker/object LTO and does not optimize the runtime or foreign libraries.
pub fn run_optimization_with_lto(
    program: &mut MirProgram,
    types: &TypeInterner,
    level: OptimizationLevel,
    enable_lto: bool,
) -> Result<PassManagerOutcome, PassManagerError> {
    let inline = enable_lto.then(InlinePass::whole_program);
    run_optimization_internal(program, types, level, inline)
}

/// Run local optimization with profile-guided inlining restricted to the
/// provided hot-callee set. Explicit LTO takes precedence and inlines every
/// eligible tiny scalar callee.
pub fn run_optimization_with_profile(
    program: &mut MirProgram,
    types: &TypeInterner,
    level: OptimizationLevel,
    enable_lto: bool,
    hot_callees: BTreeSet<MirFunctionId>,
) -> Result<PassManagerOutcome, PassManagerError> {
    let inline = if enable_lto {
        Some(InlinePass::whole_program())
    } else if hot_callees.is_empty() {
        None
    } else {
        Some(InlinePass::profile_guided(hot_callees))
    };
    run_optimization_internal(program, types, level, inline)
}

#[inline]
fn fnv_u64(hash: &mut u64, val: u64) {
    for b in val.to_le_bytes() {
        *hash ^= u64::from(b);
        *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

fn hash_operand(hash: &mut u64, op: Operand) {
    match op {
        Operand::Copy(local) => {
            fnv_u64(hash, 1);
            fnv_u64(hash, u64::from(local.raw()));
        }
        Operand::Function(f) => {
            fnv_u64(hash, 2);
            fnv_u64(hash, u64::from(f.raw()));
        }
        Operand::Constant(c) => {
            fnv_u64(hash, 3);
            match c {
                Constant::Integer(i) => {
                    fnv_u64(hash, 1);
                    fnv_u64(hash, i as u64);
                }
                Constant::FloatBits(bits) => {
                    fnv_u64(hash, 2);
                    fnv_u64(hash, bits);
                }
                Constant::String { string, .. } => {
                    fnv_u64(hash, 3);
                    fnv_u64(hash, u64::from(string.raw()));
                }
                Constant::Character { character, .. } => {
                    fnv_u64(hash, 4);
                    fnv_u64(hash, u64::from(u32::from(character)));
                }
                Constant::Bool(b) => {
                    fnv_u64(hash, 5);
                    fnv_u64(hash, u64::from(b));
                }
            }
        }
    }
}

fn mir_structural_digest(program: &MirProgram) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for (fn_id, function) in program.functions() {
        fnv_u64(&mut hash, u64::from(fn_id.raw()));
        let blocks = program.function_blocks(function);
        fnv_u64(&mut hash, blocks.len() as u64);
        for &block_id in blocks {
            let block = program.block(block_id).expect("block");
            fnv_u64(&mut hash, u64::from(block_id.raw()));
            let insts = program.block_instructions(block);
            fnv_u64(&mut hash, insts.len() as u64);
            for &inst_id in insts {
                let inst = program.instruction(inst_id).expect("inst");
                match inst.kind {
                    InstructionKind::Assign { target, value } => {
                        fnv_u64(&mut hash, 10);
                        fnv_u64(&mut hash, u64::from(target.raw()));
                        match value {
                            Rvalue::Use(op) => {
                                fnv_u64(&mut hash, 1);
                                hash_operand(&mut hash, op);
                            }
                            Rvalue::Unary { operator, operand } => {
                                fnv_u64(&mut hash, 2);
                                fnv_u64(&mut hash, operator as u64);
                                hash_operand(&mut hash, operand);
                            }
                            Rvalue::Binary {
                                left,
                                operator,
                                right,
                            } => {
                                fnv_u64(&mut hash, 3);
                                fnv_u64(&mut hash, operator as u64);
                                hash_operand(&mut hash, left);
                                hash_operand(&mut hash, right);
                            }
                            Rvalue::Cast { value, target } => {
                                fnv_u64(&mut hash, 4);
                                hash_operand(&mut hash, value);
                                fnv_u64(&mut hash, u64::from(target.raw()));
                            }
                            Rvalue::Tuple(ops)
                            | Rvalue::List(ops)
                            | Rvalue::ConstructStruct { fields: ops, .. }
                            | Rvalue::ConstructVariant { fields: ops, .. } => {
                                fnv_u64(&mut hash, 5);
                                for &op in program.operands(ops) {
                                    hash_operand(&mut hash, op);
                                }
                            }
                            Rvalue::Load(place_id) => {
                                fnv_u64(&mut hash, 6);
                                let place = program.place(place_id).expect("place");
                                fnv_u64(&mut hash, u64::from(place.root.raw()));
                                fnv_u64(&mut hash, place.projections.len() as u64);
                            }
                            Rvalue::ListLen(op) | Rvalue::Await(op) | Rvalue::Spawn(op) => {
                                fnv_u64(&mut hash, 7);
                                hash_operand(&mut hash, op);
                            }
                            Rvalue::Call { callee, arguments } => {
                                fnv_u64(&mut hash, 8);
                                hash_operand(&mut hash, callee);
                                for &op in program.operands(arguments) {
                                    hash_operand(&mut hash, op);
                                }
                            }
                            Rvalue::ExternalCall {
                                function: f,
                                arguments,
                            } => {
                                fnv_u64(&mut hash, 9);
                                fnv_u64(&mut hash, u64::from(f.raw()));
                                for &op in program.operands(arguments) {
                                    hash_operand(&mut hash, op);
                                }
                            }
                            Rvalue::Builtin { builtin, arguments } => {
                                fnv_u64(&mut hash, 10);
                                fnv_u64(&mut hash, u64::from(builtin.raw()));
                                for &op in program.operands(arguments) {
                                    hash_operand(&mut hash, op);
                                }
                            }
                            Rvalue::Intrinsic {
                                intrinsic,
                                type_argument,
                                arguments,
                            } => {
                                fnv_u64(&mut hash, 11);
                                fnv_u64(&mut hash, intrinsic as u64);
                                fnv_u64(
                                    &mut hash,
                                    type_argument.map_or(u64::MAX, |t| u64::from(t.raw())),
                                );
                                for &op in program.operands(arguments) {
                                    hash_operand(&mut hash, op);
                                }
                            }
                            Rvalue::MakeClosure { function, captures } => {
                                fnv_u64(&mut hash, 12);
                                fnv_u64(&mut hash, u64::from(function.raw()));
                                for &op in program.operands(captures) {
                                    hash_operand(&mut hash, op);
                                }
                            }
                        }
                    }
                    InstructionKind::Store { place, value } => {
                        fnv_u64(&mut hash, 20);
                        let p = program.place(place).expect("place");
                        fnv_u64(&mut hash, u64::from(p.root.raw()));
                        fnv_u64(&mut hash, p.projections.len() as u64);
                        hash_operand(&mut hash, value);
                    }
                }
            }
            match block.terminator {
                Terminator::Goto(t) => {
                    fnv_u64(&mut hash, 30);
                    fnv_u64(&mut hash, u64::from(t.raw()));
                }
                Terminator::Branch {
                    condition,
                    then_block,
                    else_block,
                } => {
                    fnv_u64(&mut hash, 31);
                    hash_operand(&mut hash, condition);
                    fnv_u64(&mut hash, u64::from(then_block.raw()));
                    fnv_u64(&mut hash, u64::from(else_block.raw()));
                }
                Terminator::SwitchEnum {
                    subject,
                    aggregate,
                    targets,
                } => {
                    fnv_u64(&mut hash, 32);
                    hash_operand(&mut hash, subject);
                    fnv_u64(&mut hash, u64::from(aggregate.raw()));
                    for &t in program.switch_targets(targets) {
                        fnv_u64(&mut hash, u64::from(t.raw()));
                    }
                }
                Terminator::Return(op) => {
                    fnv_u64(&mut hash, 33);
                    if let Some(op) = op {
                        hash_operand(&mut hash, op);
                    }
                }
                Terminator::Unreachable => {
                    fnv_u64(&mut hash, 34);
                }
            }
        }
    }
    hash
}

fn run_optimization_internal(
    program: &mut MirProgram,
    types: &TypeInterner,
    level: OptimizationLevel,
    inline: Option<InlinePass>,
) -> Result<PassManagerOutcome, PassManagerError> {
    let sweeps = level.budget().fixed_point_iterations;
    let mut outcome: Option<PassManagerOutcome> = None;
    for sweep in 0..sweeps {
        let before = if sweeps > 1 && sweep + 1 < sweeps {
            Some(mir_structural_digest(program))
        } else {
            None
        };
        let mut manager = optimization_passes();
        if let Some(inline) = inline.clone() {
            manager.push(inline);
        }
        outcome = Some(manager.run(program, types)?);
        if let Some(before) = before
            && mir_structural_digest(program) == before
        {
            break;
        }
    }
    Ok(outcome.expect("every optimization budget allows at least one sweep"))
}
