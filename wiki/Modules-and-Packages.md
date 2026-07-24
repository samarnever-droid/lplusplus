# Modules and Packages

## Imports

```lpp
import utils.mathx
from stdlib.math import pow
```

Dotted imports map to paths:

```text
utils.mathx -> utils/mathx.lpp
```

## Example

`utils/mathx.lpp`:

```lpp
def double(x: Int) -> Int:
    return x * 2

def triple(x: Int) -> Int:
    return x * 3
```

`main.lpp`:

```lpp
import utils.mathx
from stdlib.math import pow

def main():
    print(double(21))
    print(triple(10))
    print(pow(2, 8))
```

## Search order

When the compiler sees an import, it searches:

1. Relative to the importing file
2. `.lpp_packages/<package>/...`
3. The bundled `stdlib/`

## Package manager

```bash
lpp new app
cd app
lpp add lpp-zip
lpp install
lpp build
lpp run
```

## Registry

The JSON registry is hosted through GitHub Pages:

```text
https://samarnever-droid.github.io/lplusplus/registry/index.json
```

Packages currently include:

- `lpp-zip`
- `lpp-math`
- `lpp-strings`
- `lpp-collections`
- `lpp-algo`
- `lpp-convert`

## Import aliases

Alias syntax is parsed:

```lpp
import utils.mathx as mathx
```

Namespace behavior for aliases is still experimental, so stable examples should prefer plain `import module` or `from module import item`.
