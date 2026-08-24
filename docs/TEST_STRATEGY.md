# GitMesh Test Strategy

Testing must prove protocol invariants under normal, faulty, and adversarial
conditions.

## Unit Tests

Cover:

- deterministic encoding
- CID construction
- algorithm registry parsing
- signed object verification
- Git object mapping
- segment descriptors
- storage leases
- ref update policy checks
- key epoch validation

## Property Tests

Required properties:

```text
decode(encode(data)) == data
decrypt(encrypt(data)) == data
any k valid shards reconstruct exact ciphertext
invalid signatures never mutate state
unknown critical fields fail closed
retry(transaction_id) returns original receipt
```

## Fuzzing

Fuzz every externally reachable parser:

- protocol object decoding
- storage messages
- network messages
- Git pack ingest
- object index pages
- collaboration events
- gateway request inputs

## Integration Tests

Run multi-node tests with real protocol networking:

- local daemon smoke: socket server, persisted stores, collaboration writes, Git
  object storage, ref publication, persisted network peer/listen/bootstrap
  state, persisted availability-provider records, `gm` CLI reads/writes,
  web-gateway API reads/writes, and status reads
- local 5-node and 20-node networks
- public push/fetch/clone
- lease renewal and expiry
- audit challenge/response
- repair after shard loss
- coordinator ref CAS conflict
- gateway retrieval from the same data plane

## Chaos Tests

Automatically inject:

- node kill/restart
- network partition
- corrupted disks
- deleted shards
- expired leases
- corrupted bytes
- delayed, duplicated, reordered, and dropped packets
- IP address changes
- forced relay use
- full disks
- device revocation
- key rotation
- coordinator partition

## Adversarial Tests

Simulate Sybil, replay, rollback, stale checkpoint, malicious provider,
malicious gateway, DHT poisoning, decompression bombs, oversized Git objects, and
malformed signed objects.

## Formal Models

Model these state machines in TLA+/PlusCal or equivalent:

- `RefUpdate` CAS
- push transaction
- coordinator replacement
- key epoch rotation
- lease/repair transitions

## Acceptance Gate

No milestone advances until its acceptance tests are automated, documented, and
repeatable in the local development environment.
