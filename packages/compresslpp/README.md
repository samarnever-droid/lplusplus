# compresslpp

**Archive and compression library written entirely in L++.**

Real DEFLATE — not a stored-only stub. `zlib`, Python's `zipfile`/`tarfile`/
`gzip`, and the `unzip`/`tar` CLIs all read what this produces, and it reads
what they produce, including password-protected ZIPs.

```sh
$ compresslpp zip backup.zip notes.txt data.csv -p s3cret -9
  adding: notes.txt
  adding: data.csv
2 file(s) -> backup.zip

$ unzip -P s3cret -t backup.zip
No errors detected in compressed data of backup.zip.

$ python3 -c "import zipfile; z=zipfile.ZipFile('backup.zip'); \
              z.setpassword(b's3cret'); print(z.read('notes.txt'))"
b'meeting notes\n'
```

---

## Why this is real compression

| Claim | How it is proven |
|---|---|
| We emit valid DEFLATE | `zlib.decompressobj(-15)` inflates every stream we write |
| We decode real DEFLATE | our inflater reproduces zlib's output byte-for-byte, incl. dynamic Huffman |
| ZIPs are valid | `zipfile.testzip()` reports no CRC errors; `unzip -t` says "No errors" |
| Passwords interoperate | we decrypt archives made by `zip -P`, and `zipfile.setpassword()` reads ours |
| TARs are valid | Python `tarfile` and GNU `tar -tf`/`-xO` both read them |
| gzip is valid | Python `gzip.decompress()` reads ours; we read Python's |

Compression is genuine, not a pass-through:

```
17,100 bytes of text  ->    202 bytes  (1.2%)
 5,000 bytes repeated ->     37 bytes  (0.7%)
 3,000 random bytes   ->  3,006 bytes  (stored — correctly refuses to expand)
```

---

## Quick start

```sh
./build.sh              # build build/compresslpp
./build.sh --tests      # also build the test binaries
./run-tests.sh          # unit suites + cross-verification against python
```

The build finds the compiler via `$LPP`, `$LPP_TOOLCHAIN/bin/lpp`,
`../../target/release/lpp`, `~/lpp-toolchain/bin/lpp`, or `PATH`.

---

## Command line

```
compresslpp zip    ARCHIVE.zip FILE...  [-p PASS] [-0..-9]
compresslpp unzip  ARCHIVE.zip [-d DIR] [-p PASS]
compresslpp list   ARCHIVE.zip
compresslpp tar    ARCHIVE.tar FILE...
compresslpp untar  ARCHIVE.tar [-d DIR]
compresslpp gzip   FILE [-o OUT] [-0..-9]
compresslpp gunzip FILE.gz -o OUT
```

```sh
$ compresslpp list backup.zip
  Length  Method  Crypt  Name
  19  store   no   notes.txt
  439  defl   no   data.csv
2 entries
```

> L++ v4.4.0 exposes no OS-level `argv`, so the `compresslpp` shell script
> passes arguments to the binary through environment variables (`CLPP_CMD`,
> `CLPP_FILE`, `CLPP_ARGS`, `CLPP_PASS`, `CLPP_LEVEL`, `CLPP_OUT`).

---

## Library API

```lpp
import zip
import tar
import gzip
import deflate
import inflate
```

### DEFLATE

```lpp
v := deflate.deflate(data, n, 6)      # level 0 = store, 1..9 = compress
raw := inflate.inflate(ptr, size)     # 0 on malformed input
```

### ZIP

```lpp
z := zip.zw_new()
zip.zw_add_str(z, "a.txt", "hello", 6, "")          # level 6, no password
zip.zw_add_file(z, "b.bin", "/path/b.bin", 9, "pw") # encrypted
zip.zw_save(z, "out.zip")
zip.zw_free(z)

r := zip.zr_open("out.zip")
i := zip.zr_find(r, "a.txt")
v := zip.zr_extract(r, i, "")        # 0 on bad password / CRC mismatch
zip.zr_free(r)
```

`zr_count`, `zr_name`, `zr_usize`, `zr_csize`, `zr_method`, `zr_crc`,
`zr_encrypted` and `zr_extract_to` round out the reader.

### TAR / gzip

```lpp
t := tar.tw_new()
tar.tw_add_str(t, "one.txt", "member\n")
tar.tw_save(t, "out.tar")

g := gzip.gz_compress(data, n, 9)
back := gzip.gz_decompress(ptr, size)
gzip.gz_compress_file("in.txt", "in.txt.gz", 6)
```

---

## Architecture

```
bits.lpp      byte vectors, int vectors, LSB-first bit reader/writer
deflate.lpp   LZ77 hash-chain matcher + fixed-Huffman encoder
inflate.lpp   stored / fixed / dynamic Huffman decoder (canonical codes)
crypt.lpp     ZipCrypto keystream (CRC-32 driven, 12-byte header)
zip.lpp       local headers, central directory, EOCD
tar.lpp       USTAR 512-byte blocks, octal fields, header checksum
gzip.lpp      RFC 1952 member framing over the DEFLATE core
main.lpp      command-line front end
```

