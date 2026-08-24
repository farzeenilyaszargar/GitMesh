# gitmesh-coordination

Repository-level mutable state coordination primitives.

Currently implemented:

- validated Git ref names
- `RefUpdate` compare-and-swap updates
- signed `RefUpdate` verification through certified device keys
- idempotent `TransactionId` receipts
- chained `RefCheckpoint` records and signed checkpoint verification
- in-memory `RefStore` plus persisted snapshots for local coordination tests

This is not a distributed coordinator yet. It is the correctness core that the
future coordinator service will wrap.
