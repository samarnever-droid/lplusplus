#!/usr/bin/env python3
"""Create deterministic L++ release archives from one package directory."""

from __future__ import annotations

import argparse
import gzip
import os
import shutil
import stat
import tarfile
import time
import zipfile
from pathlib import Path

ZIP_EPOCH = 315532800  # 1980-01-01, the earliest timestamp representable by ZIP.


def archive_entries(source: Path) -> list[tuple[Path, str, os.stat_result]]:
    source = source.resolve()
    if not source.is_dir():
        raise ValueError(f"release package source is not a directory: {source}")

    entries: list[tuple[Path, str, os.stat_result]] = []
    paths = [source, *sorted(source.rglob("*"), key=lambda path: path.relative_to(source).as_posix())]
    for path in paths:
        metadata = path.lstat()
        relative = path.relative_to(source)
        name = source.name if relative == Path(".") else f"{source.name}/{relative.as_posix()}"
        if stat.S_ISLNK(metadata.st_mode):
            raise ValueError(f"release packages may not contain symbolic links: {path}")
        if not (stat.S_ISDIR(metadata.st_mode) or stat.S_ISREG(metadata.st_mode)):
            raise ValueError(f"unsupported release package entry: {path}")
        entries.append((path, name, metadata))
    return entries


def normalized_mode(metadata: os.stat_result) -> int:
    if stat.S_ISDIR(metadata.st_mode):
        return 0o755
    return 0o755 if metadata.st_mode & 0o111 else 0o644


def create_tar_gz(source: Path, output: Path, epoch: int) -> None:
    entries = archive_entries(source)
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=epoch, compresslevel=9) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.GNU_FORMAT) as archive:
                for path, name, metadata in entries:
                    info = tarfile.TarInfo(name + ("/" if stat.S_ISDIR(metadata.st_mode) else ""))
                    info.mtime = epoch
                    info.uid = 0
                    info.gid = 0
                    info.uname = ""
                    info.gname = ""
                    info.mode = normalized_mode(metadata)
                    if stat.S_ISDIR(metadata.st_mode):
                        info.type = tarfile.DIRTYPE
                        archive.addfile(info)
                    else:
                        info.type = tarfile.REGTYPE
                        info.size = metadata.st_size
                        with path.open("rb") as contents:
                            archive.addfile(info, contents)


def create_zip(source: Path, output: Path, epoch: int) -> None:
    entries = archive_entries(source)
    output.parent.mkdir(parents=True, exist_ok=True)
    timestamp = time.gmtime(max(epoch, ZIP_EPOCH))[:6]
    with zipfile.ZipFile(output, mode="w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for path, name, metadata in entries:
            is_directory = stat.S_ISDIR(metadata.st_mode)
            info = zipfile.ZipInfo(name + ("/" if is_directory else ""), date_time=timestamp)
            info.create_system = 3
            info.external_attr = normalized_mode(metadata) << 16
            if is_directory:
                info.external_attr |= 0x10
                info.compress_type = zipfile.ZIP_STORED
                archive.writestr(info, b"")
            else:
                info.compress_type = zipfile.ZIP_DEFLATED
                with path.open("rb") as contents, archive.open(info, mode="w") as destination:
                    shutil.copyfileobj(contents, destination)


def parse_epoch(raw: str) -> int:
    try:
        epoch = int(raw)
    except ValueError as error:
        raise argparse.ArgumentTypeError("epoch must be an integer") from error
    if epoch < 0:
        raise argparse.ArgumentTypeError("epoch must not be negative")
    return epoch


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("format", choices=("tar.gz", "zip"))
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument(
        "--epoch",
        type=parse_epoch,
        default=parse_epoch(os.environ.get("SOURCE_DATE_EPOCH", "0")),
        help="normalized archive timestamp (defaults to SOURCE_DATE_EPOCH, then 0)",
    )
    args = parser.parse_args()

    try:
        if args.format == "tar.gz":
            create_tar_gz(args.source, args.output, args.epoch)
        else:
            create_zip(args.source, args.output, args.epoch)
    except (OSError, ValueError) as error:
        parser.exit(1, f"package_release.py: {error}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
