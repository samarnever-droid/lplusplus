#!/usr/bin/env python3
"""Tests for deterministic release archive creation."""

from __future__ import annotations

import hashlib
import importlib.util
import os
import stat
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("package_release", ROOT / "scripts" / "package_release.py")
assert SPEC and SPEC.loader
package_release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(package_release)

EPOCH = 1_700_000_000


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


class PackageReleaseTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name) / "lpp-test"
        (self.root / "bin").mkdir(parents=True)
        (self.root / "lib").mkdir()
        compiler = self.root / "bin" / "lpp"
        compiler.write_bytes(b"compiler\n")
        compiler.chmod(0o755)
        (self.root / "lib" / "runtime.txt").write_text("runtime\n", encoding="utf-8")

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def test_tar_gz_is_reproducible_and_normalized(self) -> None:
        first = Path(self.temporary.name) / "first.tar.gz"
        second = Path(self.temporary.name) / "second.tar.gz"
        package_release.create_tar_gz(self.root, first, EPOCH)
        os.utime(self.root / "lib" / "runtime.txt", (EPOCH + 500, EPOCH + 500))
        package_release.create_tar_gz(self.root, second, EPOCH)
        self.assertEqual(digest(first), digest(second))

        with tarfile.open(first, "r:gz") as archive:
            members = archive.getmembers()
            self.assertEqual([member.name for member in members], sorted(member.name for member in members))
            self.assertTrue(all(member.mtime == EPOCH for member in members))
            self.assertTrue(all(member.uid == member.gid == 0 for member in members))
            compiler = archive.getmember("lpp-test/bin/lpp")
            self.assertEqual(stat.S_IMODE(compiler.mode), 0o755)
            self.assertEqual(archive.extractfile(compiler).read(), b"compiler\n")

    def test_zip_is_reproducible_and_preserves_executable_mode(self) -> None:
        first = Path(self.temporary.name) / "first.zip"
        second = Path(self.temporary.name) / "second.zip"
        package_release.create_zip(self.root, first, EPOCH)
        os.utime(self.root / "bin" / "lpp", (EPOCH + 500, EPOCH + 500))
        package_release.create_zip(self.root, second, EPOCH)
        self.assertEqual(digest(first), digest(second))

        with zipfile.ZipFile(first) as archive:
            names = archive.namelist()
            self.assertEqual(names, sorted(names))
            compiler = archive.getinfo("lpp-test/bin/lpp")
            self.assertEqual((compiler.external_attr >> 16) & 0o777, 0o755)
            self.assertEqual(archive.read(compiler), b"compiler\n")

    @unittest.skipUnless(hasattr(os, "symlink"), "symbolic links unavailable")
    def test_symbolic_links_are_rejected(self) -> None:
        os.symlink("runtime.txt", self.root / "lib" / "runtime-link")
        with self.assertRaisesRegex(ValueError, "symbolic links"):
            package_release.create_tar_gz(self.root, Path(self.temporary.name) / "bad.tar.gz", EPOCH)


if __name__ == "__main__":
    unittest.main()
