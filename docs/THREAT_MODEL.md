# GitMesh Threat Model

GitMesh does not promise data can never be lost or hacked. It defines measurable
durability, availability, verification, detection, and recovery properties.

## Threat Matrix

| Threat | Asset | Attacker capability | Attack | Mitigation | Remaining risk | Detection | Recovery |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Malicious storage peer | Shards | Stores or serves bytes | Delete, corrupt, withhold | CIDs, shard hashes, erasure coding, leases, audits | Availability loss if many independent peers fail | Audit failures, retrieval failures | Repair from `k` healthy shards |
| Malicious cache peer | Git objects/packs | Serves cached data | Serve stale or corrupt pack | Verify checkpoints, indexes, Git OIDs, segment hashes | DoS through bad responses | Cache error metrics | Retry other providers |
| Malicious relay | Connectivity | Observes or drops traffic | Traffic analysis, selective DoS | End-to-end transport security, multiple relays | Metadata leakage | Relay failure/latency metrics | Direct/alternate relay paths |
| Malicious gateway | Web access | Controls gateway responses | Rollback, omit refs, serve bad blobs | Browser/client verification of checkpoints and CIDs | UX degradation, DoS | Checkpoint gossip divergence | Switch gateways/direct access |
| DHT poisoning | Discovery | Publishes bad providers | Misroute clients | Signed provider records, verification, multiple lookups | Slower discovery | Lookup failure rates | Query alternate peers/direct hints |
| Sybil peers | Durability | Creates many PeerIDs | Fake independent storage | Signed node announcements, qualified operators, failure-domain policy | Early network bootstrap weakness | Reputation/audit correlation | Exclude unqualified peers, repair |
| Eclipse attack | State freshness | Controls neighborhood | Hide fresh checkpoints | Multiple discovery paths, checkpoint witnesses | Temporary stale view | Stale checkpoint alarms | Reconnect via trusted hints |
| Replay/rollback | Mutable refs | Replays old signed state | Present stale refs | Chained checkpoints, sequence numbers | Offline clients may lag | Freshness comparison | Fetch newer checkpoints |
| Coordinator equivocation | Refs/ACLs | Coordinator signs conflicts | Split history | Checkpoint gossip, witnesses, high-assurance mode | Standard mode may need recovery | Conflicting signed checkpoints | Replace coordinator set |
| Compromised device | Repo authority | Holds device key | Malicious signed ops | Device revocation, policy, protected branches | Damage before revocation | Audit logs, anomaly detection | Revoke, rotate keys, repair refs |
| Malicious collaborator | Repo integrity | Legitimate repo role | Bad push, destructive force | ACLs, protected branches, signed history | Authorized damage | Review/audit trail | Revert, rotate/revoke access |
| Corrupt shard | Data integrity | Bitrot or tampering | Bad reconstruction | Shard hash, audit tree | DoS if many shards corrupt | Audits/retrieval validation | Repair |
| Region outage | Availability | Infrastructure failure | Many peers unavailable | Region/provider placement policy | Insufficient diversity | Health metrics | Repair/rebalance |
| Network partition | Refs/storage | Splits clients/coordinators | Divergent state or stale reads | CAS, quorum/BFT options, checkpoint validation | Reduced availability | Coordinator availability metrics | Reconcile from signed logs |
| Decompression bomb | Node resources | Sends malicious payload | CPU/memory exhaustion | Size limits, streaming validation | DoS attempts | Parser/resource metrics | Ban/rate limit |
| Parser bugs | Node security | Sends malformed inputs | Crash/RCE | Fuzzing, memory-safe Rust, limits | Dependency bugs | Crash/fuzz telemetry | Patch, rotate, quarantine |
| Traffic analysis | Privacy metadata | Observes network | Infer repo activity | Relays, padding future option, private indexes | Metadata remains hard | Access pattern monitoring | Route policy changes |

## Risk Ranking

Highest early risks:

1. Ref coordination correctness and equivocation handling.
2. Key management, device revocation, and private repository epochs.
3. Storage durability assumptions under Sybil and correlated failure domains.
4. Git compatibility around pack negotiation, force pushes, shallow/partial clone.
5. Parser and protocol input hardening.

These must drive testing and review order.
