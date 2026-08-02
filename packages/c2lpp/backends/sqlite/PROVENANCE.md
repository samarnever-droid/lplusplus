# Curated pure-L++ SQLite backend provenance

These modules are vendored from `packages/lppsqlite/src` in the same L++
repository and remain under the repository MIT license.

They provide a tested SQLite-file-format-compatible pure-L++ implementation.
They are not mechanically translated from `sqlite3.c`. c2lpp mode
`sqlite-backend` copies this curated implementation into a standalone generated
package while preserving an explicit report:

```text
source_translation_complete=0
curated_backend_substitution=1
```

Do not use backend functionality or line count as evidence that the general C
translator is complete.
