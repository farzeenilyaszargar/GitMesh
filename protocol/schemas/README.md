# GitMesh Protocol Schemas

This directory will hold deterministic schema definitions and canonical test
vectors for GitMesh protocol objects.

Initial schema work should define:

- object envelope
- algorithm registries
- CID formats
- `RepoManifest`
- `ObjectIndexPage`
- `SegmentDescriptor`
- `StorageLease`
- `AvailabilityRecord`
- `AuditChallenge`
- `AuditResponse`
- `RefUpdate`
- `RefCheckpoint`
- `PolicyManifest`
- `CoordinatorSet`
- `CoordinatorCheckpoint`
- `TransactionReceipt`
- `DeviceCertificate`
- `DeviceRevocation`
- `MembershipGrant`
- `MembershipRevocation`
- `KeyEpoch`
- `CollaborationEvent`

Every schema must include canonical encoding test vectors before implementation
is considered interoperable.
