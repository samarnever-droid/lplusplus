# Archived v1.2 source-validation debt

These failures belong to the frozen v1.2 compatibility snapshot. They are not
accepted semantics for the clean compiler and must not silently become new
language behavior. `tests/source_baseline.json` keeps their paths and diagnostic
fingerprints blocking in CI until a replacement feature test or an explicit
compatibility decision removes them.

| Area | Frozen paths | Archived reason | Rewrite disposition |
|---|---|---|---|
| Legacy package-manager sources | `pm/src/add_remove.lpp`, `pm/src/install.lpp` | unresolved `dir_remove_all` builtin | implement through reviewed filesystem capability metadata or remove the dead source path |
| Legacy package-manager sources | `pm/src/main.lpp`, `pm/src/publisher.lpp` | brace syntax rejected during import/lexing | parse only if the v1 oracle proves it is user-visible syntax; otherwise migrate these internal sources |
| Historical stress programs | `test/Samar.lpp`, `test/mega_stress_test.lpp`, `test/stress_test.lpp` | mutation now requires an explicit `mut` binding | preserve the safety rejection and migrate fixtures when they become active tests |
| Analyzer package | `packages/lpp-analyzer/src/ownership_graph.lpp`, `packages/lpp-analyzer/src/ownership_proof.lpp` | referenced `c_call_graph` module is absent | restore a pinned module or remove the stale imports before publishing |
| Semver package | `packages/lpp-semver/src/semver.lpp` | legacy colon expression is rejected | migrate source unless differential testing establishes required syntax |
| TOML package | `packages/lpp-toml/src/toml.lpp` | legacy colon/newline form is rejected | migrate source unless differential testing establishes required syntax |

The three SamarOS standalone failures are **not** in this debt archive. They are
project-dialect fragments and are validated as one ordered source set by the
separate SamarOS validator.
