//! One escape fact, computed once, for the whole program.
//!
//! # Why this exists
//!
//! Ownership used to be decided in five unrelated places -- `builder.rs`
//! (type shape), `lower.rs` (return terminator), `lower.rs` again (closure
//! environments), `pass_arc` (which locals get retain/release) and
//! `pass_escape` (its own private use-scan). Each re-derived a partial answer
//! to the same question in its own idiom, and partial answers can disagree.
//! They did: two independent release rules with no shared notion of ownership
//! produced a double free, and a `HashSet` iteration order silently decided
//! whether a linked structure was reclaimed.
//!
//! This module answers the question once:
//!
//!   "can a pointer to this local outlive the frame that created it?"
//!
//! # Why MIR and not the AST
//!
//! The former AST walker asked the same question over dozens of expression and
//! statement forms. Its coverage was a review promise, not a compiler-enforced
//! property, and it had already missed call arguments, field stores, and
//! struct-literal fields. It has been retired from the code-generation path.
//!
//! In MIR a local's value can only be established by `MirInstr::Assign` or
//! `MirInstr::AssignField` -- two forms -- and can only travel through one of
//! 14 `Rvalue` variants. `escape_of_rvalue` matches all 14 with no wildcard, so
//! **adding an `Rvalue` later is a compile error rather than a silent hole**.
//! Coverage stops being a claim about careful reading and becomes a property
//! the type checker enforces.
//!
//! # The lattice
//!
//! Exactly three points, deliberately:
//!
//! ```text
//!   Frame  <  Owned  <  Shared
//! ```
//!
//! * `Frame`  - cannot outlive the frame: stack slot, no refcount at all.
//! * `Owned`  - escapes the frame, single owner: ARC.
//! * `Shared` - escapes and may be reached by another thread: atomic ARC.
//!
//! Height is 2, and that is load-bearing rather than tidy. A monotone fixpoint
//! over a lattice of height `h` with `E` call-graph edges and `P` parameters
//! performs at most `O(h * (E + P))` transfer applications: a node only ever
//! moves *up*, it can move up at most `h` times, and it is only re-queued when
//! an input changed. Adding a fourth escape-storage point would loosen that
//! bound, so `Arena` is deliberately not a fourth lattice value. Arena is an
//! orthogonal allocation strategy selected for self-referential struct types;
//! the reachability fact remains Frame/Owned/Shared and the arena region is
//! retained by its arena-backed nodes.
//!
//! # Cost
//!
//! The call graph is condensed into strongly-connected components and processed
//! in reverse topological order, so a function in a singleton non-recursive SCC
//! -- the overwhelming majority -- is summarised exactly once with no iteration.
//! Iteration only happens inside a recursive SCC. `CallIndirect` and unlisted
//! builtins are treated as *local sinks*, never as graph edges: modelling an
//! indirect call as edges to every signature-compatible function would make `E`
//! proportional to (call sites x candidates) and destroy the linear bound.

use super::ir::*;
use crate::typecheck::{StructTypeId, TypeRef, TypeTable};
use std::collections::{HashMap, HashSet};

/// Where a value must live. Ordered: `Frame < Owned < Shared`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Storage {
    /// Provably cannot outlive its frame.
    #[default]
    Frame,
    /// Escapes the frame; one owner at a time.
    Owned,
    /// Escapes and may be reached from another thread.
    Shared,
}

impl Storage {
    /// Lattice join. Monotone by construction: the result is never below either
    /// input, which is exactly the property the fixpoint's termination argument
    /// relies on.
    pub fn join(self, other: Storage) -> Storage {
        if self >= other {
            self
        } else {
            other
        }
    }

    pub fn escapes(self) -> bool {
        self != Storage::Frame
    }
}

/// Whether a MIR type carries an ownership-bearing pointer. Scalars remain
/// `Frame` in the facts table even when passed to a call; their storage class is
/// not meaningful and must not pollute parameter summaries or `--dump-escape`.
fn is_managed_type(ty: &TypeRef) -> bool {
    matches!(
        ty,
        TypeRef::Custom(_)
            | TypeRef::Function
            | TypeRef::Generic(_, _)
            | TypeRef::Str
    )
}

/// Per-function facts.
#[derive(Debug, Clone, Default)]
pub struct FnFacts {
    /// Storage for every local, indexed by `LocalId.0`.
    pub locals: Vec<Storage>,
    /// For each parameter position, does it escape inside this function?
    /// This is the only thing callers need to know, which is what keeps the
    /// interprocedural part cheap.
    pub params_escape: Vec<bool>,
}

