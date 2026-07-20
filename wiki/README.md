# L++ Wiki

This directory is the version-controlled source for the L++ project wiki. It is intentionally kept in the main repository so reviews, releases, and documentation changes remain auditable. It can be published to the GitHub Wiki repository (`lplusplus.wiki.git`) when repository credentials are available.

## Start here

- [[Home]] — project status and navigation
- [[Getting-Started]] — installation and first program
- [[Language-Guide]] — syntax and ownership model
- [[Compiler-and-Builds]] — commands, packages, and artifacts
- [[Networking]] — native network architecture and current API
- [[Native-Linking]] — ELF, PE, and Mach-O status
- [[Contributing]] — safe contribution workflow
- [[Roadmap]] — honest milestones and non-goals

## Documentation policy

Every public capability must state its backend and platform boundary. “Implemented” means there is source, automated verification, and a documented user-facing contract. Experimental features must say so directly.

## v0.1.3 documentation status

For the current supported subset and explicit feature boundaries, see
[`documentation/CURRENT_CAPABILITIES.md`](../../documentation/CURRENT_CAPABILITIES.md).

Do not use historical benchmark numbers or roadmap text as current guarantees.
