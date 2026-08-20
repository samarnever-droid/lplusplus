//! Optional LLVM object backend.
//!
//! This backend lowers the complete ordinary MIR shape used by the current
//! tests to textual LLVM IR and asks clang/LLVM to assemble it. Cranelift stays
//! the default. LLVM is selected explicitly with `--backend llvm`.

use crate::ast::BinaryOperator;
use crate::mir::ir::*;
use crate::types::{StructTypeId, TypeRef, TypeTable};
use crate::layout::{struct_layout, tuple_layout, tuple_runtime_metadata};
use crate::type_facts::{AbiClass, ListElementClass};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::process::Command;

fn llvm_type(ty: &TypeRef) -> &'static str {
    match ty.abi_class() {
        AbiClass::Void => "void",
        AbiClass::I8 => "i8",
        AbiClass::I64 => "i64",
        AbiClass::F64 => "double",
        AbiClass::Pointer => "ptr",
        AbiClass::VectorI64x2 => "<2 x i64>",
    }
}

fn is_supported_type(ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Unresolved(_) | TypeRef::TypeParam(_) => false,
        TypeRef::Tuple(elements) => elements.iter().all(is_supported_type),
        TypeRef::Slice(element) | TypeRef::Task(element) => is_supported_type(element),
        _ => true,
    }
}

fn escape_bytes(bytes: &[u8]) -> String {
    let mut out = String::new();
    for byte in bytes {
        match byte {
            b'\\' => out.push_str("\\5C"),
            b'"' => out.push_str("\\22"),
            0x20..=0x7e => out.push(*byte as char),
            _ => out.push_str(&format!("\\{:02X}", byte)),
        }
    }
    out
}

fn literal_blob(value: &str) -> Vec<u8> {
    let magic = 0x4152_4331u32.to_le_bytes();
    let mut bytes = Vec::with_capacity(24 + value.len() + 1);
    bytes.extend_from_slice(&magic);
    bytes.extend_from_slice(&magic);
    bytes.extend_from_slice(&[0u8; 16]);
    bytes.extend_from_slice(value.as_bytes());
    bytes.push(0);
    bytes
}

