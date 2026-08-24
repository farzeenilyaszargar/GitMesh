# gitmesh-collaboration

Signed collaboration event primitives for GitMesh.

This crate models issues, pull requests, comments, and reviews as deterministic
protocol objects. V0 exposes local typed records for the CLI and web prototype
and also provides signed-event wrappers that bind authoritative collaboration
events to certified device keys. Production storage will persist the verified
event graph through repository collaboration indexes.

Snapshots persist event and parent references as full typed GitMesh CIDs while
remaining able to read older raw-digest local snapshots.
