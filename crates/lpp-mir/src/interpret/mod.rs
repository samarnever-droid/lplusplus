mod eval;

use std::fmt;

use lpp_hir::OriginId;
use lpp_types::{PrimitiveType, TypeId, TypeInterner, TypeKind};

use crate::{
    BasicBlockId, MirAggregateId, MirFunctionId, MirLocalId, MirProgram, MirVariantId, verify_mir,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionValue {
    Void,
    Bool(bool),
    Int(i64),
    FloatBits(u64),
    String(String),
    Char(char),
    Tuple(Vec<ExecutionValue>),
    List(Vec<ExecutionValue>),
    Nominal {
        aggregate: MirAggregateId,
        variant: Option<MirVariantId>,
        fields: Vec<ExecutionValue>,
    },
    Function(MirFunctionId),
    Closure {
        function: MirFunctionId,
        captures: Vec<ExecutionValue>,
    },
    Task {
        function: MirFunctionId,
        arguments: Vec<ExecutionValue>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionValueKind {
    Void,
    Bool,
    Int,
    Float,
    String,
    Char,
    Tuple,
    List,
    Nominal,
    Function,
    Closure,
    Task,
}

impl ExecutionValue {
    #[must_use]
    pub const fn kind(&self) -> ExecutionValueKind {
        match self {
            Self::Void => ExecutionValueKind::Void,
            Self::Bool(_) => ExecutionValueKind::Bool,
            Self::Int(_) => ExecutionValueKind::Int,
            Self::FloatBits(_) => ExecutionValueKind::Float,
            Self::String(_) => ExecutionValueKind::String,
            Self::Char(_) => ExecutionValueKind::Char,
            Self::Tuple(_) => ExecutionValueKind::Tuple,
            Self::List(_) => ExecutionValueKind::List,
            Self::Nominal { .. } => ExecutionValueKind::Nominal,
            Self::Function(_) => ExecutionValueKind::Function,
            Self::Closure { .. } => ExecutionValueKind::Closure,
            Self::Task { .. } => ExecutionValueKind::Task,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterpreterLimits {
    pub max_steps: usize,
    pub max_call_depth: usize,
    pub max_aggregate_elements: usize,
    pub max_heap_nodes: usize,
}

impl Default for InterpreterLimits {
    fn default() -> Self {
        Self {
            max_steps: 1_000_000,
            max_call_depth: 256,
            max_aggregate_elements: 1_000_000,
            max_heap_nodes: 1_000_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionStats {
    pub steps: usize,
    pub calls: usize,
    pub peak_call_depth: usize,
    pub aggregate_elements: usize,
    pub heap_nodes: usize,
}

/// Accounted ARC traffic for one `execute_mir_arc` run. The legacy
/// entries report `arc: None` and keep their exact behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArcStats {
    /// Borrow transfers that gained a reference (`list_get` results,
    /// `await` results, `MakeClosure` captures, `Use` reads that create
    /// a new owner, entry string-cache hits).
    pub retains: usize,
    /// Reference drops, including the drops that trigger frees.
    pub releases: usize,
    /// Heap nodes whose count reached zero and were deallocated.
    pub frees: usize,
    /// Pinned (cycle-member) nodes still alive at end of execution;
    /// pinned nodes are reported, never leaked.
    pub pinned_live: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionOutcome {
    pub value: ExecutionValue,
    /// Printed output, in emission order. Each entry is one builtin print
    /// operation (including its trailing newline, matching v1 `print_str`).
    pub output: Vec<String>,
    pub stats: ExecutionStats,
    /// ARC accounting; `Some` only for `execute_mir_arc`.
    pub arc: Option<ArcStats>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterpreterLimit {
    Steps,
    CallDepth,
    AggregateElements,
    HeapNodes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterpreterErrorKind {
    InvalidMir {
        diagnostics: usize,
    },
    MissingFunction(MirFunctionId),
    ArityMismatch {
        expected: usize,
        actual: usize,
    },
    ValueTypeMismatch {
        expected: TypeId,
        actual: ExecutionValueKind,
    },
    UninitializedLocal(MirLocalId),
    InvalidCondition(ExecutionValueKind),
    InvalidUnaryOperands,
    InvalidBinaryOperands,
    InvalidCallee(ExecutionValueKind),
    InvalidAggregateProjection,
    InvalidListProjection,
    IndexOutOfBounds {
        index: i64,
        len: usize,
    },
    UnsupportedConstant,
    UnsupportedBuiltin,
    InvalidBuiltinArgument {
        argument: usize,
        actual: ExecutionValueKind,
    },
    InvalidAwaitTarget(ExecutionValueKind),
    InvalidTaskProjection,
    DivisionByZero,
    IntegerOverflow,
    ReachedUnreachable,
    LimitExceeded(InterpreterLimit),
    /// A release or free found no live reference (refcount underflow or
    /// double free).
    OwnershipUnderflow,
    /// Non-pinned heap nodes survived end of execution with a live
    /// reference.
    OwnershipLeak {
        nodes: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterpreterError {
    pub function: Option<MirFunctionId>,
    pub block: Option<BasicBlockId>,
    pub origin: Option<OriginId>,
    pub kind: InterpreterErrorKind,
}

impl InterpreterError {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self.kind {
            InterpreterErrorKind::InvalidMir { .. }
            | InterpreterErrorKind::MissingFunction(_)
            | InterpreterErrorKind::ArityMismatch { .. }
            | InterpreterErrorKind::ValueTypeMismatch { .. } => "E4301",
            InterpreterErrorKind::LimitExceeded(_) => "E4302",
            InterpreterErrorKind::UninitializedLocal(_)
            | InterpreterErrorKind::InvalidCondition(_)
            | InterpreterErrorKind::InvalidUnaryOperands
            | InterpreterErrorKind::InvalidBinaryOperands
            | InterpreterErrorKind::InvalidCallee(_)
            | InterpreterErrorKind::InvalidAggregateProjection
            | InterpreterErrorKind::InvalidListProjection
            | InterpreterErrorKind::IndexOutOfBounds { .. }
            | InterpreterErrorKind::UnsupportedConstant
            | InterpreterErrorKind::UnsupportedBuiltin
            | InterpreterErrorKind::InvalidBuiltinArgument { .. }
            | InterpreterErrorKind::InvalidAwaitTarget(_)
            | InterpreterErrorKind::InvalidTaskProjection
            | InterpreterErrorKind::DivisionByZero
            | InterpreterErrorKind::IntegerOverflow
            | InterpreterErrorKind::ReachedUnreachable => "E4303",
            InterpreterErrorKind::OwnershipUnderflow
            | InterpreterErrorKind::OwnershipLeak { .. } => "E4304",
        }
    }
}

impl fmt::Display for InterpreterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {:?}", self.code(), self.kind)
    }
}

impl std::error::Error for InterpreterError {}

pub fn execute_mir(
    program: &MirProgram,
    types: &TypeInterner,
    entry: MirFunctionId,
    arguments: &[ExecutionValue],
    limits: InterpreterLimits,
) -> Result<ExecutionValue, InterpreterError> {
    execute_mir_with_stats(program, types, entry, arguments, limits).map(|outcome| outcome.value)
}

pub fn execute_mir_with_stats(
    program: &MirProgram,
    types: &TypeInterner,
    entry: MirFunctionId,
    arguments: &[ExecutionValue],
    limits: InterpreterLimits,
) -> Result<ExecutionOutcome, InterpreterError> {
    let diagnostics = verify_mir(program, types);
    if !diagnostics.is_empty() {
        return Err(InterpreterError {
            function: None,
            block: None,
            origin: None,
            kind: InterpreterErrorKind::InvalidMir {
                diagnostics: diagnostics.len(),
            },
        });
    }
    execute_with(program, types, entry, arguments, limits, None)
}

/// Execute the program with ARC: refcounted heap values, the
/// consume-vs-borrow call contract, recursive free at count zero, and
/// the end-of-execution balance proof. `pinned` is the ownership plan's
/// cycle-member type ids (`OwnershipPlan::pinned_types`): heap nodes of
/// those types are pinned — never freed, reported as `pinned_live`.
pub fn execute_mir_arc(
    program: &MirProgram,
    types: &TypeInterner,
    entry: MirFunctionId,
    arguments: &[ExecutionValue],
    limits: InterpreterLimits,
    pinned: &[TypeId],
) -> Result<ExecutionOutcome, InterpreterError> {
    let mut pinned_set = std::collections::BTreeSet::new();
    pinned_set.extend(pinned.iter().copied());
    let mut interpreter = Interpreter {
        program,
        types,
        limits,
        steps: 0,
        calls: 0,
        peak_call_depth: 0,
        aggregate_elements: 0,
        heap: Vec::new(),
        string_cache: std::collections::BTreeMap::new(),
        output: Vec::new(),
        arc: Some(ArcState {
            refcounts: Vec::new(),
            node_types: Vec::new(),
            pinned: pinned_set,
            retains: 0,
            releases: 0,
            frees: 0,
        }),
    };
    let value = interpreter.execute_entry(entry, arguments)?;
    // Balance proof: the string cache releases its references, then
    // every non-pinned heap node must be dead.
    interpreter.finish_arc()?;
    // Pinned live nodes: still counted and unrecorded-or-cycle-member.
    let pinned_live = interpreter
        .arc
        .as_ref()
        .expect("arc entries keep arc state")
        .refcounts
        .iter()
        .enumerate()
        .filter(|(index, count)| **count > 0 && interpreter.node_is_pinned(HeapId(*index as u32)))
        .count();
    let arc_state = interpreter.arc.take().expect("arc entries keep arc state");
    Ok(ExecutionOutcome {
        value,
        output: interpreter.output,
        stats: ExecutionStats {
            steps: interpreter.steps,
            calls: interpreter.calls,
            peak_call_depth: interpreter.peak_call_depth,
            aggregate_elements: interpreter.aggregate_elements,
            heap_nodes: interpreter.heap.len(),
        },
        arc: Some(ArcStats {
            retains: arc_state.retains,
            releases: arc_state.releases,
            frees: arc_state.frees,
            pinned_live,
        }),
    })
}

fn execute_with(
    program: &MirProgram,
    types: &TypeInterner,
    entry: MirFunctionId,
    arguments: &[ExecutionValue],
    limits: InterpreterLimits,
    arc: Option<ArcState>,
) -> Result<ExecutionOutcome, InterpreterError> {
    let diagnostics = verify_mir(program, types);
    if !diagnostics.is_empty() {
        return Err(InterpreterError {
            function: None,
            block: None,
            origin: None,
            kind: InterpreterErrorKind::InvalidMir {
                diagnostics: diagnostics.len(),
            },
        });
    }
    let mut interpreter = Interpreter {
        program,
        types,
        limits,
        steps: 0,
        calls: 0,
        peak_call_depth: 0,
        aggregate_elements: 0,
        heap: Vec::new(),
        string_cache: std::collections::BTreeMap::new(),
        output: Vec::new(),
        arc,
    };
    let value = interpreter.execute_entry(entry, arguments)?;
    Ok(ExecutionOutcome {
        value,
        output: interpreter.output,
        stats: ExecutionStats {
            steps: interpreter.steps,
            calls: interpreter.calls,
            peak_call_depth: interpreter.peak_call_depth,
            aggregate_elements: interpreter.aggregate_elements,
            heap_nodes: interpreter.heap.len(),
        },
        arc: None,
    })
}

/// How a heap node participates in the pinned set:
/// - `Unknown`: the allocation site has no type information; the node
///   is conservatively pinned (reported, never leaked);
/// - `Ephemeral`: the node cannot be a cycle member (string literals,
///   detached task nodes with plain contents);
/// - `Of(ty)`: the node pins exactly when the 4D plan marks `ty` a
///   cycle member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NodePin {
    Unknown,
    Ephemeral,
    Of(TypeId),
}

/// Per-execution ARC state. The refcount table runs in lockstep with
/// the heap (indexed by `HeapId`); the pin table records each node's
/// participation in the pinned set (4D `Shared` type ids).
#[derive(Debug)]
struct ArcState {
    /// Live reference counts; zero means deallocated.
    refcounts: Vec<u32>,
    /// Each node's pin classification, when the allocation site knows
    /// it.
    node_types: Vec<NodePin>,
    /// Type ids of the 4D cycle members; nodes of these types are
    /// pinned (never freed, reported as `pinned_live`).
    pinned: std::collections::BTreeSet<TypeId>,
    retains: usize,
    releases: usize,
    frees: usize,
}

struct Interpreter<'input> {
    program: &'input MirProgram,
    types: &'input TypeInterner,
    limits: InterpreterLimits,
    steps: usize,
    calls: usize,
    peak_call_depth: usize,
    aggregate_elements: usize,
    heap: Vec<HeapNode>,
    string_cache: std::collections::BTreeMap<crate::MirStringId, HeapId>,
    output: Vec<String>,
    /// `Some` for `execute_mir_arc`; the legacy entries run without
    /// refcounts and prove nothing.
    arc: Option<ArcState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct HeapId(u32);

#[derive(Debug, Clone, PartialEq, Eq)]
enum RuntimeValue {
    Void,
    Bool(bool),
    Int(i64),
    FloatBits(u64),
    String(HeapId),
    Char(char),
    Tuple(Vec<RuntimeValue>),
    List(HeapId),
    /// Opaque map handle. The current frontend models legacy map handles as
    /// `Int`, but the interpreter keeps the allocation typed and ARC-managed.
    Map(HeapId),
    /// Borrowed list/string window. It does not own `source`; frontend
    /// borrow checking guarantees the source outlives the view.
    Slice {
        source: HeapId,
        start: usize,
        len: usize,
        string: bool,
    },
    Nominal(HeapId),
    Function(MirFunctionId),
    Closure(HeapId),
    Task(HeapId),
}

impl RuntimeValue {
    const fn kind(&self) -> ExecutionValueKind {
        match self {
            Self::Void => ExecutionValueKind::Void,
            Self::Bool(_) => ExecutionValueKind::Bool,
            Self::Int(_) => ExecutionValueKind::Int,
            Self::FloatBits(_) => ExecutionValueKind::Float,
            Self::String(_) => ExecutionValueKind::String,
            Self::Char(_) => ExecutionValueKind::Char,
            Self::Tuple(_) => ExecutionValueKind::Tuple,
            Self::List(_) => ExecutionValueKind::List,
            Self::Map(_) => ExecutionValueKind::Int,
            Self::Slice { .. } => ExecutionValueKind::Tuple,
            Self::Nominal(_) => ExecutionValueKind::Nominal,
            Self::Function(_) => ExecutionValueKind::Function,
            Self::Closure(_) => ExecutionValueKind::Closure,
            Self::Task(_) => ExecutionValueKind::Task,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MapKey {
    Int(i64),
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HeapNode {
    List(Vec<RuntimeValue>),
    Map(Vec<(MapKey, i64)>),
    Nominal {
        aggregate: MirAggregateId,
        variant: Option<MirVariantId>,
        fields: Vec<RuntimeValue>,
    },
    String(String),
    Closure {
        function: MirFunctionId,
        captures: Vec<RuntimeValue>,
    },
    Task {
        function: MirFunctionId,
        arguments: Vec<RuntimeValue>,
        /// The closure heap node the task was spawned from, so capture
        /// mutations persist into the closure's cells.
        closure: Option<HeapId>,
        result: Option<RuntimeValue>,
    },
}

struct Frame {
    local_ids: Vec<MirLocalId>,
    values: Vec<Option<RuntimeValue>>,
}

impl Frame {
    fn new(local_ids: &[MirLocalId]) -> Self {
        Self {
            local_ids: local_ids.to_vec(),
            values: vec![None; local_ids.len()],
        }
    }

    fn position(&self, local: MirLocalId) -> usize {
        self.local_ids
            .binary_search(&local)
            .expect("verified function-local IDs are sorted and complete")
    }

    fn get(&self, local: MirLocalId) -> Option<&RuntimeValue> {
        self.values[self.position(local)].as_ref()
    }

    fn set(&mut self, local: MirLocalId, value: RuntimeValue) {
        let position = self.position(local);
        self.values[position] = Some(value);
    }

    /// Consume the slot: return its value and leave it empty. A moved-out
    /// slot emits no release at function exit (move-out optimization).
    fn take(&mut self, local: MirLocalId) -> Option<RuntimeValue> {
        let position = self.position(local);
        self.values[position].take()
    }
}

fn value_matches_type(
    value: &ExecutionValue,
    ty: TypeId,
    types: &TypeInterner,
    program: &MirProgram,
) -> bool {
    let mut pending = vec![(value, ty)];
    while let Some((value, ty)) = pending.pop() {
        match (value, types.kind(ty)) {
            (ExecutionValue::Void, TypeKind::Primitive(PrimitiveType::Void))
            | (ExecutionValue::Bool(_), TypeKind::Primitive(PrimitiveType::Bool))
            | (ExecutionValue::Int(_), TypeKind::Primitive(PrimitiveType::Int))
            | (ExecutionValue::FloatBits(_), TypeKind::Primitive(PrimitiveType::Float))
            | (ExecutionValue::String(_), TypeKind::Primitive(PrimitiveType::String))
            | (ExecutionValue::Char(_), TypeKind::Primitive(PrimitiveType::Char)) => {}
            (ExecutionValue::Tuple(values), TypeKind::Tuple(elements)) => {
                let elements = types.list(elements);
                if values.len() != elements.len() {
                    return false;
                }
                pending.extend(values.iter().zip(elements).map(|(value, ty)| (value, *ty)));
            }
            (ExecutionValue::List(values), TypeKind::List(element)) => {
                pending.extend(values.iter().map(|value| (value, element)));
            }
            (
                ExecutionValue::Nominal {
                    aggregate,
                    variant,
                    fields,
                },
                TypeKind::Nominal { .. },
            ) => {
                let Some(descriptor) = program.aggregate(*aggregate) else {
                    return false;
                };
                if descriptor.ty != ty {
                    return false;
                }
                let field_ids = if let Some(variant) = variant {
                    let Some(variant) = program.variant(*variant) else {
                        return false;
                    };
                    if variant.aggregate != *aggregate {
                        return false;
                    }
                    program.variant_fields(variant)
                } else {
                    if descriptor.kind != crate::MirAggregateKind::Struct {
                        return false;
                    }
                    program.aggregate_fields(descriptor)
                };
                if fields.len() != field_ids.len() {
                    return false;
                }
                for (value, field) in fields.iter().zip(field_ids) {
                    let Some(field) = program.field(*field) else {
                        return false;
                    };
                    pending.push((value, field.ty));
                }
            }
            (ExecutionValue::Function(function), TypeKind::Function { .. })
                if program
                    .function(*function)
                    .is_some_and(|function| function.ty == ty) => {}
            (ExecutionValue::Closure { function, .. }, TypeKind::Function { .. })
                if program
                    .function(*function)
                    .is_some_and(|function| function.ty == ty) => {}
            (ExecutionValue::Task { function, .. }, TypeKind::Task(inner))
                if program
                    .function(*function)
                    .is_some_and(|function| function.return_type == inner) => {}
            _ => return false,
        }
    }
    true
}
