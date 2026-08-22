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
- `gm pr list`
- `gm pr status`
- `gm pr view <id>`
- `gm daemon ping [socket]`
- `gm daemon proof [socket] [payload...]`
- `gm daemon network-proof [socket] [payload...]`
- `gm ref list [socket]`
- `gm ref get [socket] <ref>`
- `gm ref update [socket] <tx> <ref> <expected|none> <new|delete> <signer>`
- `gm ref checkpoint [socket]`
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

`gm daemon network-proof` calls the daemon's Git-object transport repair proof:
encrypted erasure-coded shards are published to providers, one provider is
removed, the missing shard is rebuilt on a replacement peer, and the original
Git object is verified after rediscovery.

`signed-update-dev` uses an ephemeral certified Ed25519 device key to exercise
the production signed `RefUpdate` path. Persistent account/device key storage is
a later auth component.

Pack import accepts full objects plus OFS-delta and REF-delta entries. Pack
export currently writes full-object packs for compatibility.

Issue and pull request commands currently read local deterministic sample
collaboration events. Persistent production auth and full multi-node online
sync remain future components.
