//! Phase 4E: ownership-balance verification.
//!
//! A deterministic token dataflow over each function's CFG that proves,
//! statically, that every heap-value cell is consumed at most once and
//! that every transfer obeys the consume-vs-borrow call contract the
//! interpreter executes. The runtime's end-of-execution proof is the
//! backstop; the static checker reports first at compile time.

use std::collections::{BTreeMap, BTreeSet};

use lpp_hir::OriginId;
use lpp_mir::{
    BasicBlockId, InstructionKind, MirFunctionId, MirInvariant, MirLocalId, MirProgram, Operand,
    Rvalue, Terminator,
};
use lpp_passes::{MirPass, PassContext, PassFailure};
use lpp_types::{BuiltinId, PrimitiveType, TypeId, TypeInterner, TypeKind};

use crate::plan::{OwnershipPlan, compute_ownership_plan};

/// Whether the type can name a heap value (a runtime heap node, or an
/// inline tuple that can own one).
fn is_heap_type(types: &TypeInterner, ty: TypeId) -> bool {
    match types.kind(ty) {
        TypeKind::Function { .. }
        | TypeKind::List(_)
        | TypeKind::Slice(_)
        | TypeKind::Task(_)
        | TypeKind::Tuple(_)
        | TypeKind::Map { .. }
        | TypeKind::Nominal { .. } => true,
        TypeKind::Primitive(PrimitiveType::String) => true,
        _ => false,
    }
}

/// Whether a builtin moves (consumes) its positional value argument.
/// Mirrors the interpreter's `builtin_moves_value` exactly: the same
/// call contract, checked twice.
fn builtin_moves_value(builtin: BuiltinId, position: usize) -> bool {
    let name = builtin
        .descriptor()
        .name
        .strip_prefix("lpp_")
        .unwrap_or(builtin.descriptor().name);
    matches!(
        (name, position),
        ("list_push", 1) | ("list_set", 2) | ("print", 0) | ("write_str", 0)
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token {
    /// The cell holds its reference on every path through the point.
    Alive,
    /// The cell was moved out on every path through the point.
    Dead,
    /// The cell's fate is path-dependent.
    MayDead,
}

impl Token {
    fn meet(self, other: Token) -> Token {
        match (self, other) {
            (Self::Alive, Self::Alive) => Self::Alive,
            (Self::Dead, Self::Dead) => Self::Dead,
            _ => Self::MayDead,
        }
    }
}

/// A balance proof failure (code `E4403`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnershipBalanceErrorKind {
    /// A heap-value cell is consumed or read after being moved out.
    UseAfterMove {
        function: MirFunctionId,
        block: BasicBlockId,
        local: MirLocalId,
    },
    /// A heap-value local has no plan cell; validated, planned MIR
    /// never triggers this.
    MissingCell {
        function: MirFunctionId,
        local: MirLocalId,
    },
}

/// A balance proof failure (code `E4403`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnershipBalanceError {
    pub kind: OwnershipBalanceErrorKind,
    pub origin: Option<OriginId>,
}

impl OwnershipBalanceError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        "E4403"
    }
}

impl std::fmt::Display for OwnershipBalanceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {:?}", self.code(), self.kind)
    }
}

impl std::error::Error for OwnershipBalanceError {}

type State = BTreeMap<MirLocalId, Token>;

/// Whether the cell is not provably alive at this point (absent, dead,
/// or path-dependent) — a consume or read here is a use-after-move.
fn not_alive(state: &State, heap_locals: &BTreeSet<MirLocalId>, local: MirLocalId) -> bool {
    if !heap_locals.contains(&local) {
        return false;
    }
    matches!(
        state.get(&local),
        None | Some(Token::Dead) | Some(Token::MayDead)
    )
}

