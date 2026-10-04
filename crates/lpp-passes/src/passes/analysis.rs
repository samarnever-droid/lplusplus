//! Shared analysis for the 4F passes.
//!
//! The passes rewrite validated MIR in place. The one piece of
//! information they need that is not local to a single instruction is
//! which locals hold a single primitive constant for their whole
//! function; that analysis is computed once per function and shared
//! by `const-prop` and `branch-fold`.

use std::collections::BTreeMap;

use lpp_mir::{
    Constant, InstructionKind, MirFunction, MirLocalId, MirLocalKind, MirProgram, Operand, Rvalue,
};

/// A local is a **single-def constant** when every one of the
/// following holds:
///
/// - its kind is `User` or `Temporary` (parameters and captures are
///   excluded — a capture's value can be written back at closure exit,
///   a parameter's value arrives per call);
/// - the function assigns it exactly once, and that one assign is
///   `Use(Constant(c))`;
/// - `c` is primitive: `Integer`, `FloatBits`, `Bool`, or
///   `Character`. A `String` constant is a heap node and never
///   qualifies (rewriting a read of it would change ownership traffic);
/// - the local is not the root of any `MirPlace` (a place root can be
///   written through a store, which the assign count does not see).
#[must_use]
pub fn single_def_constants(
    program: &MirProgram,
    function: &MirFunction,
) -> BTreeMap<MirLocalId, Constant> {
    let place_roots: std::collections::BTreeSet<MirLocalId> =
        program.places().map(|(_, place)| place.root).collect();

    // Count assigns per local and remember the one constant def, if
    // there is exactly one.
    let mut defs: BTreeMap<MirLocalId, (usize, Option<Constant>)> = BTreeMap::new();
    for &block_id in program.function_blocks(function) {
        let Some(block) = program.block(block_id) else {
            continue;
        };
        for &instruction_id in program.block_instructions(block) {
            let Some(instruction) = program.instruction(instruction_id) else {
                continue;
            };
            let InstructionKind::Assign { target, value } = &instruction.kind else {
                continue;
            };
            let entry = defs.entry(*target).or_insert((0, None));
            entry.0 += 1;
            if entry.0 == 1 {
                if let Rvalue::Use(Operand::Constant(constant)) = value {
                    if is_primitive_constant(constant) {
                        entry.1 = Some(*constant);
                    }
                }
            } else {
                entry.1 = None;
            }
        }
    }

    defs.into_iter()
        .filter_map(|(local, (count, constant))| {
            let Some(constant) = constant else {
                return None;
            };
            if count != 1 {
                return None;
            }
            let kind = program.local(local)?.kind;
            if !matches!(kind, MirLocalKind::User | MirLocalKind::Temporary) {
                return None;
            }
            if place_roots.contains(&local) {
                return None;
            }
            Some((local, constant))
        })
        .collect()
}

/// Constants whose operand rewrite is ownership-neutral. String
/// constants are heap nodes and must never be treated as plain
/// values.
#[must_use]
pub const fn is_primitive_constant(constant: &Constant) -> bool {
    !matches!(constant, Constant::String { .. })
}
