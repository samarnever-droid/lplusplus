# L++ GitHub Linguist submission package

This repository is prepared for an upstream language-recognition contribution to [`github-linguist`](https://github.com/github-linguist).

## Canonical language identity

| Item | Value |
|---|---|
| Display name | L++ |
| Common name | L Plus Plus |
| File extension | `.lpp` |
| Programming-language scope | `source.lpp` |
| Existing grammar | `editors/vscode/syntaxes/lpp.tmLanguage.json` |
| VS Code language ID | `lpp` |
| Representative sample | `linguist/samples/lpp/ownership_and_closures.lpp` |

## Why this repository alone cannot force GitHub recognition

GitHub language statistics are driven by the upstream Linguist language database. A repository README, file extension, or `.gitattributes` entry cannot create a new globally recognized language. L++ needs an upstream Linguist pull request adding its language definition and grammar metadata.

## Upstream PR checklist

1. Fork `github-linguist` and create a focused branch.
2. Add an L++ entry to Linguist's language metadata with a new unique language ID.
3. Register `.lpp` as the extension and `source.lpp` as the TextMate scope.
4. Provide or reference the maintained TextMate grammar in this repository.
5. Add a representative `.lpp` sample from this directory.
6. Run the upstream Linguist test suite and language-detection checks.
7. Open the upstream pull request with a link to this repository and its VS Code grammar.

## Repository-side readiness

- `.gitattributes` marks generated benchmark reports as `linguist-generated=true`.
- Benchmark reports no longer distort source-language statistics.
- The committed sample demonstrates indentation, comments, typed functions, structs, closures, lists, conditionals, and function calls.
- The VS Code grammar is versioned in this repository rather than being only a binary extension artifact.

## Important distinction

This repository can open a **readiness PR** for its own metadata and samples. The actual global `L++` label in GitHub's language bar requires acceptance by the upstream `github-linguist` project.
