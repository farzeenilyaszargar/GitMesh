# gitmesh-core

Shared GitMesh protocol primitives.

Currently implemented:

- `HashAlgorithm`
- `CidKind`
- typed `Cid`
- `ProtocolEnvelope`
- domain-separated encrypted segment and shard CID helpers

This crate is intentionally small. Higher-level protocol schemas should be added
only when another component needs them.