fn builtin_signature(symbol: &str) -> Option<(&'static str, &'static str)> {
    Some(match symbol {
        "lpp_print_int" => ("void", "i64"),
        "lpp_print_bool" => ("void", "i8"),
        "lpp_print_float" => ("void", "double"),
        "lpp_print_str" => ("void", "ptr"),
        "lpp_str_len" => ("i64", "ptr"),
        "lpp_str_eq" => ("i64", "ptr, ptr"),
        "lpp_str_concat" => ("ptr", "ptr, ptr"),
        "lpp_str_substr" => ("ptr", "ptr, i64, i64"),
        "lpp_str_trim" | "lpp_str_upper" | "lpp_str_lower" => ("ptr", "ptr"),
        "lpp_list_new" | "lpp_list_new_arc" => ("ptr", ""),
        "lpp_list_len" => ("i64", "ptr"),
        "lpp_list_push" => ("void", "ptr, i64"),
        "lpp_list_push_bool" => ("void", "ptr, i8"),
        "lpp_list_push_float" => ("void", "ptr, double"),
        "lpp_list_push_arc" => ("void", "ptr, ptr"),
        "lpp_list_set" => ("void", "ptr, i64, i64"),
        "lpp_list_set_bool" => ("void", "ptr, i64, i8"),
        "lpp_list_set_float" => ("void", "ptr, i64, double"),
        "lpp_list_set_arc" => ("void", "ptr, i64, ptr"),
        "lpp_list_get" => ("i64", "ptr, i64"),
        "lpp_list_get_bool" => ("i8", "ptr, i64"),
        "lpp_list_get_float" => ("double", "ptr, i64"),
        "lpp_list_get_arc" => ("ptr", "ptr, i64"),
        "lpp_map_new" | "lpp_map_new_arc" => ("ptr", ""),
        "lpp_map_put" => ("void", "ptr, i64, i64"),
        "lpp_map_put_float" => ("void", "ptr, i64, double"),
        "lpp_map_put_str" => ("void", "ptr, ptr, i64"),
        "lpp_map_put_str_float" => ("void", "ptr, ptr, double"),
        "lpp_map_get" => ("i64", "ptr, i64"),
        "lpp_map_get_float" => ("double", "ptr, i64"),
        "lpp_map_get_str" => ("i64", "ptr, ptr"),
        "lpp_map_get_str_float" => ("double", "ptr, ptr"),
        "lpp_map_has" => ("i64", "ptr, i64"),
        "lpp_map_has_str" => ("i64", "ptr, ptr"),
        "lpp_map_len" => ("i64", "ptr"),
        "lpp_map_remove" => ("void", "ptr, i64"),
        "lpp_map_remove_str" => ("void", "ptr, ptr"),
        "lpp_int_to_str" => ("ptr", "i64"),
        "lpp_float_to_str" => ("ptr", "double"),
        "lpp_bool_to_str" => ("ptr", "i8"),
        "lpp_arc_retain" | "lpp_arc_release" | "lpp_arc_retain_local"
        | "lpp_arc_release_local" | "lpp_arena_retain" | "lpp_arena_release_node"
        | "lpp_closure_destroy" => ("void", "ptr"),
        // The arena handle is represented as an i64 in MIR/Cranelift. On the
        // supported 64-bit targets this is the same ABI register as a pointer.
        "lpp_arena_begin" => ("i64", ""),
        "lpp_arena_release" => ("void", "i64"),
        "lpp_arena_alloc" => ("ptr", "i64, i64, ptr"),
        "lpp_thread_spawn" => ("void", "ptr, ptr"),
        "lpp_tuple_alloc" => ("ptr", "i64, i64, i64"),
        "lpp_slice_init" => ("ptr", "ptr, ptr, i64, i64, i64"),
        "lpp_slice_len" => ("i64", "ptr"),
        "lpp_slice_get" => ("i64", "ptr, i64"),
        "lpp_slice_get_bool" => ("i8", "ptr, i64"),
        "lpp_slice_get_float" => ("double", "ptr, i64"),
        "lpp_str_slice_get" => ("ptr", "ptr, i64"),
        "lpp_str_slice_to_str" => ("ptr", "ptr"),
        "lpp_task_new" => ("ptr", "ptr, ptr, i64"),
        "lpp_task_poll" => ("i64", "ptr"),
        "lpp_task_await" | "lpp_executor_run" => ("i64", "ptr"),
        "lpp_task_destroy" => ("void", "ptr"),
        "lpp_exit" | "exit" => ("void", "i64"),
        "lpp_ord" | "ord" => ("i64", "ptr"),
        "lpp_chr" | "chr" => ("ptr", "i64"),
        "lpp_char_at" | "char_at" => ("ptr", "ptr, i64"),
        "lpp_str_find" | "str_find" => ("i64", "ptr, ptr"),
        "lpp_str_split" | "str_split" => ("ptr", "ptr, i64"),
        "lpp_str_replace" | "str_replace" => ("ptr", "ptr, ptr, ptr"),
        "lpp_read_file" | "read_file" => ("ptr", "ptr"),
        "lpp_write_file" | "write_file" => ("i64", "ptr, ptr"),
        "lpp_append_file" | "append_file" => ("i64", "ptr, ptr"),
        "lpp_delete_file" | "delete_file" => ("i64", "ptr"),
        "lpp_file_exists" | "file_exists" => ("i64", "ptr"),
        "lpp_file_size" | "file_size" => ("i64", "ptr"),
        "lpp_file_copy" | "file_copy" => ("i64", "ptr, ptr"),
        "lpp_command_exec" | "command_exec" => ("i64", "ptr"),
        "lpp_command_output" | "command_output" => ("ptr", "ptr"),
        "lpp_parse_int" | "parse_int" | "lpp_str_to_int" => ("i64", "ptr"),
        "lpp_parse_float" | "parse_float" | "lpp_str_to_float" => ("double", "ptr"),
        "lpp_clock_ms" | "clock_ms" => ("i64", ""),
        "lpp_time_now_unix" | "time_now_unix" => ("i64", ""),
        "lpp_sqrt" | "sqrt" => ("double", "double"),
        "lpp_sin" | "sin" => ("double", "double"),
        "lpp_cos" | "cos" => ("double", "double"),
        "lpp_tan" | "tan" => ("double", "double"),
        "lpp_env_get" | "env_get" => ("ptr", "ptr"),
        "lpp_env_set" | "env_set" => ("void", "ptr, ptr"),
        "lpp_random_range" | "random_range" => ("i64", "i64, i64"),
        "lpp_time_ms" | "time_ms" => ("i64", ""),
        "lpp_int_to_float" | "int_to_float" => ("double", "i64"),
        "lpp_float_to_int" | "float_to_int" => ("i64", "double"),
        "lpp_str_contains" | "str_contains" => ("i64", "ptr, ptr"),
        "lpp_str_starts_with" | "str_starts_with" => ("i64", "ptr, ptr"),
        "lpp_str_ends_with" | "str_ends_with" => ("i64", "ptr, ptr"),
        "lpp_gui_window_create" | "gui_window_create" => ("i64", "ptr, i64, i64"),
        "lpp_gui_window_is_open" | "gui_window_is_open" => ("i64", "i64"),
        "lpp_gui_window_width" | "gui_window_width" => ("i64", "i64"),
        "lpp_gui_window_height" | "gui_window_height" => ("i64", "i64"),
        "lpp_gui_window_poll_events" | "gui_window_poll_events" => ("i64", "i64"),
        "lpp_gui_clear" | "gui_clear" => ("void", "i64, i64"),
        "lpp_gui_draw_rect" | "gui_draw_rect" => ("void", "i64, i64, i64, i64, i64, i64"),
        "lpp_gui_draw_rounded_rect" | "gui_draw_rounded_rect" => ("void", "i64, i64, i64, i64, i64, i64, i64"),
        "lpp_gui_draw_circle" | "gui_draw_circle" => ("void", "i64, i64, i64, i64, i64"),
        "lpp_gui_draw_line" | "gui_draw_line" => ("void", "i64, i64, i64, i64, i64, i64, i64"),
        "lpp_gui_draw_text" | "gui_draw_text" => ("void", "i64, i64, i64, ptr, i64"),
        "lpp_gui_present" | "gui_present" => ("void", "i64"),
        "lpp_gui_mouse_x" | "gui_mouse_x" => ("i64", "i64"),
        "lpp_gui_mouse_y" | "gui_mouse_y" => ("i64", "i64"),
        "lpp_gui_mouse_down" | "gui_mouse_down" => ("i64", "i64"),
        "lpp_gui_key_down" | "gui_key_down" => ("i64", "i64, i64"),
        "lpp_gui_measure_text_width" | "gui_measure_text_width" => ("i64", "i64, ptr"),
        "lpp_gui_dialog_message" | "gui_dialog_message" => ("i64", "ptr, ptr"),
        "lpp_gui_get_ticks_ms" | "gui_get_ticks_ms" => ("i64", ""),
        "lpp_gui_window_close" | "gui_window_close" => ("void", "i64"),
        "lpp_webview_window_create" | "webview_window_create" => ("i64", "ptr, i64, i64, i64"),
        "lpp_webview_set_html" | "webview_set_html" => ("void", "i64, ptr"),
        "lpp_webview_navigate" | "webview_navigate" => ("void", "i64, ptr"),
        "lpp_webview_run" | "webview_run" => ("void", "i64"),
        "lpp_webview_terminate" | "webview_terminate" => ("void", "i64"),
        "lpp_webview_destroy" | "webview_destroy" => ("void", "i64"),
        "lpp_buf_alloc" | "buf_alloc" => ("ptr", "i64"),
        "lpp_buf_free" | "buf_free" => ("void", "ptr"),
        "lpp_buf_len" | "buf_len" => ("i64", "ptr"),
        "lpp_buf_get8" | "buf_get8" => ("i64", "ptr, i64"),
        "lpp_buf_set8" | "buf_set8" => ("void", "ptr, i64, i64"),
        "lpp_buf_set32le" | "buf_set32le" => ("void", "ptr, i64, i64"),
        "lpp_buf_get32le" | "buf_get32le" => ("i64", "ptr, i64"),
        "lpp_buf_set16le" | "buf_set16le" => ("void", "ptr, i64, i64"),
        "lpp_buf_get16le" | "buf_get16le" => ("i64", "ptr, i64"),
        "lpp_buf_read" | "buf_read" => ("ptr", "ptr"),
        "lpp_buf_write" | "buf_write" => ("i64", "ptr, ptr"),
        "lpp_buf_crc32" | "buf_crc32" => ("i64", "ptr, i64, i64"),
        "lpp_buf_copy" | "buf_copy" => ("i64", "ptr, i64, ptr, i64, i64"),
        "lpp_buf_write_str" | "buf_write_str" => ("i64", "ptr, i64, ptr"),
        "lpp_buf_read_str" | "buf_read_str" => ("ptr", "ptr, i64, i64"),
        _ => return None,
    })
}

#[derive(Clone)]
struct FunctionSig {
    name: String,
    ret: &'static str,
}

struct FunctionEmitter<'a> {
    function: &'a MirFunction,
    func_names: &'a HashMap<FuncId, String>,
    signatures: &'a HashMap<FuncId, FunctionSig>,
    type_table: &'a TypeTable,
    strings: &'a mut Vec<String>,
    declarations: &'a mut HashMap<String, (String, String)>,
    next_value: usize,
}

impl<'a> FunctionEmitter<'a> {
    fn temp(&mut self) -> String {
        let name = format!("%v{}", self.next_value);
        self.next_value += 1;
        name
    }

    fn local_ptr(id: LocalId) -> String { format!("%l{}", id.0) }

    fn operand(&mut self, operand: &Operand, out: &mut String) -> Result<(String, &'static str), String> {
        match operand {
            Operand::Local(id) | Operand::Borrowed(id) => {
                if self.function.locals[id.0].ty == TypeRef::Void {
                    return Ok(("0".to_string(), "void"));
                }
                let ty = llvm_type(&self.function.locals[id.0].ty);
                let value = self.temp();
                out.push_str(&format!("  {} = load {}, ptr {}, align 8\n", value, ty, Self::local_ptr(*id)));
                Ok((value, ty))
            }
            Operand::Int(value) => Ok((value.to_string(), "i64")),
            Operand::Float(value) => Ok((format!("{:.17}", value), "double")),
            Operand::Bool(value) => Ok(((if *value { 1 } else { 0 }).to_string(), "i8")),
            Operand::String(value) => {
                let index = self.strings.len();
                self.strings.push(value.clone());
                let global = format!("@.lpp_str{}", index);
                let value_name = self.temp();
                out.push_str(&format!(
                    "  {} = getelementptr inbounds [{} x i8], ptr {}, i64 0, i64 24\n",
                    value_name,
                    24 + value.as_bytes().len() + 1,
                    global
                ));
                Ok((value_name, "ptr"))
            }
        }
    }

