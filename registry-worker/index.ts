/**
 * L++ Official Package Registry API.
 *
 * registry.lplusplus.bond is a read-only HTTP mirror over the canonical git
 * registry. Durable writes happen through `keel publish` (git commit + push),
 * never through Cloudflare Worker memory. The Worker keeps the old public read
 * routes for the website and CLI discovery, and returns explicit guidance for
 * retired token/publish routes.
 */

export interface Env {
  DOMAIN?: string;
  REGISTRY_URL?: string;
  STATIC_INDEX_URL?: string;
  BLOB_BASE_URL?: string;
  GITHUB_REPO?: string;
  GITHUB_TOKEN?: string;
}

export interface RegistryPackage {
  name: string;
  version?: string;
  description?: string;
  authors?: string[];
  license?: string;
  repository?: string;
  git?: string;
  path?: string;
  source?: string;
  source_url?: string;
  dependencies?: string[];
  keywords?: string[];
  features?: string[] | Record<string, string[]>;
  sha256?: string;
  checksum?: string;
  size?: number;
  owner?: string;
  owner_email?: string;
  organization?: string;
  downloads?: number;
  published_at?: string;
  updated_at?: string;
  yanked?: boolean;
  deprecated?: string | boolean;
  versions?: Record<
    string,
    {
      version?: string;
      description?: string;
      sha256?: string;
      checksum?: string;
      size?: number;
      download_url?: string;
      published_at?: string;
      yanked?: boolean;
      deprecated?: string | boolean;
    }
  >;
  [key: string]: unknown;
}

interface RegistryManifest {
  registry?: Record<string, unknown>;
  packages?: Record<string, RegistryPackage> | RegistryPackage[];
  results?: RegistryPackage[];
  [key: string]: unknown;
}

const DEFAULT_DOMAIN = "lplusplus.bond";
const DEFAULT_REGISTRY_URL = "https://registry.lplusplus.bond";
const DEFAULT_STATIC_INDEX_URL = "https://lplusplus.bond/registry/index.json";
const DEFAULT_GITHUB_REPO = "samarnever-droid/llppregistry";
const INDEX_CACHE_TTL_MS = 60 * 1000;
const RELEASE_CACHE_TTL_MS = 5 * 60 * 1000;

