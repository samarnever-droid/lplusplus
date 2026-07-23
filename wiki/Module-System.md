# Module System

## Import Syntax

```lpp
import math                        # loads math.lpp from same directory
import utils.helpers               # loads utils/helpers.lpp (dotted path)
from math import add, multiply     # selective import
from stdlib.math import abs, pow   # import from standard library
import math as m                   # alias (parsed, namespace TBD)
```

## Search Order

When you write `import foo`, the compiler searches:

1. `./foo.lpp` — same directory as the source file
2. `.lpp_packages/foo/foo.lpp` — installed packages
3. `.lpp_packages/foo/src/foo.lpp` — package src layout
4. `stdlib/foo.lpp` — standard library (alongside compiler binary)

For dotted paths like `import utils.math`:
- Converts dots to directory separators: `utils/math.lpp`

## Selective Import

```lpp
from math import add, multiply
# Only add() and multiply() are available, not other functions in math.lpp
```

Currently, selective imports still load the entire file (all declarations are merged). The `from` syntax documents intent and will enforce visibility in a future version.

## Circular Import Prevention

The compiler tracks imported files in a `HashSet`. If a module has already been imported, it's skipped. This prevents infinite recursion.

## Package Layout

```
myproject/
  lpp.toml
  src/
    main.lpp          # import utils
  utils.lpp           # found by import search
  .lpp_packages/
    somelib/
      somelib.lpp     # found by import search
```

## Standard Library Modules

The `stdlib/` directory ships with the compiler:

| Module | Functions |
|--------|-----------|
| `stdlib.math` | abs, min, max, pow, gcd, fib, factorial |
| `stdlib.strings` | str_repeat, str_contains, str_reverse |
| `stdlib.collections` | list_sum, list_max, list_reverse |
| `stdlib.algo` | bubble_sort, binary_search |
| `stdlib.result` | Result, Option, is_ok, unwrap |
| `stdlib.convert` | int_to_str, bool_to_str |