    fn store(&mut self, id: LocalId, value: &str, ty: &str, out: &mut String) {
        out.push_str(&format!("  store {} {}, ptr {}, align 8\n", ty, value, Self::local_ptr(id)));
    }

    fn coerce(&mut self, value: &str, actual: &str, expected: &str, out: &mut String) -> String {
        if actual == expected { return value.to_string(); }
        if actual == "void" {
            return match expected {
                "ptr" => "null".to_string(),
                "double" => "0.0".to_string(),
                _ => "0".to_string(),
            };
        }
        let result = self.temp();
        match (actual, expected) {
            ("i64", "ptr") => out.push_str(&format!("  {} = inttoptr i64 {} to ptr\n", result, value)),
            ("ptr", "i64") => out.push_str(&format!("  {} = ptrtoint ptr {} to i64\n", result, value)),
            ("i64", "i8") => out.push_str(&format!("  {} = trunc i64 {} to i8\n", result, value)),
            ("i8", "i64") => out.push_str(&format!("  {} = zext i8 {} to i64\n", result, value)),
            _ => return value.to_string(),
        }
        result
    }

    fn compare(&mut self, op: &BinaryOperator, left: &str, right: &str, ty: &str, out: &mut String) -> String {
        let predicate = match op {
            BinaryOperator::Eq => if ty == "double" { "oeq" } else { "eq" },
            BinaryOperator::NotEq => if ty == "double" { "one" } else { "ne" },
            BinaryOperator::Less => if ty == "double" { "olt" } else { "slt" },
            BinaryOperator::Greater => if ty == "double" { "ogt" } else { "sgt" },
            BinaryOperator::LessEq => if ty == "double" { "ole" } else { "sle" },
            BinaryOperator::GreaterEq => if ty == "double" { "oge" } else { "sge" },
            _ => "eq",
        };
        let value = self.temp();
        let instruction = if ty == "double" { "fcmp" } else { "icmp" };
        out.push_str(&format!("  {} = {} {} {} {}, {}\n", value, instruction, predicate, ty, left, right));
        value
    }

    fn binary(&mut self, op: &BinaryOperator, left: &str, right: &str, ty: &str, out: &mut String) -> (String, &'static str) {
        if matches!(op, BinaryOperator::Eq | BinaryOperator::NotEq | BinaryOperator::Less | BinaryOperator::Greater | BinaryOperator::LessEq | BinaryOperator::GreaterEq) {
            let cmp = self.compare(op, left, right, ty, out);
            let value = self.temp();
            out.push_str(&format!("  {} = zext i1 {} to i8\n", value, cmp));
            return (value, "i8");
        }
        let result = self.temp();
        let instruction = match op {
            BinaryOperator::Add => if ty == "double" { "fadd" } else { "add" },
            BinaryOperator::Subtract => if ty == "double" { "fsub" } else { "sub" },
            BinaryOperator::Multiply => if ty == "double" { "fmul" } else { "mul" },
            BinaryOperator::Divide => if ty == "double" { "fdiv" } else { "sdiv" },
            BinaryOperator::Modulo => if ty == "double" { "frem" } else { "srem" },
            BinaryOperator::BitAnd | BinaryOperator::And => "and",
            BinaryOperator::BitOr | BinaryOperator::Or => "or",
            BinaryOperator::BitXor => "xor",
            BinaryOperator::Shl => "shl",
            BinaryOperator::Shr => "ashr",
            _ => "add",
        };
        out.push_str(&format!("  {} = {} {} {}, {}\n", result, instruction, ty, left, right));
        let result_ty = if ty == "double" { "double" } else if ty == "i8" { "i8" } else { "i64" };
        (result, result_ty)
    }

    fn field_ptr(&mut self, base: &str, sid: StructTypeId, field: &str, out: &mut String) -> Result<(String, TypeRef), String> {
        let definition = self.type_table.definitions.get(sid.0).ok_or_else(|| "unknown LLVM struct".to_string())?;
        let index = definition.fields.iter().position(|(name, _)| name == field).ok_or_else(|| format!("unknown field '{}.{}'", definition.name, field))?;
        let (layout, _) = struct_layout(self.type_table, sid);
        let address = self.temp();
        out.push_str(&format!("  {} = getelementptr i8, ptr {}, i64 {}\n", address, base, layout[index].offset));
        Ok((address, definition.fields[index].1.clone()))
    }

    fn register_builtin(&mut self, symbol: &str) -> Result<(&'static str, &'static str), String> {
        let (ret, params) = builtin_signature(symbol).ok_or_else(|| format!("LLVM backend does not support builtin '{}'", symbol))?;
        self.declarations.insert(symbol.to_string(), (ret.to_string(), params.to_string()));
        Ok((ret, params))
    }

