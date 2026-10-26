use std::collections::{BTreeMap, BTreeSet};

/// Buffered possible cycle roots that trigger an opportunistic
/// trial deletion mid-execution. End-of-execution collection always
/// runs regardless of this threshold.
const CYCLE_COLLECT_THRESHOLD: usize = 512;

use lpp_hir::{BinaryOperator, OriginId, UnaryOperator};

use super::*;
use crate::{Constant, InstructionKind, MirPlaceId, Operand, PlaceProjection, Rvalue, Terminator};

mod builtins;
mod execution;
mod heap;
mod places;
mod value_ops;

// ── Deterministic builtin helpers ──────────────────────────────────────────
// The helpers below mirror the v1 host runtime (runtime/lpp_str.c and
// runtime/lpp_int.c) so interpreter output matches legacy execution for the
// deterministic pure subset.

fn builtin_invalid(
    function: MirFunctionId,
    block: BasicBlockId,
    origin: OriginId,
    argument: usize,
    actual: ExecutionValueKind,
) -> InterpreterError {
    InterpreterError {
        function: Some(function),
        block: Some(block),
        origin: Some(origin),
        kind: InterpreterErrorKind::InvalidBuiltinArgument { argument, actual },
    }
}

fn builtin_division_by_zero(
    function: MirFunctionId,
    block: BasicBlockId,
    origin: OriginId,
) -> InterpreterError {
    InterpreterError {
        function: Some(function),
        block: Some(block),
        origin: Some(origin),
        kind: InterpreterErrorKind::DivisionByZero,
    }
}

fn builtin_index_out_of_bounds(
    function: MirFunctionId,
    block: BasicBlockId,
    origin: OriginId,
    index: i64,
    len: usize,
) -> InterpreterError {
    InterpreterError {
        function: Some(function),
        block: Some(block),
        origin: Some(origin),
        kind: InterpreterErrorKind::IndexOutOfBounds { index, len },
    }
}

fn builtin_argument_kind(arguments: &[RuntimeValue], position: usize) -> ExecutionValueKind {
    match arguments.get(position) {
        Some(value) => value.kind(),
        None => ExecutionValueKind::Void,
    }
}

fn argument_int(arguments: &[RuntimeValue], position: usize) -> Option<i64> {
    match arguments.get(position)? {
        RuntimeValue::Int(value) => Some(*value),
        _ => None,
    }
}

fn argument_float_bits(arguments: &[RuntimeValue], position: usize) -> Option<u64> {
    match arguments.get(position)? {
        RuntimeValue::FloatBits(bits) => Some(*bits),
        _ => None,
    }
}

fn argument_bool(arguments: &[RuntimeValue], position: usize) -> Option<bool> {
    match arguments.get(position)? {
        RuntimeValue::Bool(value) => Some(*value),
        _ => None,
    }
}

fn argument_string_id(arguments: &[RuntimeValue], position: usize) -> Option<HeapId> {
    match arguments.get(position)? {
        RuntimeValue::String(id) => Some(*id),
        _ => None,
    }
}

fn argument_list_id(arguments: &[RuntimeValue], position: usize) -> Option<HeapId> {
    match arguments.get(position)? {
        RuntimeValue::List(id) => Some(*id),
        _ => None,
    }
}

fn argument_map_id(arguments: &[RuntimeValue], position: usize) -> Option<HeapId> {
    match arguments.get(position)? {
        RuntimeValue::Map(id) => Some(*id),
        _ => None,
    }
}

/// Whether the value is a heap node (string, list, map, nominal, closure,
/// or task).
fn is_heap_value(value: &RuntimeValue) -> bool {
    matches!(
        value,
        RuntimeValue::String(_)
            | RuntimeValue::List(_)
            | RuntimeValue::Map(_)
            | RuntimeValue::Nominal(_)
            | RuntimeValue::Closure(_)
            | RuntimeValue::Task(_)
    )
}

/// Whether the value owns at least one heap reference, looking through
/// inline tuples (a tuple slot's death releases its heap elements).
fn owns_heap_refs(value: &RuntimeValue) -> bool {
    match value {
        RuntimeValue::Tuple(elements) => elements.iter().any(owns_heap_refs),
        _ => is_heap_value(value),
    }
}