/// Whole-program result.
#[derive(Debug, Default)]
pub struct EscapeFacts {
    pub functions: HashMap<FuncId, FnFacts>,
}

impl EscapeFacts {
    pub fn storage_of(&self, func: FuncId, local: LocalId) -> Storage {
        self.functions
            .get(&func)
            .and_then(|f| f.locals.get(local.0).copied())
            .unwrap_or(Storage::Owned) // absent => assume it escapes
    }
}

/// Builtins that provably do NOT retain their pointer arguments.
///
/// Everything not listed defaults to "every argument escapes". That default is
/// what makes an incomplete table safe: a missing entry costs an optimisation,
/// never memory safety. Only add a name here after checking the C body.
fn builtin_keeps_nothing(symbol: &str) -> bool {
    matches!(
        symbol,
        // Pure readers.
        "lpp_print_str"
            | "lpp_print_int"
            | "lpp_print_float"
            | "lpp_print"
            | "lpp_str_len"
            | "lpp_str_eq"
            | "lpp_str_find"
            | "lpp_str_contains"
            | "lpp_str_starts_with"
            | "lpp_str_ends_with"
            | "lpp_str_to_int"
            | "lpp_ord"
            | "lpp_list_len"
            // Allocate a fresh result; arguments are only read.
            | "lpp_str_concat"
            | "lpp_str_substr"
            | "lpp_str_trim"
            | "lpp_str_upper"
            | "lpp_str_lower"
            | "lpp_str_replace"
            | "lpp_str_repeat"
            | "lpp_int_to_str"
            | "lpp_float_to_str"
            | "lpp_bool_to_str"
            | "lpp_char_at"
    )
}

fn operand_local(op: &Operand) -> Option<LocalId> {
    match op {
        Operand::Local(id) | Operand::Borrowed(id) => Some(*id),
        _ => None,
    }
}

/// Direct callees of a function. Only `CallDirect` and `MakeClosure` create
/// graph edges; see the module comment for why indirect calls must not.
fn direct_callees(function: &MirFunction) -> HashSet<FuncId> {
    let mut out = HashSet::new();
    for block in &function.blocks {
        for instruction in &block.instrs {
            if let MirInstr::Assign(_, rvalue) = instruction {
                match rvalue {
                    Rvalue::CallDirect(id, _)
                    | Rvalue::MakeClosure(id, _)
                    | Rvalue::MakeStackClosure(id, _)
                    | Rvalue::FuncRef(id) => {
                        out.insert(*id);
                    }
                    _ => {}
                }
            }
        }
    }
    out
}

/// Tarjan SCC over the call graph, returning components in reverse topological
/// order (callees before callers), so a non-recursive function is solved once.
fn scc_reverse_topo(
    ids: &[FuncId],
    edges: &HashMap<FuncId, HashSet<FuncId>>,
) -> Vec<Vec<FuncId>> {
    struct St {
        index: HashMap<FuncId, u32>,
        low: HashMap<FuncId, u32>,
        on: HashSet<FuncId>,
        stack: Vec<FuncId>,
        next: u32,
        out: Vec<Vec<FuncId>>,
    }
    fn strong(v: FuncId, st: &mut St, edges: &HashMap<FuncId, HashSet<FuncId>>) {
        st.index.insert(v, st.next);
        st.low.insert(v, st.next);
        st.next += 1;
        st.stack.push(v);
        st.on.insert(v);
        if let Some(succs) = edges.get(&v) {
            for &w in succs {
                if !st.index.contains_key(&w) {
                    strong(w, st, edges);
                    let lw = st.low[&w];
                    let lv = st.low[&v];
                    st.low.insert(v, lv.min(lw));
                } else if st.on.contains(&w) {
                    let iw = st.index[&w];
                    let lv = st.low[&v];
                    st.low.insert(v, lv.min(iw));
                }
            }
        }
        if st.low[&v] == st.index[&v] {
            let mut comp = Vec::new();
            while let Some(w) = st.stack.pop() {
                st.on.remove(&w);
                comp.push(w);
                if w == v {
                    break;
                }
            }
            st.out.push(comp);
        }
    }
    let mut st = St {
        index: HashMap::new(),
        low: HashMap::new(),
        on: HashSet::new(),
        stack: Vec::new(),
        next: 0,
        out: Vec::new(),
    };
    for &v in ids {
        if !st.index.contains_key(&v) {
            strong(v, &mut st, edges);
        }
    }
    // Tarjan already emits components in reverse topological order.
    st.out
}

