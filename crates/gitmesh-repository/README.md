# gitmesh-repository

Repository object-store spine for GitMesh.

This crate connects canonical Git objects to the GitMesh storage layer. It is
the first non-demo backend boundary for repository contents: ingest a Git
object, segment/encrypt/erasure-code it through `gitmesh-storage`, retain an
object record, and verify read-back by reconstructing and decrypting shards.

It also exposes `run_repository_transport_repair_proof`, which exercises a Git
object through the current P2P storage boundary: distribute encrypted shards to
storage peers, publish provider leases, remove one provider, repair the missing
shard onto a replacement peer, rediscover providers, reconstruct, decrypt, and
verify the original Git object.

Repository objects can also be checked against external provider evidence with
`object_availability_report`. The report rejects provider records that do not
match the stored object's segment and shard CIDs, then evaluates the remaining
active records against shard, operator, and region requirements.
