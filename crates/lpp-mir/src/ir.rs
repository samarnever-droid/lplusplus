use lpp_hir::{
    Arena, BinaryOperator, DefId, FieldId, HirItemId, InstanceId, LocalId, OriginId, Symbol,
    UnaryOperator, VariantId,
};
use lpp_types::{BuiltinId, TypeId, TypeListId};

use crate::ids::{
    BasicBlockId, InstructionId, MirAggregateId, MirFieldId, MirFunctionId, MirLocalId, MirPlaceId,
    MirStringId, MirVariantId,
};
use crate::storage::{ListRange, ListStore};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirLocalKind {
    Parameter,
    User,
    Temporary,
    Capture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirFunctionKind {
    Function,
    Closure,
    Async,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MirLocal {
    pub ty: TypeId,
    pub source: Option<LocalId>,
    pub kind: MirLocalKind,
    pub mutable: bool,
    pub origin: OriginId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirAggregateKind {
    Struct,
    Enum,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MirAggregate {
    pub source: HirItemId,
    pub definition: DefId,
    pub instance: Option<InstanceId>,
    pub arguments: TypeListId,
    pub ty: TypeId,
    pub kind: MirAggregateKind,
    pub fields: ListRange<MirFieldId>,
    pub variants: ListRange<MirVariantId>,
    pub origin: OriginId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MirField {
    pub aggregate: MirAggregateId,
    pub variant: Option<MirVariantId>,
    pub source: FieldId,
    pub ty: TypeId,
    pub origin: OriginId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MirVariant {
    pub aggregate: MirAggregateId,
    pub source: VariantId,
    pub ordinal: u32,
    pub fields: ListRange<MirFieldId>,
    pub origin: OriginId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Constant {
    Integer(i64),
    FloatBits(u64),
    /// The literal's source origin keeps MIR constants inside the existing
    /// size budgets; the origin table resolves it to a span for diagnostics.
    String {
        origin: OriginId,
        string: MirStringId,
    },
    Character {
        origin: OriginId,
        character: char,
    },
    Bool(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operand {
    Copy(MirLocalId),
    Function(MirFunctionId),
    Constant(Constant),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceProjection {
    Downcast(MirVariantId),
    Field(MirFieldId),
    TupleField(u32),
    ListIndex(Operand),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MirPlace {
    pub root: MirLocalId,
    pub projections: ListRange<PlaceProjection>,
    pub ty: TypeId,
    pub origin: OriginId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rvalue {
    Use(Operand),
    Unary {
        operator: UnaryOperator,
        operand: Operand,
    },
    Binary {
        left: Operand,
        operator: BinaryOperator,
        right: Operand,
    },
    Tuple(ListRange<Operand>),
    List(ListRange<Operand>),
    ConstructStruct {
        aggregate: MirAggregateId,
        fields: ListRange<Operand>,
    },
    ConstructVariant {
        aggregate: MirAggregateId,
        variant: MirVariantId,
        fields: ListRange<Operand>,
    },
    Load(MirPlaceId),
    ListLen(Operand),
    Call {
        callee: Operand,
        arguments: ListRange<Operand>,
    },
    Builtin {
        builtin: BuiltinId,
        arguments: ListRange<Operand>,
    },
    MakeClosure {
        function: MirFunctionId,
        captures: ListRange<Operand>,
    },
    Await(Operand),
    Spawn(Operand),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstructionKind {
    Assign { target: MirLocalId, value: Rvalue },
    Store { place: MirPlaceId, value: Operand },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instruction {
    pub kind: InstructionKind,
    pub origin: OriginId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Terminator {
    Goto(BasicBlockId),
    Branch {
        condition: Operand,
        then_block: BasicBlockId,
        else_block: BasicBlockId,
    },
    SwitchEnum {
        subject: Operand,
        aggregate: MirAggregateId,
        targets: ListRange<BasicBlockId>,
    },
    Return(Option<Operand>),
    Unreachable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BasicBlock {
    pub instructions: ListRange<InstructionId>,
    pub terminator: Terminator,
    pub origin: OriginId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MirFunction {
    pub source: HirItemId,
    pub instance: Option<InstanceId>,
    pub instance_arguments: TypeListId,
    pub definition: Option<DefId>,
    pub name: Option<Symbol>,
    pub kind: MirFunctionKind,
    /// For closures: the enclosing function's locals captured, in declaration
    /// order. Empty for plain and async functions.
    pub captures: ListRange<MirLocalId>,
    pub ty: TypeId,
    pub return_type: TypeId,
    pub parameters: ListRange<MirLocalId>,
    pub locals: ListRange<MirLocalId>,
    pub blocks: ListRange<BasicBlockId>,
    pub entry: BasicBlockId,
    pub origin: OriginId,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct MirLists {
    pub(crate) parameters: ListStore<MirLocalId>,
    pub(crate) locals: ListStore<MirLocalId>,
    pub(crate) blocks: ListStore<BasicBlockId>,
    pub(crate) switch_targets: ListStore<BasicBlockId>,
    pub(crate) instructions: ListStore<InstructionId>,
    pub(crate) operands: ListStore<Operand>,
    pub(crate) projections: ListStore<PlaceProjection>,
    pub(crate) fields: ListStore<MirFieldId>,
    pub(crate) variants: ListStore<MirVariantId>,
    pub(crate) captures: ListStore<MirLocalId>,
}

impl MirLists {
    pub(crate) fn with_capacity(local_count: usize, expression_count: usize) -> Self {
        Self {
            parameters: ListStore::with_capacity(local_count / 4),
            locals: ListStore::with_capacity(local_count.saturating_add(expression_count)),
            blocks: ListStore::with_capacity(expression_count / 2),
            switch_targets: ListStore::with_capacity(expression_count / 4),
            instructions: ListStore::with_capacity(expression_count),
            operands: ListStore::with_capacity(expression_count),
            projections: ListStore::with_capacity(expression_count),
            fields: ListStore::with_capacity(local_count / 4),
            variants: ListStore::with_capacity(local_count / 8),
            captures: ListStore::with_capacity(local_count / 16),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirProgram {
    pub(crate) functions: Arena<MirFunctionId, MirFunction>,
    pub(crate) aggregates: Arena<MirAggregateId, MirAggregate>,
    pub(crate) fields: Arena<MirFieldId, MirField>,
    pub(crate) variants: Arena<MirVariantId, MirVariant>,
    pub(crate) blocks: Arena<BasicBlockId, BasicBlock>,
    pub(crate) locals: Arena<MirLocalId, MirLocal>,
    pub(crate) places: Arena<MirPlaceId, MirPlace>,
    pub(crate) instructions: Arena<InstructionId, Instruction>,
    pub(crate) strings: Arena<MirStringId, String>,
    pub(crate) lists: MirLists,
}

impl MirProgram {
    #[must_use]
    pub fn new() -> Self {
        Self {
            functions: Arena::new(),
            aggregates: Arena::new(),
            fields: Arena::new(),
            variants: Arena::new(),
            blocks: Arena::new(),
            locals: Arena::new(),
            places: Arena::new(),
            instructions: Arena::new(),
            strings: Arena::new(),
            lists: MirLists::default(),
        }
    }

    pub(crate) fn with_capacity(
        function_count: usize,
        local_count: usize,
        expression_count: usize,
        statement_count: usize,
    ) -> Self {
        Self {
            functions: Arena::with_capacity(function_count),
            aggregates: Arena::with_capacity(function_count / 2),
            fields: Arena::with_capacity(local_count / 4),
            variants: Arena::with_capacity(function_count / 4),
            blocks: Arena::with_capacity(statement_count.saturating_mul(2).max(function_count)),
            locals: Arena::with_capacity(local_count.saturating_add(expression_count)),
            places: Arena::with_capacity(expression_count),
            instructions: Arena::with_capacity(expression_count.saturating_add(local_count)),
            strings: Arena::with_capacity(expression_count / 8),
            lists: MirLists::with_capacity(local_count, expression_count),
        }
    }

    #[must_use]
    pub fn function_count(&self) -> usize {
        self.functions.len()
    }

    #[must_use]
    pub fn string_count(&self) -> usize {
        self.strings.len()
    }

    #[must_use]
    pub fn aggregate_count(&self) -> usize {
        self.aggregates.len()
    }

    #[must_use]
    pub fn field_count(&self) -> usize {
        self.fields.len()
    }

    #[must_use]
    pub fn variant_count(&self) -> usize {
        self.variants.len()
    }

    #[must_use]
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    #[must_use]
    pub fn local_count(&self) -> usize {
        self.locals.len()
    }

    #[must_use]
    pub fn place_count(&self) -> usize {
        self.places.len()
    }

    #[must_use]
    pub fn projection_count(&self) -> usize {
        self.lists.projections.len()
    }

    #[must_use]
    pub fn switch_target_count(&self) -> usize {
        self.lists.switch_targets.len()
    }

    #[must_use]
    pub fn instruction_count(&self) -> usize {
        self.instructions.len()
    }

    pub fn functions(&self) -> impl ExactSizeIterator<Item = (MirFunctionId, &MirFunction)> {
        self.functions.enumerate()
    }

    pub fn aggregates(&self) -> impl ExactSizeIterator<Item = (MirAggregateId, &MirAggregate)> {
        self.aggregates.enumerate()
    }

    pub fn fields(&self) -> impl ExactSizeIterator<Item = (MirFieldId, &MirField)> {
        self.fields.enumerate()
    }

    pub fn variants(&self) -> impl ExactSizeIterator<Item = (MirVariantId, &MirVariant)> {
        self.variants.enumerate()
    }

    pub fn blocks(&self) -> impl ExactSizeIterator<Item = (BasicBlockId, &BasicBlock)> {
        self.blocks.enumerate()
    }

    pub fn locals(&self) -> impl ExactSizeIterator<Item = (MirLocalId, &MirLocal)> {
        self.locals.enumerate()
    }

    pub fn places(&self) -> impl ExactSizeIterator<Item = (MirPlaceId, &MirPlace)> {
        self.places.enumerate()
    }

    pub fn instructions(&self) -> impl ExactSizeIterator<Item = (InstructionId, &Instruction)> {
        self.instructions.enumerate()
    }

    #[must_use]
    pub fn function(&self, id: MirFunctionId) -> Option<&MirFunction> {
        self.functions.get(id)
    }

    #[must_use]
    pub fn string(&self, id: MirStringId) -> Option<&String> {
        self.strings.get(id)
    }

    pub fn function_mut(&mut self, id: MirFunctionId) -> Option<&mut MirFunction> {
        self.functions.get_mut(id)
    }

    #[must_use]
    pub fn aggregate(&self, id: MirAggregateId) -> Option<&MirAggregate> {
        self.aggregates.get(id)
    }

    pub fn aggregate_mut(&mut self, id: MirAggregateId) -> Option<&mut MirAggregate> {
        self.aggregates.get_mut(id)
    }

    #[must_use]
    pub fn field(&self, id: MirFieldId) -> Option<&MirField> {
        self.fields.get(id)
    }

    #[must_use]
    pub fn variant(&self, id: MirVariantId) -> Option<&MirVariant> {
        self.variants.get(id)
    }

    pub fn variant_mut(&mut self, id: MirVariantId) -> Option<&mut MirVariant> {
        self.variants.get_mut(id)
    }

    #[must_use]
    pub fn block(&self, id: BasicBlockId) -> Option<&BasicBlock> {
        self.blocks.get(id)
    }

    pub fn block_mut(&mut self, id: BasicBlockId) -> Option<&mut BasicBlock> {
        self.blocks.get_mut(id)
    }

    #[must_use]
    pub fn local(&self, id: MirLocalId) -> Option<&MirLocal> {
        self.locals.get(id)
    }

    pub fn local_mut(&mut self, id: MirLocalId) -> Option<&mut MirLocal> {
        self.locals.get_mut(id)
    }

    #[must_use]
    pub fn place(&self, id: MirPlaceId) -> Option<&MirPlace> {
        self.places.get(id)
    }

    #[must_use]
    pub fn instruction(&self, id: InstructionId) -> Option<&Instruction> {
        self.instructions.get(id)
    }

    pub fn place_mut(&mut self, id: MirPlaceId) -> Option<&mut MirPlace> {
        self.places.get_mut(id)
    }

    pub fn instruction_mut(&mut self, id: InstructionId) -> Option<&mut Instruction> {
        self.instructions.get_mut(id)
    }

    #[must_use]
    pub fn function_parameters(&self, function: &MirFunction) -> &[MirLocalId] {
        self.lists.parameters.get(function.parameters)
    }

    #[must_use]
    pub fn function_captures(&self, function: &MirFunction) -> &[MirLocalId] {
        self.lists.captures.get(function.captures)
    }

    #[must_use]
    pub fn function_locals(&self, function: &MirFunction) -> &[MirLocalId] {
        self.lists.locals.get(function.locals)
    }

    #[must_use]
    pub fn function_blocks(&self, function: &MirFunction) -> &[BasicBlockId] {
        self.lists.blocks.get(function.blocks)
    }

    #[must_use]
    pub fn block_instructions(&self, block: &BasicBlock) -> &[InstructionId] {
        self.lists.instructions.get(block.instructions)
    }

    #[must_use]
    pub fn switch_targets(&self, range: ListRange<BasicBlockId>) -> &[BasicBlockId] {
        self.lists.switch_targets.get(range)
    }

    pub fn switch_targets_mut(&mut self, range: ListRange<BasicBlockId>) -> &mut [BasicBlockId] {
        self.lists.switch_targets.get_mut(range)
    }

    #[must_use]
    pub fn operands(&self, range: ListRange<Operand>) -> &[Operand] {
        self.lists.operands.get(range)
    }

    #[must_use]
    pub fn place_projections(&self, place: &MirPlace) -> &[PlaceProjection] {
        self.lists.projections.get(place.projections)
    }

    pub fn place_projections_mut(&mut self, place: MirPlaceId) -> Option<&mut [PlaceProjection]> {
        let projections = self.places.get(place)?.projections;
        Some(self.lists.projections.get_mut(projections))
    }

    #[must_use]
    pub fn aggregate_fields(&self, aggregate: &MirAggregate) -> &[MirFieldId] {
        self.lists.fields.get(aggregate.fields)
    }

    #[must_use]
    pub fn aggregate_variants(&self, aggregate: &MirAggregate) -> &[MirVariantId] {
        self.lists.variants.get(aggregate.variants)
    }

    #[must_use]
    pub fn variant_fields(&self, variant: &MirVariant) -> &[MirFieldId] {
        self.lists.fields.get(variant.fields)
    }

    #[must_use]
    pub fn list_entry_count(&self) -> usize {
        self.lists.parameters.len()
            + self.lists.locals.len()
            + self.lists.blocks.len()
            + self.lists.switch_targets.len()
            + self.lists.instructions.len()
            + self.lists.operands.len()
            + self.lists.projections.len()
            + self.lists.fields.len()
            + self.lists.variants.len()
            + self.lists.captures.len()
    }
}

impl Default for MirProgram {
    fn default() -> Self {
        Self::new()
    }
}