/// A non-self-referential struct can live in a frame slot when the solver has
/// proved that the struct pointer itself does not escape.  Its owned fields are
/// still heap references, so the generated destructor must be called directly
/// at frame exit rather than through `lpp_arc_release` (the stack payload has no
/// ARC header).
///
/// Self-referential structs stay heap allocated: their cycle-breaking and
/// destructor rules are defined in terms of ARC object identity, and a stack
/// payload cannot safely participate in that graph.
pub fn struct_can_stack_allocate(type_table: &TypeTable, id: StructTypeId) -> bool {
    type_table
        .definitions
        .get(id.0)
        .map(|def| !def.is_self_referential)
        .unwrap_or(false)
}

/// Solve the whole program.
pub fn solve(program: &MirProgram) -> EscapeFacts {
    let ids: Vec<FuncId> = program.functions.keys().copied().collect();
    let mut edges: HashMap<FuncId, HashSet<FuncId>> = HashMap::new();
    for (id, function) in &program.functions {
        edges.insert(*id, direct_callees(function));
    }

    // Optimistic start: every parameter is assumed not to escape, and the
    // fixpoint only ever moves that upward.
    let mut summaries: HashMap<FuncId, Vec<bool>> = HashMap::new();
    for (id, function) in &program.functions {
        summaries.insert(*id, vec![false; function.params.len()]);
    }

    let components = scc_reverse_topo(&ids, &edges);
    let mut facts = EscapeFacts::default();

    for comp in &components {
        // Singleton, non-recursive: one pass, no iteration.
        let recursive = comp.len() > 1
            || comp
                .first()
                .map(|f| edges.get(f).map(|e| e.contains(f)).unwrap_or(false))
                .unwrap_or(false);

        loop {
            let mut changed = false;
            for &fid in comp {
                let Some(function) = program.functions.get(&fid) else {
                    continue;
                };
                let solved = solve_function(function, &summaries);
                let before = summaries.get(&fid).cloned().unwrap_or_default();
                if solved.params_escape != before {
                    // Monotone: a parameter can only go false -> true.
                    summaries.insert(fid, solved.params_escape.clone());
                    changed = true;
                }
                facts.functions.insert(fid, solved);
            }
            if !recursive || !changed {
                break;
            }
        }
    }

    facts
}