/// Whether the type names a heap value (a runtime heap node, or an
/// inline tuple that can own one).
fn is_heap_type(types: &TypeInterner, ty: TypeId) -> bool {
    match types.kind(ty) {
        TypeKind::Function { .. }
        | TypeKind::List(_)
        | TypeKind::Slice(_)
        | TypeKind::Task(_)
        | TypeKind::Tuple(_)
        | TypeKind::Map { .. }
        | TypeKind::Nominal { .. } => true,
        TypeKind::Primitive(PrimitiveType::String) => true,
        _ => false,
    }
}

fn function_id_of_closure(interpreter: &Interpreter<'_>, id: HeapId) -> MirFunctionId {
    match &interpreter.heap[id.0 as usize] {
        HeapNode::Closure { function, .. } => *function,
        _ => unreachable!("closure handles reference closure heap nodes"),
    }
}

fn heap_id_of(value: &RuntimeValue) -> HeapId {
    match value {
        RuntimeValue::String(id)
        | RuntimeValue::List(id)
        | RuntimeValue::Map(id)
        | RuntimeValue::Nominal(id)
        | RuntimeValue::Closure(id)
        | RuntimeValue::Task(id) => *id,
        other => unreachable!("heap values carry heap ids: {other:?}"),
    }
}

fn heap_string(heap: &[HeapNode], id: HeapId) -> &str {
    match &heap[id.0 as usize] {
        HeapNode::String(text) => text,
        _ => unreachable!("string handles reference string heap nodes"),
    }
}

fn heap_list<'heap>(heap: &'heap [HeapNode], id: HeapId) -> &'heap Vec<RuntimeValue> {
    match &heap[id.0 as usize] {
        HeapNode::List(entries) => entries,
        _ => unreachable!("list handles reference list heap nodes"),
    }
}

fn heap_list_mut<'heap>(heap: &'heap mut [HeapNode], id: HeapId) -> &'heap mut Vec<RuntimeValue> {
    match &mut heap[id.0 as usize] {
        HeapNode::List(entries) => entries,
        _ => unreachable!("list handles reference list heap nodes"),
    }
}

fn heap_map(heap: &[HeapNode], id: HeapId) -> &[(MapKey, i64)] {
    match &heap[id.0 as usize] {
        HeapNode::Map(entries) => entries,
        _ => unreachable!("map handles reference map heap nodes"),
    }
}

fn heap_map_mut(heap: &mut [HeapNode], id: HeapId) -> &mut Vec<(MapKey, i64)> {
    match &mut heap[id.0 as usize] {
        HeapNode::Map(entries) => entries,
        _ => unreachable!("map handles reference map heap nodes"),
    }
}

fn builtin_make_string(
    interpreter: &mut Interpreter<'_>,
    function: MirFunctionId,
    block: BasicBlockId,
    origin: OriginId,
    text: String,
    produced: Option<TypeId>,
) -> Result<RuntimeValue, InterpreterError> {
    let id = interpreter.allocate_heap(
        function,
        Some(block),
        origin,
        HeapNode::String(text),
        produced,
    )?;
    Ok(RuntimeValue::String(id))
}

fn checked_binary(
    result: Option<i64>,
    function: MirFunctionId,
    block: BasicBlockId,
    origin: OriginId,
) -> Result<RuntimeValue, InterpreterError> {
    match result {
        Some(value) => Ok(RuntimeValue::Int(value)),
        None => Err(InterpreterError {
            function: Some(function),
            block: Some(block),
            origin: Some(origin),
            kind: InterpreterErrorKind::IntegerOverflow,
        }),
    }
}

fn abs_i64(value: i64) -> i64 {
    if value < 0 {
        value.wrapping_neg()
    } else {
        value
    }
}

fn min_u64(left: i64, right: i64) -> i64 {
    if (left as u64) < (right as u64) {
        left
    } else {
        right
    }
}

fn max_u64(left: i64, right: i64) -> i64 {
    if (left as u64) > (right as u64) {
        left
    } else {
        right
    }
}

/// v1 case folding touches ASCII letters only.
fn ascii_lowercase(text: &str) -> String {
    text.chars()
        .map(|character| {
            (if matches!(character, 'A'..='Z') {
                character as u8 + 32
            } else {
                character as u8
            }) as char
        })
        .collect()
}