const FALLBACK_INDEX: RegistryManifest = {
  "packages": {
    "c2lpp": {
      "authors": [
        "L++ Project"
      ],
      "dependencies": [],
      "description": "Pure-L++ JSON-configured C audit/IR translator with checked pointer, layout, globals and CFG foundations",
      "features": [
        "Strict versioned JSON project configuration",
        "Generate L++ extern bindings from C headers",
        "Whole-input and multi-file dependency audit with provenance",
        "Typed normalized-IR scalar C-to-pure-L++ translation",
        "Strict no-binding native profiles with fail-closed pure-L++ output",
        "Curated standalone pure-L++ SQLite backend with CRUD and integrity validation",
        "General arbitrary-order translation-unit graph with SQLite zero-unknown partition gate",
        "SQLite-wide base-type, body, call, control-target and ownership graphs",
        "Cross-pass denominator and ownership-site consistency validation",
        "Automatic typed semantic sweep emitting accepted pure-L++ SQLite functions",
        "Referenced immutable integer-array globals lowered to checked pure-L++ accessors",
        "Parser-integrated scalar pointer places, arithmetic, typedef casts and CPtr null semantics",
        "Lazy conditional returns, locals and assignments with pointer short-circuit logic",
        "C character literals and explicit compile-time sizeof lowering",
        "Sequenced comma expression statements",
        "Braced do/while plus unbraced typed if/else call lowering",
        "Demand-bounded SysV aggregate member, bitfield and fixed-array place lowering",
        "ABI-width aggregate data-pointer fields with raw provenance side tables",
        "Pointer-depth-two places and one-dimensional array-parameter decay",
        "Profile-v2 forward/callback typedefs, const arrays, nested aggregates, provenance and CFG",
        "Checked C pointer/allocation/place, SysV layout, globals and CFG lowering foundations",
        "Native equivalence and ASan/UBSan compatibility gates",
        "SQLite and zlib real-system-header coverage"
      ],
      "keywords": [
        "c",
        "ffi",
        "bindings",
        "generator",
        "translator",
        "sqlite",
        "zlib"
      ],
      "license": "MIT",
      "name": "c2lpp",
      "path": "packages/c2lpp",
      "repository": "https://github.com/samarnever-droid/lplusplus",
      "source": "packages/c2lpp/src/main.lpp",
      "version": "0.36.0"
    },
    "compresslpp": {
      "api": {
        "deflate.deflate": "(data: Int, n: Int, level: Int) -> Int — compress to a byte vector",
        "gzip.gz_compress": "(data, n, level) -> Int — gzip member",
        "gzip.gz_decompress": "(ptr, size) -> Int — 0 on bad header or CRC",
        "inflate.inflate": "(ptr: Int, size: Int) -> Int — decompress, 0 on malformed input",
        "tar.tr_open": "(path: Str) -> Int — open a tar",
        "tar.tw_new": "() -> Int — new tar writer",
        "zip.zr_extract": "(r, i, password) -> Int — extract entry, 0 on bad password/CRC",
        "zip.zr_open": "(path: Str) -> Int — open an archive, 0 if unreadable",
        "zip.zw_add_file": "(z, name, path, level, password) -> Int — add a file",
        "zip.zw_new": "() -> Int — new archive writer",
        "zip.zw_save": "(z, path) -> Int — write the archive"
      },
      "authors": [
        "arena-audit"
      ],
      "dependencies": [],
      "description": "Archive and compression library in pure L++ — real DEFLATE (RFC 1951), ZIP with ZipCrypto passwords, USTAR tar and gzip, verified against zlib/zipfile/tarfile.",
      "features": [
        "Real DEFLATE (RFC 1951): stored, fixed and dynamic Huffman decoding; LZ77 hash-chain encoder",
        "zlib inflates every stream we write, and we reproduce zlib's output byte-for-byte",
        "ZIP read/write with STORE and DEFLATE, nested paths and CRC-32 verification",
        "Password-protected ZIP (ZipCrypto): interoperates with `zip -P` and zipfile.setpassword()",
        "USTAR tar read/write, validated by python tarfile and GNU tar",
        "gzip (RFC 1952) read/write, validated by python gzip",
        "Command-line tool: zip, unzip, list, tar, untar, gzip, gunzip",
        "50 in-engine checks plus 32 cross-verifications against python and the unzip/tar CLIs"
      ],
      "keywords": [
        "zip",
        "tar",
        "gzip",
        "deflate",
        "compression",
        "archive",
        "password",
        "zipcrypto"
      ],
      "license": "MIT",
      "limitations": [
        "Encoder emits fixed-Huffman blocks only; output is a few percent larger than zlib",
        "ZipCrypto is cryptographically weak — interoperability only, not real confidentiality",
        "No AES/AE-x encryption, no ZIP64, no bzip2/LZMA/XZ/Zstandard",
        "Everything is processed in memory; not suitable for files larger than RAM",
        "TAR long names (>100 bytes) are rejected; symlinks, sparse files and PAX metadata ignored",
        "Permissions, ownership and timestamps are not preserved"
      ],
      "name": "compresslpp",
      "path": "packages/compresslpp",
      "repository": "https://github.com/samarnever-droid/lplusplus",
      "source": "packages/compresslpp/src/zip.lpp",
      "version": "1.0.0"
    },
    "lpp-algo": {
      "authors": [
        "0x4171341"
      ],
      "dependencies": [],
      "description": "Algorithm library — bubble_sort, binary_search, list_range, list_fill",
      "keywords": [
        "algorithm",
        "sort",
        "search",
        "stdlib"
      ],
      "license": "MIT",
      "name": "lpp-algo",
      "path": "stdlib/algo.lpp",
      "source": "stdlib/algo.lpp",
      "version": "0.1.0"
    },
    "lpp-analyzer": {
      "authors": [
        "samarnever-droid"
      ],
      "dependencies": [],
      "description": "Static code analysis, AST validation, and escape analysis linter for L++.",
      "keywords": [
        "analysis",
        "ast",
        "linter",
        "compiler"
      ],
      "license": "MIT",
      "name": "lpp-analyzer",
      "path": "packages/lpp-analyzer",
      "repository": "https://github.com/samarnever-droid/lplusplus",
      "source": "packages/lpp-analyzer/src/main.lpp",
      "version": "1.4.0"
    },
    "lpp-bindgen": {
      "authors": [
        "samarnever-droid",
        "L++ Project"
      ],
      "dependencies": [],
      "description": "Automated C and C++ FFI header bindings generator & C-to-L++ translator with safe checked pointers (CPtr)",
      "keywords": [
        "ffi",
        "c",
        "bindgen",
        "bindings",
        "translator"
      ],
      "license": "MIT",
      "name": "lpp-bindgen",
      "path": "packages/lpp-bindgen",
      "repository": "https://github.com/samarnever-droid/lplusplus",
      "source": "packages/lpp-bindgen/src/main.lpp",
      "version": "0.36.0"
    },
    "lpp-collections": {
      "authors": [
        "0x4171341"
      ],
      "dependencies": [],
      "description": "Collection utilities — list_sum, list_max, list_min, list_reverse",
      "keywords": [
        "list",
        "collections",
        "stdlib"
      ],
      "license": "MIT",
      "name": "lpp-collections",
      "path": "stdlib/collections.lpp",
      "source": "stdlib/collections.lpp",
      "version": "0.1.0"
    },
    "lpp-engine": {
      "authors": [
        "samarnever-droid"
      ],
      "dependencies": [],
      "description": "High-throughput asynchronous event loop and task runner for L++ services.",
      "keywords": [
        "async",
        "runtime",
        "event-loop"
      ],
      "license": "Apache-2.0",
      "name": "lpp-engine",
      "path": "packages/lpp-engine",
      "repository": "https://github.com/samarnever-droid/lplusplus",
      "source": "packages/lpp-engine/src/main.lpp",
      "version": "2.0.0"
    },
    "lpp-lreact": {
      "authors": [
        "samarnever-droid"
      ],
      "branch": "main",
      "dependencies": [],
      "description": "Alias for lreact — Tauri-like GUI framework for L++",
      "git": "https://github.com/samarnever-droid/lreact.git",
      "keywords": [
        "lreact",
        "react",
        "tauri",
        "gui"
      ],
      "license": "MIT",
      "name": "lpp-lreact",
      "path": "src/lreact.lpp",
      "repository": "https://github.com/samarnever-droid/lreact",
      "source_url": "https://raw.githubusercontent.com/samarnever-droid/lreact/main/src/lreact.lpp",
      "version": "1.0.0"
    },
    "lpp-math": {
      "authors": [
        "0x4171341"
      ],
      "dependencies": [],
      "description": "Math utilities — abs, min, max, pow, gcd, lcm, fib, factorial",
      "keywords": [
        "math",
        "stdlib"
      ],
      "license": "MIT",
      "name": "lpp-math",
      "path": "stdlib/math.lpp",
      "source": "stdlib/math.lpp",
      "version": "0.1.0"
    },
    "lpp-openclaude": {
      "authors": [
        "0x4171341"
      ],
      "dependencies": [
        "lpp-tui"
      ],
      "description": "OpenClaude alias for lpp-opencode — Claude-oriented OpenCode port foundation in L++",
      "features": [
        "Large terminal logo and command-hint screen",
        "Interactive OpenCode-style prompt loop when run without arguments",
        "Launcher compiles silently and runs the produced executable",
        "Windows direct runtime compatible after L++ v3.3.1 CreateFileA import fix",
        "Direct-link-safe default command entrypoint for Windows (file tools disabled until host tool mode)",
        "Immediate current-session PowerShell PATH fix for lpp-opencode command",
        "Installable lpp-opencode command on Linux, macOS, and Windows",
        "Unix shell launcher and Windows PowerShell/CMD launchers",
        "Windows PowerShell installer and runner",
        "Windows CI check and AOT object generation",
        "Claude/Anthropic provider router",
        "Anthropic Messages API payload builder",
        "OpenCode-style terminal coding agent scaffold",
        "Slash commands: /help, /provider, /model, /payload, /read, /save, /exit",
        "Transcript save/append helpers",
        "ANSI TUI renderer with line screen buffer"
      ],
      "keywords": [
        "opencode",
        "coding-agent",
        "tui",
        "terminal",
        "ai",
        "llm",
        "openclaude",
        "claude",
        "anthropic"
      ],
      "license": "MIT",
      "name": "lpp-openclaude",
      "path": "src/main.lpp",
      "provides": "lpp-opencode",
      "repository": "https://github.com/samarnever-droid/lpp-opencode",
      "source_url": "https://raw.githubusercontent.com/samarnever-droid/lpp-opencode/main/src/main.lpp",
      "version": "0.6.0"
    },
    "lpp-opencode": {
      "authors": [
        "0x4171341"
      ],
      "dependencies": [
        "lpp-tui"
      ],
      "description": "OpenCode/OpenClaude port foundation in L++ — installable lpp-opencode command, Claude router, TUI, sessions, commands, tools, and agent scaffold",
      "features": [
        "Large terminal logo and command-hint screen",
        "Interactive OpenCode-style prompt loop when run without arguments",
        "Launcher compiles silently and runs the produced executable",
        "Windows direct runtime compatible after L++ v3.3.1 CreateFileA import fix",
        "Direct-link-safe default command entrypoint for Windows (file tools disabled until host tool mode)",
        "Immediate current-session PowerShell PATH fix for lpp-opencode command",
        "Installable lpp-opencode command on Linux, macOS, and Windows",
        "Unix shell launcher and Windows PowerShell/CMD launchers",
        "Windows PowerShell installer and runner",
        "Windows CI check and AOT object generation",
        "Claude/Anthropic provider router",
        "Anthropic Messages API payload builder",
        "OpenCode-style terminal coding agent scaffold",
        "Slash commands: /help, /provider, /model, /payload, /read, /save, /exit",
        "Transcript save/append helpers",
        "ANSI TUI renderer with line screen buffer"
      ],
      "keywords": [
        "opencode",
        "coding-agent",
        "tui",
        "terminal",
        "ai",
        "llm"
      ],
      "license": "MIT",
      "name": "lpp-opencode",
      "path": "src/main.lpp",
      "repository": "https://github.com/samarnever-droid/lpp-opencode",
      "source_url": "https://raw.githubusercontent.com/samarnever-droid/lpp-opencode/main/src/main.lpp",
      "version": "0.6.0"
    },
    "lpp-strings": {
      "authors": [
        "0x4171341"
      ],
      "dependencies": [],
      "description": "String utilities — repeat, contains, starts_with, reverse, pad",
      "keywords": [
        "string",
        "text",
        "stdlib"
      ],
      "license": "MIT",
      "name": "lpp-strings",
      "path": "stdlib/strings.lpp",
      "source": "stdlib/strings.lpp",
      "version": "0.1.0"
    },
    "lpp-tui": {
      "authors": [
        "0x4171341"
      ],
      "dependencies": [],
      "description": "Reusable ANSI terminal UI helpers extracted from the L++ OpenCode port",
      "features": [
        "ANSI colors and cursor helpers",
        "Screen clearing and alternate screen helpers",
        "Simple frame/prompt/assistant rendering",
        "Line-buffer screen renderer",
        "Minimal key classifier for future raw input loop"
      ],
      "keywords": [
        "tui",
        "ansi",
        "terminal",
        "opencode",
        "ui"
      ],
      "license": "MIT",
      "name": "lpp-tui",
      "path": "src/tui",
      "repository": "https://github.com/samarnever-droid/lpp-opencode",
      "source_url": "https://raw.githubusercontent.com/samarnever-droid/lpp-opencode/main/src/tui/ansi.lpp",
      "version": "0.2.0"
    },
    "lpp-zip": {
      "api": {
        "zip_add_file": "(archive: Int, filename: Str, content: Str) — add file to archive",
        "zip_close": "(handle: Int) — close ZIP handle",
        "zip_create": "() -> Int — create new archive handle",
        "zip_entry_count": "(handle: Int) -> Int — number of entries",
        "zip_entry_data": "(handle: Int, index: Int) -> Str — get entry content",
        "zip_entry_name": "(handle: Int, index: Int) -> Str — get entry filename",
        "zip_free": "(archive: Int) — release archive memory",
        "zip_open": "(path: Str) -> Int — open ZIP file for reading",
        "zip_save": "(archive: Int, path: Str) — write ZIP to disk"
      },
      "authors": [
        "0x4171341"
      ],
      "dependencies": [],
      "description": "ZIP archive create/read library — pure L++ using buf_* primitives",
      "features": [
        "Create ZIP archives with multiple files",
        "Read ZIP archives and extract entries",
        "CRC32 verification",
        "STORE method (no compression)",
        "Pure L++ — no C code, no linker changes"
      ],
      "keywords": [
        "zip",
        "archive",
        "binary",
        "file",
        "compression"
      ],
      "license": "MIT",
      "name": "lpp-zip",
      "path": "packages/lpp-zip",
      "repository": "https://github.com/samarnever-droid/lplusplus",
      "source": "packages/lpp-zip/src/zip.lpp",
      "version": "0.1.0"
    },
    "lppdb": {
      "api": {
        "db_close": "(handle: Int) - release memory",
        "db_exec": "(handle: Int, sql: Str) -> Str - run a statement, returns JSON",
        "db_open": "(path: Str) -> Int - open or create a database file",
        "db_save": "(handle: Int, path: Str) - flush to disk"
      },
      "authors": [
        "arena-audit"
      ],
      "dependencies": [],
      "description": "A real embedded SQL database engine in pure L++: binary page storage, typed cells, real parser/executor, disk persistence. Replaces the former non-functional sqlite stub.",
      "features": [
        "Real binary page-oriented storage persisted to disk (buf_read/buf_write)",
        "Typed cells: INTEGER (8-byte LE), TEXT (length-prefixed), NULL",
        "Real SQL tokenizer/parser/executor - results computed from stored data",
        "CREATE TABLE, INSERT (multi-row, explicit columns), SELECT (*, columns)",
        "Aggregates COUNT(*)/SUM/MIN/MAX computed over matching rows",
        "WHERE (=,!=,<>,<,<=,>,>=, AND), ORDER BY ASC/DESC, LIMIT",
        "UPDATE ... SET ... WHERE, DELETE FROM ... WHERE, DROP TABLE",
        "Persistence verified across close/reopen",
        "Works with --linker host and zero-dependency --linker direct",
        "Pure L++ over buf_* builtins - no C code, no linker changes"
      ],
      "keywords": [
        "database",
        "sql",
        "storage",
        "db",
        "embedded",
        "binary"
      ],
      "license": "MIT",
      "limitations": [
        "Single-table queries only (no JOIN/subquery/CTE)",
        "No FTS, views, triggers, indexes, or transactions",
        "No REAL/floating-point type (stored as TEXT)",
        "Linear row scan (no B-tree); not for very large tables",
        "Own binary format - NOT SQLite file-format compatible"
      ],
      "name": "lppdb",
      "path": "packages/lppdb/src/lppdb.lpp",
      "repository": "https://github.com/samarnever-droid/lplusplus",
      "source": "packages/lppdb/src/lppdb.lpp",
      "version": "1.0.0"
    },
    "lppsqlite": {
      "api": {
        "exec.close_db": "(db: Int) — flush to disk and release",
        "exec.exec": "(db: Int, sql: Str) -> Int — run one statement, returns a rowset",
        "exec.exec_script": "(db: Int, sql: Str) -> Int — run several ';'-separated statements",
        "exec.open_db": "(path: Str) -> Int — open or create a database file",
        "exec.open_memory": "() -> Int — transient in-memory database",
        "rowset.rs_cell": "(r: Int, row: Int, col: Int) -> Int — value handle",
        "rowset.rs_changes": "(r: Int) -> Int — rows modified by INSERT/UPDATE/DELETE",
        "rowset.rs_err": "(r: Int) -> Int — 1 on error",
        "rowset.rs_errmsg": "(r: Int) -> Str — error text",
        "rowset.rs_name": "(r: Int, i: Int) -> Str — column name",
        "rowset.rs_ncols": "(r: Int) -> Int — column count",
        "rowset.rs_nrows": "(r: Int) -> Int — row count",
        "value.v_display": "(v: Int) -> Str — render a value for output",
        "value.v_typename": "(v: Int) -> Str — null|integer|real|text|blob"
      },
      "authors": [
        "arena-audit"
      ],
      "dependencies": [],
      "description": "SQLite-file-format-compatible database engine in pure L++ — real .db files, B+trees, secondary indexes, overflow pages, freelist, correlated subqueries, real transactions and advisory locking.",
      "features": [
        "Real SQLite on-disk format: 100-byte header, page-aligned B+trees, varints, record serial types",
        "Files pass the real sqlite3's PRAGMA integrity_check (verified in CI)",
        "Reads AND writes databases created by real SQLite, including REAL/BLOB/NULL and overflow rows",
        "Overflow page chains for payloads larger than one page",
        "Freelist with trunk/leaf pages and page reuse; empty-leaf pruning on delete",
        "SELECT with WHERE, GROUP BY, HAVING, ORDER BY, LIMIT/OFFSET, DISTINCT",
        "INNER/LEFT/CROSS joins, table aliases, qualified columns",
        "UNION, UNION ALL, EXCEPT, INTERSECT",
        "Subqueries: scalar (SELECT ...), IN (SELECT ...), EXISTS / NOT EXISTS",
        "Aggregates COUNT/SUM/TOTAL/AVG/MIN/MAX/GROUP_CONCAT with DISTINCT",
        "~30 scalar functions (substr, replace, instr, printf, hex, round, typeof, iif, ...)",
        "SQLite type affinity, three-valued logic, and SQLite's NULL<num<text<blob ordering",
        "INTEGER PRIMARY KEY rowid aliasing; rowid/_rowid_/oid",
        "CLI shell with list/CSV/JSON output and .tables/.schema/.dump dot-commands",
        "Correlated subqueries: inner SELECT resolves columns from the outer row",
        "Real transactions: BEGIN/COMMIT/ROLLBACK, where ROLLBACK genuinely undoes DML and DDL",
        "Advisory cross-process file locking so concurrent writers cannot corrupt the database",
        "Rowid fast path: WHERE id = ? seeks the B-tree instead of scanning (~11x on 20k rows)",
        "Secondary indexes: CREATE INDEX / DROP INDEX build real SQLite index b-trees (0x0a leaves, 0x02 interior root)",
        "Real sqlite3 reads those indexes and its planner reports SEARCH ... USING COVERING INDEX",
        "Indexes created by real SQLite are used for lookups here, and stay valid after this engine writes",
        "Index-driven equality lookups on the leading column; indexes rebuilt after INSERT/UPDATE/DELETE",
        "485 in-engine assertions plus 118 differential cases compared against real sqlite3"
      ],
      "keywords": [
        "sqlite",
        "database",
        "sql",
        "btree",
        "storage",
        "embedded",
        "file-format",
        "compatible"
      ],
      "license": "MIT",
      "limitations": [
        "Indexes are consulted only for equality on the leading column; ranges, ORDER BY and LIKE still scan",
        "UNIQUE indexes are parsed but uniqueness is not enforced",
        "Index maintenance rebuilds the index after each writing statement (O(rows)); bulk-load before CREATE INDEX",
        "ROLLBACK is in-memory (shadow copy), so it is not crash-safe; no journal or WAL",
        "The advisory lock is exclusive, so concurrent readers serialise too",
        "No views, triggers, window functions, date/time functions, or ALTER TABLE",
        "WITH/ALTER/SAVEPOINT/ATTACH/VACUUM/NATURAL JOIN are rejected with a clear error"
      ],
      "name": "lppsqlite",
      "path": "packages/lppsqlite",
      "repository": "https://github.com/samarnever-droid/lplusplus",
      "source": "packages/lppsqlite/src/exec.lpp",
      "version": "1.2.0"
    },
    "lppstore": {
      "authors": [
        "samarnever-droid"
      ],
      "dependencies": [],
      "description": "Persistent B-tree key-value store with atomic transactions.",
      "keywords": [
        "kv",
        "btree",
        "database"
      ],
      "license": "MIT",
      "name": "lppstore",
      "path": "packages/lppstore",
      "repository": "https://github.com/samarnever-droid/lplusplus",
      "source": "packages/lppstore/src/main.lpp",
      "version": "0.6.0"
    },
    "lreact": {
      "authors": [
        "samarnever-droid"
      ],
      "branch": "main",
      "dependencies": [],
      "description": "Tauri-like GUI framework for L++ using React/HTML frontend + native L++ HTTP IPC backend",
      "git": "https://github.com/samarnever-droid/lreact.git",
      "keywords": [
        "lreact",
        "react",
        "tauri",
        "gui",
        "desktop",
        "ipc",
        "frontend"
      ],
      "license": "MIT",
      "name": "lreact",
      "path": "src/lreact.lpp",
      "repository": "https://github.com/samarnever-droid/lreact",
      "source_url": "https://raw.githubusercontent.com/samarnever-droid/lreact/main/src/lreact.lpp",
      "version": "1.0.0"
    },
    "openclaude": {
      "authors": [
        "0x4171341"
      ],
      "dependencies": [
        "lpp-tui"
      ],
      "description": "OpenClaude short alias for lpp-opencode",
      "features": [
        "Large terminal logo and command-hint screen",
        "Interactive OpenCode-style prompt loop when run without arguments",
        "Launcher compiles silently and runs the produced executable",
        "Windows direct runtime compatible after L++ v3.3.1 CreateFileA import fix",
        "Direct-link-safe default command entrypoint for Windows (file tools disabled until host tool mode)",
        "Immediate current-session PowerShell PATH fix for lpp-opencode command",
        "Installable lpp-opencode command on Linux, macOS, and Windows",
        "Unix shell launcher and Windows PowerShell/CMD launchers",
        "Windows PowerShell installer and runner",
        "Windows CI check and AOT object generation",
        "Claude/Anthropic provider router",
        "Anthropic Messages API payload builder",
        "OpenCode-style terminal coding agent scaffold",
        "Slash commands: /help, /provider, /model, /payload, /read, /save, /exit",
        "Transcript save/append helpers",
        "ANSI TUI renderer with line screen buffer"
      ],
      "keywords": [
        "opencode",
        "coding-agent",
        "tui",
        "terminal",
        "ai",
        "llm",
        "openclaude",
        "claude",
        "anthropic"
      ],
      "license": "MIT",
      "name": "openclaude",
      "path": "src/main.lpp",
      "provides": "lpp-opencode",
      "repository": "https://github.com/samarnever-droid/lpp-opencode",
      "source_url": "https://raw.githubusercontent.com/samarnever-droid/lpp-opencode/main/src/main.lpp",
      "version": "0.6.0"
    }
  },
  "registry": {
    "description": "Official L++ package registry — git-backed, static-mirrored, SHA-256 verified",
    "name": "L++ Official Package Registry",
    "package_count": 19,
    "source_of_truth": "git",
    "static_index_url": "https://lplusplus.bond/registry/index.json",
    "url": "https://registry.lplusplus.bond",
    "version": "3.0.0"
  }
} as RegistryManifest;

