//! Per-function lowering from the 5B/5C MIR slice to cranelift IR.
//!
//! The lowering matches the v1 cranelift backend's conventions:
//! cranelift *variables* for MIR locals (sound under the 4B
//! definite-initialization invariant), the compact 16-byte native
//! string-constant header handed out at `base + 16`, hardware integer
//! division (no software trap, identical to v1), and the interpreter's
//! exact operator semantics (wrapping arithmetic, signed comparisons,
//! IEEE-754 floats, 0/1 bool lattice).
//!
//! The 5C aggregate data surface adds: managed values (structs,
//! enums, lists, strings) as `I64` heap pointers with the v1
//! freestanding ARC header in front of the payload; struct layouts
//! from the v1 `struct_layout` rule; enums with an `I64` tag at
//! offset 0; dense `SwitchEnum` dispatch on the tag; projected
//! load/store through field and list-index steps; and the ARC
//! traffic that mirrors the interpreter's consume-vs-borrow rules.
//!
//! **ARC model (retain-on-transfer).** A managed value is retained
//! exactly when a new owner is created — a `Use(Copy)`, a `Load` of a
//! managed place, a `Store` of a `Copy` into a bare local or struct
//! field, a constructed field or list element that was a `Copy` of a
//! local, a call argument, and a returned value. ARC-list element
//! stores retain inside `lpp_list_set_arc` (runtime-internal) instead.
//! A release is emitted when an owning slot is overwritten (cell
//! death, managed struct field replacement) and — at every `Return` —
//! by a release pass over all managed locals in declaration order.
//! Transfers never null their source: the source keeps its reference
//! and releases it on overwrite or exit, which balances the transfer's
//! retain. Read-after-move is impossible (the 4B verifier rejects
//! it), so the retained-then-released source is never observed.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use cranelift_codegen::entity::EntityRef;
use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{
    AbiParam, Block, FuncRef, InstBuilder, MemFlags, StackSlotData, StackSlotKind, TrapCode,
    Type as CLType, UserFuncName, Value, types as cltypes,
};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module};
use cranelift_object::ObjectModule;
use lpp_codegen_api::{CodegenError, CodegenErrorKind};
use lpp_mir::{
    BasicBlockId, BinaryOperator, Constant, InstructionKind, MirAggregateId, MirExternalFunctionId,
    MirFunction, MirFunctionId, MirFunctionKind, MirLocalId, MirProgram, MirStringId, Operand,
    PlaceProjection, Rvalue, Terminator, UnaryOperator,
};
use lpp_types::{BuiltinId, PrimitiveType, TypeId, TypeInterner, TypeKind};

use crate::layout::{AggregateIndex, AggregateLayout, aggregate_layout, static_type_size_align};
use crate::{LIST_NEW, PRINT_STR};

/// The native compact string header (magic, immortal refcount, generation,
/// flags), then payload, then NUL.
const LPP_ARC_MAGIC: u32 = 0x4152_4331;
const STRING_HEADER_OFFSET: i64 = 16;