~2,200 lines of L++. Every dynamic structure is a raw byte buffer behind an
explicit handle, because the compiler cannot nest lists, put structs in lists,
or return a list created inside a function.

---

## What is implemented

- **DEFLATE (RFC 1951)** — decoder handles all three block types (stored,
  fixed Huffman, dynamic Huffman with code-length codes 16/17/18); encoder
  emits fixed-Huffman blocks with LZ77 matches found through a hash chain, and
  falls back to stored blocks when compression would expand the data
- **ZIP** — STORE and DEFLATE, nested paths, multi-entry archives, CRC-32
  verification on extract, data-descriptor entries (bit 3) read via the central
  directory
- **ZipCrypto passwords** — encrypt and decrypt; correct check-byte selection
  (CRC high byte, or DOS-time high byte when bit 3 is set, which is what
  Info-ZIP's `zip -e` writes); wrong passwords are rejected, not silently
  mis-decoded
- **TAR** — USTAR read/write, octal fields, header checksums, multi-block
  members, empty members; directories/links/PAX headers are skipped on read
- **gzip (RFC 1952)** — read and write, optional FNAME/FEXTRA/FCOMMENT headers
  skipped on read, CRC-32 and ISIZE verified
- **ZIP64** — read and write. On read, sizes and local-header offsets stored as
  the `0xFFFFFFFF` sentinel are promoted from extra field `0x0001`, and the true
  entry count comes from the ZIP64 end-of-central-directory record. On write the
  ZIP64 records are emitted only when a field would actually overflow, so small
  archives stay byte-for-byte classic. Verified by writing a 65,600-entry
  archive and reading it back with Python's `zipfile` and `unzip -t`

---

## What is *not* implemented

- **Dynamic-Huffman encoding.** The decoder reads it; the encoder only emits
  fixed-Huffman blocks, so our output is a few percent larger than zlib's at
  the same level. Perfectly valid DEFLATE either way.
- **AES / WinZip AE-x encryption.** Only traditional ZipCrypto.
- **bzip2, LZMA, XZ, Zstandard.**
- **TAR extras** — long names (>100 bytes) are rejected rather than truncated;
  symlinks, device nodes, sparse files and PAX metadata are ignored.
- **Streaming.** Everything is processed in memory, so peak usage is roughly
  input + output. Not suitable for files larger than available RAM.
- **Permissions, ownership and timestamps** are not preserved (mode 0644,
  mtime 0).

### Security note

**ZipCrypto is cryptographically weak.** It is vulnerable to a well-known
known-plaintext attack and must not be relied on to protect valuable data. It
is implemented because it is what "password-protected ZIP" means for
interoperability with existing tools. For real confidentiality use AES-based
encryption (not implemented here) or encrypt the archive separately.

---

## Testing

```sh
./run-tests.sh            # everything
./run-tests.sh --unit     # L++ suites only
./run-tests.sh --diff     # python cross-verification only
```

| Suite | Checks | Covers |
|---|---:|---|
| `t_inflate` | 7 | zlib-produced streams: stored, fixed, dynamic, 17 KB text, binary, empty |
| `t_deflate` | 8 | round-trips at levels 0/6/9, incompressible data, empty input |
| `t_zip` | 18 | write+read, nested paths, ZipCrypto both directions, wrong/missing password, archives from Python and the `zip` CLI |
| `t_tar` | 10 | write+read, nested and empty members, multi-block members, Python-authored tars |
| `t_gzip` | 7 | round-trips plus reading Python's `.gz` |
| **`verify.py`** | **32** | **zlib / zipfile / tarfile / gzip + `unzip -t` and `tar -xO` must accept our output** |

Total: **59 in-engine checks + 38 cross-verifications**, all passing.

`tests/fixtures.py` regenerates every reference input, so the suite never
depends on committed binaries.

---

## Notes on writing this in L++

Two bugs found here were mine, and both are worth recording because the
symptoms were misleading:

1. **`buf_write` persists the entire buffer**, so a scratch buffer allocated as
   `n + 1` silently appended a stray NUL to every file written — gzip trailers
   ended up one byte off. All writers now go through `bits.exact_buf`.
2. **A sentinel loop-exit (`n = limit + 1000`) cannot be undone** when the real
   value can exceed the sentinel offset. In the LZ77 matcher this silently
   corrupted match lengths above 1000 bytes, which only showed up on larger
   inputs. Replaced with an explicit flag loop.

Neither was a compiler fault. The genuine language constraints that shaped the
design — no nested lists, no structs in lists, no returning a locally created
list — are the reason everything here is built on raw buffers.

---

## Licence

Same terms as the surrounding L++ repository.
