# Standard Library

## Builtin Functions (91 total)

### Console I/O
| Function | Signature | Description |
|----------|-----------|-------------|
| `print(n)` | `Int → Void` | Print integer + newline |
| `print_str(s)` | `Str → Void` | Print string + newline |
| `input()` | `→ Str` | Read line from stdin |
| `parse_int(s)` | `Str → Int` | Parse string to integer |

### String Operations
| Function | Signature |
|----------|-----------|
| `str_len(s)` | `Str → Int` |
| `str_concat(a, b)` | `Str, Str → Str` |
| `str_repeat(s, n)` | `Str, Int → Str` |
| `str_find(s, sub)` | `Str, Str → Int` (-1 if not found) |
| `str_replace(s, old, new)` | `Str, Str, Str → Str` |
| `str_substr(s, start, len)` | `Str, Int, Int → Str` |
| `str_split(s, delim)` | `Str, Int → List` |
| `str_trim(s)` | `Str → Str` |

### List Operations
| Function | Signature |
|----------|-----------|
| `list_new()` | `→ List` |
| `list_push(lst, val)` | `List, Int → Void` |
| `list_get(lst, idx)` | `List, Int → Int` |
| `list_len(lst)` | `List → Int` |
| `list_free(lst)` | `List → Void` |

### Map Operations
| Function | Signature |
|----------|-----------|
| `map_new()` | `→ Map` |
| `map_put(m, k, v)` | `Map, Int, Int → Void` |
| `map_get(m, k)` | `Map, Int → Int` |
| `map_has(m, k)` | `Map, Int → Bool` |
| `map_len(m)` | `Map → Int` |
| `map_remove(m, k)` | `Map, Int → Void` |

### File I/O
| Function | Signature |
|----------|-----------|
| `read_file(path)` | `Str → Str` |
| `write_file(path, data)` | `Str, Str → Void` |
| `append_file(path, data)` | `Str, Str → Void` |
| `delete_file(path)` | `Str → Int` |
| `file_exists(path)` | `Str → Int` |
| `file_size(path)` | `Str → Int` |
| `file_copy(src, dst)` | `Str, Str → Int` |
| `file_move(src, dst)` | `Str, Str → Int` |

### Directory Operations
| Function | Signature |
|----------|-----------|
| `dir_create(path)` | `Str → Int` |
| `dir_list(path)` | `Str → List` |
| `dir_remove(path)` | `Str → Int` |
| `path_exists(path)` | `Str → Int` |
| `path_join(base, child)` | `Str, Str → Str` |

### Process Execution
| Function | Signature |
|----------|-----------|
| `command_exec(cmd)` | `Str → Int` (exit code) |
| `command_output(cmd)` | `Str → Str` (stdout) |
| `env_get(key)` | `Str → Str` |
| `env_set(key, val)` | `Str, Str → Int` |

### Binary Buffers
| Function | Signature |
|----------|-----------|
| `buf_alloc(size)` | `Int → Int` (handle) |
| `buf_free(handle)` | `Int → Void` |
| `buf_len(handle)` | `Int → Int` |
| `buf_get8/set8` | byte access |
| `buf_get16le/set16le` | 16-bit LE access |
| `buf_get32le/set32le` | 32-bit LE access |
| `buf_read(path)` | `Str → Int` (file → buffer) |
| `buf_write(path, handle)` | `Str, Int → Void` (buffer → file) |
| `buf_crc32(handle, off, len)` | `Int, Int, Int → Int` |
| `buf_copy(dst, doff, src, soff, len)` | copy bytes between buffers |
| `buf_write_str / buf_read_str` | string ↔ buffer |

### Networking
| Function | Description |
|----------|-------------|
| `net_dial(host, port)` | TCP connect |
| `net_listen(port)` | TCP listen |
| `net_accept(listener)` | Accept connection |
| `net_send(conn, data)` | Send bytes |
| `net_recv(conn, size)` | Receive bytes |
| `net_close(conn)` | Close connection |
| `net_dial_udp / net_listen_udp / net_recv_udp` | UDP |
| `net_resolve(hostname)` | DNS lookup |
| `http_get(url)` | HTTP GET |
| `http_post(url, body)` | HTTP POST |

### JSON
| Function | Description |
|----------|-------------|
| `json_parse(str)` | Parse JSON string |
| `json_get_int(obj, key)` | Get integer field |
| `json_get_str(obj, key)` | Get string field |
| `json_get_obj(obj, key)` | Get nested object |
| `json_free(obj)` | Free JSON object |

---

## Pure L++ Standard Library Modules

Located in `stdlib/`, imported via `from stdlib.X import func`:

### stdlib/math.lpp
`abs`, `min`, `max`, `clamp`, `sign`, `gcd`, `pow`, `factorial`, `fib`, `is_even`, `is_odd`

### stdlib/strings.lpp
`str_repeat`, `str_contains`, `str_starts_with`, `str_ends_with`, `str_reverse`, `str_is_empty`, `str_pad_left`, `str_pad_right`

### stdlib/collections.lpp
`list_sum`, `list_max`, `list_min`, `list_contains`, `list_reverse`, `list_count`

### stdlib/algo.lpp
`bubble_sort`, `binary_search`, `list_fill`, `list_range`

### stdlib/convert.lpp
`int_to_str`, `bool_to_str`

### stdlib/result.lpp
`enum Result: Ok(value: Int), Err(code: Int)`
`enum Option: Some(value: Int), None`
`is_ok`, `is_err`, `unwrap`, `is_some`, `is_none`, `unwrap_or`

## Published Packages

### lpp-zip (packages/lpp-zip/)
ZIP archive create/read library. Pure L++ using `buf_*` primitives.
- `zip_create`, `zip_add_file`, `zip_save`, `zip_free`
- `zip_open`, `zip_entry_count`, `zip_entry_name`, `zip_entry_data`, `zip_close`
