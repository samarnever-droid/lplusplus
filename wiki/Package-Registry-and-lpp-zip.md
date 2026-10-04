# Package Registry and lpp-zip

The official package registry is a GitHub Pages-hosted JSON index:

```text
https://samarnever-droid.github.io/lplusplus/registry/index.json
```

The registry is Git-based and simple by design. A package entry points to source files in the repository or to raw URLs.

## Current packages

- `lpp-zip`
- `lpp-math`
- `lpp-strings`
- `lpp-collections`
- `lpp-algo`
- `lpp-convert`

## lpp-zip

`lpp-zip` is the first real package-style library. It is written in L++ and uses binary buffer builtins.

Source:

```text
packages/lpp-zip/src/zip.lpp
```

Package manifest:

```text
packages/lpp-zip/lpp.toml
```

## lpp-zip API

```lpp
zip_create() -> Int
zip_add_file(archive: Int, filename: Str, content: Str)
zip_save(archive: Int, path: Str)
zip_free(archive: Int)

zip_open(path: Str) -> Int
zip_entry_count(handle: Int) -> Int
zip_entry_name(handle: Int, index: Int) -> Str
zip_entry_data(handle: Int, index: Int) -> Str
zip_close(handle: Int)
```

## Example

```lpp
import zip

def main():
    archive := zip_create()
    zip_add_file(archive, "hello.txt", "Hello from L++!")
    zip_add_file(archive, "numbers.txt", "1 2 3 4 5")
    zip_save(archive, "/tmp/example.zip")
    zip_free(archive)

    handle := zip_open("/tmp/example.zip")
    count := zip_entry_count(handle)
    print(count)
    print_str(zip_entry_name(handle, 0))
    print_str(zip_entry_data(handle, 0))
    zip_close(handle)
```

## Current limitations

- `lpp-zip` depends heavily on `buf_*` builtins.
- Host linker runtime is the safest path.
- Freestanding direct runtimes may not support every buffer builtin on every platform yet.
- Package tests must run with the correct package import layout. If `import zip` fails, run from the package root or ensure `src/zip.lpp` is visible in the import path.

## Publishing package entries

A registry entry should include:

```json
{
  "name": "lpp-example",
  "version": "0.1.0",
  "description": "Example package",
  "source_url": "https://raw.githubusercontent.com/.../example.lpp",
  "dependencies": [],
  "keywords": ["example"]
}
```