    fn rvalue(&mut self, rvalue: &Rvalue, dest_ty: &TypeRef, out: &mut String) -> Result<Option<(String, &'static str)>, String> {
        match rvalue {
            Rvalue::AllocateTuple(types, values) => {
                let (layout, size) = tuple_layout(types);
                let (mask, offsets) = tuple_runtime_metadata(types);
                self.register_builtin("lpp_tuple_alloc")?;
                let tuple = self.temp();
                out.push_str(&format!(
                    "  {} = call ptr @lpp_tuple_alloc(i64 {}, i64 {}, i64 {})\n",
                    tuple, size, mask, offsets
                ));
                for ((operand, field), ty) in values.iter().zip(layout.iter()).zip(types.iter()) {
                    let (value, actual) = self.operand(operand, out)?;
                    let expected = llvm_type(ty);
                    let value = self.coerce(&value, actual, expected, out);
                    let address = self.temp();
                    out.push_str(&format!(
                        "  {} = getelementptr i8, ptr {}, i64 {}\n",
                        address, tuple, field.offset
                    ));
                    let align = field.align;
                    out.push_str(&format!(
                        "  store {} {}, ptr {}, align {}\n",
                        expected, value, address, align
                    ));
                }
                Ok(Some((tuple, "ptr")))
            }
            Rvalue::TupleField(base, index) => {
                let local = match base {
                    Operand::Local(id) | Operand::Borrowed(id) => *id,
                    _ => return Err("LLVM tuple field base is not a local".to_string()),
                };
                let types = match &self.function.locals[local.0].ty {
                    TypeRef::Tuple(types) => types,
                    other => return Err(format!("LLVM tuple field base has type {:?}", other)),
                };
                let (layout, _) = tuple_layout(types);
                let field = layout.get(*index).ok_or_else(|| "LLVM tuple index out of range".to_string())?;
                let field_ty = &types[*index];
                let (tuple, _) = self.operand(base, out)?;
                let address = self.temp();
                out.push_str(&format!("  {} = getelementptr i8, ptr {}, i64 {}\n", address, tuple, field.offset));
                let value = self.temp();
                let align = field.align;
                out.push_str(&format!("  {} = load {}, ptr {}, align {}\n", value, llvm_type(field_ty), address, align));
                Ok(Some((value, llvm_type(field_ty))))
            }
            Rvalue::MakeSlice { base, start, length, kind } => {
                self.register_builtin("lpp_slice_init")?;
                let storage = self.temp();
                out.push_str(&format!("  {} = alloca [40 x i8], align 8\n", storage));
                let (base, base_ty) = self.operand(base, out)?;
                let base = self.coerce(&base, base_ty, "ptr", out);
                let (start, _) = self.operand(start, out)?;
                let (length, _) = self.operand(length, out)?;
                let view = self.temp();
                out.push_str(&format!(
                    "  {} = call ptr @lpp_slice_init(ptr {}, ptr {}, i64 {}, i64 {}, i64 {})\n",
                    view, storage, base, start, length, kind
                ));
                Ok(Some((view, "ptr")))
            }
            Rvalue::SliceLen(view) => {
                self.register_builtin("lpp_slice_len")?;
                let (view, _) = self.operand(view, out)?;
                let value = self.temp();
                out.push_str(&format!("  {} = call i64 @lpp_slice_len(ptr {})\n", value, view));
                Ok(Some((value, "i64")))
            }
            Rvalue::SliceGet(view, index) => {
                let view_id = match view {
                    Operand::Local(id) | Operand::Borrowed(id) => *id,
                    _ => return Err("LLVM slice view is not a local".to_string()),
                };
                let symbol = match (&self.function.locals[view_id.0].ty, dest_ty) {
                    (TypeRef::StrSlice, _) => "lpp_str_slice_get",
                    (_, TypeRef::Bool) => "lpp_slice_get_bool",
                    (_, TypeRef::Float) => "lpp_slice_get_float",
                    _ => "lpp_slice_get",
                };
                let (ret, _) = self.register_builtin(symbol)?;
                let (view, _) = self.operand(view, out)?;
                let (index, _) = self.operand(index, out)?;
                let value = self.temp();
                out.push_str(&format!("  {} = call {} @{}(ptr {}, i64 {})\n", value, ret, symbol, view, index));
                Ok(Some((value, ret)))
            }
            Rvalue::SliceToStr(view) => {
                self.register_builtin("lpp_str_slice_to_str")?;
                let (view, _) = self.operand(view, out)?;
                let value = self.temp();
                out.push_str(&format!("  {} = call ptr @lpp_str_slice_to_str(ptr {})\n", value, view));
                Ok(Some((value, "ptr")))
            }
            Rvalue::MakeTask(id, argument_types, arguments, result_type) => {
                let (layout, size) = tuple_layout(argument_types);
                let (mask, offsets) = tuple_runtime_metadata(argument_types);
                self.register_builtin("lpp_tuple_alloc")?;
                self.register_builtin("lpp_task_new")?;
                let environment = self.temp();
                out.push_str(&format!(
                    "  {} = call ptr @lpp_tuple_alloc(i64 {}, i64 {}, i64 {})\n",
                    environment, size, mask, offsets
                ));
                for ((operand, field), ty) in arguments.iter().zip(layout.iter()).zip(argument_types.iter()) {
                    let (value, actual) = self.operand(operand, out)?;
                    let expected = llvm_type(ty);
                    let value = self.coerce(&value, actual, expected, out);
                    let address = self.temp();
                    out.push_str(&format!("  {} = getelementptr i8, ptr {}, i64 {}\n", address, environment, field.offset));
                    let align = field.align;
                    out.push_str(&format!("  store {} {}, ptr {}, align {}\n", expected, value, address, align));
                }
                let task = self.temp();
                out.push_str(&format!(
                    "  {} = call ptr @lpp_task_new(ptr @__lpp_task_thunk_{}, ptr {}, i64 {})\n",
                    task, id.0, environment, result_type.is_managed() as i32
                ));
                Ok(Some((task, "ptr")))
            }
            Rvalue::Await(task) => {
                self.register_builtin("lpp_task_await")?;
                let (task, _) = self.operand(task, out)?;
                let raw = self.temp();
                out.push_str(&format!("  {} = call i64 @lpp_task_await(ptr {})\n", raw, task));
                match dest_ty {
                    TypeRef::Float => {
                        let value = self.temp();
                        out.push_str(&format!("  {} = bitcast i64 {} to double\n", value, raw));
                        Ok(Some((value, "double")))
                    }
                    TypeRef::Bool => {
                        let value = self.temp();
                        out.push_str(&format!("  {} = trunc i64 {} to i8\n", value, raw));
                        Ok(Some((value, "i8")))
                    }
                    ty if llvm_type(ty) == "ptr" => {
                        let value = self.temp();
                        out.push_str(&format!("  {} = inttoptr i64 {} to ptr\n", value, raw));
                        Ok(Some((value, "ptr")))
                    }
                    _ => Ok(Some((raw, "i64"))),
                }
            }
            Rvalue::Use(op) => Ok(Some(self.operand(op, out)?)),
            Rvalue::Move(id) => {
                let op = Operand::Local(*id);
                Ok(Some(self.operand(&op, out)?))
            }
            Rvalue::BinaryOp(op, left, right) => {
                let (left, lty) = self.operand(left, out)?;
                let (right, rty) = self.operand(right, out)?;
                let cmp_ty = if matches!(op, BinaryOperator::Eq | BinaryOperator::NotEq | BinaryOperator::Less | BinaryOperator::Greater | BinaryOperator::LessEq | BinaryOperator::GreaterEq) {
                    lty
                } else {
                    llvm_type(dest_ty)
                };
                let right = self.coerce(&right, rty, cmp_ty, out);
                Ok(Some(self.binary(op, &left, &right, cmp_ty, out)))
            }
            Rvalue::CallDirect(id, args) => {
                let target = self.signatures.get(id).ok_or_else(|| format!("unknown LLVM callee {:?}", id))?.clone();
                let mut values = Vec::new();
                for arg in args { let (value, ty) = self.operand(arg, out)?; values.push(format!("{} {}", ty, value)); }
                if target.ret == "void" {
                    out.push_str(&format!("  call void {}({})\n", target.name, values.join(", ")));
                    Ok(None)
                } else {
                    let value = self.temp();
                    out.push_str(&format!("  {} = call {} {}({})\n", value, target.ret, target.name, values.join(", ")));
                    Ok(Some((value, target.ret)))
                }
            }
            Rvalue::BuiltinCall(symbol, args) => {
                if matches!(symbol.as_str(), "lpp_vec_i64x2" | "lpp_vec_i64x2_splat" | "lpp_vec_i64x2_add" | "lpp_vec_i64x2_sub" | "lpp_vec_i64x2_mul" | "lpp_vec_i64x2_xor" | "lpp_vec_i64x2_shr" | "lpp_vec_i64x2_shr_var" | "lpp_vec_i64x2_extract" | "lpp_vec_i64x2_sum") {
                    let vector_ty = "<2 x i64>";
                    let operand = |this: &mut Self, op: &Operand, out: &mut String| this.operand(op, out);
                    let vector = |this: &mut Self, op: &Operand, out: &mut String| operand(this, op, out);
                    let result = match symbol.as_str() {
                        "lpp_vec_i64x2" => {
                            if args.len() != 2 { return Err("vec_i64x2 requires two lanes".to_string()); }
                            let (first, _) = operand(self, &args[0], out)?;
                            let mut value = self.temp();
                            out.push_str(&format!("  {} = insertelement {} poison, i64 {}, i32 0\n", value, vector_ty, first));
                            for (index, arg) in args.iter().enumerate().skip(1) {
                                let (lane, _) = operand(self, arg, out)?;
                                let next = self.temp();
                                out.push_str(&format!("  {} = insertelement {} {}, i64 {}, i32 {}\n", next, vector_ty, value, lane, index));
                                value = next;
                            }
                            (value, vector_ty)
                        }
                        "lpp_vec_i64x2_splat" => {
                            let (lane, _) = operand(self, args.first().ok_or_else(|| "vector splat needs one argument".to_string())?, out)?;
                            let mut value = self.temp();
                            out.push_str(&format!("  {} = insertelement {} poison, i64 {}, i32 0\n", value, vector_ty, lane));
                            for index in 1..2 {
                                let next = self.temp();
                                out.push_str(&format!("  {} = insertelement {} {}, i64 {}, i32 {}\n", next, vector_ty, value, lane, index));
                                value = next;
                            }
                            (value, vector_ty)
                        }
                        "lpp_vec_i64x2_add" | "lpp_vec_i64x2_sub" | "lpp_vec_i64x2_mul" | "lpp_vec_i64x2_xor" | "lpp_vec_i64x2_shr" | "lpp_vec_i64x2_shr_var" => {
                            let (left, _) = vector(self, &args[0], out)?;
                            let (right_raw, right_ty) = operand(self, &args[1], out)?;
                            let right = if symbol == "lpp_vec_i64x2_shr" {
                                let mut splat = self.temp();
                                out.push_str(&format!("  {} = insertelement {} poison, i64 {}, i32 0\n", splat, vector_ty, right_raw));
                                for index in 1..2 { let next=self.temp(); out.push_str(&format!("  {} = insertelement {} {}, i64 {}, i32 {}\n",next,vector_ty,splat,right_raw,index)); splat=next; }
                                splat
                            } else { let _ = right_ty; right_raw };
                            let next = self.temp();
                            let instruction = match symbol.as_str() { "lpp_vec_i64x2_add" => "add", "lpp_vec_i64x2_sub" => "sub", "lpp_vec_i64x2_mul" => "mul", "lpp_vec_i64x2_xor" => "xor", _ => "ashr" };
                            out.push_str(&format!("  {} = {} {} {}, {}\n", next, instruction, vector_ty, left, right));
                            (next, vector_ty)
                        }
                        "lpp_vec_i64x2_extract" => {
                            let (value, _) = vector(self, &args[0], out)?;
                            let lane = match args.get(1) { Some(Operand::Int(index)) if (0..2).contains(index) => *index, _ => return Err("vector extract lane must be constant 0..3".to_string()) };
                            let next=self.temp(); out.push_str(&format!("  {} = extractelement {} {}, i32 {}\n",next,vector_ty,value,lane)); (next,"i64")
                        }
                        _ => {
                            let (value, _) = vector(self, &args[0], out)?;
                            let mut result=self.temp(); out.push_str(&format!("  {} = extractelement {} {}, i32 0\n",result,vector_ty,value));
                            for lane in 1..2 { let item=self.temp(); out.push_str(&format!("  {} = extractelement {} {}, i32 {}\n",item,vector_ty,value,lane)); let next=self.temp(); out.push_str(&format!("  {} = add i64 {}, {}\n",next,result,item)); result=next; }
                            (result,"i64")
                        }
                    };
                    return Ok(Some(result));
                }
                if symbol == "lpp_vec_i64_checksum" {
                    let arg = args.first().ok_or_else(|| "vector checksum needs a length".to_string())?;
                    let (n, _) = self.operand(arg, out)?;
                    let value = self.temp();
                    out.push_str(&format!("  {} = call i64 @__lpp_vec_i64_checksum(i64 {})\n", value, n));
                    return Ok(Some((value, "i64")));
                }
                let (ret, params) = self.register_builtin(symbol)?;
                let param_types: Vec<&str> = if params.is_empty() {
                    Vec::new()
                } else {
                    params.split(',').map(|s| s.trim()).collect()
                };
                let mut values = Vec::new();
                for (index, arg) in args.iter().enumerate() {
                    let (value, ty) = self.operand(arg, out)?;
                    let expected = param_types.get(index).copied().unwrap_or(ty);
                    let coerced = self.coerce(&value, ty, expected, out);
                    values.push(format!("{} {}", expected, coerced));
                }
                if ret == "void" {
                    out.push_str(&format!("  call void @{}({})\n", symbol, values.join(", ")));
                    Ok(None)
                } else {
                    let value = self.temp();
                    out.push_str(&format!("  {} = call {} @{}({})\n", value, ret, symbol, values.join(", ")));
                    Ok(Some((value, ret)))
                }
            }
            Rvalue::FieldAccess(base, field) => {
                let sid = match base {
                    Operand::Local(id) | Operand::Borrowed(id) => match self.function.locals[id.0].ty {
                        TypeRef::Custom(sid) => sid,
                        _ => return Err("LLVM field base is not a custom struct".to_string()),
                    },
                    _ => return Err("LLVM field base is not a local struct".to_string()),
                };
                let (base, _) = self.operand(base, out)?;
                let (address, ty) = self.field_ptr(&base, sid, field, out)?;
                let value = self.temp();
                out.push_str(&format!("  {} = load {}, ptr {}, align 8\n", value, llvm_type(&ty), address));
                Ok(Some((value, llvm_type(&ty))))
            }
            Rvalue::AllocateArcStruct(TypeRef::Custom(sid)) => {
                let (_, size) = struct_layout(self.type_table, *sid);
                let drop_name = format!("@__lpp_drop_{}", sid.0);
                let value = self.temp();
                self.declarations.insert("lpp_arc_alloc_with_destructor".to_string(), ("ptr".to_string(), "i64, ptr".to_string()));
                out.push_str(&format!("  {} = call ptr @lpp_arc_alloc_with_destructor(i64 {}, ptr {})\n", value, size, drop_name));
                Ok(Some((value, "ptr")))
            }
            Rvalue::AllocateArenaStruct(TypeRef::Custom(sid), arena) => {
                let (_, size) = struct_layout(self.type_table, *sid);
                let (arena, _) = self.operand(arena, out)?;
                let value = self.temp();
                let drop_name = format!("@__lpp_drop_{}", sid.0);
                self.declarations.insert("lpp_arena_alloc".to_string(), ("ptr".to_string(), "i64, i64, ptr".to_string()));
                out.push_str(&format!("  {} = call ptr @lpp_arena_alloc(i64 {}, i64 {}, ptr {})\n", value, size, arena, drop_name));
                Ok(Some((value, "ptr")))
            }
            Rvalue::AllocateStackStruct(TypeRef::Custom(sid)) => {
                let (_, size) = struct_layout(self.type_table, *sid);
                let value = self.temp();
                out.push_str(&format!("  {} = alloca [{} x i8], align 8\n", value, size.max(1)));
                Ok(Some((value, "ptr")))
            }
            Rvalue::AllocateList(element) => {
                let symbol = match element.list_element_class() {
                    ListElementClass::Scalar
                    | ListElementClass::Bool
                    | ListElementClass::Float => "lpp_list_new",
                    ListElementClass::Arc => "lpp_list_new_arc",
                    ListElementClass::Unsupported => {
                        return Err(format!("LLVM does not support List[{:?}] safely", element));
                    }
                };
                let (ret, _) = self.register_builtin(symbol)?;
                let value = self.temp();
                out.push_str(&format!("  {} = call {} @{}()\n", value, ret, symbol));
                Ok(Some((value, ret)))
            }
            Rvalue::MakeClosure(id, args) | Rvalue::MakeStackClosure(id, args) => {
                let stack = matches!(rvalue, Rvalue::MakeStackClosure(_, _));
                let capsule = self.temp();
                if stack { out.push_str(&format!("  {} = alloca [16 x i8], align 8\n", capsule)); }
                else {
                    self.declarations.insert("lpp_arc_alloc_with_destructor".to_string(), ("ptr".to_string(), "i64, ptr".to_string()));
                    self.declarations.insert("lpp_closure_destroy".to_string(), ("void".to_string(), "ptr".to_string()));
                    out.push_str(&format!("  {} = call ptr @lpp_arc_alloc_with_destructor(i64 16, ptr @__lpp_llvm_closure_destroy)\n", capsule));
                }
                let code = self.temp();
                out.push_str(&format!("  {} = bitcast ptr {} to ptr\n", code, self.func_names.get(id).ok_or_else(|| "unknown closure function".to_string())?));
                out.push_str(&format!("  store ptr {}, ptr {}, align 8\n", code, capsule));
                let env = args.first().ok_or_else(|| "closure missing environment".to_string())?;
                let (env, _) = self.operand(env, out)?;
                let env_ptr = self.temp();
                out.push_str(&format!("  {} = getelementptr i8, ptr {}, i64 8\n", env_ptr, capsule));
                out.push_str(&format!("  store ptr {}, ptr {}, align 8\n", env, env_ptr));
                Ok(Some((capsule, "ptr")))
            }
            Rvalue::CallIndirect(callee, args) => {
                let (closure, _) = self.operand(callee, out)?;
                let code = self.temp();
                let env_ptr = self.temp();
                out.push_str(&format!("  {} = load ptr, ptr {}, align 8\n", code, closure));
                out.push_str(&format!("  {} = getelementptr i8, ptr {}, i64 8\n", env_ptr, closure));
                let env = self.temp();
                out.push_str(&format!("  {} = load ptr, ptr {}, align 8\n", env, env_ptr));
                let mut values = vec![format!("ptr {}", env)];
                for arg in args { let (value, ty) = self.operand(arg, out)?; values.push(format!("{} {}", ty, value)); }
                let ret = llvm_type(dest_ty);
                if ret == "void" { out.push_str(&format!("  call void {}({})\n", code, values.join(", "))); Ok(None) }
                else { let value=self.temp(); out.push_str(&format!("  {} = call {} {}({})\n",value,ret,code,values.join(", "))); Ok(Some((value,ret))) }
            }
            Rvalue::SpawnThread(closure) => {
                let (closure, _) = self.operand(closure, out)?;
                let code = self.temp(); let env_ptr=self.temp(); let env=self.temp();
                out.push_str(&format!("  {} = load ptr, ptr {}, align 8\n",code,closure));
                out.push_str(&format!("  {} = getelementptr i8, ptr {}, i64 8\n",env_ptr,closure));
                out.push_str(&format!("  {} = load ptr, ptr {}, align 8\n",env,env_ptr));
                self.register_builtin("lpp_thread_spawn")?;
                out.push_str(&format!("  call void @lpp_thread_spawn(ptr {}, ptr {})\n",code,env));
                Ok(Some(("0".to_string(), "i64")))
            }
            Rvalue::FuncRef(id) => {
                let value=self.temp();
                out.push_str(&format!("  {} = ptrtoint ptr {} to i64\n",value,self.func_names.get(id).ok_or_else(|| "unknown function reference".to_string())?));
                Ok(Some((value,"i64")))
            }
            Rvalue::AllocateStruct(_) => Err("raw struct allocation reached LLVM backend".to_string()),
            _ => Err(format!("LLVM backend does not support MIR rvalue {}", rvalue)),
        }
    }

