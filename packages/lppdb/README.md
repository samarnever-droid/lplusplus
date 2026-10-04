# lppdb — a real embedded SQL database, written in pure L++

`lppdb` is a genuine, working embedded database engine. It is **not** a stub and
**not** SQLite-compatible: it uses its own binary file format and implements an
honest, clearly-scoped subset of SQL. Every query result is computed from the
bytes actually stored on disk — nothing is hardcoded or faked.

It replaces the previous `sqlite` / `lpp-sqlite` package in this repository,
which was a non-functional stub that returned hardcoded responses (for example
`SELECT COUNT(*)` always reported `42`, and `JOIN`/`CTE`/`FTS5` returned empty
or canned results). That package and its false claims have been removed.

## What it really does

- **Real binary storage.** Tables, typed cells, and rows are serialized into a
  length-prefixed binary format and persisted with `buf_read` / `buf_write`.
  Close the handle, reopen the file, and the data is still there (the test
  suite verifies this).
- **Real SQL parsing & execution.** A tokenizer + parser drives genuine
  execution: rows are scanned, predicates evaluated, aggregates accumulated,
  and results sorted — against the stored data.
- **Typed cells.** `INTEGER` is stored as an 8-byte little-endian two's
  complement integer; `TEXT` as length-prefixed bytes; `NULL` is supported.
- **Works on both linkers.** Pure L++ over the `buf_*` builtins, so it builds
  and runs with `--linker host` *and* the zero-dependency `--linker direct`
  freestanding runtime.

## Supported SQL (single table)

```
CREATE TABLE t (col INTEGER | TEXT, ...)
INSERT INTO t [(col, ...)] VALUES (v, ...) [, (v, ...)]
SELECT * | col, col | COUNT(*) | SUM(c) | MIN(c) | MAX(c)
       FROM t [WHERE cond [AND cond ...]] [ORDER BY c [ASC|DESC]] [LIMIT n]
UPDATE t SET c = v [, c = v] [WHERE cond]
DELETE FROM t [WHERE cond]
DROP TABLE t
```

`WHERE` conditions support `=`, `!=`/`<>`, `<`, `<=`, `>`, `>=`, joined with
`AND`. Values may be integers, `'single'`/`"double"`-quoted strings, or `NULL`.

## What it does NOT do (and does not claim to)

No `JOIN`, subqueries, CTEs (`WITH`), full-text search, views, triggers,
indexes, transactions/savepoints, or `REAL`/floating-point arithmetic (a `REAL`
column is stored as `TEXT`). These were all falsely advertised by the old stub.
The storage engine scans rows linearly (no B-tree / index), which is fine for
small-to-medium data but is not designed for very large tables.

## File format (v1, little-endian)

```
[magic "LPPDB001" (8 bytes)] [u32 num_tables]
table block:
  [u32 block_len] [u32 name_len] [name bytes] [u32 ncols]
  per column: [u32 name_len] [name bytes] [u8 type]     # type: 1=INT 2=TEXT
  [u32 num_rows]
  per row: [u32 row_len] cells...
cell:
  [u8 type] [u32 len] [bytes]                           # type 0=NULL(len 0)
                                                        #       1=INT(len 8)
                                                        #       2=TEXT(len n)
```

## API

```
db_open(path) -> handle        # open or create a database file
db_exec(handle, sql) -> Str    # run a statement; returns a JSON result string
db_save(handle, path)          # flush to disk
db_close(handle)               # release memory
```

`db_exec` returns JSON, e.g.
`{"status":"ok","columns":["name","age"],"rows":[["Alice",30],...]}` for
`SELECT`, `{"status":"ok","changes":3}` for `INSERT`/`UPDATE`/`DELETE`, or
`{"status":"error","message":"..."}` on failure.

## Running the test suite

`src/main.lpp` imports the library and runs 24 end-to-end checks (schema,
inserts, every aggregate, `WHERE`/`ORDER BY`/`LIMIT`, update, delete, drop, and
persistence across a close/reopen):

```
lpp packages/lppdb/src/main.lpp --linker host && packages/lppdb/src/main
```

All 24 pass.
