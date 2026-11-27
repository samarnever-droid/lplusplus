//! `dispatch-thread` — CFG constant-state jump-threading pass.
//!
//! Eliminates software state-machine dispatchers (`while true: if _bb <= mid: ...`)
//! by propagating per-block constant local state (`known`) forward along chains of
//! pure single-block temporary blocks (`Branch`/`Goto`) up to a bounded depth,
//! rewriting the source block's terminator edge in place.

use std::collections::{BTreeMap, BTreeSet};

use lpp_mir::{
    BasicBlockId, BinaryOperator, Constant, InstructionKind, MirFunction, MirFunctionId,
    MirInvariant, MirLocalId, MirLocalKind, MirProgram, Operand, PlaceProjection, Rvalue,
    Terminator, UnaryOperator,
};
use lpp_types::TypeKind;

use crate::passes::analysis::is_primitive_constant;
use crate::passes::constfold::{IN_PLACE_PRESERVED, fold_binary, fold_unary};
use crate::{MirPass, PassContext, PassFailure};

const MAX_THREAD_DEPTH: usize = 64;

/// `dispatch-thread`: thread terminators through pure dispatch blocks using
/// constant state established at the end of each predecessor block.
#[derive(Debug, Clone, Copy, Default)]
pub struct DispatchThreadPass;

impl MirPass for DispatchThreadPass {
    fn name(&self) -> &'static str {
        "dispatch-thread"
    }

    fn preserved(&self) -> &'static [MirInvariant] {
        IN_PLACE_PRESERVED
    }

    fn run(
        &mut self,
        program: &mut MirProgram,
        _context: &PassContext<'_>,
    ) -> Result<(), PassFailure> {
        let function_ids: Vec<MirFunctionId> = program.functions().map(|(id, _)| id).collect();
        for function_id in function_ids {
            let Some(function) = program.function(function_id) else {
                continue;
            };
            let single_block_temps = find_single_block_temps(program, function, _context.types);
            let block_ids: Vec<BasicBlockId> = program.function_blocks(function).to_vec();
            let mut updates: Vec<(BasicBlockId, Terminator)> = Vec::new();
            for &block_id in &block_ids {
                let Some(block) = program.block(block_id) else {
                    continue;
                };
                if !matches!(
                    block.terminator,
                    Terminator::Goto(_) | Terminator::Branch { .. }
                ) {
                    continue;
                }
                let known = compute_end_constants(program, block_id);
                if known.is_empty() {
                    continue;
                }
                match block.terminator {
                    Terminator::Goto(target) => {
                        let resolved = thread_target(program, target, &known, &single_block_temps);
                        if resolved != target {
                            updates.push((block_id, Terminator::Goto(resolved)));
                        }
                    }
                    Terminator::Branch {
                        condition,
                        then_block,
                        else_block,
                    } => {
                        let new_then =
                            thread_target(program, then_block, &known, &single_block_temps);
                        let new_else =
                            thread_target(program, else_block, &known, &single_block_temps);
                        if new_then != then_block || new_else != else_block {
                            updates.push((
                                block_id,
                                Terminator::Branch {
                                    condition,
                                    then_block: new_then,
                                    else_block: new_else,
                                },
                            ));
                        }
                    }
                    _ => {}
                }
            }
            for (block_id, terminator) in updates {
                if let Some(block) = program.block_mut(block_id) {
                    block.terminator = terminator;
                }
            }
        }
        Ok(())
    }
}

