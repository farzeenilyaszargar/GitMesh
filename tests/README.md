# Tests

This directory contains integration, chaos, interop, and adversarial test
fixtures for GitMesh. See `docs/TEST_STRATEGY.md`.

Implemented:

- `local-daemon-smoke.sh`: starts `gitmeshd` with persisted store files, writes
  collaboration events, stores a Git object, publishes a ref, registers
  network listen/bootstrap/storage peer state, verifies `gm` issue/PR/network
  reads and writes, starts the built Next.js gateway against the same socket,
  verifies HTTP issue/PR writes and reads plus network reads, and checks the
  object/ref/collaboration/network stores were written.
