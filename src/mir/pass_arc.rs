use crate::mir::ir::*;
use crate::typecheck::TypeRef;
use std::collections::HashSet;

fn successors(terminator: &Terminator) -> Vec<usize> {
    match terminator {
        Terminator::Goto(target) => vec![target.0],
        Terminator::If {
            then_block,
            else_block,
            ..
        }
        | Terminator::IfCmp {
            then_block,
            else_block,
            ..
        } => vec![then_block.0, else_block.0],
        Terminator::Return(_) | Terminator::ReturnOwned(_) | Terminator::Unreachable => Vec::new(),
    }
}

/// Every local read by an instruction. Retain/Release are bookkeeping, not uses.
fn collect_reads(instr: &MirInstr, out: &mut HashSet<LocalId>) {
    fn op(o: &Operand, out: &mut HashSet<LocalId>) {
        if let Operand::Local(id) | Operand::Borrowed(id) = o {
            out.insert(*id);
        }
    }
    match instr {
        MirInstr::Assign(_, rv) => match rv {
            Rvalue::AllocateTuple(_, values) | Rvalue::MakeTask(_, _, values, _) => {
                for value in values { op(value, out); }
            }
            Rvalue::TupleField(base, _) | Rvalue::SliceLen(base)
            | Rvalue::SliceToStr(base) | Rvalue::Await(base) => op(base, out),
            Rvalue::MakeSlice { base, start, length, .. } => {
                op(base, out); op(start, out); op(length, out);
            }
            Rvalue::SliceGet(view, index) => { op(view, out); op(index, out); }
            Rvalue::Use(o) => op(o, out),
            Rvalue::Move(id) => {
                out.insert(*id);
            }
            Rvalue::BinaryOp(_, a, b) => {
                op(a, out);
                op(b, out);
            }
            Rvalue::CallDirect(_, args) | Rvalue::BuiltinCall(_, args) => {
                for a in args {
                    op(a, out);
                }
            }
            Rvalue::CallIndirect(c, args) => {
                op(c, out);
                for a in args {
                    op(a, out);
                }
            }
            Rvalue::MakeClosure(_, caps) | Rvalue::MakeStackClosure(_, caps) => {
                for c in caps {
                    op(c, out);
                }
            }
            Rvalue::FieldAccess(b, _) => op(b, out),
            Rvalue::AllocateArenaStruct(_, arena) => op(arena, out),
            Rvalue::SpawnThread(o) => op(o, out),
            _ => {}
        },
        MirInstr::AssignField { base, value, .. } => {
            out.insert(*base);
            op(value, out);
        }
        MirInstr::Retain(_) | MirInstr::Release(_) => {}
    }
}

/// Locals read by a block terminator.
fn collect_terminator_reads(t: &Terminator, out: &mut HashSet<LocalId>) {
    let mut op = |o: &Operand| {
        if let Operand::Local(id) | Operand::Borrowed(id) = o {
            out.insert(*id);
        }
    };
    match t {
        Terminator::If { cond, .. } => op(cond),
        Terminator::IfCmp { left, right, .. } => {
            op(left);
            op(right);
        }
        Terminator::Return(Some(o)) | Terminator::ReturnOwned(o) => op(o),
        _ => {}
    }
}

