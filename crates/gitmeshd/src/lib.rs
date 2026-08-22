//! Local GitMesh daemon protocol.
//!
//! V0 uses a deliberately tiny line protocol over a Unix domain socket. It is
//! enough for local integration tests and future remote-helper plumbing without
//! committing to the production IPC shape too early.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
#[cfg(unix)]
use std::sync::{Arc, Mutex};
#[cfg(unix)]
use std::thread;

use gitmesh_accounts::{
    AccountError, AccountStore, NewAccountProfile, ProfileUpdate, RepositoryVisibility, Username,
    now_unix,
};
use gitmesh_collaboration::{
    CollaborationError, CollaborationEventStore, IssueSummary, PullRequestSummary,
    sample_issue_events, sample_pull_request_events,
};
use gitmesh_coordination::{
    CoordinationError, RefName, RefStore, RefUpdate, RefUpdateActor, RejectionReason, RepoId,
    RepoPolicy, SignedRefUpdate, SignedRefUpdateActor, TransactionId, TransactionReceipt,
};
use gitmesh_git::{GitError, GitObject, GitSha1Oid, parse_packfile};
use gitmesh_identity::{
    DeviceCertificate, DeviceId, IdentityError, RepoKeyGrant, RepoKeyGrantStore,
};
use gitmesh_repository::{
    RepositoryError, RepositoryObjectAudit, RepositoryObjectStore, RepositoryRepairReport,
    RepositoryTransportRepairProof, decode_hex, encode_hex, parse_git_object_kind, parse_oid,
    run_repository_transport_repair_proof,
};
use gitmesh_storage::{StoragePolicy, run_v0_local_storage_proof};
use thiserror::Error;

#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