/// Prove the ownership balance of a planned program. For every
/// heap-value cell the checker tracks one token through the CFG:
/// definitions and borrows keep it alive, moves kill it, and any
/// consume or read of a dead (or path-dependent) cell is
/// `UseAfterMove`. The dataflow is a worklist fixed point over
/// block-ascending successors with a bounded iteration count, so two
/// runs agree byte for byte.
pub fn verify_ownership_balance(
    program: &MirProgram,
    types: &TypeInterner,
    plan: &OwnershipPlan,
) -> Vec<OwnershipBalanceError> {
    let planned: BTreeSet<(MirFunctionId, MirLocalId)> = plan
        .cells
        .iter()
        .map(|cell| (cell.function, cell.local))
        .collect();

    let mut result: Vec<OwnershipBalanceError> = Vec::new();
    for (function_id, function) in program.functions() {
        let locals = program.function_locals(function).to_vec();
        let heap_locals: BTreeSet<MirLocalId> = locals
            .iter()
            .copied()
            .filter(|local| {
                program
                    .local(*local)
                    .is_some_and(|local| is_heap_type(types, local.ty))
            })
            .collect();

        let mut violations: BTreeSet<(BasicBlockId, MirLocalId)> = BTreeSet::new();
        let mut worklist: Vec<BasicBlockId> = Vec::new();
        let mut queued: BTreeSet<BasicBlockId> = BTreeSet::new();

        // Structural guard: every heap-value local is planned.
        for local in &heap_locals {
            if !planned.contains(&(function_id, *local)) {
                result.push(OwnershipBalanceError {
                    kind: OwnershipBalanceErrorKind::MissingCell {
                        function: function_id,
                        local: *local,
                    },
                    origin: program.local(*local).map(|local| local.origin),
                });
            }
        }

        let mut initial: State = BTreeMap::new();
        for parameter in program.function_parameters(function) {
            if heap_locals.contains(parameter) {
                initial.insert(*parameter, Token::Alive);
            }
        }

        let mut in_states: BTreeMap<BasicBlockId, State> = BTreeMap::new();
        let mut out_states: BTreeMap<BasicBlockId, State> = BTreeMap::new();
        in_states.insert(function.entry, initial);
        worklist.push(function.entry);
        queued.insert(function.entry);
        let bound = 64 + program.function_blocks(function).len().saturating_mul(4);
        for _ in 0..bound {
            let Some(block_id) = worklist.pop() else {
                break;
            };
            queued.remove(&block_id);
            let in_state = in_states.get(&block_id).cloned().unwrap_or_default();
            let (out_state, block_violations) =
                apply_block(program, block_id, &in_state, &heap_locals);
            violations.extend(block_violations);
            if out_states.get(&block_id) == Some(&out_state) {
                // The transfer is stable for this in-state.
                continue;
            }
            out_states.insert(block_id, out_state.clone());
            for target in successors(program, block_id) {
                // A block with no in-state yet is bottom: the first
                // reach installs the incoming state, later reaches
                // meet it in.
                let changed = match in_states.get(&target) {
                    None => {
                        in_states.insert(target, out_state.clone());
                        true
                    }
                    Some(previous) => {
                        let mut merged: State = BTreeMap::new();
                        let keys: BTreeSet<MirLocalId> =
                            previous.keys().chain(out_state.keys()).copied().collect();
                        for key in keys {
                            let left = previous.get(&key).copied().unwrap_or(Token::Dead);
                            let right = out_state.get(&key).copied().unwrap_or(Token::Dead);
                            let met = left.meet(right);
                            if met != Token::Dead {
                                merged.insert(key, met);
                            }
                        }
                        if *previous == merged {
                            false
                        } else {
                            in_states.insert(target, merged);
                            true
                        }
                    }
                };
                if changed && queued.insert(target) {
                    worklist.push(target);
                }
            }
        }
        for (block, local) in violations {
            result.push(OwnershipBalanceError {
                kind: OwnershipBalanceErrorKind::UseAfterMove {
                    function: function_id,
                    block,
                    local,
                },
                origin: program.local(local).map(|local| local.origin),
            });
        }
    }
    result
}

fn successors(program: &MirProgram, block_id: BasicBlockId) -> Vec<BasicBlockId> {
    let block = program
        .block(block_id)
        .expect("verified MIR retains every block");
    match block.terminator {
        Terminator::Goto(target) => vec![target],
        Terminator::Branch {
            then_block,
            else_block,
            ..
        } => vec![then_block, else_block],
        Terminator::SwitchEnum { targets, .. } => {
            let mut targets = program.switch_targets(targets).to_vec();
            targets.sort_unstable();
            targets
        }
        Terminator::Return(_) | Terminator::Unreachable => Vec::new(),
    }
}

