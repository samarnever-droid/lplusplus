/// Copy propagation pass: eliminates redundant temporary assignments.
///
/// Pattern: `_tmp = _a Op _b; _a = _tmp;`  →  `_a = _a Op _b;`
///
/// This removes one register move per loop iteration, which matters for
/// tight loops where Cranelift cannot do this itself.
use crate::mir::ir::*;

pub fn run(program: &mut MirProgram) {
    for function in program.functions.values_mut() {
        for block in &mut function.blocks {
            let instrs = &mut block.instrs;
            let mut i = 0;
            while i + 1 < instrs.len() {
                let fold = match (&instrs[i], &instrs[i + 1]) {
                    // Pattern: _tmp = expr; _dest = _tmp  →  _dest = expr
                    (
                        MirInstr::Assign(tmp, rvalue),
                        MirInstr::Assign(dest, Rvalue::Use(Operand::Local(src))),
                    ) if *src == *tmp && *tmp != *dest => {
                        Some((*dest, rvalue.clone()))
                    }
                    // Pattern: _tmp = expr; _dest = move(_tmp)  →  _dest = expr
                    (
                        MirInstr::Assign(tmp, rvalue),
                        MirInstr::Assign(dest, Rvalue::Move(src)),
                    ) if *src == *tmp && *tmp != *dest => {
                        Some((*dest, rvalue.clone()))
                    }
                    _ => None,
                };

                if let Some((dest, rvalue)) = fold {
                    instrs[i] = MirInstr::Assign(dest, rvalue);
                    instrs.remove(i + 1);
                } else {
                    i += 1;
                }
            }
        }
    }
}
