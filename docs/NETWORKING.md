# GitMesh Networking

GitMesh uses libp2p unless a later ADR identifies a compelling replacement.

## Initial Stack

- QUIC transport
- TCP fallback
- Noise and/or TLS 1.3 security
- Kademlia DHT for routing and discovery
- AutoNAT
- DCUtR hole punching
- Circuit Relay v2
- mDNS for local development and LAN discovery

Multiple independent bootstrap and relay nodes must be supported. No
GitMesh-operated bootstrap node is authoritative.

## Node Announcements

Nodes publish signed announcements containing their peer ID, operator ID, roles,
region, supported protocols, reachable addresses, signer device ID, issue time,
and expiry time. Announcements are signed by a certified device key and are not
accepted after expiry. A peer registry may cache announcements locally, but the
cached copy is not authority without signature, certificate, and freshness
verification.

## Provider Records

Storage providers advertise shard availability with signed provider records.
Each record binds the segment CID, shard CID, shard index, peer ID, operator ID,
storage role set, lease epoch, and expiry time. Directories may cache these
records, but clients verify signatures, expiry, shard CIDs, and placement policy
before counting a shard toward durability.

## DHT Usage

The DHT is not an application database. It is used for:

- `PeerId -> addresses`
- `RepoId -> checkpoint/manifest providers`
- `SegmentId -> availability directory providers`

GitMesh avoids one expensive DHT lookup per shard by using higher-level
availability directories.

## Availability Directory

```text
Segment CID
  |
  v
DHT provider lookup
  |
  v
Availability Directory peers
  |
  v
signed StorageLease records
  |
  v
shard providers
```

Availability records are signed and expire. Clients verify leases, shard CIDs,
provider signatures, placement policy, and audit status before treating data as
durable.

## Retrieval

Clients rank providers by locality, historical success, latency, bandwidth,
failure domain, and recent audit health. Fetches use parallel multi-source
requests, hedging, retry budgets, and cancellation once `k` valid shards arrive.

## NAT And Partitions

Clients should attempt direct QUIC first, then TCP fallback, hole punching, and
relay use. Relay use is observable and rate-limited. Network partitions must not
allow rollback or stale checkpoint acceptance.

## Abuse Controls

Nodes enforce:

- connection limits
- malformed-message limits
- request and bandwidth quotas
- relay limits
- decompression and object-size limits
- per-peer reputation
- temporary bans
- protocol abuse reports

These controls must not become centralized requirements for repository
readability.
