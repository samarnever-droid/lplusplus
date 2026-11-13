# Type System & Safety

L++ is statically typed with local inference (`:=`), immutable bindings by default, generic monomorphization, traits, and checked `Option` / `Result` error handling.

---

## 1. Local Type Inference & Immutability

```lpp
def compound_interest(principal: Float, rate: Float, periods: Int) -> Float:
    # 'principal', 'rate', and 'periods' are explicitly typed
    factor := lpp_pow(1.0 + rate, 1.0 * periods)
    return principal * factor
```

- Local `x := expr` bindings are **immutable** unless declared `mut x := expr`.
- Function parameters and return types (when returning a non-`Void` value) are explicitly annotated.

---

## 2. Explicit Optionality (`Option` / `Result`)

Safe L++ code has no null reference values; optional and fallible results use sum types (`enum`):

```lpp
enum OptionInt:
    Some(value: Int)
    None

def show_opt(opt: OptionInt):
    match opt:
        OptionInt.Some(v) =>
            print_str("Found value: " + int_to_str(v))
        OptionInt.None =>
            print_str("No value")
```

---

## 3. Generics & Traits

Generic structs and functions are specialized via monomorphization:

```lpp
struct Box[T]:
    items: List[T]

trait Printable:
    def to_str(self) -> Str

def render[T: Printable](item: T):
    print_str("Item: " + item.to_str())
```

---

## 4. Exhaustive `match` & Variant Payloads

```lpp
enum Direction:
    North
    South
    East
    West

def move_player(dir: Direction):
    match dir:
        Direction.North => print_str("Going north")
        Direction.South => print_str("Going south")
        Direction.East  => print_str("Going east")
        Direction.West  => print_str("Going west")
```

---

## 5. Safety Boundaries

| Concern | Current L++ Guarantee (`v0.2.0`) |
|---|---|
| **Type mismatches** | Caught statically during type checking before MIR/codegen. |
| **Bounds checks** | List and slice accesses validate bounds and trap cleanly on out-of-range indices. |
| **Ownership balance & leaks** | Enforced via `lpp-ownership` (`E4403`) on Cranelift native; trial-deletion collects `Shared` cycles (`CYCLE_BREAKING.md`). WASM lacks cycle collection today. |
| **Raw FFI pointers** | C pointers carried in `Int` via `extern "C"` are caller-managed (`EXTERNAL_C_ABI.md`). |
| **Concurrent shared mutation** | ARC refcounts are atomic, but `Send`/`Sync` compile-time trait enforcement is still open (`CONCURRENCY.md`). |
