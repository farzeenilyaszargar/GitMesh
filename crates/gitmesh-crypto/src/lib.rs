//! Cryptographic wrappers for GitMesh.
//!
//! This crate owns primitive selection and misuse-resistant wrappers. It should
//! stay boring: no custom cryptography, no protocol policy shortcuts.

use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum AeadAlgorithm {
    XChaCha20Poly1305 = 1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RepoContentKey {
    bytes: [u8; 32],
}

impl RepoContentKey {
    pub fn generate() -> Self {
        Self {
            bytes: XChaCha20Poly1305::generate_key(&mut OsRng).into(),
        }
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self { bytes }
    }

    pub fn expose_bytes(self) -> [u8; 32] {
        self.bytes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyWrappingKey {
    bytes: [u8; 32],
}

impl KeyWrappingKey {
    pub fn from_device_secret(
        device_secret: [u8; 32],
        repo_id: &str,
        epoch: u64,
        recipient_device_id: &str,
    ) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"gitmesh.v0.device-repo-key-wrap");
        hasher.update(&device_secret);
        put_field(&mut hasher, repo_id.as_bytes());
        hasher.update(&epoch.to_be_bytes());
        put_field(&mut hasher, recipient_device_id.as_bytes());
        Self {
            bytes: *hasher.finalize().as_bytes(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WrappedRepoKey {
    pub algorithm: AeadAlgorithm,
    pub nonce: [u8; 24],
    pub ciphertext: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SegmentKey {
    bytes: [u8; 32],
}

impl SegmentKey {
    pub fn generate() -> Self {
        Self {
            bytes: XChaCha20Poly1305::generate_key(&mut OsRng).into(),
        }
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self { bytes }
    }

    pub fn expose_bytes(self) -> [u8; 32] {
        self.bytes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SegmentNonce {
    bytes: [u8; 24],
}

impl SegmentNonce {
    pub fn generate() -> Self {
        Self {
            bytes: XChaCha20Poly1305::generate_nonce(&mut OsRng).into(),
        }
    }

    pub fn from_bytes(bytes: [u8; 24]) -> Self {
        Self { bytes }
    }

    pub fn expose_bytes(self) -> [u8; 24] {
        self.bytes
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncryptedBytes {
    pub algorithm: AeadAlgorithm,
    pub key: SegmentKey,
    pub nonce: SegmentNonce,
    pub ciphertext: Vec<u8>,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum CryptoError {
    #[error("encryption failed")]
    Encryption,
    #[error("decryption failed")]
    Decryption,
}

pub type Result<T> = std::result::Result<T, CryptoError>;

pub fn encrypt_segment(plaintext: &[u8], aad: &[u8]) -> Result<EncryptedBytes> {
    let key = SegmentKey::generate();
    let nonce = SegmentNonce::generate();
    let cipher = XChaCha20Poly1305::new(Key::from_slice(&key.bytes));
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce.bytes),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| CryptoError::Encryption)?;

    Ok(EncryptedBytes {
        algorithm: AeadAlgorithm::XChaCha20Poly1305,
        key,
        nonce,
        ciphertext,
    })
}

pub fn wrap_repo_key(
    repo_key: RepoContentKey,
    wrapping_key: KeyWrappingKey,
    aad: &[u8],
) -> Result<WrappedRepoKey> {
    let nonce = SegmentNonce::generate();
    let cipher = XChaCha20Poly1305::new(Key::from_slice(&wrapping_key.bytes));
    let plaintext = repo_key.expose_bytes();
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce.bytes),
            Payload {
                msg: &plaintext,
                aad,
            },
        )
        .map_err(|_| CryptoError::Encryption)?;
    Ok(WrappedRepoKey {
        algorithm: AeadAlgorithm::XChaCha20Poly1305,
        nonce: nonce.expose_bytes(),
        ciphertext,
    })
}

pub fn unwrap_repo_key(
    wrapped: &WrappedRepoKey,
    wrapping_key: KeyWrappingKey,
    aad: &[u8],
) -> Result<RepoContentKey> {
    let plaintext = match wrapped.algorithm {
        AeadAlgorithm::XChaCha20Poly1305 => {
            let cipher = XChaCha20Poly1305::new(Key::from_slice(&wrapping_key.bytes));
            cipher
                .decrypt(
                    XNonce::from_slice(&wrapped.nonce),
                    Payload {
                        msg: wrapped.ciphertext.as_slice(),
                        aad,
                    },
                )
                .map_err(|_| CryptoError::Decryption)?
        }
    };
    let bytes: [u8; 32] = plaintext.try_into().map_err(|_| CryptoError::Decryption)?;
    Ok(RepoContentKey::from_bytes(bytes))
}

pub fn decrypt_segment(encrypted: &EncryptedBytes, aad: &[u8]) -> Result<Vec<u8>> {
    match encrypted.algorithm {
        AeadAlgorithm::XChaCha20Poly1305 => {
            let cipher = XChaCha20Poly1305::new(Key::from_slice(&encrypted.key.bytes));
            cipher
                .decrypt(
                    XNonce::from_slice(&encrypted.nonce.bytes),
                    Payload {
                        msg: encrypted.ciphertext.as_slice(),
                        aad,
                    },
                )
                .map_err(|_| CryptoError::Decryption)
        }
    }
}

pub fn decrypt_segment_bytes(
    algorithm: AeadAlgorithm,
    key: SegmentKey,
    nonce: SegmentNonce,
    ciphertext: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>> {
    decrypt_segment(
        &EncryptedBytes {
            algorithm,
            key,
            nonce,
            ciphertext: ciphertext.to_vec(),
        },
        aad,
    )
}

fn put_field(hasher: &mut blake3::Hasher, value: &[u8]) {
    hasher.update(&(value.len() as u64).to_be_bytes());
    hasher.update(value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_round_trips() {
        let encrypted = encrypt_segment(b"private segment", b"gitmesh.test").unwrap();
        let plaintext = decrypt_segment(&encrypted, b"gitmesh.test").unwrap();

        assert_eq!(plaintext, b"private segment");
    }

    #[test]
    fn aad_mismatch_fails_closed() {
        let encrypted = encrypt_segment(b"private segment", b"gitmesh.good").unwrap();
        let err = decrypt_segment(&encrypted, b"gitmesh.bad").unwrap_err();

        assert!(matches!(err, CryptoError::Decryption));
    }

    #[test]
    fn repo_key_wrap_unwrap_round_trips() {
        let repo_key = RepoContentKey::generate();
        let wrapping_key = KeyWrappingKey::from_device_secret([7_u8; 32], "repo", 1, "device");
        let wrapped = wrap_repo_key(repo_key, wrapping_key, b"grant").unwrap();
        let unwrapped = unwrap_repo_key(&wrapped, wrapping_key, b"grant").unwrap();

        assert_eq!(unwrapped, repo_key);
    }

    #[test]
    fn repo_key_wrap_aad_mismatch_fails_closed() {
        let repo_key = RepoContentKey::generate();
        let wrapping_key = KeyWrappingKey::from_device_secret([7_u8; 32], "repo", 1, "device");
        let wrapped = wrap_repo_key(repo_key, wrapping_key, b"grant").unwrap();

        let err = unwrap_repo_key(&wrapped, wrapping_key, b"other").unwrap_err();

        assert!(matches!(err, CryptoError::Decryption));
    }
}
