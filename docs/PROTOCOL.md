# GitMesh Protocol Specification

This document defines protocol-level entities and invariants. Implementation
details belong in component documents unless they affect interoperability.

## Protocol Principles

- All protocol objects are explicitly versioned.
- Signed or hashed objects use deterministic serialization.
- Signed payloads include domain separation/type strings.
- Cryptographic algorithms are identified by registry values, not implicit code.
- All state-changing operations carry transaction IDs and are idempotent.
- DHT, gateway, cache, and storage responses are untrusted until verified.
- Content integrity and content availability are separate properties.

## Deterministic Encoding

Initial default: deterministic CBOR using a constrained profile:

- canonical map key ordering
- no duplicate map keys
- explicit integer bounds
- byte strings for binary data
- UTF-8 strings for names and type identifiers
- no floats in signed protocol objects

Future options may include another deterministic format only through versioned
object envelopes and test vectors.

## Object Envelope

All hashed protocol objects use a common envelope:

```text
ProtocolObject {
  domain: "gitmesh.<type>"
  version: u32
  hash_algorithm: HashAlgorithm
  body: deterministic-cbor
}
```

Signed objects wrap the canonical object hash:

```text
SignedObject {
  object: ProtocolObject
  signature_algorithm: SignatureAlgorithm
  signer: IdentityRef
  signature: bytes
}
```

Signatures are over a domain-separated transcript containing the object domain,
version, hash algorithm, object CID, and serialized body.

## Identifier Types

GitMesh distinguishes these identifiers:

- `GitOid`: Git object ID, SHA-1 or SHA-256 depending on repository format.
- `GitMeshCid`: CID for GitMesh protocol objects.
- `PlainSegmentId`: hash of canonical plaintext segment content.
- `EncryptedSegmentCid`: hash of encrypted segment ciphertext and descriptor.
- `ShardCid`: hash of one erasure-coded shard plus shard metadata.
- `RepoId`: stable cryptographic repository identity.
- `AccountId`: stable account root identity.
- `DeviceId`: revocable device identity.
- `TransactionId`: client-generated mutation idempotency key.

GitMesh storage integrity must not rely only on legacy Git SHA-1.

## Required Protocol Object Types

The first schema set must include:

- `RepoManifest`: repository identity, storage policy, coordinator set, index root,
  crypto policy, and current policy epoch.
- `ObjectIndexPage`: Merkle-DAG page mapping Git OIDs to segment locations.
- `SegmentDescriptor`: segment layout, compression, encryption, coding policy,
  checksums, shard CIDs, and audit tree root.
- `StorageLease` / `ShardProviderRecord`: signed promise from a storage node to
  hold a shard until expiry.
- `NodeAnnouncement`: signed peer, role, protocol, region, and address
  advertisement with expiry.
- `AvailabilityRecord`: signed directory record listing current shard leases.
- `AuditChallenge` and `AuditResponse`: bounded shard byte-range proof protocol
  messages, with Merkle-backed proofs as the production target.
- `RefUpdate`: signed compare-and-swap mutation request.
- `RefCheckpoint`: signed append-only checkpoint of ref state and history root.
- `PolicyManifest`: protected branches, permissions, mutation requirements.
- `CoordinatorSet`: active coordinator members and replacement policy.
- `CoordinatorCheckpoint`: signed coordinator state checkpoint.
- `TransactionReceipt`: committed, rejected, or pending mutation result.
- `DeviceCertificate` and `DeviceRevocation`: account-to-device authority.
- `MembershipGrant` and `MembershipRevocation`: account/org repository access.
- `KeyEpoch`: repository encryption epoch and encrypted member key material.
- `CollaborationEvent`: signed issues, PRs, comments, reviews, releases, stars,
  and discussions event.

## Idempotency

Every state-changing operation includes:

```text
transaction_id
repo_id
actor
operation_type
operation_body_hash
```

If a client retries the same transaction after losing an ACK, the receiver returns
the original `TransactionReceipt`. If the same transaction ID is reused with a
different body hash, the receiver rejects it as an idempotency violation.

## Replay And Rollback Protection

Mutable protocol objects include repository epoch, policy epoch, actor identity,
transaction ID where applicable, parent/checkpoint references where applicable,
and expiration timestamps for leases/provider records. Clients reject stale
checkpoints when a fresher valid checkpoint is known and detect conflicting
checkpoints at the same sequence as coordinator equivocation.

## Compatibility Requirements

Protocol readers must reject unknown critical fields and may ignore unknown
non-critical extension fields. Writers must include a minimum reader version for
objects that depend on extension semantics.