/// The pre-scan's result: what to declare, in what order.
pub(crate) struct Plan {
    /// Export symbol per function (a source-level `main` exports as
    /// `lpp_main`; the C-ABI `main` wrapper is generated separately).
    pub(crate) export_names: BTreeMap<MirFunctionId, String>,
    /// Runtime imports, in stable (lexicographic) order.
    pub(crate) imports: BTreeSet<&'static str>,
    /// Direct foreign symbols used by the program, in stable MIR-ID order.
    pub(crate) external_functions: Vec<MirExternalFunctionId>,
    /// Nominal aggregates that are constructed, in `MirAggregateId`
    /// order — one generated destructor each.
    pub(crate) destructors: Vec<MirAggregateId>,
    /// Closures with at least one capture, in `MirFunctionId` order —
    /// one generated env destructor (`lpp_drop_c{n}`) each, releasing
    /// the managed capture slots.
    pub(crate) closure_dtors: Vec<MirFunctionId>,
    /// Async functions whose task thunk is used, in `MirFunctionId`
    /// order (`__lpp_task_thunk{n}`).
    pub(crate) task_thunks: Vec<MirFunctionId>,
    /// Zero-user-parameter closures, in `MirFunctionId` order — the
    /// capsule code pointer for each is the generated task-code
    /// trampoline (`__lpp_closure_thunk{n}`), because
    /// `lpp_task_new`'s code pointer must have the
    /// `int64_t (*)(void*)` shape while the closure function returns
    /// its user result type.
    pub(crate) closure_thunks: Vec<MirFunctionId>,
    /// Provenance class of every function-value local (closure vs
    /// materialized function): the two capsule call ABIs differ by
    /// the leading `env` parameter, so a call site needs the class.
    pub(crate) value_classes: BTreeMap<(MirFunctionId, MirLocalId), ValueClass>,
    /// The ownership plan's `Shared` (cycle-member) type ids: heap
    /// values of these types can sit on a reference cycle, so their
    /// allocations are marked collectible for the runtime's
    /// trial-deletion collector. Empty for acyclic programs, which
    /// keep the exact plain-ARC codegen.
    pub(crate) shared_types: BTreeSet<TypeId>,
    /// Constructed aggregates whose type is `Shared`, in
    /// `MirAggregateId` order — one generated trace function each
    /// (`lpp_trace_{s|e}{n}`), mirroring the destructor walk.
    pub(crate) traces: Vec<MirAggregateId>,
    /// Capture-carrying closures whose function type is `Shared`, in
    /// `MirFunctionId` order — one generated env trace each
    /// (`lpp_trace_c{n}`).
    pub(crate) closure_traces: Vec<MirFunctionId>,
    /// Constructed structural-tuple types, in `TypeId` order — one
    /// generated destructor each (`lpp_drop_t{n}`), releasing the
    /// record's managed element slots.
    pub(crate) tuple_destructors: Vec<TypeId>,
    /// Constructed tuple types that are `Shared`, in `TypeId` order —
    /// one generated trace each (`lpp_trace_t{n}`), mirroring the
    /// tuple destructor walk.
    pub(crate) tuple_traces: Vec<TypeId>,
    /// Nominal type id to aggregate instance (place resolution).
    pub(crate) aggregates: AggregateIndex,
    /// Layouts for every nominal in the program.
    pub(crate) layouts: HashMap<MirAggregateId, AggregateLayout>,
    /// Non-escaping `Frame` POD structs/tuples lowered directly to stack slots.
    pub(crate) stack_locals: BTreeMap<(MirFunctionId, MirLocalId), u32>,
}

/// The shared per-function data an instruction pass reads: bundled so
/// method signatures stay short. The (invariant-lifetime) builder is
/// always passed separately.
struct Inst<'a> {
    program: &'a MirProgram,
    types: &'a TypeInterner,
    local_vars: &'a HashMap<MirLocalId, Variable>,
    env_var: Option<Variable>,
    capture_slots: &'a HashMap<MirLocalId, u32>,
    mir_id: MirFunctionId,
    function: &'a MirFunction,
}

/// The module-side state of one compilation.
pub(crate) struct Lowering<'m> {
    module: &'m mut ObjectModule,
    aggregates: AggregateIndex,
    layouts: HashMap<MirAggregateId, AggregateLayout>,
    func_ids: BTreeMap<MirFunctionId, FuncId>,
    dtor_ids: BTreeMap<MirAggregateId, FuncId>,
    trace_ids: BTreeMap<MirAggregateId, FuncId>,
    closure_dtor_ids: BTreeMap<MirFunctionId, FuncId>,
    closure_trace_ids: BTreeMap<MirFunctionId, FuncId>,
    tuple_dtor_ids: BTreeMap<TypeId, FuncId>,
    tuple_trace_ids: BTreeMap<TypeId, FuncId>,
    /// The 4D `Shared` type ids (see `Plan::shared_types`).
    shared_types: BTreeSet<TypeId>,
    /// Non-escaping `Frame` POD locals allocated on the stack (`byte_size >= 8`).
    stack_locals: BTreeMap<(MirFunctionId, MirLocalId), u32>,
    pgo_fingerprint: Option<u64>,
    task_thunk_ids: BTreeMap<MirFunctionId, FuncId>,
    closure_thunk_ids: BTreeMap<MirFunctionId, FuncId>,
    value_classes: BTreeMap<(MirFunctionId, MirLocalId), ValueClass>,
    import_ids: BTreeMap<&'static str, FuncId>,
    external_ids: BTreeMap<MirExternalFunctionId, FuncId>,
    exported: BTreeSet<String>,
    imported: BTreeSet<String>,
    entry: Option<String>,
    lpp_main_id: Option<FuncId>,
    lpp_main_function_id: Option<MirFunctionId>,
    emitted_strings: BTreeMap<MirStringId, DataId>,
    debug_records: Vec<crate::dwarf::CompiledDebugFunction>,
}

mod preflight;
mod validation;

use preflight::user_parameters;
pub(crate) use preflight::{ValueClass, pre_scan};
use validation::*;

mod assignment;
mod collections;
mod imports;
mod intrinsics;

use imports::*;
use intrinsics::{arch_import_signature, intrinsic_arch_import};

mod declarations;
mod entry;
mod functions;
mod operators;
mod tasks;

