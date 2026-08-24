# gm

GitMesh command line interface.

`gm` is intentionally familiar to users of GitHub-style CLIs while using
GitMesh terminology and current local capabilities.

Implemented command families:

- `gm auth status`
- `gm repo view [owner/repo]`
- `gm repo clone <gitmesh-url> [directory]`
- `gm repo create [owner/repo] [--public|--private] [-d description]`
- `gm repo materialize [socket] <bare-dir>`
- `gm issue list`
- `gm issue view <id>`
- `gm issue create <title> [body] [label,label]`
- `gm pr list`
- `gm pr status`
- `gm pr view <id>`
- `gm pr create <title> <source-ref> [target-ref] [body] [label,label]`
- `gm daemon ping [socket]`
- `gm daemon proof [socket] [payload...]`
- `gm daemon network-proof [socket] [payload...]`
- `gm daemon network-status [socket]`
- `gm daemon network-listen [socket] <multiaddr>`
- `gm daemon network-bootstrap [socket] <peer-id> <operator-id> <region> <multiaddr>`
- `gm daemon network-peer-add [socket] <peer-id> <operator-id> <roles-csv> <region> <protocols-csv> <addresses-csv|->`
- `gm daemon network-peer-list [socket]`
- `gm daemon network-provider-publish [socket] <segment-cid> <shard-cid> <shard-index> <peer-id> <operator-id> <region> <roles-csv> <lease-epoch> <expires-at>`
- `gm daemon network-provider-find [socket] <segment-cid>`
- `gm daemon network-provider-prune-expired [socket] [now-unix]`
- `gm policy storage-show [socket]`
- `gm policy storage-set [socket] <data-shards> <parity-shards> <min-operators> <min-regions>`
- `gm ref list [socket]`
- `gm ref get [socket] <ref>`
- `gm ref update [socket] <tx> <ref> <expected|none> <new|delete> <signer>`
- `gm ref checkpoint [socket]`
- `gm ref signed-checkpoint [socket]`
- `gm ref signed-update-dev [socket] <tx> <ref> <expected|none> <new|delete>`
- `gm object put [socket] <blob|tree|commit|tag> <hex-payload>`
- `gm object get [socket] <oid>`
- `gm object list [socket]`
- `gm object import-loose [socket] <git-dir>`
- `gm object import-pack [socket] <pack-file>`
- `gm object export-pack [socket] <pack-file>`
- `gm object status [socket]`
- `gm account repos [socket] <owner>`
- `gm proof [payload...]`

Repository creation persists local manifests to `~/.gitmesh/gm-state.tsv` by
default. When `gitmeshd` is running, it creates or reuses the local device
identity, ensures the owner account profile exists, and registers the repository
with a stable `repo:<owner>/<name>` repository ID. Set `GITMESH_GM_STATE` to
point at a different state file for testing.

When paired with `gitmeshd serve [socket] [object-store] [ref-store]`, object
and ref commands exercise the durable local repository spine. `repo
materialize` exports daemon objects/refs into a native bare Git repository using
`git index-pack` and `git update-ref`; `repo clone` uses that same Gen 1 cached
bare-repo bridge and native `git clone`.

`gm object availability [socket] <oid> [min-shards] [min-operators]
[min-regions]` asks the daemon to build local provider evidence for an object
and evaluate it against shard, operator, and region requirements. This is the
current CLI hook for durability-before-ref-publication checks.

`gm daemon network-proof` calls the daemon's Git-object transport repair proof:
encrypted erasure-coded shards are published to providers, one provider is
removed, the missing shard is rebuilt on a replacement peer, and the original
Git object is verified after rediscovery.

`gm daemon network-status`, `network-listen`, `network-bootstrap`, and
`network-peer-add` operate on the daemon's persisted P2P node registry. This is
the current bootstrap/control-plane spine for known peers before the libp2p
runtime takes over live dialing.

`signed-update-dev` uses an ephemeral certified Ed25519 device key to exercise
the production signed `RefUpdate` path. Persistent account/device key storage is
a later auth component.

Pack import accepts full objects plus OFS-delta and REF-delta entries. Pack
export currently writes full-object packs for compatibility.

Issue and pull request list/view commands read daemon collaboration summaries
when `gitmeshd` has persisted collaboration events, with deterministic local
samples as an offline fallback. Create commands write new daemon
collaboration-event records. Persistent production auth and full multi-node
online sync remain future components.

For local end-to-end collaboration demos, start `gitmeshd serve` with a
collaboration store path and run `gitmeshd collab-seed-samples [socket]` once.
The web gateway, `gm issue`, and `gm pr` then read the same persisted daemon
event summaries.
