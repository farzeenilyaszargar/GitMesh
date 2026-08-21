# ADR-010: No Blockchain

## Status

Accepted for initial implementation.

## Context

GitMesh is decentralized and P2P, but global blockchain consensus is unnecessary
for immutable object storage and costly for repository-specific mutable refs.

## Decision

Do not use a blockchain as the system's authority.

## PROTOCOL REQUIREMENT

Repository survival and verification must not depend on GitMesh-operated servers
or a global chain. Mutable state must be repository-scoped, signed, and
recoverable.

## IMPLEMENTATION CHOICE

Use content-addressed immutable storage plus repository-specific coordination and
signed checkpoints.

## INITIAL DEFAULT

No token, cryptocurrency, or global ledger is part of V0-V5.

## FUTURE OPTION

External transparency logs or witness networks may be added for checkpoint
gossip/equivocation detection if they are optional and non-authoritative.