let indexCache: { manifest: RegistryManifest; timestamp: number; source: string } | null = null;
let releasesCache: { data: any[]; timestamp: number } | null = null;

function securityHeaders(): Record<string, string> {
  return {
    "X-Content-Type-Options": "nosniff",
    "X-Frame-Options": "DENY",
    "Referrer-Policy": "strict-origin-when-cross-origin",
    "Strict-Transport-Security": "max-age=31536000; includeSubDomains; preload",
    "Permissions-Policy": "camera=(), microphone=(), geolocation=()",
  };
}

function corsHeaders(): HeadersInit {
  return {
    "Access-Control-Allow-Origin": "*",
    "Access-Control-Allow-Methods": "GET, HEAD, OPTIONS, POST",
    "Access-Control-Allow-Headers": "Content-Type, Authorization, x-api-key, sentry-trace",
    "Access-Control-Max-Age": "86400",
    ...securityHeaders(),
  };
}

function jsonResponse(data: unknown, status = 200, extraHeaders: HeadersInit = {}): Response {
  return new Response(JSON.stringify(data, null, 2), {
    status,
    headers: {
      "Content-Type": "application/json; charset=utf-8",
      ...corsHeaders(),
      ...extraHeaders,
    },
  });
}

function errorResponse(status: number, code: string, message: string, details: Record<string, unknown> = {}): Response {
  return jsonResponse({ error: { code, message, status, ...details } }, status);
}

