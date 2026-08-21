# ADR-007: Derived Search And Index Plane

## Status

Accepted for initial implementation.

## Context

GitHub-class experiences need search, trending, Explore, language stats,
notifications, and analytics. These services should not become repository
authority.

## Decision

Search and indexes are derived from signed network state.

## PROTOCOL REQUIREMENT

Loss or corruption of index infrastructure must not lose repositories or
authoritative refs.

## IMPLEMENTATION CHOICE

Independent indexers read public signed state and populate disposable search
databases.

## INITIAL DEFAULT

Public indexes are rebuildable. Private search requires trusted authorization or
local/client-side indexing.

## FUTURE OPTION

Multiple competing/federated index providers can serve the same repository
network.
