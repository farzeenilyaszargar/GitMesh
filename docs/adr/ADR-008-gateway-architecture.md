# ADR-008: Gateway Architecture

## Status

Accepted for initial implementation.

## Context

The website needs fast repository browsing while preserving the rule that the
website is not authoritative storage.

## Decision

Gateways use the same GitMesh retrieval and verification protocol as local
`gitmeshd`.

## PROTOCOL REQUIREMENT

Web/API access must derive from verifiable repository network state, not a
separate authoritative repository database.

## IMPLEMENTATION CHOICE

Deploy `gitmesh-gateway` behind web/API/CDN services with immutable caches and
checkpoint-aware invalidation.

## INITIAL DEFAULT

Public repositories use aggressive CID/checkpoint-keyed caching and fall back to
durable segment reconstruction on cache misses.

## FUTURE OPTION

Organizations can operate independent gateways; browsers may perform more direct
verification and retrieval over time.