/// v1 case folding touches ASCII letters only.
fn ascii_uppercase(text: &str) -> String {
    text.chars()
        .map(|character| {
            (if matches!(character, 'a'..='z') {
                character as u8 - 32
            } else {
                character as u8
            }) as char
        })
        .collect()
}

/// C `strtoll(s, NULL, 10)`: skip whitespace, optional sign, leading decimal
/// digits, clamp on overflow, and return 0 when no conversion is possible.
fn parse_i64_decimal(text: &str) -> i64 {
    let mut characters = text.chars().peekable();
    while matches!(
        characters.peek(),
        Some(' ') | Some('\t') | Some('\n') | Some('\u{0B}') | Some('\u{0C}') | Some('\r')
    ) {
        characters.next();
    }
    let sign = match characters.peek() {
        Some('+') => {
            characters.next();
            1
        }
        Some('-') => {
            characters.next();
            -1
        }
        _ => 1,
    };
    let mut value: i64 = 0;
    let mut converted = false;
    // Track true overflow: `saturating` arithmetic alone cannot tell
    // "exactly i64::MAX" from "past it", and a negative input past
    // i64::MAX must clamp to i64::MIN, not i64::MIN + 1.
    let mut overflow = false;
    for character in characters {
        let Some(digit) = (character as u8).checked_sub(b'0') else {
            break;
        };
        if digit > 9 {
            break;
        }
        let digit = digit as i64;
        if value > (i64::MAX - digit) / 10 {
            overflow = true;
            value = i64::MAX;
        } else {
            value = value * 10 + digit;
        }
        converted = true;
    }
    if !converted {
        return 0;
    }
    if sign < 0 {
        if overflow { i64::MIN } else { -value }
    } else {
        value
    }
}

/// v1 `lpp_str_to_u64`: skip whitespace, optional `0x` prefix, then hex or
/// decimal digits collected with wrapping shifts; stops at the first
/// non-digit.
fn parse_u64_hex_or_decimal(text: &str) -> i64 {
    let mut characters = text.chars().peekable();
    while matches!(
        characters.peek(),
        Some(' ') | Some('\t') | Some('\n') | Some('\r')
    ) {
        characters.next();
    }
    let mut hexadecimal = false;
    if characters.peek() == Some(&'0') {
        let second = characters.clone().nth(1);
        if matches!(second, Some('x') | Some('X')) {
            characters.next();
            characters.next();
            hexadecimal = true;
        }
    }
    let mut value: u64 = 0;
    for character in characters {
        if hexadecimal {
            match character.to_ascii_lowercase().to_digit(16) {
                Some(digit) => value = value.wrapping_shl(4) | digit as u64,
                None => break,
            }
        } else {
            match character {
                '0'..='9' => value = value.wrapping_mul(10) + character as u64 - b'0' as u64,
                _ => break,
            }
        }
    }
    value as i64
}

/// C `%g` with the default precision (six significant digits): trailing
/// zeros are stripped, and the exponent form appears when the exponent is
/// below -4 or at/above 6.
fn format_percent_g(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value.is_infinite() {
        return if value < 0.0 {
            "-inf".to_owned()
        } else {
            "inf".to_owned()
        };
    }
    let negative = value.is_sign_negative();
    let magnitude = value.abs();
    if magnitude == 0.0 {
        return if negative { "-0".into() } else { "0".into() };
    }
    let scientific = format!("{magnitude:.5e}");
    let (mantissa, exponent_part) = scientific.split_once('e').expect("scientific form");
    let exponent = i32::from_str_radix(exponent_part, 10).unwrap_or(0);
    let digits: String = mantissa.chars().filter(|&c| c != '.').collect();
    let trimmed = digits.trim_end_matches('0');
    let trimmed = if trimmed.is_empty() { "0" } else { trimmed };
    let sign = if negative { "-" } else { "" };
    let significant = i32::try_from(trimmed.len()).unwrap_or(i32::MAX);
    if exponent < -4 || exponent >= 6 {
        // C `%g` exponent form: `d[.ddd]e±EE` with trailing zeros stripped.
        let mantissa = if significant == 1 {
            trimmed.to_string()
        } else {
            format!("{}.{}", &trimmed[..1], &trimmed[1..])
        };
        format!("{sign}{mantissa}e{:+03}", exponent)
    } else if exponent >= significant - 1 {
        // Whole number: remaining positions are zeros.
        let zeros = "0".repeat((exponent - (significant - 1)) as usize);
        format!("{sign}{trimmed}{zeros}")
    } else if exponent >= 0 {
        let split_at = (exponent as usize) + 1;
        format!("{sign}{}.{}", &trimmed[..split_at], &trimmed[split_at..])
    } else {
        let zeros = "0".repeat((-exponent - 1) as usize);
        format!("{sign}0.{zeros}{trimmed}")
    }
}

