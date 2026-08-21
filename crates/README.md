# Crates

This directory contains the Rust workspace crates.

Implemented:

- `gitmesh-core`: shared protocol primitives, typed CIDs, algorithm IDs, and
  canonical protocol envelope bytes.
- `gitmesh-collaboration`: deterministic issue and pull request event
  primitives for the signed collaboration graph.
- `gitmesh-coordination`: repository ref compare-and-swap updates with idempotent
  transaction receipts.
- `gitmesh-crypto`: wrapper crate for established AEAD segment encryption
  primitives.
- `gitmesh-git`: canonical Git object bytes, SHA-1 Git object IDs, and basic Git
  object validation.
- `gitmesh-identity`: Ed25519 account/device identity primitives and signed
  device certificates.
- `gitmesh-network`: V0 in-memory availability directory and provider records.
- `gitmesh-repository`: repository object-store spine mapping canonical Git
  objects into encrypted, erasure-coded GitMesh storage records.
- `gitmesh-storage`: V0 local storage proof using encryption, erasure coding,
  simulated nodes, shard loss, reconstruction, and exact recovery.
- `gitmeshd`: local daemon binary skeleton exposing the V0 proof command.
- `git-remote-gitmesh`: Git remote-helper skeleton with capabilities/options
  handshake and V0 proof smoke command.

Planned crate order:

1. `gitmesh-core`
2. `gitmesh-crypto`
3. `gitmesh-storage`
4. `gitmesh-network`
5. `gitmesh-git`
6. `gitmesh-identity`
7. `gitmesh-collaboration`
8. `gitmesh-coordination`
9. `gitmesh-repository`
10. `gitmeshd`
11. `git-remote-gitmesh`
12. `gm`
