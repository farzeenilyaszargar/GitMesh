//! Collaboration event primitives for GitMesh.
//!
//! Issues, pull requests, comments, reviews, and later releases/discussions are
//! derived collaboration state. Their durable representation is an independently
//! verifiable event graph, not a web database row.

use gitmesh_core::{Cid, CoreError, ProtocolEnvelope};
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
    #[error(transparent)]
    Core(#[from] CoreError),
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
