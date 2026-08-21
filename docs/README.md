# GitMesh Planning Documents

GitMesh is a temporary codename. Protocol objects should use versioned namespaces
so product naming can change without breaking interoperability.

## Read Order

1. [ARCHITECTURE.md](ARCHITECTURE.md)
2. [PROTOCOL.md](PROTOCOL.md)
3. [STORAGE.md](STORAGE.md)
4. [COORDINATION.md](COORDINATION.md)
5. [CRYPTO.md](CRYPTO.md)
6. [IDENTITY.md](IDENTITY.md)
7. [GIT_INTEGRATION.md](GIT_INTEGRATION.md)
8. [NETWORKING.md](NETWORKING.md)
9. [WEB_GATEWAY.md](WEB_GATEWAY.md)
10. [WEB_DESIGN.md](WEB_DESIGN.md)
11. [DATA_MODEL.md](DATA_MODEL.md)
12. [THREAT_MODEL.md](THREAT_MODEL.md)
13. [OBSERVABILITY.md](OBSERVABILITY.md)
14. [TEST_STRATEGY.md](TEST_STRATEGY.md)
15. [ROADMAP.md](ROADMAP.md)
16. [adr/](adr/)

## Shared Glossary

- `RepoId`: cryptographic repository identity; not a human path.
- `GitOid`: Git's own object identifier.
- `GitMeshCid`: content identifier for GitMesh protocol objects.
- `Segment`: immutable aggregate of canonical Git object records.
- `Shard`: erasure-coded piece of an encrypted or public segment.
- `StorageLease`: signed, expiring storage promise for a shard.
- `CoordinatorSet`: repository-specific group that serializes mutable state.
- `RefUpdate`: signed compare-and-swap ref mutation.
- `RefCheckpoint`: signed checkpoint of refs and ref history.
- `KeyEpoch`: repository encryption epoch for private repositories.
- `Gateway`: web/API bridge into the same GitMesh data plane used by `gitmeshd`.

## Highest-Risk Open Questions

1. Which mature coordination implementation best fits standard mode before
   high-assurance BFT is needed?
2. Which deterministic CBOR library/profile provides the best Rust/WASM support
   and rejects malformed encodings safely?
3. Which erasure-coding implementation is mature enough for production after V0
   experiments?
4. What minimum independent storage-operator qualification policy is required
   before durability claims are meaningful?
5. How much browser-side GitMesh logic is practical in WASM for opaque private
   repository mode?
6. What recovery UX balances survivability with social-engineering resistance
   for accounts and organizations?

These questions should be resolved by experiments, ADR updates, and external
review before production commitments.
