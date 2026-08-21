# GitMesh Roadmap

## V0 - Local Storage Proof

Goal: prove the data pipeline locally with simulated nodes.

Acceptance:

- bytes encrypt, erasure-code, distribute, lose nodes, reconstruct, decrypt
  exactly
- 5-20 local simulated nodes
- property tests for coding and encryption
- repeatable demo command

Mocks:

- network
- placement reputation
- coordinator
- web

Must be correct:

- deterministic IDs
- encryption boundary
- shard verification
- reconstruction exactness

## V1 - Real P2P Storage

Goal: real libp2p storage protocol with leases, audit, and repair.

Acceptance:

- multi-node network survives churn, shard loss, corrupt responses, and lease
  expiry
- availability directory records are signed and expiring
- repair replaces missing or unhealthy shards

## V2 - Git Compatibility

Goal: public clone/fetch/push through `git-remote-gitmesh` and `gitmeshd`.

Acceptance:

- ordinary Git commands work for small repos, many tiny objects, and large repos
- Gen 1 cached bare-repo adapter passes Git interop tests
- refs do not advance before storage policy is met

## V3 - Repository Coordination

Goal: signed manifests, CAS refs, checkpoints, concurrent pushes, and recovery.

Acceptance:

- simultaneous push conflicts produce normal Git behavior
- rollback, replay, stale checkpoint, and equivocation tests fail safely
- transaction retries return original receipts

## V4 - Private Repositories

Goal: private content encryption, identities, ACLs, device revocation, and key
epochs.

Acceptance:

- storage nodes never receive plaintext keys
- revoked members cannot decrypt future epochs
- browser/device revocation is auditable

## V5 - Web Gateway

Goal: browse repositories through the same GitMesh data plane.

Acceptance:

- gateway can rebuild from network state
- public repo pages use immutable caches
- opaque private mode does not expose plaintext keys to the gateway

## V6 - Collaboration

Goal: issues, PRs, reviews, comments, releases, stars, organizations, and
discussions as signed events.

Acceptance:

- eventual events verify independently
- strongly coordinated operations route through repository coordination
- event indexes are rebuildable

## V7 - Platform Features

Goal: search, LFS, releases, packages, Pages.

Acceptance:

- public indexes are rebuildable
- private search requires trusted authorization or local/client-side indexing
- large binary flows avoid pathological Git object handling

## V8 - Compute

Goal: CI/CD, runners, security scanning, AI, and Codespaces-like services.

Acceptance:

- compute consumes explicitly authorized data
- compute services never become storage authority
- outputs are signed or linked to verifiable inputs where needed

## Build Order

1. `gitmesh-core`: IDs, deterministic encoding, object envelope, schema test
   vectors.
2. `gitmesh-crypto`: hash/signature/AEAD/HPKE wrappers and key epochs.
3. `gitmesh-storage`: segment packing, erasure coding, leases, audit, repair model.
4. `gitmesh-network`: libp2p transport, provider lookup, availability directory.
5. `gitmesh-git`: Git object validation, pack ingest/export, cached bare repo
   adapter.
6. `gitmeshd`: local daemon API and durable local cache.
7. `git-remote-gitmesh`: remote-helper protocol integration.
8. coordinator role: ref CAS and checkpoints.
9. gateway role: public and private web retrieval modes.
10. indexer and compute roles.

## Local Development Environment

Provide `deploy/local/` scripts or compose-like tooling to run:

- 1 client daemon
- 1-3 coordinators
- 5-20 storage nodes
- 1 bootstrap node
- 1 relay node
- 1 gateway
- optional repair and indexer nodes

The environment must support deterministic failure injection for tests.

## Benchmarks

Benchmark suites:

- small repository
- Linux-scale large repository
- many tiny objects
- giant binary history
- monorepo
- high-latency WAN
- high peer churn
- NAT-heavy environment

Metrics:

- clone/fetch/push time
- time to first byte
- DHT resolution
- shard throughput
- erasure encode/decode speed
- cache hit rate
- repair bandwidth
- metadata overhead
- storage overhead
- CPU and memory
