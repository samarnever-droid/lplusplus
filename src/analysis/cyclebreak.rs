//! Static cycle breaking for owned struct graphs.
//!
//! # What this replaces
//!
//! L++ used to reject any struct reachable from itself:
//!
//! ```text
//! struct Node:            error: Cyclic owned struct 'Node' detected.
//!     left: Node                 ARC cannot reclaim ownership cycles.
//!     right: Node
//! ```
//!
//! That ruled out binary trees, linked lists and parent pointers — the ordinary
//! vocabulary of data structures — and the cost was visible in this repo's own
//! libraries, which reach for raw byte buffers instead (`lppsqlite` carries 267
//! buffer/handle calls; `compresslpp` says outright it uses raw buffers
//! "because the L++ compiler cannot nest lists, store structs in lists…").
//!
//! Rather than allow cycles and leak, or keep rejecting and stay unable to
//! express a tree, this pass **breaks** every cycle at compile time by demoting
//! exactly one edge per cycle to non-owning.
//!
//! # The safety property
//!
//! > **Theorem.** After [`break_cycles`] runs, the subgraph of `Owning` edges
//! > is acyclic.
//!
//! This is the whole point: "no leaks" becomes a structural fact about the
//! program's memory graph, not a runtime behaviour that happens to occur.
//!
//! # Proof
//!
//! The algorithm is the textbook three-colour DFS — *unvisited*, *visiting* (on
//! the stack), *done*. The only part beyond that standard result is the
//! classification step, so that is the only part argued here.
//!
//! **Invariant.** An edge is classified `Owning` if and only if, at the moment
//! it was visited, its target was **not** in the `visiting` set.
//!
//! **Claim.** The invariant implies the owning subgraph is acyclic.
//!
//! Suppose for contradiction the owning subgraph contains a cycle
//! `n₁ → n₂ → … → nₖ → n₁`. Consider the edge `nₖ → n₁`. By the time DFS
//! processes it, `n₁` must be on the stack: `nₖ` was reached through the path
//! `n₁ → … → nₖ`, so DFS descended from `n₁` and cannot have popped it before
//! returning through `nₖ`. Hence `n₁ ∈ visiting`, so by the invariant that edge
//! is classified `NonOwning`, not `Owning` — contradiction. ∎
//!
//! The load-bearing engineering claim is not the DFS (textbook) but that
//! **classification is total**: every edge is classified exactly once before
//! the function returns, and the `NonOwning` branch is taken in exactly the
//! cases the invariant requires. `classification_is_total` checks that
//! directly, and `owning_subgraph_is_acyclic_property` re-verifies the theorem
//! with an *independently written* topological sort rather than trusting this
//! module's own reasoning.
//!
//! # What this does not claim
//!
//! * **Which** edge gets demoted is a heuristic about programmer intent, not a
//!   safety property. Picking a surprising field yields a working, leak-free
//!   program with an unexpected weak field — an ergonomics bug, never
//!   unsoundness. The two claims are deliberately kept separate so a heuristic
//!   complaint is never mistaken for a safety one.
//! * Runtime safety of reading through a demoted field is a **separate**
//!   obligation, discharged by the generation counter in the runtime and
//!   validated under ThreadSanitizer, not by this pass.

use crate::types::{StructTypeId, TypeRef, TypeTable};
use std::collections::{HashMap, HashSet};

/// How a struct field participates in ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    /// Field owns its target: retained on store, released by the destructor.
    Owning,
    /// Field was demoted to break a cycle: stored without a retain, and read
    /// back through a liveness check.
    NonOwning,
}

/// A field edge from one struct to another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldEdge {
    pub from: StructTypeId,
    pub field: String,
    pub to: StructTypeId,
    pub kind: EdgeKind,
}

/// Every field edge in the program, with its ownership classification.
#[derive(Debug, Default, Clone)]
pub struct OwnershipGraph {
    pub edges: Vec<FieldEdge>,
}

impl OwnershipGraph {
    /// Fields demoted to non-owning, as `(struct, field name)`.
    pub fn weak_fields(&self) -> HashSet<(StructTypeId, String)> {
        self.edges
            .iter()
            .filter(|e| e.kind == EdgeKind::NonOwning)
            .map(|e| (e.from, e.field.clone()))
            .collect()
    }

    #[allow(dead_code)]
    pub fn is_weak(&self, owner: StructTypeId, field: &str) -> bool {
        self.edges.iter().any(|e| {
            e.kind == EdgeKind::NonOwning && e.from == owner && e.field == field
        })
    }
}

