# Security policy

## Reporting a vulnerability

Do not open a public issue for a suspected vulnerability. Use GitHub's private
security-advisory reporting for `samarnever-droid/lplusplus`. Include affected
versions, reproduction steps, impact, and any proposed mitigation.

## CI policy

- GitHub Actions permissions default to `contents: read`.
- Write permissions are allowed only in reviewed release/deployment jobs.
- Workflows must not expose an interactive shell, unauthenticated HTTP command
  endpoint, Cloudflare quick tunnel, or evaluate user-provided shell text.
- Secrets are never available to untrusted pull-request code.
- Every third-party action is pinned to a reviewed immutable commit SHA. The
  nearby version comment is informational and must never replace the SHA.

## Download and update policy

- Release archives require a `SHA256SUMS` manifest from the same immutable
  GitHub release before installation.
- Installers reject missing/malformed checksums, digest mismatches, and archive
  paths that escape the temporary extraction directory.
- The compiler and package manager must not execute `curl | sh`, `curl | bash`,
  `irm | iex`, or equivalent downloaded code pipelines.
- Automated self-update remains disabled until it can verify an immutable
  release manifest and perform an atomic, rollback-capable replacement.
- SHA-256 protects release integrity but is not publisher authentication.
  Release archives and `SHA256SUMS` therefore receive GitHub/Sigstore build
  provenance attestations. Verify both before installation, for example:

  ```sh
  gh attestation verify SHA256SUMS --repo samarnever-droid/lplusplus
  gh attestation verify lpp-linux-x86_64.tar.gz --repo samarnever-droid/lplusplus
  ```

  The local installer still performs the separate digest comparison after the
  attestation has authenticated the downloaded manifest.

## Package-manager policy

Future package installation must pin immutable commits or content hashes. All
archive extraction must reject absolute paths, parent traversal, escaping
links, duplicate overwrite tricks, and configured size-limit violations.
