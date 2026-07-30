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

## Phase 2 — experimental C source translation

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