function registryUrl(env: Env): string {
  return env.REGISTRY_URL || DEFAULT_REGISTRY_URL;
}

function githubRepo(env: Env): string {
  return env.GITHUB_REPO || DEFAULT_GITHUB_REPO;
}

function normalizePackageName(raw: string): string | null {
  const decoded = decodeURIComponent(raw).trim().toLowerCase();
  const nameRegex = /^(?:@[a-z0-9_-]+\/)?[a-z0-9][a-z0-9_-]{0,63}$/;
  if (!nameRegex.test(decoded)) return null;
  if (decoded.includes("..") || decoded.includes("\\") || decoded.includes("%00")) return null;
  return decoded;
}

function normalizePackages(manifest: RegistryManifest): Record<string, RegistryPackage> {
  const raw = manifest.packages ?? manifest.results ?? {};
  if (Array.isArray(raw)) {
    return Object.fromEntries(raw.filter((pkg) => pkg && pkg.name).map((pkg) => [pkg.name, pkg]));
  }
  return raw as Record<string, RegistryPackage>;
}

function packageVersionCount(pkg: RegistryPackage): number {
  if (pkg.versions && Object.keys(pkg.versions).length > 0) return Object.keys(pkg.versions).length;
  return pkg.version ? 1 : 0;
}

function latestVersion(pkg: RegistryPackage): string {
  if (pkg.version) return pkg.version;
  const versions = pkg.versions ? Object.keys(pkg.versions).sort() : [];
  return versions.length > 0 ? versions[versions.length - 1] : "0.0.0";
}

