# gitmeshd

Local GitMesh daemon entrypoint.

Currently implemented:

- `gitmeshd v0-proof [payload...]`
- `gitmeshd network-repair-proof [payload...]`
- `gitmeshd serve [socket] [object-store] [ref-store] [policy-store] [key-grant-store] [account-store] [collaboration-store]`
- `gitmeshd ping [socket]`
- `gitmeshd socket-v0-proof [socket] [payload...]`
- `gitmeshd socket-network-repair-proof [socket] [payload...]`
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
- `gitmeshd collab-seed-samples [socket]`
- `gitmeshd issue-open [socket] <owner/repo> <actor> <title-hex> <body-hex|-> <labels-hex-list|->`
- `gitmeshd issue-list [socket] <owner/repo>`
- `gitmeshd pr-open [socket] <owner/repo> <actor> <source-ref> <target-ref> <title-hex> <body-hex|-> <labels-hex-list|->`
- `gitmeshd pr-list [socket] <owner/repo>`
- `gitmeshd key-grant-list [socket] <repo-id> [latest|all|epoch]`
- `gitmeshd key-grant-revoke-device [socket] <device-cid> <effective-epoch>`
- `gitmeshd key-grant-status [socket] <repo-id>`

The socket protocol is a V0 line protocol over Unix domain sockets:

- `PING`
- `V0_PROOF [payload...]`
- `NETWORK_REPAIR_PROOF [payload...]`
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
- `KEY_GRANT_PUT <repo-id> <epoch> <account-cid> <device-cid> <device-key-hex> <algorithm> <nonce-hex> <wrapped-key-hex> <signer-key-hex> <signature-hex>`
- `KEY_GRANT_LIST <repo-id> [latest|all|epoch]`
- `KEY_GRANT_REVOKE_DEVICE <device-cid> <effective-epoch>`
- `KEY_GRANT_STATUS <repo-id>`
- `COLLAB_SEED_SAMPLES`
- `ISSUE_OPEN <owner/repo> <actor> <title-hex> <body-hex|-> <labels-hex-list|->`
- `ISSUE_LIST <owner/repo>`
- `PR_OPEN <owner/repo> <actor> <source-ref> <target-ref> <title-hex> <body-hex|-> <labels-hex-list|->`
- `PR_LIST <owner/repo>`
- `REPO_STATUS`

The optional store paths persist canonical Git object storage state and
repository ref, transaction-receipt, checkpoint, policy, key-grant, account, and
collaboration event state across daemon restarts. This is not the final daemon
API, but it is now a real local IPC boundary. `PACK_PUT` imports full objects
plus OFS-delta and REF-delta entries with pack checksum validation. `PACK_GET
all` currently exports full-object packs for compatibility.

`NETWORK_REPAIR_PROOF` is a Git-object-level backend proof for the current P2P
storage spine. It stores a blob as encrypted erasure-coded shards, publishes
providers, removes a storage provider, repairs the missing shard to a
replacement peer, and verifies that the original Git object is recovered exactly.

The Next.js web app exposes gateway routes under `/api/gitmesh/*`. Those routes
translate HTTP JSON requests into local `gitmeshd` socket reads for health,
status, refs, key grants, and the network repair proof. Account/session
infrastructure is exposed through server-side JSON routes for local development:

- `GET /api/gitmesh/health`
- `GET /api/gitmesh/status`
- `GET /api/gitmesh/refs`
- `GET /api/gitmesh/key-grants`
- `GET /api/gitmesh/network-repair-proof`
- `POST /api/gitmesh/network-repair-proof`
- `GET /api/gitmesh/issues`
- `POST /api/gitmesh/issues`
- `GET /api/gitmesh/pulls`
- `POST /api/gitmesh/pulls`
- `GET /api/gitmesh/accounts`
- `POST /api/gitmesh/accounts`
- `GET /api/gitmesh/accounts/:username`
- `PATCH /api/gitmesh/accounts/:username`
- `POST /api/gitmesh/accounts/:username/repositories`
- `POST /api/gitmesh/sessions`
- `POST /api/gitmesh/sessions/auth`
- `DELETE /api/gitmesh/sessions/:sessionId`

Mutating routes require `x-gitmesh-admin-token` when
`GITMESH_WEB_ADMIN_TOKEN` is set, and the web gateway wraps daemon mutations
with `GITMESHD_ADMIN_TOKEN` when the daemon itself has admin auth enabled. This
is still a local backend API, not a production browser auth flow.