/// Pure leak classification over recomputed counts: `internal[i]` is
/// the number of owning edges into `i` from live nodes, `edges` the
/// live adjacency. Returns `None` for nodes that are dead and
/// unreferenced (the healthy case).
fn classify_leak_classes(
    refcounts: &[u32],
    internal: &[u32],
    edges: &[Vec<usize>],
) -> Vec<Option<LeakClass>> {
    let n = refcounts.len().min(internal.len());
    let mut classes: Vec<Option<LeakClass>> = vec![None; n];
    let mut stack: Vec<usize> = Vec::new();
    for i in 0..n {
        if refcounts[i] == 0 {
            if internal[i] > 0 {
                classes[i] = Some(LeakClass::DanglingTarget);
            }
        } else if refcounts[i] != internal[i] {
            classes[i] = Some(LeakClass::PhantomRoot);
            stack.push(i);
        }
    }
    // Everything live and reachable from a phantom root is held by
    // it; anything else live belongs to a closed component.
    while let Some(i) = stack.pop() {
        if let Some(targets) = edges.get(i) {
            for &target in targets {
                if target < n && classes[target].is_none() && refcounts[target] > 0 {
                    classes[target] = Some(LeakClass::HeldByPhantom);
                    stack.push(target);
                }
            }
        }
    }
    for i in 0..n {
        if classes[i].is_none() && refcounts[i] > 0 {
            classes[i] = Some(LeakClass::ClosedGarbage);
        }
    }
    classes
}

#[cfg(test)]
mod leak_classification_tests {
    use super::LeakClass;
    use super::classify_leak_classes;

    #[test]
    fn closed_two_cycle_is_closed_garbage() {
        // 0 <-> 1, counts fully explained internally.
        let classes = classify_leak_classes(&[1, 1], &[1, 1], &[vec![1], vec![0]]);
        assert_eq!(
            classes,
            vec![
                Some(LeakClass::ClosedGarbage),
                Some(LeakClass::ClosedGarbage)
            ]
        );
    }

    #[test]
    fn phantom_root_holds_its_reachable_subgraph() {
        // 0 has one unexplained reference and holds 1, which holds 2;
        // 3 is an independent self-loop.
        let classes = classify_leak_classes(
            &[2, 1, 1, 1],
            &[1, 1, 1, 1],
            &[vec![1], vec![2], vec![], vec![3]],
        );
        assert_eq!(classes[0], Some(LeakClass::PhantomRoot));
        assert_eq!(classes[1], Some(LeakClass::HeldByPhantom));
        assert_eq!(classes[2], Some(LeakClass::HeldByPhantom));
        assert_eq!(classes[3], Some(LeakClass::ClosedGarbage));
    }

    #[test]
    fn dead_but_referenced_node_is_dangling() {
        // 0 leaks (phantom) and points at dead 1.
        let classes = classify_leak_classes(&[1, 0], &[0, 1], &[vec![1], vec![]]);
        assert_eq!(classes[0], Some(LeakClass::PhantomRoot));
        assert_eq!(classes[1], Some(LeakClass::DanglingTarget));
    }

    #[test]
    fn undercounted_live_node_is_phantom() {
        // More owning edges than the refcount admits: a retain was
        // missed somewhere in the modelled traffic.
        let classes = classify_leak_classes(&[1], &[2], &[vec![]]);
        assert_eq!(classes[0], Some(LeakClass::PhantomRoot));
    }

    #[test]
    fn healthy_heap_classifies_nothing() {
        let classes = classify_leak_classes(&[0, 0], &[0, 0], &[vec![], vec![]]);
        assert_eq!(classes, vec![None, None]);
    }
}
