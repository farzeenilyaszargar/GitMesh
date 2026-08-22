//! Collaboration event primitives for GitMesh.
//!
//! Issues, pull requests, comments, reviews, and later releases/discussions are
//! derived collaboration state. Their durable representation is an independently
//! verifiable event graph, not a web database row.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use gitmesh_core::{Cid, CidKind, CoreError, HashAlgorithm, ProtocolEnvelope, hex};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CollaborationEventKind {
    IssueOpened,
    IssueClosed,
    PullRequestOpened,
    PullRequestMerged,
    CommentAdded,
    ReviewSubmitted,
}

impl CollaborationEventKind {
    fn code(self) -> u16 {
        match self {
            Self::IssueOpened => 1,
            Self::IssueClosed => 2,
            Self::PullRequestOpened => 3,
            Self::PullRequestMerged => 4,
            Self::CommentAdded => 5,
            Self::ReviewSubmitted => 6,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::IssueOpened => "issue.opened",
            Self::IssueClosed => "issue.closed",
            Self::PullRequestOpened => "pull_request.opened",
            Self::PullRequestMerged => "pull_request.merged",
            Self::CommentAdded => "comment.added",
            Self::ReviewSubmitted => "review.submitted",
        }
    }

    fn parse_label(value: &str) -> Result<Self, CollaborationError> {
        match value {
            "issue.opened" => Ok(Self::IssueOpened),
            "issue.closed" => Ok(Self::IssueClosed),
            "pull_request.opened" => Ok(Self::PullRequestOpened),
            "pull_request.merged" => Ok(Self::PullRequestMerged),
            "comment.added" => Ok(Self::CommentAdded),
            "review.submitted" => Ok(Self::ReviewSubmitted),
            _ => Err(CollaborationError::InvalidSnapshot),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollaborationPayload {
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    pub source_ref: Option<String>,
    pub target_ref: Option<String>,
}

impl CollaborationPayload {
    pub fn issue(title: impl Into<String>, body: impl Into<String>, labels: Vec<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            labels,
            source_ref: None,
            target_ref: None,
        }
    }

    pub fn pull_request(
        title: impl Into<String>,
        body: impl Into<String>,
        labels: Vec<String>,
        source_ref: impl Into<String>,
        target_ref: impl Into<String>,
    ) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            labels,
            source_ref: Some(source_ref.into()),
            target_ref: Some(target_ref.into()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollaborationEvent {
    pub event_id: Cid,
    pub repo: String,
    pub kind: CollaborationEventKind,
    pub actor: String,
    pub parents: Vec<Cid>,
    pub timestamp_unix: u64,
    pub payload: CollaborationPayload,
}

impl CollaborationEvent {
    pub fn new(
        repo: impl Into<String>,
        kind: CollaborationEventKind,
        actor: impl Into<String>,
        parents: Vec<Cid>,
        timestamp_unix: u64,
        payload: CollaborationPayload,
    ) -> Result<Self, CollaborationError> {
        let repo = repo.into();
        validate_actor_or_repo(&repo)?;
        let actor = actor.into();
        validate_actor_or_repo(&actor)?;
        validate_payload(&payload)?;

        let body = canonical_event_body(&repo, kind, &actor, &parents, timestamp_unix, &payload)?;
        let event_id = ProtocolEnvelope::new("gitmesh.collaboration-event", body)?.cid()?;

        Ok(Self {
            event_id,
            repo,
            kind,
            actor,
            parents,
            timestamp_unix,
            payload,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssueSummary {
    pub number: u64,
    pub title: String,
    pub actor: String,
    pub labels: Vec<String>,
    pub event_id: Cid,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PullRequestSummary {
    pub number: u64,
    pub title: String,
    pub actor: String,
    pub source_ref: String,
    pub target_ref: String,
    pub labels: Vec<String>,
    pub event_id: Cid,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CollaborationEventStore {
    events: Vec<CollaborationEvent>,
    seen: BTreeSet<Cid>,
}

impl CollaborationEventStore {
    pub fn insert(&mut self, event: CollaborationEvent) -> bool {
        if !self.seen.insert(event.event_id) {
            return false;
        }
        self.events.push(event);
        true
    }

    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    pub fn events_for_repo(&self, repo: &str) -> Vec<&CollaborationEvent> {
        self.events
            .iter()
            .filter(|event| event.repo == repo)
            .collect()
    }

    pub fn issue_summaries(&self, repo: &str) -> Vec<IssueSummary> {
        self.events_for_repo(repo)
            .into_iter()
            .filter(|event| event.kind == CollaborationEventKind::IssueOpened)
            .enumerate()
            .map(|(index, event)| IssueSummary {
                number: (index + 1) as u64,
                title: event.payload.title.clone(),
                actor: event.actor.clone(),
                labels: event.payload.labels.clone(),
                event_id: event.event_id,
            })
            .collect()
    }

    pub fn pull_request_summaries(&self, repo: &str) -> Vec<PullRequestSummary> {
        self.events_for_repo(repo)
            .into_iter()
            .filter(|event| event.kind == CollaborationEventKind::PullRequestOpened)
            .enumerate()
            .map(|(index, event)| PullRequestSummary {
                number: (index + 1) as u64,
                title: event.payload.title.clone(),
                actor: event.actor.clone(),
                source_ref: event.payload.source_ref.clone().unwrap_or_default(),
                target_ref: event.payload.target_ref.clone().unwrap_or_default(),
                labels: event.payload.labels.clone(),
                event_id: event.event_id,
            })
            .collect()
    }

    pub fn to_snapshot(&self) -> Result<String, CollaborationError> {
        let mut out = String::from("gitmesh-collaboration-store-v0\n");
        for event in &self.events {
            out.push_str(&format!(
                "event\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                event.event_id.as_hex(),
                encode_text(&event.repo),
                event.kind.label(),
                encode_text(&event.actor),
                encode_parents(&event.parents),
                event.timestamp_unix,
                encode_text(&event.payload.title),
                encode_text(&event.payload.body),
                encode_text_list(&event.payload.labels),
                encode_optional_text(event.payload.source_ref.as_deref()),
                encode_optional_text(event.payload.target_ref.as_deref())
            ));
        }
        Ok(out)
    }

    pub fn from_snapshot(text: &str) -> Result<Self, CollaborationError> {
        let mut lines = text.lines();
        if lines.next() != Some("gitmesh-collaboration-store-v0") {
            return Err(CollaborationError::InvalidSnapshot);
        }
        let mut store = Self::default();
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            let parts = line.split('\t').collect::<Vec<_>>();
            if parts.len() != 12 || parts[0] != "event" {
                return Err(CollaborationError::InvalidSnapshot);
            }
            let stored_event_id = parse_protocol_cid_digest(parts[1])?;
            let event = CollaborationEvent::new(
                decode_text(parts[2])?,
                CollaborationEventKind::parse_label(parts[3])?,
                decode_text(parts[4])?,
                decode_parents(parts[5])?,
                parse_u64(parts[6])?,
                CollaborationPayload {
                    title: decode_text(parts[7])?,
                    body: decode_text(parts[8])?,
                    labels: decode_text_list(parts[9])?,
                    source_ref: decode_optional_text(parts[10])?,
                    target_ref: decode_optional_text(parts[11])?,
                },
            )?;
            if event.event_id != stored_event_id || !store.insert(event) {
                return Err(CollaborationError::InvalidSnapshot);
            }
        }
        Ok(store)
    }

    pub fn save_to_path(&self, path: impl AsRef<Path>) -> Result<(), CollaborationError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp_path = path.with_extension("tmp");
        fs::write(&tmp_path, self.to_snapshot()?)?;
        fs::rename(tmp_path, path)?;
        Ok(())
    }

    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, CollaborationError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }
        Self::from_snapshot(&fs::read_to_string(path)?)
    }
}

pub fn sample_issue_events() -> Vec<CollaborationEvent> {
    vec![
        CollaborationEvent::new(
            "farzeen/gitmesh",
            CollaborationEventKind::IssueOpened,
            "farzeen",
            Vec::new(),
            1_787_166_000,
            CollaborationPayload::issue(
                "Persist collaboration event logs",
                "Store issue and discussion events as signed, repo-scoped objects.",
                vec!["protocol".to_string(), "collaboration".to_string()],
            ),
        )
        .expect("sample issue event is valid"),
        CollaborationEvent::new(
            "farzeen/gitmesh",
            CollaborationEventKind::IssueOpened,
            "mesh-dev",
            Vec::new(),
            1_787_169_600,
            CollaborationPayload::issue(
                "Add private repository key epoch UI",
                "Expose revocation and epoch status without revealing private index contents.",
                vec!["security".to_string(), "web".to_string()],
            ),
        )
        .expect("sample issue event is valid"),
    ]
}

pub fn sample_pull_request_events() -> Vec<CollaborationEvent> {
    vec![
        CollaborationEvent::new(
            "farzeen/gitmesh",
            CollaborationEventKind::PullRequestOpened,
            "farzeen",
            Vec::new(),
            1_787_173_200,
            CollaborationPayload::pull_request(
                "Wire gm collaboration commands",
                "Replace placeholder issue and pull request commands with typed event summaries.",
                vec!["cli".to_string(), "collaboration".to_string()],
                "refs/heads/collaboration-cli",
                "refs/heads/main",
            ),
        )
        .expect("sample pull request event is valid"),
        CollaborationEvent::new(
            "farzeen/gitmesh",
            CollaborationEventKind::PullRequestOpened,
            "mesh-dev",
            Vec::new(),
            1_787_176_800,
            CollaborationPayload::pull_request(
                "Sketch trusted gateway mode checks",
                "Capture browser boundary expectations for private repository views.",
                vec!["gateway".to_string(), "security".to_string()],
                "refs/heads/private-gateway-mode",
                "refs/heads/main",
            ),
        )
        .expect("sample pull request event is valid"),
    ]
}

pub fn sample_issues() -> Vec<IssueSummary> {
    sample_issue_events()
        .into_iter()
        .enumerate()
        .map(|(index, event)| IssueSummary {
            number: (index + 1) as u64,
            title: event.payload.title,
            actor: event.actor,
            labels: event.payload.labels,
            event_id: event.event_id,
        })
        .collect()
}

pub fn sample_pull_requests() -> Vec<PullRequestSummary> {
    sample_pull_request_events()
        .into_iter()
        .enumerate()
        .map(|(index, event)| PullRequestSummary {
            number: (index + 1) as u64,
            title: event.payload.title,
            actor: event.actor,
            source_ref: event
                .payload
                .source_ref
                .expect("sample pull request has a source ref"),
            target_ref: event
                .payload
                .target_ref
                .expect("sample pull request has a target ref"),
            labels: event.payload.labels,
            event_id: event.event_id,
        })
        .collect()
}

#[derive(Debug, Error)]
pub enum CollaborationError {
    #[error("invalid actor or repository identifier")]
    InvalidActorOrRepo,
    #[error("collaboration payload title is required")]
    EmptyTitle,
    #[error("pull request events require source and target refs")]
    MissingPullRequestRefs,
    #[error("invalid collaboration snapshot")]
    InvalidSnapshot,
    #[error("I/O failed: {0}")]
    Io(String),
    #[error(transparent)]
    Core(#[from] CoreError),
}

impl From<std::io::Error> for CollaborationError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

fn canonical_event_body(
    repo: &str,
    kind: CollaborationEventKind,
    actor: &str,
    parents: &[Cid],
    timestamp_unix: u64,
    payload: &CollaborationPayload,
) -> Result<Vec<u8>, CollaborationError> {
    let mut out = Vec::new();
    out.extend_from_slice(b"gitmesh-collaboration-event-v0");
    put_string(&mut out, repo);
    out.extend_from_slice(&kind.code().to_be_bytes());
    put_string(&mut out, kind.label());
    put_string(&mut out, actor);
    out.extend_from_slice(&timestamp_unix.to_be_bytes());
    put_cid_vec(&mut out, parents)?;
    put_string(&mut out, &payload.title);
    put_string(&mut out, &payload.body);
    put_string_vec(&mut out, &payload.labels)?;
    put_optional_string(&mut out, payload.source_ref.as_deref());
    put_optional_string(&mut out, payload.target_ref.as_deref());
    Ok(out)
}

fn validate_actor_or_repo(value: &str) -> Result<(), CollaborationError> {
    if value.is_empty()
        || value.len() > 160
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
    {
        return Err(CollaborationError::InvalidActorOrRepo);
    }
    Ok(())
}

fn validate_payload(payload: &CollaborationPayload) -> Result<(), CollaborationError> {
    if payload.title.trim().is_empty() {
        return Err(CollaborationError::EmptyTitle);
    }
    if payload.source_ref.is_some() != payload.target_ref.is_some() {
        return Err(CollaborationError::MissingPullRequestRefs);
    }
    Ok(())
}

fn put_string(out: &mut Vec<u8>, value: &str) {
    put_bytes(out, value.as_bytes());
}

fn put_optional_string(out: &mut Vec<u8>, value: Option<&str>) {
    match value {
        Some(value) => {
            out.push(1);
            put_string(out, value);
        }
        None => out.push(0),
    }
}

fn put_string_vec(out: &mut Vec<u8>, values: &[String]) -> Result<(), CollaborationError> {
    let len = u32::try_from(values.len())
        .map_err(|_| CollaborationError::Core(CoreError::FieldTooLarge))?;
    out.extend_from_slice(&len.to_be_bytes());
    for value in values {
        put_string(out, value);
    }
    Ok(())
}

fn put_cid_vec(out: &mut Vec<u8>, values: &[Cid]) -> Result<(), CollaborationError> {
    let len = u32::try_from(values.len())
        .map_err(|_| CollaborationError::Core(CoreError::FieldTooLarge))?;
    out.extend_from_slice(&len.to_be_bytes());
    for cid in values {
        out.extend_from_slice(&cid.digest());
    }
    Ok(())
}

fn put_bytes(out: &mut Vec<u8>, value: &[u8]) {
    let len = u32::try_from(value.len()).expect("collaboration field length fits u32");
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(value);
}

fn encode_text(value: &str) -> String {
    hex(value.as_bytes())
}

fn decode_text(value: &str) -> Result<String, CollaborationError> {
    String::from_utf8(decode_hex(value)?).map_err(|_| CollaborationError::InvalidSnapshot)
}

fn encode_optional_text(value: Option<&str>) -> String {
    value.map_or_else(|| "-".to_string(), encode_text)
}

fn decode_optional_text(value: &str) -> Result<Option<String>, CollaborationError> {
    if value == "-" {
        Ok(None)
    } else {
        Ok(Some(decode_text(value)?))
    }
}

fn encode_text_list(values: &[String]) -> String {
    if values.is_empty() {
        return "-".to_string();
    }
    values
        .iter()
        .map(|value| encode_text(value))
        .collect::<Vec<_>>()
        .join(",")
}

fn decode_text_list(value: &str) -> Result<Vec<String>, CollaborationError> {
    if value == "-" {
        return Ok(Vec::new());
    }
    value.split(',').map(decode_text).collect()
}

fn encode_parents(parents: &[Cid]) -> String {
    if parents.is_empty() {
        return "-".to_string();
    }
    parents
        .iter()
        .map(|parent| parent.as_hex())
        .collect::<Vec<_>>()
        .join(",")
}

fn decode_parents(value: &str) -> Result<Vec<Cid>, CollaborationError> {
    if value == "-" {
        return Ok(Vec::new());
    }
    value.split(',').map(parse_protocol_cid_digest).collect()
}

fn parse_protocol_cid_digest(value: &str) -> Result<Cid, CollaborationError> {
    Ok(Cid::from_digest(
        CidKind::ProtocolObject,
        HashAlgorithm::Blake3_256,
        decode_fixed_hex::<32>(value)?,
    ))
}

fn parse_u64(value: &str) -> Result<u64, CollaborationError> {
    value
        .parse::<u64>()
        .map_err(|_| CollaborationError::InvalidSnapshot)
}

fn decode_fixed_hex<const N: usize>(value: &str) -> Result<[u8; N], CollaborationError> {
    let bytes = decode_hex(value)?;
    bytes
        .try_into()
        .map_err(|_| CollaborationError::InvalidSnapshot)
}

fn decode_hex(value: &str) -> Result<Vec<u8>, CollaborationError> {
    if !value.len().is_multiple_of(2) {
        return Err(CollaborationError::InvalidSnapshot);
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|chunk| {
            let high = hex_nibble(chunk[0]).ok_or(CollaborationError::InvalidSnapshot)?;
            let low = hex_nibble(chunk[1]).ok_or(CollaborationError::InvalidSnapshot)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn issue_event_with_parents(parents: Vec<Cid>) -> CollaborationEvent {
        CollaborationEvent::new(
            "farzeen/gitmesh",
            CollaborationEventKind::IssueOpened,
            "farzeen",
            parents,
            1_787_166_000,
            CollaborationPayload::issue(
                "Persist collaboration event logs",
                "Store issue and discussion events as signed, repo-scoped objects.",
                vec!["protocol".to_string()],
            ),
        )
        .unwrap()
    }

    #[test]
    fn event_id_is_stable_for_same_body() {
        let first = issue_event_with_parents(Vec::new());
        let second = issue_event_with_parents(Vec::new());

        assert_eq!(first.event_id, second.event_id);
    }

    #[test]
    fn parent_change_changes_event_id() {
        let first = issue_event_with_parents(Vec::new());
        let second = issue_event_with_parents(vec![first.event_id]);

        assert_ne!(first.event_id, second.event_id);
    }

    #[test]
    fn sample_summaries_have_stable_numbers() {
        let issues = sample_issues();
        let prs = sample_pull_requests();

        assert_eq!(issues[0].number, 1);
        assert_eq!(prs[0].number, 1);
        assert!(prs[0].source_ref.starts_with("refs/heads/"));
    }

    #[test]
    fn event_store_summarizes_issues_and_pull_requests_by_repo() {
        let mut store = CollaborationEventStore::default();
        for event in sample_issue_events()
            .into_iter()
            .chain(sample_pull_request_events())
        {
            assert!(store.insert(event));
        }

        let issues = store.issue_summaries("farzeen/gitmesh");
        let prs = store.pull_request_summaries("farzeen/gitmesh");

        assert_eq!(store.event_count(), 4);
        assert_eq!(issues.len(), 2);
        assert_eq!(prs.len(), 2);
        assert_eq!(issues[1].number, 2);
        assert_eq!(prs[0].source_ref, "refs/heads/collaboration-cli");
        assert!(store.issue_summaries("other/repo").is_empty());
    }

    #[test]
    fn event_store_rejects_duplicate_events() {
        let mut store = CollaborationEventStore::default();
        let event = sample_issue_events().remove(0);

        assert!(store.insert(event.clone()));
        assert!(!store.insert(event));
        assert_eq!(store.event_count(), 1);
    }

    #[test]
    fn event_store_snapshot_round_trips_and_detects_tampering() {
        let mut store = CollaborationEventStore::default();
        for event in sample_issue_events()
            .into_iter()
            .chain(sample_pull_request_events())
        {
            store.insert(event);
        }

        let snapshot = store.to_snapshot().unwrap();
        let restored = CollaborationEventStore::from_snapshot(&snapshot).unwrap();
        let tampered = snapshot.replacen("50657273697374", "54616d7065726564", 1);

        assert_eq!(restored, store);
        assert!(CollaborationEventStore::from_snapshot(&tampered).is_err());
    }

    #[test]
    fn event_store_saves_and_loads_snapshot_file() {
        let path = std::env::temp_dir().join(format!(
            "gitmesh-collaboration-store-test-{}.tsv",
            std::process::id()
        ));
        let mut store = CollaborationEventStore::default();
        store.insert(sample_issue_events().remove(0));

        store.save_to_path(&path).unwrap();
        let restored = CollaborationEventStore::load_from_path(&path).unwrap();
        let _ = std::fs::remove_file(path);

        assert_eq!(restored.event_count(), 1);
        assert_eq!(restored.issue_summaries("farzeen/gitmesh")[0].number, 1);
    }

    #[test]
    fn empty_titles_are_rejected() {
        let err = CollaborationEvent::new(
            "farzeen/gitmesh",
            CollaborationEventKind::IssueOpened,
            "farzeen",
            Vec::new(),
            1,
            CollaborationPayload::issue("", "body", Vec::new()),
        )
        .unwrap_err();

        assert!(matches!(err, CollaborationError::EmptyTitle));
    }
}