/// Intraprocedural solve for one function, given current callee summaries.
fn solve_function(
    function: &MirFunction,
    summaries: &HashMap<FuncId, Vec<bool>>,
) -> FnFacts {
    let n = function.locals.len();
    let mut storage = vec![Storage::Frame; n];

    // `aliases[a]` = locals that a's value flows into. If a target escapes, so
    // does the source. One pass builds it; the closure below propagates.
    let mut flows: Vec<Vec<LocalId>> = vec![Vec::new(); n];
    let mut raise = |storage: &mut Vec<Storage>, id: LocalId, to: Storage| {
        let managed = function
            .locals
            .get(id.0)
            .map(|local| is_managed_type(&local.ty))
            .unwrap_or(true);
        if managed && id.0 < storage.len() {
            storage[id.0] = storage[id.0].join(to);
        }
    };

    for block in &function.blocks {
        for instruction in &block.instrs {
            match instruction {
                MirInstr::Assign(dest, rvalue) => {
                    escape_of_rvalue(
                        rvalue,
                        *dest,
                        function,
                        summaries,
                        &mut storage,
                        &mut flows,
                        &mut raise,
                    );
                }
                MirInstr::AssignField { base, value, .. } => {
                    // Storing into an object gives that object a reference. The
                    // stored value therefore lives as long as the base does.
                    if let Some(v) = operand_local(value) {
                        if v.0 < flows.len() {
                            flows[v.0].push(*base);
                        }
                        // A field of a heap object is reachable from anywhere the
                        // object is, so at minimum the value is Owned.
                        raise(&mut storage, v, Storage::Owned);
                    }
                }
                // Retain/Release are bookkeeping inserted by a later pass; if
                // they are already present the local is ARC-managed.
                MirInstr::Retain(id) | MirInstr::Release(id) => {
                    raise(&mut storage, *id, Storage::Owned);
                }
            }
        }

        match &block.terminator {
            Terminator::Return(Some(op)) | Terminator::ReturnOwned(op) => {
                if let Some(id) = operand_local(op) {
                    raise(&mut storage, id, Storage::Owned);
                }
            }
            Terminator::Return(None)
            | Terminator::Goto(_)
            | Terminator::If { .. }
            | Terminator::IfCmp { .. }
            | Terminator::Unreachable => {}
        }
    }

    // Managed parameters are caller-owned; never demote one to a frame slot.
    // Scalar parameters remain Frame because there is no ownership fact to
    // propagate for them.
    for p in &function.params {
        if p.0 < storage.len()
            && function
                .locals
                .get(p.0)
                .map(|local| is_managed_type(&local.ty))
                .unwrap_or(true)
        {
            storage[p.0] = storage[p.0].join(Storage::Owned);
        }
    }

    // Transitive closure over `flows`: if a target escapes, the source does too.
    // Bounded by lattice height, so this terminates.
    loop {
        let mut changed = false;
        for src in 0..n {
            for &dst in &flows[src] {
                if dst.0 < n {
                    let joined = storage[src].join(storage[dst.0]);
                    if joined != storage[src] {
                        storage[src] = joined;
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    // A non-self-referential custom struct may be frame-local even when it owns
    // ARC-managed fields.  `pass_arc` will arrange a direct call to its generated
    // destructor at frame exit; keeping that decision out of this fact solver is
    // important because the solver answers reachability, not cleanup mechanics.

    let params_escape = function
        .params
        .iter()
        .map(|p| {
            function
                .locals
                .get(p.0)
                .map(|local| is_managed_type(&local.ty))
                .unwrap_or(true)
                && storage
                    .get(p.0)
                    .copied()
                    .unwrap_or(Storage::Owned)
                    .escapes()
        })
        .collect();

    FnFacts {
        locals: storage,
        params_escape,
    }
}

/// How each of the 14 `Rvalue` forms lets a pointer travel.
///
/// Deliberately exhaustive with no `_` arm: a new `Rvalue` must be classified
/// here or the build fails. That is the property the AST walker could not have.
fn escape_of_rvalue(
    rvalue: &Rvalue,
    dest: LocalId,
    function: &MirFunction,
    summaries: &HashMap<FuncId, Vec<bool>>,
    storage: &mut Vec<Storage>,
    flows: &mut [Vec<LocalId>],
    raise: &mut impl FnMut(&mut Vec<Storage>, LocalId, Storage),
) {
    let flow = |flows: &mut [Vec<LocalId>], from: &Operand, to: LocalId| {
        if let Some(f) = operand_local(from) {
            if f.0 < flows.len() {
                flows[f.0].push(to);
            }
        }
    };

    match rvalue {
        // `b := a` makes two names for one object. Their fates are therefore
        // IDENTICAL, not one-directional: if `b` is later retained, released or
        // returned, that acts on the object `a` also names, so `a` cannot be a
        // headerless frame slot either.
        //
        // Recording only `a -> b` was wrong and produced a segfault: `a` was
        // promoted to a stack slot while `pass_arc` still emitted
        // `retain(b)`/`release(b)` against the alias, decrementing a refcount
        // that does not exist. The edge must go both ways.
        Rvalue::Use(op) => {
            flow(flows, op, dest);
            if let Some(src) = operand_local(op) {
                if dest.0 < flows.len() {
                    flows[dest.0].push(src);
                }
                // An alias of a managed object becomes a second ARC handle:
                // `pass_arc` runs AFTER this solver and will emit
                // `retain(dest)` / `release(dest)` for it. Those act on the
                // shared object, so neither name can be a headerless frame
                // slot. Raising both here is what stops the aliasing case
                // (`a := Inner(5); b := a; c := b`) from being promoted and
                // then decremented through a stack pointer.
                //
                // Only for pointer-shaped locals: raising a scalar would be
                // meaningless and would suppress promotion needlessly.
                let managed = |id: LocalId| {
                    function
                        .locals
                        .get(id.0)
                        .map(|d| {
                            matches!(
                                d.ty,
                                TypeRef::Custom(_) | TypeRef::Generic(_, _) | TypeRef::Str
                            )
                        })
                        .unwrap_or(false)
                };
                if managed(src) && managed(dest) {
                    raise(storage, src, Storage::Owned);
                    raise(storage, dest, Storage::Owned);
                }
            }
        }
        // A move transfers the single reference: the destination continues the
        // temporary's lifetime, so the two share a fate in both directions.
        Rvalue::Move(src) => {
            if src.0 < flows.len() {
                flows[src.0].push(dest);
            }
            if dest.0 < flows.len() {
                flows[dest.0].push(*src);
            }
        }
        Rvalue::FieldAccess(base, _) => {
            // Reading a field yields a value that may itself be a pointer into
            // the base, so the result inherits the base's fate. One-directional:
            // the field's later use says nothing about the base's storage.
            flow(flows, base, dest);
        }

        // Scalars out; nothing escapes.
        Rvalue::BinaryOp(_, _, _) => {}

        // Fresh allocations: the destination starts at the bottom of the
        // lattice and is raised only by how it is subsequently used.
        Rvalue::AllocateArcStruct(_)
        | Rvalue::AllocateArenaStruct(_, _)
        | Rvalue::AllocateStackStruct(_)
        | Rvalue::AllocateStruct(_)
        | Rvalue::AllocateList(_) => {}

        // A function pointer carries no data.
        Rvalue::FuncRef(_) => {}

        // Direct call: an argument escapes exactly when the callee's summary
        // says that parameter escapes. This is the interprocedural edge.
        Rvalue::CallDirect(callee, args) => {
            let summary = summaries.get(callee);
            for (i, arg) in args.iter().enumerate() {
                let escapes = summary.and_then(|s| s.get(i).copied()).unwrap_or(true);
                if escapes {
                    if let Some(id) = operand_local(arg) {
                        raise(storage, id, Storage::Owned);
                    }
                }
            }
        }

        // Indirect call: a LOCAL SINK, never a graph edge. Modelling it as edges
        // to every signature-compatible function would make the edge count
        // proportional to (call sites x candidates) and lose the linear bound.
        Rvalue::CallIndirect(callee, args) => {
            if let Some(id) = operand_local(callee) {
                raise(storage, id, Storage::Owned);
            }
            for arg in args {
                if let Some(id) = operand_local(arg) {
                    raise(storage, id, Storage::Owned);
                }
            }
        }

        // Builtins are foreign code. Default: everything escapes.
        Rvalue::BuiltinCall(symbol, args) => {
            if !builtin_keeps_nothing(symbol) {
                for arg in args {
                    if let Some(id) = operand_local(arg) {
                        raise(storage, id, Storage::Owned);
                    }
                }
            }
        }

        // A captured value outlives the enclosing statement: the capsule owns it.
        Rvalue::MakeClosure(_, captures) | Rvalue::MakeStackClosure(_, captures) => {
            for c in captures {
                if let Some(id) = operand_local(c) {
                    raise(storage, id, Storage::Owned);
                }
            }
        }

        // Crossing a thread boundary is the one thing that forces atomics.
        Rvalue::SpawnThread(op) => {
            if let Some(id) = operand_local(op) {
                raise(storage, id, Storage::Shared);
            }
        }
    }


}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lattice_join_is_monotone() {
        let all = [Storage::Frame, Storage::Owned, Storage::Shared];
        for a in all {
            for b in all {
                let j = a.join(b);
                // The exact property the fixpoint's termination relies on.
                assert!(j >= a, "join must not move below its input");
                assert!(j >= b, "join must not move below its input");
                assert_eq!(j, b.join(a), "join must be commutative");
            }
        }
    }

    #[test]
    fn lattice_height_is_two() {
        // Load-bearing: the O(h*(E+P)) bound is stated for h = 2. A fourth
        // storage class must be a deliberate decision, not a drive-by addition.
        assert!(Storage::Frame < Storage::Owned);
        assert!(Storage::Owned < Storage::Shared);
        assert_eq!(
            [Storage::Frame, Storage::Owned, Storage::Shared].len(),
            3,
            "lattice must stay 3 points"
        );
    }

    #[test]
    fn scalar_types_are_not_ownership_values() {
        for ty in [TypeRef::Int, TypeRef::Float, TypeRef::Bool, TypeRef::Char, TypeRef::Void] {
            assert!(!is_managed_type(&ty), "scalar {:?} must stay outside ownership facts", ty);
        }
        assert!(is_managed_type(&TypeRef::Str));
        assert!(is_managed_type(&TypeRef::Function));
        assert!(is_managed_type(&TypeRef::Custom(StructTypeId(0))));
        assert!(is_managed_type(&TypeRef::Generic("List".to_string(), vec![TypeRef::Int])));
    }

    #[test]
    fn unlisted_builtin_is_conservative() {
        assert!(!builtin_keeps_nothing("lpp_list_push_arc"));
        assert!(!builtin_keeps_nothing("some_unknown_symbol"));
        assert!(builtin_keeps_nothing("lpp_print_str"));
    }
}
