# gitmesh-git

Git compatibility primitives for GitMesh.

Currently implemented:

- Git object kinds
- canonical loose-object bytes: `<type> <len>\0<payload>`
- SHA-1 Git object IDs for existing repositories
- basic parse/validation of canonical Git object bytes
- zlib decoding for existing loose objects under `.git/objects`
- strict Git pack v2/v3 parsing with trailer checksum validation
- OFS-delta and REF-delta resolution
- deterministic Git pack v2 writing for full objects
- commit parent/tree link parsing and tree entry parsing for graph validation

Pack negotiation and the smart-protocol adapter are still future work.