async function loadRegistryManifest(env: Env): Promise<{ manifest: RegistryManifest; source: string }> {
  const now = Date.now();
  if (indexCache && now - indexCache.timestamp < INDEX_CACHE_TTL_MS) {
    return { manifest: indexCache.manifest, source: indexCache.source };
  }

  const staticUrl = env.STATIC_INDEX_URL || DEFAULT_STATIC_INDEX_URL;
  try {
    const res = await fetch(staticUrl, {
      headers: { Accept: "application/json", "User-Agent": "Lplusplus-Registry-Worker" },
      cf: { cacheTtl: 60, cacheEverything: true },
    } as RequestInit & { cf?: unknown });
    if (res.ok) {
      const manifest = (await res.json()) as RegistryManifest;
      if (manifest && (manifest.packages || manifest.results)) {
        indexCache = { manifest, timestamp: now, source: staticUrl };
        return { manifest, source: staticUrl };
      }
    }
  } catch (error) {
    console.warn("registry static index fetch failed; using bundled fallback", error);
  }

  indexCache = { manifest: FALLBACK_INDEX, timestamp: now, source: "bundled-fallback" };
  return { manifest: FALLBACK_INDEX, source: "bundled-fallback" };
}

async function fetchGitHubReleases(env: Env): Promise<any[]> {
  const now = Date.now();
  if (releasesCache && now - releasesCache.timestamp < RELEASE_CACHE_TTL_MS) return releasesCache.data;

  try {
    const headers: Record<string, string> = {
      "User-Agent": "Lplusplus-Registry-Worker",
      Accept: "application/vnd.github.v3+json",
    };
    if (env.GITHUB_TOKEN) headers.Authorization = `Bearer ${env.GITHUB_TOKEN}`;

    const res = await fetch(`https://api.github.com/repos/${githubRepo(env)}/releases`, { headers });
    if (res.ok) {
      const data = (await res.json()) as any[];
      releasesCache = { data, timestamp: now };
      return data;
    }
  } catch (error) {
    console.warn("GitHub release stats unavailable", error);
  }

  return releasesCache?.data || [];
}

