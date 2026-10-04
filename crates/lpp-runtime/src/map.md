# `crates/lpp-runtime/src/map.rs` — hash map runtime

Rust re-implementation of `runtime/lpp_map.c` with an **identical ABI**. The
rewrite links the Rust runtime cdylib (`liblpp_runtime.so`), which previously
had no map symbols, so map programs failed to lower/link. This module supplies
every `lpp_map_*` symbol the frozen v1 ABI declares.

## Data model
- `LppMap` — `#[repr(C)]` header: `entries` pointer, `cap`, `len`, `arc_values`
  flag. The map object itself is ARC-allocated (`lpp_arc_alloc_with_destructor`)
  with `map_destroy` as its destructor, so it participates in the same ownership
  discipline as lists/strings.
- `LppMapEntry` — `#[repr(C)]`: `key`, `val` (both `i64`), `is_str_key`,
  `occupied` (0 empty / 1 live / 2 tombstone). The entry table is a `calloc`'d
  C array (zeroed → all-empty), freed in the destructor.

## Algorithm (byte-for-byte with the C reference)
- Open addressing, linear probing.
- FNV-1a hash for string keys, a bit-mix hash for integer keys.
- Load factor 0.7 (counting tombstones); growth doubles unless the live count is
  very sparse (tombstone-heavy → rehash same cap). Rehash drops tombstones.
- Tombstone reuse on insert (first tombstone in the probe chain).
- String keys store the raw payload pointer; compared with `strcmp`. The
  `is_str_key` flag keeps an int key and a str key with the same bit pattern
  from colliding.

## Exported symbols
`lpp_map_new`, `lpp_map_new_arc`, `lpp_map_put`, `lpp_map_put_str`,
`lpp_map_put_float`, `lpp_map_put_str_float`, `lpp_map_get`, `lpp_map_get_str`,
`lpp_map_get_float`, `lpp_map_get_str_float`, `lpp_map_has`, `lpp_map_has_str`,
`lpp_map_len`, `lpp_map_remove`, `lpp_map_remove_str`, `lpp_map_keys`,
`lpp_map_values`, `lpp_map_clear`, `lpp_map_capacity`.

Values may be plain `i64`, bit-packed `f64` (the `_float` variants), or
ARC-managed pointers (in `arc`-mode maps: retained on insert, released on
overwrite/remove/clear/destroy). `map_keys`/`map_values` return `List` handles
(ARC-owning when they carry string keys / managed values).

## How the compiler reaches these symbols
- **Checker** (`lpp-types`): map builtins result in `Any`/`i64` types; an unbound
  `Any` handle is grounded to `Int` in `normalize_assignments`. `map_has` is
  typed `Bool`. String-key calls are specialized to the `_str` builtin in
  `specialize_polymorphic_builtin` (dispatch on the key argument type).
- **Codegen** (`lpp-codegen-cranelift`): the map builtins form **Family MAP**
  (`is_family_map` = name starts with `map_`/`lpp_map_`), lowered like Family A
  (direct call to the descriptor's runtime symbol). `import_signature` carries a
  Cranelift signature for each `lpp_map_*` symbol; predicates (`map_has*`)
  reduce the runtime `i64` 0/1 to a machine `i8`. The `phase5c2_gate` census
  accounts for the 30 map ids as Family MAP (moved out of the Family D
  rejection set).
