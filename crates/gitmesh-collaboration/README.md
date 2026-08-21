# gitmesh-collaboration

Signed collaboration event primitives for GitMesh.

This crate models issues, pull requests, comments, and reviews as deterministic
protocol objects. V0 keeps the records local and unsigned so the CLI and web
prototype have typed data to consume; production storage will attach account
signatures and persist the event graph through repository collaboration indexes.