async function downloadCounts(env: Env, packageNames: string[]): Promise<{ total: number; byPackage: Map<string, number> }> {
  const releases = await fetchGitHubReleases(env);
  const byPackage = new Map<string, number>();
  let total = 0;

  for (const release of releases) {
    for (const asset of Array.isArray(release.assets) ? release.assets : []) {
      const count = typeof asset.download_count === "number" ? asset.download_count : 0;
      total += count;
      const assetName = String(asset.name || "").toLowerCase();
      for (const name of packageNames) {
        if (assetName.includes(name.toLowerCase())) {
          byPackage.set(name, (byPackage.get(name) || 0) + count);
        }
      }
    }
  }

  return { total, byPackage };
}

async function packagesWithStats(env: Env): Promise<{ packages: Record<string, RegistryPackage>; source: string; totalDownloads: number }> {
  const { manifest, source } = await loadRegistryManifest(env);
  const packages = normalizePackages(manifest);
  const names = Object.keys(packages);
  const counts = await downloadCounts(env, names);
  const enriched = Object.fromEntries(
    Object.entries(packages).map(([name, pkg]) => [
      name,
      { ...pkg, name: pkg.name || name, version: latestVersion(pkg), downloads: counts.byPackage.get(name) || pkg.downloads || 0 },
    ])
  );
  return { packages: enriched, source, totalDownloads: counts.total };
}

