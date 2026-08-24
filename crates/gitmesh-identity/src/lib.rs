//! GitMesh protocol identity primitives.
//!
//! This crate models account root keys and independently revocable device keys.
//! It intentionally uses established Ed25519 primitives and keeps persistence
//! and recovery policy out of scope for this first slice.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use gitmesh_core::{Cid, CidKind, HashAlgorithm, hex};
use gitmesh_crypto::{
    AeadAlgorithm, KeyWrappingKey, RepoContentKey, WrappedRepoKey, unwrap_repo_key, wrap_repo_key,
};
use rand::RngCore;
use thiserror::Error;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AccountId(Cid);

impl AccountId {
    pub fn from_verifying_key(key: &VerifyingKey) -> Self {
        Self(Cid::new(
            CidKind::ProtocolObject,
            HashAlgorithm::Blake3_256,
            key.as_bytes(),
        ))
    }

    pub fn as_cid(&self) -> Cid {
        self.0
    }

    pub fn from_protocol_cid_text(value: &str) -> Result<Self, IdentityError> {
        Ok(Self(parse_protocol_cid(value)?))
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct DeviceId(Cid);

impl DeviceId {
    pub fn from_verifying_key(key: &VerifyingKey) -> Self {
        Self(Cid::new(
            CidKind::ProtocolObject,
            HashAlgorithm::Blake3_256,
            key.as_bytes(),
        ))
    }

    pub fn as_cid(&self) -> Cid {
        self.0
    }

    pub fn from_protocol_cid_text(value: &str) -> Result<Self, IdentityError> {
        Ok(Self(parse_protocol_cid(value)?))
    }
}

#[derive(Clone)]
pub struct AccountRootKey {
    signing_key: SigningKey,
}

impl AccountRootKey {
    pub fn generate() -> Self {
        Self {
            signing_key: SigningKey::from_bytes(&random_seed()),
        }
    }

    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self {
            signing_key: SigningKey::from_bytes(&seed),
        }
    }

    pub fn seed_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    pub fn account_id(&self) -> AccountId {
        AccountId::from_verifying_key(&self.verifying_key())
    }

    pub fn certify_device(
        &self,
        device: &DeviceKey,
        label: impl Into<String>,
    ) -> DeviceCertificate {
        let label = label.into();
        let account_key = self.verifying_key();
        let device_key = device.verifying_key();
        let transcript = certificate_transcript(&account_key, &device_key, &label);
        let signature = self.signing_key.sign(&transcript);

        DeviceCertificate {
            account_id: self.account_id(),
            device_id: device.device_id(),
            account_verifying_key: account_key.to_bytes(),
            device_verifying_key: device_key.to_bytes(),
            label,
            signature: signature.to_bytes(),
        }
    }

    pub fn grant_repo_key_to_device(
        &self,
        repo_id: impl Into<String>,
        epoch: u64,
        repo_key: RepoContentKey,
        recipient: &DeviceKey,
        recipient_certificate: &DeviceCertificate,
    ) -> Result<RepoKeyGrant, IdentityError> {
        recipient_certificate.verify()?;
        if recipient_certificate.account_id != self.account_id()
            || recipient_certificate.device_id != recipient.device_id()
        {
            return Err(IdentityError::IdentityMismatch);
        }
        let repo_id = repo_id.into();
        validate_repo_id_text(&repo_id)?;
        let wrapping_key = KeyWrappingKey::from_device_secret(
            recipient.seed_bytes(),
            &repo_id,
            epoch,
            &recipient_certificate.device_id.as_cid().to_string(),
        );
        let wrapped_key = wrap_repo_key(
            repo_key,
            wrapping_key,
            &repo_key_grant_aad(&repo_id, epoch, &recipient_certificate.device_id),
        )?;
        let unsigned = RepoKeyGrant {
            repo_id,
            epoch,
            account_id: self.account_id(),
            recipient_device_id: recipient_certificate.device_id.clone(),
            recipient_device_verifying_key: recipient_certificate.device_verifying_key,
            wrapping_algorithm: wrapped_key.algorithm,
            wrapping_nonce: wrapped_key.nonce,
            wrapped_key_ciphertext: wrapped_key.ciphertext,
            signer_account_verifying_key: self.verifying_key().to_bytes(),
            signature: [0_u8; 64],
        };
        let signature = self.signing_key.sign(&unsigned.signing_transcript());
        Ok(RepoKeyGrant {
            signature: signature.to_bytes(),
            ..unsigned
        })
    }
}

#[derive(Clone)]
pub struct DeviceKey {
    signing_key: SigningKey,
}

impl DeviceKey {
    pub fn generate() -> Self {
        Self {
            signing_key: SigningKey::from_bytes(&random_seed()),
        }
    }

    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self {
            signing_key: SigningKey::from_bytes(&seed),
        }
    }

