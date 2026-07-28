#!/usr/bin/env python3
"""fixtures.py — build the reference inputs the L++ test suites read.

Two kinds of fixture:
  r_*.bin / c_*.bin   raw data and its zlib-DEFLATE form, so the L++ inflater
                      can be checked against a known-good encoder
  py_*.zip/tar/gz     archives written by Python (and by the `zip` CLI when
                      available, for real ZipCrypto), so the L++ readers are
                      checked against real-world producers
"""
import gzip as pygzip
import io
import os
import random
import subprocess
import tarfile
import zipfile
import zlib

random.seed(7)

# ── raw + deflated pairs ──
cases = {
    'stored':  b'abc',
    'fixed':   b'hello world\n',
    'dynamic': (b'the quick brown fox jumps over the lazy dog. ' * 40),
    'repeat':  b'A' * 5000,
    'binary':  bytes(random.randrange(256) for _ in range(3000)),
    'empty':   b'',
    'large':   (b'Lorem ipsum dolor sit amet, consectetur adipiscing elit. ' * 300),
}
lvl = {'stored': 0, 'fixed': 1, 'dynamic': 9, 'repeat': 9,
       'binary': 6, 'empty': 6, 'large': 9}
for name, raw in cases.items():
    c = zlib.compressobj(lvl[name], zlib.DEFLATED, -15)
    open('r_%s.bin' % name, 'wb').write(raw)
    open('c_%s.bin' % name, 'wb').write(c.compress(raw) + c.flush())

# ── ZIPs written by Python ──
with zipfile.ZipFile('py_store.zip', 'w', zipfile.ZIP_STORED) as z:
    z.writestr('a.txt', 'hello from python\n')
with zipfile.ZipFile('py_deflate.zip', 'w', zipfile.ZIP_DEFLATED) as z:
    z.writestr('b.txt', 'python deflate payload ' * 100)
with zipfile.ZipFile('py_multi.zip', 'w', zipfile.ZIP_DEFLATED) as z:
    for i in range(10):
        z.writestr('file%02d.txt' % i, ('entry %d ' % i) * (i * 20 + 5))

# ── ZipCrypto archive: prefer the real `zip` CLI ──
made = False
if subprocess.run(['which', 'zip'], capture_output=True).returncode == 0:
    open('c.txt', 'w').write('python encrypted content\n')
    if subprocess.run(['zip', '-q', '-P', 'pw123', 'py_crypt.zip', 'c.txt']).returncode == 0:
        made = True
if not made:
    # Minimal traditional-PKWARE writer so the suite still runs without `zip`.
    import struct

    def _tab():
        t = []
        for n in range(256):
            c = n
            for _ in range(8):
                c = 0xEDB88320 ^ (c >> 1) if c & 1 else c >> 1
            t.append(c)
        return t
    T = _tab()

    class K:
        def __init__(self, pw):
            self.k = [0x12345678, 0x23456789, 0x34567890]
            for ch in pw:
                self.upd(ch)

        def upd(self, b):
            self.k[0] = (T[(self.k[0] ^ b) & 0xff] ^ (self.k[0] >> 8)) & 0xffffffff
            self.k[1] = (self.k[1] + (self.k[0] & 0xff)) & 0xffffffff
            self.k[1] = (self.k[1] * 134775813 + 1) & 0xffffffff
            self.k[2] = (T[(self.k[2] ^ (self.k[1] >> 24)) & 0xff] ^ (self.k[2] >> 8)) & 0xffffffff

        def enc(self, b):
            t = (self.k[2] | 2) & 0xffff
            c = b ^ (((t * (t ^ 1)) >> 8) & 0xff)
            self.upd(b)
            return c

    data = b'python encrypted content\n'
    crc = zlib.crc32(data) & 0xffffffff
    k = K(b'pw123')
    body = bytes(k.enc(random.randrange(256)) for _ in range(11))
    body += bytes([k.enc((crc >> 24) & 0xff)])
    body += bytes(k.enc(b) for b in data)
    name = b'c.txt'
    out = struct.pack('<IHHHHHIIIHH', 0x04034b50, 20, 1, 0, 0, 33, crc,
                      len(body), len(data), len(name), 0) + name + body
    cd = struct.pack('<IHHHHHHIIIHHHHHII', 0x02014b50, 20, 20, 1, 0, 0, 33, crc,
                     len(body), len(data), len(name), 0, 0, 0, 0, 32, 0) + name
    eocd = struct.pack('<IHHHHIIH', 0x06054b50, 0, 0, 1, 1, len(cd), len(out), 0)
    open('py_crypt.zip', 'wb').write(out + cd + eocd)

# ── TAR written by Python ──
with tarfile.open('py.tar', 'w', format=tarfile.USTAR_FORMAT) as t:
    for name, data in [
        ('alpha.txt', b'alpha from python\n'),
        ('beta.txt', b'beta member\n'),
        ('sub/gamma.txt', b'gamma nested\n'),
        ('big.bin', bytes(i % 256 for i in range(4096))),
        ('zero.txt', b''),
    ]:
        ti = tarfile.TarInfo(name)
        ti.size = len(data)
        t.addfile(ti, io.BytesIO(data))

# ── gzip written by Python ──
open('py_text.gz', 'wb').write(pygzip.compress(open('r_dynamic.bin', 'rb').read(), 6))
open('py_large.gz', 'wb').write(pygzip.compress(open('r_large.bin', 'rb').read(), 9))

print('fixtures ready in', os.getcwd())
