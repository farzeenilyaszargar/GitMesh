# git-remote-gitmesh

Git remote-helper entrypoint for GitMesh URLs.

Currently implemented:

- basic remote-helper command loop
- required `capabilities` command
- conservative `option` support
- `fetch` capability that installs the daemon-exported pack into `$GIT_DIR`
  with `git index-pack --strict`
- real `check-connectivity` handling through `git fsck --connectivity-only`
- integration coverage for installing an advertised pack into a real bare Git
  object database
- `push` capability for branch updates: imports reachable local objects into
  `gitmeshd` and publishes refs through daemon CAS updates
- live daemon coverage for helper push: a real local Git commit is imported,
  published as a daemon ref, and exported back as a valid GitMesh pack
- force-push and ref deletion mapping to daemon ref-update semantics
- `list` / `list for-push` ref advertisement from local `gitmeshd` when
  available
- `--v0-proof` smoke command for exercising the storage pipeline through this
  binary and a running local `gitmeshd`
- `--network-proof` smoke command for exercising the Git-object P2P repair proof
  through this binary and a running local `gitmeshd`

This now supports the first Gen 1 clone/fetch/push path through a running local
`gitmeshd`. Push certs and incremental negotiation are still future work.
