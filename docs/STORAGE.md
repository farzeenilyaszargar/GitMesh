# GitMesh Storage Architecture

GitMesh storage separates Git-compatible delivery from durable authoritative
representation.

## Durable Representation

```text
Git pack
  |
  v
validate Git objects
  |
  v
canonical Git object CAS
  |
  v
object index pages
  |
  v
immutable segments
  |
  v
compress
  |
  v
encrypt when private
  |
  v
erasure-code ciphertext
  |
  v
place shards across qualified storage nodes
```

Git packfiles are accepted and produced for compatibility, but they are not the
only authoritative representation.

## Segment Format

Initial default segment target: 8-32 MiB before encryption and erasure coding.
The exact size is benchmark-driven and repository-policy configurable.

Segments contain ordered records:

```text
Segment {
  segment_version
  repo_id
  object_records[]
  compression_algorithm
  plaintext_hash
}

ObjectRecord {
  git_oid
  git_object_type
  offset
  length
  uncompressed_length
}
```

Private repositories encrypt compressed plaintext segments before erasure
coding. Public repositories may skip encryption but still use authenticated
hashes and shard integrity checks.

## Object Index

Large repositories must not use one giant manifest.

```text
RepoManifest
  |
  v
ObjectIndexRoot
  |
  v
ObjectIndexPage*
  |
  v
GitOid -> SegmentLocation
```

Index pages form a Merkle-DAG. Private repositories may encrypt index pages so
storage nodes cannot enumerate content.

## Erasure Coding

Initial default: Reed-Solomon-style coding with `k = 10` data shards and `m = 6`
parity shards for V0 experiments only. These numbers are not protocol constants.

Repository `StoragePolicy` currently persists the correctness-critical shard and
diversity thresholds:

```text
data_shards
parity_shards
minimum_storage_operators
minimum_regions
```

The same policy object is used to derive placement requirements and
availability requirements for durability-before-ref-publication checks. Later
policy revisions add:

```text
maximum_shards_per_operator
maximum_shards_per_asn
audit_policy
repair_threshold
lease_duration
```

Any `k` valid shards reconstruct the exact ciphertext. Invalid shard bytes are
detected before reconstruction output is accepted.

## Storage Leases

Storage is lease-based. Upload is not enough.

```text
StorageLease {
  shard_id
  peer_id
  lease_epoch
  starts_at
  expires_at
  storage_class
  signature
}
```

A repository counts a shard as durably placed only while valid, signed,
unexpired leases satisfy the repository's placement policy. Lease renewal must
produce a new signed lease. Expired or missing leases trigger repair.
Availability reports are derived from current provider-directory lease evidence;
local object presence or cached shard bytes are not counted unless they have
corresponding active provider records.

## Auditing

Each shard is challenged through bounded byte-range audit messages. The current
implementation verifies deterministic challenge transcripts and response hashes;
the production protocol should back those ranges with a Merkle root or
equivalent authenticated structure so auditors can verify responses without
downloading the full shard.

Repeated audit failures reduce peer reliability, trigger replacement, and may
make the node ineligible for high-durability placement. V1 starts with simple
random challenge/response, not novel proof-of-retrievability cryptography.

## Repair

Repair is a first-class protocol:

```text
detect unhealthy segment
  |
  v
obtain >= k healthy shards
  |
  v
reconstruct ciphertext
  |
  v
select independent replacement peer
  |
  v
create missing shard
  |
  v
upload
  |
  v
verify receipt and lease
  |
  v
publish updated availability
```

Multiple independent repair nodes use deterministic responsibility assignment,
repair leases, and jitter. Duplicate repairs must be harmless.

## Placement

The scheduler reasons about failure domains:

- operator identity
- ASN and IP prefix
- region and country
- hosting provider
- device class
- uptime and audit history
- retrieval success
- bandwidth
- disk reliability
- correlated failure history

Unknown anonymous peers may cache data but do not count as independent durable
storage operators.

## Derived Delivery Caches

Fast Git delivery uses disposable caches:

- optimized clone packs
- recent fetch packs
- hot branch packs
- gateway cache
- regional cache
- CDN cache

Clone/fetch attempts hot paths first and falls back to durable segment
reconstruction. Destroying derived packs must never lose authoritative data.

## Garbage Collection

Reachability states:

- reachable
- recently unreachable
- retention window
- release pinned
- legal hold
- GC eligible

Public data cannot be guaranteed globally deleted once copied. Private repos
support cryptographic erasure by destroying keys, subject to previously
authorized users possibly retaining plaintext.