/// As `transfer_live`, but also removes the owners the rewriter will release at
/// the end of this block.
///
/// The end-of-block rule and the return-block rule are two separate release
/// sites, and the dataflow that feeds the second must know what the first will
/// do. When it did not, an owner released at the end of a loop body was still
/// "definitely live" on entry to the `return` block, which released it again --
/// a double free that ASan caught in lppsqlite's `sc_find`.
fn transfer_live_with_drops(
    instructions: &[MirInstr],
    mut live: HashSet<LocalId>,
    cleanup_locals: &HashSet<LocalId>,
    dropped: Option<&HashSet<LocalId>>,
) -> HashSet<LocalId> {
    for instruction in instructions {
        match instruction {
            MirInstr::Assign(destination, rvalue) => {
                match rvalue {
                    Rvalue::Move(source) => {
                        live.remove(source);
                    }
                    // Closure construction transfers the owned environment into
                    // the closure capsule; the capsule destructor releases it.
                    Rvalue::MakeClosure(_, captures) | Rvalue::MakeStackClosure(_, captures) => {
                        if let Some(Operand::Local(environment)) = captures.first() {
                            live.remove(environment);
                        }
                    }
                    Rvalue::AllocateTuple(_, values) | Rvalue::MakeTask(_, _, values, _) => {
                        for value in values {
                            if let Operand::Local(source) = value {
                                live.remove(source);
                            }
                        }
                    }
                    _ => {}
                }
                if cleanup_locals.contains(destination) {
                    live.insert(*destination);
                }
            }
            // Storing an owned local into a field hands the reference to the
            // parent, whose destructor will release it. The local therefore
            // stops being an owner here, exactly as the rewriter below models.
            //
            // This case used to be missing. While every block started from the
            // empty set that was invisible -- the local was never "live" long
            // enough for the omission to matter. Once the analysis was
            // corrected to start from TOP, the transferred local stayed in the
            // set and the return block released it a second time, after the
            // parent's destructor had already freed it: `Kennel(Dog(11))`
            // double-freed the inner Dog.
            MirInstr::AssignField {
                value: Operand::Local(source),
                ..
            } => {
                live.remove(source);
            }
            _ => {}
        }
    }
    if let Some(dropped) = dropped {
        for local in dropped {
            live.remove(local);
        }
    }
    live
}

/// Does this block assign `local` (i.e. create the reference it holds)?
fn block_defines(instructions: &[MirInstr], local: LocalId) -> bool {
    instructions
        .iter()
        .any(|instruction| matches!(instruction, MirInstr::Assign(dest, _) if *dest == local))
}

/// Insert ARC operations from explicit ownership information in MIR.
///
/// This pass is deliberately conservative. It calculates *definitely live*
/// owners with an intersection dataflow analysis, so it never emits a release
/// for a local that might be uninitialized on a branch. That may leave an
/// unsupported alias case allocated, but avoids the more serious failure of
/// dereferencing/freeing an uninitialized or moved value.
pub fn run_arc_insertion_pass(program: &mut MirProgram) {
    run_arc_insertion_pass_with_weak(program, &HashSet::new())
}

