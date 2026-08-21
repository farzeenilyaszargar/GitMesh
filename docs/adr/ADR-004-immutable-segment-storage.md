# ADR-004: Immutable Segment Storage

## Status

Accepted for initial implementation.

## Context

Git packs are optimized delivery artifacts, but treating each pack as the only
authoritative representation would make deduplication, repair, indexing, and
partial retrieval harder.

## Decision

Ingest Git objects into a canonical CAS and aggregate them into immutable GitMesh
segments.

## PROTOCOL REQUIREMENT

Authoritative repository data must be content-addressed, immutable, verifiable,
and decoupled from disposable Git delivery packs.

## IMPLEMENTATION CHOICE

Use segment descriptors and Merkle-DAG object indexes mapping Git OIDs to segment
locations.

## INITIAL DEFAULT

Target 8-32 MiB plaintext segments for experiments, with size controlled by
repository storage policy and benchmarks.

## FUTURE OPTION

Segment sizing, packing heuristics, compression, and hot-pack derivation may
evolve without changing Git compatibility.
