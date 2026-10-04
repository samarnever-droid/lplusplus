use std::fmt::Write as _;

use crate::MirProgram;

#[must_use]
pub fn mir_snapshot(program: &MirProgram) -> String {
    let mut snapshot = String::new();
    writeln!(snapshot, "mir-v6").expect("writing to a String cannot fail");
    writeln!(
        snapshot,
        "counts functions={} aggregates={} fields={} variants={} blocks={} switch_targets={} locals={} places={} projections={} instructions={} strings={} list_entries={}",
        program.function_count(),
        program.aggregate_count(),
        program.field_count(),
        program.variant_count(),
        program.block_count(),
        program.switch_target_count(),
        program.local_count(),
        program.place_count(),
        program.projection_count(),
        program.instruction_count(),
        program.string_count(),
        program.list_entry_count(),
    )
    .expect("writing to a String cannot fail");
    writeln!(snapshot, "strings").expect("writing to a String cannot fail");
    for (id, string) in program.strings.enumerate() {
        writeln!(snapshot, "  {id:?} {string:?}").expect("writing to a String cannot fail");
    }
    writeln!(snapshot, "aggregates").expect("writing to a String cannot fail");
    for (id, aggregate) in program.aggregates() {
        writeln!(snapshot, "  {id:?} {aggregate:?}").expect("writing to a String cannot fail");
        writeln!(
            snapshot,
            "    fields {:?}",
            program.aggregate_fields(aggregate)
        )
        .expect("writing to a String cannot fail");
        writeln!(
            snapshot,
            "    variants {:?}",
            program.aggregate_variants(aggregate)
        )
        .expect("writing to a String cannot fail");
    }
    writeln!(snapshot, "fields").expect("writing to a String cannot fail");
    for (id, field) in program.fields() {
        writeln!(snapshot, "  {id:?} {field:?}").expect("writing to a String cannot fail");
    }
    writeln!(snapshot, "variants").expect("writing to a String cannot fail");
    for (id, variant) in program.variants() {
        writeln!(snapshot, "  {id:?} {variant:?}").expect("writing to a String cannot fail");
        writeln!(snapshot, "    fields {:?}", program.variant_fields(variant))
            .expect("writing to a String cannot fail");
    }
    writeln!(snapshot, "locals").expect("writing to a String cannot fail");
    for (id, local) in program.locals() {
        writeln!(snapshot, "  {id:?} {local:?}").expect("writing to a String cannot fail");
    }
    writeln!(snapshot, "places").expect("writing to a String cannot fail");
    for (id, place) in program.places() {
        writeln!(snapshot, "  {id:?} {place:?}").expect("writing to a String cannot fail");
        writeln!(
            snapshot,
            "    projections {:?}",
            program.place_projections(place),
        )
        .expect("writing to a String cannot fail");
    }
    writeln!(snapshot, "instructions").expect("writing to a String cannot fail");
    for (id, instruction) in program.instructions() {
        writeln!(snapshot, "  {id:?} {instruction:?}").expect("writing to a String cannot fail");
    }
    writeln!(snapshot, "blocks").expect("writing to a String cannot fail");
    for (id, block) in program.blocks() {
        writeln!(snapshot, "  {id:?} {block:?}").expect("writing to a String cannot fail");
        writeln!(
            snapshot,
            "    instruction_ids {:?}",
            program.block_instructions(block),
        )
        .expect("writing to a String cannot fail");
        if let crate::Terminator::SwitchEnum { targets, .. } = block.terminator {
            writeln!(
                snapshot,
                "    switch_targets {:?}",
                program.switch_targets(targets),
            )
            .expect("writing to a String cannot fail");
        }
    }
    writeln!(snapshot, "functions").expect("writing to a String cannot fail");
    for (id, function) in program.functions() {
        writeln!(snapshot, "  {id:?} {function:?}").expect("writing to a String cannot fail");
        writeln!(
            snapshot,
            "    instance_arguments {:?}",
            function.instance_arguments,
        )
        .expect("writing to a String cannot fail");
        writeln!(
            snapshot,
            "    parameters {:?} captures {:?} kind={:?}",
            program.function_parameters(function),
            program.function_captures(function),
            function.kind,
        )
        .expect("writing to a String cannot fail");
        writeln!(
            snapshot,
            "    locals {:?}",
            program.function_locals(function)
        )
        .expect("writing to a String cannot fail");
        writeln!(
            snapshot,
            "    blocks {:?}",
            program.function_blocks(function)
        )
        .expect("writing to a String cannot fail");
    }
    writeln!(snapshot, "operand_lists").expect("writing to a String cannot fail");
    for (_, instruction) in program.instructions() {
        let crate::InstructionKind::Assign { value, .. } = instruction.kind else {
            continue;
        };
        match value {
            crate::Rvalue::Tuple(range)
            | crate::Rvalue::List(range)
            | crate::Rvalue::ConstructStruct { fields: range, .. }
            | crate::Rvalue::ConstructVariant { fields: range, .. }
            | crate::Rvalue::Call {
                arguments: range, ..
            }
            | crate::Rvalue::Builtin {
                arguments: range, ..
            }
            | crate::Rvalue::MakeClosure {
                captures: range, ..
            } => {
                writeln!(snapshot, "  {range:?} {:?}", program.operands(range),)
                    .expect("writing to a String cannot fail");
            }
            crate::Rvalue::Use(_)
            | crate::Rvalue::Unary { .. }
            | crate::Rvalue::Binary { .. }
            | crate::Rvalue::Load(_)
            | crate::Rvalue::ListLen(_)
            | crate::Rvalue::Await(_)
            | crate::Rvalue::Spawn(_) => {}
        }
    }
    snapshot
}