/// `weak_fields` are `(struct, field)` pairs demoted by `analysis::cyclebreak`.
/// Storing into one must NOT retain: the edge is non-owning, the destructor
/// will not release it, and retaining would leak the target forever -- exactly
/// the cycle the demotion exists to break.
pub fn run_arc_insertion_pass_with_weak(
    program: &mut MirProgram,
    weak_fields: &HashSet<(crate::typecheck::StructTypeId, String)>,
) {
    for function in program.functions.values_mut() {
        // Managed locals may be heap-backed ARC values or frame-local values
        // that will be rewritten by pass_escape. Keep both kinds in the
        // liveness analysis; the backend distinguishes ARC release from direct
        // stack-destructor cleanup. Borrowed parameters remain caller-owned and
        // are excluded.
        let arc_locals: HashSet<LocalId> = function
            .locals
            .iter()
            .filter(|local| {
                local.ownership.is_managed()
            })
            .map(|local| local.id)
            .collect();
        // `pass_escape` changes promoted custom locals to `Copy`. They still
        // have a lifetime that must be ended, but their cleanup is a direct call
        // to the generated destructor rather than an ARC release (there is no
        // header in front of a stack payload). Keep them in the same liveness
        // dataflow and let the backend distinguish the two Release meanings.
        let stack_locals: HashSet<LocalId> = function
            .locals
            .iter()
            .filter(|local| {
                local.ownership.is_copy()
                    && matches!(local.ty, TypeRef::Custom(_) | TypeRef::Function)
            })
            .map(|local| local.id)
            .collect();
        let cleanup_locals: HashSet<LocalId> = arc_locals
            .union(&stack_locals)
            .copied()
            .collect();

        if cleanup_locals.is_empty() {
            continue;
        }

        let block_count = function.blocks.len();
        let mut predecessors: Vec<Vec<usize>> = vec![Vec::new(); block_count];
        for block in &function.blocks {
            for successor in successors(&block.terminator) {
                if successor < block_count {
                    predecessors[successor].push(block.id.0);
                }
            }
        }

        // Blocks that lie on a cycle: reachable from a back edge's head and
        // able to reach its tail.
        let loop_blocks: HashSet<usize> = {
            let mut back_edges: Vec<(usize, usize)> = Vec::new();
            let mut state = vec![0u8; block_count];
            let mut stack = vec![(function.start_block.0, 0usize)];
            if function.start_block.0 < block_count {
                state[function.start_block.0] = 1;
                while let Some((b, next)) = stack.pop() {
                    let succs = successors(&function.blocks[b].terminator);
                    if next < succs.len() {
                        stack.push((b, next + 1));
                        let sc = succs[next];
                        if sc < block_count {
                            match state[sc] {
                                1 => back_edges.push((b, sc)),
                                0 => {
                                    state[sc] = 1;
                                    stack.push((sc, 0));
                                }
                                _ => {}
                            }
                        }
                    } else {
                        state[b] = 2;
                    }
                }
            }
            let mut in_loop: HashSet<usize> = HashSet::new();
            for (tail, head) in back_edges {
                let mut work = vec![tail];
                let mut seen: HashSet<usize> = HashSet::new();
                seen.insert(head);
                while let Some(b) = work.pop() {
                    if !seen.insert(b) {
                        continue;
                    }
                    for p in &predecessors[b] {
                        work.push(*p);
                    }
                }
                in_loop.extend(seen);
            }
            in_loop
        };

        // Backward liveness: `live_out[b]` is every ARC local that some path
        // leaving `b` can still read.
        //
        // This is what lets an owner be released at its true last use rather than
        // only at a `return`. The previous pass released solely at return
        // blocks, so anything allocated inside a loop body leaked once per
        // iteration -- and a value created on one arm of a branch inside a loop
        // leaked even with a back-edge rule, because it is not "definitely
        // live" at the join and so was invisible to the intersection analysis.
        let live_out: Vec<HashSet<LocalId>> = {
            let mut reads: Vec<HashSet<LocalId>> = Vec::with_capacity(block_count);
            let mut kills: Vec<HashSet<LocalId>> = Vec::with_capacity(block_count);
            for b in &function.blocks {
                let mut r: HashSet<LocalId> = HashSet::new();
                let mut k: HashSet<LocalId> = HashSet::new();
                for i in &b.instrs {
                    let mut used: HashSet<LocalId> = HashSet::new();
                    collect_reads(i, &mut used);
                    for u in used {
                        if !k.contains(&u) {
                            r.insert(u);
                        }
                    }
                    if let MirInstr::Assign(dest, _) = i {
                        k.insert(*dest);
                    }
                }
                let mut t: HashSet<LocalId> = HashSet::new();
                collect_terminator_reads(&b.terminator, &mut t);
                for u in t {
                    if !k.contains(&u) {
                        r.insert(u);
                    }
                }
                reads.push(r);
                kills.push(k);
            }

            let mut live_in: Vec<HashSet<LocalId>> = vec![HashSet::new(); block_count];
            let mut live_out: Vec<HashSet<LocalId>> = vec![HashSet::new(); block_count];
            let mut changed = true;
            while changed {
                changed = false;
                for b in (0..block_count).rev() {
                    let mut out: HashSet<LocalId> = HashSet::new();
                    for sc in successors(&function.blocks[b].terminator) {
                        if sc < block_count {
                            out.extend(live_in[sc].iter().copied());
                        }
                    }
                    // live_in = reads ∪ (live_out − kills)
                    let mut inn = reads[b].clone();
                    for l in &out {
                        if !kills[b].contains(l) {
                            inn.insert(*l);
                        }
                    }
                    if out != live_out[b] {
                        live_out[b] = out;
                        changed = true;
                    }
                    if inn != live_in[b] {
                        live_in[b] = inn;
                        changed = true;
                    }
                }
            }
            live_out
        };

        // Locals written in more than one block have a lifetime that spans
        // blocks (a loop-carried variable reassigned each iteration, or one
        // initialised before the loop and updated inside it). Releasing those
        // from a block-local rule is unsafe, so they stay with the existing
        // reassignment/return handling.
        let loop_carried: HashSet<LocalId> = {
            let mut seen_once: HashSet<LocalId> = HashSet::new();
            let mut multi: HashSet<LocalId> = HashSet::new();
            for b in &function.blocks {
                let mut in_this: HashSet<LocalId> = HashSet::new();
                for i in &b.instrs {
                    if let MirInstr::Assign(d, _) = i {
                        if cleanup_locals.contains(d) {
                            in_this.insert(*d);
                        }
                    }
                }
                for d in in_this {
                    if !seen_once.insert(d) {
                        multi.insert(d);
                    }
                }
            }
            multi
        };

        // `entry_live[block]` is an intersection over all predecessor exits, and
        // `block_dropped[block]` is the set the end-of-block rule will release.
        //
        // These two are mutually dependent: what a block drops depends on what
        // it owns on entry, and what a block owns on entry depends on what its
        // predecessors dropped. They are therefore solved together, to a fixed
        // point, instead of computing liveness once against instructions that do
        // not yet contain the releases the rewriter is about to insert.
        // `entry_live` is a MUST analysis: "this local is definitely an owner on
        // every path reaching this block". For an intersection dataflow the
        // correct initial value is TOP -- every candidate -- for all blocks
        // except the entry, which starts at BOTTOM because nothing is owned
        // before the function begins. Iterating intersections then shrinks
        // monotonically to the fixed point, and a local survives only if it is
        // genuinely present on every incoming path.
        //
        // Initialising every block to the empty set instead was a real bug, not
        // a conservative choice. At a loop header the intersection is taken
        // against the back edge, whose entry set is still empty on the first
        // sweep; that empties the header permanently and every successor
        // inherits the loss. An owner created *before* a loop was therefore
        // invisible after it, the return block emitted no release, and the
        // value leaked -- bounded, but real: three allocations for a captured
        // struct plus its closure capsule in any function that also has a loop.
        //
        // Starting from TOP cannot over-release: the fixpoint only removes
        // locals, so anything still present at the end is owned on all paths.
        let mut entry_live: Vec<HashSet<LocalId>> = vec![cleanup_locals.clone(); block_count];
        if function.start_block.0 < block_count {
            entry_live[function.start_block.0] = HashSet::new();
        }
        let mut block_dropped: Vec<HashSet<LocalId>> = vec![HashSet::new(); block_count];
        loop {
            let mut changed = true;
            while changed {
                changed = false;
                for block in &function.blocks {
                    if block.id == function.start_block {
                        continue;
                    }
                    let preds = &predecessors[block.id.0];
                    if preds.is_empty() {
                        // Unreachable block. It was initialised to TOP for the
                        // intersection above, but nothing flows into it, so TOP
                        // would claim it owns everything and the rewriter would
                        // emit releases for locals that were never created.
                        // Nothing reaches it at run time either way; empty is
                        // the only safe value.
                        if !entry_live[block.id.0].is_empty() {
                            entry_live[block.id.0] = HashSet::new();
                            changed = true;
                        }
                        continue;
                    }
                    let mut incoming = transfer_live_with_drops(
                        &function.blocks[preds[0]].instrs,
                        entry_live[preds[0]].clone(),
                        &cleanup_locals,
                        Some(&block_dropped[preds[0]]),
                    );
                    for predecessor in &preds[1..] {
                        let predecessor_exit = transfer_live_with_drops(
                            &function.blocks[*predecessor].instrs,
                            entry_live[*predecessor].clone(),
                            &cleanup_locals,
                            Some(&block_dropped[*predecessor]),
                        );
                        incoming.retain(|local| predecessor_exit.contains(local));
                    }
                    if incoming != entry_live[block.id.0] {
                        entry_live[block.id.0] = incoming;
                        changed = true;
                    }
                }
            }

            // Recompute what each block drops under the liveness just solved.
            let mut next_dropped: Vec<HashSet<LocalId>> = vec![HashSet::new(); block_count];
            for block in &function.blocks {
                if matches!(
                    &block.terminator,
                    Terminator::Return(_) | Terminator::ReturnOwned(_)
                ) || !loop_blocks.contains(&block.id.0)
                {
                    continue;
                }
                let exit = transfer_live_with_drops(
                    &block.instrs,
                    entry_live[block.id.0].clone(),
                    &cleanup_locals,
                    None,
                );
                let entry = &entry_live[block.id.0];
                for local in exit {
                    // Only owners *created in this block* are dropped here.
                    // An owner that merely flows through a loop header still
                    // belongs to whoever created it, and stealing it here left
                    // the function's return block with nothing to release --
                    // a leak of one allocation per such value.
                    if !entry.contains(&local)
                        && !live_out[block.id.0].contains(&local)
                        && !loop_carried.contains(&local)
                        && block_defines(&block.instrs, local)
                    {
                        next_dropped[block.id.0].insert(local);
                    }
                }
            }
            if next_dropped == block_dropped {
                break;
            }
            block_dropped = next_dropped;
        }

        for block in &mut function.blocks {
            let mut live = entry_live[block.id.0].clone();
            let original = std::mem::take(&mut block.instrs);
            let mut rewritten = Vec::with_capacity(original.len() + cleanup_locals.len());

            for instruction in original {
                match &instruction {
                    MirInstr::Assign(destination, rvalue) => {
                        // Copy everything needed from the borrowed instruction
                        // before moving it into the rewritten block.
                        let destination = *destination;
                        let transferred_sources: Vec<LocalId> = match rvalue {
                            Rvalue::Move(source) => vec![*source],
                            // The environment reference becomes owned by the
                            // ARC closure capsule and is released by its
                            // destructor, not by the creating scope.
                            Rvalue::MakeClosure(_, captures)
                            | Rvalue::MakeStackClosure(_, captures) => captures
                                .first()
                                .and_then(|operand| match operand {
                                    Operand::Local(environment) => Some(vec![*environment]),
                                    _ => None,
                                })
                                .unwrap_or_default(),
                            Rvalue::AllocateTuple(_, values)
                            | Rvalue::MakeTask(_, _, values, _) => values
                                .iter()
                                .filter_map(|value| match value {
                                    Operand::Local(source) => Some(*source),
                                    _ => None,
                                })
                                .collect(),
                            _ => Vec::new(),
                        };
                        let borrowed_source = match rvalue {
                            Rvalue::Use(Operand::Borrowed(source)) => Some(*source),
                            _ => None,
                        };

                        // Reassignment drops the old owned reference.
                        //
                        // ORDER MATTERS. Releasing *before* the instruction is
                        // only safe when the new value does not derive from the
                        // old one. For a self-referential update such as
                        // `s = s + "ab"`, lowered to
                        // `_0 = lpp_str_concat(borrow(_0), "ab")`, an early
                        // release frees the buffer the call is about to read --
                        // a use-after-free that returned a truncated string
                        // ("ab" instead of "ababab") rather than crashing.
                        //
                        // When the destination is also read by this
                        // instruction, the old reference is therefore released
                        // *after* the new value has been computed. The release
                        // targets the old pointer, so it is materialised into a
                        // temporary first; otherwise the release would decrement
                        // the freshly assigned object instead.
                        let mut reads = HashSet::new();
                        collect_reads(&instruction, &mut reads);
                        let self_referential = reads.contains(&destination);

                        let mut deferred_release: Option<LocalId> = None;
                        if cleanup_locals.contains(&destination) && live.remove(&destination) {
                            if stack_locals.contains(&destination) {
                                // A promoted object has no ARC header. `Release`
                                // is intentionally used as the cleanup opcode,
                                // and the backend turns it into a direct call to
                                // the type-specific destructor.
                                rewritten.push(MirInstr::Release(destination));
                            } else if self_referential {
                                deferred_release = Some(destination);
                            } else {
                                rewritten.push(MirInstr::Release(destination));
                            }
                        }

                        if let Some(old) = deferred_release {
                            // Stash the old pointer, run the instruction, then
                            // release what the local used to hold.
                            let saved = LocalId(function.locals.len());
                            function.locals.push(LocalDecl {
                                id: saved,
                                ty: function.locals[old.0].ty.clone(),
                                is_mut: false,
                                ownership: Ownership::Borrowed,
                                binding_id: None,
                                debug_name: None,
                            });
                            rewritten.push(MirInstr::Assign(
                                saved,
                                Rvalue::Use(Operand::Borrowed(old)),
                            ));
                            rewritten.push(instruction);
                            rewritten.push(MirInstr::Release(saved));
                        } else {
                            rewritten.push(instruction);
                        }

                        for source in transferred_sources {
                            live.remove(&source);
                        }
                        if cleanup_locals.contains(&destination) {
                            live.insert(destination);
                            // A borrow becomes an additional owner at this
                            // assignment boundary ONLY when the destination
                            // is itself an ARC-managed local. Stack payloads
                            // have no header and are never retained.
                            if arc_locals.contains(&destination) && borrowed_source.is_some() {
                                rewritten.push(MirInstr::Retain(destination));
                            }
                        }
                    }
                    MirInstr::AssignField {
                        base,
                        field,
                        value: Operand::Borrowed(source),
                    } if function.locals[source.0].ty.is_managed() =>
                    {
                        let source = *source;
                        let is_weak = match &function.locals[base.0].ty {
                            TypeRef::Custom(sid) => {
                                weak_fields.contains(&(*sid, field.clone()))
                            }
                            _ => false,
                        };
                        rewritten.push(instruction);
                        // Struct fields are owning edges under the current ARC
                        // model, EXCEPT ones demoted to break a cycle.
                        if !is_weak {
                            rewritten.push(MirInstr::Retain(source));
                        }
                    }
                    // Storing an *owned* local into a field hands that field the
                    // reference -- the field edge is owning, and the parent's
                    // destructor will release it. The local must therefore stop
                    // being an owner here.
                    //
                    // Without this, nested construction `Kennel(Dog(1))`
                    // released the inner object twice: once via the parent's
                    // destructor and once at scope exit. The program printed the
                    // right answer and then SIGSEGV'd on the double free.
                    MirInstr::AssignField {
                        base,
                        field,
                        value: Operand::Local(source),
                    } if arc_locals.contains(source) => {
                        let source = *source;
                        let is_weak = match &function.locals[base.0].ty {
                            TypeRef::Custom(sid) => {
                                weak_fields.contains(&(*sid, field.clone()))
                            }
                            _ => false,
                        };
                        rewritten.push(instruction);
                        // An owning field store transfers the reference, so the
                        // local stops being an owner. A weak field store does
                        // not transfer anything, so the local keeps its
                        // reference and is released normally at scope exit.
                        if !is_weak {
                            live.remove(&source);
                        }
                    }
                    _ => rewritten.push(instruction),
                }
            }

            if let Terminator::Return(_) | Terminator::ReturnOwned(_) = &block.terminator {
                let returned_local = match &block.terminator {
                    Terminator::ReturnOwned(Operand::Local(local)) => Some(*local),
                    _ => None,
                };
                // Release in REVERSE CREATION ORDER, deterministically.
                //
                // `live` is a HashSet, so iterating it directly emitted
                // releases in an order that depended on the hash seed and on
                // how many locals the function happened to have. That is not
                // merely untidy: for a chain `e -> d -> c` built by
                // `e.next = d; d.next = c`, releasing the head first drops the
                // whole chain through the generated destructors, while
                // releasing a tail element first leaves the head holding a
                // reference and the chain is stranded. The old order passed by
                // luck, and any unrelated change to the local numbering could
                // silently flip it -- which is exactly what happened when the
                // return rule started consulting escape analysis.
                //
                // Ascending LocalId, i.e. creation order.
                //
                // For `e.next = d; d.next = c` the cycle breaker has already
                // demoted the back edge, so `e` holds a strong reference to `d`
                // and `d` to `c`. Releasing `c` first drops its local reference
                // while `d` still owns it, and the object dies later when `d`
                // is released and runs its destructor -- every object is
                // reclaimed. Releasing the head `e` first destroys `d` and `c`
                // through the destructor chain, and the subsequent releases of
                // `d` and `c` then act on already-freed memory, which the ARC
                // pass compensates for by leaving them allocated.
                //
                // Creation order is the order that reclaims everything, and it
                // is also stable: sorting removes the dependence on HashSet
                // iteration order, which previously made this correct only by
                // luck and flipped as soon as local numbering shifted.
                let mut to_release: Vec<LocalId> = live
                    .iter()
                    .copied()
                    .filter(|local| Some(*local) != returned_local)
                    .collect();
                to_release.sort_by_key(|local| local.0);
                for local in to_release {
                    rewritten.push(MirInstr::Release(local));
                }
            } else {
                // Owners created inside a loop body die at the end of the
                // iteration. The entry-live analysis is an intersection and one
                // predecessor of the loop header is the pre-loop edge, where
                // these are not yet initialized -- so they are never
                // "definitely live" on entry, the reassignment-release never
                // fires, and every iteration leaked its allocation.
                //
                // Release, at the end of the block that created it, any owner
                // that this block created and that no successor can still read.
                // Values already live on entry are loop-carried and handled by
                // the reassignment path; releasing those here would free a
                // reference the next iteration still uses.
                // Owners still held at the end of this block that no
                // successor can read are dead here. Releasing them at this
                // point -- rather than only at a `return` -- is what stops a
                // loop body leaking one allocation per iteration, including
                // values created on just one arm of a branch inside the loop.
                //
                // Restricted to owners created *in this block*, and only when
                // this block lies on a cycle.
                //
                // Two constraints make this narrow on purpose:
                //
                //  * `entry_live` is a definitely-live intersection while
                //    `live_out` is a may-be-read union. Mixing them lets a
                //    local be released here *and* again at a return block that
                //    still believes it owns it -- a double free.
                //  * Outside a loop the existing return-block rule already
                //    releases every owner exactly once, so there is nothing to
                //    gain and a double free to risk.
                //
                // Inside a loop the return rule fires only once for a value
                // that is allocated once per iteration, which is the leak this
                // exists to close. Releasing it here also removes it from
                // `live`, so the return block no longer sees it as an owner.
                // Exactly the set the fixpoint above already accounted for, so
                // successor blocks agree this local is no longer owned.
                let mut to_release: Vec<LocalId> =
                    block_dropped[block.id.0].iter().copied().collect();
                to_release.sort_by_key(|l| l.0); // deterministic output
                for local in to_release {
                    if live.remove(&local) {
                        rewritten.push(MirInstr::Release(local));
                    }
                }
            }

            block.instrs = rewritten;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typecheck::TypeRef;
    use std::collections::HashMap;

    fn arc_local(i: usize) -> LocalDecl {
        LocalDecl {
            id: LocalId(i),
            ty: TypeRef::Custom(crate::typecheck::StructTypeId(0)),
            is_mut: true,
            debug_name: None,
            binding_id: None,
            ownership: Ownership::Owned,
        }
    }

    fn run_one(blocks: Vec<MirBlock>, nlocals: usize) -> MirFunction {
        let f = MirFunction {
            id: FuncId(0),
            name: "main".to_string(),
            params: Vec::new(),
            locals: (0..nlocals).map(arc_local).collect(),
            blocks,
            start_block: BlockId(0),
            return_type: TypeRef::Void,
            is_async: false,
        };
        let mut functions = HashMap::new();
        functions.insert(FuncId(0), f);
        let mut p = MirProgram { functions };
        run_arc_insertion_pass(&mut p);
        p.functions.remove(&FuncId(0)).unwrap()
    }

    fn releases_of(f: &MirFunction, local: LocalId) -> usize {
        f.blocks
            .iter()
            .flat_map(|b| b.instrs.iter())
            .filter(|i| matches!(i, MirInstr::Release(l) if *l == local))
            .count()
    }

    #[test]
    fn loop_body_allocation_is_released_each_iteration() {
        // bb1 is a loop body allocating _0 and jumping back to itself. Without
        // a release inside the body the allocation leaks once per iteration:
        // the return-block rule fires only once, for the final value.
        let blocks = vec![
            MirBlock {
                id: BlockId(0),
                instrs: vec![],
                terminator: Terminator::Goto(BlockId(1)),
            },
            MirBlock {
                id: BlockId(1),
                instrs: vec![MirInstr::Assign(
                    LocalId(0),
                    Rvalue::AllocateArcStruct(TypeRef::Custom(crate::typecheck::StructTypeId(0))),
                )],
                terminator: Terminator::If {
                    cond: Operand::Bool(true),
                    then_block: BlockId(1),
                    else_block: BlockId(2),
                },
            },
            MirBlock {
                id: BlockId(2),
                instrs: vec![],
                terminator: Terminator::Return(None),
            },
        ];
        let f = run_one(blocks, 1);
        let in_body = f.blocks[1]
            .instrs
            .iter()
            .filter(|i| matches!(i, MirInstr::Release(LocalId(0))))
            .count();
        assert!(in_body >= 1, "loop-body allocation must be released in the body");
    }

    #[test]
    fn straight_line_owner_is_released_exactly_once() {
        // The block-local rule must not double up with the return-block rule.
        // Releasing here *and* at the return was a double free.
        let blocks = vec![
            MirBlock {
                id: BlockId(0),
                instrs: vec![MirInstr::Assign(
                    LocalId(0),
                    Rvalue::AllocateArcStruct(TypeRef::Custom(crate::typecheck::StructTypeId(0))),
                )],
                terminator: Terminator::Goto(BlockId(1)),
            },
            MirBlock {
                id: BlockId(1),
                instrs: vec![],
                terminator: Terminator::Return(None),
            },
        ];
        let f = run_one(blocks, 1);
        assert_eq!(
            releases_of(&f, LocalId(0)),
            1,
            "a straight-line owner must be released exactly once"
        );
    }

    #[test]
    fn returned_owner_is_not_released() {
        let blocks = vec![MirBlock {
            id: BlockId(0),
            instrs: vec![MirInstr::Assign(
                LocalId(0),
                Rvalue::AllocateArcStruct(TypeRef::Custom(crate::typecheck::StructTypeId(0))),
            )],
            terminator: Terminator::ReturnOwned(Operand::Local(LocalId(0))),
        }];
        let f = run_one(blocks, 1);
        assert_eq!(
            releases_of(&f, LocalId(0)),
            0,
            "the returned value is handed to the caller, not released"
        );
    }
}