    fn emit(&mut self) -> Result<String, String> {
        let return_type = llvm_type(&self.function.return_type);
        let mut body = String::new();
        body.push_str(&format!("define internal {} {}(", return_type, self.func_names[&self.function.id]));
        let args: Vec<String> = self.function.params.iter().enumerate().map(|(i,p)| format!("{} %arg{}",llvm_type(&self.function.locals[p.0].ty),i)).collect();
        body.push_str(&args.join(", ")); body.push_str(") {\nentry:\n");
        for local in &self.function.locals {
            if local.ty != TypeRef::Void {
                let ptr = Self::local_ptr(local.id);
                let ty_str = llvm_type(&local.ty);
                body.push_str(&format!("  {} = alloca {}, align 8\n", ptr, ty_str));
                let zero_val = if local.ty == TypeRef::Float { "0.0" } else if ty_str == "ptr" { "null" } else if ty_str == "<2 x i64>" { "zeroinitializer" } else { "0" };
                body.push_str(&format!("  store {} {}, ptr {}, align 8\n", ty_str, zero_val, ptr));
            }
        }
        for (i,param) in self.function.params.iter().enumerate() { if self.function.locals[param.0].ty != TypeRef::Void { body.push_str(&format!("  store {} %arg{}, ptr {}, align 8\n",llvm_type(&self.function.locals[param.0].ty),i,Self::local_ptr(*param))); } }
        let first=self.function.blocks.first().ok_or_else(|| "LLVM function has no blocks".to_string())?;
        body.push_str(&format!("  br label %bb{}\n",first.id.0));
        for block in &self.function.blocks {
            body.push_str(&format!("bb{}:\n",block.id.0));
            for instruction in &block.instrs {
                match instruction {
                    MirInstr::Assign(dest,rv) => { if let Some((value,ty))=self.rvalue(rv,&self.function.locals[dest.0].ty,&mut body)? { if self.function.locals[dest.0].ty!=TypeRef::Void { self.store(*dest,&value,ty,&mut body); } } }
                    MirInstr::AssignField{base,field,value} => {
                        let (base_value,_) = self.operand(&Operand::Local(*base),&mut body)?;
                        let sid=match &self.function.locals[base.0].ty { TypeRef::Custom(id)=>*id, _=>return Err("LLVM field store base is not a custom struct".to_string()) };
                        let (address,field_ty)=self.field_ptr(&base_value,sid,field,&mut body)?;
                        let (raw_value, actual_ty) = self.operand(value,&mut body)?;
                        let value = self.coerce(&raw_value, actual_ty, llvm_type(&field_ty), &mut body);
                        body.push_str(&format!("  store {} {}, ptr {}, align 8\n",llvm_type(&field_ty),value,address));
                    }
                    MirInstr::Retain(local) => {
                        let arena = matches!(self.function.locals[local.0].ty, TypeRef::Custom(id) if self.type_table.definitions.get(id.0).map(|d| d.is_self_referential).unwrap_or(false));
                        let symbol = if arena { "lpp_arena_retain" } else { "lpp_arc_retain" };
                        self.register_builtin(symbol)?;
                        let (value, _) = self.operand(&Operand::Local(*local), &mut body)?;
                        body.push_str(&format!("  call void @{}(ptr {})\n", symbol, value));
                    }
                    MirInstr::Release(local) => {
                        let (value, _) = self.operand(&Operand::Local(*local), &mut body)?;
                        if self.function.locals[local.0].ownership.is_copy() {
                            match self.function.locals[local.0].ty {
                                TypeRef::Custom(id) => body.push_str(&format!("  call void @__lpp_drop_{}(ptr {})\n", id.0, value)),
                                TypeRef::Function => {
                                    self.register_builtin("lpp_closure_destroy")?;
                                    body.push_str(&format!("  call void @lpp_closure_destroy(ptr {})\n", value));
                                }
                                _ => {}
                            }
                        } else {
                            let arena = matches!(self.function.locals[local.0].ty, TypeRef::Custom(id) if self.type_table.definitions.get(id.0).map(|d| d.is_self_referential).unwrap_or(false));
                            let symbol = if arena { "lpp_arena_release_node" } else { "lpp_arc_release" };
                            self.register_builtin(symbol)?;
                            body.push_str(&format!("  call void @{}(ptr {})\n", symbol, value));
                        }
                    }
                }
            }
            match &block.terminator {
                Terminator::Goto(target)=>body.push_str(&format!("  br label %bb{}\n",target.0)),
                Terminator::If{cond,then_block,else_block}=>{let (value,ty)=self.operand(cond,&mut body)?;let test=self.temp();if ty=="ptr"{body.push_str(&format!("  {} = icmp ne ptr {}, null\n",test,value));}else{body.push_str(&format!("  {} = icmp ne {} {}, 0\n",test,ty,value));}body.push_str(&format!("  br i1 {}, label %bb{}, label %bb{}\n",test,then_block.0,else_block.0));}
                Terminator::IfCmp{op,left,right,then_block,else_block}=>{let (left,ty)=self.operand(left,&mut body)?;let (right,rty)=self.operand(right,&mut body)?;let right=self.coerce(&right,rty,ty,&mut body);let test=self.compare(op,&left,&right,ty,&mut body);body.push_str(&format!("  br i1 {}, label %bb{}, label %bb{}\n",test,then_block.0,else_block.0));}
                Terminator::Return(Some(op))|Terminator::ReturnOwned(op)=>{let (value,ty)=self.operand(op,&mut body)?;body.push_str(&format!("  ret {} {}\n",ty,value));}
                Terminator::Return(None)=>{
                    if return_type=="void"{body.push_str("  ret void\n");}
                    else if return_type=="ptr"{body.push_str("  ret ptr null\n");}
                    else if return_type=="double"{body.push_str("  ret double 0.0\n");}
                    else{body.push_str(&format!("  ret {} 0\n",return_type));}
                }
                Terminator::Unreachable=>body.push_str("  unreachable\n"),
            }
        }
        body.push_str("}\n"); Ok(body)
    }
}