fn find_single_block_temps(
    program: &MirProgram,
    function: &MirFunction,
    types: &lpp_types::TypeInterner,
) -> BTreeSet<MirLocalId> {
    let mut def_block: BTreeMap<MirLocalId, Option<BasicBlockId>> = BTreeMap::new();
    let record = |map: &mut BTreeMap<MirLocalId, Option<BasicBlockId>>,
                  local: MirLocalId,
                  bb: BasicBlockId| {
        map.entry(local)
            .and_modify(|prev| {
                if *prev != Some(bb) {
                    *prev = None;
                }
            })
            .or_insert(Some(bb));
    };
    let visit_op =
        |map: &mut BTreeMap<MirLocalId, Option<BasicBlockId>>, op: Operand, bb: BasicBlockId| {
            if let Operand::Copy(local) = op {
                record(map, local, bb);
            }
        };

    for &bb in program.function_blocks(function) {
        let Some(block) = program.block(bb) else {
            continue;
        };
        for &inst_id in program.block_instructions(block) {
            let Some(inst) = program.instruction(inst_id) else {
                continue;
            };
            match inst.kind {
                InstructionKind::Assign { target, value } => {
                    record(&mut def_block, target, bb);
                    match value {
                        Rvalue::Use(op)
                        | Rvalue::Await(op)
                        | Rvalue::Spawn(op)
                        | Rvalue::ListLen(op) => {
                            visit_op(&mut def_block, op, bb);
                        }
                        Rvalue::Unary { operand, .. } | Rvalue::Cast { value: operand, .. } => {
                            visit_op(&mut def_block, operand, bb);
                        }
                        Rvalue::Binary { left, right, .. } => {
                            visit_op(&mut def_block, left, bb);
                            visit_op(&mut def_block, right, bb);
                        }
                        Rvalue::Load(place) => {
                            if let Some(p) = program.place(place) {
                                record(&mut def_block, p.root, bb);
                                for &proj in program.place_projections(p) {
                                    if let PlaceProjection::ListIndex(op) = proj {
                                        visit_op(&mut def_block, op, bb);
                                    }
                                }
                            }
                        }
                        Rvalue::Tuple(ops)
                        | Rvalue::List(ops)
                        | Rvalue::ConstructStruct { fields: ops, .. }
                        | Rvalue::ConstructVariant { fields: ops, .. }
                        | Rvalue::ExternalCall { arguments: ops, .. }
                        | Rvalue::Builtin { arguments: ops, .. }
                        | Rvalue::Intrinsic { arguments: ops, .. }
                        | Rvalue::MakeClosure { captures: ops, .. } => {
                            for &op in program.operands(ops) {
                                visit_op(&mut def_block, op, bb);
                            }
                        }
                        Rvalue::Call { callee, arguments } => {
                            visit_op(&mut def_block, callee, bb);
                            for &op in program.operands(arguments) {
                                visit_op(&mut def_block, op, bb);
                            }
                        }
                    }
                }
                InstructionKind::Store { place, value } => {
                    if let Some(p) = program.place(place) {
                        record(&mut def_block, p.root, bb);
                        for &proj in program.place_projections(p) {
                            if let PlaceProjection::ListIndex(op) = proj {
                                visit_op(&mut def_block, op, bb);
                            }
                        }
                    }
                    visit_op(&mut def_block, value, bb);
                }
            }
        }
        match block.terminator {
            Terminator::Branch { condition, .. } => visit_op(&mut def_block, condition, bb),
            Terminator::SwitchEnum { subject, .. } => visit_op(&mut def_block, subject, bb),
            Terminator::Return(Some(op)) => visit_op(&mut def_block, op, bb),
            _ => {}
        }
    }

    def_block
        .into_iter()
        .filter_map(|(local, bb_opt)| {
            bb_opt?;
            let desc = program.local(local)?;
            if desc.kind != MirLocalKind::Temporary {
                return None;
            }
            if !matches!(types.kind(desc.ty), TypeKind::Primitive(_)) {
                return None;
            }
            Some(local)
        })
        .collect()
}

fn eval_op(env: &BTreeMap<MirLocalId, Constant>, op: Operand) -> Option<Constant> {
    match op {
        Operand::Constant(c) if is_primitive_constant(&c) => Some(c),
        Operand::Copy(local) => env.get(&local).copied(),
        _ => None,
    }
}

fn is_pure_branch_op(op: BinaryOperator) -> bool {
    matches!(
        op,
        BinaryOperator::Equal
            | BinaryOperator::NotEqual
            | BinaryOperator::Less
            | BinaryOperator::LessEqual
            | BinaryOperator::Greater
            | BinaryOperator::GreaterEqual
            | BinaryOperator::LogicalAnd
            | BinaryOperator::LogicalOr
    )
}

fn compute_end_constants(
    program: &MirProgram,
    block_id: BasicBlockId,
) -> BTreeMap<MirLocalId, Constant> {
    let mut known: BTreeMap<MirLocalId, Constant> = BTreeMap::new();
    let Some(block) = program.block(block_id) else {
        return known;
    };
    for &inst_id in program.block_instructions(block) {
        let Some(inst) = program.instruction(inst_id) else {
            continue;
        };
        match inst.kind {
            InstructionKind::Assign { target, value } => {
                let folded = match value {
                    Rvalue::Use(op) => eval_op(&known, op),
                    Rvalue::Load(place) => program.place(place).and_then(|p| {
                        p.projections
                            .is_empty()
                            .then(|| known.get(&p.root).copied())
                            .flatten()
                    }),
                    Rvalue::Unary { operator, operand } => eval_op(&known, operand)
                        .and_then(|c| fold_unary(operator, &c))
                        .filter(is_primitive_constant),
                    Rvalue::Binary {
                        left,
                        operator,
                        right,
                    } => eval_op(&known, left)
                        .zip(eval_op(&known, right))
                        .and_then(|(l, r)| fold_binary(program, operator, &l, &r))
                        .filter(is_primitive_constant),
                    _ => None,
                };
                if let Some(c) = folded {
                    known.insert(target, c);
                } else {
                    known.remove(&target);
                }
            }
            InstructionKind::Store { place, value } => {
                if let Some(p) = program.place(place)
                    && p.projections.is_empty()
                {
                    if let Some(c) = eval_op(&known, value) {
                        known.insert(p.root, c);
                    } else {
                        known.remove(&p.root);
                    }
                }
            }
        }
    }
    known
}

