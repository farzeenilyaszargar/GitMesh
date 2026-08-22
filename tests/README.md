# Tests

This directory contains integration, chaos, interop, and adversarial test
fixtures for GitMesh. See `docs/TEST_STRATEGY.md`.

Implemented:

- `local-daemon-smoke.sh`: starts `gitmeshd` with persisted store files, writes
  collaboration events, stores a Git object, publishes a ref, verifies `gm`
  issue/PR writes and reads, starts the built Next.js gateway against the same
  socket, verifies HTTP issue/PR writes and reads, and checks the
  object/ref/collaboration stores were written.
