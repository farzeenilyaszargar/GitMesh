//! Core GitMesh protocol primitives.
//!
//! This crate owns the small protocol building blocks that every other crate
//! should share: algorithm identifiers, typed CIDs, and deterministic envelope
//! bytes for hashed protocol objects.

use std::fmt;

use thiserror::Error;

pub const GITMESH_PROTOCOL_VERSION: u32 = 0;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u16)]
pub enum HashAlgorithm {
    Blake3_256 = 1,
}

impl HashAlgorithm {
    pub fn hash(self, bytes: &[u8]) -> [u8; 32] {
        match self {
            Self::Blake3_256 => blake3::hash(bytes).into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u16)]
pub enum CidKind {
    ProtocolObject = 1,
    PlainSegment = 2,
    EncryptedSegment = 3,
    Shard = 4,
}

#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Cid {
    kind: CidKind,
    hash_algorithm: HashAlgorithm,
    digest: [u8; 32],
}

impl Cid {
    pub fn new(kind: CidKind, hash_algorithm: HashAlgorithm, payload: &[u8]) -> Self {
        let mut transcript = Vec::with_capacity(32 + payload.len());
        transcript.extend_from_slice(b"gitmesh.v0.cid");
        transcript.extend_from_slice(&(kind as u16).to_be_bytes());
        transcript.extend_from_slice(&(hash_algorithm as u16).to_be_bytes());
        transcript.extend_from_slice(&(payload.len() as u64).to_be_bytes());
        transcript.extend_from_slice(payload);

        Self {
            kind,
            hash_algorithm,
            digest: hash_algorithm.hash(&transcript),
        }
    }

    pub fn from_digest(kind: CidKind, hash_algorithm: HashAlgorithm, digest: [u8; 32]) -> Self {
        Self {
            kind,
            hash_algorithm,
            digest,
        }
    }

    pub fn kind(self) -> CidKind {
        self.kind
    }

    pub fn hash_algorithm(self) -> HashAlgorithm {
        self.hash_algorithm
    }

    pub fn digest(self) -> [u8; 32] {
        self.digest
    }

    pub fn as_hex(self) -> String {
        hex(&self.digest)
    }
}

impl fmt::Debug for Cid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cid")
            .field("kind", &self.kind)
            .field("hash_algorithm", &self.hash_algorithm)
            .field("digest", &self.as_hex())
            .finish()
    }
}

impl fmt::Display for Cid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "gitmesh:v{}:{:?}:{:?}:{}",
            GITMESH_PROTOCOL_VERSION,
            self.kind,
            self.hash_algorithm,
            self.as_hex()
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtocolEnvelope {
    pub domain: String,
    pub version: u32,
    pub hash_algorithm: HashAlgorithm,
    pub body: Vec<u8>,
}

impl ProtocolEnvelope {
    pub fn new(domain: impl Into<String>, body: impl Into<Vec<u8>>) -> Result<Self, CoreError> {
        let domain = domain.into();
        validate_domain(&domain)?;
        Ok(Self {
            domain,
            version: GITMESH_PROTOCOL_VERSION,
            hash_algorithm: HashAlgorithm::Blake3_256,
            body: body.into(),
        })
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, CoreError> {
        validate_domain(&self.domain)?;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"gitmesh-envelope-v0");
        put_string(&mut bytes, &self.domain)?;
        bytes.extend_from_slice(&self.version.to_be_bytes());
        bytes.extend_from_slice(&(self.hash_algorithm as u16).to_be_bytes());
        put_bytes(&mut bytes, &self.body)?;
        Ok(bytes)
    }

    pub fn cid(&self) -> Result<Cid, CoreError> {
        Ok(Cid::new(
            CidKind::ProtocolObject,
            self.hash_algorithm,
            &self.canonical_bytes()?,
        ))
    }
}

#[derive(Debug, Error)]
pub enum CoreError {
    #[error(
        "protocol domain must start with 'gitmesh.' and contain only ASCII letters, digits, '.', '-', or '_'"
    )]
    InvalidDomain,
    #[error("field is too large to encode")]
    FieldTooLarge,
}

pub fn encrypted_segment_cid(ciphertext: &[u8]) -> Cid {
    Cid::new(
        CidKind::EncryptedSegment,
        HashAlgorithm::Blake3_256,
        ciphertext,
    )
}

pub fn shard_cid(segment_cid: Cid, shard_index: usize, bytes: &[u8]) -> Cid {
    let mut transcript = Vec::with_capacity(48 + bytes.len());
    transcript.extend_from_slice(b"gitmesh.v0.shard");
    transcript.extend_from_slice(&segment_cid.digest());
    transcript.extend_from_slice(&(shard_index as u64).to_be_bytes());
    transcript.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    transcript.extend_from_slice(bytes);
    Cid::new(CidKind::Shard, HashAlgorithm::Blake3_256, &transcript)
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn validate_domain(domain: &str) -> Result<(), CoreError> {
    if !domain.starts_with("gitmesh.") {
        return Err(CoreError::InvalidDomain);
    }
    if !domain
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err(CoreError::InvalidDomain);
    }
    Ok(())
}

fn put_string(out: &mut Vec<u8>, value: &str) -> Result<(), CoreError> {
    put_bytes(out, value.as_bytes())
}

fn put_bytes(out: &mut Vec<u8>, value: &[u8]) -> Result<(), CoreError> {
    let len = u32::try_from(value.len()).map_err(|_| CoreError::FieldTooLarge)?;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cids_are_domain_separated_by_kind() {
        let payload = b"same bytes";

        let segment = Cid::new(
            CidKind::EncryptedSegment,
            HashAlgorithm::Blake3_256,
            payload,
        );
        let shard = Cid::new(CidKind::Shard, HashAlgorithm::Blake3_256, payload);

        assert_ne!(segment, shard);
        assert_eq!(segment.kind(), CidKind::EncryptedSegment);
    }

    #[test]
    fn protocol_envelope_has_stable_canonical_bytes() {
        let envelope = ProtocolEnvelope::new("gitmesh.test", b"body").unwrap();

        assert_eq!(
            hex(&envelope.canonical_bytes().unwrap()),
            "6769746d6573682d656e76656c6f70652d76300000000c6769746d6573682e7465737400000000000100000004626f6479"
        );
        assert_eq!(envelope.cid().unwrap(), envelope.cid().unwrap());
    }

    #[test]
    fn protocol_envelope_rejects_bad_domains() {
        assert!(ProtocolEnvelope::new("other.test", b"body").is_err());
        assert!(ProtocolEnvelope::new("gitmesh.bad domain", b"body").is_err());
    }
}
