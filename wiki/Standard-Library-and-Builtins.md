# Standard Library and Builtins

L++ has two standard-library layers:

1. **Builtins** declared in `src/builtins.rs` and implemented by the runtime.
2. **Pure L++ modules** in `stdlib/`.

## Console

| Function | Meaning |
|---|---|
| `print(x)` | print integer/bool-ish value |
| `print_str(s)` | print string |
| `input()` | read a line |
| `parse_int(s)` | parse integer |

## Strings

Common string functions:

```lpp
str_len(s)
str_concat(a, b)
str_repeat(s, n)
str_find(s, needle)
str_contains(s, needle)
str_starts_with(s, prefix)
str_ends_with(s, suffix)
str_replace(s, old, new)
str_substr(s, start, len)
str_trim(s)
str_upper(s)
str_lower(s)
char_at(s, index)
ord(s)
chr(code)
int_to_str(n)
str_to_int(s)
```

Example:

```lpp
def main():
    print_str(str_upper("abc"))
    print_str(chr(65))
    print(ord("A"))
```

## Lists

```lpp
list_new()
list_push(list, value)
list_get(list, index)
list_len(list)
list_free(list)
```

List literals are also supported:

```lpp
nums := [10, 20, 30]
print(nums[0])
```

## Maps

```lpp
map_new()
map_put(map, key, value)
map_get(map, key)
map_has(map, key)
map_len(map)
map_remove(map, key)
```

## Files and paths

```lpp
read_file(path)
write_file(path, data)
append_file(path, data)
delete_file(path)
file_exists(path)
file_size(path)
file_copy(src, dst)
file_move(src, dst)
dir_create(path)
dir_list(path)
dir_remove(path)
path_exists(path)
path_join(base, child)
```

## Binary buffers

The buffer API is important for binary formats such as ZIP:

```lpp
buf_alloc(size)
buf_free(handle)
buf_len(handle)
buf_get8(handle, off)
buf_set8(handle, off, value)
buf_get16le(handle, off)
buf_set16le(handle, off, value)
buf_get32le(handle, off)
buf_set32le(handle, off, value)
buf_read(path)
buf_write(path, handle)
buf_crc32(handle, off, len)
buf_copy(dst, dst_off, src, src_off, len)
buf_write_str(handle, off, str)
buf_read_str(handle, off, len)
```

## Process, environment, networking, JSON

```lpp
command_exec(cmd)
command_output(cmd)
env_get(key)
env_set(key, value)

net_dial(host, port)
net_listen(port)
net_accept(listener)
net_send(conn, data)
net_recv(conn, size)
net_close(conn)
http_get(url)
http_post(url, body)

json_parse(text)
json_get_int(obj, key)
json_get_str(obj, key)
json_get_obj(obj, key)
json_free(obj)
```

## Pure L++ modules

| Module | Purpose |
|---|---|
| `stdlib.math` | `abs`, `min`, `max`, `pow`, `gcd`, `fib`, `factorial` |
| `stdlib.strings` | higher-level string helpers |
| `stdlib.collections` | list helpers |
| `stdlib.algo` | sorting/search helpers, experimental |
| `stdlib.convert` | conversion helpers |
| `stdlib.assert` | assertions and panic helpers |

Some stdlib files are still experimental; the core verified examples avoid the experimental modules that are not yet repo-wide `--checkall` clean.
