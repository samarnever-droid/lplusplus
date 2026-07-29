//! Value-by-default: move provably frame-local structs off the heap.
//!
//! This pass used to carry its own use-scan -- a private, hand-written answer to
//! "can this local escape?" that duplicated, in a different idiom, the question
//! `analysis::escape` was already asking over the AST and `pass_arc` was asking
//! again over ownership flags. Three partial answers to one question is how the
//! double-free and the nondeterministic release order happened.
//!
//! The scan now lives in `escape_solver`, which computes one fact per local over
//! MIR with a compiler-enforced exhaustive match. This pass is what remains:
//! read the fact, rewrite the allocation.

use super::escape_solver::{EscapeFacts, Storage};
use super::ir::*;
use std::collections::{HashMap, HashSet};

/// Reported by `--dump-escape`.
#[derive(Debug, Default, Clone)]
pub struct EscapeStats {
    pub promoted: usize,
    pub considered: usize,
}

/// Rewrite `AllocateArcStruct` to `AllocateStackStruct` for every local the
/// solver proved cannot outlive its frame.
pub fn run(program: &mut MirProgram, facts: &EscapeFacts) -> EscapeStats {
    let mut stats = EscapeStats::default();

    for (fid, function) in program.functions.iter_mut() {
        let Some(fn_facts) = facts.functions.get(fid) else {
            continue;
        };

        // Lowering builds a struct into a temporary and then moves it into the
        // named local: `_1 = alloc; _1.x = ...; _0 = move(_1)`. That pair is one
        // object with one lifetime. The solver already links them -- the move
        // makes the temporary's fate follow the destination's -- so both ends
        // agree, and this map only exists so the ownership flag is cleared on
        // both halves.
        let mut moved_into: HashMap<LocalId, LocalId> = HashMap::new();
        let mut move_count: HashMap<LocalId, usize> = HashMap::new();
        for block in &function.blocks {
            for instruction in &block.instrs {
                if let MirInstr::Assign(dest, Rvalue::Move(src)) = instruction {
                    *move_count.entry(*src).or_insert(0) += 1;
                    moved_into.insert(*src, *dest);
                }
            }
        }
        moved_into.retain(|src, _| move_count.get(src).copied().unwrap_or(0) == 1);

        let frame_local = |id: LocalId| {
            fn_facts
                .locals
                .get(id.0)
                .copied()
                .unwrap_or(Storage::Owned)
                == Storage::Frame
        };

        let mut promote: HashSet<LocalId> = HashSet::new();
        for block in &function.blocks {
            for instruction in &block.instrs {
                if let MirInstr::Assign(dest, Rvalue::AllocateArcStruct(_)) = instruction {
                    stats.considered += 1;
                    // Both halves of a construct-then-move pair must be frame
                    // locals; the object is one lifetime.
                    let dest_ok = frame_local(*dest);
                    let pair_ok = moved_into
                        .get(dest)
                        .map(|named| frame_local(*named))
                        .unwrap_or(true);
                    if dest_ok && pair_ok {
                        promote.insert(*dest);
                        if let Some(named) = moved_into.get(dest) {
                            promote.insert(*named);
                        }
                    }
                }
            }
        }

        if promote.is_empty() {
            continue;
        }

        for block in &mut function.blocks {
            for instruction in &mut block.instrs {
                if let MirInstr::Assign(dest, rvalue) = instruction {
                    if promote.contains(dest) {
                        if let Rvalue::AllocateArcStruct(ty) = rvalue {
                            *rvalue = Rvalue::AllocateStackStruct(ty.clone());
                            stats.promoted += 1;
                        }
                    }
                }
            }
        }

        // A stack slot has no ARC header, so no later pass may treat it as an
        // owner and emit retain/release against it.
        for local in &mut function.locals {
            if promote.contains(&local.id) {
                local.ownership = Ownership::Copy;
            }
        }
    }

    stats
}