fn thread_target(
    program: &MirProgram,
    start_bb: BasicBlockId,
    known: &BTreeMap<MirLocalId, Constant>,
    single_block_temps: &BTreeSet<MirLocalId>,
) -> BasicBlockId {
    let mut cur = start_bb;
    let mut env = known.clone();
    let mut visited = Vec::with_capacity(MAX_THREAD_DEPTH);
    for _ in 0..MAX_THREAD_DEPTH {
        if visited.contains(&cur) {
            break;
        }
        visited.push(cur);
        let Some(block) = program.block(cur) else {
            break;
        };
        let mut pure = true;
        for &inst_id in program.block_instructions(block) {
            let Some(inst) = program.instruction(inst_id) else {
                pure = false;
                break;
            };
            let InstructionKind::Assign { target, value } = inst.kind else {
                pure = false;
                break;
            };
            if !single_block_temps.contains(&target) {
                pure = false;
                break;
            }
            let val = match value {
                Rvalue::Use(op) => eval_op(&env, op),
                Rvalue::Load(place) => program.place(place).and_then(|p| {
                    p.projections
                        .is_empty()
                        .then(|| env.get(&p.root).copied())
                        .flatten()
                }),
                Rvalue::Binary {
                    left,
                    operator,
                    right,
                } if is_pure_branch_op(operator) => eval_op(&env, left)
                    .zip(eval_op(&env, right))
                    .and_then(|(l, r)| fold_binary(program, operator, &l, &r))
                    .filter(is_primitive_constant),
                Rvalue::Unary {
                    operator: UnaryOperator::Not,
                    operand,
                } => eval_op(&env, operand)
                    .and_then(|c| fold_unary(UnaryOperator::Not, &c))
                    .filter(is_primitive_constant),
                _ => None,
            };
            if let Some(c) = val {
                env.insert(target, c);
            } else {
                pure = false;
                break;
            }
        }
        if !pure {
            break;
        }
        match block.terminator {
            Terminator::Goto(next) => {
                cur = next;
            }
            Terminator::Branch {
                condition,
                then_block,
                else_block,
            } => match eval_op(&env, condition) {
                Some(Constant::Bool(true)) => cur = then_block,
                Some(Constant::Bool(false)) => cur = else_block,
                _ => break,
            },
            _ => break,
        }
    }
    cur
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use lpp_hir::{
        FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
        lower_package,
    };
    use lpp_mir::{MirBuildOptions, build_mir, verify_mir};
    use lpp_types::{ShadowInferenceOptions, infer_hir_package};

    use super::*;
    use crate::optimization_passes;

    #[derive(Debug)]
    struct MemoryFs(String);

    impl FileSystem for MemoryFs {
        fn is_file(&self, path: &Path) -> Result<bool, FileSystemError> {
            Ok(path == Path::new("/passes/main.lpp"))
        }

        fn canonicalize(&self, path: &Path) -> Result<PathBuf, FileSystemError> {
            Ok(path.to_owned())
        }

        fn read_to_string(&self, path: &Path) -> Result<String, FileSystemError> {
            if path == Path::new("/passes/main.lpp") {
                Ok(self.0.clone())
            } else {
                Err(FileSystemError::new("read", path, "not found"))
            }
        }
    }

    #[test]
    fn threads_state_machine_dispatch_loop_directly_to_leaf_blocks() {
        let src = "\
def machine(n: Int) -> Int:
    mut acc := 0
    mut _bb := 0
    while true:
        if _bb <= 1:
            if _bb <= 0:
                if n > 0:
                    acc = 10
                    _bb = 2
                    continue
                else:
                    acc = 20
                    _bb = 3
                    continue
            else:
                acc = acc + 1
                _bb = 3
                continue
        else:
            if _bb <= 2:
                acc = acc * 2
                _bb = 1
                continue
            else:
                return acc
    return 0
";
        let fs = MemoryFs(src.to_owned());
        let graph = GraphBuilder::new(&fs)
            .build(GraphRequest::new(
                "/passes/main.lpp",
                PackageSpec::new("passes", "/passes"),
            ))
            .expect("graph");
        let package = lower_package(&graph, ResolutionMode::Namespaced).expect("hir");
        let mut types = infer_hir_package(&package, ShadowInferenceOptions::default()).expect("ty");
        let mut mir = build_mir(
            &package,
            &graph.sources,
            &mut types,
            MirBuildOptions::default(),
        )
        .expect("mir");

        let outcome = optimization_passes()
            .run(&mut mir, &types.interner)
            .expect("passes");
        assert!(outcome.executed.contains(&"dispatch-thread"));
        assert!(verify_mir(&mir, &types.interner).is_empty());

        let func = mir.functions().next().expect("machine").1;
        let blocks = mir.function_blocks(func);
        let mut direct_gotos_past_header = 0usize;
        let entry_next = match mir.block(func.entry).expect("entry").terminator {
            Terminator::Goto(t) => t,
            other => panic!("expected entry goto, got {other:?}"),
        };
        assert_ne!(entry_next, func.entry);
        for &bb in blocks {
            if let Terminator::Goto(t) = mir.block(bb).expect("bb").terminator {
                if !mir
                    .block_instructions(mir.block(t).expect("tgt"))
                    .is_empty()
                {
                    direct_gotos_past_header += 1;
                }
            }
        }
        assert!(direct_gotos_past_header >= 1);
    }
}
