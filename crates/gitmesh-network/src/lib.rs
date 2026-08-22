//! V0 network/discovery primitives.
//!
//! This crate deliberately starts with an in-memory availability directory. It
//! gives storage and future daemons a real boundary for provider discovery
//! before libp2p is introduced.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use gitmesh_core::{Cid, shard_cid};
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

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct OperatorId(String);

impl OperatorId {
    pub fn new(value: impl Into<String>) -> Result<Self, NetworkError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(NetworkError::InvalidOperatorId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OperatorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NodeDescriptor {
    pub peer_id: PeerId,
    pub operator_id: OperatorId,
    pub roles: BTreeSet<NodeRole>,
    pub region: String,
    pub protocols: BTreeSet<ProtocolId>,
}

impl NodeDescriptor {
    pub fn new(
        peer_id: PeerId,
        operator_id: OperatorId,
        roles: impl IntoIterator<Item = NodeRole>,
        region: impl Into<String>,
        protocols: impl IntoIterator<Item = ProtocolId>,
    ) -> Result<Self, NetworkError> {
        let region = region.into();
        if region.is_empty()
            || region.len() > 32
            || !region
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(NetworkError::InvalidRegion);
        }
        let roles = roles.into_iter().collect::<BTreeSet<_>>();
        if roles.is_empty() {
            return Err(NetworkError::InvalidNodeDescriptor);
        }
        let protocols = protocols.into_iter().collect::<BTreeSet<_>>();
        if protocols.is_empty() {
            return Err(NetworkError::InvalidNodeDescriptor);
        }
        Ok(Self {
            peer_id,
            operator_id,
            roles,
            region,
            protocols,
        })
    }

    pub fn has_role(&self, role: NodeRole) -> bool {
        self.roles.contains(&role)
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ProtocolId {
    PingV0,
    AvailabilityV0,
    ShardTransferV0,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShardRef {
    pub segment_cid: Cid,
    pub shard_cid: Cid,
    pub shard_index: usize,
}

impl ShardRef {
    pub fn new(segment_cid: Cid, shard_index: usize, bytes: &[u8]) -> Self {
        Self {
            segment_cid,
            shard_cid: shard_cid(segment_cid, shard_index, bytes),
            shard_index,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShardEnvelope {
    pub shard_ref: ShardRef,
    pub shard_count: usize,
    pub data_shards: usize,
    pub bytes: Vec<u8>,
}

impl ShardEnvelope {
    pub fn new(
        segment_cid: Cid,
        shard_index: usize,
        shard_count: usize,
        data_shards: usize,
        bytes: Vec<u8>,
    ) -> Result<Self, NetworkError> {
        if shard_count == 0 || data_shards == 0 || data_shards > shard_count {
            return Err(NetworkError::InvalidShardEnvelope);
        }
        if shard_index >= shard_count {
            return Err(NetworkError::InvalidShardIndex);
        }
        let shard_ref = ShardRef::new(segment_cid, shard_index, &bytes);
        Ok(Self {
            shard_ref,
            shard_count,
            data_shards,
            bytes,
        })
    }

    pub fn verify(&self) -> Result<(), NetworkError> {
        if self.shard_count == 0
            || self.data_shards == 0
            || self.data_shards > self.shard_count
            || self.shard_ref.shard_index >= self.shard_count
        {
            return Err(NetworkError::InvalidShardEnvelope);
        }
        let expected = shard_cid(
            self.shard_ref.segment_cid,
            self.shard_ref.shard_index,
            &self.bytes,
        );
        if expected != self.shard_ref.shard_cid {
            return Err(NetworkError::ShardIntegrity);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderLease {
    pub lease_epoch: u64,
    pub expires_at_unix: u64,
}

impl ProviderLease {
    pub fn new(lease_epoch: u64, expires_at_unix: u64) -> Result<Self, NetworkError> {
        if expires_at_unix == 0 {
            return Err(NetworkError::InvalidProviderRecord);
        }
        Ok(Self {
            lease_epoch,
            expires_at_unix,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShardProviderRecord {
    pub segment_cid: Cid,
    pub shard_cid: Cid,
    pub shard_index: usize,
    pub peer_id: PeerId,
    pub operator_id: OperatorId,
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
        operator_id: OperatorId,
        roles: impl IntoIterator<Item = NodeRole>,
        lease: ProviderLease,
    ) -> Self {
        Self {
            segment_cid,
            shard_cid,
            shard_index,
            peer_id,
            operator_id,
            roles: roles.into_iter().collect(),
            lease_epoch: lease.lease_epoch,
            expires_at_unix: lease.expires_at_unix,
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
    pub fn publish(&mut self, record: ShardProviderRecord) -> Result<(), NetworkError> {
        if record.expires_at_unix == 0 || record.roles.is_empty() {
            return Err(NetworkError::InvalidProviderRecord);
        }
        let records = self
            .records_by_segment
            .entry(record.segment_cid)
            .or_default();
        if let Some(existing) = records.iter_mut().find(|existing| {
            existing.shard_cid == record.shard_cid
                && existing.peer_id == record.peer_id
                && existing.shard_index == record.shard_index
        }) {
            if record.lease_epoch >= existing.lease_epoch {
                *existing = record;
            }
        } else {
            records.push(record);
        }
        Ok(())
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

    pub fn qualified_durable_shard_count(&self, segment_cid: Cid, now_unix: u64) -> usize {
        self.active_records_for_segment(segment_cid, now_unix)
            .into_iter()
            .filter(ShardProviderRecord::counts_for_durability)
            .map(|record| (record.shard_index, record.operator_id))
            .collect::<BTreeSet<_>>()
            .len()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlacementPolicy {
    pub shard_count: usize,
    pub min_distinct_operators: usize,
    pub min_distinct_regions: usize,
    pub require_distinct_operator_per_shard: bool,
}

impl PlacementPolicy {
    pub fn new(
        shard_count: usize,
        min_distinct_operators: usize,
        min_distinct_regions: usize,
        require_distinct_operator_per_shard: bool,
    ) -> Result<Self, NetworkError> {
        if shard_count == 0
            || min_distinct_operators == 0
            || min_distinct_regions == 0
            || min_distinct_operators > shard_count
            || min_distinct_regions > shard_count
        {
            return Err(NetworkError::InvalidPlacementPolicy);
        }
        Ok(Self {
            shard_count,
            min_distinct_operators,
            min_distinct_regions,
            require_distinct_operator_per_shard,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlacementAssignment {
    pub shard_index: usize,
    pub peer_id: PeerId,
    pub operator_id: OperatorId,
    pub region: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlacementPlan {
    pub assignments: Vec<PlacementAssignment>,
}

impl PlacementPlan {
    pub fn peers(&self) -> Vec<PeerId> {
        self.assignments
            .iter()
            .map(|assignment| assignment.peer_id.clone())
            .collect()
    }

    pub fn distinct_operator_count(&self) -> usize {
        self.assignments
            .iter()
            .map(|assignment| assignment.operator_id.clone())
            .collect::<BTreeSet<_>>()
            .len()
    }

    pub fn distinct_region_count(&self) -> usize {
        self.assignments
            .iter()
            .map(|assignment| assignment.region.clone())
            .collect::<BTreeSet<_>>()
            .len()
    }
}

pub fn plan_shard_placement(
    descriptors: impl IntoIterator<Item = NodeDescriptor>,
    policy: &PlacementPolicy,
) -> Result<PlacementPlan, NetworkError> {
    let mut candidates = descriptors
        .into_iter()
        .filter(|descriptor| {
            descriptor.has_role(NodeRole::Storage)
                && descriptor.protocols.contains(&ProtocolId::ShardTransferV0)
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.peer_id.cmp(&right.peer_id));

    let mut assignments = Vec::with_capacity(policy.shard_count);
    let mut used_peers = BTreeSet::new();
    let mut used_operators = BTreeSet::new();
    let mut used_regions = BTreeSet::new();

    for descriptor in &candidates {
        if assignments.len() == policy.shard_count {
            break;
        }
        if used_peers.contains(&descriptor.peer_id) {
            continue;
        }
        if policy.require_distinct_operator_per_shard
            && used_operators.contains(&descriptor.operator_id)
        {
            continue;
        }
        let shard_index = assignments.len();
        used_peers.insert(descriptor.peer_id.clone());
        used_operators.insert(descriptor.operator_id.clone());
        used_regions.insert(descriptor.region.clone());
        assignments.push(PlacementAssignment {
            shard_index,
            peer_id: descriptor.peer_id.clone(),
            operator_id: descriptor.operator_id.clone(),
            region: descriptor.region.clone(),
        });
    }

    if assignments.len() < policy.shard_count
        || used_operators.len() < policy.min_distinct_operators
        || used_regions.len() < policy.min_distinct_regions
    {
        return Err(NetworkError::InsufficientStoragePeers);
    }

    Ok(PlacementPlan { assignments })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NetworkRequest {
    Ping,
    PublishProvider(ShardProviderRecord),
    FindProviders {
        segment_cid: Cid,
        now_unix: u64,
    },
    PutShard {
        envelope: ShardEnvelope,
        lease_epoch: u64,
        expires_at_unix: u64,
    },
    GetShard {
        shard_ref: ShardRef,
    },
    AuditShard {
        shard_ref: ShardRef,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NetworkResponse {
    Pong {
        peer_id: PeerId,
    },
    Providers(Vec<ShardProviderRecord>),
    ShardStored {
        shard_ref: ShardRef,
    },
    ShardFound {
        envelope: ShardEnvelope,
    },
    ShardAudit {
        shard_ref: ShardRef,
        present: bool,
        valid: bool,
    },
    Ack,
}

pub trait NetworkTransport {
    fn request(
        &mut self,
        from: &PeerId,
        to: &PeerId,
        request: NetworkRequest,
    ) -> Result<NetworkResponse, NetworkError>;
}

#[derive(Clone, Debug)]
pub struct InMemoryPeer {
    descriptor: NodeDescriptor,
    shards: BTreeMap<Cid, ShardEnvelope>,
    directory: InMemoryAvailabilityDirectory,
}

impl InMemoryPeer {
    pub fn new(descriptor: NodeDescriptor) -> Self {
        Self {
            descriptor,
            shards: BTreeMap::new(),
            directory: InMemoryAvailabilityDirectory::default(),
        }
    }

    pub fn descriptor(&self) -> &NodeDescriptor {
        &self.descriptor
    }

    pub fn stored_shard_count(&self) -> usize {
        self.shards.len()
    }

    fn handle(&mut self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        match request {
            NetworkRequest::Ping => Ok(NetworkResponse::Pong {
                peer_id: self.descriptor.peer_id.clone(),
            }),
            NetworkRequest::PublishProvider(record) => {
                self.require_protocol(ProtocolId::AvailabilityV0)?;
                self.directory.publish(record)?;
                Ok(NetworkResponse::Ack)
            }
            NetworkRequest::FindProviders {
                segment_cid,
                now_unix,
            } => {
                self.require_protocol(ProtocolId::AvailabilityV0)?;
                Ok(NetworkResponse::Providers(
                    self.directory
                        .active_records_for_segment(segment_cid, now_unix),
                ))
            }
            NetworkRequest::PutShard {
                envelope,
                lease_epoch,
                expires_at_unix,
            } => {
                self.require_role(NodeRole::Storage)?;
                self.require_protocol(ProtocolId::ShardTransferV0)?;
                envelope.verify()?;
                let shard_ref = envelope.shard_ref.clone();
                self.shards.insert(shard_ref.shard_cid, envelope);
                self.directory.publish(ShardProviderRecord::new(
                    shard_ref.segment_cid,
                    shard_ref.shard_cid,
                    shard_ref.shard_index,
                    self.descriptor.peer_id.clone(),
                    self.descriptor.operator_id.clone(),
                    self.descriptor.roles.clone(),
                    ProviderLease::new(lease_epoch, expires_at_unix)?,
                ))?;
                Ok(NetworkResponse::ShardStored { shard_ref })
            }
            NetworkRequest::GetShard { shard_ref } => {
                self.require_protocol(ProtocolId::ShardTransferV0)?;
                let envelope = self
                    .shards
                    .get(&shard_ref.shard_cid)
                    .ok_or(NetworkError::ShardNotFound)?;
                if envelope.shard_ref != shard_ref {
                    return Err(NetworkError::ShardIntegrity);
                }
                envelope.verify()?;
                Ok(NetworkResponse::ShardFound {
                    envelope: envelope.clone(),
                })
            }
            NetworkRequest::AuditShard { shard_ref } => {
                self.require_protocol(ProtocolId::ShardTransferV0)?;
                let present = self.shards.contains_key(&shard_ref.shard_cid);
                let valid = self
                    .shards
                    .get(&shard_ref.shard_cid)
                    .map(|envelope| envelope.shard_ref == shard_ref && envelope.verify().is_ok())
                    .unwrap_or(false);
                Ok(NetworkResponse::ShardAudit {
                    shard_ref,
                    present,
                    valid,
                })
            }
        }
    }

    fn require_role(&self, role: NodeRole) -> Result<(), NetworkError> {
        if self.descriptor.has_role(role) {
            Ok(())
        } else {
            Err(NetworkError::UnsupportedRole)
        }
    }

    fn require_protocol(&self, protocol: ProtocolId) -> Result<(), NetworkError> {
        if self.descriptor.protocols.contains(&protocol) {
            Ok(())
        } else {
            Err(NetworkError::UnsupportedProtocol)
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct InMemorySwarm {
    peers: BTreeMap<PeerId, InMemoryPeer>,
}

impl InMemorySwarm {
    pub fn add_peer(&mut self, peer: InMemoryPeer) -> Result<(), NetworkError> {
        let peer_id = peer.descriptor.peer_id.clone();
        if self.peers.insert(peer_id, peer).is_some() {
            return Err(NetworkError::DuplicatePeer);
        }
        Ok(())
    }

    pub fn peer(&self, peer_id: &PeerId) -> Option<&InMemoryPeer> {
        self.peers.get(peer_id)
    }

    pub fn remove_peer(&mut self, peer_id: &PeerId) -> Result<InMemoryPeer, NetworkError> {
        self.peers.remove(peer_id).ok_or(NetworkError::UnknownPeer)
    }

    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    pub fn storage_peers(&self) -> Vec<PeerId> {
        self.peers
            .values()
            .filter(|peer| peer.descriptor.has_role(NodeRole::Storage))
            .map(|peer| peer.descriptor.peer_id.clone())
            .collect()
    }

    pub fn descriptors(&self) -> Vec<NodeDescriptor> {
        self.peers
            .values()
            .map(|peer| peer.descriptor.clone())
            .collect()
    }

    pub fn remove_shard(&mut self, peer_id: &PeerId, shard_cid: Cid) -> Result<(), NetworkError> {
        let peer = self
            .peers
            .get_mut(peer_id)
            .ok_or(NetworkError::UnknownPeer)?;
        peer.shards
            .remove(&shard_cid)
            .map(|_| ())
            .ok_or(NetworkError::ShardNotFound)
    }

    pub fn corrupt_shard(&mut self, peer_id: &PeerId, shard_cid: Cid) -> Result<(), NetworkError> {
        let peer = self
            .peers
            .get_mut(peer_id)
            .ok_or(NetworkError::UnknownPeer)?;
        let envelope = peer
            .shards
            .get_mut(&shard_cid)
            .ok_or(NetworkError::ShardNotFound)?;
        if let Some(first) = envelope.bytes.first_mut() {
            *first ^= 0xff;
        } else {
            envelope.bytes.push(0xff);
        }
        Ok(())
    }
}

impl NetworkTransport for InMemorySwarm {
    fn request(
        &mut self,
        from: &PeerId,
        to: &PeerId,
        request: NetworkRequest,
    ) -> Result<NetworkResponse, NetworkError> {
        if !self.peers.contains_key(from) {
            return Err(NetworkError::UnknownPeer);
        }
        let peer = self.peers.get_mut(to).ok_or(NetworkError::UnknownPeer)?;
        peer.handle(request)
    }
}

pub fn storage_descriptor(
    peer: &str,
    operator: &str,
    region: &str,
) -> Result<NodeDescriptor, NetworkError> {
    NodeDescriptor::new(
        PeerId::new(peer)?,
        OperatorId::new(operator)?,
        [NodeRole::Storage],
        region,
        [
            ProtocolId::PingV0,
            ProtocolId::AvailabilityV0,
            ProtocolId::ShardTransferV0,
        ],
    )
}

pub fn client_descriptor(peer: &str) -> Result<NodeDescriptor, NetworkError> {
    NodeDescriptor::new(
        PeerId::new(peer)?,
        OperatorId::new(format!("{peer}-operator"))?,
        [NodeRole::Client],
        "client",
        [ProtocolId::PingV0],
    )
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum NetworkError {
    #[error("peer id must be non-empty ASCII alphanumeric, '-' or '_'")]
    InvalidPeerId,
    #[error("operator id must be non-empty ASCII alphanumeric, '-' or '_'")]
    InvalidOperatorId,
    #[error("region must be non-empty ASCII alphanumeric, '-' or '_'")]
    InvalidRegion,
    #[error("node descriptor requires at least one role and protocol")]
    InvalidNodeDescriptor,
    #[error("invalid provider record")]
    InvalidProviderRecord,
    #[error("invalid shard envelope")]
    InvalidShardEnvelope,
    #[error("invalid shard index")]
    InvalidShardIndex,
    #[error("shard integrity check failed")]
    ShardIntegrity,
    #[error("peer is not known")]
    UnknownPeer,
    #[error("peer already exists")]
    DuplicatePeer,
    #[error("peer does not support required role")]
    UnsupportedRole,
    #[error("peer does not support required protocol")]
    UnsupportedProtocol,
    #[error("shard not found")]
    ShardNotFound,
    #[error("invalid placement policy")]
    InvalidPlacementPolicy,
    #[error("not enough qualified independent storage peers")]
    InsufficientStoragePeers,
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitmesh_core::{CidKind, HashAlgorithm};

    fn cid(kind: CidKind, byte: u8) -> Cid {
        Cid::new(kind, HashAlgorithm::Blake3_256, &[byte])
    }

    fn provider(
        segment: Cid,
        shard_index: usize,
        peer: &str,
        operator: &str,
        expires: u64,
    ) -> ShardProviderRecord {
        let bytes = vec![shard_index as u8, 42];
        ShardProviderRecord::new(
            segment,
            shard_cid(segment, shard_index, &bytes),
            shard_index,
            PeerId::new(peer).unwrap(),
            OperatorId::new(operator).unwrap(),
            [NodeRole::Storage],
            ProviderLease::new(1, expires).unwrap(),
        )
    }

    #[test]
    fn availability_directory_filters_expired_records() {
        let segment = cid(CidKind::EncryptedSegment, 1);
        let mut directory = InMemoryAvailabilityDirectory::default();
        directory
            .publish(provider(segment, 0, "peer-a", "op-a", 50))
            .unwrap();
        directory
            .publish(provider(segment, 1, "peer-b", "op-b", 150))
            .unwrap();

        assert_eq!(directory.active_records_for_segment(segment, 100).len(), 1);
        assert_eq!(directory.durable_shard_count(segment, 100), 1);
    }

    #[test]
    fn cache_records_do_not_count_for_durability() {
        let segment = cid(CidKind::EncryptedSegment, 1);
        let mut directory = InMemoryAvailabilityDirectory::default();
        let mut record = provider(segment, 0, "cache-a", "op-cache", 150);
        record.roles = [NodeRole::Cache].into_iter().collect();
        directory.publish(record).unwrap();

        assert_eq!(directory.active_records_for_segment(segment, 100).len(), 1);
        assert_eq!(directory.durable_shard_count(segment, 100), 0);
    }

    #[test]
    fn newer_provider_records_replace_older_lease_epochs() {
        let segment = cid(CidKind::EncryptedSegment, 1);
        let mut directory = InMemoryAvailabilityDirectory::default();
        let mut old = provider(segment, 0, "peer-a", "op-a", 50);
        old.lease_epoch = 1;
        let mut new = provider(segment, 0, "peer-a", "op-a", 200);
        new.lease_epoch = 2;

        directory.publish(old).unwrap();
        directory.publish(new).unwrap();

        let records = directory.active_records_for_segment(segment, 100);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].lease_epoch, 2);
        assert_eq!(records[0].expires_at_unix, 200);
    }

    #[test]
    fn shard_envelope_verifies_integrity() {
        let segment = cid(CidKind::EncryptedSegment, 9);
        let mut envelope = ShardEnvelope::new(segment, 0, 16, 10, b"hello".to_vec()).unwrap();

        envelope.verify().unwrap();
        envelope.bytes[0] ^= 0xff;

        assert_eq!(envelope.verify().unwrap_err(), NetworkError::ShardIntegrity);
    }

    #[test]
    fn in_memory_swarm_routes_put_get_and_audit_shard_requests() {
        let client = PeerId::new("client-a").unwrap();
        let storage = PeerId::new("storage-a").unwrap();
        let segment = cid(CidKind::EncryptedSegment, 7);
        let envelope = ShardEnvelope::new(segment, 0, 16, 10, b"shard bytes".to_vec()).unwrap();
        let shard_ref = envelope.shard_ref.clone();
        let mut swarm = InMemorySwarm::default();
        swarm
            .add_peer(InMemoryPeer::new(client_descriptor("client-a").unwrap()))
            .unwrap();
        swarm
            .add_peer(InMemoryPeer::new(
                storage_descriptor("storage-a", "operator-a", "iad").unwrap(),
            ))
            .unwrap();

        let stored = swarm
            .request(
                &client,
                &storage,
                NetworkRequest::PutShard {
                    envelope: envelope.clone(),
                    lease_epoch: 1,
                    expires_at_unix: 500,
                },
            )
            .unwrap();
        let fetched = swarm
            .request(
                &client,
                &storage,
                NetworkRequest::GetShard {
                    shard_ref: shard_ref.clone(),
                },
            )
            .unwrap();
        let audit = swarm
            .request(
                &client,
                &storage,
                NetworkRequest::AuditShard {
                    shard_ref: shard_ref.clone(),
                },
            )
            .unwrap();

        assert_eq!(
            stored,
            NetworkResponse::ShardStored {
                shard_ref: shard_ref.clone()
            }
        );
        assert_eq!(fetched, NetworkResponse::ShardFound { envelope });
        assert_eq!(
            audit,
            NetworkResponse::ShardAudit {
                shard_ref,
                present: true,
                valid: true
            }
        );
        assert_eq!(swarm.peer(&storage).unwrap().stored_shard_count(), 1);
    }

    #[test]
    fn in_memory_swarm_audit_distinguishes_corrupt_and_missing_shards() {
        let client = PeerId::new("client-a").unwrap();
        let storage = PeerId::new("storage-a").unwrap();
        let segment = cid(CidKind::EncryptedSegment, 7);
        let envelope = ShardEnvelope::new(segment, 0, 16, 10, b"shard bytes".to_vec()).unwrap();
        let shard_ref = envelope.shard_ref.clone();
        let mut swarm = InMemorySwarm::default();
        swarm
            .add_peer(InMemoryPeer::new(client_descriptor("client-a").unwrap()))
            .unwrap();
        swarm
            .add_peer(InMemoryPeer::new(
                storage_descriptor("storage-a", "operator-a", "iad").unwrap(),
            ))
            .unwrap();
        swarm
            .request(
                &client,
                &storage,
                NetworkRequest::PutShard {
                    envelope,
                    lease_epoch: 1,
                    expires_at_unix: 500,
                },
            )
            .unwrap();

        swarm.corrupt_shard(&storage, shard_ref.shard_cid).unwrap();
        let corrupt_audit = swarm
            .request(
                &client,
                &storage,
                NetworkRequest::AuditShard {
                    shard_ref: shard_ref.clone(),
                },
            )
            .unwrap();
        swarm.remove_shard(&storage, shard_ref.shard_cid).unwrap();
        let missing_audit = swarm
            .request(
                &client,
                &storage,
                NetworkRequest::AuditShard {
                    shard_ref: shard_ref.clone(),
                },
            )
            .unwrap();

        assert_eq!(
            corrupt_audit,
            NetworkResponse::ShardAudit {
                shard_ref: shard_ref.clone(),
                present: true,
                valid: false
            }
        );
        assert_eq!(
            missing_audit,
            NetworkResponse::ShardAudit {
                shard_ref,
                present: false,
                valid: false
            }
        );
    }

    #[test]
    fn client_peer_cannot_store_shards() {
        let client = PeerId::new("client-a").unwrap();
        let other_client = PeerId::new("client-b").unwrap();
        let envelope =
            ShardEnvelope::new(cid(CidKind::EncryptedSegment, 7), 0, 16, 10, b"x".to_vec())
                .unwrap();
        let mut swarm = InMemorySwarm::default();
        swarm
            .add_peer(InMemoryPeer::new(client_descriptor("client-a").unwrap()))
            .unwrap();
        swarm
            .add_peer(InMemoryPeer::new(client_descriptor("client-b").unwrap()))
            .unwrap();

        let err = swarm
            .request(
                &client,
                &other_client,
                NetworkRequest::PutShard {
                    envelope,
                    lease_epoch: 1,
                    expires_at_unix: 500,
                },
            )
            .unwrap_err();

        assert_eq!(err, NetworkError::UnsupportedRole);
    }

    #[test]
    fn provider_discovery_uses_availability_protocol() {
        let client = PeerId::new("client-a").unwrap();
        let directory = PeerId::new("directory-a").unwrap();
        let segment = cid(CidKind::EncryptedSegment, 3);
        let mut swarm = InMemorySwarm::default();
        swarm
            .add_peer(InMemoryPeer::new(client_descriptor("client-a").unwrap()))
            .unwrap();
        swarm
            .add_peer(InMemoryPeer::new(
                NodeDescriptor::new(
                    directory.clone(),
                    OperatorId::new("operator-directory").unwrap(),
                    [NodeRole::Dht],
                    "iad",
                    [ProtocolId::PingV0, ProtocolId::AvailabilityV0],
                )
                .unwrap(),
            ))
            .unwrap();

        swarm
            .request(
                &client,
                &directory,
                NetworkRequest::PublishProvider(provider(segment, 0, "storage-a", "op-a", 500)),
            )
            .unwrap();
        let response = swarm
            .request(
                &client,
                &directory,
                NetworkRequest::FindProviders {
                    segment_cid: segment,
                    now_unix: 100,
                },
            )
            .unwrap();

        let NetworkResponse::Providers(records) = response else {
            panic!("expected providers response");
        };
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].peer_id, PeerId::new("storage-a").unwrap());
    }

    #[test]
    fn placement_selects_distinct_storage_operators_and_regions() {
        let descriptors = vec![
            storage_descriptor("storage-a", "operator-a", "iad").unwrap(),
            storage_descriptor("storage-b", "operator-b", "sfo").unwrap(),
            storage_descriptor("storage-c", "operator-c", "fra").unwrap(),
            storage_descriptor("storage-d", "operator-c", "fra").unwrap(),
            client_descriptor("client-a").unwrap(),
        ];
        let policy = PlacementPolicy::new(3, 3, 2, true).unwrap();

        let plan = plan_shard_placement(descriptors, &policy).unwrap();

        assert_eq!(plan.assignments.len(), 3);
        assert_eq!(plan.distinct_operator_count(), 3);
        assert_eq!(plan.distinct_region_count(), 3);
        assert_eq!(
            plan.peers(),
            vec![
                PeerId::new("storage-a").unwrap(),
                PeerId::new("storage-b").unwrap(),
                PeerId::new("storage-c").unwrap()
            ]
        );
    }

    #[test]
    fn placement_rejects_insufficient_independent_operators() {
        let descriptors = vec![
            storage_descriptor("storage-a", "operator-a", "iad").unwrap(),
            storage_descriptor("storage-b", "operator-a", "sfo").unwrap(),
            storage_descriptor("storage-c", "operator-a", "fra").unwrap(),
        ];
        let policy = PlacementPolicy::new(3, 3, 2, true).unwrap();

        let err = plan_shard_placement(descriptors, &policy).unwrap_err();

        assert_eq!(err, NetworkError::InsufficientStoragePeers);
    }
}