    pub fn seed_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    pub fn device_id(&self) -> DeviceId {
        DeviceId::from_verifying_key(&self.verifying_key())
    }

    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.signing_key.sign(message).to_bytes()
    }

    pub fn unwrap_repo_key_grant(
        &self,
        grant: &RepoKeyGrant,
    ) -> Result<RepoContentKey, IdentityError> {
        grant.verify()?;
        if grant.recipient_device_id != self.device_id() {
            return Err(IdentityError::IdentityMismatch);
        }
        let wrapping_key = KeyWrappingKey::from_device_secret(
            self.seed_bytes(),
            &grant.repo_id,
            grant.epoch,
            &grant.recipient_device_id.as_cid().to_string(),
        );
        let wrapped = WrappedRepoKey {
            algorithm: grant.wrapping_algorithm,
            nonce: grant.wrapping_nonce,
            ciphertext: grant.wrapped_key_ciphertext.clone(),
        };
        Ok(unwrap_repo_key(
            &wrapped,
            wrapping_key,
            &repo_key_grant_aad(&grant.repo_id, grant.epoch, &grant.recipient_device_id),
        )?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceCertificate {
    pub account_id: AccountId,
    pub device_id: DeviceId,
    pub account_verifying_key: [u8; 32],
    pub device_verifying_key: [u8; 32],
    pub label: String,
    pub signature: [u8; 64],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepoKeyGrant {
    pub repo_id: String,
    pub epoch: u64,
    pub account_id: AccountId,
    pub recipient_device_id: DeviceId,
    pub recipient_device_verifying_key: [u8; 32],
    pub wrapping_algorithm: AeadAlgorithm,
    pub wrapping_nonce: [u8; 24],
    pub wrapped_key_ciphertext: Vec<u8>,
    pub signer_account_verifying_key: [u8; 32],
    pub signature: [u8; 64],
}

impl RepoKeyGrant {
    pub const WIRE_FIELD_COUNT: usize = 10;

    pub fn grant_id(&self) -> Cid {
        let mut transcript = self.signing_transcript();
        transcript.extend_from_slice(&self.signature);
        Cid::new(
            CidKind::ProtocolObject,
            HashAlgorithm::Blake3_256,
            &transcript,
        )
    }

    pub fn verify(&self) -> Result<(), IdentityError> {
        validate_repo_id_text(&self.repo_id)?;
        let signer_key = VerifyingKey::from_bytes(&self.signer_account_verifying_key)
            .map_err(|_| IdentityError::InvalidKey)?;
        if self.account_id != AccountId::from_verifying_key(&signer_key) {
            return Err(IdentityError::IdentityMismatch);
        }
        let recipient_key = VerifyingKey::from_bytes(&self.recipient_device_verifying_key)
            .map_err(|_| IdentityError::InvalidKey)?;
        if self.recipient_device_id != DeviceId::from_verifying_key(&recipient_key) {
            return Err(IdentityError::IdentityMismatch);
        }
        let signature = Signature::from_bytes(&self.signature);
        signer_key
            .verify(&self.signing_transcript(), &signature)
            .map_err(|_| IdentityError::InvalidSignature)
    }

    pub fn signing_transcript(&self) -> Vec<u8> {
        let mut transcript = Vec::new();
        transcript.extend_from_slice(b"gitmesh.v0.repo-key-grant");
        put_transcript_field(&mut transcript, self.repo_id.as_bytes());
        transcript.extend_from_slice(&self.epoch.to_be_bytes());
        put_transcript_field(
            &mut transcript,
            self.account_id.as_cid().to_string().as_bytes(),
        );
        put_transcript_field(
            &mut transcript,
            self.recipient_device_id.as_cid().to_string().as_bytes(),
        );
        put_transcript_field(&mut transcript, &self.recipient_device_verifying_key);
        transcript.extend_from_slice(&(self.wrapping_algorithm as u16).to_be_bytes());
        put_transcript_field(&mut transcript, &self.wrapping_nonce);
        put_transcript_field(&mut transcript, &self.wrapped_key_ciphertext);
        put_transcript_field(&mut transcript, &self.signer_account_verifying_key);
        transcript
    }

    pub fn to_wire_string(&self) -> Result<String, IdentityError> {
        self.verify()?;
        Ok(format!(
            "{} {} {} {} {} {} {} {} {} {}",
            self.repo_id,
            self.epoch,
            self.account_id.as_cid(),
            self.recipient_device_id.as_cid(),
            hex(&self.recipient_device_verifying_key),
            encode_aead_algorithm(self.wrapping_algorithm),
            hex(&self.wrapping_nonce),
            hex(&self.wrapped_key_ciphertext),
            hex(&self.signer_account_verifying_key),
            hex(&self.signature)
        ))
    }

    pub fn from_wire_fields(fields: &[&str]) -> Result<Self, IdentityError> {
        if fields.len() != Self::WIRE_FIELD_COUNT {
            return Err(IdentityError::InvalidGrantStore);
        }
        let grant = Self {
            repo_id: fields[0].to_string(),
            epoch: parse_u64(fields[1])?,
            account_id: AccountId(parse_protocol_cid(fields[2])?),
            recipient_device_id: DeviceId(parse_protocol_cid(fields[3])?),
            recipient_device_verifying_key: decode_fixed_hex::<32>(fields[4])?,
            wrapping_algorithm: parse_aead_algorithm(fields[5])?,
            wrapping_nonce: decode_fixed_hex::<24>(fields[6])?,
            wrapped_key_ciphertext: decode_hex(fields[7])?,
            signer_account_verifying_key: decode_fixed_hex::<32>(fields[8])?,
            signature: decode_fixed_hex::<64>(fields[9])?,
        };
        grant.verify()?;
        Ok(grant)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RepoKeyGrantStore {
    grants: BTreeMap<(String, u64, DeviceId), RepoKeyGrant>,
    revoked_devices: BTreeMap<DeviceId, u64>,
}

impl RepoKeyGrantStore {
    pub fn insert_grant(&mut self, grant: RepoKeyGrant) -> Result<(), IdentityError> {
        grant.verify()?;
        self.grants.insert(
            (
                grant.repo_id.clone(),
                grant.epoch,
                grant.recipient_device_id.clone(),
            ),
            grant,
        );
        Ok(())
    }

    pub fn revoke_device_from_epoch(
        &mut self,
        device_id: DeviceId,
        effective_epoch: u64,
    ) -> Result<(), IdentityError> {
        if effective_epoch == 0 {
            return Err(IdentityError::InvalidEpoch);
        }
        self.revoked_devices
            .entry(device_id)
            .and_modify(|epoch| *epoch = (*epoch).min(effective_epoch))
            .or_insert(effective_epoch);
        Ok(())
    }

    pub fn latest_epoch(&self, repo_id: &str) -> Option<u64> {
        self.grants
            .keys()
            .filter(|(grant_repo_id, _, _)| grant_repo_id == repo_id)
            .map(|(_, epoch, _)| *epoch)
            .max()
    }

    pub fn active_grants_for_epoch(&self, repo_id: &str, epoch: u64) -> Vec<&RepoKeyGrant> {
        self.grants
            .iter()
            .filter(|((grant_repo_id, grant_epoch, device_id), _)| {
                grant_repo_id == repo_id
                    && *grant_epoch == epoch
                    && !self.is_device_revoked_for_epoch(device_id, epoch)
            })
            .map(|(_, grant)| grant)
            .collect()
    }

    pub fn grants_for_repo(&self, repo_id: &str) -> Vec<&RepoKeyGrant> {
        self.grants
            .iter()
            .filter(|((grant_repo_id, _, _), _)| grant_repo_id == repo_id)
            .map(|(_, grant)| grant)
            .collect()
    }

    pub fn grants_for_repo_epoch(&self, repo_id: &str, epoch: u64) -> Vec<&RepoKeyGrant> {
        self.grants
            .iter()
            .filter(|((grant_repo_id, grant_epoch, _), _)| {
                grant_repo_id == repo_id && *grant_epoch == epoch
            })
            .map(|(_, grant)| grant)
            .collect()
    }

    pub fn grant_for_device(
        &self,
        repo_id: &str,
        epoch: u64,
        device_id: &DeviceId,
    ) -> Option<&RepoKeyGrant> {
        if self.is_device_revoked_for_epoch(device_id, epoch) {
            return None;
        }
        self.grants
            .get(&(repo_id.to_string(), epoch, device_id.clone()))
    }

    pub fn devices_with_active_grants(&self, repo_id: &str, epoch: u64) -> BTreeSet<DeviceId> {
        self.active_grants_for_epoch(repo_id, epoch)
            .into_iter()
            .map(|grant| grant.recipient_device_id.clone())
            .collect()
    }

    pub fn grant_count(&self) -> usize {
        self.grants.len()
    }

    pub fn revoked_device_count(&self) -> usize {
        self.revoked_devices.len()
    }

    pub fn is_device_revoked_for_epoch(&self, device_id: &DeviceId, epoch: u64) -> bool {
        self.revoked_devices
            .get(device_id)
            .is_some_and(|effective_epoch| epoch >= *effective_epoch)
    }

    pub fn to_snapshot(&self) -> Result<String, IdentityError> {
        let mut snapshot = String::from("gitmesh-repo-key-grant-store-v0\n");
        for grant in self.grants.values() {
            grant.verify()?;
            snapshot.push_str(&format!(
                "grant\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                grant.repo_id,
                grant.epoch,
                grant.account_id.as_cid(),
                grant.recipient_device_id.as_cid(),
                hex(&grant.recipient_device_verifying_key),
                encode_aead_algorithm(grant.wrapping_algorithm),
                hex(&grant.wrapping_nonce),
                hex(&grant.wrapped_key_ciphertext),
                hex(&grant.signer_account_verifying_key),
                hex(&grant.signature)
            ));
        }
        for (device_id, epoch) in &self.revoked_devices {
            snapshot.push_str(&format!(
                "revoked_device\t{}\t{}\n",
                device_id.as_cid(),
                epoch
            ));
        }
        Ok(snapshot)
    }

    pub fn from_snapshot(text: &str) -> Result<Self, IdentityError> {
        let mut lines = text.lines();
        if lines.next() != Some("gitmesh-repo-key-grant-store-v0") {
            return Err(IdentityError::InvalidGrantStore);
        }
        let mut store = Self::default();
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            let parts = line.split('\t').collect::<Vec<_>>();
            match parts.first().copied() {
                Some("grant") => {
                    if parts.len() != 11 {
                        return Err(IdentityError::InvalidGrantStore);
                    }
                    let grant = RepoKeyGrant {
                        repo_id: parts[1].to_string(),
                        epoch: parse_u64(parts[2])?,
                        account_id: AccountId(parse_protocol_cid(parts[3])?),
                        recipient_device_id: DeviceId(parse_protocol_cid(parts[4])?),
                        recipient_device_verifying_key: decode_fixed_hex::<32>(parts[5])?,
                        wrapping_algorithm: parse_aead_algorithm(parts[6])?,
                        wrapping_nonce: decode_fixed_hex::<24>(parts[7])?,
                        wrapped_key_ciphertext: decode_hex(parts[8])?,
                        signer_account_verifying_key: decode_fixed_hex::<32>(parts[9])?,
                        signature: decode_fixed_hex::<64>(parts[10])?,
                    };
                    store.insert_grant(grant)?;
                }
                Some("revoked_device") => {
                    if parts.len() != 3 {
                        return Err(IdentityError::InvalidGrantStore);
                    }
                    store.revoke_device_from_epoch(
                        DeviceId(parse_protocol_cid(parts[1])?),
                        parse_u64(parts[2])?,
                    )?;
                }
                _ => return Err(IdentityError::InvalidGrantStore),
            }
        }
        Ok(store)
    }

    pub fn save_to_path(&self, path: impl AsRef<Path>) -> Result<(), IdentityError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp_path = path.with_extension("tmp");
        fs::write(&tmp_path, self.to_snapshot()?)?;
        fs::rename(tmp_path, path)?;
        Ok(())
    }

    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, IdentityError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }
        Self::from_snapshot(&fs::read_to_string(path)?)
    }
}

impl DeviceCertificate {
    pub fn from_key_bytes(
        label: impl Into<String>,
        account_verifying_key: [u8; 32],
        device_verifying_key: [u8; 32],
        signature: [u8; 64],
    ) -> Result<Self, IdentityError> {
        let account_key = VerifyingKey::from_bytes(&account_verifying_key)
            .map_err(|_| IdentityError::InvalidKey)?;
        let device_key = VerifyingKey::from_bytes(&device_verifying_key)
            .map_err(|_| IdentityError::InvalidKey)?;
        Ok(Self {
            account_id: AccountId::from_verifying_key(&account_key),
            device_id: DeviceId::from_verifying_key(&device_key),
            account_verifying_key,
            device_verifying_key,
            label: label.into(),
            signature,
        })
    }

    pub fn verify(&self) -> Result<(), IdentityError> {
        let account_key = VerifyingKey::from_bytes(&self.account_verifying_key)
            .map_err(|_| IdentityError::InvalidKey)?;
        let device_key = VerifyingKey::from_bytes(&self.device_verifying_key)
            .map_err(|_| IdentityError::InvalidKey)?;
        if self.account_id != AccountId::from_verifying_key(&account_key) {
            return Err(IdentityError::IdentityMismatch);
        }
        if self.device_id != DeviceId::from_verifying_key(&device_key) {
            return Err(IdentityError::IdentityMismatch);
        }
        let signature = Signature::from_bytes(&self.signature);
        account_key
            .verify(
                &certificate_transcript(&account_key, &device_key, &self.label),
                &signature,
            )
            .map_err(|_| IdentityError::InvalidSignature)
    }

    pub fn verify_device_signature(
        &self,
        message: &[u8],
        signature: &[u8; 64],
    ) -> Result<(), IdentityError> {
        self.verify()?;
        let device_key = VerifyingKey::from_bytes(&self.device_verifying_key)
            .map_err(|_| IdentityError::InvalidKey)?;
        let signature = Signature::from_bytes(signature);
        device_key
            .verify(message, &signature)
            .map_err(|_| IdentityError::InvalidSignature)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DevIdentity {
    pub account_id: AccountId,
    pub device_id: DeviceId,
    pub certificate: DeviceCertificate,
}

impl DevIdentity {
    pub fn generate(label: impl Into<String>) -> Self {
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let certificate = account.certify_device(&device, label);
        Self {
            account_id: account.account_id(),
            device_id: device.device_id(),
            certificate,
        }
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum IdentityError {
    #[error("invalid Ed25519 verifying key")]
    InvalidKey,
    #[error("identity does not match verifying key")]
    IdentityMismatch,
    #[error("invalid device certificate signature")]
    InvalidSignature,
    #[error("invalid repository id text")]
    InvalidRepoId,
    #[error("invalid key epoch")]
    InvalidEpoch,
    #[error("invalid repo key grant store")]
    InvalidGrantStore,
    #[error("I/O failed: {0}")]
    Io(String),
    #[error("crypto failed: {0}")]
    Crypto(#[from] gitmesh_crypto::CryptoError),
}

impl From<std::io::Error> for IdentityError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

fn random_seed() -> [u8; 32] {
    let mut seed = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut seed);
    seed
}

fn certificate_transcript(
    account_key: &VerifyingKey,
    device_key: &VerifyingKey,
    label: &str,
) -> Vec<u8> {
    let mut transcript = Vec::new();
    transcript.extend_from_slice(b"gitmesh.v0.device-certificate");
    transcript.extend_from_slice(account_key.as_bytes());
    transcript.extend_from_slice(device_key.as_bytes());
    transcript.extend_from_slice(&(label.len() as u64).to_be_bytes());
    transcript.extend_from_slice(label.as_bytes());
    transcript
}

fn repo_key_grant_aad(repo_id: &str, epoch: u64, device_id: &DeviceId) -> Vec<u8> {
    let mut aad = Vec::new();
    aad.extend_from_slice(b"gitmesh.v0.repo-key-grant-wrap");
    put_transcript_field(&mut aad, repo_id.as_bytes());
    aad.extend_from_slice(&epoch.to_be_bytes());
    put_transcript_field(&mut aad, device_id.as_cid().to_string().as_bytes());
    aad
}

fn put_transcript_field(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(&(value.len() as u64).to_be_bytes());
    out.extend_from_slice(value);
}

fn validate_repo_id_text(value: &str) -> Result<(), IdentityError> {
    if value.is_empty()
        || value.contains(char::is_whitespace)
        || value.contains('\t')
        || value.contains('\n')
        || value.contains('\r')
    {
        return Err(IdentityError::InvalidRepoId);
    }
    Ok(())
}

fn encode_aead_algorithm(algorithm: AeadAlgorithm) -> &'static str {
    match algorithm {
        AeadAlgorithm::XChaCha20Poly1305 => "xchacha20poly1305",
    }
}

fn parse_aead_algorithm(value: &str) -> Result<AeadAlgorithm, IdentityError> {
    match value {
        "xchacha20poly1305" => Ok(AeadAlgorithm::XChaCha20Poly1305),
        _ => Err(IdentityError::InvalidGrantStore),
    }
}

fn parse_u64(value: &str) -> Result<u64, IdentityError> {
    value
        .parse::<u64>()
        .map_err(|_| IdentityError::InvalidGrantStore)
}

fn parse_protocol_cid(value: &str) -> Result<Cid, IdentityError> {
    let cid = value
        .parse::<Cid>()
        .map_err(|_| IdentityError::InvalidGrantStore)?;
    if cid.kind() != CidKind::ProtocolObject || cid.hash_algorithm() != HashAlgorithm::Blake3_256 {
        return Err(IdentityError::InvalidGrantStore);
    }
    Ok(cid)
}

fn decode_fixed_hex<const N: usize>(value: &str) -> Result<[u8; N], IdentityError> {
    let bytes = decode_hex(value)?;
    bytes
        .try_into()
        .map_err(|_| IdentityError::InvalidGrantStore)
}

fn decode_hex(value: &str) -> Result<Vec<u8>, IdentityError> {
    if !value.len().is_multiple_of(2) {
        return Err(IdentityError::InvalidGrantStore);
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|chunk| {
            let high = hex_nibble(chunk[0]).ok_or(IdentityError::InvalidGrantStore)?;
            let low = hex_nibble(chunk[1]).ok_or(IdentityError::InvalidGrantStore)?;
            Ok((high << 4) | low)
        })
        .collect()
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub fn short_id(cid: Cid) -> String {
    hex(&cid.digest()[..8])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_device_certificate_verifies() {
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let cert = account.certify_device(&device, "laptop");

        cert.verify().unwrap();
        DeviceCertificate::from_key_bytes(
            cert.label.clone(),
            cert.account_verifying_key,
            cert.device_verifying_key,
            cert.signature,
        )
        .unwrap()
        .verify()
        .unwrap();
    }

    #[test]
    fn tampered_device_certificate_fails() {
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let mut cert = account.certify_device(&device, "laptop");
        cert.label = "other".to_string();

        assert_eq!(cert.verify().unwrap_err(), IdentityError::InvalidSignature);
    }

    #[test]
    fn device_signature_verifies_through_certificate() {
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let cert = account.certify_device(&device, "laptop");
        let signature = device.sign(b"gitmesh message");

        cert.verify_device_signature(b"gitmesh message", &signature)
            .unwrap();
        assert_eq!(
            cert.verify_device_signature(b"other message", &signature)
                .unwrap_err(),
            IdentityError::InvalidSignature
        );
    }

    #[test]
    fn repo_key_grant_verifies_and_unwraps_for_recipient_device() {
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let cert = account.certify_device(&device, "laptop");
        let repo_key = RepoContentKey::generate();

        let grant = account
            .grant_repo_key_to_device("repo:farzeen/gitmesh", 1, repo_key, &device, &cert)
            .unwrap();
        let unwrapped = device.unwrap_repo_key_grant(&grant).unwrap();

        grant.verify().unwrap();
        assert_eq!(unwrapped, repo_key);
    }

    #[test]
    fn repo_key_grant_rejects_wrong_device_unwrap() {
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let cert = account.certify_device(&device, "laptop");
        let other_device = DeviceKey::generate();
        let repo_key = RepoContentKey::generate();
        let grant = account
            .grant_repo_key_to_device("repo:farzeen/gitmesh", 1, repo_key, &device, &cert)
            .unwrap();

        let err = other_device.unwrap_repo_key_grant(&grant).unwrap_err();

        assert_eq!(err, IdentityError::IdentityMismatch);
    }

    #[test]
    fn repo_key_grant_rejects_tampering() {
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let cert = account.certify_device(&device, "laptop");
        let repo_key = RepoContentKey::generate();
        let mut grant = account
            .grant_repo_key_to_device("repo:farzeen/gitmesh", 1, repo_key, &device, &cert)
            .unwrap();
        grant.epoch = 2;

        let err = grant.verify().unwrap_err();

        assert_eq!(err, IdentityError::InvalidSignature);
    }

    #[test]
    fn repo_key_grant_wire_format_round_trips_and_verifies() {
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let cert = account.certify_device(&device, "laptop");
        let grant = account
            .grant_repo_key_to_device(
                "repo:farzeen/gitmesh",
                3,
                RepoContentKey::generate(),
                &device,
                &cert,
            )
            .unwrap();

        let wire = grant.to_wire_string().unwrap();
        let fields = wire.split_whitespace().collect::<Vec<_>>();
        let restored = RepoKeyGrant::from_wire_fields(&fields).unwrap();

        assert_eq!(restored, grant);
        restored.verify().unwrap();
    }

    #[test]
    fn repo_key_grant_store_tracks_latest_epoch_and_active_devices() {
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let cert = account.certify_device(&device, "laptop");
        let grant_v1 = account
            .grant_repo_key_to_device(
                "repo:farzeen/gitmesh",
                1,
                RepoContentKey::generate(),
                &device,
                &cert,
            )
            .unwrap();
        let grant_v2 = account
            .grant_repo_key_to_device(
                "repo:farzeen/gitmesh",
                2,
                RepoContentKey::generate(),
                &device,
                &cert,
            )
            .unwrap();
        let mut store = RepoKeyGrantStore::default();

        store.insert_grant(grant_v1).unwrap();
        store.insert_grant(grant_v2).unwrap();

        assert_eq!(store.grant_count(), 2);
        assert_eq!(store.latest_epoch("repo:farzeen/gitmesh"), Some(2));
        assert_eq!(
            store
                .devices_with_active_grants("repo:farzeen/gitmesh", 2)
                .len(),
            1
        );
        assert!(
            store
                .grant_for_device("repo:farzeen/gitmesh", 2, &cert.device_id)
                .is_some()
        );
    }

    #[test]
    fn repo_key_grant_store_revokes_future_epochs_only() {
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let cert = account.certify_device(&device, "laptop");
        let grant_v1 = account
            .grant_repo_key_to_device(
                "repo:farzeen/gitmesh",
                1,
                RepoContentKey::generate(),
                &device,
                &cert,
            )
            .unwrap();
        let grant_v2 = account
            .grant_repo_key_to_device(
                "repo:farzeen/gitmesh",
                2,
                RepoContentKey::generate(),
                &device,
                &cert,
            )
            .unwrap();
        let mut store = RepoKeyGrantStore::default();
        store.insert_grant(grant_v1).unwrap();
        store.insert_grant(grant_v2).unwrap();

        store
            .revoke_device_from_epoch(cert.device_id.clone(), 2)
            .unwrap();

        assert_eq!(store.revoked_device_count(), 1);
        assert!(
            store
                .grant_for_device("repo:farzeen/gitmesh", 1, &cert.device_id)
                .is_some()
        );
        assert!(
            store
                .grant_for_device("repo:farzeen/gitmesh", 2, &cert.device_id)
                .is_none()
        );
        assert_eq!(
            store
                .active_grants_for_epoch("repo:farzeen/gitmesh", 2)
                .len(),
            0
        );
    }

    #[test]
    fn repo_key_grant_store_snapshot_round_trips() {
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let cert = account.certify_device(&device, "laptop");
        let grant = account
            .grant_repo_key_to_device(
                "repo:farzeen/gitmesh",
                7,
                RepoContentKey::generate(),
                &device,
                &cert,
            )
            .unwrap();
        let mut store = RepoKeyGrantStore::default();
        store.insert_grant(grant).unwrap();
        store
            .revoke_device_from_epoch(cert.device_id.clone(), 8)
            .unwrap();

        let restored = RepoKeyGrantStore::from_snapshot(&store.to_snapshot().unwrap()).unwrap();

        assert_eq!(restored.grant_count(), 1);
        assert_eq!(restored.revoked_device_count(), 1);
        assert_eq!(restored.latest_epoch("repo:farzeen/gitmesh"), Some(7));
        assert!(
            restored
                .grant_for_device("repo:farzeen/gitmesh", 7, &cert.device_id)
                .is_some()
        );
        assert!(restored.is_device_revoked_for_epoch(&cert.device_id, 8));
    }
}