fn emit_drop_functions(type_table: &TypeTable, weak_fields: &HashSet<(StructTypeId,String)>, declarations: &mut HashMap<String,(String,String)>) -> String {
    let mut out=String::new();
    declarations.insert("lpp_arc_release".to_string(),("void".to_string(),"ptr".to_string()));
    declarations.insert("lpp_arena_release_node".to_string(),("void".to_string(),"ptr".to_string()));
    for (index,definition) in type_table.definitions.iter().enumerate(){
        out.push_str(&format!("define internal void @__lpp_drop_{}(ptr %payload) {{\nentry:\n",index));
        let (layout,_)=struct_layout(type_table,StructTypeId(index));
        for ((field,ty),field_layout) in definition.fields.iter().zip(layout.iter()){
            if weak_fields.contains(&(StructTypeId(index),field.clone())) || !ty.is_managed(){continue;}
            let ptr=format!("%field{}_{}",index,field_layout.offset); out.push_str(&format!("  {} = getelementptr i8, ptr %payload, i64 {}\n",ptr,field_layout.offset));
            let value=format!("%value{}_{}",index,field_layout.offset); out.push_str(&format!("  {} = load ptr, ptr {}, align 8\n",value,ptr));
            let release=match ty{TypeRef::Custom(id) if type_table.definitions.get(id.0).map(|d|d.is_self_referential).unwrap_or(false)=>"lpp_arena_release_node",_=>"lpp_arc_release"};
            out.push_str(&format!("  call void @{}(ptr {})\n",release,value));
        }
        out.push_str("  ret void\n}\n");
    }
    out
}