async function handleIndex(env: Env): Promise<Response> {
  const { packages, source } = await packagesWithStats(env);
  return jsonResponse(
    {
      registry: {
        name: "L++ Official Package Registry",
        version: "3.0.0",
        url: registryUrl(env),
        domain: env.DOMAIN || DEFAULT_DOMAIN,
        source_of_truth: "git",
        mirror_source: source,
        description: "Read-only HTTP mirror for the git-backed L++ registry.",
        package_count: Object.keys(packages).length,
        updated_at: new Date().toISOString(),
      },
      packages,
    },
    200,
    { "Cache-Control": "public, max-age=60, s-maxage=60" }
  );
}

async function handleStats(env: Env): Promise<Response> {
  const { packages, source, totalDownloads } = await packagesWithStats(env);
  const values = Object.values(packages);
  const publishers = new Set<string>();
  let versions = 0;
  let yanked = 0;
  let deprecated = 0;

  for (const pkg of values) {
    versions += packageVersionCount(pkg);
    if (pkg.owner_email) publishers.add(pkg.owner_email);
    for (const author of pkg.authors || []) publishers.add(author);
    if (pkg.yanked) yanked += 1;
    if (pkg.deprecated) deprecated += 1;
    for (const version of Object.values(pkg.versions || {})) {
      if (version.yanked) yanked += 1;
      if (version.deprecated) deprecated += 1;
    }
  }

  return jsonResponse({
    packages_count: values.length,
    downloads_count: totalDownloads,
    versions_count: versions,
    publishers_count: publishers.size,
    yanked_versions: yanked,
    deprecated_versions: deprecated,
    source_of_truth: "git",
    mirror_source: source,
    updated_at: new Date().toISOString(),
  });
}

async function handleSearch(env: Env, url: URL): Promise<Response> {
  const q = (url.searchParams.get("q") || "").trim().toLowerCase();
  const { packages } = await packagesWithStats(env);
  const results = Object.values(packages)
    .filter((pkg) => {
      if (!q) return true;
      const haystack = [
        pkg.name,
        pkg.description || "",
        ...(pkg.keywords || []),
        ...(pkg.authors || []),
        pkg.license || "",
      ].join(" ").toLowerCase();
      return haystack.includes(q);
    })
    .sort((a, b) => a.name.localeCompare(b.name));

  return jsonResponse({
    query: q,
    count: results.length,
    results: results.map((pkg) => ({
      name: pkg.name,
      version: latestVersion(pkg),
      description: pkg.description || "",
      keywords: pkg.keywords || [],
      authors: pkg.authors || [],
      license: pkg.license,
      downloads: pkg.downloads || 0,
      owner: pkg.owner_email || pkg.owner || "community",
      organization: pkg.organization,
      yanked: pkg.yanked || false,
      deprecated: pkg.deprecated || false,
      download_url: `${registryUrl(env)}/download/${pkg.name}/${latestVersion(pkg)}.tar.gz`,
    })),
  });
}

async function handleGetPackage(env: Env, rawName: string): Promise<Response> {
  const name = normalizePackageName(rawName);
  if (!name) return errorResponse(400, "invalid_name", "Invalid package name format.");

  const { packages } = await packagesWithStats(env);
  const pkg = packages[name];
  if (!pkg) return errorResponse(404, "not_found", `Package '${name}' not found.`);

  return jsonResponse(pkg, 200, { "Cache-Control": "public, max-age=60, s-maxage=60" });
}

