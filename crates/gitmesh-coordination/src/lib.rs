//! Repository-level coordination primitives.
//!
//! This crate models the GitMesh rule that mutable refs advance only through
//! idempotent compare-and-swap transactions.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::str::FromStr;

use gitmesh_core::{Cid, CidKind, HashAlgorithm};
use gitmesh_git::GitSha1Oid;
use gitmesh_identity::{AccountId, DeviceCertificate, DeviceId, DeviceKey, IdentityError};
use thiserror::Error;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RepoId(Cid);

impl RepoId {
    pub fn new(seed: &[u8]) -> Self {
        Self(Cid::new(
            CidKind::ProtocolObject,
            HashAlgorithm::Blake3_256,
            seed,
        ))
    }

    pub fn cid(&self) -> Cid {
        self.0
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RefName(String);

impl RefName {
    pub fn new(value: impl Into<String>) -> Result<Self, CoordinationError> {
        let value = value.into();
        validate_ref_name(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TransactionId(String);

impl TransactionId {
    pub fn new(value: impl Into<String>) -> Result<Self, CoordinationError> {
        let value = value.into();
        if value.is_empty()
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(CoordinationError::InvalidTransactionId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RefUpdate {
    pub repo_id: RepoId,
    pub ref_name: RefName,
    pub expected_old_oid: Option<GitSha1Oid>,
    pub new_oid: Option<GitSha1Oid>,
    pub force: bool,
    pub policy_epoch: u64,
    pub transaction_id: TransactionId,
    pub signer: String,
}

impl RefUpdate {
    pub fn operation_fingerprint(&self) -> String {
        format!(
            "{}:{}:{:?}:{:?}:{}:{}:{}",
            self.repo_id.cid(),
            self.ref_name.as_str(),
            self.expected_old_oid,
            self.new_oid,
            self.force,
            self.policy_epoch,
            self.signer
        )
    }

    pub fn signing_transcript(&self) -> Vec<u8> {
        let mut transcript = Vec::new();
        transcript.extend_from_slice(b"gitmesh.v0.ref-update");
        put_transcript_field(&mut transcript, self.repo_id.cid().to_string().as_bytes());
        put_transcript_field(&mut transcript, self.ref_name.as_str().as_bytes());
        put_transcript_field(
            &mut transcript,
            format_optional_oid(self.expected_old_oid).as_bytes(),
        );
        put_transcript_field(
            &mut transcript,
            format_optional_oid(self.new_oid).as_bytes(),
        );
        transcript.push(u8::from(self.force));
        transcript.extend_from_slice(&self.policy_epoch.to_be_bytes());
        put_transcript_field(&mut transcript, self.transaction_id.as_str().as_bytes());
        put_transcript_field(&mut transcript, self.signer.as_bytes());
        transcript
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignedRefUpdate {
    pub update: RefUpdate,
    pub certificate: DeviceCertificate,
    pub signature: [u8; 64],
}

impl SignedRefUpdate {
    pub fn new(update: RefUpdate, certificate: DeviceCertificate, signature: [u8; 64]) -> Self {
        Self {
            update,
            certificate,
            signature,
        }
    }

    pub fn verify(self) -> Result<RefUpdate, CoordinationError> {
        Ok(self.verify_with_identity()?.update)
    }

    pub fn verify_with_identity(self) -> Result<VerifiedRefUpdate, CoordinationError> {
        let signer = self.certificate.device_id.as_cid().to_string();
        if self.update.signer != signer {
            return Err(CoordinationError::SignerMismatch);
        }
        self.certificate
            .verify_device_signature(&self.update.signing_transcript(), &self.signature)
            .map_err(CoordinationError::Identity)?;
        Ok(VerifiedRefUpdate {
            update: self.update,
            account_id: self.certificate.account_id,
            device_id: self.certificate.device_id,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedRefUpdate {
    pub update: RefUpdate,
    pub account_id: AccountId,
    pub device_id: DeviceId,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RepoPolicy {
    require_signed_refs: bool,
    writer_accounts: BTreeSet<String>,
    writer_devices: BTreeSet<String>,
    force_push_accounts: BTreeSet<String>,
    force_push_devices: BTreeSet<String>,
    protected_refs: BTreeSet<RefName>,
}

impl RepoPolicy {
    pub fn require_signed_refs(&self) -> bool {
        self.require_signed_refs
    }

    pub fn set_require_signed_refs(&mut self, value: bool) {
        self.require_signed_refs = value;
    }

    pub fn grant_writer_account(&mut self, account_id: &AccountId) {
        self.writer_accounts.insert(account_id.as_cid().to_string());
    }

    pub fn grant_writer_account_id_string(&mut self, account_id: &str) {
        self.writer_accounts.insert(account_id.to_string());
    }

    pub fn grant_writer_device(&mut self, device_id: &DeviceId) {
        self.writer_devices.insert(device_id.as_cid().to_string());
    }

    pub fn grant_force_push_account(&mut self, account_id: &AccountId) {
        self.force_push_accounts
            .insert(account_id.as_cid().to_string());
    }

    pub fn grant_force_push_account_id_string(&mut self, account_id: &str) {
        self.force_push_accounts.insert(account_id.to_string());
    }

    pub fn grant_force_push_device(&mut self, device_id: &DeviceId) {
        self.force_push_devices
            .insert(device_id.as_cid().to_string());
    }

    pub fn protect_ref(&mut self, ref_name: RefName) {
        self.protected_refs.insert(ref_name);
    }

    pub fn protected_ref_count(&self) -> usize {
        self.protected_refs.len()
    }

    pub fn writer_count(&self) -> usize {
        self.writer_accounts.len() + self.writer_devices.len()
    }

    pub fn force_pusher_count(&self) -> usize {
        self.force_push_accounts.len() + self.force_push_devices.len()
    }

    pub fn authorize_ref_update(
        &self,
        update: &RefUpdate,
        actor: RefUpdateActor<'_>,
    ) -> Result<(), CoordinationError> {
        let actor = match actor {
            RefUpdateActor::Unsigned if self.require_signed_refs => {
                return Err(CoordinationError::UnsignedRefUpdateDenied);
            }
            RefUpdateActor::Unsigned
                if !self.writer_accounts.is_empty() || !self.writer_devices.is_empty() =>
            {
                return Err(CoordinationError::UnsignedRefUpdateDenied);
            }
            RefUpdateActor::Unsigned => return Ok(()),
            RefUpdateActor::Signed(actor) => actor,
        };

        if !self.writer_accounts.is_empty() || !self.writer_devices.is_empty() {
            let writer_allowed = self.writer_accounts.contains(actor.account_id)
                || self.writer_devices.contains(actor.device_id);
            if !writer_allowed {
                return Err(CoordinationError::RefUpdateNotAuthorized);
            }
        }

        if self.protected_refs.contains(&update.ref_name)
            && (update.force || update.new_oid.is_none())
        {
            let force_allowed = self.force_push_accounts.contains(actor.account_id)
                || self.force_push_devices.contains(actor.device_id);
            if !force_allowed {
                return Err(CoordinationError::ProtectedRefDenied);
            }
        }

        Ok(())
    }

    pub fn to_snapshot(&self) -> Result<String, CoordinationError> {
        let mut snapshot = String::from("gitmesh-repo-policy-v0\n");
        snapshot.push_str(&format!(
            "require_signed_refs\t{}\n",
            format_bool(self.require_signed_refs)
        ));
        for account in &self.writer_accounts {
            validate_snapshot_field(account)?;
            snapshot.push_str(&format!("writer_account\t{account}\n"));
        }
        for device in &self.writer_devices {
            validate_snapshot_field(device)?;
            snapshot.push_str(&format!("writer_device\t{device}\n"));
        }
        for account in &self.force_push_accounts {
            validate_snapshot_field(account)?;
            snapshot.push_str(&format!("force_push_account\t{account}\n"));
        }
        for device in &self.force_push_devices {
            validate_snapshot_field(device)?;
            snapshot.push_str(&format!("force_push_device\t{device}\n"));
        }
        for ref_name in &self.protected_refs {
            snapshot.push_str(&format!("protected_ref\t{}\n", ref_name.as_str()));
        }
        Ok(snapshot)
    }

    pub fn from_snapshot(text: &str) -> Result<Self, CoordinationError> {
        let mut lines = text.lines();
        if lines.next() != Some("gitmesh-repo-policy-v0") {
            return Err(CoordinationError::InvalidSnapshot);
        }
        let mut policy = Self::default();
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            let parts = line.split('\t').collect::<Vec<_>>();
            match parts.as_slice() {
                ["require_signed_refs", value] => policy.require_signed_refs = parse_bool(value)?,
                ["writer_account", value] => {
                    policy.writer_accounts.insert(snapshot_field(value)?);
                }
                ["writer_device", value] => {
                    policy.writer_devices.insert(snapshot_field(value)?);
                }
                ["force_push_account", value] => {
                    policy.force_push_accounts.insert(snapshot_field(value)?);
                }
                ["force_push_device", value] => {
                    policy.force_push_devices.insert(snapshot_field(value)?);
                }
                ["protected_ref", value] => {
                    policy.protected_refs.insert(RefName::new(*value)?);
                }
                _ => return Err(CoordinationError::InvalidSnapshot),
            }
        }
        Ok(policy)
    }

    pub fn save_to_path(&self, path: impl AsRef<Path>) -> Result<(), CoordinationError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp_path = path.with_extension("tmp");
        fs::write(&tmp_path, self.to_snapshot()?)?;
        fs::rename(tmp_path, path)?;
        Ok(())
    }

    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, CoordinationError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }
        Self::from_snapshot(&fs::read_to_string(path)?)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefUpdateActor<'a> {
    Unsigned,
    Signed(SignedRefUpdateActor<'a>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SignedRefUpdateActor<'a> {
    pub account_id: &'a str,
    pub device_id: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionReceipt {
    Committed {
        transaction_id: TransactionId,
        ref_name: RefName,
        old_oid: Option<GitSha1Oid>,
        new_oid: Option<GitSha1Oid>,
    },
    Rejected {
        transaction_id: TransactionId,
        reason: RejectionReason,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RejectionReason {
    Conflict {
        expected: Option<GitSha1Oid>,
        actual: Option<GitSha1Oid>,
    },
    IdempotencyViolation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RefCheckpoint {
    pub sequence: u64,
    pub parent: Option<Cid>,
    pub refs_root: Cid,
    pub history_root: Cid,
    pub checkpoint_cid: Cid,
}

impl RefCheckpoint {
    pub fn signing_transcript(&self) -> Vec<u8> {
        let mut transcript = Vec::new();
        transcript.extend_from_slice(b"gitmesh.v0.signed-ref-checkpoint");
        transcript.extend_from_slice(&self.sequence.to_be_bytes());
        put_transcript_field(&mut transcript, format_optional_cid(self.parent).as_bytes());
        put_transcript_field(&mut transcript, self.refs_root.as_hex().as_bytes());
        put_transcript_field(&mut transcript, self.history_root.as_hex().as_bytes());
        put_transcript_field(&mut transcript, self.checkpoint_cid.to_string().as_bytes());
        transcript
    }

    pub fn verify_cid(&self) -> Result<(), CoordinationError> {
        let expected = compute_checkpoint_cid(
            self.sequence,
            self.parent,
            self.refs_root,
            self.history_root,
        );
        if self.checkpoint_cid != expected {
            return Err(CoordinationError::InvalidSnapshot);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignedRefCheckpoint {
    pub checkpoint: RefCheckpoint,
    pub certificate: DeviceCertificate,
    pub signature: [u8; 64],
}

impl SignedRefCheckpoint {
    pub fn sign(
        checkpoint: RefCheckpoint,
        certificate: DeviceCertificate,
        device: &DeviceKey,
    ) -> Result<Self, CoordinationError> {
        if certificate.device_id != device.device_id() {
            return Err(CoordinationError::CheckpointSignerMismatch);
        }
        checkpoint.verify_cid()?;
        let signature = device.sign(&checkpoint.signing_transcript());
        Ok(Self {
            checkpoint,
            certificate,
            signature,
        })
    }

    pub fn new(
        checkpoint: RefCheckpoint,
        certificate: DeviceCertificate,
        signature: [u8; 64],
    ) -> Self {
        Self {
            checkpoint,
            certificate,
            signature,
        }
    }

    pub fn verify(
        &self,
        previous: Option<&RefCheckpoint>,
    ) -> Result<VerifiedRefCheckpoint, CoordinationError> {
        validate_checkpoint_chain(previous, &self.checkpoint)?;
        self.checkpoint.verify_cid()?;
        self.certificate
            .verify_device_signature(&self.checkpoint.signing_transcript(), &self.signature)?;
        Ok(VerifiedRefCheckpoint {
            checkpoint: self.checkpoint.clone(),
            account_id: self.certificate.account_id.clone(),
            device_id: self.certificate.device_id.clone(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedRefCheckpoint {
    pub checkpoint: RefCheckpoint,
    pub account_id: AccountId,
    pub device_id: DeviceId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RefMutation {
    pub sequence: u64,
    pub transaction_id: TransactionId,
    pub fingerprint: String,
    pub ref_name: RefName,
    pub old_oid: Option<GitSha1Oid>,
    pub new_oid: Option<GitSha1Oid>,
    pub force: bool,
    pub signer: String,
}

#[derive(Clone, Debug, Default)]
pub struct RefStore {
    refs: BTreeMap<RefName, GitSha1Oid>,
    receipts: BTreeMap<TransactionId, (String, TransactionReceipt)>,
    mutations: Vec<RefMutation>,
    checkpoints: Vec<RefCheckpoint>,
}

impl RefStore {
    pub fn current(&self, ref_name: &RefName) -> Option<GitSha1Oid> {
        self.refs.get(ref_name).copied()
    }

    pub fn ref_count(&self) -> usize {
        self.refs.len()
    }

    pub fn refs(&self) -> impl Iterator<Item = (&RefName, GitSha1Oid)> {
        self.refs.iter().map(|(ref_name, oid)| (ref_name, *oid))
    }

    pub fn checkpoint_count(&self) -> usize {
        self.checkpoints.len()
    }

    pub fn mutation_count(&self) -> usize {
        self.mutations.len()
    }

    pub fn latest_checkpoint(&self) -> Option<&RefCheckpoint> {
        self.checkpoints.last()
    }

    pub fn checkpoint_before_latest(&self) -> Option<&RefCheckpoint> {
        self.checkpoints
            .len()
            .checked_sub(2)
            .and_then(|index| self.checkpoints.get(index))
    }

    pub fn preflight_receipt(&self, update: &RefUpdate) -> Option<TransactionReceipt> {
        let (existing_fingerprint, receipt) = self.receipts.get(&update.transaction_id)?;
        if existing_fingerprint == &update.operation_fingerprint() {
            return Some(receipt.clone());
        }
        Some(TransactionReceipt::Rejected {
            transaction_id: update.transaction_id.clone(),
            reason: RejectionReason::IdempotencyViolation,
        })
    }

    pub fn apply(&mut self, update: RefUpdate) -> TransactionReceipt {
        let fingerprint = update.operation_fingerprint();
        if let Some((existing_fingerprint, receipt)) = self.receipts.get(&update.transaction_id) {
            if existing_fingerprint == &fingerprint {
                return receipt.clone();
            }
            return TransactionReceipt::Rejected {
                transaction_id: update.transaction_id,
                reason: RejectionReason::IdempotencyViolation,
            };
        }

        let actual_old_oid = self.current(&update.ref_name);
        let receipt = if actual_old_oid == update.expected_old_oid {
            if let Some(new_oid) = update.new_oid {
                self.refs.insert(update.ref_name.clone(), new_oid);
            } else {
                self.refs.remove(&update.ref_name);
            }
            TransactionReceipt::Committed {
                transaction_id: update.transaction_id.clone(),
                ref_name: update.ref_name.clone(),
                old_oid: actual_old_oid,
                new_oid: update.new_oid,
            }
        } else {
            TransactionReceipt::Rejected {
                transaction_id: update.transaction_id.clone(),
                reason: RejectionReason::Conflict {
                    expected: update.expected_old_oid,
                    actual: actual_old_oid,
                },
            }
        };

        self.receipts.insert(
            update.transaction_id.clone(),
            (fingerprint.clone(), receipt.clone()),
        );
        if matches!(receipt, TransactionReceipt::Committed { .. }) {
            let mutation = committed_mutation(
                self.mutations.len() as u64 + 1,
                &update,
                fingerprint,
                &receipt,
            );
            self.append_checkpoint(&mutation);
            self.mutations.push(mutation);
        }
        receipt
    }

    pub fn save_to_path(&self, path: impl AsRef<Path>) -> Result<(), CoordinationError> {
        let path = path.as_ref();
        let mut snapshot = String::from("gitmesh-ref-store-v0\n");
        for (ref_name, oid) in &self.refs {
            snapshot.push_str(&format!("ref\t{}\t{}\n", ref_name.as_str(), oid));
        }
        for (transaction_id, (fingerprint, receipt)) in &self.receipts {
            validate_snapshot_field(fingerprint)?;
            match receipt {
                TransactionReceipt::Committed {
                    ref_name,
                    old_oid,
                    new_oid,
                    ..
                } => snapshot.push_str(&format!(
                    "receipt_committed\t{}\t{}\t{}\t{}\t{}\n",
                    transaction_id.as_str(),
                    fingerprint,
                    ref_name.as_str(),
                    format_optional_oid(*old_oid),
                    format_optional_oid(*new_oid)
                )),
                TransactionReceipt::Rejected { reason, .. } => match reason {
                    RejectionReason::Conflict { expected, actual } => snapshot.push_str(&format!(
                        "receipt_rejected_conflict\t{}\t{}\t{}\t{}\n",
                        transaction_id.as_str(),
                        fingerprint,
                        format_optional_oid(*expected),
                        format_optional_oid(*actual)
                    )),
                    RejectionReason::IdempotencyViolation => snapshot.push_str(&format!(
                        "receipt_rejected_idempotency\t{}\t{}\n",
                        transaction_id.as_str(),
                        fingerprint
                    )),
                },
            }
        }
        for mutation in &self.mutations {
            validate_snapshot_field(&mutation.fingerprint)?;
            validate_snapshot_field(&mutation.signer)?;
            snapshot.push_str(&format!(
                "mutation\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                mutation.sequence,
                mutation.transaction_id.as_str(),
                mutation.fingerprint,
                mutation.ref_name.as_str(),
                format_optional_oid(mutation.old_oid),
                format_optional_oid(mutation.new_oid),
                format_bool(mutation.force),
                mutation.signer
            ));
        }
        for checkpoint in &self.checkpoints {
            snapshot.push_str(&format!(
                "checkpoint\t{}\t{}\t{}\t{}\t{}\n",
                checkpoint.sequence,
                format_optional_cid(checkpoint.parent),
                checkpoint.refs_root,
                checkpoint.history_root,
                checkpoint.checkpoint_cid
            ));
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp_path = path.with_extension("tmp");
        fs::write(&tmp_path, snapshot)?;
        fs::rename(tmp_path, path)?;
        Ok(())
    }

    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, CoordinationError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(path)?;
        Self::from_snapshot(&text)
    }

    pub fn from_snapshot(text: &str) -> Result<Self, CoordinationError> {
        let mut lines = text.lines();
        if lines.next() != Some("gitmesh-ref-store-v0") {
            return Err(CoordinationError::InvalidSnapshot);
        }
        let mut store = Self::default();
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            let parts = line.split('\t').collect::<Vec<_>>();
            match parts.first().copied() {
                Some("ref") => {
                    if parts.len() != 3 {
                        return Err(CoordinationError::InvalidSnapshot);
                    }
                    store.refs.insert(
                        RefName::new(parts[1])?,
                        parse_optional_oid(parts[2])?.ok_or(CoordinationError::InvalidSnapshot)?,
                    );
                }
                Some("receipt_committed") => {
                    if parts.len() != 6 {
                        return Err(CoordinationError::InvalidSnapshot);
                    }
                    let transaction_id = TransactionId::new(parts[1])?;
                    let fingerprint = snapshot_field(parts[2])?;
                    let ref_name = RefName::new(parts[3])?;
                    let receipt = TransactionReceipt::Committed {
                        transaction_id: transaction_id.clone(),
                        ref_name,
                        old_oid: parse_optional_oid(parts[4])?,
                        new_oid: parse_optional_oid(parts[5])?,
                    };
                    store
                        .receipts
                        .insert(transaction_id, (fingerprint, receipt));
                }
                Some("receipt_rejected_conflict") => {
                    if parts.len() != 5 {
                        return Err(CoordinationError::InvalidSnapshot);
                    }
                    let transaction_id = TransactionId::new(parts[1])?;
                    let fingerprint = snapshot_field(parts[2])?;
                    let receipt = TransactionReceipt::Rejected {
                        transaction_id: transaction_id.clone(),
                        reason: RejectionReason::Conflict {
                            expected: parse_optional_oid(parts[3])?,
                            actual: parse_optional_oid(parts[4])?,
                        },
                    };
                    store
                        .receipts
                        .insert(transaction_id, (fingerprint, receipt));
                }
                Some("receipt_rejected_idempotency") => {
                    if parts.len() != 3 {
                        return Err(CoordinationError::InvalidSnapshot);
                    }
                    let transaction_id = TransactionId::new(parts[1])?;
                    let fingerprint = snapshot_field(parts[2])?;
                    let receipt = TransactionReceipt::Rejected {
                        transaction_id: transaction_id.clone(),
                        reason: RejectionReason::IdempotencyViolation,
                    };
                    store
                        .receipts
                        .insert(transaction_id, (fingerprint, receipt));
                }
                Some("checkpoint") => {
                    if parts.len() != 6 {
                        return Err(CoordinationError::InvalidSnapshot);
                    }
                    let sequence = parse_u64(parts[1])?;
                    let parent = parse_optional_cid(parts[2])?;
                    let refs_root = protocol_cid_from_text(parts[3])?;
                    let history_root = protocol_cid_from_text(parts[4])?;
                    let checkpoint_cid = protocol_cid_from_text(parts[5])?;
                    let checkpoint = RefCheckpoint {
                        sequence,
                        parent,
                        refs_root,
                        history_root,
                        checkpoint_cid,
                    };
                    validate_checkpoint_chain(store.checkpoints.last(), &checkpoint)?;
                    store.checkpoints.push(checkpoint);
                }
                Some("mutation") => {
                    if parts.len() != 9 {
                        return Err(CoordinationError::InvalidSnapshot);
                    }
                    let mutation = RefMutation {
                        sequence: parse_u64(parts[1])?,
                        transaction_id: TransactionId::new(parts[2])?,
                        fingerprint: snapshot_field(parts[3])?,
                        ref_name: RefName::new(parts[4])?,
                        old_oid: parse_optional_oid(parts[5])?,
                        new_oid: parse_optional_oid(parts[6])?,
                        force: parse_bool(parts[7])?,
                        signer: snapshot_field(parts[8])?,
                    };
                    validate_mutation_sequence(store.mutations.last(), &mutation)?;
                    store.mutations.push(mutation);
                }
                _ => return Err(CoordinationError::InvalidSnapshot),
            }
        }
        validate_store_consistency(&store)?;
        Ok(store)
    }

    fn append_checkpoint(&mut self, mutation: &RefMutation) {
        let sequence = mutation.sequence;
        let parent = self
            .latest_checkpoint()
            .map(|checkpoint| checkpoint.checkpoint_cid);
        let previous_history = self
            .latest_checkpoint()
            .map(|checkpoint| checkpoint.history_root);
        let refs_root = compute_refs_root(&self.refs);
        let history_root = compute_history_root(previous_history, mutation);
        let checkpoint_cid = compute_checkpoint_cid(sequence, parent, refs_root, history_root);
        self.checkpoints.push(RefCheckpoint {
            sequence,
            parent,
            refs_root,
            history_root,
            checkpoint_cid,
        });
    }
}

#[derive(Debug, Error)]
pub enum CoordinationError {
    #[error("invalid ref name")]
    InvalidRefName,
    #[error("invalid transaction id")]
    InvalidTransactionId,
    #[error("invalid ref-store snapshot")]
    InvalidSnapshot,
    #[error("ref update signer does not match the device certificate")]
    SignerMismatch,
    #[error("ref checkpoint signer does not match the device certificate")]
    CheckpointSignerMismatch,
    #[error("unsigned ref update denied by repository policy")]
    UnsignedRefUpdateDenied,
    #[error("ref update is not authorized by repository policy")]
    RefUpdateNotAuthorized,
    #[error("protected ref update denied by repository policy")]
    ProtectedRefDenied,
    #[error("identity verification failed: {0}")]
    Identity(#[from] IdentityError),
    #[error("I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

fn parse_optional_oid(value: &str) -> Result<Option<GitSha1Oid>, CoordinationError> {
    if value == "none" {
        Ok(None)
    } else {
        GitSha1Oid::from_str(value)
            .map(Some)
            .map_err(|_| CoordinationError::InvalidSnapshot)
    }
}

fn format_optional_oid(oid: Option<GitSha1Oid>) -> String {
    oid.map_or_else(|| "none".to_string(), |oid| oid.hex())
}

fn format_bool(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn parse_bool(value: &str) -> Result<bool, CoordinationError> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(CoordinationError::InvalidSnapshot),
    }
}

fn parse_optional_cid(value: &str) -> Result<Option<Cid>, CoordinationError> {
    if value == "none" {
        Ok(None)
    } else {
        protocol_cid_from_text(value).map(Some)
    }
}

fn format_optional_cid(cid: Option<Cid>) -> String {
    cid.map_or_else(|| "none".to_string(), |cid| cid.to_string())
}

fn protocol_cid_from_text(value: &str) -> Result<Cid, CoordinationError> {
    if value.starts_with("gitmesh:") {
        let cid = value
            .parse::<Cid>()
            .map_err(|_| CoordinationError::InvalidSnapshot)?;
        if cid.kind() != CidKind::ProtocolObject
            || cid.hash_algorithm() != HashAlgorithm::Blake3_256
        {
            return Err(CoordinationError::InvalidSnapshot);
        }
        Ok(cid)
    } else {
        Ok(Cid::from_digest(
            CidKind::ProtocolObject,
            HashAlgorithm::Blake3_256,
            parse_digest(value)?,
        ))
    }
}

fn parse_digest(value: &str) -> Result<[u8; 32], CoordinationError> {
    if value.len() != 64 {
        return Err(CoordinationError::InvalidSnapshot);
    }
    let mut digest = [0_u8; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        let high = hex_nibble(value.as_bytes()[index * 2])?;
        let low = hex_nibble(value.as_bytes()[index * 2 + 1])?;
        *byte = (high << 4) | low;
    }
    Ok(digest)
}

fn hex_nibble(byte: u8) -> Result<u8, CoordinationError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(CoordinationError::InvalidSnapshot),
    }
}

fn parse_u64(value: &str) -> Result<u64, CoordinationError> {
    value
        .parse::<u64>()
        .map_err(|_| CoordinationError::InvalidSnapshot)
}

fn compute_refs_root(refs: &BTreeMap<RefName, GitSha1Oid>) -> Cid {
    let mut transcript = Vec::new();
    transcript.extend_from_slice(b"gitmesh.v0.refs-root");
    transcript.extend_from_slice(&(refs.len() as u64).to_be_bytes());
    for (ref_name, oid) in refs {
        put_transcript_field(&mut transcript, ref_name.as_str().as_bytes());
        put_transcript_field(&mut transcript, oid.hex().as_bytes());
    }
    Cid::new(
        CidKind::ProtocolObject,
        HashAlgorithm::Blake3_256,
        &transcript,
    )
}

fn compute_history_root(previous_history: Option<Cid>, mutation: &RefMutation) -> Cid {
    let mut transcript = Vec::new();
    transcript.extend_from_slice(b"gitmesh.v0.ref-history-root");
    put_transcript_field(
        &mut transcript,
        format_optional_cid(previous_history).as_bytes(),
    );
    transcript.extend_from_slice(&mutation.sequence.to_be_bytes());
    put_transcript_field(&mut transcript, mutation.fingerprint.as_bytes());
    put_transcript_field(&mut transcript, mutation.transaction_id.as_str().as_bytes());
    put_transcript_field(&mut transcript, mutation.ref_name.as_str().as_bytes());
    put_transcript_field(
        &mut transcript,
        format_optional_oid(mutation.old_oid).as_bytes(),
    );
    put_transcript_field(
        &mut transcript,
        format_optional_oid(mutation.new_oid).as_bytes(),
    );
    transcript.push(u8::from(mutation.force));
    put_transcript_field(&mut transcript, mutation.signer.as_bytes());
    Cid::new(
        CidKind::ProtocolObject,
        HashAlgorithm::Blake3_256,
        &transcript,
    )
}

fn compute_checkpoint_cid(
    sequence: u64,
    parent: Option<Cid>,
    refs_root: Cid,
    history_root: Cid,
) -> Cid {
    let mut transcript = Vec::new();
    transcript.extend_from_slice(b"gitmesh.v0.ref-checkpoint");
    transcript.extend_from_slice(&sequence.to_be_bytes());
    put_transcript_field(&mut transcript, format_optional_cid(parent).as_bytes());
    put_transcript_field(&mut transcript, refs_root.as_hex().as_bytes());
    put_transcript_field(&mut transcript, history_root.as_hex().as_bytes());
    Cid::new(
        CidKind::ProtocolObject,
        HashAlgorithm::Blake3_256,
        &transcript,
    )
}

fn validate_checkpoint_chain(
    previous: Option<&RefCheckpoint>,
    checkpoint: &RefCheckpoint,
) -> Result<(), CoordinationError> {
    let expected_sequence = previous.map_or(1, |previous| previous.sequence + 1);
    let expected_parent = previous.map(|previous| previous.checkpoint_cid);
    let expected_cid = compute_checkpoint_cid(
        checkpoint.sequence,
        checkpoint.parent,
        checkpoint.refs_root,
        checkpoint.history_root,
    );
    if checkpoint.sequence != expected_sequence
        || checkpoint.parent != expected_parent
        || checkpoint.checkpoint_cid != expected_cid
    {
        return Err(CoordinationError::InvalidSnapshot);
    }
    Ok(())
}

fn committed_mutation(
    sequence: u64,
    update: &RefUpdate,
    fingerprint: String,
    receipt: &TransactionReceipt,
) -> RefMutation {
    if let TransactionReceipt::Committed {
        ref_name,
        old_oid,
        new_oid,
        ..
    } = receipt
    {
        RefMutation {
            sequence,
            transaction_id: update.transaction_id.clone(),
            fingerprint,
            ref_name: ref_name.clone(),
            old_oid: *old_oid,
            new_oid: *new_oid,
            force: update.force,
            signer: update.signer.clone(),
        }
    } else {
        unreachable!("only committed receipts are turned into mutations")
    }
}

fn validate_mutation_sequence(
    previous: Option<&RefMutation>,
    mutation: &RefMutation,
) -> Result<(), CoordinationError> {
    let expected_sequence = previous.map_or(1, |previous| previous.sequence + 1);
    if mutation.sequence != expected_sequence {
        return Err(CoordinationError::InvalidSnapshot);
    }
    Ok(())
}

fn validate_store_consistency(store: &RefStore) -> Result<(), CoordinationError> {
    if store.mutations.len() != store.checkpoints.len() {
        return Err(CoordinationError::InvalidSnapshot);
    }

    let mut refs = BTreeMap::<RefName, GitSha1Oid>::new();
    let mut previous_history = None;
    let mut previous_checkpoint = None;

    for (index, mutation) in store.mutations.iter().enumerate() {
        let current = refs.get(&mutation.ref_name).copied();
        if current != mutation.old_oid {
            return Err(CoordinationError::InvalidSnapshot);
        }

        match mutation.new_oid {
            Some(new_oid) => {
                refs.insert(mutation.ref_name.clone(), new_oid);
            }
            None => {
                refs.remove(&mutation.ref_name);
            }
        }

        let receipt = store
            .receipts
            .get(&mutation.transaction_id)
            .ok_or(CoordinationError::InvalidSnapshot)?;
        validate_mutation_receipt(mutation, receipt)?;

        let refs_root = compute_refs_root(&refs);
        let history_root = compute_history_root(previous_history, mutation);
        let checkpoint = &store.checkpoints[index];
        let expected_checkpoint = compute_checkpoint_cid(
            mutation.sequence,
            previous_checkpoint,
            refs_root,
            history_root,
        );
        if checkpoint.sequence != mutation.sequence
            || checkpoint.parent != previous_checkpoint
            || checkpoint.refs_root != refs_root
            || checkpoint.history_root != history_root
            || checkpoint.checkpoint_cid != expected_checkpoint
        {
            return Err(CoordinationError::InvalidSnapshot);
        }

        previous_history = Some(history_root);
        previous_checkpoint = Some(checkpoint.checkpoint_cid);
    }

    if refs != store.refs {
        return Err(CoordinationError::InvalidSnapshot);
    }
    Ok(())
}

fn validate_mutation_receipt(
    mutation: &RefMutation,
    (fingerprint, receipt): &(String, TransactionReceipt),
) -> Result<(), CoordinationError> {
    if fingerprint != &mutation.fingerprint {
        return Err(CoordinationError::InvalidSnapshot);
    }
    match receipt {
        TransactionReceipt::Committed {
            transaction_id,
            ref_name,
            old_oid,
            new_oid,
        } if transaction_id == &mutation.transaction_id
            && ref_name == &mutation.ref_name
            && old_oid == &mutation.old_oid
            && new_oid == &mutation.new_oid =>
        {
            Ok(())
        }
        _ => Err(CoordinationError::InvalidSnapshot),
    }
}

fn snapshot_field(value: &str) -> Result<String, CoordinationError> {
    validate_snapshot_field(value)?;
    Ok(value.to_string())
}

fn validate_snapshot_field(value: &str) -> Result<(), CoordinationError> {
    if value.contains('\t') || value.contains('\n') || value.contains('\r') {
        return Err(CoordinationError::InvalidSnapshot);
    }
    Ok(())
}

fn put_transcript_field(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(&(value.len() as u64).to_be_bytes());
    out.extend_from_slice(value);
}

fn validate_ref_name(value: &str) -> Result<(), CoordinationError> {
    if !value.starts_with("refs/") || value.ends_with('/') || value.contains("..") {
        return Err(CoordinationError::InvalidRefName);
    }
    if value
        .bytes()
        .any(|byte| byte <= 0x20 || matches!(byte, b'~' | b'^' | b':' | b'?' | b'*' | b'[' | b'\\'))
    {
        return Err(CoordinationError::InvalidRefName);
    }
    if value
        .split('/')
        .any(|part| part.is_empty() || part.ends_with(".lock"))
    {
        return Err(CoordinationError::InvalidRefName);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitmesh_identity::{AccountRootKey, DeviceKey};

    fn oid(byte: u8) -> GitSha1Oid {
        GitSha1Oid::from_digest([byte; 20])
    }

    fn update(
        tx: &str,
        expected_old_oid: Option<GitSha1Oid>,
        new_oid: Option<GitSha1Oid>,
    ) -> RefUpdate {
        RefUpdate {
            repo_id: RepoId::new(b"repo"),
            ref_name: RefName::new("refs/heads/main").unwrap(),
            expected_old_oid,
            new_oid,
            force: false,
            policy_epoch: 0,
            transaction_id: TransactionId::new(tx).unwrap(),
            signer: "acct_farzeen".to_string(),
        }
    }

    #[test]
    fn creates_ref_when_expected_old_is_absent() {
        let mut store = RefStore::default();
        let receipt = store.apply(update("tx1", None, Some(oid(1))));

        assert!(matches!(receipt, TransactionReceipt::Committed { .. }));
        assert_eq!(
            store.current(&RefName::new("refs/heads/main").unwrap()),
            Some(oid(1))
        );
    }

    #[test]
    fn rejects_conflicting_update() {
        let mut store = RefStore::default();
        store.apply(update("tx1", None, Some(oid(1))));

        let receipt = store.apply(update("tx2", None, Some(oid(2))));

        assert_eq!(
            receipt,
            TransactionReceipt::Rejected {
                transaction_id: TransactionId::new("tx2").unwrap(),
                reason: RejectionReason::Conflict {
                    expected: None,
                    actual: Some(oid(1))
                }
            }
        );
        assert_eq!(
            store.current(&RefName::new("refs/heads/main").unwrap()),
            Some(oid(1))
        );
    }

    #[test]
    fn retry_returns_original_receipt() {
        let mut store = RefStore::default();
        let first = store.apply(update("tx1", None, Some(oid(1))));
        let second = store.apply(update("tx1", None, Some(oid(1))));

        assert_eq!(first, second);
        assert_eq!(store.mutation_count(), 1);
        assert_eq!(store.checkpoint_count(), 1);
    }

    #[test]
    fn refs_iterates_current_refs_in_order() {
        let mut store = RefStore::default();
        let mut update_z = update("tx-z", None, Some(oid(1)));
        update_z.ref_name = RefName::new("refs/heads/z").unwrap();
        let mut update_a = update("tx-a", None, Some(oid(1)));
        update_a.ref_name = RefName::new("refs/heads/a").unwrap();
        store.apply(update_z);
        store.apply(update_a);

        let names = store
            .refs()
            .map(|(ref_name, _)| ref_name.as_str().to_string())
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["refs/heads/a", "refs/heads/z"]);
    }

    #[test]
    fn preflight_reports_idempotent_receipt_before_apply() {
        let mut store = RefStore::default();
        let update = update("tx1", None, Some(oid(1)));
        let first = store.apply(update.clone());

        assert_eq!(store.preflight_receipt(&update), Some(first));
    }

    #[test]
    fn transaction_id_reuse_with_different_body_is_rejected() {
        let mut store = RefStore::default();
        store.apply(update("tx1", None, Some(oid(1))));
        let receipt = store.apply(update("tx1", Some(oid(1)), Some(oid(2))));

        assert_eq!(
            receipt,
            TransactionReceipt::Rejected {
                transaction_id: TransactionId::new("tx1").unwrap(),
                reason: RejectionReason::IdempotencyViolation
            }
        );
        assert_eq!(store.checkpoint_count(), 1);
    }

    #[test]
    fn committed_updates_create_chained_checkpoints() {
        let mut store = RefStore::default();

        store.apply(update("tx1", None, Some(oid(1))));
        store.apply(update("tx2", Some(oid(1)), Some(oid(2))));

        assert_eq!(store.checkpoint_count(), 2);
        assert_eq!(store.mutation_count(), 2);
        let first = &store.checkpoints[0];
        let second = store.latest_checkpoint().unwrap();
        assert_eq!(first.sequence, 1);
        assert_eq!(first.parent, None);
        assert_eq!(second.sequence, 2);
        assert_eq!(second.parent, Some(first.checkpoint_cid));
        assert_ne!(first.refs_root, second.refs_root);
        assert_ne!(first.history_root, second.history_root);
        assert_eq!(store.checkpoint_before_latest(), Some(first));
    }

    #[test]
    fn signed_ref_checkpoint_verifies_chain_and_identity() {
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let certificate = account.certify_device(&device, "coordinator");
        let mut store = RefStore::default();
        store.apply(update("tx1", None, Some(oid(1))));
        let checkpoint = store.latest_checkpoint().unwrap().clone();
        let account_id = certificate.account_id.clone();
        let device_id = certificate.device_id.clone();

        let signed = SignedRefCheckpoint::sign(checkpoint.clone(), certificate, &device).unwrap();
        let verified = signed.verify(None).unwrap();

        assert_eq!(verified.checkpoint, checkpoint);
        assert_eq!(verified.account_id, account_id);
        assert_eq!(verified.device_id, device_id);
    }

    #[test]
    fn signed_ref_checkpoint_rejects_tampering() {
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let certificate = account.certify_device(&device, "coordinator");
        let mut store = RefStore::default();
        store.apply(update("tx1", None, Some(oid(1))));
        let mut checkpoint = store.latest_checkpoint().unwrap().clone();
        let signed = SignedRefCheckpoint::sign(checkpoint.clone(), certificate, &device).unwrap();
        checkpoint.refs_root = RepoId::new(b"tampered").cid();
        let tampered = SignedRefCheckpoint::new(checkpoint, signed.certificate, signed.signature);

        let err = tampered.verify(None).unwrap_err();

        assert!(matches!(err, CoordinationError::InvalidSnapshot));
    }

    #[test]
    fn signed_ref_checkpoint_rejects_wrong_parent_chain() {
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let certificate = account.certify_device(&device, "coordinator");
        let mut store = RefStore::default();
        store.apply(update("tx1", None, Some(oid(1))));
        store.apply(update("tx2", Some(oid(1)), Some(oid(2))));
        let first = store.checkpoints[0].clone();
        let second = store.checkpoints[1].clone();
        let signed = SignedRefCheckpoint::sign(first.clone(), certificate, &device).unwrap();

        let err = signed.verify(Some(&second)).unwrap_err();

        assert!(matches!(err, CoordinationError::InvalidSnapshot));
    }

    #[test]
    fn rejected_conflict_does_not_create_checkpoint() {
        let mut store = RefStore::default();

        store.apply(update("tx1", None, Some(oid(1))));
        store.apply(update("tx2", None, Some(oid(2))));

        assert_eq!(store.checkpoint_count(), 1);
        assert_eq!(store.mutation_count(), 1);
    }

    #[test]
    fn signed_ref_update_verifies() {
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let certificate = account.certify_device(&device, "laptop");
        let mut update = update("tx1", None, Some(oid(1)));
        update.signer = certificate.device_id.as_cid().to_string();
        let signature = device.sign(&update.signing_transcript());

        let verified = SignedRefUpdate::new(update.clone(), certificate, signature)
            .verify()
            .unwrap();

        assert_eq!(verified, update);
    }

    #[test]
    fn signed_ref_update_verification_exposes_actor_identity() {
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let certificate = account.certify_device(&device, "laptop");
        let account_id = certificate.account_id.clone();
        let device_id = certificate.device_id.clone();
        let mut update = update("tx1", None, Some(oid(1)));
        update.signer = certificate.device_id.as_cid().to_string();
        let signature = device.sign(&update.signing_transcript());

        let verified = SignedRefUpdate::new(update.clone(), certificate, signature)
            .verify_with_identity()
            .unwrap();

        assert_eq!(verified.update, update);
        assert_eq!(verified.account_id, account_id);
        assert_eq!(verified.device_id, device_id);
    }

    #[test]
    fn signed_ref_update_rejects_tampering() {
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let certificate = account.certify_device(&device, "laptop");
        let mut update = update("tx1", None, Some(oid(1)));
        update.signer = certificate.device_id.as_cid().to_string();
        let signature = device.sign(&update.signing_transcript());
        update.new_oid = Some(oid(2));

        let err = SignedRefUpdate::new(update, certificate, signature)
            .verify()
            .unwrap_err();

        assert!(matches!(
            err,
            CoordinationError::Identity(IdentityError::InvalidSignature)
        ));
    }

    #[test]
    fn validates_ref_names() {
        assert!(RefName::new("refs/heads/main").is_ok());
        assert!(RefName::new("main").is_err());
        assert!(RefName::new("refs/heads/bad..name").is_err());
        assert!(RefName::new("refs/heads/bad.lock").is_err());
    }

    #[test]
    fn policy_can_require_signed_ref_updates() {
        let mut policy = RepoPolicy::default();
        policy.set_require_signed_refs(true);

        let err = policy
            .authorize_ref_update(&update("tx1", None, Some(oid(1))), RefUpdateActor::Unsigned)
            .unwrap_err();

        assert!(matches!(err, CoordinationError::UnsignedRefUpdateDenied));
    }

    #[test]
    fn policy_authorizes_granted_writer_account() {
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let certificate = account.certify_device(&device, "laptop");
        let mut policy = RepoPolicy::default();
        policy.grant_writer_account(&certificate.account_id);

        policy
            .authorize_ref_update(
                &update("tx1", None, Some(oid(1))),
                RefUpdateActor::Signed(SignedRefUpdateActor {
                    account_id: &certificate.account_id.as_cid().to_string(),
                    device_id: &certificate.device_id.as_cid().to_string(),
                }),
            )
            .unwrap();
    }

    #[test]
    fn policy_rejects_ungranted_writer_when_acl_exists() {
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let certificate = account.certify_device(&device, "laptop");
        let other = AccountRootKey::generate();
        let other_device = DeviceKey::generate();
        let other_certificate = other.certify_device(&other_device, "other");
        let mut policy = RepoPolicy::default();
        policy.grant_writer_account(&certificate.account_id);

        let err = policy
            .authorize_ref_update(
                &update("tx1", None, Some(oid(1))),
                RefUpdateActor::Signed(SignedRefUpdateActor {
                    account_id: &other_certificate.account_id.as_cid().to_string(),
                    device_id: &other_certificate.device_id.as_cid().to_string(),
                }),
            )
            .unwrap_err();

        assert!(matches!(err, CoordinationError::RefUpdateNotAuthorized));
    }

    #[test]
    fn policy_rejects_protected_ref_force_without_force_grant() {
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let certificate = account.certify_device(&device, "laptop");
        let mut ref_update = update("tx1", Some(oid(1)), Some(oid(2)));
        ref_update.force = true;
        let mut policy = RepoPolicy::default();
        policy.grant_writer_account(&certificate.account_id);
        policy.protect_ref(RefName::new("refs/heads/main").unwrap());

        let err = policy
            .authorize_ref_update(
                &ref_update,
                RefUpdateActor::Signed(SignedRefUpdateActor {
                    account_id: &certificate.account_id.as_cid().to_string(),
                    device_id: &certificate.device_id.as_cid().to_string(),
                }),
            )
            .unwrap_err();

        assert!(matches!(err, CoordinationError::ProtectedRefDenied));
    }

    #[test]
    fn policy_allows_protected_ref_force_with_force_grant() {
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let certificate = account.certify_device(&device, "laptop");
        let mut ref_update = update("tx1", Some(oid(1)), Some(oid(2)));
        ref_update.force = true;
        let mut policy = RepoPolicy::default();
        policy.grant_writer_account(&certificate.account_id);
        policy.grant_force_push_account(&certificate.account_id);
        policy.protect_ref(RefName::new("refs/heads/main").unwrap());

        policy
            .authorize_ref_update(
                &ref_update,
                RefUpdateActor::Signed(SignedRefUpdateActor {
                    account_id: &certificate.account_id.as_cid().to_string(),
                    device_id: &certificate.device_id.as_cid().to_string(),
                }),
            )
            .unwrap();
    }

    #[test]
    fn policy_snapshot_round_trips() {
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let certificate = account.certify_device(&device, "laptop");
        let mut policy = RepoPolicy::default();
        policy.set_require_signed_refs(true);
        policy.grant_writer_account(&certificate.account_id);
        policy.grant_writer_device(&certificate.device_id);
        policy.grant_force_push_account(&certificate.account_id);
        policy.grant_force_push_device(&certificate.device_id);
        policy.protect_ref(RefName::new("refs/heads/main").unwrap());

        let restored = RepoPolicy::from_snapshot(&policy.to_snapshot().unwrap()).unwrap();

        assert_eq!(restored, policy);
        assert!(restored.require_signed_refs());
        assert_eq!(restored.writer_count(), 2);
        assert_eq!(restored.force_pusher_count(), 2);
        assert_eq!(restored.protected_ref_count(), 1);
    }

    #[test]
    fn snapshot_preserves_refs_and_idempotency_receipts() {
        let mut store = RefStore::default();
        let first = store.apply(update("tx1", None, Some(oid(1))));
        let mut snapshot_path = std::env::temp_dir();
        snapshot_path.push(format!("gitmesh-ref-store-test-{}.txt", std::process::id()));
        store.save_to_path(&snapshot_path).unwrap();
        let snapshot = fs::read_to_string(&snapshot_path).unwrap();

        let mut restored = RefStore::load_from_path(&snapshot_path).unwrap();
        let retry = restored.apply(update("tx1", None, Some(oid(1))));

        assert_eq!(
            restored.current(&RefName::new("refs/heads/main").unwrap()),
            Some(oid(1))
        );
        assert_eq!(first, retry);
        assert_eq!(restored.mutation_count(), 1);
        assert_eq!(restored.checkpoint_count(), 1);
        assert_eq!(restored.latest_checkpoint(), store.latest_checkpoint());
        assert!(snapshot.contains("gitmesh:v0:ProtocolObject:Blake3_256:"));
        let _ = fs::remove_file(snapshot_path);
    }

    #[test]
    fn invalid_snapshot_is_rejected() {
        assert!(RefStore::from_snapshot("bad\n").is_err());
        assert!(RefStore::from_snapshot(
            "gitmesh-ref-store-v0\ncheckpoint\t2\tnone\t0000000000000000000000000000000000000000000000000000000000000000\t0000000000000000000000000000000000000000000000000000000000000000\t0000000000000000000000000000000000000000000000000000000000000000\n"
        )
        .is_err());
    }

    #[test]
    fn snapshot_rejects_tampered_refs_or_mutation_history() {
        let mut store = RefStore::default();
        store.apply(update("tx1", None, Some(oid(1))));
        let mut snapshot_path = std::env::temp_dir();
        snapshot_path.push(format!(
            "gitmesh-ref-store-tamper-test-{}.txt",
            std::process::id()
        ));
        store.save_to_path(&snapshot_path).unwrap();
        let snapshot = fs::read_to_string(&snapshot_path).unwrap();

        let tampered_ref = snapshot.replace(
            "ref\trefs/heads/main\t0101010101010101010101010101010101010101",
            "ref\trefs/heads/main\t0202020202020202020202020202020202020202",
        );
        let tampered_mutation = snapshot.replace("mutation\t1\ttx1", "mutation\t2\ttx1");

        assert!(RefStore::from_snapshot(&tampered_ref).is_err());
        assert!(RefStore::from_snapshot(&tampered_mutation).is_err());
        let _ = fs::remove_file(snapshot_path);
    }
}
