use super::*;

/// Emit the full module text.
pub(crate) fn lower_module(
    program: &MirProgram,
    types: &TypeInterner,
    plan: &Plan,
) -> Result<String, CodegenError> {
    let mut text = String::new();
    let _ = std::fmt::Write::write_str(
        &mut text,
        "target triple = \"x86_64-unknown-linux-gnu\"\n\n",
    );

    // String globals use the same 16-byte immortal ARC header as runtime
    // allocations. The generated value is a pointer to byte 16 (the NUL-
    // terminated payload), not to the header itself.
    for (content, index) in &plan.string_of {
        let bytes = content.as_bytes();
        let headered = llvm_arc_string(content);
        let _ = std::fmt::Write::write_fmt(
            &mut text,
            format_args!(
                "@str_{index} = private unnamed_addr constant [{} x i8] c\"{}\", align 16\n",
                bytes.len() + 17,
                headered
            ),
        );
    }
    text.push('\n');

    // Builtin declarations.
    for &symbol in &plan.builtins {
        match symbol {
            "lpp_print_str" => {
                let _ = std::fmt::Write::write_str(&mut text, "declare void @lpp_print_str(ptr)\n");
            }
            "lpp_print_int" => {
                let _ = std::fmt::Write::write_str(&mut text, "declare void @lpp_print_int(i64)\n");
            }
            "lpp_print_bool" => {
                let _ = std::fmt::Write::write_str(&mut text, "declare void @lpp_print_bool(i8)\n");
            }
            "lpp_print_float" => {
                let _ = std::fmt::Write::write_str(
                    &mut text,
                    "declare void @lpp_print_float(double)\n",
                );
            }
            "lpp_write_str" => {
                let _ = std::fmt::Write::write_str(&mut text, "declare void @lpp_write_str(ptr)\n");
            }
            "lpp_str_len" => {
                let _ = std::fmt::Write::write_str(&mut text, "declare i64 @lpp_str_len(ptr)\n");
            }
            "lpp_str_eq" => {
                let _ =
                    std::fmt::Write::write_str(&mut text, "declare i64 @lpp_str_eq(ptr, ptr)\n");
            }
            "lpp_str_concat" => {
                let _ = std::fmt::Write::write_str(
                    &mut text,
                    "declare ptr @lpp_str_concat(ptr, ptr)\n",
                );
            }
            "lpp_panic_div_zero" => {
                let _ =
                    std::fmt::Write::write_str(&mut text, "declare void @lpp_panic_div_zero()\n");
            }
            _ => {}
        }
    }
    text.push('\n');

    // Functions (MirFunctionId order).
    for (fn_id, function) in program.functions() {
        emit_function(program, types, plan, fn_id, function, &mut text)?;
        text.push('\n');
    }
    Ok(text)
}
