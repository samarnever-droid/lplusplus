use std::collections::VecDeque;

use lpp_hir::ArenaId;

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefiniteInitializationLimits {
    pub max_state_words: usize,
    pub max_iterations: usize,
}

impl Default for DefiniteInitializationLimits {
    fn default() -> Self {
        Self {
            max_state_words: 16_000_000,
            max_iterations: 1_000_000,
        }
    }
}

pub(super) fn verify_definite_initialization(
    program: &MirProgram,
    limits: DefiniteInitializationLimits,
) -> Vec<MirVerificationError> {
    let mut errors = Vec::new();
    for (function_id, function) in program.functions() {
        analyze_function(program, function_id, function, limits, &mut errors);
    }
    errors
}

fn analyze_function(
    program: &MirProgram,
    function_id: MirFunctionId,
    function: &crate::MirFunction,
    limits: DefiniteInitializationLimits,
    errors: &mut Vec<MirVerificationError>,
) {
    let locals = program.function_locals(function);
    let blocks = program.function_blocks(function);
    let word_count = locals.len().div_ceil(64);
    let required_words = word_count
        .checked_mul(blocks.len())
        .and_then(|words| words.checked_mul(2));
    if required_words.is_none_or(|required| required > limits.max_state_words) {
        errors.push(MirVerificationError {
            function: Some(function_id),
            block: None,
            instruction: None,
            origin: function.origin,
            kind: MirVerificationErrorKind::DefiniteInitializationLimit {
                resource: DefiniteInitializationResource::StateWords,
                required: required_words.unwrap_or(usize::MAX),
                limit: limits.max_state_words,
            },
        });
        return;
    }

    let mut local_positions = vec![usize::MAX; program.local_count()];
    for (position, local) in locals.iter().enumerate() {
        local_positions[local.index()] = position;
    }
    let mut block_positions = vec![usize::MAX; program.block_count()];
    for (position, block) in blocks.iter().enumerate() {
        block_positions[block.index()] = position;
    }
    let entry = block_positions[function.entry.index()];
    let mut successors = vec![Vec::new(); blocks.len()];
    let mut predecessors = vec![Vec::new(); blocks.len()];
    for (position, block_id) in blocks.iter().enumerate() {
        let block = program
            .block(*block_id)
            .expect("structural verification precedes definite initialization");
        for successor in terminator_successors(program, block.terminator) {
            let target = block_positions[successor.index()];
            successors[position].push(target);
            predecessors[target].push(position);
        }
    }

    let mut reachable = vec![false; blocks.len()];
    let mut queue = VecDeque::from([entry]);
    reachable[entry] = true;
    while let Some(block) = queue.pop_front() {
        for successor in &successors[block] {
            if !reachable[*successor] {
                reachable[*successor] = true;
                queue.push_back(*successor);
            }
        }
    }

    let mut entry_state = DenseBitSet::empty(locals.len());
    for parameter in program.function_parameters(function) {
        entry_state.insert(local_positions[parameter.index()]);
    }
    let top = DenseBitSet::full(locals.len());
    let mut inputs = vec![top.clone(); blocks.len()];
    let mut outputs = vec![top.clone(); blocks.len()];
    inputs[entry] = entry_state;

    let mut iterations = 0usize;
    loop {
        if iterations >= limits.max_iterations {
            errors.push(MirVerificationError {
                function: Some(function_id),
                block: None,
                instruction: None,
                origin: function.origin,
                kind: MirVerificationErrorKind::DefiniteInitializationLimit {
                    resource: DefiniteInitializationResource::Iterations,
                    required: iterations.saturating_add(1),
                    limit: limits.max_iterations,
                },
            });
            return;
        }
        iterations += 1;
        let mut changed = false;
        for position in 0..blocks.len() {
            if !reachable[position] {
                continue;
            }
            let next_input = if position == entry {
                inputs[entry].clone()
            } else {
                let mut incoming = top.clone();
                for predecessor in predecessors[position]
                    .iter()
                    .copied()
                    .filter(|predecessor| reachable[*predecessor])
                {
                    incoming.intersect_with(&outputs[predecessor]);
                }
                incoming
            };
            let mut next_output = next_input.clone();
            let block = program.block(blocks[position]).unwrap();
            for instruction_id in program.block_instructions(block) {
                let instruction = program.instruction(*instruction_id).unwrap();
                if let InstructionKind::Assign { target, .. } = instruction.kind {
                    next_output.insert(local_positions[target.index()]);
                }
            }
            if inputs[position] != next_input {
                inputs[position] = next_input;
                changed = true;
            }
            if outputs[position] != next_output {
                outputs[position] = next_output;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    for (position, block_id) in blocks.iter().enumerate() {
        if !reachable[position] {
            continue;
        }
        let block = program.block(*block_id).unwrap();
        let mut initialized = inputs[position].clone();
        for instruction_id in program.block_instructions(block) {
            let instruction = program.instruction(*instruction_id).unwrap();
            report_uninitialized_reads(
                ReadSite {
                    function: function_id,
                    block: *block_id,
                    instruction: Some(*instruction_id),
                    origin: instruction.origin,
                },
                instruction_operands(program, instruction.kind),
                &initialized,
                &local_positions,
                errors,
            );
            if let InstructionKind::Assign { target, .. } = instruction.kind {
                initialized.insert(local_positions[target.index()]);
            }
        }
        report_uninitialized_reads(
            ReadSite {
                function: function_id,
                block: *block_id,
                instruction: None,
                origin: block.origin,
            },
            terminator_operands(block.terminator),
            &initialized,
            &local_positions,
            errors,
        );
    }
}

#[derive(Clone, Copy)]
struct ReadSite {
    function: MirFunctionId,
    block: BasicBlockId,
    instruction: Option<InstructionId>,
    origin: OriginId,
}

fn report_uninitialized_reads(
    site: ReadSite,
    operands: impl IntoIterator<Item = Operand>,
    initialized: &DenseBitSet,
    positions: &[usize],
    errors: &mut Vec<MirVerificationError>,
) {
    for operand in operands {
        if let Operand::Copy(local) = operand
            && !initialized.contains(positions[local.index()])
        {
            errors.push(MirVerificationError {
                function: Some(site.function),
                block: Some(site.block),
                instruction: site.instruction,
                origin: site.origin,
                kind: MirVerificationErrorKind::UninitializedRead(local),
            });
        }
    }
}

fn instruction_operands(program: &MirProgram, instruction: InstructionKind) -> Vec<Operand> {
    match instruction {
        InstructionKind::Assign { value, .. } => rvalue_operands(program, value),
        InstructionKind::Store { place, value } => {
            let mut operands = place_operands(program, place);
            operands.push(value);
            operands
        }
    }
}

fn rvalue_operands(program: &MirProgram, value: Rvalue) -> Vec<Operand> {
    match value {
        Rvalue::Use(operand) | Rvalue::Unary { operand, .. } | Rvalue::ListLen(operand) => {
            vec![operand]
        }
        Rvalue::Binary { left, right, .. } => vec![left, right],
        Rvalue::Tuple(operands)
        | Rvalue::List(operands)
        | Rvalue::ConstructStruct {
            fields: operands, ..
        }
        | Rvalue::ConstructVariant {
            fields: operands, ..
        } => program.operands(operands).to_vec(),
        Rvalue::Load(place) => place_operands(program, place),
        Rvalue::Call { callee, arguments } => {
            let mut operands = Vec::with_capacity(arguments.len().saturating_add(1));
            operands.push(callee);
            operands.extend_from_slice(program.operands(arguments));
            operands
        }
        Rvalue::Builtin {
            builtin: _,
            arguments,
        } => program.operands(arguments).to_vec(),
        Rvalue::MakeClosure {
            function: _,
            captures,
        } => program.operands(captures).to_vec(),
        Rvalue::Await(operand) | Rvalue::Spawn(operand) => vec![operand],
    }
}

fn place_operands(program: &MirProgram, place: MirPlaceId) -> Vec<Operand> {
    let place = program
        .place(place)
        .expect("structural verification precedes definite initialization");
    let mut operands = vec![Operand::Copy(place.root)];
    operands.extend(program.place_projections(place).iter().filter_map(
        |projection| match projection {
            PlaceProjection::ListIndex(index) => Some(*index),
            PlaceProjection::Downcast(_)
            | PlaceProjection::Field(_)
            | PlaceProjection::TupleField(_) => None,
        },
    ));
    operands
}

fn terminator_operands(terminator: Terminator) -> Vec<Operand> {
    match terminator {
        Terminator::Branch { condition, .. } => vec![condition],
        Terminator::SwitchEnum { subject, .. } => vec![subject],
        Terminator::Return(Some(value)) => vec![value],
        Terminator::Goto(_) | Terminator::Return(None) | Terminator::Unreachable => Vec::new(),
    }
}

fn terminator_successors(program: &MirProgram, terminator: Terminator) -> Vec<BasicBlockId> {
    match terminator {
        Terminator::Goto(target) => vec![target],
        Terminator::Branch {
            then_block,
            else_block,
            ..
        } => vec![then_block, else_block],
        Terminator::SwitchEnum { targets, .. } => program.switch_targets(targets).to_vec(),
        Terminator::Return(_) | Terminator::Unreachable => Vec::new(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DenseBitSet {
    words: Vec<u64>,
}

impl DenseBitSet {
    fn empty(bits: usize) -> Self {
        Self {
            words: vec![0; bits.div_ceil(64)],
        }
    }

    fn full(bits: usize) -> Self {
        let mut set = Self {
            words: vec![u64::MAX; bits.div_ceil(64)],
        };
        if let Some(last) = set.words.last_mut()
            && !bits.is_multiple_of(64)
        {
            *last = (1_u64 << (bits % 64)) - 1;
        }
        set
    }

    fn insert(&mut self, bit: usize) {
        self.words[bit / 64] |= 1_u64 << (bit % 64);
    }

    fn contains(&self, bit: usize) -> bool {
        self.words[bit / 64] & (1_u64 << (bit % 64)) != 0
    }

    fn intersect_with(&mut self, other: &Self) {
        for (word, other) in self.words.iter_mut().zip(&other.words) {
            *word &= *other;
        }
    }
}
