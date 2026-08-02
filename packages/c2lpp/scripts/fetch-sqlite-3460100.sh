#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
WORK=${1:-$ROOT/work/sqlite-3460100}
URL=https://sqlite.org/2024/sqlite-amalgamation-3460100.zip
ARCHIVE=$WORK/sqlite-amalgamation-3460100.zip
ARCHIVE_SHA=77823cb110929c2bcb0f5d48e4833b5c59a8a6e40cdea3936b99e199dbbe5784
SOURCE_SHA=6c35bc5f7f85eac9c49928bacbb02bb694b547aabf69197e058cca245ad80e83
SOURCE=$WORK/sqlite-amalgamation-3460100/sqlite3.c

mkdir -p "$WORK"
if [ ! -f "$ARCHIVE" ]; then
    curl -L --fail --retry 3 -o "$ARCHIVE" "$URL"
fi
printf '%s  %s\n' "$ARCHIVE_SHA" "$ARCHIVE" | sha256sum -c - >/dev/null
if [ ! -f "$SOURCE" ]; then
    unzip -q "$ARCHIVE" -d "$WORK"
fi
printf '%s  %s\n' "$SOURCE_SHA" "$SOURCE" | sha256sum -c - >/dev/null
printf '%s\n' "$SOURCE"
