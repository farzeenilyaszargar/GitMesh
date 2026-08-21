# GitMesh Cryptography

GitMesh must not invent cryptographic algorithms. It uses established libraries and
versioned algorithm identifiers so future migrations are possible.

## Algorithm Registry

Every encrypted, hashed, or signed object records algorithm identifiers:

```text
HashAlgorithm
SignatureAlgorithm
KeyAgreementAlgorithm
KdfAlgorithm
AeadAlgorithm
PasswordHashAlgorithm
TransportSecurityAlgorithm
```

Initial defaults:

- signatures: Ed25519
- private segment encryption: XChaCha20-Poly1305 unless benchmarks or platform
  constraints justify AES-GCM-SIV or another reviewed AEAD
- public-key key distribution: HPKE with X25519
- password-protected recovery material: Argon2id
- transport: libp2p Noise and/or TLS 1.3
- hashing: BLAKE3 or SHA-256 for GitMesh CIDs, with Git SHA-1/SHA-256 preserved
  only as Git object IDs

## Private Repository Encryption Boundary

Private storage follows:

```text
plaintext segment
  |
  v
compression
  |
  v
authenticated encryption
  |
  v
ciphertext
  |
  v
erasure coding
```

Storage nodes never require plaintext keys. Erasure coding operates on
ciphertext, so no shard provider can see private content.

## Key Epochs

Repository content encryption is organized by `KeyEpoch`.

```text
KeyEpoch {
  repo_id
  epoch
  parent_epoch
  content_key_wrapping_algorithm
  authorized_members[]
  encrypted_epoch_keys[]
  created_by
  signature
}
```

Removing a member rotates future repository encryption state. GitMesh cannot force
a previously authorized user who already downloaded plaintext to forget it.

## Signed Object Rules

Signed objects include:

- domain separation string
- protocol object version
- repository or account scope
- policy/key epoch when relevant
- hash and signature algorithm identifiers
- signer identity
- replay protection fields

Verification must fail closed on unknown critical algorithm IDs, malformed
encodings, duplicate map keys, invalid signatures, or unsupported epochs.

## Recovery Material

Account and organization recovery must not require GitMesh to permanently hold
plaintext root private keys. Recovery packages may be encrypted to recovery
contacts, hardware-backed device keys, or password-derived keys using Argon2id.
The exact UX can evolve, but protocol objects must support rotating compromised
recovery material.

## External Review

Before production, external review is required for key management, deterministic
encoding, signed object verification, browser crypto/WASM handling, recovery,
and revocation flows.
