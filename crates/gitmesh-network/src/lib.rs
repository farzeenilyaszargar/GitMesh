//! V0 network/discovery primitives.
//!
//! This crate deliberately starts with an in-memory availability directory. It
//! gives storage and future daemons a real boundary for provider discovery
//! before libp2p is introduced.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use gitmesh_core::Cid;
use thiserror::Error;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PeerId(String);

impl PeerId {
    pub fn new(value: impl Into<String>) -> Result<Self, NetworkError> {
        let value = value.into();
        if value.is_empty()
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(NetworkError::InvalidPeerId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum NodeRole {
    Client,
    Cache,
    Storage,
    Bootstrap,
    Relay,
    Dht,
    Gateway,
    Coordinator,
    Repair,
    Indexer,
    Runner,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShardProviderRecord {
    pub segment_cid: Cid,
    pub shard_cid: Cid,
    pub shard_index: usize,
    pub peer_id: PeerId,
    pub roles: BTreeSet<NodeRole>,
    pub lease_epoch: u64,
    pub expires_at_unix: u64,
}

impl ShardProviderRecord {
    pub fn new(
        segment_cid: Cid,
        shard_cid: Cid,
        shard_index: usize,
        peer_id: PeerId,
        roles: impl IntoIterator<Item = NodeRole>,
        lease_epoch: u64,
        expires_at_unix: u64,
    ) -> Self {
        Self {
            segment_cid,
            shard_cid,
            shard_index,
            peer_id,
            roles: roles.into_iter().collect(),
            lease_epoch,
            expires_at_unix,
        }
    }

    pub fn is_active_at(&self, now_unix: u64) -> bool {
        self.expires_at_unix > now_unix
    }

    pub fn counts_for_durability(&self) -> bool {
        self.roles.contains(&NodeRole::Storage)
    }
}

#[derive(Clone, Debug, Default)]
pub struct InMemoryAvailabilityDirectory {
    records_by_segment: BTreeMap<Cid, Vec<ShardProviderRecord>>,
}

impl InMemoryAvailabilityDirectory {
    pub fn publish(&mut self, record: ShardProviderRecord) {
        self.records_by_segment
            .entry(record.segment_cid)
            .or_default()
            .push(record);
    }

    pub fn active_records_for_segment(
        &self,
        segment_cid: Cid,
        now_unix: u64,
    ) -> Vec<ShardProviderRecord> {
        self.records_by_segment
            .get(&segment_cid)
            .into_iter()
            .flatten()
            .filter(|record| record.is_active_at(now_unix))
            .cloned()
            .collect()
    }

    pub fn durable_shard_count(&self, segment_cid: Cid, now_unix: u64) -> usize {
        self.active_records_for_segment(segment_cid, now_unix)
            .into_iter()
            .filter(ShardProviderRecord::counts_for_durability)
            .map(|record| record.shard_index)
            .collect::<BTreeSet<_>>()
            .len()
    }
}

#[derive(Debug, Error)]
pub enum NetworkError {
    #[error("peer id must be non-empty ASCII alphanumeric, '-' or '_'")]
    InvalidPeerId,
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitmesh_core::{CidKind, HashAlgorithm};

    fn cid(kind: CidKind, byte: u8) -> Cid {
        Cid::new(kind, HashAlgorithm::Blake3_256, &[byte])
    }

    #[test]
    fn availability_directory_filters_expired_records() {
        let segment = cid(CidKind::EncryptedSegment, 1);
        let mut directory = InMemoryAvailabilityDirectory::default();
        directory.publish(ShardProviderRecord::new(
            segment,
            cid(CidKind::Shard, 2),
            0,
            PeerId::new("peer-a").unwrap(),
            [NodeRole::Storage],
            0,
            50,
        ));
        directory.publish(ShardProviderRecord::new(
            segment,
            cid(CidKind::Shard, 3),
            1,
            PeerId::new("peer-b").unwrap(),
            [NodeRole::Storage],
            0,
            150,
        ));

        assert_eq!(directory.active_records_for_segment(segment, 100).len(), 1);
        assert_eq!(directory.durable_shard_count(segment, 100), 1);
    }

    #[test]
    fn cache_records_do_not_count_for_durability() {
        let segment = cid(CidKind::EncryptedSegment, 1);
        let mut directory = InMemoryAvailabilityDirectory::default();
        directory.publish(ShardProviderRecord::new(
            segment,
            cid(CidKind::Shard, 2),
            0,
            PeerId::new("cache-a").unwrap(),
            [NodeRole::Cache],
            0,
            150,
        ));

        assert_eq!(directory.active_records_for_segment(segment, 100).len(), 1);
        assert_eq!(directory.durable_shard_count(segment, 100), 0);
    }
}