/// Struct-typed targets reachable through a field's type.
fn field_targets(ty: &TypeRef, out: &mut Vec<StructTypeId>) {
    match ty {
        TypeRef::Custom(id) => out.push(*id),
        TypeRef::Generic(_, args) => {
            for a in args {
                field_targets(a, out);
            }
        }
        TypeRef::Tuple(tys) => {
            for t in tys {
                field_targets(t, out);
            }
        }
        TypeRef::Slice(inner) => {
            field_targets(inner, out);
        }
        TypeRef::Task(inner) => {
            field_targets(inner, out);
        }
        _ => {}
    }
}

/// Build the field-edge graph and classify every edge, breaking all cycles.
///
/// Deterministic: nodes are visited in `StructTypeId` order and fields in
/// declaration order, so the same program always demotes the same field.
///
/// `trait_impls` maps struct name to the set of trait names it implements.
/// For each field typed `TypeRef::Unresolved(name)` where `name` is a trait
/// (i.e., it appears as a value in `trait_impls`), a conservative edge is
/// added from the owning struct to every struct implementing that trait.
/// This is conservative but sound: it may demote an edge that didn't actually
/// form a cycle at runtime (false positive = extra weak field, never unsound).
pub fn break_cycles_with_traits(
    table: &TypeTable,
    trait_impls: &HashMap<String, HashSet<String>>,
) -> OwnershipGraph {
    #[derive(Clone, Copy, PartialEq)]
    enum Colour {
        Unvisited,
        Visiting,
        Done,
    }

    // Collect the set of known trait names (values of trait_impls).
    let trait_names: HashSet<&str> = trait_impls
        .values()
        .flat_map(|set| set.iter().map(String::as_str))
        .collect();

    let n = table.definitions.len();
    let mut colour = vec![Colour::Unvisited; n];
    let mut graph = OwnershipGraph::default();

    // Adjacency in declaration order, so the demoted edge is predictable.
    let mut adj: HashMap<usize, Vec<(String, StructTypeId)>> = HashMap::new();
    for (i, def) in table.definitions.iter().enumerate() {
        let mut list = Vec::new();
        for (fname, fty) in &def.fields {
            // 1. Direct / container targets.
            let mut targets = Vec::new();
            field_targets(fty, &mut targets);
            for t in targets {
                if t.0 < n {
                    list.push((fname.clone(), t));
                }
            }
            // 2. Trait-typed fields: conservative edges to every implementor.
            if let TypeRef::Unresolved(type_name) = fty {
                if trait_names.contains(type_name.as_str()) {
                    // Every struct that implements this trait is a potential
                    // runtime target of this field.
                    for (implementor_name, traits) in trait_impls {
                        if traits.contains(type_name.as_str()) {
                            if let Some(&impl_id) =
                                table.structs_by_name.get(implementor_name)
                            {
                                if impl_id.0 < n {
                                    list.push((fname.clone(), impl_id));
                                }
                            }
                        }
                    }
                }
            }
        }
        adj.insert(i, list);
    }

    // Iterative DFS: recursion would blow the stack on a deep type graph, and
    // the explicit stack makes the `visiting` set inspectable.
    for root in 0..n {
        if colour[root] != Colour::Unvisited {
            continue;
        }
        // (node, index of next outgoing edge to process)
        let mut stack: Vec<(usize, usize)> = vec![(root, 0)];
        colour[root] = Colour::Visiting;

        while let Some((node, edge_idx)) = stack.pop() {
            let empty = Vec::new();
            let edges = adj.get(&node).unwrap_or(&empty);
            if edge_idx >= edges.len() {
                colour[node] = Colour::Done;
                continue;
            }
            stack.push((node, edge_idx + 1));
            let (fname, target) = edges[edge_idx].clone();

            // THE classification step. `Visiting` means the target is an
            // ancestor on the current DFS path, i.e. this edge closes a cycle.
            let kind = if colour[target.0] == Colour::Visiting {
                EdgeKind::NonOwning
            } else {
                EdgeKind::Owning
            };
            graph.edges.push(FieldEdge {
                from: StructTypeId(node),
                field: fname,
                to: target,
                kind,
            });

            if colour[target.0] == Colour::Unvisited {
                colour[target.0] = Colour::Visiting;
                stack.push((target.0, 0));
            }
        }
    }

    graph
}

/// Convenience wrapper: calls [`break_cycles_with_traits`] with an empty trait map.
/// Existing call sites that don't have trait information are unaffected.
#[allow(dead_code)]
pub fn break_cycles(table: &TypeTable) -> OwnershipGraph {
    break_cycles_with_traits(table, &HashMap::new())
}

