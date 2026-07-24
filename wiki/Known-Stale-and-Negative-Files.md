# Known Stale and Negative Files

Repo-wide `lpp --checkall` currently checks every `.lpp` file it can find. The repository also contains:

- negative tests that are supposed to fail,
- old stress/scratch files,
- stale examples from earlier language versions,
- package tests that require a specific working directory/import layout.

Therefore, full repo `--checkall` is not currently expected to be clean.

## Known categories

### Negative safety tests

These intentionally fail to prove the compiler rejects unsafe ownership cycles:

```text
tests/aot_reject_arc_cycle.lpp
tests/aot_reject_list_arc_cycle.lpp
```

### Old scratch/stress files

Examples include root-level or `test/` scratch files that may use older mutability rules or old struct-cycle behavior.

Typical failures:

```text
Cannot mutate field of immutable variable
Cyclic owned struct detected
```

### Experimental stdlib files

Some stdlib modules are ahead of stable builtin coverage.

Known examples:

```text
stdlib/algo.lpp       # uses list_set, which is not currently public/stable
stdlib/result.lpp     # helper arithmetic over enum custom type is experimental
```

### Package layout issues

Some package tests assume a package-root working directory. If run from the wrong directory, imports may fail.

Example:

```text
packages/lpp-zip/tests/test_zip.lpp may not find import zip unless src/zip.lpp is on the import path
```

## Recommended documentation validation

For wiki/docs examples, create a clean directory and run:

```bash
lpp --checkall
```

Do not use the entire repository as the documentation validation unit until negative/stale files are isolated.

## Recommended cleanup task

Future cleanup should move files into:

```text
tests/positive/
tests/negative/
examples/current/
examples/legacy/
```

Then `lpp --checkall` can become a clean positive-only gate.
