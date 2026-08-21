# ADR-005: Encryption Before Erasure Coding

## Status

Accepted for initial implementation.

## Context

Private repository data must remain opaque to storage nodes while still gaining
durability from erasure coding.

## Decision

Compress and encrypt private segments before erasure coding.

## PROTOCOL REQUIREMENT

Storage nodes must not need private repository plaintext keys, and shard
integrity must be verifiable before decrypted data is trusted.

## IMPLEMENTATION CHOICE

Use authenticated encryption on compressed segment plaintext, then erasure-code
the ciphertext.

## INITIAL DEFAULT

Use XChaCha20-Poly1305 unless security review or platform requirements choose a
different reviewed AEAD.

## FUTURE OPTION

AEAD algorithms can be rotated through versioned algorithm identifiers and key
epochs.
