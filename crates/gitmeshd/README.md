# gitmeshd

Local GitMesh daemon entrypoint.

Currently implemented:

- `gitmeshd v0-proof [payload...]`
- `gitmeshd serve [socket] [object-store] [ref-store]`
- `gitmeshd ping [socket]`
- `gitmeshd socket-v0-proof [socket] [payload...]`
- `gitmeshd ref-get [socket] <ref>`
- `gitmeshd ref-list [socket]`
- `gitmeshd ref-update [socket] <tx> <ref> <expected|none> <new|delete> <signer>`
- `gitmeshd ref-checkpoint [socket]`
- `gitmeshd object-put [socket] <blob|tree|commit|tag> <hex-payload>`
- `gitmeshd pack-put [socket] <pack-hex>`
- `gitmeshd pack-import [socket] <pack-file>`
- `gitmeshd pack-get [socket]`
- `gitmeshd pack-export [socket] <pack-file>`
- `gitmeshd object-get [socket] <oid>`
- `gitmeshd object-list [socket]`
- `gitmeshd repo-status [socket]`

The socket protocol is a V0 line protocol over Unix domain sockets:

- `PING`
- `V0_PROOF [payload...]`
- `REF_GET <ref>`
- `REF_LIST`
- `REF_UPDATE <tx> <ref> <expected|none> <new|delete> <signer>`
- `REF_UPDATE_FORCE <tx> <ref> <expected|none> <new|delete> <signer>`
- `REF_UPDATE_SIGNED <tx> <ref> <expected|none> <new|delete> <label-hex> <account-key-hex> <device-key-hex> <cert-signature-hex> <update-signature-hex>`
- `REF_CHECKPOINT`
- `OBJECT_PUT <blob|tree|commit|tag> <hex-payload|-for-empty>`
- `PACK_PUT <pack-hex>`
- `PACK_GET all`
- `OBJECT_GET <oid>`
- `OBJECT_LIST`
- `REPO_STATUS`

The optional store paths persist canonical Git object storage state and
repository ref, transaction-receipt, and checkpoint state across daemon
restarts. This is not the final daemon API, but it is now a real local IPC
boundary. `PACK_PUT` imports full objects plus OFS-delta and REF-delta entries
with pack checksum validation. `PACK_GET all` currently exports full-object
packs for compatibility.
