//! Tiny, safety-preserving direct-call inliner for scalar MIR functions.
//!
//! It only accepts a single straight-line basic block made of scalar `Use` and
//! `BinaryOp` assignments. No calls, allocation, ownership operations, field
//! access, branches, or borrowed operands can enter this pass.

use std::collections::HashMap;
use crate::mir::ir::*;

#[derive(Clone)]
struct Candidate { params: Vec<LocalId>, locals: Vec<LocalDecl>, instrs: Vec<MirInstr>, result: Operand }

fn remap_operand(op: &Operand, map: &HashMap<LocalId, Operand>) -> Option<Operand> {
    match op { Operand::Local(id) => map.get(id).cloned(), Operand::Int(_) | Operand::Bool(_) => Some(op.clone()), _ => None }
}

fn candidate(function: &MirFunction) -> Option<Candidate> {
    if function.locals.iter().any(|local| local.ownership != Ownership::Copy) { return None; }
    let block = function.blocks.first()?;
    let Terminator::Return(Some(result)) = &block.terminator else { return None; };
    if !matches!(result, Operand::Local(_) | Operand::Int(_) | Operand::Bool(_)) { return None; }
    for instruction in &block.instrs {
        match instruction {
            MirInstr::Assign(_, Rvalue::Use(Operand::Local(_) | Operand::Int(_) | Operand::Bool(_))) => {}
            MirInstr::Assign(_, Rvalue::BinaryOp(_, left, right))
                if matches!(left, Operand::Local(_) | Operand::Int(_) | Operand::Bool(_))
                && matches!(right, Operand::Local(_) | Operand::Int(_) | Operand::Bool(_)) => {}
            _ => return None,
        }
    }
    // Lowering currently leaves an unreachable trailing return block; accept it
    // only when it contains no instructions and no control-flow work.
    if function.blocks.iter().skip(1).any(|block| !block.instrs.is_empty()) { return None; }
    Some(Candidate { params: function.params.clone(), locals: function.locals.clone(), instrs: block.instrs.clone(), result: result.clone() })
}

/// Inline scalar-only direct calls before ARC insertion. Returns inlined calls.
pub fn run(program: &mut MirProgram) -> usize {
    let candidates: HashMap<FuncId, Candidate> = program.functions.iter()
        .filter_map(|(id, function)| candidate(function).map(|candidate| (*id, candidate))).collect();
    let mut inlined = 0;
    for function in program.functions.values_mut() {
        for block in &mut function.blocks {
            let original = std::mem::take(&mut block.instrs);
            let mut expanded = Vec::with_capacity(original.len());
            for instruction in original {
                let MirInstr::Assign(destination, Rvalue::CallDirect(target, args)) = &instruction else {
                    expanded.push(instruction); continue;
                };
                let Some(candidate) = candidates.get(target) else { expanded.push(instruction); continue; };
                if candidate.params.len() != args.len() || *target == function.id { expanded.push(instruction); continue; }
                let mut map: HashMap<LocalId, Operand> = candidate.params.iter().copied().zip(args.iter().cloned()).collect();
                let returned = match &candidate.result { Operand::Local(id) => Some(*id), _ => None };
                let mut generated = Vec::new();
                let mut valid = true;
                for candidate_instruction in &candidate.instrs {
                    let MirInstr::Assign(old_dest, rvalue) = candidate_instruction else { valid = false; break; };
                    let new_dest = if Some(*old_dest) == returned { *destination } else {
                        let source = &candidate.locals[old_dest.0];
                        let id = LocalId(function.locals.len());
                        let mut local = source.clone(); local.id = id; local.debug_name = Some(format!("inline_{}_{}", target.0, old_dest.0));
                        function.locals.push(local); map.insert(*old_dest, Operand::Local(id)); id
                    };
                    let new_rvalue = match rvalue {
                        Rvalue::Use(value) => remap_operand(value, &map).map(Rvalue::Use),
                        Rvalue::BinaryOp(op, left, right) => match (remap_operand(left, &map), remap_operand(right, &map)) {
                            (Some(left), Some(right)) => Some(Rvalue::BinaryOp(op.clone(), left, right)),
                            _ => None,
                        },
                        _ => None,
                    };
                    let Some(new_rvalue) = new_rvalue else { valid = false; break; };
                    map.insert(*old_dest, Operand::Local(new_dest));
                    generated.push(MirInstr::Assign(new_dest, new_rvalue));
                }
                if valid {
                    if !matches!(candidate.result, Operand::Local(_)) {
                        if let Some(value) = remap_operand(&candidate.result, &map) { generated.push(MirInstr::Assign(*destination, Rvalue::Use(value))); }
                    }
                    expanded.extend(generated); inlined += 1;
                } else { expanded.push(instruction); }
            }
            block.instrs = expanded;
        }
    }
    inlined
}