/// Mark every struct that participates in an ownership cycle as
/// `is_self_referential` so the whole cycle is arena-allocated.
///
/// # Why this closes a safety gap
///
/// The static cycle breaker demotes exactly one edge per cycle to
/// `NonOwning`, so reading back through a demoted (weak) field must never
/// touch freed memory. Direct self-referential structs already get this
/// guarantee because they are arena-allocated: an arena region keeps every
/// node in it alive until the *last* node dies, so a weak read through a
/// still-live source node in the same region cannot dangle.
///
/// Mutual / indirect cycles (e.g. `Parent <-> Child`) are **not** direct
/// self-references, so they were left as plain ARC objects and their demoted
/// edges had no such protection. Marking every struct that sits on a cycle as
/// self-referential extends the arena guarantee to them. This must run before
/// MIR lowering, because the arena-vs-ARC allocation choice is made there from
/// `is_self_referential`.
///
/// This is conservative: a struct that merely *could* form a cycle (fields
/// typed with the target, even if a given runtime value is acyclic) becomes
/// arena-allocated. Arena allocation is valid for any struct, so this only
/// costs a little, never correctness.
pub fn mark_cyclic_structs(table: &mut TypeTable) {
    let n = table.definitions.len();
    if n == 0 {
        return;
    }
    // Struct-level adjacency: struct i reaches every struct named by one of
    // its fields' types (descending into List / tuple / slice / task).
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, def) in table.definitions.iter().enumerate() {
        let mut targets = Vec::new();
        for (_, fty) in &def.fields {
            field_targets(fty, &mut targets);
        }
        for t in &targets {
            if t.0 < n {
                adj[i].push(t.0);
            }
        }
    }
    // Tarjan SCC. A node is on a cycle iff it is in an SCC of size > 1 or has
    // a self-loop. Depth is bounded by the number of struct types, which is
    // small, so recursion is safe here.
    let n = adj.len();
    let mut indices = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut next = 0usize;
    let mut cyclic: Vec<usize> = Vec::new();
    for root in 0..n {
        if indices[root] != usize::MAX {
            continue;
        }
        // Explicit frame so we don't need a recursive closure with many
        // `&mut` captures.
        let mut frames = vec![(root, 0usize)];
        indices[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root);
        on_stack[root] = true;
        while let Some((v, edge)) = frames.pop() {
            if edge < adj[v].len() {
                let w = adj[v][edge];
                frames.push((v, edge + 1));
                if indices[w] == usize::MAX {
                    indices[w] = next;
                    low[w] = next;
                    next += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    frames.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(indices[w]);
                }
            } else {
                // Done with v's outgoing edges: propagate lowlink to parent.
                if let Some(&(parent, _)) = frames.last() {
                    low[parent] = low[parent].min(low[v]);
                }
                if low[v] == indices[v] {
                    // v roots an SCC; pop all its members off the stack.
                    let mut scc: Vec<usize> = Vec::new();
                    loop {
                        let w = stack.pop().expect("tarjan stack underflow");
                        on_stack[w] = false;
                        scc.push(w);
                        if w == v {
                            break;
                        }
                    }
                    let self_loop = scc.len() == 1 && adj[scc[0]].contains(&scc[0]);
                    if scc.len() > 1 || self_loop {
                        cyclic.extend(scc);
                    }
                }
            }
        }
    }
    for id in cyclic {
        table.definitions[id].is_self_referential = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(defs: Vec<(&str, Vec<(&str, usize)>)>) -> TypeTable {
        let mut t = TypeTable::new();
        for (name, _) in &defs {
            t.register_struct(name.to_string());
        }
        for (i, (_, fields)) in defs.iter().enumerate() {
            t.definitions[i].fields = fields
                .iter()
                .map(|(f, target)| (f.to_string(), TypeRef::Custom(StructTypeId(*target))))
                .collect();
        }
        t
    }

    /// Independent verifier: Kahn's algorithm, written separately from
    /// `break_cycles` so a self-consistent-but-wrong implementation cannot hide
    /// a bug from its own tests.
    fn owning_subgraph_acyclic(g: &OwnershipGraph, node_count: usize) -> bool {
        let mut indeg = vec![0usize; node_count];
        let mut out: HashMap<usize, Vec<usize>> = HashMap::new();
        for e in g.edges.iter().filter(|e| e.kind == EdgeKind::Owning) {
            indeg[e.to.0] += 1;
            out.entry(e.from.0).or_default().push(e.to.0);
        }
        let mut queue: Vec<usize> = (0..node_count).filter(|i| indeg[*i] == 0).collect();
        let mut removed = 0;
        while let Some(nd) = queue.pop() {
            removed += 1;
            if let Some(succs) = out.get(&nd) {
                for s in succs.clone() {
                    indeg[s] -= 1;
                    if indeg[s] == 0 {
                        queue.push(s);
                    }
                }
            }
        }
        removed == node_count
    }

    #[test]
    fn self_reference_is_broken() {
        let t = table(vec![("Node", vec![("next", 0)])]);
        let g = break_cycles(&t);
        assert_eq!(g.edges.len(), 1);
        assert_eq!(g.edges[0].kind, EdgeKind::NonOwning);
        assert!(owning_subgraph_acyclic(&g, 1));
    }

    #[test]
    fn two_struct_cycle_is_broken() {
        let t = table(vec![("A", vec![("b", 1)]), ("B", vec![("a", 0)])]);
        let g = break_cycles(&t);
        let weak = g.edges.iter().filter(|e| e.kind == EdgeKind::NonOwning).count();
        assert_eq!(weak, 1, "exactly one edge should be demoted");
        assert!(owning_subgraph_acyclic(&g, 2));
    }

    #[test]
    fn three_struct_cycle_is_broken() {
        let t = table(vec![
            ("A", vec![("b", 1)]),
            ("B", vec![("c", 2)]),
            ("C", vec![("a", 0)]),
        ]);
        let g = break_cycles(&t);
        assert_eq!(g.edges.iter().filter(|e| e.kind == EdgeKind::NonOwning).count(), 1);
        assert!(owning_subgraph_acyclic(&g, 3));
    }

    #[test]
    fn mutual_cycle_is_marked_self_referential() {
        // Parent <-> Child is a two-struct cycle, not a direct self-reference.
        let mut t = table(vec![
            ("Child", vec![("parent", 1)]),
            ("Parent", vec![("kid", 0)]),
        ]);
        mark_cyclic_structs(&mut t);
        assert!(
            t.definitions[0].is_self_referential,
            "Child participates in a cycle and must be arena-allocated"
        );
        assert!(
            t.definitions[1].is_self_referential,
            "Parent participates in a cycle and must be arena-allocated"
        );
    }

    #[test]
    fn direct_self_reference_is_marked() {
        let mut t = table(vec![("Node", vec![("next", 0)])]);
        mark_cyclic_structs(&mut t);
        assert!(t.definitions[0].is_self_referential);
    }

    #[test]
    fn acyclic_diamond_is_not_marked() {
        let mut t = table(vec![
            ("A", vec![("b", 1), ("c", 2)]),
            ("B", vec![("d", 3)]),
            ("C", vec![("d", 3)]),
            ("D", vec![]),
        ]);
        mark_cyclic_structs(&mut t);
        for (i, def) in t.definitions.iter().enumerate() {
            assert!(
                !def.is_self_referential,
                "struct {} in an acyclic diamond must not be arena-allocated",
                def.name
            );
            let _ = i;
        }
    }

    #[test]
    fn linked_list_is_marked() {
        // Singly-linked list: Cell -> Cell is a self-cycle (the null terminator
        // only breaks it at runtime, the type graph is still cyclic).
        let mut t = table(vec![("Cell", vec![("next", 0)])]);
        mark_cyclic_structs(&mut t);
        assert!(t.definitions[0].is_self_referential);
    }

    #[test]
    fn diamond_demotes_nothing() {
        // A → B, A → C, B → D, C → D. Shared dependency, no cycle.
        let t = table(vec![
            ("A", vec![("b", 1), ("c", 2)]),
            ("B", vec![("d", 3)]),
            ("C", vec![("d", 3)]),
            ("D", vec![]),
        ]);
        let g = break_cycles(&t);
        assert_eq!(
            g.edges.iter().filter(|e| e.kind == EdgeKind::NonOwning).count(),
            0,
            "a DAG must keep every edge owning"
        );
        assert!(owning_subgraph_acyclic(&g, 4));
    }

    #[test]
    fn disconnected_components_are_handled() {
        // A ↔ B, and separately C → D (acyclic).
        let t = table(vec![
            ("A", vec![("b", 1)]),
            ("B", vec![("a", 0)]),
            ("C", vec![("d", 3)]),
            ("D", vec![]),
        ]);
        let g = break_cycles(&t);
        assert_eq!(g.edges.iter().filter(|e| e.kind == EdgeKind::NonOwning).count(), 1);
        assert!(owning_subgraph_acyclic(&g, 4));
    }

    #[test]
    fn binary_tree_shape_demotes_nothing_extra() {
        // struct Node: left: Node, right: Node -- two self edges. The first
        // closes the cycle; the second is also a back edge to the same node.
        let t = table(vec![("Node", vec![("left", 0), ("right", 0)])]);
        let g = break_cycles(&t);
        assert_eq!(g.edges.len(), 2);
        assert!(
            g.edges.iter().all(|e| e.kind == EdgeKind::NonOwning),
            "both self edges close a cycle, so both must be demoted"
        );
        assert!(owning_subgraph_acyclic(&g, 1));
    }

    #[test]
    fn classification_is_total() {
        // Every field edge in the input must appear exactly once in the output.
        let t = table(vec![
            ("A", vec![("b", 1), ("c", 2)]),
            ("B", vec![("a", 0), ("c", 2)]),
            ("C", vec![("a", 0)]),
        ]);
        let expected: usize = t
            .definitions
            .iter()
            .map(|d| {
                d.fields
                    .iter()
                    .filter(|(_, ty)| matches!(ty, TypeRef::Custom(_)))
                    .count()
            })
            .sum();
        let g = break_cycles(&t);
        assert_eq!(g.edges.len(), expected, "every edge must be classified once");
        assert!(owning_subgraph_acyclic(&g, 3));
    }

    #[test]
    fn owning_subgraph_is_acyclic_property() {
        // Seeded pseudo-random graphs, verified with the independent topo sort.
        // Deterministic so a failure is reproducible.
        let mut seed: u64 = 0x2545F4914F6CDD1D;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for case in 0..600 {
            let n = 2 + (next() % 7) as usize;
            let mut defs: Vec<(String, Vec<(String, usize)>)> = Vec::new();
            for i in 0..n {
                let nf = (next() % 4) as usize;
                let mut fields = Vec::new();
                for f in 0..nf {
                    fields.push((format!("f{}", f), (next() % n as u64) as usize));
                }
                defs.push((format!("S{}", i), fields));
            }
            let mut t = TypeTable::new();
            for (name, _) in &defs {
                t.register_struct(name.clone());
            }
            for (i, (_, fields)) in defs.iter().enumerate() {
                t.definitions[i].fields = fields
                    .iter()
                    .map(|(f, tgt)| (f.clone(), TypeRef::Custom(StructTypeId(*tgt))))
                    .collect();
            }
            let g = break_cycles(&t);
            assert!(
                owning_subgraph_acyclic(&g, n),
                "case {} produced a cyclic owning subgraph (seed path)",
                case
            );
            let total: usize = t
                .definitions
                .iter()
                .map(|d| d.fields.len())
                .sum();
            assert_eq!(g.edges.len(), total, "case {}: classification not total", case);
        }
    }

    #[test]
    fn is_weak_reports_demoted_fields() {
        let t = table(vec![("Node", vec![("next", 0)])]);
        let g = break_cycles(&t);
        assert!(g.is_weak(StructTypeId(0), "next"));
        assert!(!g.is_weak(StructTypeId(0), "absent"));
    }

    #[test]
    fn indirect_cycle_through_tuple_is_broken() {
        let mut t = TypeTable::new();
        t.register_struct("A".to_string());
        t.register_struct("B".to_string());

        // S0 (A) has field `b` of type Tuple[B, Int]
        t.definitions[0].fields = vec![
            ("b".to_string(), TypeRef::Tuple(vec![TypeRef::Custom(StructTypeId(1)), TypeRef::Int])),
        ];
        // S1 (B) has field `a` of type A
        t.definitions[1].fields = vec![
            ("a".to_string(), TypeRef::Custom(StructTypeId(0))),
        ];

        let g = break_cycles(&t);
        let weak = g.edges.iter().filter(|e| e.kind == EdgeKind::NonOwning).count();
        assert_eq!(weak, 1, "exactly one edge should be demoted in cycle through tuple");
        assert!(owning_subgraph_acyclic(&g, 2));
    }

    /// Used by the doc comment's totality argument; kept as a named check so
    /// the claim is executable rather than prose.
    #[test]
    fn empty_table_is_trivially_sound() {
        let t = TypeTable::new();
        let g = break_cycles(&t);
        assert!(g.edges.is_empty());
        assert!(owning_subgraph_acyclic(&g, 0));
    }
}
