# gitmesh-coordination

Repository-level mutable state coordination primitives.

Currently implemented:

- validated Git ref names
- `RefUpdate` compare-and-swap updates
- idempotent `TransactionId` receipts
- in-memory `RefStore` for local coordination tests

This is not a distributed coordinator yet. It is the correctness core that the
future coordinator service will wrap.
