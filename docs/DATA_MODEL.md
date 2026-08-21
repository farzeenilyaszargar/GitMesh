# GitMesh Data Model

This document summarizes core entities. Schemas are refined in
`protocol/schemas/`.

## Repository Entities

- `RepoId`: cryptographic repository identity.
- `RepoManifest`: repository root metadata, storage policy, coordinator set,
  object index root, crypto policy, and policy epoch.
- `PolicyManifest`: permissions, protected branches, force-push rules, trusted
  services, and mutation requirements.
- `ObjectIndexRoot` / `ObjectIndexPage`: Merkle-DAG index from Git OID to segment
  locations.

## Storage Entities

- `SegmentDescriptor`: immutable segment metadata, hashes, encryption, coding,
  shard set, and audit root.
- `Shard`: erasure-coded shard bytes.
- `StorageLease`: signed storage promise from a peer.
- `AvailabilityRecord`: signed current provider/lease set for a segment.
- `AuditChallenge` / `AuditResponse`: challenge-response proof over shard
  subchunks.

## Coordination Entities

- `CoordinatorSet`: repository coordinator membership and replacement policy.
- `RefUpdate`: signed CAS mutation.
- `RefCheckpoint`: signed ref map and append-only history commitment.
- `CoordinatorCheckpoint`: signed coordinator-state checkpoint.
- `TransactionReceipt`: idempotent result for a mutation.

## Identity Entities

- `AccountId`: stable account root identity.
- `DeviceId`: revocable device identity.
- `DeviceCertificate`: account-to-device binding.
- `DeviceRevocation`: revocation record.
- `OrganizationId`: stable organization root identity.
- `MembershipGrant` / `MembershipRevocation`: repository or organization access.
- `KeyEpoch`: encrypted repository key distribution epoch.

## Collaboration Entities

`CollaborationEvent` is a signed immutable event:

```text
CollaborationEvent {
  event_id
  repo_id
  type
  actor
  parents[]
  payload
  timestamp
  signature
}
```

Issues, PRs, reviews, comments, releases, stars, and discussions are derived
from signed event graphs/logs. Gossip can propagate events quickly, but authority
comes from signatures and graph validation.

## Derived Entities

Search documents, repository popularity, language statistics, trending,
analytics, notification state, rendered diffs, and cached packs are derived and
rebuildable.
