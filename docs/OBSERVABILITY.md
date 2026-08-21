# GitMesh Observability

GitMesh observability must support debugging, SLOs, abuse handling, auditability,
and repair. It is not required for protocol correctness.

## Tracing

Use OpenTelemetry-compatible traces. Core spans:

- name resolution
- checkpoint fetch and verification
- pack cache lookup
- segment availability lookup
- shard retrieval
- erasure reconstruction
- decryption
- Git pack generation
- push ingest
- storage lease acquisition
- ref CAS submission
- repair workflow

Trace context must not leak private repository plaintext or keys.

## Metrics

Minimum metrics:

```text
peer_count
connection_success_rate
relay_usage
hole_punch_success
DHT_lookup_latency
DHT_failure_rate
segment_fetch_latency
shard_fetch_latency
cache_hit_ratio
healthy_shards
under_replicated_segments
repair_queue_depth
repair_latency
audit_success_rate
push_latency
fetch_latency
clone_latency
ref_conflicts
coordinator_availability
bytes_in
bytes_out
storage_used
```

## Dashboards

Initial dashboards:

- local node health
- storage durability and repair
- networking and NAT traversal
- Git operation latency
- coordinator availability and conflicts
- gateway cache effectiveness
- audit failure and peer reputation

## Logs

Logs use structured events with transaction IDs, repo IDs where safe, peer IDs,
checkpoint IDs, and operation results. Private paths, blob contents, decrypted
payloads, and key material must not be logged.

## Alerts

Production alerts cover under-replicated segments, repair backlog growth, audit
failure spikes, coordinator unavailability, DHT failure spikes, gateway cache
poisoning attempts, and unusually high ref conflict rates.