fn emit_task_thunks(
    program: &MirProgram,
    func_names: &HashMap<FuncId, String>,
) -> String {
    let mut output = String::new();
    let mut functions: Vec<_> = program.functions.values().filter(|f| f.is_async).collect();
    functions.sort_by_key(|function| function.id.0);
    for function in functions {
        output.push_str(&format!(
            "define internal i64 @__lpp_task_thunk_{}(ptr %env) {{\nentry:\n",
            function.id.0
        ));
        let parameter_types: Vec<TypeRef> = function.params.iter()
            .map(|id| function.locals[id.0].ty.clone())
            .collect();
        let (layout, _) = tuple_layout(&parameter_types);
        let mut arguments = Vec::new();
        for (index, (ty, field)) in parameter_types.iter().zip(layout.iter()).enumerate() {
            output.push_str(&format!(
                "  %a{}_ptr = getelementptr i8, ptr %env, i64 {}\n",
                index, field.offset
            ));
            let align = field.align;
            output.push_str(&format!(
                "  %a{} = load {}, ptr %a{}_ptr, align {}\n",
                index, llvm_type(ty), index, align
            ));
            arguments.push(format!("{} %a{}", llvm_type(ty), index));
        }
        if function.return_type == TypeRef::Void {
            output.push_str(&format!(
                "  call void {}({})\n  ret i64 0\n}}\n",
                func_names[&function.id], arguments.join(", ")
            ));
        } else {
            output.push_str(&format!(
                "  %result = call {} {}({})\n",
                llvm_type(&function.return_type),
                func_names[&function.id],
                arguments.join(", ")
            ));
            match &function.return_type {
                TypeRef::Float => output.push_str("  %raw = bitcast double %result to i64\n"),
                TypeRef::Bool => output.push_str("  %raw = zext i8 %result to i64\n"),
                ty if llvm_type(ty) == "ptr" =>
                    output.push_str("  %raw = ptrtoint ptr %result to i64\n"),
                _ => output.push_str("  %raw = add i64 %result, 0\n"),
            }
            output.push_str("  ret i64 %raw\n}\n");
        }
    }
    output
}

fn emit_vector_checksum() -> &'static str {
    r#"define internal i64 @__lpp_vec_i64_checksum(i64 %n) {
entry:
  %has_vec = icmp uge i64 %n, 4
  br i1 %has_vec, label %vec_loop, label %tail_loop
