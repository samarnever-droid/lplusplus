#!/usr/bin/env python3
"""verify.py — cross-verify compresslpp output against Python's own codecs.

Every archive/stream this engine writes is handed to zlib / zipfile / tarfile /
gzip (and to the `unzip` and `tar` CLIs when installed) and must round-trip
exactly. Fixtures written by Python are also fed back to the L++ readers by the
t_*.lpp suites.

Run from the build directory after `build.sh --tests` and the t_* binaries.
"""
import gzip as pygzip
import os
import subprocess
import sys
import tarfile
import zipfile
import zlib

bad = 0


def chk(cond, msg):
    global bad
    print(('PASS  ' if cond else 'FAIL  ') + msg)
    if not cond:
        bad += 1


def have(prog):
    return subprocess.run(['which', prog], capture_output=True).returncode == 0


# ── raw DEFLATE ──────────────────────────────────────────────────────────
print('== raw DEFLATE (zlib must inflate what we deflate) ==')
for comp, raw in [
    ('o_stored.bin', 'r_fixed.bin'), ('o_tiny.bin', 'r_stored.bin'),
    ('o_text.bin', 'r_fixed.bin'), ('o_dynamic.bin', 'r_dynamic.bin'),
    ('o_repeat.bin', 'r_repeat.bin'), ('o_binary.bin', 'r_binary.bin'),
    ('o_empty.bin', 'r_empty.bin'), ('o_large.bin', 'r_large.bin'),
]:
    if not os.path.exists(comp):
        chk(False, 'missing %s' % comp)
        continue
    c = open(comp, 'rb').read()
    want = open(raw, 'rb').read()
    try:
        got = zlib.decompressobj(-15).decompress(c)
    except Exception as e:
        chk(False, '%-16s zlib error: %s' % (comp, e))
        continue
    pct = (len(c) / len(want) * 100) if want else 0
    chk(got == want, '%-16s %6d -> %6d (%.1f%%)' % (comp, len(want), len(c), pct))

# ── ZIP ──────────────────────────────────────────────────────────────────
print()
print('== ZIP (python zipfile) ==')
with zipfile.ZipFile('out_plain.zip') as z:
    chk(z.testzip() is None, 'testzip(): no CRC errors')
    chk(z.namelist() == ['hello.txt', 'big.txt', 'dir/nested.txt'], 'namelist')
    chk(z.read('hello.txt') == b'hello world\n', 'read stored entry')
    chk(len(z.read('big.txt')) == 179, 'read deflated entry')
    chk(z.read('dir/nested.txt') == b'nested entry\n', 'nested path')
    info = z.getinfo('big.txt')
    chk(info.compress_type == zipfile.ZIP_DEFLATED, 'entry really is DEFLATE')
    chk(info.compress_size < info.file_size, 'deflate actually shrank it')

with zipfile.ZipFile('out_crypt.zip') as z:
    z.setpassword(b'hunter2')
    chk(z.read('secret.txt') == b'classified payload\n', 'ZipCrypto stored entry')
    chk(z.read('secret2.txt').startswith(b'another secret'), 'ZipCrypto deflated entry')
with zipfile.ZipFile('out_crypt.zip') as z:
    z.setpassword(b'wrongpw')
    try:
        z.read('secret.txt')
        chk(False, 'wrong password must fail')
    except RuntimeError:
        chk(True, 'wrong password rejected')

if have('unzip'):
    r = subprocess.run(['unzip', '-t', 'out_plain.zip'], capture_output=True, text=True)
    chk(r.returncode == 0 and 'No errors' in r.stdout, 'unzip -t: archive OK')
    r = subprocess.run(['unzip', '-P', 'hunter2', '-t', 'out_crypt.zip'],
                       capture_output=True, text=True)
    chk(r.returncode == 0 and 'No errors' in r.stdout, 'unzip -t -P: encrypted OK')
else:
    print('SKIP  unzip CLI not installed')

# ── ZIP64 ────────────────────────────────────────────────────────────────
# The >65535-entry archive t_zip writes must be readable by real tools. The
# classic EOCD count field is 16 bits, so this only works if the zip64 EOCD
# record is present AND the classic record carries the 0xFFFF sentinel.
print()
print('== ZIP64 (python zipfile + unzip) ==')
if os.path.exists('out_z64_many.zip'):
    with open('out_z64_many.zip', 'rb') as f:
        raw = f.read()
    chk(raw.rfind(b'PK\x06\x06') > 0, 'zip64 EOCD record emitted')
    chk(raw.rfind(b'PK\x06\x07') > 0, 'zip64 EOCD locator emitted')
    with zipfile.ZipFile('out_z64_many.zip') as z:
        names = z.namelist()
        chk(len(names) == 65600, f'python reads all 65600 entries (got {len(names)})')
        chk(z.read('n65599.txt') == b'x', 'python reads the last entry')
        chk(z.testzip() is None, 'testzip(): no CRC errors across 65600 entries')
    if have('unzip'):
        r = subprocess.run(['unzip', '-t', 'out_z64_many.zip'],
                           capture_output=True, text=True)
        chk(r.returncode == 0 and 'No errors' in r.stdout, 'unzip -t: zip64 archive OK')
else:
    print('SKIP  out_z64_many.zip not produced')

# A small archive must NOT gain zip64 records; emitting them unconditionally
# would break readers that predate the extension.
with open('out_plain.zip', 'rb') as f:
    small = f.read()
chk(small.find(b'PK\x06\x06') < 0 and small.find(b'PK\x06\x07') < 0,
    'small archive stays classic (no zip64 records)')

# ── TAR ──────────────────────────────────────────────────────────────────
print()
print('== TAR (python tarfile) ==')
with tarfile.open('out.tar') as t:
    chk(t.getnames() == ['one.txt', 'two.txt', 'dir/three.txt', 'empty.txt'], 'names')
    chk(t.extractfile('one.txt').read() == b'first member\n', 'read member')
    chk(t.extractfile('dir/three.txt').read() == b'nested member\n', 'nested member')
    chk(t.extractfile('empty.txt').read() == b'', 'empty member')
    chk(t.getmember('two.txt').isfile(), 'typeflag is regular file')
if have('tar'):
    r = subprocess.run(['tar', '-tf', 'out.tar'], capture_output=True, text=True)
    chk(r.returncode == 0, 'GNU tar -tf: lists without error')
    r = subprocess.run(['tar', '-xOf', 'out.tar', 'two.txt'], capture_output=True)
    chk(r.stdout == b'second member with more text in it\n', 'GNU tar -xO: extracts')
else:
    print('SKIP  tar CLI not installed')

# ── gzip ─────────────────────────────────────────────────────────────────
print()
print('== gzip (python gzip) ==')
for f, r in [('o_dyn.gz', 'r_dynamic.bin'), ('o_rep.gz', 'r_repeat.bin'),
             ('o_bin.gz', 'r_binary.bin'), ('o_empty.gz', 'r_empty.bin'),
             ('o_large.gz', 'r_large.bin')]:
    if not os.path.exists(f):
        chk(False, 'missing %s' % f)
        continue
    got = pygzip.decompress(open(f, 'rb').read())
    want = open(r, 'rb').read()
    chk(got == want, '%-12s %d bytes' % (f, len(want)))

print()
print('cross-verification:', 'ALL PASS' if bad == 0 else '%d FAILED' % bad)
sys.exit(1 if bad else 0)
