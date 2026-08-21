# ADR-001: Rust As Protocol Implementation Language

## Status

Accepted for initial implementation.

## Context

GitMesh has protocol-critical code paths: deterministic encoding, parser
boundaries, cryptographic wrappers, Git object handling, storage verification,
network messages, and ref mutation validation.

## Decision

Use Rust for protocol-critical crates and binaries.

## PROTOCOL REQUIREMENT

Protocol behavior must be deterministic, memory-safe in exposed parsers,
testable, fuzzable, and portable across clients, nodes, gateways, and browser
WASM where feasible.

## IMPLEMENTATION CHOICE

Implement shared protocol logic in Rust crates, with `gitmesh-core` as the primary
source of truth.

## INITIAL DEFAULT

Start with a Rust workspace containing `gitmesh-core`, `gitmesh-crypto`,
`gitmesh-storage`, `gitmesh-network`, `gitmesh-git`, `gitmeshd`, and
`git-remote-gitmesh`.

## FUTURE OPTION

Other languages may bind to stable schemas and test vectors, but they must not
define divergent protocol behavior.