function versionMetadata(pkg: RegistryPackage, requested: string): Record<string, unknown> | undefined {
  const clean = requested.replace(/\.tar\.gz$/i, "");
  const version = clean.startsWith(`${pkg.name}-`) ? clean.slice(pkg.name.length + 1) : clean;
  return pkg.versions?.[version] || pkg.versions?.[pkg.version || ""];
}

async function handleDownload(env: Env, rawName: string, requested: string): Promise<Response> {
  const name = normalizePackageName(rawName);
  if (!name) return errorResponse(400, "invalid_name", "Invalid package name.");

  const { packages } = await packagesWithStats(env);
  const pkg = packages[name];
  if (!pkg) return errorResponse(404, "not_found", `Package '${name}' not found.`);

  const metadata = versionMetadata(pkg, requested);
  const direct = typeof metadata?.download_url === "string" ? metadata.download_url : undefined;
  if (direct) return Response.redirect(direct, 302);

  const checksum = String(metadata?.checksum || metadata?.sha256 || pkg.checksum || pkg.sha256 || "");
  if (checksum && /^[a-f0-9]{64}$/i.test(checksum)) {
    const blobBase = env.BLOB_BASE_URL || registryUrl(env);
    return Response.redirect(`${blobBase.replace(/\/$/, "")}/blob/${checksum}`, 302);
  }

  // Compatibility fallback for source-tree packages already listed in the old
  // static registry: send users to the package's source repository when present;
  // otherwise fall back to the configured registry repo.
  const sourceRepo =
    typeof pkg.repository === "string" && pkg.repository.startsWith("https://github.com/")
      ? pkg.repository.replace(/\.git$/, "")
      : `https://github.com/${githubRepo(env)}`;
  const path = String(pkg.path || pkg.source || "").replace(/^\/+/, "");
  if (path) return Response.redirect(`${sourceRepo}/tree/master/${path}`, 302);

  return errorResponse(404, "artifact_unavailable", "This package has metadata but no mirrored artifact yet.");
}

async function handleBlob(env: Env, sha: string): Promise<Response> {
  if (!/^[a-f0-9]{64}$/i.test(sha)) return errorResponse(400, "invalid_checksum", "Blob path must be a SHA-256 hex digest.");
  if (!env.BLOB_BASE_URL || env.BLOB_BASE_URL.replace(/\/$/, "") === registryUrl(env).replace(/\/$/, "")) {
    return errorResponse(404, "blob_mirror_not_configured", "The git registry is authoritative; this HTTP mirror has no blob storage configured.", {
      hint: "Use `keel fetch` against the git registry, or configure BLOB_BASE_URL to a static blob mirror.",
    });
  }
  return Response.redirect(`${env.BLOB_BASE_URL.replace(/\/$/, "")}/blob/${sha}`, 302);
}

function retiredWriteRoute(): Response {
  return errorResponse(410, "git_registry_only", "Browser tokens and Worker-side publishing were removed. Publish through Keel so git remains the durable source of truth.", {
    publish: "KEEL_REGISTRY=git@github.com:samarnever-droid/llppregistry.git keel publish",
    docs: "https://lplusplus.bond/account.html",
  });
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    if (request.method === "OPTIONS") return new Response(null, { status: 204, headers: corsHeaders() });

    const url = new URL(request.url);
    const path = url.pathname.replace(/\/$/, "") || "/";

    try {
      if (path === "/health" || path === "/status") {
        return jsonResponse({
          status: "healthy",
          service: "lplusplus-registry-api",
          version: "4.0.0-readonly-git-mirror",
          domain: env.DOMAIN || DEFAULT_DOMAIN,
          registry_url: registryUrl(env),
          source_of_truth: "git",
          writes: "keel publish (git commit + push)",
          timestamp: new Date().toISOString(),
        });
      }

      if (path === "/stats" || path === "/telemetry") return await handleStats(env);
      if (path === "/search" || path === "/api/v1/search") return await handleSearch(env, url);
      if (path === "/index.json" || path === "/registry/index.json" || path === "/") return await handleIndex(env);

      const pkgMatch = path.match(/^\/packages\/(.+)$/);
      if (pkgMatch && request.method === "GET") return await handleGetPackage(env, pkgMatch[1]);

      const downloadMatch = path.match(/^\/download\/(@[a-zA-Z0-9_-]+\/[a-zA-Z0-9_-]+|[^/]+)\/(.+)$/);
      if (downloadMatch && request.method === "GET") return await handleDownload(env, downloadMatch[1], downloadMatch[2]);

      const blobMatch = path.match(/^\/blob\/([a-fA-F0-9]{64})$/);
      if (blobMatch && request.method === "GET") return await handleBlob(env, blobMatch[1]);

      if (["/tokens", "/api/v1/tokens", "/auth/create-token", "/publish", "/api/v1/publish"].includes(path)) {
        return retiredWriteRoute();
      }

      return errorResponse(404, "route_not_found", `Route '${path}' not found.`);
    } catch (error: any) {
      console.error("Unhandled Worker Error:", error);
      return errorResponse(500, "internal_error", error?.message || "An unexpected error occurred.");
    }
  },
};
