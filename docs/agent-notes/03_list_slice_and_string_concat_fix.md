# LIST builtin slice + string `+` concat fix

_Corpus: PASS 114 → 116 (59.5%), E5003 9 → 7, ASSERTFAIL 0._

## 1. LIST builtin slice (E5003 → PASS)

Ported the `List[T]` mutation / ordering / search family from the C reference
into the Rust runtime (`crates/lpp-runtime/src/list.rs`) and wired each symbol
into the Cranelift lowering (`crates/lpp-codegen-cranelift/src/lower.rs`).

### Runtime (`list.rs`, before `#[cfg(test)]`)
`slots_mut(l)` helper returns a `&mut [i64]` view over the payload (empty list →
`NonNull::dangling()` base, len 0). Then 14 `extern "C"` fns:

| symbol | behaviour |
| --- | --- |
| `lpp_list_insert(l, idx, v)` | clamp idx to `[0,len]`, grow, `ptr::copy` shift right, `retain_element`, len+1 |
| `lpp_list_remove(l, idx)` | panic on OOB, save slot, shift left, len-1, return the raw value **without** dropping (ownership transfers to caller) |
| `lpp_list_swap(l, i, j)` | panic on OOB, swap slots |
| `lpp_list_reverse(l)` | reverse in place |
| `lpp_list_truncate(l, n)` | `drop_element` each dropped tail slot, set len |
| `lpp_list_reserve` / `lpp_list_capacity` / `lpp_list_clear` | capacity mgmt |
| `lpp_list_sort` / `lpp_list_sort_desc` | `sort_unstable` signed |
| `lpp_list_sort_u` | unsigned (`as u64`) compare |
| `lpp_list_index_of(l, v)` | linear scan → idx or `-1` |
| `lpp_list_binary_search(l, v)` | → idx, or `-(insertion_point+1)` when absent |
| `lpp_list_extend(dst, src)` | snapshot `src` slots first (safe when `dst == src`), then `lpp_list_push` each |

Key runtime facts: `LppList { data:*mut i64, len:i64, cap:i64, retain_element/drop_element:Option<extern "C" fn(i64)> }`.
`grow()` doubles capacity and **updates `l.data`** — always call it *before* a
shift. All 14 descriptors have lowering params `I64` (result `Void`/`I64`), so
they take the generic `FAMILY_A` path — no float/bool dispatch. Value/list slots
are `Any`; both int and str elements lower to machine type `I64` (str = ptr), so
one symbol serves both element classes.

### Codegen (`lower.rs`)
- 14 names added to the `FAMILY_A` const.
- 14 `import_signature()` arms: `insert`/`swap` → `[I64,I64,I64]`; `remove` /
  `capacity` / `index_of` / `binary_search` → `Some(I64)` result; the rest
  `[I64]`/`[I64,I64]` → `None`.

## 2. String `+` operator bug (pre-existing, now FIXED)

**Symptom:** `s := "a" + "b"` silently aborted the program (exit 0, no output);
the following statement never ran. `str_concat("a","b")` worked fine.

**Root cause:** `lower_binary` (lower.rs ~4365) lowered `BinaryOperator::Add`
for a String operand (CLType `I64` pointer) as `builder.ins().iadd(l, r)` — it
*added the two heap pointers as integers*, producing a garbage pointer.

**Fix (mirrors the existing string-`==` path):**
1. New const `IMP_STR_CONCAT = "lpp_str_concat"` (`import_signature` already
   maps it to `([I64,I64], Some(I64))`).
2. Pre-scan pass: when a `Rvalue::Binary` has operator `Add` and the left
   operand's language kind is `Primitive(String)`, insert `IMP_STR_CONCAT` into
   the import set (so the func gets declared).
3. `Rvalue::Binary` call-site: for `Add` on a string, emit a call to
   `lpp_str_concat(l, r)` and take its first result, instead of calling
   `lower_binary`. `lpp_str_concat` **borrows** both operands (reads bytes, does
   not release them) and returns a fresh owned ARC string (+1), so the operands
   are passed borrowed (`operand_value`), exactly like the `str_eq` path.

This is a codegen/lowering bug (not E5003). It blocked any program using `+`
for string concatenation — the two list stress files used
`"ASSERTIONS: " + int_to_str(...)`, so before the fix they exited 0 with **no**
output (a hollow harness pass). After the fix they print real assertions
(`8/8`, `6/6`, `SUCCESS`).

## Verification
- `cargo build --bin lpp` rc=0.
- `cargo test -p lpp-runtime` (6 ok) and `-p lpp-codegen-cranelift` (6 ok,
  incl. byte-identical-object + C-shim-drop-in differential tests).
- Corpus harness `scripts/corpus_correctness.sh run`: PASS=116, E5003=7,
  ASSERTFAIL=0.
- Manual: `"a"+"b"`, `"x="+int_to_str(5)`, chained `"a"+"-"+"b"+"!"`, and both
  list stress files all produce correct output.
