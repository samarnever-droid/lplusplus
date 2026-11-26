use super::*;

#[path = "pre_scan.rs"]
mod pre_scan_impl;
mod scan_builtins;
mod scan_instructions;
mod scan_place;
mod scan_types;

pub(super) use pre_scan_impl::pre_scan;
use scan_builtins::check_builtin;
use scan_instructions::check_instruction;
use scan_place::scan_operand_class;
pub(super) use scan_place::{PlaceResolution, PlaceStep, collect_strings, resolve_place};
pub(super) use scan_types::{SliceKind, slice_kind_of};
use scan_types::{
    check_terminator, list_element_of, require_bool, require_f64, require_i64, require_str,
};

struct ScanLower<'a> {
    program: &'a MirProgram,
    types: &'a TypeInterner,
    fn_id: MirFunctionId,
    needs: &'a mut BTreeSet<H>,
    strings: &'a mut BTreeSet<String>,
    has_list: &'a mut bool,
    has_tuple: &'a mut bool,
    has_managed: &'a mut bool,
    has_function_value: &'a mut bool,
    /// 5D2b slice 3: a task is constructed somewhere in the program.
    has_tasks: &'a mut bool,
    /// 5D2b slice 3: async functions whose task thunk is used.
    task_thunks: &'a mut Vec<MirFunctionId>,
    /// 5D2b slice 3: zero-parameter closures spawned via `spawn`.
    closure_thunks: &'a mut Vec<MirFunctionId>,
    /// 5D2b slice 3: function-value provenance.
    value_origins: &'a mut BTreeMap<(MirFunctionId, MirLocalId), MirFunctionId>,
    layouts: &'a BTreeMap<MirAggregateId, AggregateLayout>,
    aggregates: &'a AggregateIndex,
}
