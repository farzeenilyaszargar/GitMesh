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
- `NodeAnnouncement`
- `SignedNodeAnnouncement`
- `KnownPeerRecord`
- `NetworkNodeStore`
- `InMemoryAvailabilityDirectory`
- `NetworkRequest` / `NetworkResponse`
- `NetworkTransport`
- `InMemoryPeer`
- `InMemorySwarm`
- `PlacementPolicy`
- `PlacementPlan`
- deterministic shard placement over qualified storage descriptors
- durable node/listen/bootstrap/known-peer snapshots for `gitmeshd`

This is the V0 testable P2P architecture boundary, not production libp2p yet.
The purpose is to make core storage flows depend on an explicit transport
interface before QUIC/TCP/Noise/Kademlia are introduced.

The in-memory swarm supports:

- peer descriptors with roles, operator identity, region, and protocol support
- signed node announcements with device-certificate verification and expiry
- persistent local node identity, listen addresses, bootstrap peers, and known
  peer records
- request/response routing between peers
- storage-role enforcement for shard writes
- shard integrity verification before store/fetch success
- provider publication and lease expiry
- provider discovery through the availability protocol
- placement across qualified storage peers with operator and region constraints
- transport-level provider publish/discover flows from `gitmesh-storage`
- remote shard audits that distinguish valid, corrupt, and missing shards
- repair of known missing/corrupt provider shards through the same transport
- replacement-peer repair when an original storage provider disappears
- lease refresh and republish after successful transport repair

`gitmesh-storage` now uses this boundary for a round-trip test:

1. encrypt plaintext into a segment
2. erasure-code ciphertext into shards
3. plan placement across independent storage operators
4. send shards to storage peers through `NetworkTransport`
5. publish provider leases to an availability peer
6. discover active providers through the availability protocol
7. audit provider shards through remote storage peers
8. repair missing/corrupt shards from the surviving erasure set
9. move shards to replacement peers when a provider has disappeared
10. refresh provider leases in the availability directory
11. fetch enough shards back from provider records
12. reconstruct ciphertext
13. decrypt exactly to the original plaintext

The later libp2p implementation should implement `NetworkTransport` or a
compatible async form of it, while preserving these request/response semantics.
