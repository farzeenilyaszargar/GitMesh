//! Product account infrastructure for GitMesh.
//!
//! This crate is deliberately separate from protocol identity. A product
//! account owns display/profile/session state and references a cryptographic
//! account root identity by CID.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use gitmesh_core::hex;
use gitmesh_identity::AccountId;
use rand::RngCore;
use thiserror::Error;

const SNAPSHOT_HEADER: &str = "gitmesh-account-store-v0";
const SESSION_TOKEN_BYTES: usize = 32;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Username(String);

impl Username {
    pub fn new(value: impl Into<String>) -> Result<Self, AccountError> {
        let value = value.into();
        validate_username(&value)?;
        Ok(Self(value.to_ascii_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountProfile {
    pub username: Username,
    pub account_id: String,
    pub display_name: String,
    pub bio: String,
    pub avatar_uri: String,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewAccountProfile {
    pub username: Username,
    pub account_id: AccountId,
    pub display_name: String,
    pub bio: String,
    pub avatar_uri: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfileUpdate {
    pub display_name: Option<String>,
    pub bio: Option<String>,
    pub avatar_uri: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionToken(String);

impl SessionToken {
    pub fn expose_for_client(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountSession {
    pub session_id: String,
    pub token_hash: String,
    pub username: Username,
    pub device_id: Option<String>,
    pub created_at_unix: u64,
    pub expires_at_unix: u64,
    pub revoked_at_unix: Option<u64>,
}

impl AccountSession {
    pub fn is_active_at(&self, now_unix: u64) -> bool {
        self.revoked_at_unix.is_none() && now_unix < self.expires_at_unix
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssuedSession {
    pub token: SessionToken,
    pub session: AccountSession,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryRegistration {
    pub owner: Username,
    pub name: String,
    pub repo_id: String,
    pub visibility: RepositoryVisibility,
    pub created_at_unix: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepositoryVisibility {
    Public,
    Private,
}

impl RepositoryVisibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
        }
    }

    pub fn parse(value: &str) -> Result<Self, AccountError> {
        match value {
            "public" => Ok(Self::Public),
            "private" => Ok(Self::Private),
            _ => Err(AccountError::InvalidSnapshot),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AccountStore {
    profiles: BTreeMap<Username, AccountProfile>,
    account_index: BTreeMap<String, Username>,
    sessions: BTreeMap<String, AccountSession>,
    session_tokens: BTreeMap<String, String>,
    repositories: BTreeMap<(Username, String), RepositoryRegistration>,
}

impl AccountStore {
    pub fn create_profile(
        &mut self,
        profile: NewAccountProfile,
        now_unix: u64,
    ) -> Result<&AccountProfile, AccountError> {
        validate_profile_field(&profile.display_name, 96)?;
        validate_profile_field(&profile.bio, 512)?;
        validate_uri_field(&profile.avatar_uri)?;
        let account_id = profile.account_id.as_cid().to_string();
        if self.profiles.contains_key(&profile.username) {
            return Err(AccountError::UsernameAlreadyExists);
        }
        if self.account_index.contains_key(&account_id) {
            return Err(AccountError::AccountAlreadyExists);
        }

        let username = profile.username;
        let record = AccountProfile {
            username: username.clone(),
            account_id: account_id.clone(),
            display_name: profile.display_name,
            bio: profile.bio,
            avatar_uri: profile.avatar_uri,
            created_at_unix: now_unix,
            updated_at_unix: now_unix,
        };
        self.profiles.insert(username.clone(), record);
        self.account_index.insert(account_id, username.clone());
        Ok(self.profiles.get(&username).expect("profile inserted"))
    }

    pub fn update_profile(
        &mut self,
        username: &Username,
        update: ProfileUpdate,
        now_unix: u64,
    ) -> Result<&AccountProfile, AccountError> {
        let profile = self
            .profiles
            .get_mut(username)
            .ok_or(AccountError::AccountNotFound)?;
        if let Some(display_name) = update.display_name {
            validate_profile_field(&display_name, 96)?;
            profile.display_name = display_name;
        }
        if let Some(bio) = update.bio {
            validate_profile_field(&bio, 512)?;
            profile.bio = bio;
        }
        if let Some(avatar_uri) = update.avatar_uri {
            validate_uri_field(&avatar_uri)?;
            profile.avatar_uri = avatar_uri;
        }
        profile.updated_at_unix = now_unix;
        Ok(profile)
    }

    pub fn profile(&self, username: &Username) -> Option<&AccountProfile> {
        self.profiles.get(username)
    }

    pub fn profile_by_account_id(&self, account_id: &str) -> Option<&AccountProfile> {
        self.account_index
            .get(account_id)
            .and_then(|username| self.profiles.get(username))
    }

    pub fn issue_session(
        &mut self,
        username: &Username,
        device_id: Option<String>,
        ttl_seconds: u64,
        now_unix: u64,
    ) -> Result<IssuedSession, AccountError> {
        if ttl_seconds == 0 {
            return Err(AccountError::InvalidSessionTtl);
        }
        if !self.profiles.contains_key(username) {
            return Err(AccountError::AccountNotFound);
        }
        if let Some(device_id) = &device_id {
            validate_token_field(device_id)?;
        }
        let token = SessionToken(random_token());
        let token_hash = hash_session_token(token.expose_for_client());
        let session_id = hex(&blake3::hash(
            format!(
                "gitmesh.account-session.{username}.{now}.{token_hash}",
                username = username.as_str(),
                now = now_unix
            )
            .as_bytes(),
        )
        .as_bytes()[..16]);
        let session = AccountSession {
            session_id: session_id.clone(),
            token_hash: token_hash.clone(),
            username: username.clone(),
            device_id,
            created_at_unix: now_unix,
            expires_at_unix: now_unix
                .checked_add(ttl_seconds)
                .ok_or(AccountError::InvalidSessionTtl)?,
            revoked_at_unix: None,
        };
        self.sessions.insert(session_id.clone(), session.clone());
        self.session_tokens.insert(token_hash, session_id);
        Ok(IssuedSession { token, session })
    }

    pub fn authenticate_session(
        &self,
        token: &str,
        now_unix: u64,
    ) -> Result<&AccountSession, AccountError> {
        validate_token_field(token)?;
        let token_hash = hash_session_token(token);
        let session_id = self
            .session_tokens
            .get(&token_hash)
            .ok_or(AccountError::InvalidSession)?;
        let session = self
            .sessions
            .get(session_id)
            .ok_or(AccountError::InvalidSession)?;
        if session.is_active_at(now_unix) {
            Ok(session)
        } else {
            Err(AccountError::InvalidSession)
        }
    }

    pub fn revoke_session(&mut self, session_id: &str, now_unix: u64) -> Result<(), AccountError> {
        validate_token_field(session_id)?;
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or(AccountError::InvalidSession)?;
        session.revoked_at_unix = Some(now_unix);
        Ok(())
    }

    pub fn register_repository(
        &mut self,
        owner: &Username,
        name: impl Into<String>,
        repo_id: impl Into<String>,
        visibility: RepositoryVisibility,
        now_unix: u64,
    ) -> Result<&RepositoryRegistration, AccountError> {
        if !self.profiles.contains_key(owner) {
            return Err(AccountError::AccountNotFound);
        }
        let name = name.into();
        validate_repository_name(&name)?;
        let repo_id = repo_id.into();
        validate_token_field(&repo_id)?;
        let key = (owner.clone(), name.clone());
        if self.repositories.contains_key(&key) {
            return Err(AccountError::RepositoryAlreadyExists);
        }
        let registration = RepositoryRegistration {
            owner: owner.clone(),
            name,
            repo_id,
            visibility,
            created_at_unix: now_unix,
        };
        self.repositories.insert(key.clone(), registration);
        Ok(self
            .repositories
            .get(&key)
            .expect("repository registration inserted"))
    }

    pub fn repositories_for_owner(&self, owner: &Username) -> Vec<&RepositoryRegistration> {
        self.repositories
            .iter()
            .filter(|((repo_owner, _), _)| repo_owner == owner)
            .map(|(_, registration)| registration)
            .collect()
    }

    pub fn repository(&self, owner: &Username, name: &str) -> Option<&RepositoryRegistration> {
        self.repositories.get(&(owner.clone(), name.to_string()))
    }

    pub fn profile_count(&self) -> usize {
        self.profiles.len()
    }

    pub fn active_session_count(&self, now_unix: u64) -> usize {
        self.sessions
            .values()
            .filter(|session| session.is_active_at(now_unix))
            .count()
    }

    pub fn repository_count(&self) -> usize {
        self.repositories.len()
    }

    pub fn to_snapshot(&self) -> Result<String, AccountError> {
        let mut out = format!("{SNAPSHOT_HEADER}\n");
        for profile in self.profiles.values() {
            validate_snapshot_field(profile.username.as_str())?;
            validate_snapshot_field(&profile.account_id)?;
            validate_snapshot_field(&profile.display_name)?;
            validate_snapshot_field(&profile.bio)?;
            validate_snapshot_field(&profile.avatar_uri)?;
            out.push_str(&format!(
                "profile\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                profile.username.as_str(),
                profile.account_id,
                profile.display_name,
                profile.bio,
                profile.avatar_uri,
                profile.created_at_unix,
                profile.updated_at_unix
            ));
        }
        for session in self.sessions.values() {
            validate_snapshot_field(&session.session_id)?;
            validate_snapshot_field(&session.token_hash)?;
            validate_snapshot_field(session.username.as_str())?;
            if let Some(device_id) = &session.device_id {
                validate_snapshot_field(device_id)?;
            }
            out.push_str(&format!(
                "session\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                session.session_id,
                session.token_hash,
                session.username.as_str(),
                session.device_id.as_deref().unwrap_or("none"),
                session.created_at_unix,
                session.expires_at_unix,
                session
                    .revoked_at_unix
                    .map_or_else(|| "none".to_string(), |value| value.to_string())
            ));
        }
        for repository in self.repositories.values() {
            validate_snapshot_field(repository.owner.as_str())?;
            validate_snapshot_field(&repository.name)?;
            validate_snapshot_field(&repository.repo_id)?;
            out.push_str(&format!(
                "repository\t{}\t{}\t{}\t{}\t{}\n",
                repository.owner.as_str(),
                repository.name,
                repository.repo_id,
                repository.visibility.as_str(),
                repository.created_at_unix
            ));
        }
        Ok(out)
    }

    pub fn from_snapshot(text: &str) -> Result<Self, AccountError> {
        let mut lines = text.lines();
        if lines.next() != Some(SNAPSHOT_HEADER) {
            return Err(AccountError::InvalidSnapshot);
        }
        let mut store = Self::default();
        let mut pending_sessions = Vec::new();
        let mut pending_repositories = Vec::new();
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            let parts = line.split('\t').collect::<Vec<_>>();
            match parts.first().copied() {
                Some("profile") => {
                    if parts.len() != 8 {
                        return Err(AccountError::InvalidSnapshot);
                    }
                    let username = Username::new(parts[1])?;
                    let account_id = parts[2].to_string();
                    validate_token_field(&account_id)?;
                    if store.profiles.contains_key(&username)
                        || store.account_index.contains_key(&account_id)
                    {
                        return Err(AccountError::InvalidSnapshot);
                    }
                    let profile = AccountProfile {
                        username: username.clone(),
                        account_id: account_id.clone(),
                        display_name: parts[3].to_string(),
                        bio: parts[4].to_string(),
                        avatar_uri: parts[5].to_string(),
                        created_at_unix: parse_u64(parts[6])?,
                        updated_at_unix: parse_u64(parts[7])?,
                    };
                    validate_profile_field(&profile.display_name, 96)?;
                    validate_profile_field(&profile.bio, 512)?;
                    validate_uri_field(&profile.avatar_uri)?;
                    store.profiles.insert(username.clone(), profile);
                    store.account_index.insert(account_id, username);
                }
                Some("session") => {
                    if parts.len() != 8 {
                        return Err(AccountError::InvalidSnapshot);
                    }
                    pending_sessions.push(parts);
                }
                Some("repository") => {
                    if parts.len() != 6 {
                        return Err(AccountError::InvalidSnapshot);
                    }
                    pending_repositories.push(parts);
                }
                _ => return Err(AccountError::InvalidSnapshot),
            }
        }
        for parts in pending_sessions {
            let username = Username::new(parts[3])?;
            if !store.profiles.contains_key(&username) {
                return Err(AccountError::InvalidSnapshot);
            }
            let session_id = parts[1].to_string();
            let token_hash = parts[2].to_string();
            validate_token_field(&session_id)?;
            validate_token_field(&token_hash)?;
            let session = AccountSession {
                session_id: session_id.clone(),
                token_hash: token_hash.clone(),
                username,
                device_id: if parts[4] == "none" {
                    None
                } else {
                    validate_token_field(parts[4])?;
                    Some(parts[4].to_string())
                },
                created_at_unix: parse_u64(parts[5])?,
                expires_at_unix: parse_u64(parts[6])?,
                revoked_at_unix: if parts[7] == "none" {
                    None
                } else {
                    Some(parse_u64(parts[7])?)
                },
            };
            if store.sessions.insert(session_id.clone(), session).is_some()
                || store
                    .session_tokens
                    .insert(token_hash, session_id)
                    .is_some()
            {
                return Err(AccountError::InvalidSnapshot);
            }
        }
        for parts in pending_repositories {
            let owner = Username::new(parts[1])?;
            if !store.profiles.contains_key(&owner) {
                return Err(AccountError::InvalidSnapshot);
            }
            let name = parts[2].to_string();
            validate_repository_name(&name)?;
            let repo_id = parts[3].to_string();
            validate_token_field(&repo_id)?;
            let key = (owner.clone(), name.clone());
            let registration = RepositoryRegistration {
                owner,
                name,
                repo_id,
                visibility: RepositoryVisibility::parse(parts[4])?,
                created_at_unix: parse_u64(parts[5])?,
            };
            if store.repositories.insert(key, registration).is_some() {
                return Err(AccountError::InvalidSnapshot);
            }
        }
        Ok(store)
    }

    pub fn save_to_path(&self, path: impl AsRef<Path>) -> Result<(), AccountError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp_path = path.with_extension("tmp");
        fs::write(&tmp_path, self.to_snapshot()?)?;
        fs::rename(tmp_path, path)?;
        Ok(())
    }

    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, AccountError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }
        Self::from_snapshot(&fs::read_to_string(path)?)
    }
}

pub fn now_unix() -> Result<u64, AccountError> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AccountError::InvalidTimestamp)?
        .as_secs())
}

pub fn hash_session_token(token: &str) -> String {
    let mut transcript = Vec::new();
    transcript.extend_from_slice(b"gitmesh.v0.account-session-token");
    transcript.extend_from_slice(&(token.len() as u64).to_be_bytes());
    transcript.extend_from_slice(token.as_bytes());
    hex(blake3::hash(&transcript).as_bytes())
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum AccountError {
    #[error("invalid username")]
    InvalidUsername,
    #[error("username already exists")]
    UsernameAlreadyExists,
    #[error("account already exists")]
    AccountAlreadyExists,
    #[error("account not found")]
    AccountNotFound,
    #[error("invalid profile field")]
    InvalidProfileField,
    #[error("invalid session token")]
    InvalidSession,
    #[error("invalid session ttl")]
    InvalidSessionTtl,
    #[error("repository already exists")]
    RepositoryAlreadyExists,
    #[error("invalid repository name")]
    InvalidRepositoryName,
    #[error("invalid account snapshot")]
    InvalidSnapshot,
    #[error("invalid timestamp")]
    InvalidTimestamp,
    #[error("I/O failed: {0}")]
    Io(String),
}

impl From<std::io::Error> for AccountError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

fn random_token() -> String {
    let mut bytes = [0_u8; SESSION_TOKEN_BYTES];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex(&bytes)
}

fn validate_username(value: &str) -> Result<(), AccountError> {
    if value.len() < 2
        || value.len() > 39
        || value.starts_with('-')
        || value.ends_with('-')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(AccountError::InvalidUsername);
    }
    Ok(())
}

fn validate_repository_name(value: &str) -> Result<(), AccountError> {
    if value.is_empty()
        || value.len() > 100
        || value.starts_with('.')
        || value.ends_with('.')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(AccountError::InvalidRepositoryName);
    }
    Ok(())
}

fn validate_profile_field(value: &str, max_len: usize) -> Result<(), AccountError> {
    if value.len() > max_len || value.contains('\t') || value.contains('\n') || value.contains('\r')
    {
        return Err(AccountError::InvalidProfileField);
    }
    Ok(())
}

fn validate_uri_field(value: &str) -> Result<(), AccountError> {
    if value.len() > 512 || value.contains('\t') || value.contains('\n') || value.contains('\r') {
        return Err(AccountError::InvalidProfileField);
    }
    Ok(())
}

fn validate_snapshot_field(value: &str) -> Result<(), AccountError> {
    if value.contains('\t') || value.contains('\n') || value.contains('\r') {
        return Err(AccountError::InvalidSnapshot);
    }
    Ok(())
}

fn validate_token_field(value: &str) -> Result<(), AccountError> {
    if value.is_empty() || value.contains(char::is_whitespace) {
        return Err(AccountError::InvalidSnapshot);
    }
    Ok(())
}

fn parse_u64(value: &str) -> Result<u64, AccountError> {
    value
        .parse::<u64>()
        .map_err(|_| AccountError::InvalidSnapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitmesh_identity::AccountRootKey;

    fn new_account(username: &str) -> NewAccountProfile {
        let root = AccountRootKey::generate();
        NewAccountProfile {
            username: Username::new(username).unwrap(),
            account_id: root.account_id(),
            display_name: "Farzeen".to_string(),
            bio: "building GitMesh".to_string(),
            avatar_uri: "asset:prof_pic.jpeg".to_string(),
        }
    }

    #[test]
    fn creates_and_updates_profile() {
        let mut store = AccountStore::default();
        let username = Username::new("farzeen").unwrap();

        store.create_profile(new_account("farzeen"), 10).unwrap();
        let profile = store
            .update_profile(
                &username,
                ProfileUpdate {
                    display_name: Some("Farzeen Ilyas".to_string()),
                    bio: None,
                    avatar_uri: Some("asset:new.png".to_string()),
                },
                20,
            )
            .unwrap();

        assert_eq!(profile.display_name, "Farzeen Ilyas");
        assert_eq!(profile.avatar_uri, "asset:new.png");
        assert_eq!(profile.updated_at_unix, 20);
    }

    #[test]
    fn rejects_duplicate_usernames_and_account_ids() {
        let mut store = AccountStore::default();
        let profile = new_account("farzeen");
        let account_id = profile.account_id.clone();

        store.create_profile(profile, 1).unwrap();
        let duplicate_username = NewAccountProfile {
            username: Username::new("Farzeen").unwrap(),
            account_id: AccountRootKey::generate().account_id(),
            display_name: String::new(),
            bio: String::new(),
            avatar_uri: String::new(),
        };
        let duplicate_account = NewAccountProfile {
            username: Username::new("mesh").unwrap(),
            account_id,
            display_name: String::new(),
            bio: String::new(),
            avatar_uri: String::new(),
        };

        assert_eq!(
            store.create_profile(duplicate_username, 2).unwrap_err(),
            AccountError::UsernameAlreadyExists
        );
        assert_eq!(
            store.create_profile(duplicate_account, 2).unwrap_err(),
            AccountError::AccountAlreadyExists
        );
    }

    #[test]
    fn sessions_authenticate_expire_and_revoke() {
        let mut store = AccountStore::default();
        let username = Username::new("farzeen").unwrap();
        store.create_profile(new_account("farzeen"), 1).unwrap();

        let issued = store
            .issue_session(&username, Some("device-1".to_string()), 100, 10)
            .unwrap();
        let active = store
            .authenticate_session(issued.token.expose_for_client(), 50)
            .unwrap();

        assert_eq!(active.username, username);
        assert!(
            store
                .authenticate_session(issued.token.expose_for_client(), 111)
                .is_err()
        );
        store
            .revoke_session(&issued.session.session_id, 60)
            .unwrap();
        assert!(
            store
                .authenticate_session(issued.token.expose_for_client(), 70)
                .is_err()
        );
    }

    #[test]
    fn registers_repositories_under_owner_namespace() {
        let mut store = AccountStore::default();
        let username = Username::new("farzeen").unwrap();
        store.create_profile(new_account("farzeen"), 1).unwrap();

        let repo = store
            .register_repository(
                &username,
                "GitMesh",
                "repo:farzeen/gitmesh",
                RepositoryVisibility::Private,
                2,
            )
            .unwrap();

        assert_eq!(repo.owner, username);
        assert_eq!(repo.visibility, RepositoryVisibility::Private);
        assert_eq!(store.repositories_for_owner(&username).len(), 1);
        assert!(store.repository(&username, "GitMesh").is_some());
    }

    #[test]
    fn snapshot_round_trips() {
        let mut store = AccountStore::default();
        let username = Username::new("farzeen").unwrap();
        store.create_profile(new_account("farzeen"), 1).unwrap();
        let issued = store.issue_session(&username, None, 100, 2).unwrap();
        store
            .register_repository(
                &username,
                "GitMesh",
                "repo:farzeen/gitmesh",
                RepositoryVisibility::Private,
                3,
            )
            .unwrap();

        let restored = AccountStore::from_snapshot(&store.to_snapshot().unwrap()).unwrap();

        assert_eq!(restored.profile_count(), 1);
        assert_eq!(restored.repository_count(), 1);
        assert_eq!(restored.active_session_count(10), 1);
        assert!(
            restored
                .authenticate_session(issued.token.expose_for_client(), 10)
                .is_ok()
        );
    }

    #[test]
    fn save_and_load_snapshot_file() {
        let path = std::env::temp_dir().join(format!(
            "gitmesh-account-store-test-{}.tsv",
            std::process::id()
        ));
        let mut store = AccountStore::default();
        store.create_profile(new_account("farzeen"), 1).unwrap();

        store.save_to_path(&path).unwrap();
        let restored = AccountStore::load_from_path(&path).unwrap();
        let _ = fs::remove_file(path);

        assert_eq!(restored.profile_count(), 1);
    }

    #[test]
    fn username_validation_matches_namespace_rules() {
        assert!(Username::new("farzeen").is_ok());
        assert!(Username::new("farzeen-ilyas").is_ok());
        assert_eq!(
            Username::new("-bad").unwrap_err(),
            AccountError::InvalidUsername
        );
        assert_eq!(
            Username::new("bad_name").unwrap_err(),
            AccountError::InvalidUsername
        );
    }
}
