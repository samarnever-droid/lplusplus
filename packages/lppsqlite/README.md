# lppsqlite

**A SQLite-file-format-compatible database engine written entirely in L++.**

Databases written by this engine are opened, read, written and verified by the
real `sqlite3` — `PRAGMA integrity_check` returns `ok` — and databases created
by the real `sqlite3` are read and modified by this engine. Nothing is
delegated to a C SQLite library: the file header, varints, record encoding,
B+tree pages, overflow chains, freelist, `sqlite_schema` catalogue, SQL
tokenizer, parser and executor are all implemented in ~9,700 lines of L++.

```
$ ./lppsqlite mydb.db "CREATE TABLE users(id INTEGER PRIMARY KEY, name TEXT, age INTEGER);"
$ ./lppsqlite mydb.db "INSERT INTO users VALUES (1,'Alice',30),(2,'Bob',25);"
$ ./lppsqlite -header mydb.db "SELECT name, age FROM users WHERE age > 26;"
name|age
Alice|30

$ sqlite3 mydb.db "PRAGMA integrity_check; SELECT COUNT(*) FROM users;"
ok
2
```

---

## Table of contents

- [Why this is "real" compatibility](#why-this-is-real-compatibility)
- [Quick start](#quick-start)
- [Command-line shell](#command-line-shell)
- [Architecture](#architecture)
- [What is implemented](#what-is-implemented)
- [What is *not* implemented](#what-is-not-implemented)
- [Known differences from SQLite](#known-differences-from-sqlite)
- [Testing](#testing)
- [Performance](#performance)
- [Notes on writing this in L++](#notes-on-writing-this-in-l)

---

## Why this is "real" compatibility

Compatibility is claimed only where it is mechanically verified:

| Claim | How it is proven |
|---|---|
| Files are valid SQLite databases | Every differential test opens the engine's output with the real `sqlite3` and asserts `PRAGMA integrity_check` = `ok` |
| Query results agree with SQLite | 87 differential cases run the *same SQL* through both engines and compare output byte for byte |
| Real SQLite files can be read | `t_interop` opens a database built by Python's `sqlite3`, including REAL/BLOB/NULL, 9 KB overflow rows, non-ASCII text and 64-bit integers |
| Writes into real files survive | The same test inserts, updates and deletes in a SQLite-authored file, then hands it back to `sqlite3` for verification |
| Encoding is byte-exact | Record and varint encoders are compared against an independent reference implementation of the published format |

The record encoder emits exactly the bytes real SQLite does, e.g. for the row
`(1, 'Alice', 30)`:

```
04 09 17 01 41 6c 69 63 65 1e     <- lppsqlite
04 09 17 01 41 6c 69 63 65 1e     <- SQLite file-format spec
```

---

## Quick start

Requires the L++ toolchain (`lpp` + its runtime). By default the build looks in
`~/lpp-toolchain`; override with `LPP_TOOLCHAIN`.

```sh
./build.sh                 # build build/lppsqlite
./build.sh --tests         # also build the test binaries
./run-tests.sh             # unit suites + differential tests vs real sqlite3
```

---

## Command-line shell

```
lppsqlite [options] <file.db|:memory:> ["SQL" ...]
```

| Option | Meaning |
|---|---|
| `-header` | print column headers |
| `-csv` | CSV output (RFC-4180 quoting) |
| `-json` | JSON array output |
| `-f FILE` | execute SQL read from a file |

Dot-commands: `.tables`, `.schema [TABLE]`, `.headers on|off`,
`.mode list|csv|json`, `.read FILE`, `.dump`, `.databases`, `.help`, `.quit`.
With no SQL argument the shell starts a REPL.

```sh
./lppsqlite -json data.db "SELECT name, salary FROM emp ORDER BY salary DESC LIMIT 2;"
[
  {"name":"Ada","salary":120},
  {"name":"Bo","salary":100}
]
```

> L++ v4.3.0 exposes no OS-level `argv`, so the `lppsqlite` shell script passes
> arguments to the compiled binary through environment variables
> (`LPPSQLITE_DB`, `LPPSQLITE_SQL`, …). The binary can be driven directly the
> same way.

---

## Architecture

```
                     SQL text
                        │
   lexer.lpp     ──►  tokens          keywords, strings, blobs, params, comments
   parser.lpp    ──►  AST + stmt      recursive descent, full operator precedence
   ast.lpp             (flat arena)   struct-of-arrays node storage
                        │
   exec.lpp      ──►  execution       scan → join → filter → group → project
    ├── eval.lpp       expressions    3-valued logic, affinity-aware comparison
    ├── func.lpp       ~30 built-ins  string/math/type functions
    └── rowset.lpp     result sets
                        │
   schema.lpp    ──►  sqlite_schema   CREATE parsing, affinity rules, catalogue
   btwrite.lpp   ──►  insert/split/delete/prune
   btree.lpp     ──►  B+tree pages    cells, overflow chains, defragmentation
   record.lpp    ──►  record codec    serial types 0–9, 12+
   varint.lpp    ──►  varints         big-endian base-128, 1–9 bytes
   pager.lpp     ──►  pages+freelist  100-byte header, page alloc/free
   value.lpp     ──►  dynamic values  NULL/INTEGER/REAL/TEXT/BLOB
   ieee754.lpp   ──►  binary64        bit-exact double ⇄ 8-byte pattern
   mem.lpp       ──►  buffers         growable byte/i64 vectors
```

**On-disk layout produced** (identical to SQLite's documented format):
100-byte file header · page-size-aligned pages · table B+trees keyed by rowid
(`0x0d` leaf / `0x05` interior) · length-prefixed records with per-column serial
types · overflow chains for large payloads · freelist trunk/leaf pages ·
`sqlite_schema` on page 1.

---

## What is implemented

### Statements
`SELECT` · `INSERT` (multi-row `VALUES`, `INSERT … SELECT`, explicit column
lists) · `UPDATE` · `DELETE` · `CREATE TABLE` (`IF NOT EXISTS`) ·
`DROP TABLE` (`IF EXISTS`) · `BEGIN` / `COMMIT` / `ROLLBACK` (accepted; see
limitations) · `PRAGMA table_info | table_list | integrity_check | page_size |
page_count | encoding`.

### Query features
- `WHERE`, `GROUP BY`, `HAVING`, `ORDER BY` (multi-key, `ASC`/`DESC`, ordinals,
  aliases, and expressions over columns not in the result list), `LIMIT`/`OFFSET`
  (both `LIMIT n OFFSET m` and `LIMIT m, n`), `DISTINCT`
- Joins: `INNER`/`JOIN`, `LEFT [OUTER] JOIN`, `CROSS JOIN`, comma joins, table
  aliases, qualified `tbl.col` and `tbl.*`
- Compound queries: `UNION`, `UNION ALL`, `EXCEPT`, `INTERSECT`
- Subqueries: scalar `(SELECT …)`, `IN (SELECT …)`, `NOT IN (SELECT …)`,
  `EXISTS` / `NOT EXISTS` — **including correlated** subqueries that reference
  columns of the outer row
- Predicates: `=` `==` `!=` `<>` `<` `<=` `>` `>=`, `AND`/`OR`/`NOT`,
  `IS [NOT] NULL`, `IS [NOT]`, `IN (list)`, `BETWEEN`, `LIKE` (with `ESCAPE`),
  `GLOB` (incl. `[a-z]` classes), `ISNULL`/`NOTNULL`
- Operators: `+ - * / %`, `||`, `& | << >>`, unary `-` and `~`, `CASE`
  (both simple and searched), `CAST(x AS type)`
- Aggregates: `COUNT(*)`, `COUNT(expr)`, `COUNT(DISTINCT …)`, `SUM`, `TOTAL`,
  `AVG`, `MIN`, `MAX`, `GROUP_CONCAT`

### Scalar functions
`abs` `length` `lower` `upper` `substr`/`substring` `trim` `ltrim` `rtrim`
`replace` `instr` `coalesce` `ifnull` `nullif` `typeof` `round` `min` `max`
`hex` `quote` `char` `unicode` `printf`/`format` `iif` `sign` `like`
`zeroblob` `random` `sqlite_version`.

### Transactions
`BEGIN` snapshots the database image; `COMMIT` writes through and drops the
snapshot; **`ROLLBACK` genuinely restores it**, undoing inserts, updates,
deletes and even `CREATE`/`DROP TABLE`. This is a shadow-copy scheme, not
SQLite's rollback journal — correct in-process, but not crash-safe.

### Concurrency
Opening a file-backed database takes an advisory lock (a lock *directory*
beside the file, created with `mkdir`, which is atomic on POSIX and Windows).
A second writer waits ~2 s and then fails with
`database is locked by another lppsqlite process` instead of corrupting the
file. Set `LPPSQLITE_NO_LOCK=1` to disable.

> Measured: three processes each inserting 200 rows concurrently produce
> 600/600 rows with locking, and **382/600 without** it.

### Indexes
`CREATE INDEX` / `DROP INDEX` build **real SQLite index B-trees** — page type
`0x0a` leaves under a `0x02` interior root, keyed by a record of the indexed
columns followed by the rowid. The real `sqlite3` reads them, reports
`integrity_check` = `ok`, and its planner picks them up:

```
sqlite> EXPLAIN QUERY PLAN SELECT id FROM t WHERE name='cy';
SEARCH t USING COVERING INDEX idx_name (name=?)
```

The engine's own planner uses an index for equality on its **leading column**,
and indexes are rebuilt automatically after `INSERT`/`UPDATE`/`DELETE`.
Indexes created by real SQLite are used for lookups here too.

### Query performance
Equality on the rowid — `WHERE id = 42`, `WHERE rowid = 42`, or an
`INTEGER PRIMARY KEY` column, optionally `AND`-ed with other predicates —
seeks through the B-tree instead of scanning. On a 20,000-row table a point
lookup takes ~6 ms versus ~67 ms for a non-key column: **~11× faster**.

Indexed columns take a similar path: on a 3,000-row table an indexed equality
lookup is ~7 ms versus ~16 ms for an unindexed column. Anything without a
usable index is still a linear scan.

### Storage semantics
- All five storage classes: NULL, INTEGER (1/2/3/4/6/8-byte + the 0/1 literal
  serial types), REAL (IEEE-754 binary64), TEXT (UTF-8), BLOB
- Column **type affinity** (INTEGER/TEXT/BLOB/REAL/NUMERIC) applied on write,
  following SQLite's substring rules
- `INTEGER PRIMARY KEY` as a true rowid alias; `rowid`/`_rowid_`/`oid`
- SQLite's comparison ordering (NULL < numbers < text < blob) with numeric
  affinity applied when comparing a number against numeric-looking text
- Three-valued logic throughout `WHERE`/`AND`/`OR`
- Overflow pages for payloads larger than a page (verified at 9.6 KB)
- Page splits, page defragmentation, empty-leaf pruning, and a real freelist
  with page reuse

---

## What is *not* implemented

These are **rejected with a clear error**, never silently mis-executed:

`WITH` / CTEs · `ALTER TABLE` · `SAVEPOINT` / `RELEASE` · `ATTACH` / `DETACH` ·
`VACUUM` · `ANALYZE` · `NATURAL JOIN`.

Also absent:

- **Index-aware planning beyond equality** — an index is only consulted for
  equality on its leading column. Range predicates (`>`, `BETWEEN`), `ORDER BY`
  and `LIKE 'prefix%'` do not yet use one, and `UNIQUE` is not enforced.
- **Incremental index maintenance** — after a mutation the affected indexes are
  rebuilt from the table rather than patched entry by entry. Correct, but
  O(rows) per writing statement, so bulk loads are best done before
  `CREATE INDEX` (or inside one transaction).
- **Views, triggers, virtual tables, FTS, R-Tree, JSON1, window functions,
  date/time functions, RIGHT/FULL OUTER JOIN, `UPSERT` (`ON CONFLICT DO …`),
  generated columns, `WITHOUT ROWID` tables, foreign-key enforcement,
  `CHECK`/`NOT NULL`/`UNIQUE` constraint enforcement, collations other than
  BINARY, `AUTOINCREMENT` semantics.**
- **Crash-safe transactions** — `ROLLBACK` works in-process (see above), but the
  snapshot lives in memory: there is no journal or WAL, so a process killed
  mid-`COMMIT` can still leave a partially written file. Nested transactions
  and `SAVEPOINT` are not supported.
- **Multi-process reader concurrency** — the advisory lock is exclusive, so
  concurrent *readers* also serialise. It is cooperative and does not
  coordinate with the real `sqlite3`.
- **WAL mode**, incremental vacuum, and `PRAGMA` beyond the handful listed above.

---

## Known differences from SQLite

1. **Correlated subqueries are re-evaluated per candidate row.** They are
   correct, but a correlated subquery over an N-row outer query costs N inner
   queries; there is no caching or decorrelation.
2. **`ROLLBACK` is in-memory only** — correct while the process lives, but not
   crash-safe.
3. **Deleted pages are recycled through the freelist**, but the database file is
   never truncated, so a file can stay larger than `sqlite3` would leave it.
4. **`PRAGMA integrity_check`** always reports `ok` from this engine; use the
   real `sqlite3` for genuine structural verification.
5. **REAL formatting** uses 15 significant digits (`%!.15g`-style). This matches
   SQLite for every tested value but is an independent implementation.
6. **No query planner** — joins are nested-loop over fully materialised tables,
   so result *ordering* without `ORDER BY` may differ from SQLite, and large
   joins are memory-hungry.

---

## Testing

```sh
./run-tests.sh            # everything
./run-tests.sh --unit     # unit suites only
./run-tests.sh --diff     # differential suite only
```

| Suite | Checks | Covers |
|---|---:|---|
| `t_mem` | 35 | buffers, growable vectors, big-endian codecs, sign extension |
| `t_varint` | 101 | varint round-trips 1–9 bytes + exact spec byte patterns |
| `t_ieee` | 41 | bit-exact double ⇄ pattern, formatting, parsing |
| `t_record` | 46 | serial types, record encode/decode, value comparison |
| `t_btree` | 21 | 500-row splits, ordered scan, point lookup, delete, 9 KB overflow |
| `t_lexer` | 50 | keywords, quoting styles, blobs, params, comments, operators |
| `t_schema` | 42 | affinity rules, `CREATE TABLE` parsing, catalogue round-trip |
| `t_parser` | 54 | precedence, all statement forms, joins, `CASE`, errors |
| `t_sql` | 48 | end-to-end SQL incl. joins, aggregates, persistence |
| `t_interop` | 19 | reading *and writing* a database authored by real `sqlite3` |
| `t_stress` | 28 | 3,000 rows, mass delete/update, overflow, edge values |
| **`difftest.py`** | **118** | **same SQL through both engines + `integrity_check`** |

Total: **485 in-engine assertions + 118 differential cases**, all passing.
The differential suite covers correlated subqueries, the rowid fast path
(including cases where it must *not* apply, such as `id = 1 OR id = 3`),
`COMMIT`/`ROLLBACK` semantics, and secondary indexes — equality lookups,
duplicate and NULL keys, maintenance across `INSERT`/`UPDATE`/`DELETE`,
`DROP INDEX`, rollback, and an 800-row multi-page index.

---

## Performance

Measured in the dev container (2 vCPU), engine compiled with the host linker:

| Workload | Result |
|---|---|
| 3,000 single-row `INSERT` statements (each parsed, executed and flushed) | ~1.0 s |
| Point lookup by rowid, 20,000-row table | **~6 ms** |
| Equality on a non-indexed column, same table | ~67 ms |
| Indexed equality lookup, 3,000-row table | **~7 ms** |
| Same query without an index | ~16 ms |
| Full scan + aggregate over 2,000 rows | milliseconds |
| 3,000-row database file | 48 pages / 192 KB |

Each statement outside a transaction re-writes the whole database image, which
dominates insert cost — wrapping a batch in `BEGIN … COMMIT` avoids that, since
writes are held until commit. Note that indexes are rebuilt after each writing
statement, so bulk-load first and `CREATE INDEX` afterwards.

---

## Notes on writing this in L++

Building this surfaced two genuine **miscompilation bugs** in L++ v4.3.0. Both
have been **fixed in the compiler** (`~/lplusplus`, see `FIXES.md` there), so
this codebase no longer contains workarounds for them:

1. **Copy propagation destroyed live variables.** `mut y := x` silently
   corrupted `x` when `x` was an immutable local initialised from a call —
   `pass_copyprop` folded the call's destination into `y` and never assigned
   `x`. This engine originally carried **110** hand-written
   `mut x := 0; x = expr` workarounds; after the compiler fix all 110 were
   reverted to natural `mut x := expr` and the full suite still passes, which
   is the strongest evidence the fix is correct.
2. **Two modules could define the same name silently.** Imported declarations
   are flattened into one global namespace, so `a.shared()` and `b.shared()`
   both called whichever was linked last. Now a clear
   `duplicate definition of 'shared'` compile error.

A third issue reported in an earlier draft — a "loop-exit sentinel miscompile"
— **was not a compiler bug**. `j = n + 1000` followed by `if j > n: j = j - 1000`
restores `j` to exactly `n`; the generated code was correct and my logic was
wrong. That claim is withdrawn.

Remaining real constraints of the language (not bugs) that still shape the
design: lists cannot be nested, structs cannot be stored in lists, and a list
created inside a function cannot be returned as a handle. Consequently **every
data structure here is built on raw byte buffers** with explicit handle
indirection — which, for a byte-oriented file format, turned out to be a good
fit anyway.

`from` and `fn` are reserved words; there is no global mutable state, no
`argv`, no exponent float literals (`1.0e308`), and comparisons are typed `i8`
so they cannot be passed to `Int` parameters.

## Licence

Same terms as the surrounding L++ repository.
