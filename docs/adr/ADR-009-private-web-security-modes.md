# ADR-009: Private Repository Web Security Modes

## Status

Accepted for initial implementation.

## Context

Private repositories need browser access, but different users will choose
different tradeoffs between zero-knowledge access and server-side features.

## Decision

Support opaque gateway mode and trusted integration mode.

## PROTOCOL REQUIREMENT

Private repository plaintext keys must be released only to authorized clients or
explicitly authorized trusted services.

## IMPLEMENTATION CHOICE

Opaque mode sends ciphertext through gateways to browser-side decryption. Trusted
mode uses signed capability grants for services such as CI, AI, search, and
scanning.

In the initial opaque mode, gateways perform provider lookup, shard download,
erasure decoding, and ciphertext integrity verification. Browsers receive
encrypted repository segments and use an authorized device key plus a WASM
GitMesh verifier/parser to decrypt, verify, and parse Git objects locally.
Gateways may learn repository and segment access metadata, but not source code
plaintext or repository keys.

## INITIAL DEFAULT

Opaque mode is the privacy-preserving baseline. Trusted access is explicit,
auditable, scoped, and revocable.

## FUTURE OPTION

Browser-side shard retrieval, browser-side Reed-Solomon reconstruction,
hardware-backed keys, and client-side/local indexing can improve opaque-mode
functionality.