/// Apply one block's instructions to the in-state; returns the out-state
/// and the violations found along the way.
fn apply_block(
    program: &MirProgram,
    block_id: BasicBlockId,
    in_state: &State,
    heap_locals: &BTreeSet<MirLocalId>,
) -> (State, Vec<(BasicBlockId, MirLocalId)>) {
    let block = program
        .block(block_id)
        .expect("verified MIR retains every block");
    let mut state: State = in_state.clone();
    let mut violations: Vec<(BasicBlockId, MirLocalId)> = Vec::new();
    macro_rules! check_local {
        ($local:expr) => {{
            let local = $local;
            if not_alive(&state, heap_locals, local) {
                violations.push((block_id, local));
            }
        }};
    }

    for &instruction_id in program.block_instructions(block) {
        let instruction = program
            .instruction(instruction_id)
            .expect("verified MIR retains every instruction");
        match instruction.kind {
            InstructionKind::Assign { target, value } => {
                match value {
                    Rvalue::Use(operand) => {
                        if let Operand::Copy(local) = operand {
                            check_local!(local);
                        }
                    }
                    Rvalue::Unary { operand, .. } | Rvalue::ListLen(operand) => {
                        if let Operand::Copy(local) = operand {
                            check_local!(local);
                        }
                    }
                    Rvalue::Binary { left, right, .. } => {
                        for operand in [left, right] {
                            if let Operand::Copy(local) = operand {
                                check_local!(local);
                            }
                        }
                    }
                    Rvalue::Tuple(operands)
                    | Rvalue::List(operands)
                    | Rvalue::ConstructStruct {
                        fields: operands, ..
                    }
                    | Rvalue::ConstructVariant {
                        fields: operands, ..
                    } => {
                        // Elements move into the container.
                        for operand in program.operands(operands) {
                            if let Operand::Copy(local) = *operand {
                                check_local!(local);
                                if heap_locals.contains(&local) {
                                    state.insert(local, Token::Dead);
                                }
                            }
                        }
                    }
                    Rvalue::Load(place) => {
                        // The container is read; the target gains a
                        // fresh reference.
                        if let Some(place) = program.place(place) {
                            check_local!(place.root);
                        }
                    }
                    Rvalue::Call {
                        callee, arguments, ..
                    } => {
                        // Invocation is indirection, not transfer: the
                        // callee is read, not consumed — but it must be
                        // alive (a moved callee is a use-after-move).
                        if let Operand::Copy(local) = callee {
                            check_local!(local);
                        }
                        // Arguments move into the callee's frame.
                        for operand in program.operands(arguments) {
                            if let Operand::Copy(local) = *operand {
                                check_local!(local);
                                if heap_locals.contains(&local) {
                                    state.insert(local, Token::Dead);
                                }
                            }
                        }
                    }
                    Rvalue::Builtin { builtin, arguments } => {
                        for (position, operand) in program.operands(arguments).iter().enumerate() {
                            if let Operand::Copy(local) = *operand {
                                if builtin_moves_value(builtin, position) {
                                    check_local!(local);
                                    if heap_locals.contains(&local) {
                                        state.insert(local, Token::Dead);
                                    }
                                } else {
                                    check_local!(local);
                                }
                            }
                        }
                    }
                    Rvalue::MakeClosure { captures, .. } => {
                        // Captures are borrows: the closure retains, the
                        // source stays alive.
                        for operand in program.operands(captures) {
                            if let Operand::Copy(local) = *operand {
                                check_local!(local);
                            }
                        }
                    }
                    Rvalue::Await(operand) | Rvalue::Spawn(operand) => {
                        // The task/closure handle is a receiver.
                        if let Operand::Copy(local) = operand {
                            check_local!(local);
                        }
                    }
                }
                // The target owns a fresh value (the old one is
                // released at runtime on reassignment).
                if heap_locals.contains(&target) {
                    state.insert(target, Token::Alive);
                }
            }
            InstructionKind::Store { place, value } => {
                if let Operand::Copy(local) = value {
                    check_local!(local);
                }
                if let Some(place) = program.place(place) {
                    let projections = program.place_projections(place).to_vec();
                    if projections.is_empty() {
                        if heap_locals.contains(&place.root) {
                            state.insert(place.root, Token::Alive);
                        }
                    } else {
                        check_local!(place.root);
                    }
                }
            }
        }
    }
    match block.terminator {
        Terminator::Return(Some(Operand::Copy(local))) => {
            check_local!(local);
            if heap_locals.contains(&local) {
                state.insert(local, Token::Dead);
            }
        }
        _ => {}
    }
    (state, violations)
}

/// The pass adapter: requires the core invariants plus `Ownership`
/// (4D), preserves the core invariants plus `Ownership` and
/// `NoOwningCycles`, and establishes `OwnershipBalance` by proof.
pub struct OwnershipBalancePass;

const BALANCE_PRESERVED: &[MirInvariant] = &[
    MirInvariant::References,
    MirInvariant::ControlFlow,
    MirInvariant::Types,
    MirInvariant::DefiniteInitialization,
    MirInvariant::Ownership,
    MirInvariant::NoOwningCycles,
];

impl MirPass for OwnershipBalancePass {
    fn name(&self) -> &'static str {
        "ownership-balance"
    }

    fn required(&self) -> &'static [MirInvariant] {
        &[
            MirInvariant::References,
            MirInvariant::ControlFlow,
            MirInvariant::Types,
            MirInvariant::DefiniteInitialization,
            MirInvariant::Ownership,
        ]
    }

    fn preserved(&self) -> &'static [MirInvariant] {
        BALANCE_PRESERVED
    }

    fn established(&self) -> &'static [MirInvariant] {
        &[MirInvariant::OwnershipBalance]
    }

    fn run(
        &mut self,
        program: &mut MirProgram,
        context: &PassContext<'_>,
    ) -> Result<(), PassFailure> {
        let plan = compute_ownership_plan(program, context.types).map_err(|_| PassFailure {
            message: "ownership plan construction failed",
        })?;
        if verify_ownership_balance(program, context.types, &plan).is_empty() {
            Ok(())
        } else {
            Err(PassFailure {
                message: "ownership balance proof failed",
            })
        }
    }
}
