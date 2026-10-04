# L++ v1.2.0 compatibility oracle

This directory freezes the first behavioral oracle for the clean compiler
rewrite. The authoritative implementation is tag `v1.2.0`, commit
`a7b61748396a48c03f2d4cd416774fe8fbd845fb`.

`files.sha256` protects the selected source programs and expected outputs from
accidental edits. Run:

```sh
python3 scripts/check_compatibility_freeze.py
```

A fixture hash may change only when the change is classified as one of:

1. required v1 compatibility;
2. edition-gated legacy behavior;
3. confirmed bug fix with a regression and changelog entry;
4. previously unspecified behavior that is now documented.

The oracle intentionally does not freeze compiler binaries in Git. Materialize
one from the exact commit and verified `Cargo.lock` with:

```sh
python3 scripts/materialize_v1_oracle.py
```

The helper writes the binary under `target/compatibility/` with a SHA-256
sidecar. It can also install a prebuilt oracle, but only when both
`--binary-url` and an independently trusted `--binary-sha256` are supplied.
