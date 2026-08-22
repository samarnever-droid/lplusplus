//! Defence-in-depth validation for first-tier borrowed slices.
//!
//! Slice views are stack records and never own their base. Until the language
//! has general lifetime parameters, they are deliberately confined to the
//! creating function and to the explicit non-retaining slice operations.

use super::ir::*;

fn local_of(operand: &Operand) -> Option<LocalId> {
    match operand {
        Operand::Local(id) | Operand::Borrowed(id) => Some(*id),
        _ => None,
    }
}

fn is_view(function: &MirFunction, operand: &Operand) -> bool {
    local_of(operand)
        .and_then(|id| function.locals.get(id.0))
        .map(|local| local.ty.is_borrowed_view())
        .unwrap_or(false)
}

fn is_task(function: &MirFunction, operand: &Operand) -> bool {
    local_of(operand)
        .and_then(|id| function.locals.get(id.0))
        .map(|local| local.ty.contains_task())
        .unwrap_or(false)
}

pub fn validate(program: &MirProgram) -> Result<(), String> {
    for function in program.functions.values() {
        for block in &function.blocks {
            for instruction in &block.instrs {
                match instruction {
                    MirInstr::AssignField { value, .. } if is_task(function, value) => {
                        return Err(format!(
                            "Async safety error in '{}': task values cannot be stored or captured; the first executor is single-thread confined",
                            function.name
                        ));
                    }
                    MirInstr::AssignField { value, .. } if is_view(function, value) => {
                        return Err(format!(
                            "Borrow error in '{}': a slice view cannot be stored in an owning aggregate",
                            function.name
                        ));
                    }
                    MirInstr::Assign(_, Rvalue::AllocateTuple(_, values))
                    | MirInstr::Assign(_, Rvalue::MakeTask(_, _, values, _))
                        if values.iter().any(|value| is_task(function, value)) =>
                    {
                        return Err(format!(
                            "Async safety error in '{}': task values cannot be nested in aggregates or task environments",
                            function.name
                        ));
                    }
                    MirInstr::Assign(_, Rvalue::AllocateTuple(_, values))
                    | MirInstr::Assign(_, Rvalue::MakeTask(_, _, values, _))
                        if values.iter().any(|value| is_view(function, value)) =>
                    {
                        return Err(format!(
                            "Borrow error in '{}': a slice view cannot be stored in a tuple or task",
                            function.name
                        ));
                    }
                    MirInstr::Assign(_, Rvalue::MakeClosure(_, captures))
                    | MirInstr::Assign(_, Rvalue::MakeStackClosure(_, captures))
                        if captures.iter().any(|value| is_view(function, value)) =>
                    {
                        return Err(format!(
                            "Borrow error in '{}': a slice view cannot be captured by a closure",
                            function.name
                        ));
                    }
                    MirInstr::Assign(_, Rvalue::SpawnThread(value)) if is_task(function, value) => {
                        return Err(format!(
                            "Async safety error in '{}': a task cannot cross a thread boundary",
                            function.name
                        ));
                    }
                    MirInstr::Assign(_, Rvalue::SpawnThread(value)) if is_view(function, value) => {
                        return Err(format!(
                            "Borrow error in '{}': a slice view cannot cross a thread boundary",
                            function.name
                        ));
                    }
                    MirInstr::Assign(_, Rvalue::CallIndirect(callee, args))
                        if is_task(function, callee)
                            || args.iter().any(|value| is_task(function, value)) =>
                    {
                        return Err(format!(
                            "Async safety error in '{}': task values cannot reach indirect calls",
                            function.name
                        ));
                    }
                    MirInstr::Assign(_, Rvalue::CallIndirect(callee, args))
                        if is_view(function, callee)
                            || args.iter().any(|value| is_view(function, value)) =>
                    {
                        return Err(format!(
                            "Borrow error in '{}': a slice view cannot be passed to an unknown retaining call",
                            function.name
                        ));
                    }
                    MirInstr::Assign(_, Rvalue::CallDirect(_, args))
                        if args.iter().any(|value| is_task(function, value)) =>
                    {
                        return Err(format!(
                            "Async safety error in '{}': task values cannot be passed between functions in the single-executor tier",
                            function.name
                        ));
                    }
                    MirInstr::Assign(_, Rvalue::CallDirect(callee, args))
                        if args.iter().any(|value| is_view(function, value)) =>
                    {
                        let target = program.functions.get(callee).ok_or_else(|| {
                            format!("Borrow error in '{}': unknown direct slice reader", function.name)
                        })?;
                        for (index, argument) in args.iter().enumerate() {
                            if !is_view(function, argument) { continue; }
                            let accepts_view = target.params.get(index)
                                .and_then(|id| target.locals.get(id.0))
                                .map(|local| local.ty.is_borrowed_view())
                                .unwrap_or(false);
                            if !accepts_view {
                                return Err(format!(
                                    "Borrow error in '{}': slice argument {} is passed to retaining function '{}'",
                                    function.name, index + 1, target.name
                                ));
                            }
                        }
                        // The target is validated by this same whole-program
                        // pass. A return, capture, store, thread handoff, or
                        // unknown call in that reader is rejected in its body.
                    }
                    MirInstr::Assign(_, Rvalue::BuiltinCall(_, args))
                        if args.iter().any(|value| is_task(function, value)) =>
                    {
                        return Err(format!(
                            "Async safety error in '{}': task values cannot be passed to runtime/foreign calls",
                            function.name
                        ));
                    }
                    MirInstr::Assign(_, Rvalue::BuiltinCall(sym, args))
                        if args.iter().any(|value| is_view(function, value)) =>
                    {
                        if sym != "lpp_str_slice_to_str"
                            && sym != "lpp_slice_len"
                            && sym != "lpp_slice_get"
                            && sym != "lpp_str_slice_get"
                        {
                            return Err(format!(
                                "Borrow error in '{}': a slice view may only be consumed by explicit slice operations or a known non-retaining reader",
                                function.name
                            ));
                        }
                    }
                    _ => {}
                }
            }
            match &block.terminator {
                Terminator::Return(Some(value)) | Terminator::ReturnOwned(value)
                    if is_view(function, value) =>
                {
                    return Err(format!(
                        "Borrow error in '{}': a borrowed slice cannot be returned",
                        function.name
                    ));
                }
                _ => {}
            }
        }
    }
    Ok(())
}
