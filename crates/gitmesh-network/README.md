# gitmesh-network

Network-facing discovery primitives for GitMesh.

Currently implemented:

- `PeerId`
- `OperatorId`
- `NodeRole`
- `NodeDescriptor`
- `ProtocolId`
- `ShardRef`
- `ShardEnvelope`
- `ShardProviderRecord`
- `InMemoryAvailabilityDirectory`
- `NetworkRequest` / `NetworkResponse`
- `NetworkTransport`
- `InMemoryPeer`
- `InMemorySwarm`

This is the V0 testable P2P architecture boundary, not production libp2p yet.
The purpose is to make core storage flows depend on an explicit transport
interface before QUIC/TCP/Noise/Kademlia are introduced.

The in-memory swarm supports:

- peer descriptors with roles, operator identity, region, and protocol support
- request/response routing between peers
- storage-role enforcement for shard writes
- shard integrity verification before store/fetch success
- provider publication and lease expiry
- provider discovery through the availability protocol

`gitmesh-storage` now uses this boundary for a round-trip test:

1. encrypt plaintext into a segment
2. erasure-code ciphertext into shards
3. send shards to storage peers through `NetworkTransport`
4. fetch enough shards back from provider records
5. reconstruct ciphertext
6. decrypt exactly to the original plaintext

The later libp2p implementation should implement `NetworkTransport` or a
compatible async form of it, while preserving these request/response semantics.
