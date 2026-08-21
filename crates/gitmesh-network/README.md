# gitmesh-network

Network-facing discovery primitives for GitMesh.

Currently implemented:

- `PeerId`
- `NodeRole`
- `ShardProviderRecord`
- `InMemoryAvailabilityDirectory`

This is a V0 local component, not libp2p yet. The purpose is to establish the
availability-directory boundary that V1 will move onto real networking.
