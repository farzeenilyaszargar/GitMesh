# gitmesh-repository

Repository object-store spine for GitMesh.

This crate connects canonical Git objects to the GitMesh storage layer. It is
the first non-demo backend boundary for repository contents: ingest a Git
object, segment/encrypt/erasure-code it through `gitmesh-storage`, retain an
object record, and verify read-back by reconstructing and decrypting shards.
