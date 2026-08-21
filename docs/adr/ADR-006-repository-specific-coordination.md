# ADR-006: Repository-Specific Coordination

## Status

Accepted for initial implementation.

## Context

Immutable data does not require global consensus, but refs and permissions need
strong repository-level consistency.

## Decision

Use repository-specific signed coordination for mutable state.

## PROTOCOL REQUIREMENT

Ref and policy mutations must be serialized, signed, auditable, idempotent, and
verifiable without trusting a global blockchain.

## IMPLEMENTATION CHOICE

Use CoordinatorSets that accept signed CAS transactions and publish chained
checkpoints.

## INITIAL DEFAULT

Standard mode starts with simple highly available coordination suitable for
normal repositories.

## FUTURE OPTION

High-assurance repositories may use mature BFT or threshold-signature protocols
after evaluation.