/// A free inst.function so it can be shared by the terminator and the
/// destructor lowering without borrowing the mutable module twice.
fn emit_tag_cascade(builder: &mut FunctionBuilder, tag: Value, targets: &[Block]) {
    let count = targets.len();
    if count == 1 {
        // A single-variant dense switch is an unconditional jump.
        builder.ins().jump(targets[0], &[]);
        return;
    }
    let cascade: Vec<Block> = (0..count).map(|_| builder.create_block()).collect();
    // The dispatch block takes its one terminator here; the cascade
    // blocks are still pristine when their bodies are filled below.
    builder.ins().jump(cascade[0], &[]);
    for index in 0..count {
        builder.switch_to_block(cascade[index]);
        if index + 1 < count {
            let expected = builder.ins().iconst(cltypes::I64, index as i64);
            let matches_ordinal = builder.ins().icmp(IntCC::Equal, tag, expected);
            builder.ins().brif(
                matches_ordinal,
                targets[index],
                &[],
                cascade[index + 1],
                &[],
            );
        } else {
            // The dense ordinal cannot be out of range in verified
            // MIR; the final target takes the rest.
            builder.ins().jump(targets[index], &[]);
        }
    }
}

/// Match `value` to the exact machine type of the storage site.
/// Box a user result into the `I64` task-result shape (the v1 thunk
/// rule): void → `0`, bool uextend, float bit-cast, `I64` unchanged.
fn box_result(
    types: &TypeInterner,
    builder: &mut FunctionBuilder,
    value: Option<Value>,
    return_type: TypeId,
) -> Value {
    match machine_type(types, return_type) {
        None => builder.ins().iconst(cltypes::I64, 0),
        Some(cltypes::I64) => value.expect("I64 result"),
        Some(cltypes::I8) => {
            let v = value.expect("bool result");
            builder.ins().uextend(cltypes::I64, v)
        }
        Some(cltypes::F64) => {
            let v = value.expect("float result");
            builder.ins().bitcast(cltypes::I64, MemFlags::new(), v)
        }
        Some(_) => unreachable!("slice machine types"),
    }
}

/// The inverse of `box_result` at a call site: unbox the `I64`
/// task/closure thunk result to the user result type (`None` for
/// void).
fn unbox_result(
    types: &TypeInterner,
    builder: &mut FunctionBuilder,
    boxed: Value,
    return_type: TypeId,
) -> Option<Value> {
    match machine_type(types, return_type) {
        None => None,
        Some(cltypes::I64) => Some(boxed),
        Some(cltypes::I8) => Some(builder.ins().ireduce(cltypes::I8, boxed)),
        Some(cltypes::F64) => Some(builder.ins().bitcast(cltypes::F64, MemFlags::new(), boxed)),
        Some(_) => unreachable!("slice machine types"),
    }
}

fn is_signed_integer_kind(kind: Option<TypeKind>) -> bool {
    matches!(
        kind,
        Some(TypeKind::Primitive(
            PrimitiveType::Int
                | PrimitiveType::I8
                | PrimitiveType::I16
                | PrimitiveType::I32
                | PrimitiveType::ISize,
        ))
    )
}

fn normalize_integer_operands(
    builder: &mut FunctionBuilder,
    left: Value,
    right: Value,
    left_signed: bool,
    right_signed: bool,
) -> (Value, Value, CLType) {
    let left_type = builder.func.dfg.value_type(left);
    let right_type = builder.func.dfg.value_type(right);
    if !(left_type.is_int() && right_type.is_int()) || left_type == right_type {
        return (left, right, left_type);
    }
    let target = if left_type.bits() >= right_type.bits() {
        left_type
    } else {
        right_type
    };
    (
        widen_integer_to(builder, left, target, left_signed),
        widen_integer_to(builder, right, target, right_signed),
        target,
    )
}

fn widen_integer_to(
    builder: &mut FunctionBuilder,
    value: Value,
    target: CLType,
    signed: bool,
) -> Value {
    let have = builder.func.dfg.value_type(value);
    if have == target {
        return value;
    }
    if have.bits() > target.bits() {
        return builder.ins().ireduce(target, value);
    }
    if signed {
        builder.ins().sextend(target, value)
    } else {
        builder.ins().uextend(target, value)
    }
}

fn coerce_to(builder: &mut FunctionBuilder, value: Value, target: CLType) -> Value {
    let have = builder.func.dfg.value_type(value);
    if have == target {
        return value;
    }
    if have.is_int() && target.is_int() {
        if have.bits() > target.bits() {
            builder.ins().ireduce(target, value)
        } else {
            builder.ins().uextend(target, value)
        }
    } else {
        value
    }
}
