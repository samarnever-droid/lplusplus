# lpp-zip

**ZIP archive library for L++** — the first package in the L++ registry.

Pure L++ implementation using `buf_*` binary primitives. No C code, no external dependencies, no linker changes needed.

## Usage

```lpp
import zip

def main():
    # Create
    archive := zip_create()
    zip_add_file(archive, "hello.txt", "Hello, World!")
    zip_add_file(archive, "data.csv", "name,age\nAlice,30\nBob,25")
    zip_save(archive, "output.zip")
    zip_free(archive)

    # Read
    handle := zip_open("output.zip")
    count := zip_entry_count(handle)
    print(count)

    name := zip_entry_name(handle, 0)
    print_str(name)

    data := zip_entry_data(handle, 0)
    print_str(data)

    zip_close(handle)
```

## API

| Function | Signature | Description |
|----------|-----------|-------------|
| `zip_create` | `() -> Int` | Create new archive handle |
| `zip_add_file` | `(archive, filename, content)` | Add file to archive |
| `zip_save` | `(archive, path)` | Write ZIP to disk |
| `zip_free` | `(archive)` | Release memory |
| `zip_open` | `(path) -> Int` | Open ZIP for reading |
| `zip_entry_count` | `(handle) -> Int` | Number of entries |
| `zip_entry_name` | `(handle, index) -> Str` | Get entry filename |
| `zip_entry_data` | `(handle, index) -> Str` | Get entry content |
| `zip_close` | `(handle)` | Close handle |

## Features

- ✅ Create ZIP archives with multiple files and directories
- ✅ Read ZIP archives and extract individual entries
- ✅ CRC32 checksum verification
- ✅ STORE method (no compression — data stored as-is)
- ✅ Compatible with standard ZIP tools (7-Zip, WinZip, macOS Archive Utility)
- ✅ Pure L++ — works with both host linker (cc) and direct linker (lpp-link PE)
- ✅ ~200 lines of L++ code

## Install

```toml
# lpp.toml
[dependencies]
lpp-zip = "0.1.0"
```

## License

MIT
