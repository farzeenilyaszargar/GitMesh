# GitMesh Web Gateway

The web product is a client of the same GitMesh repository network used by local
Git clients. It must not maintain a separate authoritative repository database.

## Public Repository Flow

```text
browser
  |
  v
CDN
  |
  v
web/API
  |
  v
gitmesh-gateway
  |
  v
gitmesh-core
  |
  +--> coordinator checkpoints
  +--> pack/tree/blob caches
  +--> P2P storage network
```

Public pages aggressively cache immutable assets: Git objects, rendered blobs,
tree listings, optimized clone packs, release artifacts, and static metadata.
Mutable views are keyed by signed checkpoint identity and invalidated by newer
verified checkpoints.

Public repository requests may return plaintext Git object content from the
gateway because the repository is intentionally public. The gateway still is not
authoritative: clients can verify signed ref checkpoints, object indexes,
segment CIDs, and Git object IDs.

```text
browser
  |
  v
gateway/cache
  |
  +--> signed ref state: main -> commit
  +--> object index: blob -> segment/offset/length
  +--> shard providers
  |
  v
erasure decode + verify
  |
  v
plaintext Git object response
```

## Opaque Private Mode

```text
storage nodes
  |
  v
encrypted shards
  |
  v
gateway
  |
  v
erasure decode + ciphertext verification
  |
  v
encrypted segment
  |
  v
browser WASM
  |
  v
decrypt + verify + parse Git objects
```

In opaque mode, the gateway never receives repository plaintext keys. Browser
clients use a WASM build of GitMesh verification, object-index, and crypto logic
where feasible. This mode limits server-side search, AI, and code intelligence.

The browser gets repository keys through authorized device identity:

```text
user login
  |
  v
browser unlocks device key
  |
  v
decrypts repository key for current key epoch
  |
  v
decrypts encrypted segment locally
```

The gateway may know that repository `R` requested segment `S`, but must not see
source code contents or plaintext repository keys in opaque private mode.

Initial private web mode uses gateway-side erasure reconstruction and
browser-side decryption. Browser-side shard retrieval and Reed-Solomon decoding
remain a future option for stronger decentralization.

## Trusted Integration Mode

Users and organizations may explicitly authorize plaintext access to trusted
services such as CI, AI, code search, code scanning, or dependency scanning.
Authorization is represented by signed capability grants that are scoped,
auditable, revocable, and tied to key epochs.

## Gateway Guarantees

Gateways may cache and accelerate, but clients still verify:

- signed checkpoints
- repository policy
- object indexes
- segment and shard integrity
- Git object IDs
- private-content decryption tags

Losing all GitMesh-operated gateways must not lose repositories.

## Cache Model

Gateways and CDNs cache aggressively, but only at the correct trust boundary:

- public immutable objects may be cached as plaintext by CID or checkpoint-stable
  URL
- private opaque mode may cache ciphertext segments, never plaintext or repo keys
- trusted integration mode may cache plaintext only under explicit scoped grants

Git objects and GitMesh segments are immutable, so cache hits are nearly ideal:
`blob ABC123` cannot legitimately change while remaining `ABC123`.

GitMesh-operated web servers are therefore index, cache, and gateway services,
not a master database of all source code. A cache miss locates providers,
downloads enough valid shards, reconstructs the segment, verifies it, and then
serves either plaintext for public repos or ciphertext for private opaque repos.

## CDN Rules

Immutable public content may be CDN cached by CID or checkpoint-stable URL.
Private ciphertext may be cached only when cache policy permits and keys never
leave authorized clients/services. CDN content is never authority.
