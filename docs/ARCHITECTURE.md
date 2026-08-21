# GitMesh Architecture

GitMesh is a temporary codename for a decentralized Git hosting and collaboration
platform. Git remains standard Git. GitMesh provides a remote-helper, local daemon,
gateway, and P2P protocol so repository data can outlive any one company's
servers.

## Current Repository State

This repository currently contains no implementation or product documentation.
The initial work is therefore a greenfield architecture and protocol
specification. No existing APIs, crates, migrations, or data formats need to be
preserved.

## End-State Invariant

A developer uses ordinary Git. GitMesh acts as a decentralized remote. Immutable
repository data is encrypted when private, content-addressed, erasure-coded,
distributed across independent storage operators, continuously audited and
repaired, and cryptographically verified on retrieval. Mutable repository state
is maintained through signed repository-specific coordination. The GitMesh website
is one gateway, index, and client of the network. If GitMesh-operated servers
disappear, repository data and cryptographic identity remain recoverable through
the open protocol.

This is a measurable target, not an absolute promise that data can never be lost
or hacked. Production service levels must specify durability, availability,
audit, repair, and compromise-detection objectives.

## Four Planes

```text
                  +--------------------------+
                  | Service / Index Plane    |
                  | web, API, search, CDN    |
                  +------------+-------------+
                               |
Git client -> helper -> gitmeshd | gateway -> browser/API
                               |
                  +------------v-------------+
                  | Coordination Plane       |
                  | refs, ACLs, policies     |
                  +------------+-------------+
                               |
                  +------------v-------------+
                  | Data Plane                |
                  | objects, segments, shards |
                  +------------+-------------+
                               |
                  +------------v-------------+
                  | P2P Storage Network      |
                  +--------------------------+

                  Compute Plane consumes data through
                  explicit authorization and is never authoritative.
```

### Data Plane

The data plane contains authoritative immutable repository data: Git objects,
object-index pages, segments, shards, large objects, releases, packages,
artifacts, and encrypted private repository content. It must survive the loss of
GitMesh-operated infrastructure.

### Coordination Plane

The coordination plane serializes mutable repository state: branch refs, mutable
tags, repository ownership, ACLs, protected branch policy, key epochs, and
coordinator membership. It is repository-specific and signed; it does not use a
global blockchain.

### Service / Index Plane

The service/index plane provides website, API, search, Explore, trending,
analytics, notifications, SEO, public indexes, and CDN/cache. These systems are
derived from verifiable network state and can be rebuilt.

### Compute Plane

The compute plane provides optional CI/CD, runners, Pages builds, AI, code
scanning, dependency scanning, package builds, and Codespaces-like environments.
It consumes repository data but is not authoritative for storage or refs.

## Primary Flows

Developer Git flow:

```text
git
  |
  v
git-remote-gitmesh
  |
  v
gitmeshd
  |
  v
gitmesh-core libraries
  |
  +--> repository coordinators
  |
  +--> P2P storage network
```

Website flow:

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
gitmesh-core libraries
  |
  +--> repository coordinators
  |
  +--> P2P storage network
```

The website must not maintain a separate authoritative repository database.

## Trust Boundaries

GitMesh clients trust local user devices and verified cryptographic protocol
objects. They do not trust storage peers, relays, DHT results, gateways, caches,
or search indexes without verification.

Private repository plaintext keys are held only by authorized clients or
explicitly trusted services. Storage nodes, DHT nodes, relays, ordinary caches,
and opaque gateways must not receive private repository plaintext keys.

Coordinators are trusted to provide availability and serialization, not
unverified authority. Their signed checkpoints and append-only ref histories must
be independently verifiable by clients.

## Node Roles

One process may hold multiple roles:

- `CLIENT`: user machine running `gitmeshd`.
- `CACHE`: best-effort object, segment, or pack cache.
- `STORAGE`: qualified long-lived shard storage node.
- `BOOTSTRAP`: introduces peers, never authoritative.
- `RELAY`: libp2p relay for NAT/firewall traversal.
- `DHT`: routing/provider discovery participant.
- `GATEWAY`: web/API bridge into the GitMesh network.
- `COORDINATOR`: repository-specific mutable-state serializer.
- `REPAIR`: detects and repairs under-replicated data.
- `INDEXER`: derives search and public metadata indexes.
- `RUNNER`: executes compute workloads under explicit authorization.

Ordinary developer laptops default to `CLIENT` and `CACHE`; they do not count
toward guaranteed durability unless deliberately configured and qualified as
storage nodes.

## Monorepo Layout

Initial implementation should start with a small Rust workspace and split only
when boundaries stabilize:

```text
/
├── Cargo.toml
├── crates/
│   ├── gitmesh-core/
│   ├── gitmesh-crypto/
│   ├── gitmesh-storage/
│   ├── gitmesh-network/
│   ├── gitmesh-git/
│   ├── gitmeshd/
│   └── git-remote-gitmesh/
├── protocol/
│   └── schemas/
├── docs/
│   └── adr/
├── tests/
│   ├── integration/
│   ├── chaos/
│   ├── interop/
│   └── adversarial/
└── deploy/
```

Later binaries such as `gitmesh-gateway`, `gitmesh-coordinator`, `gitmesh-repair`,
`gitmesh-indexer`, and `gitmesh-runner` may either be standalone crates or roles in
one multi-role node daemon. The first implementation should prefer fewer
binaries until operational boundaries are proven.

## Architectural Decision Format

Every major decision must distinguish:

- `PROTOCOL REQUIREMENT`: durable interoperability rule.
- `IMPLEMENTATION CHOICE`: current way to satisfy the requirement.
- `INITIAL DEFAULT`: first shipped setting or algorithm.
- `FUTURE OPTION`: compatible evolution path.

Temporary implementation choices must not accidentally become protocol
requirements.