const MAX_DAEMON_COMMAND_BYTES: usize = 64 * 1024 * 1024;
const MAX_REQUEST_ID_BYTES: usize = 64;
const DAEMON_FRAME_MAGIC: &[u8; 4] = b"GMB1";
const DAEMON_FRAME_HEADER_BYTES: usize = 14;
const DAEMON_FRAME_VERSION: u8 = 1;
const DAEMON_FRAME_FLAG_ERROR: u8 = 0b0000_0001;

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("storage failed: {0}")]
    Storage(#[from] gitmesh_storage::StorageError),
    #[error("coordination failed: {0}")]
    Coordination(#[from] CoordinationError),
    #[error("Git object id failed: {0}")]
    Git(#[from] GitError),
    #[error("identity failed: {0}")]
    Identity(#[from] IdentityError),
    #[error("repository object store failed: {0}")]
    Repository(#[from] RepositoryError),
    #[error("account store failed: {0}")]
    Accounts(#[from] AccountError),
    #[error("collaboration store failed: {0}")]
    Collaboration(#[from] CollaborationError),
    #[error("ref target object is not durably stored: {0}")]
    MissingDurableObject(GitSha1Oid),
    #[error("invalid daemon command: {0}")]
    InvalidCommand(String),
    #[error("unknown daemon command '{0}'")]
    UnknownCommand(String),
    #[error("daemon request exceeds maximum command size")]
    RequestTooLarge,
    #[error("daemon request is empty")]
    EmptyRequest,
    #[error("invalid daemon request id")]
    InvalidRequestId,
    #[error("invalid daemon binary frame")]
    InvalidFrame,
    #[error("unsupported daemon binary frame version {0}")]
    UnsupportedFrameVersion(u8),
    #[error("daemon command requires admin authentication")]
    Unauthorized,
    #[error("daemon state lock was poisoned")]
    StatePoisoned,
    #[error("socket serving is not implemented on this platform yet")]
    UnsupportedPlatform,
}

pub type Result<T> = std::result::Result<T, DaemonError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DaemonResponse {
    Ok(String),
    Error(String),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DaemonAuth {
    admin_token: Option<String>,
}

impl DaemonAuth {
    pub fn disabled() -> Self {
        Self { admin_token: None }
    }

    pub fn from_admin_token(token: impl Into<String>) -> Result<Self> {
        let token = token.into();
        validate_admin_token(&token)?;
        Ok(Self {
            admin_token: Some(token),
        })
    }

    pub fn from_env() -> Result<Self> {
        match std::env::var("GITMESHD_ADMIN_TOKEN") {
            Ok(token) if !token.is_empty() => Self::from_admin_token(token),
            _ => Ok(Self::disabled()),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.admin_token.is_some()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DaemonStorePaths {
    pub object_store_path: Option<PathBuf>,
    pub ref_store_path: Option<PathBuf>,
    pub policy_store_path: Option<PathBuf>,
    pub key_grant_store_path: Option<PathBuf>,
    pub account_store_path: Option<PathBuf>,
    pub collaboration_store_path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct DaemonState {
    repo_id: RepoId,
    refs: RefStore,
    policy: RepoPolicy,
    objects: RepositoryObjectStore,
    key_grants: RepoKeyGrantStore,
    accounts: AccountStore,
    collaboration: CollaborationEventStore,
    object_store_path: Option<PathBuf>,
    ref_store_path: Option<PathBuf>,
    policy_store_path: Option<PathBuf>,
    key_grant_store_path: Option<PathBuf>,
    account_store_path: Option<PathBuf>,
    collaboration_store_path: Option<PathBuf>,
}

impl Default for DaemonState {
    fn default() -> Self {
        Self {
            repo_id: RepoId::new(b"gitmeshd-v0-repo"),
            refs: RefStore::default(),
            policy: RepoPolicy::default(),
            objects: RepositoryObjectStore::default(),
            key_grants: RepoKeyGrantStore::default(),
            accounts: AccountStore::default(),
            collaboration: CollaborationEventStore::default(),
            object_store_path: None,
            ref_store_path: None,
            policy_store_path: None,
            key_grant_store_path: None,
            account_store_path: None,
            collaboration_store_path: None,
        }
    }
}

impl DaemonState {
    pub fn with_object_store_path(path: impl Into<PathBuf>) -> Result<Self> {
        Self::with_store_paths(Some(path.into()), None, None)
    }

    pub fn with_store_paths(
        object_store_path: Option<PathBuf>,
        ref_store_path: Option<PathBuf>,
        policy_store_path: Option<PathBuf>,
    ) -> Result<Self> {
        Self::with_store_paths_and_key_grants(
            object_store_path,
            ref_store_path,
            policy_store_path,
            None,
        )
    }

    pub fn with_store_paths_and_key_grants(
        object_store_path: Option<PathBuf>,
        ref_store_path: Option<PathBuf>,
        policy_store_path: Option<PathBuf>,
        key_grant_store_path: Option<PathBuf>,
    ) -> Result<Self> {
        Self::with_all_store_paths(
            object_store_path,
            ref_store_path,
            policy_store_path,
            key_grant_store_path,
            None,
        )
    }

    pub fn with_all_store_paths(
        object_store_path: Option<PathBuf>,
        ref_store_path: Option<PathBuf>,
        policy_store_path: Option<PathBuf>,
        key_grant_store_path: Option<PathBuf>,
        account_store_path: Option<PathBuf>,
    ) -> Result<Self> {
        Self::with_all_store_paths_and_collaboration(
            object_store_path,
            ref_store_path,
            policy_store_path,
            key_grant_store_path,
            account_store_path,
            None,
        )
    }

    pub fn with_all_store_paths_and_collaboration(
        object_store_path: Option<PathBuf>,
        ref_store_path: Option<PathBuf>,
        policy_store_path: Option<PathBuf>,
        key_grant_store_path: Option<PathBuf>,
        account_store_path: Option<PathBuf>,
        collaboration_store_path: Option<PathBuf>,
    ) -> Result<Self> {
        Ok(Self {
            repo_id: RepoId::new(b"gitmeshd-v0-repo"),
            refs: if let Some(path) = &ref_store_path {
                RefStore::load_from_path(path)?
            } else {
                RefStore::default()
            },
            policy: if let Some(path) = &policy_store_path {
                RepoPolicy::load_from_path(path)?
            } else {
                RepoPolicy::default()
            },
            objects: if let Some(path) = &object_store_path {
                RepositoryObjectStore::load_from_path(path)?
            } else {
                RepositoryObjectStore::default()
            },
            key_grants: if let Some(path) = &key_grant_store_path {
                RepoKeyGrantStore::load_from_path(path)?
            } else {
                RepoKeyGrantStore::default()
            },
            accounts: if let Some(path) = &account_store_path {
                AccountStore::load_from_path(path)?
            } else {
                AccountStore::default()
            },
            collaboration: if let Some(path) = &collaboration_store_path {
                CollaborationEventStore::load_from_path(path)?
            } else {
                CollaborationEventStore::default()
            },
            object_store_path,
            ref_store_path,
            policy_store_path,
            key_grant_store_path,
            account_store_path,
            collaboration_store_path,
        })
    }

    pub fn handle_command(&mut self, line: &str) -> Result<DaemonResponse> {
        let trimmed = line.trim_end();
        if trimmed == "PING" {
            return Ok(DaemonResponse::Ok("pong".to_string()));
        }

        if let Some(payload) = trimmed.strip_prefix("V0_PROOF") {
            let payload = payload.trim_start();
            let payload = if payload.is_empty() {
                b"gitmeshd socket V0 proof".to_vec()
            } else {
                payload.as_bytes().to_vec()
            };
            let policy = StoragePolicy::default();
            let result =
                run_v0_local_storage_proof(&payload, policy.clone(), vec![0, 3, 6, 9, 12, 15])?;
            return Ok(DaemonResponse::Ok(format!(
                "recovered_exactly={} data_shards={} parity_shards={} available_shards={} segment_cid={}",
                result.recovered == payload,
                policy.data_shards,
                policy.parity_shards,
                result.available_shards,
                result.segment_cid
            )));
        }

        if let Some(payload) = trimmed.strip_prefix("NETWORK_REPAIR_PROOF") {
            let payload = payload.trim_start();
            let payload = if payload.is_empty() {
                b"gitmeshd transport repair proof".to_vec()
            } else {
                payload.as_bytes().to_vec()
            };
            let proof = run_repository_transport_repair_proof(&payload)?;
            return Ok(DaemonResponse::Ok(format_transport_repair_proof(&proof)));
        }

        if let Some(ref_name) = trimmed.strip_prefix("REF_GET ") {
            return self.ref_get(ref_name);
        }

        if trimmed == "REF_LIST" {
            return Ok(DaemonResponse::Ok(format_ref_list(&self.refs)));
        }

        if let Some(rest) = trimmed.strip_prefix("REF_UPDATE ") {
            return self.ref_update(rest, false);
        }

        if let Some(rest) = trimmed.strip_prefix("REF_UPDATE_FORCE ") {
            return self.ref_update(rest, true);
        }

        if let Some(rest) = trimmed.strip_prefix("REF_UPDATE_SIGNED ") {
            return self.ref_update_signed(rest, false);
        }

        if let Some(rest) = trimmed.strip_prefix("REF_UPDATE_SIGNED_FORCE ") {
            return self.ref_update_signed(rest, true);
        }

        if trimmed == "REF_CHECKPOINT" {
            return Ok(DaemonResponse::Ok(format_checkpoint(&self.refs)));
        }

        if trimmed == "POLICY_SHOW" {
            return Ok(DaemonResponse::Ok(format_policy(&self.policy)));
        }

        if let Some(rest) = trimmed.strip_prefix("POLICY_SET_REQUIRE_SIGNED ") {
            return self.policy_set_require_signed(rest);
        }

        if let Some(rest) = trimmed.strip_prefix("POLICY_GRANT_WRITER_ACCOUNT ") {
            return self.policy_grant_writer_account(rest);
        }

        if let Some(rest) = trimmed.strip_prefix("POLICY_GRANT_FORCE_ACCOUNT ") {
            return self.policy_grant_force_account(rest);
        }

        if let Some(rest) = trimmed.strip_prefix("POLICY_PROTECT_REF ") {
            return self.policy_protect_ref(rest);
        }

        if let Some(rest) = trimmed.strip_prefix("PACK_PUT ") {
            return self.pack_put(rest);
        }

        if let Some(rest) = trimmed.strip_prefix("PACK_GET ") {
            return self.pack_get(rest);
        }

        if let Some(rest) = trimmed.strip_prefix("PACK_GET_REACHABLE ") {
            return self.pack_get_reachable(rest);
        }

        if let Some(rest) = trimmed.strip_prefix("OBJECT_PUT ") {
            return self.object_put(rest);
        }

        if let Some(oid) = trimmed.strip_prefix("OBJECT_GET ") {
            return self.object_get(oid);
        }

        if let Some(rest) = trimmed.strip_prefix("OBJECT_AUDIT ") {
            return self.object_audit(rest);
        }

        if let Some(rest) = trimmed.strip_prefix("OBJECT_REPAIR ") {
            return self.object_repair(rest);
        }

        if trimmed == "OBJECT_LIST" {
            return Ok(DaemonResponse::Ok(format_object_list(&self.objects)));
        }

        if let Some(rest) = trimmed.strip_prefix("KEY_GRANT_PUT ") {
            return self.key_grant_put(rest);
        }

        if let Some(rest) = trimmed.strip_prefix("KEY_GRANT_LIST ") {
            return self.key_grant_list(rest);
        }

        if let Some(rest) = trimmed.strip_prefix("KEY_GRANT_REVOKE_DEVICE ") {
            return self.key_grant_revoke_device(rest);
        }

        if let Some(rest) = trimmed.strip_prefix("KEY_GRANT_STATUS ") {
            return self.key_grant_status(rest);
        }

        if let Some(rest) = trimmed.strip_prefix("ACCOUNT_CREATE ") {
            return self.account_create(rest);
        }

        if let Some(rest) = trimmed.strip_prefix("ACCOUNT_PROFILE ") {
            return self.account_profile(rest);
        }

        if let Some(rest) = trimmed.strip_prefix("ACCOUNT_UPDATE_PROFILE ") {
            return self.account_update_profile(rest);
        }

        if let Some(rest) = trimmed.strip_prefix("SESSION_ISSUE ") {
            return self.session_issue(rest);
        }

        if let Some(rest) = trimmed.strip_prefix("SESSION_AUTH ") {
            return self.session_auth(rest);
        }

        if let Some(rest) = trimmed.strip_prefix("SESSION_REVOKE ") {
            return self.session_revoke(rest);
        }

        if let Some(rest) = trimmed.strip_prefix("REPO_REGISTER ") {
            return self.repo_register(rest);
        }

        if let Some(owner) = trimmed.strip_prefix("REPO_LIST ") {
            return self.repo_list(owner);
        }

        if let Some(rest) = trimmed.strip_prefix("REPO_GET ") {
            return self.repo_get(rest);
        }

        if trimmed == "COLLAB_SEED_SAMPLES" {
            return self.collab_seed_samples();
        }

        if let Some(repo) = trimmed.strip_prefix("ISSUE_LIST ") {
            return self.issue_list(repo);
        }

        if let Some(repo) = trimmed.strip_prefix("PR_LIST ") {
            return self.pr_list(repo);
        }

        if trimmed == "ACCOUNT_STATUS" {
            return Ok(DaemonResponse::Ok(format_account_status(&self.accounts)?));
        }

        if trimmed == "REPO_STATUS" {
            return Ok(DaemonResponse::Ok(format!(
                "objects={} refs={} mutations={} checkpoints={} key_grants={} revoked_devices={} accounts={} active_sessions={} registered_repos={} collaboration_events={} data_shards={} parity_shards={}",
                self.objects.object_count(),
                self.refs.ref_count(),
                self.refs.mutation_count(),
                self.refs.checkpoint_count(),
                self.key_grants.grant_count(),
                self.key_grants.revoked_device_count(),
                self.accounts.profile_count(),
                self.accounts.active_session_count(now_unix()?),
                self.accounts.repository_count(),
                self.collaboration.event_count(),
                self.objects.policy().data_shards,
                self.objects.policy().parity_shards
            )));
        }

        Err(DaemonError::UnknownCommand(trimmed.to_string()))
    }

    fn ref_get(&self, ref_name: &str) -> Result<DaemonResponse> {
        let ref_name = RefName::new(ref_name)?;
        let value = self
            .refs
            .current(&ref_name)
            .map_or_else(|| "none".to_string(), |oid| oid.hex());
        Ok(DaemonResponse::Ok(format!(
            "ref={} oid={value}",
            ref_name.as_str()
        )))
    }

    fn ref_update(&mut self, rest: &str, force: bool) -> Result<DaemonResponse> {
        let parts = rest.split_whitespace().collect::<Vec<_>>();
        if parts.len() != 5 {
            return Err(DaemonError::InvalidCommand(
                "REF_UPDATE requires <tx> <ref> <expected|none> <new|delete> <signer>".to_string(),
            ));
        }
        let transaction_id = TransactionId::new(parts[0])?;
        let ref_name = RefName::new(parts[1])?;
        let expected_old_oid = parse_optional_oid(parts[2])?;
        let new_oid = if parts[3] == "delete" {
            None
        } else {
            Some(GitSha1Oid::from_str(parts[3])?)
        };
        let update = RefUpdate {
            repo_id: self.repo_id.clone(),
            ref_name,
            expected_old_oid,
            new_oid,
            force,
            policy_epoch: 0,
            transaction_id,
            signer: parts[4].to_string(),
        };
        self.apply_verified_ref_update(update, RefUpdateActor::Unsigned)
    }

    fn ref_update_signed(&mut self, rest: &str, force: bool) -> Result<DaemonResponse> {
        let parts = rest.split_whitespace().collect::<Vec<_>>();
        if parts.len() != 9 {
            return Err(DaemonError::InvalidCommand(
                "REF_UPDATE_SIGNED requires <tx> <ref> <expected|none> <new|delete> <label-hex> <account-key-hex> <device-key-hex> <cert-signature-hex> <update-signature-hex>".to_string(),
            ));
        }
        let certificate = DeviceCertificate::from_key_bytes(
            decode_label(parts[4])?,
            decode_fixed_hex::<32>(parts[5])?,
            decode_fixed_hex::<32>(parts[6])?,
            decode_fixed_hex::<64>(parts[7])?,
        )?;
        let signer = certificate.device_id.as_cid().to_string();
        let update = RefUpdate {
            repo_id: self.repo_id.clone(),
            ref_name: RefName::new(parts[1])?,
            expected_old_oid: parse_optional_oid(parts[2])?,
            new_oid: if parts[3] == "delete" {
                None
            } else {
                Some(GitSha1Oid::from_str(parts[3])?)
            },
            force,
            policy_epoch: 0,
            transaction_id: TransactionId::new(parts[0])?,
            signer,
        };
        let verified = SignedRefUpdate::new(update, certificate, decode_fixed_hex::<64>(parts[8])?)
            .verify_with_identity()?;
        let account_id = verified.account_id.as_cid().to_string();
        let device_id = verified.device_id.as_cid().to_string();
        self.apply_verified_ref_update(
            verified.update,
            RefUpdateActor::Signed(SignedRefUpdateActor {
                account_id: &account_id,
                device_id: &device_id,
            }),
        )
    }

    fn apply_verified_ref_update(
        &mut self,
        update: RefUpdate,
        actor: RefUpdateActor<'_>,
    ) -> Result<DaemonResponse> {
        if let Some(receipt) = self.refs.preflight_receipt(&update) {
            return Ok(DaemonResponse::Ok(format_receipt(receipt)));
        }
        self.policy.authorize_ref_update(&update, actor)?;
        self.objects.validate_ref_update(
            update.ref_name.as_str(),
            update.expected_old_oid,
            update.new_oid,
            update.force,
        )?;
        let receipt = self.refs.apply(update);
        self.save_refs()?;
        Ok(DaemonResponse::Ok(format_receipt(receipt)))
    }

    fn policy_set_require_signed(&mut self, rest: &str) -> Result<DaemonResponse> {
        let value = parse_bool_arg(rest.trim())?;
        self.policy.set_require_signed_refs(value);
        self.save_policy()?;
        Ok(DaemonResponse::Ok(format_policy(&self.policy)))
    }

    fn policy_grant_writer_account(&mut self, rest: &str) -> Result<DaemonResponse> {
        self.policy
            .grant_writer_account_id_string(parse_policy_identity(rest.trim())?);
        self.save_policy()?;
        Ok(DaemonResponse::Ok(format_policy(&self.policy)))
    }

    fn policy_grant_force_account(&mut self, rest: &str) -> Result<DaemonResponse> {
        self.policy
            .grant_force_push_account_id_string(parse_policy_identity(rest.trim())?);
        self.save_policy()?;
        Ok(DaemonResponse::Ok(format_policy(&self.policy)))
    }

    fn policy_protect_ref(&mut self, rest: &str) -> Result<DaemonResponse> {
        self.policy.protect_ref(RefName::new(rest.trim())?);
        self.save_policy()?;
        Ok(DaemonResponse::Ok(format_policy(&self.policy)))
    }

    fn object_put(&mut self, rest: &str) -> Result<DaemonResponse> {
        let parts = rest.split_whitespace().collect::<Vec<_>>();
        if parts.len() != 2 {
            return Err(DaemonError::InvalidCommand(
                "OBJECT_PUT requires <kind> <hex-payload>".to_string(),
            ));
        }
        let kind = parse_git_object_kind(parts[0])?;
        let payload = if parts[1] == "-" {
            Vec::new()
        } else {
            decode_hex(parts[1])?
        };
        let record = self.objects.put_git_object(GitObject::new(kind, payload))?;
        self.save_objects()?;
        Ok(DaemonResponse::Ok(format!(
            "oid={} kind={:?} canonical_bytes={} segment_cid={} shards={} available_shards={} durability_satisfied={}",
            record.oid,
            record.kind,
            record.canonical_len,
            record.segment_cid,
            record.shard_cids.len(),
            record.available_shards,
            record.durability_satisfied
        )))
    }

    fn pack_put(&mut self, rest: &str) -> Result<DaemonResponse> {
        let bytes = decode_hex(rest.trim())?;
        let pack = parse_packfile(&bytes)?;
        let mut imported = 0_usize;
        let mut oids = Vec::new();
        for object in pack.objects {
            let record = self.objects.put_git_object(object)?;
            imported += 1;
            oids.push(record.oid.to_string());
        }
        self.save_objects()?;
        Ok(DaemonResponse::Ok(format!(
            "pack_version={} imported={} objects={}",
            pack.version,
            imported,
            if oids.is_empty() {
                "none".to_string()
            } else {
                oids.join(",")
            }
        )))
    }

    fn pack_get(&self, rest: &str) -> Result<DaemonResponse> {
        match rest.trim() {
            "all" => {
                let pack = self.objects.export_pack_all()?;
                Ok(DaemonResponse::Ok(format!(
                    "pack_version=2 objects={} pack_hex={}",
                    self.objects.object_count(),
                    encode_hex(&pack)
                )))
            }
            value => Err(DaemonError::InvalidCommand(format!(
                "PACK_GET supports 'all', got '{value}'"
            ))),
        }
    }

    fn pack_get_reachable(&self, rest: &str) -> Result<DaemonResponse> {
        let tips = parse_oid_list(rest.trim())?;
        let pack = self.objects.export_pack_reachable_from(&tips)?;
        Ok(DaemonResponse::Ok(format!(
            "pack_version=2 tips={} pack_hex={}",
            tips.len(),
            encode_hex(&pack)
        )))
    }

    fn object_get(&self, oid: &str) -> Result<DaemonResponse> {
        let oid = parse_oid(oid.trim())?;
        let object = self.objects.get_git_object(oid)?;
        Ok(DaemonResponse::Ok(format!(
            "oid={} kind={:?} payload_hex={}",
            object.sha1_oid(),
            object.kind,
            encode_hex(&object.payload)
        )))
    }

    fn object_audit(&self, rest: &str) -> Result<DaemonResponse> {
        let rest = rest.trim();
        let audits = if rest == "all" {
            self.objects.audit_all()?
        } else {
            vec![self.objects.audit_object(parse_oid(rest)?)?]
        };
        Ok(DaemonResponse::Ok(format_audit_reports(&audits)))
    }

    fn object_repair(&mut self, rest: &str) -> Result<DaemonResponse> {
        let rest = rest.trim();
        let reports = if rest == "all" {
            self.objects.repair_all()?
        } else {
            vec![self.objects.repair_object(parse_oid(rest)?)?]
        };
        self.save_objects()?;
        Ok(DaemonResponse::Ok(format_repair_reports(&reports)))
    }

    fn key_grant_put(&mut self, rest: &str) -> Result<DaemonResponse> {
        let parts = rest.split_whitespace().collect::<Vec<_>>();
        let grant = RepoKeyGrant::from_wire_fields(&parts)?;
        let grant_id = grant.grant_id();
        let repo_id = grant.repo_id.clone();
        let epoch = grant.epoch;
        let device_id = grant.recipient_device_id.as_cid();
        self.key_grants.insert_grant(grant)?;
        self.save_key_grants()?;
        Ok(DaemonResponse::Ok(format!(
            "grant={} repo={} epoch={} device={} grants={} active={}",
            grant_id,
            repo_id,
            epoch,
            device_id,
            self.key_grants.grant_count(),
            self.key_grants
                .active_grants_for_epoch(&repo_id, epoch)
                .len()
        )))
    }

    fn key_grant_list(&self, rest: &str) -> Result<DaemonResponse> {
        let parts = rest.split_whitespace().collect::<Vec<_>>();
        if parts.is_empty() || parts.len() > 2 {
            return Err(DaemonError::InvalidCommand(
                "KEY_GRANT_LIST requires <repo-id> [latest|all|epoch]".to_string(),
            ));
        }
        let repo_id = parts[0];
        let grants = match parts.get(1).copied().unwrap_or("latest") {
            "all" => self.key_grants.grants_for_repo(repo_id),
            "latest" => self
                .key_grants
                .latest_epoch(repo_id)
                .map_or_else(Vec::new, |epoch| {
                    self.key_grants.grants_for_repo_epoch(repo_id, epoch)
                }),
            value => {
                let epoch = value.parse::<u64>().map_err(|_| {
                    DaemonError::InvalidCommand(
                        "KEY_GRANT_LIST epoch must be a positive integer".to_string(),
                    )
                })?;
                self.key_grants.grants_for_repo_epoch(repo_id, epoch)
            }
        };
        Ok(DaemonResponse::Ok(format_key_grant_list(
            repo_id,
            &self.key_grants,
            &grants,
        )))
    }

    fn key_grant_revoke_device(&mut self, rest: &str) -> Result<DaemonResponse> {
        let parts = rest.split_whitespace().collect::<Vec<_>>();
        if parts.len() != 2 {
            return Err(DaemonError::InvalidCommand(
                "KEY_GRANT_REVOKE_DEVICE requires <device-cid> <effective-epoch>".to_string(),
            ));
        }
        let device_id = DeviceId::from_protocol_cid_text(parts[0])?;
        let effective_epoch = parts[1].parse::<u64>().map_err(|_| {
            DaemonError::InvalidCommand("effective epoch must be a positive integer".to_string())
        })?;
        self.key_grants
            .revoke_device_from_epoch(device_id.clone(), effective_epoch)?;
        self.save_key_grants()?;
        Ok(DaemonResponse::Ok(format!(
            "revoked_device={} effective_epoch={} revoked_devices={}",
            device_id.as_cid(),
            effective_epoch,
            self.key_grants.revoked_device_count()
        )))
    }

    fn key_grant_status(&self, rest: &str) -> Result<DaemonResponse> {
        let repo_id = rest.trim();
        if repo_id.is_empty() || repo_id.contains(char::is_whitespace) {
            return Err(DaemonError::InvalidCommand(
                "KEY_GRANT_STATUS requires <repo-id>".to_string(),
            ));
        }
        let latest_epoch = self
            .key_grants
            .latest_epoch(repo_id)
            .map_or_else(|| "none".to_string(), |epoch| epoch.to_string());
        let active_latest = self.key_grants.latest_epoch(repo_id).map_or(0, |epoch| {
            self.key_grants
                .active_grants_for_epoch(repo_id, epoch)
                .len()
        });
        Ok(DaemonResponse::Ok(format!(
            "repo={} latest_epoch={} grants={} active_latest={} revoked_devices={}",
            repo_id,
            latest_epoch,
            self.key_grants.grants_for_repo(repo_id).len(),
            active_latest,
            self.key_grants.revoked_device_count()
        )))
    }

    fn account_create(&mut self, rest: &str) -> Result<DaemonResponse> {
        let parts = rest.split_whitespace().collect::<Vec<_>>();
        if parts.len() != 5 {
            return Err(DaemonError::InvalidCommand(
                "ACCOUNT_CREATE requires <username> <account-cid> <display-hex|-> <bio-hex|-> <avatar-hex|->".to_string(),
            ));
        }
        let username = Username::new(parts[0])?;
        let account_id = gitmesh_identity::AccountId::from_protocol_cid_text(parts[1])?;
        let profile = self.accounts.create_profile(
            NewAccountProfile {
                username,
                account_id,
                display_name: decode_text_arg(parts[2])?,
                bio: decode_text_arg(parts[3])?,
                avatar_uri: decode_text_arg(parts[4])?,
            },
            now_unix()?,
        )?;
        let response = format_profile(profile);
        self.save_accounts()?;
        Ok(DaemonResponse::Ok(response))
    }

    fn account_profile(&self, rest: &str) -> Result<DaemonResponse> {
        let username = Username::new(rest.trim())?;
        let profile = self
            .accounts
            .profile(&username)
            .ok_or(AccountError::AccountNotFound)?;
        Ok(DaemonResponse::Ok(format_profile(profile)))
    }

    fn account_update_profile(&mut self, rest: &str) -> Result<DaemonResponse> {
        let parts = rest.split_whitespace().collect::<Vec<_>>();
        if parts.len() != 4 {
            return Err(DaemonError::InvalidCommand(
                "ACCOUNT_UPDATE_PROFILE requires <username> <display-hex|keep|-> <bio-hex|keep|-> <avatar-hex|keep|->".to_string(),
            ));
        }
        let username = Username::new(parts[0])?;
        let profile = self.accounts.update_profile(
            &username,
            ProfileUpdate {
                display_name: decode_optional_text_arg(parts[1])?,
                bio: decode_optional_text_arg(parts[2])?,
                avatar_uri: decode_optional_text_arg(parts[3])?,
            },
            now_unix()?,
        )?;
        let response = format_profile(profile);
        self.save_accounts()?;
        Ok(DaemonResponse::Ok(response))
    }

    fn session_issue(&mut self, rest: &str) -> Result<DaemonResponse> {
        let parts = rest.split_whitespace().collect::<Vec<_>>();
        if parts.len() != 3 {
            return Err(DaemonError::InvalidCommand(
                "SESSION_ISSUE requires <username> <ttl-seconds> <device-id|none>".to_string(),
            ));
        }
        let username = Username::new(parts[0])?;
        let ttl = parts[1].parse::<u64>().map_err(|_| {
            DaemonError::InvalidCommand("ttl seconds must be a positive integer".to_string())
        })?;
        let device_id = if parts[2] == "none" {
            None
        } else {
            Some(parts[2].to_string())
        };
        let issued = self
            .accounts
            .issue_session(&username, device_id, ttl, now_unix()?)?;
        let response = format!(
            "session={} username={} expires_at={} token={}",
            issued.session.session_id,
            issued.session.username.as_str(),
            issued.session.expires_at_unix,
            issued.token.expose_for_client()
        );
        self.save_accounts()?;
        Ok(DaemonResponse::Ok(response))
    }

    fn session_auth(&self, rest: &str) -> Result<DaemonResponse> {
        let session = self
            .accounts
            .authenticate_session(rest.trim(), now_unix()?)?;
        Ok(DaemonResponse::Ok(format!(
            "session={} username={} expires_at={} active=true",
            session.session_id,
            session.username.as_str(),
            session.expires_at_unix
        )))
    }

    fn session_revoke(&mut self, rest: &str) -> Result<DaemonResponse> {
        let session_id = rest.trim();
        self.accounts.revoke_session(session_id, now_unix()?)?;
        self.save_accounts()?;
        Ok(DaemonResponse::Ok(format!(
            "session={session_id} revoked=true"
        )))
    }

    fn repo_register(&mut self, rest: &str) -> Result<DaemonResponse> {
        let parts = rest.split_whitespace().collect::<Vec<_>>();
        if parts.len() != 4 {
            return Err(DaemonError::InvalidCommand(
                "REPO_REGISTER requires <owner> <name> <repo-id> <public|private>".to_string(),
            ));
        }
        let owner = Username::new(parts[0])?;
        let visibility = parse_repository_visibility(parts[3])?;
        let registration = self.accounts.register_repository(
            &owner,
            parts[1],
            parts[2],
            visibility,
            now_unix()?,
        )?;
        let response = format_repository_registration(registration);
        self.save_accounts()?;
        Ok(DaemonResponse::Ok(response))
    }

    fn repo_list(&self, owner: &str) -> Result<DaemonResponse> {
        let owner = Username::new(owner.trim())?;
        Ok(DaemonResponse::Ok(format_repository_list(
            owner.as_str(),
            &self.accounts.repositories_for_owner(&owner),
        )))
    }

    fn repo_get(&self, rest: &str) -> Result<DaemonResponse> {
        let parts = rest.split_whitespace().collect::<Vec<_>>();
        if parts.len() != 2 {
            return Err(DaemonError::InvalidCommand(
                "REPO_GET requires <owner> <name>".to_string(),
            ));
        }
        let owner = Username::new(parts[0])?;
        let registration = self
            .accounts
            .repository(&owner, parts[1])
            .ok_or(gitmesh_accounts::AccountError::InvalidRepositoryName)?;
        Ok(DaemonResponse::Ok(format_repository_registration(
            registration,
        )))
    }

    fn collab_seed_samples(&mut self) -> Result<DaemonResponse> {
        let before = self.collaboration.event_count();
        for event in sample_issue_events()
            .into_iter()
            .chain(sample_pull_request_events())
        {
            self.collaboration.insert(event);
        }
        self.save_collaboration()?;
        let after = self.collaboration.event_count();
        Ok(DaemonResponse::Ok(format!(
            "events={} inserted={}",
            after,
            after.saturating_sub(before)
        )))
    }

    fn issue_list(&self, repo: &str) -> Result<DaemonResponse> {
        validate_repo_selector(repo.trim())?;
        Ok(DaemonResponse::Ok(format_issue_list(
            repo.trim(),
            &self.collaboration.issue_summaries(repo.trim()),
        )))
    }

    fn pr_list(&self, repo: &str) -> Result<DaemonResponse> {
        validate_repo_selector(repo.trim())?;
        Ok(DaemonResponse::Ok(format_pr_list(
            repo.trim(),
            &self.collaboration.pull_request_summaries(repo.trim()),
        )))
    }

    fn save_objects(&self) -> Result<()> {
        if let Some(path) = &self.object_store_path {
            self.objects.save_to_path(path)?;
        }
        Ok(())
    }

    fn save_refs(&self) -> Result<()> {
        if let Some(path) = &self.ref_store_path {
            self.refs.save_to_path(path)?;
        }
        Ok(())
    }

    fn save_policy(&self) -> Result<()> {
        if let Some(path) = &self.policy_store_path {
            self.policy.save_to_path(path)?;
        }
        Ok(())
    }

    fn save_key_grants(&self) -> Result<()> {
        if let Some(path) = &self.key_grant_store_path {
            self.key_grants.save_to_path(path)?;
        }
        Ok(())
    }

    fn save_accounts(&self) -> Result<()> {
        if let Some(path) = &self.account_store_path {
            self.accounts.save_to_path(path)?;
        }
        Ok(())
    }

    fn save_collaboration(&self) -> Result<()> {
        if let Some(path) = &self.collaboration_store_path {
            self.collaboration.save_to_path(path)?;
        }
        Ok(())
    }
}

impl DaemonResponse {
    pub fn into_line(self) -> String {
        self.into_protocol_line(None)
    }

    pub fn into_protocol_line(self, request_id: Option<&str>) -> String {
        let id = request_id.map_or_else(String::new, |request_id| format!(" id={request_id}"));
        match self {
            Self::Ok(message) => format!("OK{id} {message}"),
            Self::Error(message) => format!("ERR{id} {message}"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ClientRequest<'a> {
    request_id: Option<&'a str>,
    command: &'a str,
}

fn parse_client_request(line: &str) -> Result<ClientRequest<'_>> {
    let trimmed = line.trim_end_matches(['\n', '\r']);
    if trimmed.is_empty() {
        return Err(DaemonError::EmptyRequest);
    }
    if let Some(rest) = trimmed.strip_prefix("GMD1 ") {
        let (request_id, command) = rest.split_once(' ').ok_or(DaemonError::EmptyRequest)?;
        validate_request_id(request_id)?;
        if command.trim().is_empty() {
            return Err(DaemonError::EmptyRequest);
        }
        Ok(ClientRequest {
            request_id: Some(request_id),
            command,
        })
    } else {
        Ok(ClientRequest {
            request_id: None,
            command: trimmed,
        })
    }
}

fn validate_request_id(request_id: &str) -> Result<()> {
    if request_id.is_empty()
        || request_id.len() > MAX_REQUEST_ID_BYTES
        || !request_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(DaemonError::InvalidRequestId);
    }
    Ok(())
}

fn validate_admin_token(token: &str) -> Result<()> {
    if token.len() < 16
        || token.len() > 256
        || token
            .bytes()
            .any(|byte| byte <= 0x20 || byte == 0x7f || matches!(byte, b'\'' | b'"'))
    {
        return Err(DaemonError::InvalidCommand(
            "admin token must be 16-256 printable non-whitespace ASCII bytes".to_string(),
        ));
    }
    Ok(())
}

fn authorize_command<'a>(auth: &DaemonAuth, command: &'a str) -> Result<&'a str> {
    if !auth.is_enabled() {
        return Ok(command);
    }
    if let Some(rest) = command.strip_prefix("AUTH ") {
        let (token, inner_command) = rest.split_once(' ').ok_or(DaemonError::Unauthorized)?;
        if inner_command.trim().is_empty() {
            return Err(DaemonError::EmptyRequest);
        }
        return match auth.admin_token.as_deref() {
            Some(expected) if constant_time_eq(token.as_bytes(), expected.as_bytes()) => {
                Ok(inner_command)
            }
            _ => Err(DaemonError::Unauthorized),
        };
    }
    if requires_admin_auth(command) {
        Err(DaemonError::Unauthorized)
    } else {
        Ok(command)
    }
}

fn requires_admin_auth(command: &str) -> bool {
    matches!(
        command.split_whitespace().next(),
        Some(
            "V0_PROOF"
                | "REF_UPDATE"
                | "REF_UPDATE_FORCE"
                | "REF_UPDATE_SIGNED"
                | "REF_UPDATE_SIGNED_FORCE"
                | "POLICY_SET_REQUIRE_SIGNED"
                | "POLICY_GRANT_WRITER_ACCOUNT"
                | "POLICY_GRANT_FORCE_ACCOUNT"
                | "POLICY_PROTECT_REF"
                | "PACK_PUT"
                | "OBJECT_PUT"
                | "OBJECT_REPAIR"
                | "KEY_GRANT_PUT"
                | "KEY_GRANT_REVOKE_DEVICE"
                | "ACCOUNT_CREATE"
                | "ACCOUNT_UPDATE_PROFILE"
                | "SESSION_ISSUE"
                | "SESSION_REVOKE"
                | "REPO_REGISTER"
                | "COLLAB_SEED_SAMPLES"
        )
    )
}

fn maybe_auth_wrap_command(command: &str) -> Result<String> {
    if command.starts_with("AUTH ") || !requires_admin_auth(command) {
        return Ok(command.to_string());
    }
    match std::env::var("GITMESHD_ADMIN_TOKEN") {
        Ok(token) if !token.is_empty() => {
            validate_admin_token(&token)?;
            Ok(format!("AUTH {token} {command}"))
        }
        _ => Ok(command.to_string()),
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |acc, (left, right)| acc | (left ^ right))
        == 0
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BinaryDaemonFrame {
    pub request_id: String,
    pub is_error: bool,
    pub payload: Vec<u8>,
}

impl BinaryDaemonFrame {
    pub fn request(request_id: impl Into<String>, command: impl AsRef<[u8]>) -> Result<Self> {
        let request_id = request_id.into();
        validate_request_id(&request_id)?;
        let payload = command.as_ref().to_vec();
        if payload.is_empty() {
            return Err(DaemonError::EmptyRequest);
        }
        validate_frame_lengths(request_id.as_bytes(), &payload)?;
        Ok(Self {
            request_id,
            is_error: false,
            payload,
        })
    }

    pub fn response(request_id: impl Into<String>, response: DaemonResponse) -> Result<Self> {
        let request_id = request_id.into();
        validate_request_id(&request_id)?;
        let (is_error, payload) = match response {
            DaemonResponse::Ok(message) => (false, message.into_bytes()),
            DaemonResponse::Error(message) => (true, message.into_bytes()),
        };
        validate_frame_lengths(request_id.as_bytes(), &payload)?;
        Ok(Self {
            request_id,
            is_error,
            payload,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        validate_request_id(&self.request_id)?;
        validate_frame_lengths(self.request_id.as_bytes(), &self.payload)?;
        let mut out = Vec::with_capacity(
            DAEMON_FRAME_HEADER_BYTES + self.request_id.len() + self.payload.len(),
        );
        out.extend_from_slice(DAEMON_FRAME_MAGIC);
        out.push(DAEMON_FRAME_VERSION);
        out.push(u8::from(self.is_error) & DAEMON_FRAME_FLAG_ERROR);
        out.extend_from_slice(&(self.request_id.len() as u16).to_be_bytes());
        out.extend_from_slice(&(self.payload.len() as u32).to_be_bytes());
        out.extend_from_slice(&[0_u8; 2]);
        out.extend_from_slice(self.request_id.as_bytes());
        out.extend_from_slice(&self.payload);
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < DAEMON_FRAME_HEADER_BYTES
            || &bytes[..4] != DAEMON_FRAME_MAGIC
            || bytes[12] != 0
            || bytes[13] != 0
        {
            return Err(DaemonError::InvalidFrame);
        }
        let version = bytes[4];
        if version != DAEMON_FRAME_VERSION {
            return Err(DaemonError::UnsupportedFrameVersion(version));
        }
        let flags = bytes[5];
        if flags & !DAEMON_FRAME_FLAG_ERROR != 0 {
            return Err(DaemonError::InvalidFrame);
        }
        let request_id_len = u16::from_be_bytes([bytes[6], bytes[7]]) as usize;
        let payload_len = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
        let expected_len = DAEMON_FRAME_HEADER_BYTES
            .checked_add(request_id_len)
            .and_then(|len| len.checked_add(payload_len))
            .ok_or(DaemonError::InvalidFrame)?;
        if bytes.len() != expected_len {
            return Err(DaemonError::InvalidFrame);
        }
        if payload_len > MAX_DAEMON_COMMAND_BYTES || request_id_len > MAX_REQUEST_ID_BYTES {
            return Err(DaemonError::RequestTooLarge);
        }
        let request_id_start = DAEMON_FRAME_HEADER_BYTES;
        let payload_start = request_id_start + request_id_len;
        let request_id = String::from_utf8(bytes[request_id_start..payload_start].to_vec())
            .map_err(|_| DaemonError::InvalidRequestId)?;
        validate_request_id(&request_id)?;
        Ok(Self {
            request_id,
            is_error: flags & DAEMON_FRAME_FLAG_ERROR != 0,
            payload: bytes[payload_start..].to_vec(),
        })
    }

    pub fn payload_text(&self) -> Result<&str> {
        std::str::from_utf8(&self.payload).map_err(|_| {
            DaemonError::InvalidCommand("daemon frame payload must be valid UTF-8".to_string())
        })
    }
}

fn validate_frame_lengths(request_id: &[u8], payload: &[u8]) -> Result<()> {
    if request_id.is_empty() || request_id.len() > MAX_REQUEST_ID_BYTES {
        return Err(DaemonError::InvalidRequestId);
    }
    if payload.len() > MAX_DAEMON_COMMAND_BYTES {
        return Err(DaemonError::RequestTooLarge);
    }
    Ok(())
}

pub fn handle_daemon_command(line: &str) -> Result<DaemonResponse> {
    DaemonState::default().handle_command(line)
}

#[cfg(unix)]
pub fn serve_unix_socket(socket_path: impl AsRef<Path>) -> Result<()> {
    serve_unix_socket_with_store(socket_path, None::<PathBuf>)
}

#[cfg(unix)]
pub fn serve_unix_socket_with_store(
    socket_path: impl AsRef<Path>,
    object_store_path: Option<impl Into<PathBuf>>,
) -> Result<()> {
    serve_unix_socket_with_stores(
        socket_path,
        object_store_path,
        None::<PathBuf>,
        None::<PathBuf>,
    )
}

#[cfg(unix)]
pub fn serve_unix_socket_with_stores(
    socket_path: impl AsRef<Path>,
    object_store_path: Option<impl Into<PathBuf>>,
    ref_store_path: Option<impl Into<PathBuf>>,
    policy_store_path: Option<impl Into<PathBuf>>,
) -> Result<()> {
    serve_unix_socket_with_stores_and_auth(
        socket_path,
        DaemonStorePaths {
            object_store_path: object_store_path.map(Into::into),
            ref_store_path: ref_store_path.map(Into::into),
            policy_store_path: policy_store_path.map(Into::into),
            ..DaemonStorePaths::default()
        },
        DaemonAuth::disabled(),
    )
}

#[cfg(unix)]
pub fn serve_unix_socket_with_stores_and_auth(
    socket_path: impl AsRef<Path>,
    stores: DaemonStorePaths,
    auth: DaemonAuth,
) -> Result<()> {
    let socket_path = socket_path.as_ref();
    remove_stale_socket(socket_path)?;
    let listener = UnixListener::bind(socket_path)?;
    let state = Arc::new(Mutex::new(
        DaemonState::with_all_store_paths_and_collaboration(
            stores.object_store_path,
            stores.ref_store_path,
            stores.policy_store_path,
            stores.key_grant_store_path,
            stores.account_store_path,
            stores.collaboration_store_path,
        )?,
    ));
    let auth = Arc::new(auth);
    for stream in listener.incoming() {
        let stream = stream?;
        let state = Arc::clone(&state);
        let auth = Arc::clone(&auth);
        thread::spawn(move || {
            if let Err(err) = handle_unix_stream(stream, state, auth) {
                eprintln!("gitmeshd socket handler failed: {err}");
            }
        });
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn serve_unix_socket(_socket_path: impl AsRef<Path>) -> Result<()> {
    Err(DaemonError::UnsupportedPlatform)
}

#[cfg(not(unix))]
pub fn serve_unix_socket_with_store(
    _socket_path: impl AsRef<Path>,
    _object_store_path: Option<impl Into<PathBuf>>,
) -> Result<()> {
    Err(DaemonError::UnsupportedPlatform)
}

#[cfg(not(unix))]
pub fn serve_unix_socket_with_stores(
    _socket_path: impl AsRef<Path>,
    _object_store_path: Option<impl Into<PathBuf>>,
    _ref_store_path: Option<impl Into<PathBuf>>,
    _policy_store_path: Option<impl Into<PathBuf>>,
) -> Result<()> {
    Err(DaemonError::UnsupportedPlatform)
}

#[cfg(not(unix))]
pub fn serve_unix_socket_with_stores_and_auth(
    _socket_path: impl AsRef<Path>,
    _stores: DaemonStorePaths,
    _auth: DaemonAuth,
) -> Result<()> {
    Err(DaemonError::UnsupportedPlatform)
}

#[cfg(unix)]
pub fn request_unix_socket(socket_path: impl AsRef<Path>, command: &str) -> Result<String> {
    let mut stream = UnixStream::connect(socket_path)?;
    writeln!(stream, "{}", maybe_auth_wrap_command(command)?)?;
    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response)?;
    Ok(response.trim_end().to_string())
}

#[cfg(unix)]
pub fn request_unix_socket_v1(
    socket_path: impl AsRef<Path>,
    request_id: &str,
    command: &str,
) -> Result<String> {
    validate_request_id(request_id)?;
    request_unix_socket(
        socket_path,
        &format!("GMD1 {request_id} {}", maybe_auth_wrap_command(command)?),
    )
}

#[cfg(unix)]
pub fn request_unix_socket_frame(
    socket_path: impl AsRef<Path>,
    request_id: &str,
    command: &str,
) -> Result<BinaryDaemonFrame> {
    let mut stream = UnixStream::connect(socket_path)?;
    let request = BinaryDaemonFrame::request(request_id, maybe_auth_wrap_command(command)?)?;
    stream.write_all(&request.encode()?)?;
    read_binary_frame(&mut stream)
}

#[cfg(not(unix))]
pub fn request_unix_socket(_socket_path: impl AsRef<Path>, _command: &str) -> Result<String> {
    Err(DaemonError::UnsupportedPlatform)
}

#[cfg(not(unix))]
pub fn request_unix_socket_v1(
    _socket_path: impl AsRef<Path>,
    _request_id: &str,
    _command: &str,
) -> Result<String> {
    Err(DaemonError::UnsupportedPlatform)
}

#[cfg(not(unix))]
pub fn request_unix_socket_frame(
    _socket_path: impl AsRef<Path>,
    _request_id: &str,
    _command: &str,
) -> Result<BinaryDaemonFrame> {
    Err(DaemonError::UnsupportedPlatform)
}

#[cfg(unix)]
fn handle_unix_stream(
    mut stream: UnixStream,
    state: Arc<Mutex<DaemonState>>,
    auth: Arc<DaemonAuth>,
) -> Result<()> {
    let first = read_initial_bytes(&mut stream)?;
    if first == *DAEMON_FRAME_MAGIC {
        let frame = read_binary_frame_after_magic(&mut stream);
        let response = handle_binary_frame_for_stream(frame, state, auth)?;
        stream.write_all(&response.encode()?)?;
    } else {
        let mut reader = BufReader::new(stream.try_clone()?);
        let line = read_bounded_line_with_prefix(&mut reader, first.to_vec())?;
        let response = handle_text_line_for_stream(&line, state, &auth);
        writeln!(stream, "{response}")?;
    }
    Ok(())
}

fn read_initial_bytes(reader: &mut impl Read) -> Result<[u8; 4]> {
    let mut first = [0_u8; 4];
    reader.read_exact(&mut first)?;
    Ok(first)
}

#[cfg(unix)]
fn handle_text_line_for_stream(
    line: &str,
    state: Arc<Mutex<DaemonState>>,
    auth: &DaemonAuth,
) -> String {
    let request = parse_client_request(line);
    match request {
        Ok(request) => {
            let command = match authorize_command(auth, request.command) {
                Ok(command) => command,
                Err(err) => {
                    return DaemonResponse::Error(err.to_string())
                        .into_protocol_line(request.request_id);
                }
            };
            let mut state = match state.lock() {
                Ok(state) => state,
                Err(_) => {
                    return DaemonResponse::Error(DaemonError::StatePoisoned.to_string())
                        .into_line();
                }
            };
            match state.handle_command(command) {
                Ok(response) => response.into_protocol_line(request.request_id),
                Err(err) => {
                    DaemonResponse::Error(err.to_string()).into_protocol_line(request.request_id)
                }
            }
        }
        Err(err) => DaemonResponse::Error(err.to_string()).into_line(),
    }
}

#[cfg(unix)]
fn handle_binary_frame_for_stream(
    frame: Result<BinaryDaemonFrame>,
    state: Arc<Mutex<DaemonState>>,
    auth: Arc<DaemonAuth>,
) -> Result<BinaryDaemonFrame> {
    let frame = frame?;
    let response = match frame.payload_text() {
        Ok(command) => {
            let command = match authorize_command(&auth, command) {
                Ok(command) => command,
                Err(err) => {
                    return BinaryDaemonFrame::response(
                        frame.request_id,
                        DaemonResponse::Error(err.to_string()),
                    );
                }
            };
            let mut state = state.lock().map_err(|_| DaemonError::StatePoisoned)?;
            match state.handle_command(command) {
                Ok(response) => response,
                Err(err) => DaemonResponse::Error(err.to_string()),
            }
        }
        Err(err) => DaemonResponse::Error(err.to_string()),
    };
    BinaryDaemonFrame::response(frame.request_id, response)
}

fn read_binary_frame(reader: &mut impl Read) -> Result<BinaryDaemonFrame> {
    let first = read_initial_bytes(reader)?;
    if first != *DAEMON_FRAME_MAGIC {
        return Err(DaemonError::InvalidFrame);
    }
    read_binary_frame_after_magic(reader)
}

fn read_binary_frame_after_magic(reader: &mut impl Read) -> Result<BinaryDaemonFrame> {
    let mut header = [0_u8; DAEMON_FRAME_HEADER_BYTES];
    header[..4].copy_from_slice(DAEMON_FRAME_MAGIC);
    reader.read_exact(&mut header[4..])?;
    let request_id_len = u16::from_be_bytes([header[6], header[7]]) as usize;
    let payload_len = u32::from_be_bytes([header[8], header[9], header[10], header[11]]) as usize;
    if request_id_len > MAX_REQUEST_ID_BYTES || payload_len > MAX_DAEMON_COMMAND_BYTES {
        return Err(DaemonError::RequestTooLarge);
    }
    let mut bytes = header.to_vec();
    bytes.resize(DAEMON_FRAME_HEADER_BYTES + request_id_len + payload_len, 0);
    reader.read_exact(&mut bytes[DAEMON_FRAME_HEADER_BYTES..])?;
    BinaryDaemonFrame::decode(&bytes)
}

#[cfg(test)]
fn read_bounded_line(reader: &mut impl BufRead) -> Result<String> {
    read_bounded_line_with_prefix(reader, Vec::new())
}

fn read_bounded_line_with_prefix(reader: &mut impl BufRead, mut line: Vec<u8>) -> Result<String> {
    if line.len() > MAX_DAEMON_COMMAND_BYTES {
        return Err(DaemonError::RequestTooLarge);
    }
    if let Some(newline_index) = line.iter().position(|byte| *byte == b'\n') {
        line.truncate(newline_index + 1);
        return String::from_utf8(line).map_err(|_| {
            DaemonError::InvalidCommand("daemon commands must be valid UTF-8".to_string())
        });
    }
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            break;
        }
        if line.len() + available.len() > MAX_DAEMON_COMMAND_BYTES {
            return Err(DaemonError::RequestTooLarge);
        }
        if let Some(newline_index) = available.iter().position(|byte| *byte == b'\n') {
            line.extend_from_slice(&available[..=newline_index]);
            reader.consume(newline_index + 1);
            break;
        }
        let consumed = available.len();
        line.extend_from_slice(available);
        reader.consume(consumed);
    }
    String::from_utf8(line)
        .map_err(|_| DaemonError::InvalidCommand("daemon commands must be valid UTF-8".to_string()))
}

#[cfg(test)]
fn handle_unix_stream_for_test(line: &str, state: &mut DaemonState) -> String {
    let request = parse_client_request(line);
    match request {
        Ok(request) => match state.handle_command(request.command) {
            Ok(response) => response.into_protocol_line(request.request_id),
            Err(err) => {
                DaemonResponse::Error(err.to_string()).into_protocol_line(request.request_id)
            }
        },
        Err(err) => DaemonResponse::Error(err.to_string()).into_line(),
    }
}

fn parse_optional_oid(value: &str) -> Result<Option<GitSha1Oid>> {
    if value == "none" {
        Ok(None)
    } else {
        Ok(Some(GitSha1Oid::from_str(value)?))
    }
}

fn parse_oid_list(value: &str) -> Result<Vec<GitSha1Oid>> {
    if value.is_empty() || value == "none" {
        return Err(DaemonError::InvalidCommand(
            "expected one or more comma-separated Git object IDs".to_string(),
        ));
    }
    value
        .split(',')
        .map(|oid| GitSha1Oid::from_str(oid).map_err(DaemonError::Git))
        .collect()
}

fn decode_label(value: &str) -> Result<String> {
    String::from_utf8(decode_hex(value)?)
        .map_err(|_| DaemonError::InvalidCommand("label must be UTF-8 encoded as hex".to_string()))
}

fn decode_text_arg(value: &str) -> Result<String> {
    if value == "-" {
        return Ok(String::new());
    }
    decode_label(value)
}

fn decode_optional_text_arg(value: &str) -> Result<Option<String>> {
    if value == "keep" {
        Ok(None)
    } else {
        Ok(Some(decode_text_arg(value)?))
    }
}

fn decode_fixed_hex<const N: usize>(value: &str) -> Result<[u8; N]> {
    let bytes = decode_hex(value)?;
    bytes
        .try_into()
        .map_err(|_| DaemonError::InvalidCommand(format!("expected {N} bytes encoded as hex")))
}

fn parse_bool_arg(value: &str) -> Result<bool> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(DaemonError::InvalidCommand(
            "expected boolean value 'true' or 'false'".to_string(),
        )),
    }
}

fn parse_repository_visibility(value: &str) -> Result<RepositoryVisibility> {
    match value {
        "public" => Ok(RepositoryVisibility::Public),
        "private" => Ok(RepositoryVisibility::Private),
        _ => Err(DaemonError::InvalidCommand(
            "repository visibility must be public or private".to_string(),
        )),
    }
}

fn validate_repo_selector(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 160
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
    {
        return Err(DaemonError::InvalidCommand(
            "repository selector must be owner/repo".to_string(),
        ));
    }
    Ok(())
}

fn parse_policy_identity(value: &str) -> Result<&str> {
    if value.starts_with("gitmesh:v0:ProtocolObject:Blake3_256:")
        && !value.contains(char::is_whitespace)
    {
        Ok(value)
    } else {
        Err(DaemonError::InvalidCommand(
            "expected GitMesh account identity CID".to_string(),
        ))
    }
}

fn format_receipt(receipt: TransactionReceipt) -> String {
    match receipt {
        TransactionReceipt::Committed {
            ref_name,
            old_oid,
            new_oid,
            ..
        } => format!(
            "status=committed ref={} old={} new={}",
            ref_name.as_str(),
            format_optional_oid(old_oid),
            format_optional_oid(new_oid)
        ),
        TransactionReceipt::Rejected { reason, .. } => match reason {
            RejectionReason::Conflict { expected, actual } => format!(
                "status=rejected reason=conflict expected={} actual={}",
                format_optional_oid(expected),
                format_optional_oid(actual)
            ),
            RejectionReason::IdempotencyViolation => {
                "status=rejected reason=idempotency_violation".to_string()
            }
        },
    }
}

fn format_policy(policy: &RepoPolicy) -> String {
    format!(
        "require_signed_refs={} writers={} force_pushers={} protected_refs={}",
        policy.require_signed_refs(),
        policy.writer_count(),
        policy.force_pusher_count(),
        policy.protected_ref_count()
    )
}

fn format_optional_oid(oid: Option<GitSha1Oid>) -> String {
    oid.map_or_else(|| "none".to_string(), GitSha1Oid::hex)
}

fn format_checkpoint(refs: &RefStore) -> String {
    refs.latest_checkpoint().map_or_else(
        || "checkpoint=none".to_string(),
        |checkpoint| {
            format!(
                "checkpoint={} sequence={} parent={} refs_root={} history_root={}",
                checkpoint.checkpoint_cid,
                checkpoint.sequence,
                checkpoint
                    .parent
                    .map_or_else(|| "none".to_string(), |cid| cid.to_string()),
                checkpoint.refs_root,
                checkpoint.history_root
            )
        },
    )
}

fn format_ref_list(refs: &RefStore) -> String {
    let parts = refs
        .refs()
        .map(|(ref_name, oid)| format!("{}:{}", ref_name.as_str(), oid))
        .collect::<Vec<_>>();
    if parts.is_empty() {
        "refs=none".to_string()
    } else {
        format!("refs={}", parts.join(","))
    }
}

fn format_object_list(objects: &RepositoryObjectStore) -> String {
    let mut parts = Vec::new();
    for record in objects.records() {
        parts.push(format!(
            "{}:{:?}:{}:{}",
            record.oid, record.kind, record.canonical_len, record.durability_satisfied
        ));
    }
    if parts.is_empty() {
        "objects=none".to_string()
    } else {
        format!("objects={}", parts.join(","))
    }
}

fn format_key_grant_list(
    repo_id: &str,
    store: &RepoKeyGrantStore,
    grants: &[&RepoKeyGrant],
) -> String {
    let latest_epoch = store
        .latest_epoch(repo_id)
        .map_or_else(|| "none".to_string(), |epoch| epoch.to_string());
    if grants.is_empty() {
        return format!(
            "repo={} latest_epoch={} grants=none active=0 revoked_devices={}",
            repo_id,
            latest_epoch,
            store.revoked_device_count()
        );
    }
    let entries = grants
        .iter()
        .map(|grant| {
            let active =
                !store.is_device_revoked_for_epoch(&grant.recipient_device_id, grant.epoch);
            format!(
                "{}:{}:{}:active={}",
                grant.grant_id(),
                grant.epoch,
                grant.recipient_device_id.as_cid(),
                active
            )
        })
        .collect::<Vec<_>>();
    format!(
        "repo={} latest_epoch={} grants={} active={} revoked_devices={}",
        repo_id,
        latest_epoch,
        entries.join(","),
        grants
            .iter()
            .filter(
                |grant| !store.is_device_revoked_for_epoch(&grant.recipient_device_id, grant.epoch)
            )
            .count(),
        store.revoked_device_count()
    )
}

fn format_profile(profile: &gitmesh_accounts::AccountProfile) -> String {
    format!(
        "username={} account={} display_hex={} bio_hex={} avatar_hex={} created_at={} updated_at={}",
        profile.username.as_str(),
        profile.account_id,
        encode_hex(profile.display_name.as_bytes()),
        encode_hex(profile.bio.as_bytes()),
        encode_hex(profile.avatar_uri.as_bytes()),
        profile.created_at_unix,
        profile.updated_at_unix
    )
}

fn format_repository_registration(
    registration: &gitmesh_accounts::RepositoryRegistration,
) -> String {
    format!(
        "owner={} name={} repo={} visibility={} created_at={}",
        registration.owner.as_str(),
        registration.name,
        registration.repo_id,
        registration.visibility.as_str(),
        registration.created_at_unix
    )
}

fn format_repository_list(
    owner: &str,
    registrations: &[&gitmesh_accounts::RepositoryRegistration],
) -> String {
    if registrations.is_empty() {
        return format!("owner={owner} repos=none count=0");
    }
    let entries = registrations
        .iter()
        .map(|registration| {
            format!(
                "{};{};{};{}",
                registration.name,
                registration.repo_id,
                registration.visibility.as_str(),
                registration.created_at_unix
            )
        })
        .collect::<Vec<_>>();
    format!(
        "owner={owner} repos={} count={}",
        entries.join("|"),
        registrations.len()
    )
}

fn format_issue_list(repo: &str, issues: &[IssueSummary]) -> String {
    if issues.is_empty() {
        return format!("repo={repo} issues=none count=0");
    }
    let entries = issues
        .iter()
        .map(|issue| {
            format!(
                "{};{};{};{};{}",
                issue.number,
                encode_hex(issue.title.as_bytes()),
                issue.actor,
                encode_label_list(&issue.labels),
                issue.event_id.as_hex()
            )
        })
        .collect::<Vec<_>>();
    format!(
        "repo={repo} issues={} count={}",
        entries.join("|"),
        issues.len()
    )
}

fn format_pr_list(repo: &str, pull_requests: &[PullRequestSummary]) -> String {
    if pull_requests.is_empty() {
        return format!("repo={repo} prs=none count=0");
    }
    let entries = pull_requests
        .iter()
        .map(|pr| {
            format!(
                "{};{};{};{};{};{};{}",
                pr.number,
                encode_hex(pr.title.as_bytes()),
                pr.actor,
                pr.source_ref,
                pr.target_ref,
                encode_label_list(&pr.labels),
                pr.event_id.as_hex()
            )
        })
        .collect::<Vec<_>>();
    format!(
        "repo={repo} prs={} count={}",
        entries.join("|"),
        pull_requests.len()
    )
}

fn encode_label_list(labels: &[String]) -> String {
    if labels.is_empty() {
        return "-".to_string();
    }
    labels
        .iter()
        .map(|label| encode_hex(label.as_bytes()))
        .collect::<Vec<_>>()
        .join(",")
}

fn format_account_status(accounts: &AccountStore) -> Result<String> {
    Ok(format!(
        "accounts={} active_sessions={} registered_repos={}",
        accounts.profile_count(),
        accounts.active_session_count(now_unix()?),
        accounts.repository_count()
    ))
}

fn format_audit_reports(audits: &[RepositoryObjectAudit]) -> String {
    if audits.is_empty() {
        return "audits=none unhealthy=0 repair_needed=0".to_string();
    }
    let unhealthy = audits
        .iter()
        .filter(|audit| !audit.report.durability_satisfied)
        .count();
    let repair_needed = audits
        .iter()
        .filter(|audit| audit.report.repair_needed)
        .count();
    let entries = audits
        .iter()
        .map(|audit| {
            format!(
                "{}:{:?}:verified={}:missing={}:corrupt={}:durable={}:repair={}",
                audit.oid,
                audit.kind,
                audit.report.verified_shards.len(),
                audit.report.missing_shards.len(),
                audit.report.corrupt_shards.len(),
                audit.report.durability_satisfied,
                audit.report.repair_needed
            )
        })
        .collect::<Vec<_>>();
    format!(
        "audits={} unhealthy={} repair_needed={}",
        entries.join(","),
        unhealthy,
        repair_needed
    )
}

fn format_repair_reports(reports: &[RepositoryRepairReport]) -> String {
    if reports.is_empty() {
        return "repairs=none repaired=0 durable=0".to_string();
    }
    let repaired = reports
        .iter()
        .filter(|report| !report.outcome.repaired_shards.is_empty())
        .count();
    let durable = reports
        .iter()
        .filter(|report| report.outcome.durability_satisfied)
        .count();
    let entries = reports
        .iter()
        .map(|report| {
            format!(
                "{}:{:?}:repaired={}:verified={}:durable={}",
                report.oid,
                report.kind,
                report.outcome.repaired_shards.len(),
                report.outcome.verified_after_repair,
                report.outcome.durability_satisfied
            )
        })
        .collect::<Vec<_>>();
    format!(
        "repairs={} repaired={} durable={}",
        entries.join(","),
        repaired,
        durable
    )
}

fn format_transport_repair_proof(proof: &RepositoryTransportRepairProof) -> String {
    format!(
        "oid={} recovered_exactly={} repaired_shards={} original_peer={} replacement_peer={} providers={} verified_after_repair={} durability_satisfied={}",
        proof.oid,
        proof.recovered_exactly,
        proof
            .repaired_shards
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(","),
        proof.original_peer,
        proof.replacement_peer,
        proof.provider_count,
        proof.verified_after_repair,
        proof.durability_satisfied
    )
}

fn remove_stale_socket(socket_path: &Path) -> Result<()> {
    if fs::symlink_metadata(socket_path).is_ok() {
        fs::remove_file(socket_path)?;
    }
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

pub fn default_socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os("GITMESHD_SOCKET") {
        return PathBuf::from(path);
    }
    std::env::temp_dir().join("gitmeshd.sock")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    use gitmesh_crypto::RepoContentKey;
    use gitmesh_git::{GitObjectKind, write_packfile};
    use gitmesh_identity::{AccountRootKey, DeviceKey};

    fn put_object(state: &mut DaemonState, kind: &str, payload_hex: &str) -> String {
        state
            .handle_command(&format!("OBJECT_PUT {kind} {payload_hex}"))
            .unwrap()
            .into_line()
            .split_whitespace()
            .find_map(|part| part.strip_prefix("oid="))
            .unwrap()
            .to_string()
    }

    fn put_root_commit(state: &mut DaemonState, message: &str) -> String {
        let tree_oid = put_object(state, "tree", "-");
        let payload = format!(
            "tree {tree_oid}\nauthor A <a@example.com> 0 +0000\ncommitter A <a@example.com> 0 +0000\n\n{message}\n"
        );
        put_object(state, "commit", &encode_hex(payload.as_bytes()))
    }

    fn put_child_commit(state: &mut DaemonState, parent_oid: &str, message: &str) -> String {
        let tree_oid = put_object(state, "tree", "-");
        let payload = format!(
            "tree {tree_oid}\nparent {parent_oid}\nauthor A <a@example.com> 0 +0000\ncommitter A <a@example.com> 0 +0000\n\n{message}\n"
        );
        put_object(state, "commit", &encode_hex(payload.as_bytes()))
    }

    fn account_cid() -> String {
        AccountRootKey::generate().account_id().as_cid().to_string()
    }

    fn repo_key_grant_wire(repo_id: &str, epoch: u64) -> (String, String) {
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let cert = account.certify_device(&device, "laptop");
        let grant = account
            .grant_repo_key_to_device(repo_id, epoch, RepoContentKey::generate(), &device, &cert)
            .unwrap();
        (
            grant.to_wire_string().unwrap(),
            grant.recipient_device_id.as_cid().to_string(),
        )
    }

    #[test]
    fn ping_returns_pong() {
        assert_eq!(
            handle_daemon_command("PING").unwrap(),
            DaemonResponse::Ok("pong".to_string())
        );
    }

    #[test]
    fn v0_proof_reports_exact_recovery() {
        let response = handle_daemon_command("V0_PROOF hello").unwrap().into_line();

        assert!(response.starts_with("OK "));
        assert!(response.contains("recovered_exactly=true"));
        assert!(response.contains("segment_cid=gitmesh:v0:EncryptedSegment"));
    }

    #[test]
    fn network_repair_proof_reports_replacement_recovery() {
        let response = handle_daemon_command("NETWORK_REPAIR_PROOF hello network")
            .unwrap()
            .into_line();

        assert!(response.starts_with("OK "));
        assert!(response.contains("recovered_exactly=true"));
        assert!(response.contains("repaired_shards=3"));
        assert!(response.contains("original_peer=repo-storage-3"));
        assert!(response.contains("replacement_peer=repo-storage-5"));
        assert!(response.contains("durability_satisfied=true"));
    }

    #[test]
    fn client_request_preserves_legacy_command_shape() {
        let request = parse_client_request("PING\n").unwrap();

        assert_eq!(request.request_id, None);
        assert_eq!(request.command, "PING");
    }

    #[test]
    fn client_request_parses_v1_request_id_envelope() {
        let request = parse_client_request("GMD1 req-1 REF_LIST\n").unwrap();

        assert_eq!(request.request_id, Some("req-1"));
        assert_eq!(request.command, "REF_LIST");
    }

    #[test]
    fn client_request_rejects_invalid_v1_request_ids() {
        assert!(matches!(
            parse_client_request("GMD1 bad/id REF_LIST").unwrap_err(),
            DaemonError::InvalidRequestId
        ));
        assert!(matches!(
            parse_client_request("GMD1 req-1").unwrap_err(),
            DaemonError::EmptyRequest
        ));
    }

    #[test]
    fn v1_protocol_response_includes_request_id() {
        let mut state = DaemonState::default();

        let response = handle_unix_stream_for_test("GMD1 req.1 PING\n", &mut state);

        assert_eq!(response, "OK id=req.1 pong");
    }

    #[test]
    fn auth_denies_privileged_command_without_token_wrapper() {
        let auth = DaemonAuth::from_admin_token("0123456789abcdef").unwrap();

        let err = authorize_command(&auth, "OBJECT_PUT blob 6869").unwrap_err();

        assert!(matches!(err, DaemonError::Unauthorized));
    }

    #[test]
    fn auth_strips_valid_token_wrapper() {
        let auth = DaemonAuth::from_admin_token("0123456789abcdef").unwrap();

        let command =
            authorize_command(&auth, "AUTH 0123456789abcdef OBJECT_PUT blob 6869").unwrap();

        assert_eq!(command, "OBJECT_PUT blob 6869");
    }

    #[test]
    fn auth_allows_read_only_commands_without_token() {
        let auth = DaemonAuth::from_admin_token("0123456789abcdef").unwrap();

        assert_eq!(authorize_command(&auth, "PING").unwrap(), "PING");
        assert_eq!(authorize_command(&auth, "REF_LIST").unwrap(), "REF_LIST");
    }

    #[test]
    fn key_grant_put_list_and_revoke_device() {
        let mut state = DaemonState::default();
        let (grant_wire, device_id) = repo_key_grant_wire("repo:farzeen/gitmesh", 1);

        let put = state
            .handle_command(&format!("KEY_GRANT_PUT {grant_wire}"))
            .unwrap()
            .into_line();
        let list = state
            .handle_command("KEY_GRANT_LIST repo:farzeen/gitmesh latest")
            .unwrap()
            .into_line();
        let revoke = state
            .handle_command(&format!("KEY_GRANT_REVOKE_DEVICE {device_id} 1"))
            .unwrap()
            .into_line();
        let list_after_revoke = state
            .handle_command("KEY_GRANT_LIST repo:farzeen/gitmesh latest")
            .unwrap()
            .into_line();

        assert!(put.contains("grants=1"));
        assert!(list.contains("active=1"));
        assert!(revoke.contains("revoked_devices=1"));
        assert!(list_after_revoke.contains("active=0"));
    }

    #[test]
    fn key_grant_store_persists_to_disk() {
        let path = std::env::temp_dir().join(format!(
            "gitmeshd-key-grants-{}-{}.tsv",
            std::process::id(),
            "persist"
        ));
        let (grant_wire, _) = repo_key_grant_wire("repo:farzeen/gitmesh", 2);
        let mut state =
            DaemonState::with_store_paths_and_key_grants(None, None, None, Some(path.clone()))
                .unwrap();

        state
            .handle_command(&format!("KEY_GRANT_PUT {grant_wire}"))
            .unwrap();
        let restored =
            DaemonState::with_store_paths_and_key_grants(None, None, None, Some(path.clone()))
                .unwrap();
        let response = restored
            .key_grant_status("repo:farzeen/gitmesh")
            .unwrap()
            .into_line();
        let _ = fs::remove_file(path);

        assert!(response.contains("latest_epoch=2"));
        assert!(response.contains("grants=1"));
    }

    #[test]
    fn account_commands_create_session_auth_revoke_and_register_repo() {
        let mut state = DaemonState::default();
        let account_id = account_cid();
        let display = encode_hex(b"Farzeen Ilyas");
        let bio = encode_hex(b"building GitMesh");
        let avatar = encode_hex(b"asset:prof_pic.jpeg");

        let create = state
            .handle_command(&format!(
                "ACCOUNT_CREATE farzeen {account_id} {display} {bio} {avatar}"
            ))
            .unwrap()
            .into_line();
        let issue = state
            .handle_command("SESSION_ISSUE farzeen 3600 none")
            .unwrap()
            .into_line();
        let token = issue
            .split_whitespace()
            .find_map(|part| part.strip_prefix("token="))
            .unwrap();
        let session_id = issue
            .split_whitespace()
            .find_map(|part| part.strip_prefix("session="))
            .unwrap()
            .to_string();
        let auth = state
            .handle_command(&format!("SESSION_AUTH {token}"))
            .unwrap()
            .into_line();
        let repo = state
            .handle_command("REPO_REGISTER farzeen GitMesh repo:farzeen/gitmesh private")
            .unwrap()
            .into_line();
        let status = state.handle_command("ACCOUNT_STATUS").unwrap().into_line();
        let revoke = state
            .handle_command(&format!("SESSION_REVOKE {session_id}"))
            .unwrap()
            .into_line();

        assert!(create.contains("username=farzeen"));
        assert!(auth.contains("active=true"));
        assert!(repo.contains("repo=repo:farzeen/gitmesh"));
        assert!(status.contains("accounts=1"));
        assert!(status.contains("registered_repos=1"));
        assert!(
            state
                .handle_command("REPO_LIST farzeen")
                .unwrap()
                .into_line()
                .contains("repos=GitMesh;repo:farzeen/gitmesh;private;")
        );
        assert!(
            state
                .handle_command("REPO_GET farzeen GitMesh")
                .unwrap()
                .into_line()
                .contains("name=GitMesh")
        );
        assert!(revoke.contains("revoked=true"));
        assert!(
            state
                .handle_command(&format!("SESSION_AUTH {token}"))
                .is_err()
        );
    }

    #[test]
    fn account_store_persists_to_disk() {
        let path = std::env::temp_dir().join(format!(
            "gitmeshd-accounts-{}-{}.tsv",
            std::process::id(),
            "persist"
        ));
        let mut state =
            DaemonState::with_all_store_paths(None, None, None, None, Some(path.clone())).unwrap();
        let account_id = account_cid();
        state
            .handle_command(&format!(
                "ACCOUNT_CREATE farzeen {account_id} {} - -",
                encode_hex(b"Farzeen")
            ))
            .unwrap();
        state
            .handle_command("REPO_REGISTER farzeen GitMesh repo:farzeen/gitmesh private")
            .unwrap();

        let mut restored =
            DaemonState::with_all_store_paths(None, None, None, None, Some(path.clone())).unwrap();
        let status = restored
            .handle_command("ACCOUNT_STATUS")
            .unwrap()
            .into_line();
        let profile = restored
            .handle_command("ACCOUNT_PROFILE farzeen")
            .unwrap()
            .into_line();
        let repos = restored
            .handle_command("REPO_LIST farzeen")
            .unwrap()
            .into_line();
        let _ = fs::remove_file(path);

        assert!(status.contains("accounts=1"));
        assert!(status.contains("registered_repos=1"));
        assert!(profile.contains("username=farzeen"));
        assert!(repos.contains("count=1"));
        assert!(repos.contains("GitMesh;repo:farzeen/gitmesh;private;"));
    }

    #[test]
    fn collaboration_commands_seed_and_list_issue_pr_state() {
        let mut state = DaemonState::default();

        let seed = state
            .handle_command("COLLAB_SEED_SAMPLES")
            .unwrap()
            .into_line();
        let second_seed = state
            .handle_command("COLLAB_SEED_SAMPLES")
            .unwrap()
            .into_line();
        let issues = state
            .handle_command("ISSUE_LIST farzeen/gitmesh")
            .unwrap()
            .into_line();
        let prs = state
            .handle_command("PR_LIST farzeen/gitmesh")
            .unwrap()
            .into_line();
        let status = state.handle_command("REPO_STATUS").unwrap().into_line();

        assert!(seed.contains("events=4"));
        assert!(seed.contains("inserted=4"));
        assert!(second_seed.contains("inserted=0"));
        assert!(issues.contains("count=2"));
        assert!(
            issues.contains("5065727369737420636f6c6c61626f726174696f6e206576656e74206c6f6773")
        );
        assert!(prs.contains("count=2"));
        assert!(prs.contains("refs/heads/collaboration-cli"));
        assert!(status.contains("collaboration_events=4"));
    }

    #[test]
    fn collaboration_store_persists_to_disk() {
        let path = std::env::temp_dir().join(format!(
            "gitmeshd-collaboration-{}-{}.tsv",
            std::process::id(),
            "persist"
        ));
        let mut state = DaemonState::with_all_store_paths_and_collaboration(
            None,
            None,
            None,
            None,
            None,
            Some(path.clone()),
        )
        .unwrap();
        state.handle_command("COLLAB_SEED_SAMPLES").unwrap();

        let mut restored = DaemonState::with_all_store_paths_and_collaboration(
            None,
            None,
            None,
            None,
            None,
            Some(path.clone()),
        )
        .unwrap();
        let issues = restored
            .handle_command("ISSUE_LIST farzeen/gitmesh")
            .unwrap()
            .into_line();
        let _ = fs::remove_file(path);

        assert!(issues.contains("count=2"));
    }

    #[cfg(unix)]
    #[test]
    fn text_stream_handler_enforces_auth() {
        let state = Arc::new(Mutex::new(DaemonState::default()));
        let auth = DaemonAuth::from_admin_token("0123456789abcdef").unwrap();

        let response = handle_text_line_for_stream(
            "GMD1 req-1 OBJECT_PUT blob 6869\n",
            Arc::clone(&state),
            &auth,
        );

        assert_eq!(
            response,
            "ERR id=req-1 daemon command requires admin authentication"
        );
    }

    #[test]
    fn bounded_line_reader_rejects_oversized_commands() {
        let command = vec![b'a'; MAX_DAEMON_COMMAND_BYTES + 1];
        let mut reader = Cursor::new(command);

        let err = read_bounded_line(&mut reader).unwrap_err();

        assert!(matches!(err, DaemonError::RequestTooLarge));
    }

    #[test]
    fn binary_frame_round_trips_request_payload() {
        let frame = BinaryDaemonFrame::request("req-1", "PING").unwrap();
        let encoded = frame.encode().unwrap();
        let decoded = BinaryDaemonFrame::decode(&encoded).unwrap();

        assert_eq!(decoded.request_id, "req-1");
        assert!(!decoded.is_error);
        assert_eq!(decoded.payload_text().unwrap(), "PING");
    }

    #[test]
    fn binary_frame_rejects_bad_magic() {
        let mut encoded = BinaryDaemonFrame::request("req-1", "PING")
            .unwrap()
            .encode()
            .unwrap();
        encoded[0] = b'X';

        assert!(matches!(
            BinaryDaemonFrame::decode(&encoded).unwrap_err(),
            DaemonError::InvalidFrame
        ));
    }

    #[test]
    fn binary_frame_reader_reads_exact_payload() {
        let encoded = BinaryDaemonFrame::request("req-1", "PING")
            .unwrap()
            .encode()
            .unwrap();
        let mut reader = Cursor::new(encoded);

        let decoded = read_binary_frame(&mut reader).unwrap();

        assert_eq!(decoded.request_id, "req-1");
        assert_eq!(decoded.payload_text().unwrap(), "PING");
    }

    #[cfg(unix)]
    #[test]
    fn binary_stream_handler_returns_framed_response() {
        let state = Arc::new(Mutex::new(DaemonState::default()));
        let frame = BinaryDaemonFrame::request("req-9", "PING").unwrap();

        let response =
            handle_binary_frame_for_stream(Ok(frame), state, Arc::new(DaemonAuth::disabled()))
                .unwrap();

        assert_eq!(response.request_id, "req-9");
        assert!(!response.is_error);
        assert_eq!(response.payload_text().unwrap(), "pong");
    }

    #[test]
    fn ref_update_and_get_round_trip() {
        let mut state = DaemonState::default();
        let oid = put_root_commit(&mut state, "hello");

        let update = state
            .handle_command(&format!("REF_UPDATE tx1 refs/heads/main none {oid} acct"))
            .unwrap()
            .into_line();
        let get = state
            .handle_command("REF_GET refs/heads/main")
            .unwrap()
            .into_line();

        assert!(update.contains("status=committed"));
        assert_eq!(get, format!("OK ref=refs/heads/main oid={oid}"));
    }

    #[test]
    fn ref_list_reports_current_refs() {
        let mut state = DaemonState::default();
        let oid = put_root_commit(&mut state, "hello");
        state
            .handle_command(&format!("REF_UPDATE tx1 refs/heads/main none {oid} acct"))
            .unwrap();

        let list = state.handle_command("REF_LIST").unwrap().into_line();

        assert_eq!(list, format!("OK refs=refs/heads/main:{oid}"));
    }

    #[test]
    fn object_list_reports_stored_objects() {
        let mut state = DaemonState::default();
        let oid = put_object(&mut state, "blob", &encode_hex(b"hello"));

        let list = state.handle_command("OBJECT_LIST").unwrap().into_line();

        assert!(list.contains(&oid));
        assert!(list.contains(":Blob:"));
        assert!(list.contains(":true"));
    }

    #[test]
    fn ref_update_conflict_is_reported() {
        let mut state = DaemonState::default();
        let oid1 = put_root_commit(&mut state, "one");
        let oid2 = put_root_commit(&mut state, "two");
        state
            .handle_command(&format!("REF_UPDATE tx1 refs/heads/main none {oid1} acct"))
            .unwrap();

        let conflict = state
            .handle_command(&format!("REF_UPDATE tx2 refs/heads/main none {oid2} acct"))
            .unwrap()
            .into_line();

        assert!(conflict.contains("status=rejected"));
        assert!(conflict.contains("reason=conflict"));
    }

    #[test]
    fn ref_update_rejects_non_fast_forward_branch_move() {
        let mut state = DaemonState::default();
        let root = put_root_commit(&mut state, "root");
        let unrelated = put_root_commit(&mut state, "unrelated");
        state
            .handle_command(&format!("REF_UPDATE tx1 refs/heads/main none {root} acct"))
            .unwrap();

        let err = state
            .handle_command(&format!(
                "REF_UPDATE tx2 refs/heads/main {root} {unrelated} acct"
            ))
            .unwrap_err();

        assert!(matches!(
            err,
            DaemonError::Repository(RepositoryError::NonFastForward { .. })
        ));
    }

    #[test]
    fn ref_update_accepts_fast_forward_branch_move() {
        let mut state = DaemonState::default();
        let root = put_root_commit(&mut state, "root");
        let child = put_child_commit(&mut state, &root, "child");
        state
            .handle_command(&format!("REF_UPDATE tx1 refs/heads/main none {root} acct"))
            .unwrap();

        let update = state
            .handle_command(&format!(
                "REF_UPDATE tx2 refs/heads/main {root} {child} acct"
            ))
            .unwrap()
            .into_line();

        assert!(update.contains("status=committed"));
        assert_eq!(
            state
                .handle_command("REF_GET refs/heads/main")
                .unwrap()
                .into_line(),
            format!("OK ref=refs/heads/main oid={child}")
        );
    }

    #[test]
    fn ref_update_force_accepts_non_fast_forward_branch_move() {
        let mut state = DaemonState::default();
        let root = put_root_commit(&mut state, "root");
        let unrelated = put_root_commit(&mut state, "unrelated");
        state
            .handle_command(&format!("REF_UPDATE tx1 refs/heads/main none {root} acct"))
            .unwrap();

        let update = state
            .handle_command(&format!(
                "REF_UPDATE_FORCE tx2 refs/heads/main {root} {unrelated} acct"
            ))
            .unwrap()
            .into_line();

        assert!(update.contains("status=committed"));
        assert_eq!(
            state
                .handle_command("REF_GET refs/heads/main")
                .unwrap()
                .into_line(),
            format!("OK ref=refs/heads/main oid={unrelated}")
        );
    }

    #[test]
    fn ref_update_deletes_existing_ref() {
        let mut state = DaemonState::default();
        let oid = put_root_commit(&mut state, "delete me");
        state
            .handle_command(&format!("REF_UPDATE tx1 refs/heads/main none {oid} acct"))
            .unwrap();

        let update = state
            .handle_command(&format!("REF_UPDATE tx2 refs/heads/main {oid} delete acct"))
            .unwrap()
            .into_line();

        assert!(update.contains("status=committed"));
        assert_eq!(
            state
                .handle_command("REF_GET refs/heads/main")
                .unwrap()
                .into_line(),
            "OK ref=refs/heads/main oid=none"
        );
    }

    #[test]
    fn ref_update_requires_existing_target_object() {
        let mut state = DaemonState::default();
        let oid = "3b18e512dba79e4c8300dd08aeb37f8e728b8dad";

        let err = state
            .handle_command(&format!("REF_UPDATE tx1 refs/heads/main none {oid} acct"))
            .unwrap_err();

        assert!(matches!(
            err,
            DaemonError::Repository(RepositoryError::MissingObject(_))
        ));
        assert_eq!(
            state
                .handle_command("REF_GET refs/heads/main")
                .unwrap()
                .into_line(),
            "OK ref=refs/heads/main oid=none"
        );
    }

    #[test]
    fn signed_ref_update_commits_after_verification() {
        let mut state = DaemonState::default();
        let oid = put_root_commit(&mut state, "signed");
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let certificate = account.certify_device(&device, "gm-dev-device");
        let update = RefUpdate {
            repo_id: state.repo_id.clone(),
            ref_name: RefName::new("refs/heads/main").unwrap(),
            expected_old_oid: None,
            new_oid: Some(GitSha1Oid::from_str(&oid).unwrap()),
            force: false,
            policy_epoch: 0,
            transaction_id: TransactionId::new("tx-signed").unwrap(),
            signer: certificate.device_id.as_cid().to_string(),
        };
        let update_signature = device.sign(&update.signing_transcript());
        let command = format!(
            "REF_UPDATE_SIGNED tx-signed refs/heads/main none {} {} {} {} {} {}",
            oid,
            encode_hex(certificate.label.as_bytes()),
            encode_hex(&certificate.account_verifying_key),
            encode_hex(&certificate.device_verifying_key),
            encode_hex(&certificate.signature),
            encode_hex(&update_signature)
        );

        let response = state.handle_command(&command).unwrap().into_line();

        assert!(response.contains("status=committed"));
        assert_eq!(
            state
                .handle_command("REF_GET refs/heads/main")
                .unwrap()
                .into_line(),
            format!("OK ref=refs/heads/main oid={oid}")
        );
    }

    #[test]
    fn signed_force_ref_update_accepts_non_fast_forward_branch_move() {
        let mut state = DaemonState::default();
        let root = put_root_commit(&mut state, "root");
        let unrelated = put_root_commit(&mut state, "unrelated signed force");
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let certificate = account.certify_device(&device, "gm-dev-device");

        state
            .handle_command(&format!(
                "REF_UPDATE tx-root refs/heads/main none {root} acct"
            ))
            .unwrap();

        let update = RefUpdate {
            repo_id: state.repo_id.clone(),
            ref_name: RefName::new("refs/heads/main").unwrap(),
            expected_old_oid: Some(GitSha1Oid::from_str(&root).unwrap()),
            new_oid: Some(GitSha1Oid::from_str(&unrelated).unwrap()),
            force: true,
            policy_epoch: 0,
            transaction_id: TransactionId::new("tx-signed-force").unwrap(),
            signer: certificate.device_id.as_cid().to_string(),
        };
        let update_signature = device.sign(&update.signing_transcript());
        let command = format!(
            "REF_UPDATE_SIGNED_FORCE tx-signed-force refs/heads/main {} {} {} {} {} {} {}",
            root,
            unrelated,
            encode_hex(certificate.label.as_bytes()),
            encode_hex(&certificate.account_verifying_key),
            encode_hex(&certificate.device_verifying_key),
            encode_hex(&certificate.signature),
            encode_hex(&update_signature)
        );

        let response = state.handle_command(&command).unwrap().into_line();

        assert!(response.contains("status=committed"));
        assert_eq!(
            state
                .handle_command("REF_GET refs/heads/main")
                .unwrap()
                .into_line(),
            format!("OK ref=refs/heads/main oid={unrelated}")
        );
    }

    #[test]
    fn policy_denies_unsigned_ref_updates_when_required() {
        let mut state = DaemonState::default();
        let oid = put_root_commit(&mut state, "policy");
        state
            .handle_command("POLICY_SET_REQUIRE_SIGNED true")
            .unwrap();

        let err = state
            .handle_command(&format!("REF_UPDATE tx1 refs/heads/main none {oid} acct"))
            .unwrap_err();

        assert!(matches!(
            err,
            DaemonError::Coordination(CoordinationError::UnsignedRefUpdateDenied)
        ));
    }

    #[test]
    fn policy_authorizes_signed_writer_account() {
        let mut state = DaemonState::default();
        let oid = put_root_commit(&mut state, "policy signed");
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let certificate = account.certify_device(&device, "gm-dev-device");
        state
            .handle_command("POLICY_SET_REQUIRE_SIGNED true")
            .unwrap();
        state
            .handle_command(&format!(
                "POLICY_GRANT_WRITER_ACCOUNT {}",
                certificate.account_id.as_cid()
            ))
            .unwrap();
        let update = RefUpdate {
            repo_id: state.repo_id.clone(),
            ref_name: RefName::new("refs/heads/main").unwrap(),
            expected_old_oid: None,
            new_oid: Some(GitSha1Oid::from_str(&oid).unwrap()),
            force: false,
            policy_epoch: 0,
            transaction_id: TransactionId::new("tx-policy-signed").unwrap(),
            signer: certificate.device_id.as_cid().to_string(),
        };
        let update_signature = device.sign(&update.signing_transcript());
        let command = format!(
            "REF_UPDATE_SIGNED tx-policy-signed refs/heads/main none {} {} {} {} {} {}",
            oid,
            encode_hex(certificate.label.as_bytes()),
            encode_hex(&certificate.account_verifying_key),
            encode_hex(&certificate.device_verifying_key),
            encode_hex(&certificate.signature),
            encode_hex(&update_signature)
        );

        let response = state.handle_command(&command).unwrap().into_line();

        assert!(response.contains("status=committed"));
    }

    #[test]
    fn persisted_policy_reloads() {
        let policy_path =
            std::env::temp_dir().join(format!("gitmeshd-test-policy-{}.txt", std::process::id()));
        let mut state =
            DaemonState::with_store_paths(None, None, Some(policy_path.clone())).unwrap();
        state
            .handle_command("POLICY_SET_REQUIRE_SIGNED true")
            .unwrap();
        state
            .handle_command("POLICY_PROTECT_REF refs/heads/main")
            .unwrap();

        let restored =
            DaemonState::with_store_paths(None, None, Some(policy_path.clone())).unwrap();
        let response = format_policy(&restored.policy);

        assert!(response.contains("require_signed_refs=true"));
        assert!(response.contains("protected_refs=1"));
        let _ = fs::remove_file(policy_path);
    }

    #[test]
    fn signed_ref_update_rejects_bad_signature() {
        let mut state = DaemonState::default();
        let oid = put_root_commit(&mut state, "bad sig");
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let certificate = account.certify_device(&device, "gm-dev-device");
        let update_signature = [7_u8; 64];
        let command = format!(
            "REF_UPDATE_SIGNED tx-bad refs/heads/main none {} {} {} {} {} {}",
            oid,
            encode_hex(certificate.label.as_bytes()),
            encode_hex(&certificate.account_verifying_key),
            encode_hex(&certificate.device_verifying_key),
            encode_hex(&certificate.signature),
            encode_hex(&update_signature)
        );

        let err = state.handle_command(&command).unwrap_err();

        assert!(matches!(
            err,
            DaemonError::Coordination(CoordinationError::Identity(IdentityError::InvalidSignature))
        ));
    }

    #[test]
    fn idempotent_ref_retry_does_not_recheck_object_store() {
        let mut state = DaemonState::default();
        let oid = "3b18e512dba79e4c8300dd08aeb37f8e728b8dad";
        let update = RefUpdate {
            repo_id: state.repo_id.clone(),
            ref_name: RefName::new("refs/heads/main").unwrap(),
            expected_old_oid: None,
            new_oid: Some(GitSha1Oid::from_str(oid).unwrap()),
            force: false,
            policy_epoch: 0,
            transaction_id: TransactionId::new("tx1").unwrap(),
            signer: "acct".to_string(),
        };
        let first = state.refs.apply(update);
        assert!(matches!(first, TransactionReceipt::Committed { .. }));

        let retry = state
            .handle_command(&format!("REF_UPDATE tx1 refs/heads/main none {oid} acct"))
            .unwrap()
            .into_line();

        assert_eq!(
            retry,
            format!("OK status=committed ref=refs/heads/main old=none new={oid}")
        );
    }

    #[test]
    fn object_put_and_get_round_trip_through_repository_store() {
        let mut state = DaemonState::default();
        let put = state
            .handle_command("OBJECT_PUT blob 68656c6c6f")
            .unwrap()
            .into_line();
        let oid = put
            .split_whitespace()
            .find_map(|part| part.strip_prefix("oid="))
            .unwrap()
            .to_string();
        let get = state
            .handle_command(&format!("OBJECT_GET {oid}"))
            .unwrap()
            .into_line();

        assert!(put.contains("durability_satisfied=true"));
        assert!(get.contains("kind=Blob"));
        assert!(get.contains("payload_hex=68656c6c6f"));
    }

    #[test]
    fn object_audit_reports_repair_needed_after_shard_loss() {
        let mut state = DaemonState::default();
        let oid = put_object(&mut state, "blob", &encode_hex(b"audit daemon"));
        state
            .objects
            .simulate_object_shard_loss(GitSha1Oid::from_str(&oid).unwrap(), &[0, 2])
            .unwrap();

        let audit = state
            .handle_command(&format!("OBJECT_AUDIT {oid}"))
            .unwrap()
            .into_line();

        assert!(audit.contains("repair_needed=1"));
        assert!(audit.contains("missing=2"));
        assert!(audit.contains("repair=true"));
    }

    #[test]
    fn object_repair_restores_lost_shards() {
        let mut state = DaemonState::default();
        let oid = put_object(&mut state, "blob", &encode_hex(b"repair daemon"));
        state
            .objects
            .simulate_object_shard_loss(GitSha1Oid::from_str(&oid).unwrap(), &[0, 2, 4])
            .unwrap();

        let repair = state
            .handle_command(&format!("OBJECT_REPAIR {oid}"))
            .unwrap()
            .into_line();
        let audit = state
            .handle_command(&format!("OBJECT_AUDIT {oid}"))
            .unwrap()
            .into_line();

        assert!(repair.contains("repaired=1"));
        assert!(repair.contains("verified=16"));
        assert!(audit.contains("repair_needed=0"));
    }

    #[test]
    fn pack_put_imports_full_objects() {
        let mut state = DaemonState::default();
        let pack = write_packfile(&[
            GitObject::new(GitObjectKind::Blob, b"hello"),
            GitObject::new(GitObjectKind::Tree, Vec::new()),
        ])
        .unwrap();
        let response = state
            .handle_command(&format!("PACK_PUT {}", encode_hex(&pack)))
            .unwrap()
            .into_line();
        let list = state.handle_command("OBJECT_LIST").unwrap().into_line();

        assert!(response.contains("pack_version=2"));
        assert!(response.contains("imported=2"));
        assert!(list.contains(":Blob:"));
        assert!(list.contains(":Tree:"));
    }

    #[test]
    fn pack_get_exports_full_objects() {
        let mut state = DaemonState::default();
        let oid = put_object(&mut state, "blob", &encode_hex(b"hello"));

        let response = state.handle_command("PACK_GET all").unwrap().into_line();
        let pack_hex = response
            .split_whitespace()
            .find_map(|part| part.strip_prefix("pack_hex="))
            .unwrap();
        let pack = decode_hex(pack_hex).unwrap();
        let parsed = parse_packfile(&pack).unwrap();

        assert!(response.contains("pack_version=2"));
        assert!(response.contains("objects=1"));
        assert_eq!(parsed.objects.len(), 1);
        assert_eq!(parsed.objects[0].sha1_oid().to_string(), oid);
    }

    #[test]
    fn pack_get_reachable_exports_requested_tip_closure() {
        let mut state = DaemonState::default();
        let commit = put_root_commit(&mut state, "reachable");
        put_object(&mut state, "blob", &encode_hex(b"unrelated"));

        let response = state
            .handle_command(&format!("PACK_GET_REACHABLE {commit}"))
            .unwrap()
            .into_line();
        let pack_hex = response
            .split_whitespace()
            .find_map(|part| part.strip_prefix("pack_hex="))
            .unwrap();
        let pack = decode_hex(pack_hex).unwrap();
        let parsed = parse_packfile(&pack).unwrap();

        assert!(response.contains("tips=1"));
        assert_eq!(parsed.objects.len(), 2);
        assert!(
            parsed
                .objects
                .iter()
                .any(|object| object.sha1_oid().to_string() == commit)
        );
        assert!(
            parsed
                .objects
                .iter()
                .any(|object| object.kind == GitObjectKind::Tree)
        );
    }

    #[test]
    fn pack_get_reachable_rejects_missing_tip() {
        let mut state = DaemonState::default();

        let err = state
            .handle_command("PACK_GET_REACHABLE 3b18e512dba79e4c8300dd08aeb37f8e728b8dad")
            .unwrap_err();

        assert!(matches!(
            err,
            DaemonError::Repository(RepositoryError::MissingObject(_))
        ));
    }

    #[test]
    fn repo_status_reports_object_count() {
        let mut state = DaemonState::default();
        state.handle_command("OBJECT_PUT blob 6869").unwrap();

        let status = state.handle_command("REPO_STATUS").unwrap().into_line();

        assert!(status.contains("objects=1"));
        assert!(status.contains("checkpoints=0"));
        assert!(status.contains("data_shards=10"));
    }

    #[test]
    fn ref_checkpoint_reports_latest_chain_tip() {
        let mut state = DaemonState::default();
        let oid = put_root_commit(&mut state, "checkpoint");

        let empty = state.handle_command("REF_CHECKPOINT").unwrap().into_line();
        state
            .handle_command(&format!("REF_UPDATE tx1 refs/heads/main none {oid} acct"))
            .unwrap();
        let checkpoint = state.handle_command("REF_CHECKPOINT").unwrap().into_line();
        let status = state.handle_command("REPO_STATUS").unwrap().into_line();

        assert_eq!(empty, "OK checkpoint=none");
        assert!(checkpoint.contains("checkpoint=gitmesh:v0:ProtocolObject"));
        assert!(checkpoint.contains("sequence=1"));
        assert!(checkpoint.contains("parent=none"));
        assert!(checkpoint.contains("refs_root=gitmesh:v0:ProtocolObject"));
        assert!(checkpoint.contains("history_root=gitmesh:v0:ProtocolObject"));
        assert!(status.contains("checkpoints=1"));
    }

    #[test]
    fn daemon_state_loads_persisted_object_store() {
        let path =
            std::env::temp_dir().join(format!("gitmeshd-test-store-{}.txt", std::process::id()));
        let mut state = DaemonState::with_object_store_path(path.clone()).unwrap();
        let put = state
            .handle_command("OBJECT_PUT blob 70657273697374")
            .unwrap()
            .into_line();
        let oid = put
            .split_whitespace()
            .find_map(|part| part.strip_prefix("oid="))
            .unwrap()
            .to_string();

        let restored = DaemonState::with_object_store_path(path.clone()).unwrap();
        let get = restored.object_get(&oid).unwrap().into_line();

        assert!(get.contains("payload_hex=70657273697374"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn daemon_state_loads_persisted_refs_and_receipts() {
        let object_path =
            std::env::temp_dir().join(format!("gitmeshd-test-objects-{}.txt", std::process::id()));
        let ref_path =
            std::env::temp_dir().join(format!("gitmeshd-test-refs-{}.txt", std::process::id()));
        let mut state =
            DaemonState::with_store_paths(Some(object_path.clone()), Some(ref_path.clone()), None)
                .unwrap();
        let oid = put_root_commit(&mut state, "persisted ref");
        let first = state
            .handle_command(&format!("REF_UPDATE tx1 refs/heads/main none {oid} acct"))
            .unwrap()
            .into_line();

        let mut restored =
            DaemonState::with_store_paths(Some(object_path.clone()), Some(ref_path.clone()), None)
                .unwrap();
        let retry = restored
            .handle_command(&format!("REF_UPDATE tx1 refs/heads/main none {oid} acct"))
            .unwrap()
            .into_line();
        let get = restored
            .handle_command("REF_GET refs/heads/main")
            .unwrap()
            .into_line();
        let checkpoint = restored
            .handle_command("REF_CHECKPOINT")
            .unwrap()
            .into_line();

        assert_eq!(first, retry);
        assert_eq!(get, format!("OK ref=refs/heads/main oid={oid}"));
        assert!(checkpoint.contains("sequence=1"));
        let _ = fs::remove_file(object_path);
        let _ = fs::remove_file(ref_path);
    }
}
