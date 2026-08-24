# gitmesh-core

Shared GitMesh protocol primitives.

Currently implemented:

- `HashAlgorithm`
- `CidKind`
- typed `Cid`
- strict full-text `Cid` parsing for `gitmesh:v0:<kind>:<hash>:<digest>`
- `ProtocolEnvelope`
- domain-separated encrypted segment and shard CID helpers

This crate is intentionally small. Higher-level protocol schemas should be added
only when another component needs them.
