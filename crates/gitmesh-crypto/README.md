# gitmesh-crypto

Reviewed-primitive wrappers for GitMesh.

Currently implemented:

- `AeadAlgorithm::XChaCha20Poly1305`
- generated 256-bit segment content keys
- generated 192-bit XChaCha nonces
- AAD-bound authenticated encryption and decryption

This crate intentionally wraps established libraries. It must not grow custom
cryptographic algorithms.
