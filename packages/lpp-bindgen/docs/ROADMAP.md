# c2lpp roadmap

## Phase 1 — usable native binding packages

- [x] Pure-L++ executable
- [x] Optional C preprocessing
- [x] Integer macro constants
- [x] Opaque pointers and pointer-to-pointer parameters
- [x] Const strings, numeric and Bool ABI types
- [x] Callback parameter recognition
- [x] Multiline prototypes
- [x] Variadic/unsupported ABI diagnostics
- [x] Native library metadata and include dependency report
- [x] SQLite and zlib fixtures

## Phase 1.1 — policy files

- Rename symbols and parameters
- Override C type mappings
- Mark owned, borrowed, nullable and out parameters
- Pair constructors/destructors
- Generate high-level safe wrappers
- Platform-specific link names and include paths

## Phase 1.2 — real SDK scale

- Consume clang JSON AST through a normalized interchange file
- Typedef graph resolution
- Full enum and bitflag extraction
- Struct/union layout records
- Calling conventions and attributes
- Conditional API availability
- Incremental cache keyed by header/compiler/flags

## Phase 1.3 — whole-input project audit

- [x] Checked-buffer streaming scanner with file/offset/line/column provenance
- [x] Exact byte-partition and zero-unknown invariants
- [x] Multi-file source/header manifest
- [x] Deterministic local/external/unresolved include graph
- [x] Stable blocked-construct reason codes and sampled source positions
- [x] Pinned SQLite 3.46.1 whole-amalgamation scale gate
- [x] Logical preprocessor directive skipping across trailing `\` lines
- [x] General arbitrary-order translation-unit partition and span graph
- [x] Pinned active SQLite graph: 4,430 declarations, zero unknown records
- [x] SQLite base-type-family shapes: 4,430/4,430 resolved
- [x] SQLite structural body graph: 2,528/2,528 balanced, 45,222 statements
- [x] Automatic typed semantic sweep: 62/2,528 pure-L++ functions
- [x] Scalar/aggregate typedef registry with ABI widths (232 SQLite families)
- [x] Lazy conditional returns, local initializers and assignments
- [x] Pointer-aware short-circuit logical operators
- [x] Character literal normalization
- [x] Primitive/pointer/aggregate/non-evaluated-dereference `sizeof`
- [x] Sequenced comma expression statements
- [x] Ternary-aware structural label scanning
- [x] Demand-emitted immutable integer arrays (3 referenced SQLite arrays)
- [x] Demand-bounded aggregate catalog (10 selected demands, 9 layouts, 100 fields)
- [x] Scalar arrow/nested-member/bitfield/fixed-array place lowering
- [x] ABI-width data-pointer fields with raw provenance side tables
- [x] Pointer-depth-two dereference/index/store lowering
- [x] One-dimensional array-parameter decay
- [x] Aggregate-pointer parameter/return signatures and null semantics
- [x] Parser-integrated scalar pointer dereference/index/address/store places
- [x] Pointer locals, arithmetic, difference, equality and typedef casts
- [x] Mutable-token ownership crash removed from pointer-prefix parsing
- [x] Unary-complement, primitive/void casts and expression statements
- [x] Recursive if/else, while and canonical for loops with scoped indices
- [x] Braced do/while with bottom-test semantics and break
- [x] Unbraced if/else call statements and empty statements
- [x] Nearest-loop break/continue; for-continue preserves induction update
- [x] 953 bounded SQLite signatures pre-registered for forward calls
- [x] Accepted-call closure removes wrappers targeting rejected functions
- [x] Swept SQLite module type-checks with zero extern/native links
- [ ] Full conditional/macro preprocessing with physical-source span remapping
- [ ] Resolve complete nested C declarators, parameter types, attributes and ABI
- [ ] Expand typed expression/statement AST coverage from 12 to all 2,528 functions
- [x] Resolve all 779 direct SQLite goto labels function-locally
- [x] Whole-SQLite call graph and ownership-site inventory
- [x] Cross-pass graph denominator/ownership consistency checks
- [ ] Resolve switch/fallthrough/break/continue targets and dominance
- [ ] Path-sensitive ownership transfer/escape proof for 560 functions

## Phase 1.35 — versioned JSON project configuration

- [x] Pure-L++ JSON parser for project settings
- [x] Required schema name and integer version
- [x] Strict unknown/duplicate key rejection
- [x] String/Bool/Int field type validation
- [x] Cross-field mode/input/manifest validation
- [x] Deterministic normalized configuration output
- [x] Non-zero failure status and malformed-config tests
- [x] No `C2LPP_*` environment settings in converter execution
- [ ] Portable argv config-path selection on every runtime backend
- [ ] Nested platform/type/ownership policy objects

## Phase 1.4 — typed normalized-IR vertical slice

- [x] On-demand tokens with file/offset/line/column provenance
- [x] Scalar C type records and per-function symbol tables
- [x] Precedence-aware unary/binary expression parser
- [x] Typed calls with argument and arity validation
- [x] Scalar locals, assignments, compound assignments and returns
- [x] Stable normalized-IR serialization before L++ emission
- [x] Function-atomic rejection and recovery
- [x] Generated L++ check/build/run plus native C equivalence
- [ ] Typedef/declaration graph and forward declarations
- [x] Provenance-bearing pointer/allocation lowering runtime
- [x] SysV x86-64 struct/union/integer-bitfield layout foundation
- [x] Explicit global/static zero/init storage foundation
- [x] Checked switch/fallthrough/goto CFG state-machine foundation
- [x] Native equivalence and ASan+UBSan/LeakSanitizer foundation gates
- [x] Strict no-binding profile integrates typedef struct/bitfield declarations
- [x] Profile integrates index/member/address/dereference/arrow places
- [x] Profile integrates global pointer relocation dependency order
- [x] Profile integrates calloc/free no-escape ownership checks
- [x] Profile integrates canonical loop and switch/fallthrough/goto CFG
- [x] Profile output contains only pure L++ and fails without binding fallback
- [x] Profile v2 forward/incomplete and callback typedef graph
- [x] Profile v2 const string/integer arrays and nested aggregate arrays
- [x] Profile v2 ordinary and variadic prototype graph without extern emission
- [x] Profile v2 macro physical/expansion provenance
- [x] Profile v2 expression places and automatic loop/switch/goto lowering
- [x] Flattened integer/bitfield place foundation + 28 differential/safety checks
- [x] Pointer-valued slot foundation + 16 differential/safety checks
- [x] Place ASan/UBSan/LeakSanitizer gate after nested-CPtr redesign
- [ ] Floating/volatile/atomic place storage
- [ ] Generalize conditional expressions to call arguments and nested expressions
- [x] Generalize scalar pointer index/dereference/address/store parsing
- [x] Generalize bounded scalar aggregate member/array parsing
- [x] Add data-pointer aggregate fields with provenance side tables
- [ ] Add function-pointer field target sets and indirect calls
- [ ] Add pointer depth above two and multidimensional array layout
- [ ] Expand aggregate declarations beyond the 10-demand budget
- [ ] Add do/while continue condition trampolines
- [ ] Build CFG automatically for all C statements, nested labels and edges
- [ ] Generalize global/static initializer DAG beyond one pointer relocation
- [ ] Generalize ownership/pairing analysis across arbitrary call graphs

## Curated functional backend

- [x] Standalone vendored pure-L++ SQLite-compatible backend
- [x] Generated adapter API and pure package manifest
- [x] CRUD, real-SQLite reopen and integrity-check gate
- [x] Zero extern/native-SQLite link/C source in generated package
- [x] Explicit `source_translation_complete=0` provenance boundary

## Phase 2 — legacy scalar C source translation

- [x] Scalar function signatures and parameters
- [x] Scalar local declarations and initialization
- [x] Arithmetic, assignments and function calls
- [x] `if`/`else` and `while`
- [x] Canonical ascending `for` -> L++ `range`
- [x] Fixed local scalar arrays -> checked L++ lists
- [x] Array reads and writes through checked indexing/`list_set`
- [x] Increment/decrement rewriting
- [x] Return translation
- [x] Unsupported-construct count, report and strict mode
- [x] Native C vs translated L++ output-equivalence gate
- [ ] Pointer/borrowed-buffer safety model
- [ ] General `for` lowering with arbitrary conditions/increments
- [ ] Region inference for C allocation patterns
- [ ] Union and tagged-union policy
- [ ] Macro expansion provenance in translated source
- [ ] `goto` to structured CFG/MIR
- [ ] Explicit quarantine for undefined behavior

A C source translator must never silently convert unsafe C ownership into
ordinary safe L++ ownership. Unproven code should remain behind an FFI or
`unsafe` boundary until the language has one.