vec_loop:
  %i = phi i64 [ 0, %entry ], [ %next_i, %vec_loop ]
  %acc = phi i64 [ 0, %entry ], [ %next_acc, %vec_loop ]
  %i1 = add i64 %i, 1
  %i2 = add i64 %i, 2
  %i3 = add i64 %i, 3
  %v0 = insertelement <4 x i64> poison, i64 %i, i64 0
  %v1 = insertelement <4 x i64> %v0, i64 %i1, i64 1
  %v2 = insertelement <4 x i64> %v1, i64 %i2, i64 2
  %vi = insertelement <4 x i64> %v2, i64 %i3, i64 3
  %mul = mul <4 x i64> %vi, <i64 3, i64 3, i64 3, i64 3>
  %shr = lshr <4 x i64> %vi, <i64 1, i64 1, i64 1, i64 1>
  %xor = xor <4 x i64> %mul, %shr
  %x0 = extractelement <4 x i64> %xor, i64 0
  %x1 = extractelement <4 x i64> %xor, i64 1
  %x2 = extractelement <4 x i64> %xor, i64 2
  %x3 = extractelement <4 x i64> %xor, i64 3
  %s01 = add i64 %x0, %x1
  %s23 = add i64 %x2, %x3
  %batch = add i64 %s01, %s23
  %next_acc = add i64 %acc, %batch
  %next_i = add i64 %i, 4
  %next_plus = add i64 %next_i, 4
  %more = icmp ule i64 %next_plus, %n
  br i1 %more, label %vec_loop, label %tail_loop
 tail_loop:
  %ti = phi i64 [ 0, %entry ], [ %next_i, %vec_loop ], [ %next_ti, %tail_body ]
  %ta = phi i64 [ 0, %entry ], [ %next_acc, %vec_loop ], [ %next_ta, %tail_body ]
  %has_tail = icmp ult i64 %ti, %n
  br i1 %has_tail, label %tail_body, label %done
 tail_body:
  %tm = mul i64 %ti, 3
  %ts = lshr i64 %ti, 1
  %tx = xor i64 %tm, %ts
  %next_ta = add i64 %ta, %tx
  %next_ti = add i64 %ti, 1
  br label %tail_loop
 done:
  ret i64 %ta
}
"#
}

pub fn compile(program: &MirProgram, type_table: &TypeTable, weak_fields: &HashSet<(StructTypeId,String)>) -> Result<Vec<u8>, String> {
    for function in program.functions.values(){
        if !is_supported_type(&function.return_type)||function.locals.iter().any(|local|!is_supported_type(&local.ty)){return Err(format!("LLVM backend cannot lower unsupported type in '{}'",function.name));}
    }
    let mut func_names=HashMap::new();let mut signatures=HashMap::new();let mut ordered:Vec<&MirFunction>=program.functions.values().collect();ordered.sort_by_key(|f|f.id.0);
    for function in &ordered{let name=format!("@lpp_fn_{}",function.id.0);func_names.insert(function.id,name.clone());signatures.insert(function.id,FunctionSig{name,ret:llvm_type(&function.return_type)});}
    let mut strings=Vec::new();let mut declarations:HashMap<String,(String,String)>=HashMap::new();let mut bodies=Vec::new();
    for function in ordered{let mut emitter=FunctionEmitter{function,func_names:&func_names,signatures:&signatures,type_table,strings:&mut strings,declarations:&mut declarations,next_value:0};bodies.push(emitter.emit()?);}
    let drops=emit_drop_functions(type_table,weak_fields,&mut declarations);
    if program.functions.values().any(|function| function.name == "main" && function.is_async) {
        declarations.insert("lpp_tuple_alloc".to_string(), ("ptr".to_string(), "i64, i64, i64".to_string()));
        declarations.insert("lpp_task_new".to_string(), ("ptr".to_string(), "ptr, ptr, i64".to_string()));
        declarations.insert("lpp_task_await".to_string(), ("i64".to_string(), "ptr".to_string()));
        declarations.insert("lpp_arc_release".to_string(), ("void".to_string(), "ptr".to_string()));
    }
    let needs_closure_wrapper = declarations.contains_key("lpp_closure_destroy");
    let mut ir=String::from("target triple = \"x86_64-pc-linux-gnu\"\n\n");let mut decls:Vec<_>=declarations.into_iter().collect();decls.sort_by(|a,b|a.0.cmp(&b.0));
    for(name,(ret,params)) in decls{ir.push_str(&format!("declare {} @{}({})\n",ret,name,params));}
    for(i,value)in strings.iter().enumerate(){let blob=literal_blob(value);ir.push_str(&format!("@.lpp_str{} = private unnamed_addr constant [{} x i8] c\"{}\", align 16\n",i,blob.len(),escape_bytes(&blob)));}
    ir.push('\n');for body in bodies{ir.push_str(&body);ir.push('\n');}ir.push_str(&drops);ir.push_str(&emit_task_thunks(program, &func_names));ir.push_str(emit_vector_checksum());
    if needs_closure_wrapper {
        ir.push_str("define internal void @__lpp_llvm_closure_destroy(ptr %closure) {\nentry:\n  call void @lpp_closure_destroy(ptr %closure)\n  ret void\n}\n");
    }
    if let Some(main)=program.functions.values().find(|f|f.name=="main"){
        if main.is_async {
            ir.push_str(&format!(
                "define i32 @main() {{\nentry:\n  %env = call ptr @lpp_tuple_alloc(i64 16, i64 0, i64 0)\n  %task = call ptr @lpp_task_new(ptr @__lpp_task_thunk_{}, ptr %env, i64 0)\n  %ignored = call i64 @lpp_task_await(ptr %task)\n  call void @lpp_arc_release(ptr %task)\n  ret i32 0\n}}\n",
                main.id.0
            ));
        } else {
            ir.push_str(&format!("define i32 @main() {{\nentry:\n  call void {}()\n  ret i32 0\n}}\n",func_names[&main.id]));
        }
    }
    if cfg!(target_os = "windows") {
        ir.push_str("define void @__main() {\nentry:\n  ret void\n}\n");
    }
    let stamp = format!(
        "lpp-llvm-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos()
    );
    let ll = std::env::temp_dir().join(format!("{}.ll", stamp));
    let obj = std::env::temp_dir().join(format!("{}.o", stamp));
    fs::write(&ll, ir).map_err(|e| format!("write LLVM IR: {}", e))?;

    let config_llvm_path = crate::config::LppConfig::load_or_create().llvm_path;
    let compiler = std::env::var("LPP_LLVM_CC")
        .ok()
        .or(config_llvm_path)
        .unwrap_or_else(|| "clang".to_string());

    let mut command = Command::new(&compiler);
    command.args(["-c", "-x", "ir", "-O2", "-ffreestanding"]);
    if let Ok(march) = std::env::var("LPP_LLVM_MARCH") {
        command.arg(format!("-march={}", march));
    }
    let status = command
        .arg(&ll)
        .args(["-o"])
        .arg(&obj)
        .status()
        .map_err(|e| format!("invoke LLVM compiler '{}': {}", compiler, e))?;
    if !status.success() {
        return Err(format!(
            "LLVM compiler '{}' failed; IR kept at {}",
            compiler,
            ll.display()
        ));
    }
    let bytes = fs::read(&obj).map_err(|e| format!("read LLVM object: {}", e))?;
    let _ = fs::remove_file(ll);
    let _ = fs::remove_file(obj);
    Ok(bytes)
}
