//! V0 local storage proof for GitMesh.
//!
//! This crate intentionally models only the first roadmap milestone:
//! bytes -> encrypt -> erasure code -> distribute -> lose shards -> reconstruct
//! -> decrypt -> exact original bytes.

use gitmesh_core::{Cid, encrypted_segment_cid, shard_cid};
use gitmesh_crypto::{
    AeadAlgorithm, CryptoError, SegmentKey, SegmentNonce,
    decrypt_segment_bytes as crypto_decrypt_segment_bytes,
    encrypt_segment as crypto_encrypt_segment,
};
use gitmesh_network::{
    InMemoryAvailabilityDirectory, NetworkError, NetworkRequest, NetworkResponse, NetworkTransport,
    NodeDescriptor, NodeRole, OperatorId, PeerId, PlacementPlan, PlacementPolicy, ProviderLease,
    ShardAuditChallenge, ShardEnvelope, ShardProviderRecord, ShardRef, SignedShardProviderRecord,
    plan_shard_placement,
};
use reed_solomon_erasure::galois_8::ReedSolomon;
use thiserror::Error;

const ENCRYPTION_AAD: &[u8] = b"gitmesh.v0.segment";

pub use gitmesh_core::hex;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("erasure coding failed: {0}")]
    Erasure(String),
    #[error("crypto failed: {0}")]
    Crypto(#[from] CryptoError),
    #[error("node {0} does not exist")]
    MissingNode(usize),
    #[error("not enough shards to reconstruct: have {available}, need {required}")]
    NotEnoughShards { available: usize, required: usize },
    #[error("invalid shard index {0}")]
    InvalidShardIndex(usize),
    #[error("network discovery failed: {0}")]
    Network(String),
}

pub type Result<T> = std::result::Result<T, StorageError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoragePolicy {
    pub data_shards: usize,
    pub parity_shards: usize,
}

impl StoragePolicy {
    pub fn total_shards(&self) -> usize {
        self.data_shards + self.parity_shards
    }
}

impl Default for StoragePolicy {
    fn default() -> Self {
        Self {
            data_shards: 10,
            parity_shards: 6,
        }
    }
}

#[derive(Clone, Debug)]
pub struct EncryptedSegment {
    pub cid: Cid,
    pub algorithm: AeadAlgorithm,
    pub plaintext_len: usize,
    pub ciphertext_len: usize,
    pub nonce: [u8; 24],
    pub key: [u8; 32],
    pub ciphertext: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct Shard {
    pub segment_cid: Cid,
    pub shard_index: usize,
    pub shard_count: usize,
    pub data_shards: usize,
    pub bytes: Vec<u8>,
    pub cid: Cid,
}

#[derive(Clone, Debug)]
pub struct StoredShard {
    pub node_id: usize,
    pub shard: Shard,
}

impl Shard {
    pub fn to_network_envelope(&self) -> Result<ShardEnvelope> {
        ShardEnvelope::new(
            self.segment_cid,
            self.shard_index,
            self.shard_count,
            self.data_shards,
            self.bytes.clone(),
        )
        .map_err(|err| StorageError::Network(err.to_string()))
    }

    pub fn from_network_envelope(envelope: ShardEnvelope) -> Result<Self> {
        envelope
            .verify()
            .map_err(|err| StorageError::Network(err.to_string()))?;
        Ok(Self {
            segment_cid: envelope.shard_ref.segment_cid,
            shard_index: envelope.shard_ref.shard_index,
            shard_count: envelope.shard_count,
            data_shards: envelope.data_shards,
            bytes: envelope.bytes,
            cid: envelope.shard_ref.shard_cid,
        })
    }
}

#[derive(Clone, Debug)]
pub struct SimulatedNode {
    pub id: usize,
    pub shard: Option<Shard>,
    pub online: bool,
}

#[derive(Clone, Debug)]
pub struct SimulatedNetwork {
    nodes: Vec<SimulatedNode>,
}

impl SimulatedNetwork {
    pub fn with_node_count(count: usize) -> Self {
        let nodes = (0..count)
            .map(|id| SimulatedNode {
                id,
                shard: None,
                online: true,
            })
            .collect();
        Self { nodes }
    }

    pub fn store_shards(&mut self, shards: Vec<Shard>) -> Result<()> {
        for shard in shards {
            let node = self
                .nodes
                .get_mut(shard.shard_index)
                .ok_or(StorageError::MissingNode(shard.shard_index))?;
            node.shard = Some(shard);
        }
        Ok(())
    }

    pub fn destroy_nodes(&mut self, node_ids: &[usize]) -> Result<()> {
        for &node_id in node_ids {
            let node = self
                .nodes
                .get_mut(node_id)
                .ok_or(StorageError::MissingNode(node_id))?;
            node.shard = None;
            node.online = false;
        }
        Ok(())
    }

    pub fn corrupt_node_shard(&mut self, node_id: usize) -> Result<()> {
        let node = self
            .nodes
            .get_mut(node_id)
            .ok_or(StorageError::MissingNode(node_id))?;
        let shard = node
            .shard
            .as_mut()
            .ok_or(StorageError::MissingNode(node_id))?;
        if let Some(first) = shard.bytes.first_mut() {
            *first ^= 0xff;
        } else {
            shard.bytes.push(0xff);
        }
        Ok(())
    }

    pub fn available_shards(&self) -> Vec<StoredShard> {
        self.nodes
            .iter()
            .filter(|node| node.online)
            .filter_map(|node| {
                node.shard.as_ref().map(|shard| StoredShard {
                    node_id: node.id,
                    shard: shard.clone(),
                })
            })
            .collect()
    }

    pub fn publish_availability(
        &self,
        directory: &mut InMemoryAvailabilityDirectory,
        lease_epoch: u64,
        expires_at_unix: u64,
    ) -> Result<()> {
        for stored in self.available_shards() {
            directory
                .publish(
                    ShardProviderRecord::new(
                        gitmesh_network::ShardRef {
                            segment_cid: stored.shard.segment_cid,
                            shard_cid: stored.shard.cid,
                            shard_index: stored.shard.shard_index,
                        },
                        PeerId::new(format!("v0-node-{}", stored.node_id))
                            .map_err(|err| StorageError::Network(err.to_string()))?,
                        OperatorId::new(format!("v0-operator-{}", stored.node_id))
                            .map_err(|err| StorageError::Network(err.to_string()))?,
                        "local",
                        [NodeRole::Storage],
                        ProviderLease::new(lease_epoch, expires_at_unix)
                            .map_err(|err| StorageError::Network(err.to_string()))?,
                    )
                    .map_err(|err| StorageError::Network(err.to_string()))?,
                )
                .map_err(|err| StorageError::Network(err.to_string()))?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShardAuditReport {
    pub segment_cid: Cid,
    pub total_shards: usize,
    pub required_shards: usize,
    pub verified_shards: Vec<usize>,
    pub missing_shards: Vec<usize>,
    pub corrupt_shards: Vec<usize>,
    pub durability_satisfied: bool,
    pub repair_needed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransportShardAuditReport {
    pub segment_cid: Cid,
    pub total_shards: usize,
    pub required_shards: usize,
    pub checked_providers: usize,
    pub verified_shards: Vec<usize>,
    pub missing_shards: Vec<usize>,
    pub corrupt_shards: Vec<usize>,
    pub durability_satisfied: bool,
    pub repair_needed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransportChallengeAuditReport {
    pub segment_cid: Cid,
    pub checked_providers: usize,
    pub verified_shards: Vec<usize>,
    pub failed_shards: Vec<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepairOutcome {
    pub segment_cid: Cid,
    pub repaired_shards: Vec<usize>,
    pub verified_after_repair: usize,
    pub durability_satisfied: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransportRepairOutcome {
    pub segment_cid: Cid,
    pub audit_before: TransportShardAuditReport,
    pub audit_after: TransportShardAuditReport,
    pub repaired_shards: Vec<usize>,
    pub providers_after_repair: Vec<ShardProviderRecord>,
    pub durability_satisfied: bool,
}

#[derive(Clone, Debug)]
pub struct TransportRepairRequest<'a> {
    pub client_peer: &'a PeerId,
    pub directory_peer: Option<&'a PeerId>,
    pub segment: &'a EncryptedSegment,
    pub policy: &'a StoragePolicy,
    pub providers: &'a [ShardProviderRecord],
    pub replacement_descriptors: &'a [NodeDescriptor],
    pub now_unix: u64,
    pub lease_epoch: u64,
    pub expires_at_unix: u64,
}

#[derive(Clone, Debug)]
pub struct V0ProofResult {
    pub plaintext_len: usize,
    pub ciphertext_len: usize,
    pub segment_cid: Cid,
    pub destroyed_nodes: Vec<usize>,
    pub available_shards: usize,
    pub recovered: Vec<u8>,
}

pub fn encrypt_segment(plaintext: &[u8]) -> Result<EncryptedSegment> {
    let encrypted = crypto_encrypt_segment(plaintext, ENCRYPTION_AAD)?;
    let cid = encrypted_segment_cid(&encrypted.ciphertext);

    Ok(EncryptedSegment {
        cid,
        algorithm: encrypted.algorithm,
        plaintext_len: plaintext.len(),
        ciphertext_len: encrypted.ciphertext.len(),
        nonce: encrypted.nonce.expose_bytes(),
        key: encrypted.key.expose_bytes(),
        ciphertext: encrypted.ciphertext,
    })
}

pub fn decrypt_segment(segment: &EncryptedSegment, ciphertext: &[u8]) -> Result<Vec<u8>> {
    Ok(crypto_decrypt_segment_bytes(
        segment.algorithm,
        SegmentKey::from_bytes(segment.key),
        SegmentNonce::from_bytes(segment.nonce),
        ciphertext,
        ENCRYPTION_AAD,
    )?)
}

pub fn erasure_encode(segment: &EncryptedSegment, policy: &StoragePolicy) -> Result<Vec<Shard>> {
    let r = ReedSolomon::new(policy.data_shards, policy.parity_shards)
        .map_err(|err| StorageError::Erasure(err.to_string()))?;
    let shard_len = segment.ciphertext.len().div_ceil(policy.data_shards);
    let mut shards = vec![vec![0_u8; shard_len]; policy.total_shards()];

    for (index, byte) in segment.ciphertext.iter().enumerate() {
        shards[index / shard_len][index % shard_len] = *byte;
    }

    r.encode(&mut shards)
        .map_err(|err| StorageError::Erasure(err.to_string()))?;

    Ok(shards
        .into_iter()
        .enumerate()
        .map(|(shard_index, bytes)| {
            let cid = shard_cid(segment.cid, shard_index, &bytes);
            Shard {
                segment_cid: segment.cid,
                shard_index,
                shard_count: policy.total_shards(),
                data_shards: policy.data_shards,
                bytes,
                cid,
            }
        })
        .collect())
}

pub fn reconstruct_ciphertext(
    segment: &EncryptedSegment,
    policy: &StoragePolicy,
    stored_shards: &[StoredShard],
) -> Result<Vec<u8>> {
    let available = stored_shards.len();
    if available < policy.data_shards {
        return Err(StorageError::NotEnoughShards {
            available,
            required: policy.data_shards,
        });
    }

    let r = ReedSolomon::new(policy.data_shards, policy.parity_shards)
        .map_err(|err| StorageError::Erasure(err.to_string()))?;
    let mut shards: Vec<Option<Vec<u8>>> = vec![None; policy.total_shards()];

    for stored in stored_shards {
        let shard = &stored.shard;
        if shard.shard_index >= policy.total_shards() {
            return Err(StorageError::InvalidShardIndex(shard.shard_index));
        }
        if shard.segment_cid != segment.cid
            || shard.cid != shard_cid(segment.cid, shard.shard_index, &shard.bytes)
        {
            continue;
        }
        shards[shard.shard_index] = Some(shard.bytes.clone());
    }

    let verified_available = shards.iter().filter(|shard| shard.is_some()).count();
    if verified_available < policy.data_shards {
        return Err(StorageError::NotEnoughShards {
            available: verified_available,
            required: policy.data_shards,
        });
    }

    r.reconstruct(&mut shards)
        .map_err(|err| StorageError::Erasure(err.to_string()))?;

    let mut ciphertext = Vec::with_capacity(segment.ciphertext_len);
    for shard in shards.into_iter().take(policy.data_shards) {
        let shard = shard.expect("reed-solomon reconstruct fills missing data shards");
        ciphertext.extend_from_slice(&shard);
    }
    ciphertext.truncate(segment.ciphertext_len);
    Ok(ciphertext)
}

fn reconstruct_all_shards(
    segment: &EncryptedSegment,
    policy: &StoragePolicy,
    stored_shards: &[StoredShard],
) -> Result<Vec<Shard>> {
    let available = stored_shards.len();
    if available < policy.data_shards {
        return Err(StorageError::NotEnoughShards {
            available,
            required: policy.data_shards,
        });
    }

    let r = ReedSolomon::new(policy.data_shards, policy.parity_shards)
        .map_err(|err| StorageError::Erasure(err.to_string()))?;
    let shard_len = segment.ciphertext.len().div_ceil(policy.data_shards);
    let mut shards: Vec<Option<Vec<u8>>> = vec![None; policy.total_shards()];
    for stored in stored_shards {
        let shard = &stored.shard;
        if verify_shard(segment, policy, shard) {
            shards[shard.shard_index] = Some(shard.bytes.clone());
        }
    }

    let verified_available = shards.iter().filter(|shard| shard.is_some()).count();
    if verified_available < policy.data_shards {
        return Err(StorageError::NotEnoughShards {
            available: verified_available,
            required: policy.data_shards,
        });
    }

    r.reconstruct(&mut shards)
        .map_err(|err| StorageError::Erasure(err.to_string()))?;

    shards
        .into_iter()
        .enumerate()
        .map(|(shard_index, bytes)| {
            let mut bytes = bytes.expect("reed-solomon reconstruct fills missing shards");
            if bytes.len() != shard_len {
                bytes.resize(shard_len, 0);
            }
            Ok(Shard {
                segment_cid: segment.cid,
                shard_index,
                shard_count: policy.total_shards(),
                data_shards: policy.data_shards,
                cid: shard_cid(segment.cid, shard_index, &bytes),
                bytes,
            })
        })
        .collect()
}

pub fn audit_segment_shards(
    segment: &EncryptedSegment,
    policy: &StoragePolicy,
    network: &SimulatedNetwork,
) -> Result<ShardAuditReport> {
    if network.nodes.len() < policy.total_shards() {
        return Err(StorageError::MissingNode(network.nodes.len()));
    }
    let mut verified_shards = Vec::new();
    let mut missing_shards = Vec::new();
    let mut corrupt_shards = Vec::new();

    for shard_index in 0..policy.total_shards() {
        let node = network
            .nodes
            .get(shard_index)
            .ok_or(StorageError::MissingNode(shard_index))?;
        match (&node.shard, node.online) {
            (Some(shard), true) if verify_shard(segment, policy, shard) => {
                verified_shards.push(shard_index);
            }
            (Some(_), true) => corrupt_shards.push(shard_index),
            _ => missing_shards.push(shard_index),
        }
    }

    let durability_satisfied = verified_shards.len() >= policy.data_shards;
    let repair_needed = verified_shards.len() < policy.total_shards()
        || !missing_shards.is_empty()
        || !corrupt_shards.is_empty();

    Ok(ShardAuditReport {
        segment_cid: segment.cid,
        total_shards: policy.total_shards(),
        required_shards: policy.data_shards,
        verified_shards,
        missing_shards,
        corrupt_shards,
        durability_satisfied,
        repair_needed,
    })
}

pub fn repair_segment_shards(
    segment: &EncryptedSegment,
    policy: &StoragePolicy,
    network: &mut SimulatedNetwork,
) -> Result<RepairOutcome> {
    let before = audit_segment_shards(segment, policy, network)?;
    if before.verified_shards.len() < policy.data_shards {
        return Err(StorageError::NotEnoughShards {
            available: before.verified_shards.len(),
            required: policy.data_shards,
        });
    }

    let r = ReedSolomon::new(policy.data_shards, policy.parity_shards)
        .map_err(|err| StorageError::Erasure(err.to_string()))?;
    let shard_len = segment.ciphertext.len().div_ceil(policy.data_shards);
    let mut shards: Vec<Option<Vec<u8>>> = vec![None; policy.total_shards()];
    for shard_index in before.verified_shards {
        let node = network
            .nodes
            .get(shard_index)
            .ok_or(StorageError::MissingNode(shard_index))?;
        let shard = node
            .shard
            .as_ref()
            .ok_or(StorageError::MissingNode(shard_index))?;
        shards[shard_index] = Some(shard.bytes.clone());
    }
    r.reconstruct(&mut shards)
        .map_err(|err| StorageError::Erasure(err.to_string()))?;

    let mut repaired_shards = Vec::new();
    for shard_index in before
        .missing_shards
        .into_iter()
        .chain(before.corrupt_shards)
    {
        let bytes = shards[shard_index]
            .clone()
            .ok_or(StorageError::InvalidShardIndex(shard_index))?;
        let mut trimmed = bytes;
        if trimmed.len() != shard_len {
            trimmed.resize(shard_len, 0);
        }
        let cid = shard_cid(segment.cid, shard_index, &trimmed);
        let node = network
            .nodes
            .get_mut(shard_index)
            .ok_or(StorageError::MissingNode(shard_index))?;
        node.online = true;
        node.shard = Some(Shard {
            segment_cid: segment.cid,
            shard_index,
            shard_count: policy.total_shards(),
            data_shards: policy.data_shards,
            bytes: trimmed,
            cid,
        });
        repaired_shards.push(shard_index);
    }

    let after = audit_segment_shards(segment, policy, network)?;
    Ok(RepairOutcome {
        segment_cid: segment.cid,
        repaired_shards,
        verified_after_repair: after.verified_shards.len(),
        durability_satisfied: after.durability_satisfied,
    })
}

pub fn distribute_shards_via_transport<T: NetworkTransport>(
    transport: &mut T,
    client_peer: &PeerId,
    storage_peers: &[PeerId],
    shards: &[Shard],
    lease_epoch: u64,
    expires_at_unix: u64,
) -> Result<Vec<ShardProviderRecord>> {
    if storage_peers.len() < shards.len() {
        return Err(StorageError::NotEnoughShards {
            available: storage_peers.len(),
            required: shards.len(),
        });
    }
    let mut providers = Vec::with_capacity(shards.len());
    for (peer, shard) in storage_peers.iter().zip(shards) {
        let response = transport
            .request(
                client_peer,
                peer,
                NetworkRequest::PutShard {
                    envelope: shard.to_network_envelope()?,
                    lease_epoch,
                    expires_at_unix,
                },
            )
            .map_err(|err| StorageError::Network(err.to_string()))?;
        let NetworkResponse::ShardStored { shard_ref } = response else {
            return Err(StorageError::Network(
                "unexpected response to PutShard".to_string(),
            ));
        };
        providers.push(
            ShardProviderRecord::new(
                shard_ref,
                peer.clone(),
                OperatorId::new(format!("remote-operator-{}", peer.as_str()))
                    .map_err(|err| StorageError::Network(err.to_string()))?,
                "unknown",
                [NodeRole::Storage],
                ProviderLease::new(lease_epoch, expires_at_unix)
                    .map_err(|err| StorageError::Network(err.to_string()))?,
            )
            .map_err(|err| StorageError::Network(err.to_string()))?,
        );
    }
    Ok(providers)
}

pub fn distribute_shards_with_plan<T: NetworkTransport>(
    transport: &mut T,
    client_peer: &PeerId,
    plan: &PlacementPlan,
    shards: &[Shard],
    lease_epoch: u64,
    expires_at_unix: u64,
) -> Result<Vec<ShardProviderRecord>> {
    if plan.assignments.len() < shards.len() {
        return Err(StorageError::NotEnoughShards {
            available: plan.assignments.len(),
            required: shards.len(),
        });
    }
    let peers = plan
        .assignments
        .iter()
        .take(shards.len())
        .map(|assignment| assignment.peer_id.clone())
        .collect::<Vec<_>>();
    let mut providers = distribute_shards_via_transport(
        transport,
        client_peer,
        &peers,
        shards,
        lease_epoch,
        expires_at_unix,
    )?;
    for (provider, assignment) in providers.iter_mut().zip(&plan.assignments) {
        provider.operator_id = assignment.operator_id.clone();
    }
    Ok(providers)
}

pub fn plan_and_distribute_shards<T: NetworkTransport>(
    transport: &mut T,
    client_peer: &PeerId,
    descriptors: impl IntoIterator<Item = NodeDescriptor>,
    placement_policy: &PlacementPolicy,
    shards: &[Shard],
    lease_epoch: u64,
    expires_at_unix: u64,
) -> Result<(PlacementPlan, Vec<ShardProviderRecord>)> {
    let plan = plan_shard_placement(descriptors, placement_policy)
        .map_err(|err| StorageError::Network(err.to_string()))?;
    let providers = distribute_shards_with_plan(
        transport,
        client_peer,
        &plan,
        shards,
        lease_epoch,
        expires_at_unix,
    )?;
    Ok((plan, providers))
}

pub fn fetch_shards_via_transport<T: NetworkTransport>(
    transport: &mut T,
    client_peer: &PeerId,
    providers: &[ShardProviderRecord],
    now_unix: u64,
) -> Result<Vec<StoredShard>> {
    let mut stored = Vec::new();
    for provider in providers {
        if !provider.is_active_at(now_unix) {
            continue;
        }
        let response = transport
            .request(
                client_peer,
                &provider.peer_id,
                NetworkRequest::GetShard {
                    shard_ref: ShardRef {
                        segment_cid: provider.segment_cid,
                        shard_cid: provider.shard_cid,
                        shard_index: provider.shard_index,
                    },
                },
            )
            .map_err(|err| StorageError::Network(err.to_string()))?;
        let NetworkResponse::ShardFound { envelope } = response else {
            return Err(StorageError::Network(
                "unexpected response to GetShard".to_string(),
            ));
        };
        stored.push(StoredShard {
            node_id: provider.shard_index,
            shard: Shard::from_network_envelope(envelope)?,
        });
    }
    Ok(stored)
}

pub fn publish_providers_via_transport<T: NetworkTransport>(
    transport: &mut T,
    client_peer: &PeerId,
    directory_peer: &PeerId,
    providers: &[ShardProviderRecord],
) -> Result<usize> {
    for provider in providers {
        let response = transport
            .request(
                client_peer,
                directory_peer,
                NetworkRequest::PublishProvider(provider.clone()),
            )
            .map_err(|err| StorageError::Network(err.to_string()))?;
        if response != NetworkResponse::Ack {
            return Err(StorageError::Network(
                "unexpected response to PublishProvider".to_string(),
            ));
        }
    }
    Ok(providers.len())
}

pub fn publish_signed_providers_via_transport<T: NetworkTransport>(
    transport: &mut T,
    client_peer: &PeerId,
    directory_peer: &PeerId,
    providers: &[SignedShardProviderRecord],
    now_unix: u64,
) -> Result<usize> {
    for provider in providers {
        let response = transport
            .request(
                client_peer,
                directory_peer,
                NetworkRequest::PublishSignedProvider {
                    signed: Box::new(provider.clone()),
                    now_unix,
                },
            )
            .map_err(|err| StorageError::Network(err.to_string()))?;
        if response != NetworkResponse::Ack {
            return Err(StorageError::Network(
                "unexpected response to PublishSignedProvider".to_string(),
            ));
        }
    }
    Ok(providers.len())
}

pub fn discover_providers_via_transport<T: NetworkTransport>(
    transport: &mut T,
    client_peer: &PeerId,
    directory_peer: &PeerId,
    segment_cid: Cid,
    now_unix: u64,
) -> Result<Vec<ShardProviderRecord>> {
    let response = transport
        .request(
            client_peer,
            directory_peer,
            NetworkRequest::FindProviders {
                segment_cid,
                now_unix,
            },
        )
        .map_err(|err| StorageError::Network(err.to_string()))?;
    let NetworkResponse::Providers(providers) = response else {
        return Err(StorageError::Network(
            "unexpected response to FindProviders".to_string(),
        ));
    };
    Ok(providers)
}

pub fn audit_shards_via_transport<T: NetworkTransport>(
    transport: &mut T,
    client_peer: &PeerId,
    segment_cid: Cid,
    total_shards: usize,
    required_shards: usize,
    providers: &[ShardProviderRecord],
    now_unix: u64,
) -> Result<TransportShardAuditReport> {
    let mut verified = std::collections::BTreeSet::new();
    let mut missing = std::collections::BTreeSet::new();
    let mut corrupt = std::collections::BTreeSet::new();
    let mut checked_providers = 0;

    for provider in providers {
        if provider.segment_cid != segment_cid || !provider.is_active_at(now_unix) {
            continue;
        }
        checked_providers += 1;
        let shard_ref = ShardRef {
            segment_cid: provider.segment_cid,
            shard_cid: provider.shard_cid,
            shard_index: provider.shard_index,
        };
        let response = transport
            .request(
                client_peer,
                &provider.peer_id,
                NetworkRequest::AuditShard { shard_ref },
            )
            .or_else(|err| match err {
                NetworkError::UnknownPeer | NetworkError::ShardNotFound => {
                    Ok(NetworkResponse::ShardAudit {
                        shard_ref: ShardRef {
                            segment_cid: provider.segment_cid,
                            shard_cid: provider.shard_cid,
                            shard_index: provider.shard_index,
                        },
                        present: false,
                        valid: false,
                    })
                }
                err => Err(StorageError::Network(err.to_string())),
            })?;
        let NetworkResponse::ShardAudit { present, valid, .. } = response else {
            return Err(StorageError::Network(
                "unexpected response to AuditShard".to_string(),
            ));
        };
        if valid {
            verified.insert(provider.shard_index);
            missing.remove(&provider.shard_index);
            corrupt.remove(&provider.shard_index);
        } else if present {
            if !verified.contains(&provider.shard_index) {
                corrupt.insert(provider.shard_index);
                missing.remove(&provider.shard_index);
            }
        } else if !verified.contains(&provider.shard_index)
            && !corrupt.contains(&provider.shard_index)
        {
            missing.insert(provider.shard_index);
        }
    }

    let verified_shards = verified.into_iter().collect::<Vec<_>>();
    let durability_satisfied = verified_shards.len() >= required_shards;
    let repair_needed = verified_shards.len() < total_shards
        || missing.len() + corrupt.len() > 0
        || checked_providers < total_shards;

    Ok(TransportShardAuditReport {
        segment_cid,
        total_shards,
        required_shards,
        checked_providers,
        verified_shards,
        missing_shards: missing.into_iter().collect(),
        corrupt_shards: corrupt.into_iter().collect(),
        durability_satisfied,
        repair_needed,
    })
}

pub fn challenge_shards_via_transport<T: NetworkTransport>(
    transport: &mut T,
    client_peer: &PeerId,
    segment_cid: Cid,
    providers: &[ShardProviderRecord],
    now_unix: u64,
    challenge_len: usize,
) -> Result<TransportChallengeAuditReport> {
    if challenge_len == 0 {
        return Err(StorageError::Network(
            "challenge length must be non-zero".to_string(),
        ));
    }
    let mut checked_providers = 0;
    let mut verified_shards = Vec::new();
    let mut failed_shards = Vec::new();

    for provider in providers {
        if provider.segment_cid != segment_cid || !provider.is_active_at(now_unix) {
            continue;
        }
        checked_providers += 1;
        let shard_ref = ShardRef {
            segment_cid: provider.segment_cid,
            shard_cid: provider.shard_cid,
            shard_index: provider.shard_index,
        };
        let challenge = ShardAuditChallenge::new(
            shard_ref,
            0,
            challenge_len,
            audit_challenge_nonce(segment_cid, provider.shard_index),
        )
        .map_err(|err| StorageError::Network(err.to_string()))?;
        let response = transport.request(
            client_peer,
            &provider.peer_id,
            NetworkRequest::ChallengeShard {
                challenge: challenge.clone(),
            },
        );
        match response {
            Ok(NetworkResponse::ShardAuditProof { proof }) if proof.verify(&challenge).is_ok() => {
                verified_shards.push(provider.shard_index);
            }
            Ok(_)
            | Err(
                NetworkError::UnknownPeer
                | NetworkError::ShardNotFound
                | NetworkError::ShardIntegrity
                | NetworkError::InvalidAuditChallenge
                | NetworkError::InvalidAuditProof,
            ) => {
                failed_shards.push(provider.shard_index);
            }
            Err(err) => return Err(StorageError::Network(err.to_string())),
        }
    }

    Ok(TransportChallengeAuditReport {
        segment_cid,
        checked_providers,
        verified_shards,
        failed_shards,
    })
}

fn choose_repair_target(
    providers_after_repair: &[ShardProviderRecord],
    fallback_provider: &ShardProviderRecord,
    replacement_descriptors: &[NodeDescriptor],
) -> ShardProviderRecord {
    let used_peers = providers_after_repair
        .iter()
        .map(|provider| provider.peer_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let used_operators = providers_after_repair
        .iter()
        .map(|provider| provider.operator_id.clone())
        .collect::<std::collections::BTreeSet<_>>();

    replacement_descriptors
        .iter()
        .filter(|descriptor| {
            descriptor.has_role(NodeRole::Storage)
                && descriptor
                    .protocols
                    .contains(&gitmesh_network::ProtocolId::ShardTransferV0)
                && !used_peers.contains(&descriptor.peer_id)
        })
        .min_by_key(|descriptor| {
            (
                used_operators.contains(&descriptor.operator_id),
                descriptor.peer_id.clone(),
            )
        })
        .map(|descriptor| {
            let mut provider = fallback_provider.clone();
            provider.peer_id = descriptor.peer_id.clone();
            provider.operator_id = descriptor.operator_id.clone();
            provider.region = descriptor.region.clone();
            provider.roles = descriptor.roles.clone();
            provider
        })
        .unwrap_or_else(|| fallback_provider.clone())
}

fn store_repaired_shard<T: NetworkTransport>(
    transport: &mut T,
    client_peer: &PeerId,
    provider: &ShardProviderRecord,
    shard: &Shard,
    lease_epoch: u64,
    expires_at_unix: u64,
) -> Result<ShardProviderRecord> {
    let response = transport
        .request(
            client_peer,
            &provider.peer_id,
            NetworkRequest::PutShard {
                envelope: shard.to_network_envelope()?,
                lease_epoch,
                expires_at_unix,
            },
        )
        .map_err(|err| StorageError::Network(err.to_string()))?;
    let NetworkResponse::ShardStored { shard_ref } = response else {
        return Err(StorageError::Network(
            "unexpected response to PutShard during repair".to_string(),
        ));
    };
    ShardProviderRecord::new(
        shard_ref,
        provider.peer_id.clone(),
        provider.operator_id.clone(),
        provider.region.clone(),
        provider.roles.clone(),
        ProviderLease::new(lease_epoch, expires_at_unix)
            .map_err(|err| StorageError::Network(err.to_string()))?,
    )
    .map_err(|err| StorageError::Network(err.to_string()))
}

pub fn repair_shards_via_transport<T: NetworkTransport>(
    transport: &mut T,
    request: TransportRepairRequest<'_>,
) -> Result<TransportRepairOutcome> {
    let audit_before = audit_shards_via_transport(
        transport,
        request.client_peer,
        request.segment.cid,
        request.policy.total_shards(),
        request.policy.data_shards,
        request.providers,
        request.now_unix,
    )?;
    if audit_before.verified_shards.len() < request.policy.data_shards {
        return Err(StorageError::NotEnoughShards {
            available: audit_before.verified_shards.len(),
            required: request.policy.data_shards,
        });
    }
    if !audit_before.repair_needed {
        return Ok(TransportRepairOutcome {
            segment_cid: request.segment.cid,
            audit_after: audit_before.clone(),
            audit_before,
            repaired_shards: Vec::new(),
            providers_after_repair: request.providers.to_vec(),
            durability_satisfied: true,
        });
    }

    let verified_indexes = audit_before
        .verified_shards
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let verified_providers = request
        .providers
        .iter()
        .filter(|provider| verified_indexes.contains(&provider.shard_index))
        .cloned()
        .collect::<Vec<_>>();
    let stored = fetch_shards_via_transport(
        transport,
        request.client_peer,
        &verified_providers,
        request.now_unix,
    )?;
    let rebuilt_shards = reconstruct_all_shards(request.segment, request.policy, &stored)?;
    let provider_by_index = request
        .providers
        .iter()
        .map(|provider| (provider.shard_index, provider.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let repair_targets = audit_before
        .missing_shards
        .iter()
        .chain(&audit_before.corrupt_shards)
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let missing_targets = audit_before
        .missing_shards
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let mut providers_after_repair = request.providers.to_vec();
    let mut repaired_shards = Vec::new();

    for shard_index in repair_targets {
        let Some(target_provider) = provider_by_index.get(&shard_index) else {
            continue;
        };
        let shard = rebuilt_shards
            .get(shard_index)
            .ok_or(StorageError::InvalidShardIndex(shard_index))?;
        let preferred_provider = if missing_targets.contains(&shard_index) {
            choose_repair_target(
                &providers_after_repair,
                target_provider,
                request.replacement_descriptors,
            )
        } else {
            target_provider.clone()
        };
        let repaired_provider = match store_repaired_shard(
            transport,
            request.client_peer,
            &preferred_provider,
            shard,
            request.lease_epoch,
            request.expires_at_unix,
        ) {
            Ok(provider) => provider,
            Err(StorageError::Network(_))
                if preferred_provider.peer_id != target_provider.peer_id =>
            {
                store_repaired_shard(
                    transport,
                    request.client_peer,
                    target_provider,
                    shard,
                    request.lease_epoch,
                    request.expires_at_unix,
                )?
            }
            Err(err) if !request.replacement_descriptors.is_empty() => {
                let replacement_provider = choose_repair_target(
                    &providers_after_repair,
                    target_provider,
                    request.replacement_descriptors,
                );
                if replacement_provider.peer_id == target_provider.peer_id {
                    return Err(err);
                }
                store_repaired_shard(
                    transport,
                    request.client_peer,
                    &replacement_provider,
                    shard,
                    request.lease_epoch,
                    request.expires_at_unix,
                )?
            }
            Err(err) => return Err(err),
        };
        if let Some(existing) = providers_after_repair
            .iter_mut()
            .find(|provider| provider.shard_index == shard_index)
        {
            *existing = repaired_provider.clone();
        } else {
            providers_after_repair.push(repaired_provider.clone());
        }
        repaired_shards.push(shard_index);
    }

    for provider in &mut providers_after_repair {
        provider.lease_epoch = request.lease_epoch;
        provider.expires_at_unix = request.expires_at_unix;
    }
    if let Some(directory_peer) = request.directory_peer {
        publish_providers_via_transport(
            transport,
            request.client_peer,
            directory_peer,
            &providers_after_repair,
        )?;
    }

    let audit_after = audit_shards_via_transport(
        transport,
        request.client_peer,
        request.segment.cid,
        request.policy.total_shards(),
        request.policy.data_shards,
        &providers_after_repair,
        request.now_unix,
    )?;
    let durability_satisfied = audit_after.durability_satisfied;

    Ok(TransportRepairOutcome {
        segment_cid: request.segment.cid,
        audit_before,
        audit_after,
        repaired_shards,
        providers_after_repair,
        durability_satisfied,
    })
}

fn audit_challenge_nonce(segment_cid: Cid, shard_index: usize) -> [u8; 16] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"gitmesh.storage.audit-challenge-nonce.v0");
    hasher.update(segment_cid.to_string().as_bytes());
    hasher.update(&(shard_index as u64).to_be_bytes());
    let hash = hasher.finalize();
    let mut nonce = [0_u8; 16];
    nonce.copy_from_slice(&hash.as_bytes()[..16]);
    nonce
}

fn verify_shard(segment: &EncryptedSegment, policy: &StoragePolicy, shard: &Shard) -> bool {
    shard.segment_cid == segment.cid
        && shard.shard_count == policy.total_shards()
        && shard.data_shards == policy.data_shards
        && shard.shard_index < policy.total_shards()
        && shard.cid == shard_cid(segment.cid, shard.shard_index, &shard.bytes)
}

pub fn run_v0_local_storage_proof(
    plaintext: &[u8],
    policy: StoragePolicy,
    destroyed_nodes: Vec<usize>,
) -> Result<V0ProofResult> {
    let encrypted = encrypt_segment(plaintext)?;
    let shards = erasure_encode(&encrypted, &policy)?;
    let mut network = SimulatedNetwork::with_node_count(policy.total_shards());
    network.store_shards(shards)?;
    network.destroy_nodes(&destroyed_nodes)?;

    let available = network.available_shards();
    let ciphertext = reconstruct_ciphertext(&encrypted, &policy, &available)?;
    let recovered = decrypt_segment(&encrypted, &ciphertext)?;

    Ok(V0ProofResult {
        plaintext_len: plaintext.len(),
        ciphertext_len: encrypted.ciphertext_len,
        segment_cid: encrypted.cid,
        destroyed_nodes,
        available_shards: available.len(),
        recovered,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitmesh_network::{InMemoryPeer, InMemorySwarm, client_descriptor, storage_descriptor};

    fn directory_descriptor(peer: &str) -> NodeDescriptor {
        NodeDescriptor::new(
            PeerId::new(peer).unwrap(),
            OperatorId::new(format!("{peer}-operator")).unwrap(),
            [NodeRole::Dht],
            "iad",
            [
                gitmesh_network::ProtocolId::PingV0,
                gitmesh_network::ProtocolId::AvailabilityV0,
            ],
        )
        .unwrap()
    }

    #[test]
    fn reconstructs_after_losing_parity_count_nodes() {
        let plaintext = b"hello from GitMesh V0; this payload must round-trip exactly";
        let policy = StoragePolicy::default();
        let destroyed = vec![0, 3, 6, 9, 12, 15];

        let result = run_v0_local_storage_proof(plaintext, policy, destroyed).unwrap();

        assert_eq!(result.recovered, plaintext);
    }

    #[test]
    fn refuses_when_too_many_nodes_are_lost() {
        let plaintext = b"not enough shards should fail";
        let policy = StoragePolicy::default();
        let encrypted = encrypt_segment(plaintext).unwrap();
        let shards = erasure_encode(&encrypted, &policy).unwrap();
        let mut network = SimulatedNetwork::with_node_count(policy.total_shards());
        network.store_shards(shards).unwrap();
        network.destroy_nodes(&[0, 1, 2, 3, 4, 5, 6]).unwrap();

        let err =
            reconstruct_ciphertext(&encrypted, &policy, &network.available_shards()).unwrap_err();

        assert!(matches!(
            err,
            StorageError::NotEnoughShards {
                available: 9,
                required: 10
            }
        ));
    }

    #[test]
    fn audit_reports_missing_and_corrupt_shards() {
        let policy = StoragePolicy::default();
        let encrypted = encrypt_segment(b"audit me").unwrap();
        let shards = erasure_encode(&encrypted, &policy).unwrap();
        let mut network = SimulatedNetwork::with_node_count(policy.total_shards());
        network.store_shards(shards).unwrap();
        network.destroy_nodes(&[2, 4]).unwrap();
        network.corrupt_node_shard(6).unwrap();

        let report = audit_segment_shards(&encrypted, &policy, &network).unwrap();

        assert_eq!(report.segment_cid, encrypted.cid);
        assert_eq!(report.missing_shards, vec![2, 4]);
        assert_eq!(report.corrupt_shards, vec![6]);
        assert_eq!(report.verified_shards.len(), policy.total_shards() - 3);
        assert!(report.durability_satisfied);
        assert!(report.repair_needed);
    }

    #[test]
    fn repair_restores_missing_and_corrupt_shards() {
        let plaintext = b"repair can rebuild the full shard set";
        let policy = StoragePolicy::default();
        let encrypted = encrypt_segment(plaintext).unwrap();
        let shards = erasure_encode(&encrypted, &policy).unwrap();
        let mut network = SimulatedNetwork::with_node_count(policy.total_shards());
        network.store_shards(shards).unwrap();
        network.destroy_nodes(&[0, 3, 6]).unwrap();
        network.corrupt_node_shard(9).unwrap();

        let outcome = repair_segment_shards(&encrypted, &policy, &mut network).unwrap();
        let report = audit_segment_shards(&encrypted, &policy, &network).unwrap();
        let ciphertext =
            reconstruct_ciphertext(&encrypted, &policy, &network.available_shards()).unwrap();
        let recovered = decrypt_segment(&encrypted, &ciphertext).unwrap();

        assert_eq!(outcome.repaired_shards, vec![0, 3, 6, 9]);
        assert_eq!(outcome.verified_after_repair, policy.total_shards());
        assert!(outcome.durability_satisfied);
        assert!(!report.repair_needed);
        assert_eq!(recovered, plaintext);
    }

    #[test]
    fn repair_refuses_below_reconstruction_threshold() {
        let policy = StoragePolicy::default();
        let encrypted = encrypt_segment(b"too damaged").unwrap();
        let shards = erasure_encode(&encrypted, &policy).unwrap();
        let mut network = SimulatedNetwork::with_node_count(policy.total_shards());
        network.store_shards(shards).unwrap();
        network.destroy_nodes(&[0, 1, 2, 3, 4, 5, 6]).unwrap();

        let err = repair_segment_shards(&encrypted, &policy, &mut network).unwrap_err();

        assert!(matches!(
            err,
            StorageError::NotEnoughShards {
                available: 9,
                required: 10
            }
        ));
    }

    #[test]
    fn distributes_and_fetches_shards_through_transport_boundary() {
        let plaintext = b"network transport should carry erasure-coded ciphertext shards";
        let policy = StoragePolicy::default();
        let encrypted = encrypt_segment(plaintext).unwrap();
        let shards = erasure_encode(&encrypted, &policy).unwrap();
        let client = PeerId::new("client-a").unwrap();
        let mut swarm = InMemorySwarm::default();
        swarm
            .add_peer(InMemoryPeer::new(client_descriptor("client-a").unwrap()))
            .unwrap();
        let storage_peers = (0..policy.total_shards())
            .map(|index| {
                let peer_id = PeerId::new(format!("storage-{index}")).unwrap();
                swarm
                    .add_peer(InMemoryPeer::new(
                        storage_descriptor(peer_id.as_str(), &format!("operator-{index}"), "iad")
                            .unwrap(),
                    ))
                    .unwrap();
                peer_id
            })
            .collect::<Vec<_>>();

        let providers =
            distribute_shards_via_transport(&mut swarm, &client, &storage_peers, &shards, 1, 1_000)
                .unwrap();
        let fetched =
            fetch_shards_via_transport(&mut swarm, &client, &providers[..policy.data_shards], 100)
                .unwrap();
        let ciphertext = reconstruct_ciphertext(&encrypted, &policy, &fetched).unwrap();
        let recovered = decrypt_segment(&encrypted, &ciphertext).unwrap();

        assert_eq!(providers.len(), policy.total_shards());
        assert_eq!(fetched.len(), policy.data_shards);
        assert_eq!(recovered, plaintext);
    }

    #[test]
    fn plans_independent_placement_before_network_distribution() {
        let plaintext = b"placement should choose independent storage operators before transfer";
        let storage_policy = StoragePolicy {
            data_shards: 3,
            parity_shards: 2,
        };
        let placement_policy = PlacementPolicy::new(
            storage_policy.total_shards(),
            storage_policy.total_shards(),
            2,
            true,
        )
        .unwrap();
        let encrypted = encrypt_segment(plaintext).unwrap();
        let shards = erasure_encode(&encrypted, &storage_policy).unwrap();
        let client = PeerId::new("client-a").unwrap();
        let mut swarm = InMemorySwarm::default();
        swarm
            .add_peer(InMemoryPeer::new(client_descriptor("client-a").unwrap()))
            .unwrap();
        for index in 0..storage_policy.total_shards() {
            let peer = format!("storage-{index}");
            let operator = format!("operator-{index}");
            let region = if index % 2 == 0 { "iad" } else { "sfo" };
            swarm
                .add_peer(InMemoryPeer::new(
                    storage_descriptor(&peer, &operator, region).unwrap(),
                ))
                .unwrap();
        }

        let descriptors = swarm.descriptors();
        let (plan, providers) = plan_and_distribute_shards(
            &mut swarm,
            &client,
            descriptors,
            &placement_policy,
            &shards,
            1,
            1_000,
        )
        .unwrap();
        let fetched = fetch_shards_via_transport(
            &mut swarm,
            &client,
            &providers[..storage_policy.data_shards],
            100,
        )
        .unwrap();
        let ciphertext = reconstruct_ciphertext(&encrypted, &storage_policy, &fetched).unwrap();
        let recovered = decrypt_segment(&encrypted, &ciphertext).unwrap();

        assert_eq!(
            plan.distinct_operator_count(),
            storage_policy.total_shards()
        );
        assert_eq!(plan.distinct_region_count(), 2);
        assert_eq!(providers.len(), storage_policy.total_shards());
        assert_eq!(recovered, plaintext);
    }

    #[test]
    fn publishes_discovers_and_fetches_providers_through_availability_peer() {
        let plaintext = b"availability discovery should resolve shard providers before fetch";
        let storage_policy = StoragePolicy {
            data_shards: 3,
            parity_shards: 2,
        };
        let placement_policy =
            PlacementPolicy::new(storage_policy.total_shards(), 5, 2, true).unwrap();
        let encrypted = encrypt_segment(plaintext).unwrap();
        let shards = erasure_encode(&encrypted, &storage_policy).unwrap();
        let client = PeerId::new("client-a").unwrap();
        let directory = PeerId::new("directory-a").unwrap();
        let mut swarm = InMemorySwarm::default();
        swarm
            .add_peer(InMemoryPeer::new(client_descriptor("client-a").unwrap()))
            .unwrap();
        swarm
            .add_peer(InMemoryPeer::new(directory_descriptor("directory-a")))
            .unwrap();
        for index in 0..storage_policy.total_shards() {
            let peer = format!("storage-{index}");
            let operator = format!("operator-{index}");
            let region = if index % 2 == 0 { "iad" } else { "sfo" };
            swarm
                .add_peer(InMemoryPeer::new(
                    storage_descriptor(&peer, &operator, region).unwrap(),
                ))
                .unwrap();
        }

        let descriptors = swarm.descriptors();
        let (_plan, providers) = plan_and_distribute_shards(
            &mut swarm,
            &client,
            descriptors,
            &placement_policy,
            &shards,
            3,
            1_000,
        )
        .unwrap();
        let published =
            publish_providers_via_transport(&mut swarm, &client, &directory, &providers).unwrap();
        let discovered =
            discover_providers_via_transport(&mut swarm, &client, &directory, encrypted.cid, 100)
                .unwrap();
        let fetched = fetch_shards_via_transport(
            &mut swarm,
            &client,
            &discovered[..storage_policy.data_shards],
            100,
        )
        .unwrap();
        let ciphertext = reconstruct_ciphertext(&encrypted, &storage_policy, &fetched).unwrap();
        let recovered = decrypt_segment(&encrypted, &ciphertext).unwrap();

        assert_eq!(published, storage_policy.total_shards());
        assert_eq!(discovered.len(), storage_policy.total_shards());
        assert_eq!(recovered, plaintext);
    }

    #[test]
    fn publishes_signed_providers_through_availability_peer() {
        let plaintext = b"signed availability records should verify before discovery";
        let storage_policy = StoragePolicy {
            data_shards: 3,
            parity_shards: 2,
        };
        let placement_policy =
            PlacementPolicy::new(storage_policy.total_shards(), 5, 2, true).unwrap();
        let encrypted = encrypt_segment(plaintext).unwrap();
        let shards = erasure_encode(&encrypted, &storage_policy).unwrap();
        let client = PeerId::new("client-a").unwrap();
        let directory = PeerId::new("directory-a").unwrap();
        let mut swarm = InMemorySwarm::default();
        swarm
            .add_peer(InMemoryPeer::new(client_descriptor("client-a").unwrap()))
            .unwrap();
        swarm
            .add_peer(InMemoryPeer::new(directory_descriptor("directory-a")))
            .unwrap();
        for index in 0..storage_policy.total_shards() {
            let peer = format!("storage-{index}");
            let operator = format!("operator-{index}");
            let region = if index % 2 == 0 { "iad" } else { "sfo" };
            swarm
                .add_peer(InMemoryPeer::new(
                    storage_descriptor(&peer, &operator, region).unwrap(),
                ))
                .unwrap();
        }
        let account = gitmesh_identity::AccountRootKey::generate();
        let device = gitmesh_identity::DeviceKey::generate();
        let certificate = account.certify_device(&device, "storage-provider");

        let descriptors = swarm.descriptors();
        let (_plan, providers) = plan_and_distribute_shards(
            &mut swarm,
            &client,
            descriptors,
            &placement_policy,
            &shards,
            3,
            1_000,
        )
        .unwrap();
        let signed_providers = providers
            .iter()
            .cloned()
            .map(|provider| {
                SignedShardProviderRecord::sign(provider, certificate.clone(), &device).unwrap()
            })
            .collect::<Vec<_>>();
        let published = publish_signed_providers_via_transport(
            &mut swarm,
            &client,
            &directory,
            &signed_providers,
            100,
        )
        .unwrap();
        let discovered =
            discover_providers_via_transport(&mut swarm, &client, &directory, encrypted.cid, 100)
                .unwrap();

        assert_eq!(published, storage_policy.total_shards());
        assert_eq!(discovered, providers);
    }

    #[test]
    fn audits_transport_providers_for_valid_corrupt_and_missing_shards() {
        let storage_policy = StoragePolicy {
            data_shards: 3,
            parity_shards: 2,
        };
        let placement_policy =
            PlacementPolicy::new(storage_policy.total_shards(), 5, 2, true).unwrap();
        let encrypted = encrypt_segment(b"remote audit catches repair targets").unwrap();
        let shards = erasure_encode(&encrypted, &storage_policy).unwrap();
        let client = PeerId::new("client-a").unwrap();
        let mut swarm = InMemorySwarm::default();
        swarm
            .add_peer(InMemoryPeer::new(client_descriptor("client-a").unwrap()))
            .unwrap();
        for index in 0..storage_policy.total_shards() {
            let peer = format!("storage-{index}");
            let operator = format!("operator-{index}");
            let region = if index % 2 == 0 { "iad" } else { "sfo" };
            swarm
                .add_peer(InMemoryPeer::new(
                    storage_descriptor(&peer, &operator, region).unwrap(),
                ))
                .unwrap();
        }

        let descriptors = swarm.descriptors();
        let (_plan, providers) = plan_and_distribute_shards(
            &mut swarm,
            &client,
            descriptors,
            &placement_policy,
            &shards,
            1,
            1_000,
        )
        .unwrap();
        swarm
            .corrupt_shard(&providers[1].peer_id, providers[1].shard_cid)
            .unwrap();
        swarm
            .remove_shard(&providers[3].peer_id, providers[3].shard_cid)
            .unwrap();

        let report = audit_shards_via_transport(
            &mut swarm,
            &client,
            encrypted.cid,
            storage_policy.total_shards(),
            storage_policy.data_shards,
            &providers,
            100,
        )
        .unwrap();

        assert_eq!(report.checked_providers, storage_policy.total_shards());
        assert_eq!(report.verified_shards, vec![0, 2, 4]);
        assert_eq!(report.corrupt_shards, vec![1]);
        assert_eq!(report.missing_shards, vec![3]);
        assert!(report.durability_satisfied);
        assert!(report.repair_needed);
    }

    #[test]
    fn challenge_audit_detects_valid_corrupt_and_missing_provider_shards() {
        let storage_policy = StoragePolicy {
            data_shards: 3,
            parity_shards: 2,
        };
        let placement_policy =
            PlacementPolicy::new(storage_policy.total_shards(), 5, 2, true).unwrap();
        let encrypted =
            encrypt_segment(b"challenge audit samples shard bytes without fetching all").unwrap();
        let shards = erasure_encode(&encrypted, &storage_policy).unwrap();
        let client = PeerId::new("client-a").unwrap();
        let mut swarm = InMemorySwarm::default();
        swarm
            .add_peer(InMemoryPeer::new(client_descriptor("client-a").unwrap()))
            .unwrap();
        for index in 0..storage_policy.total_shards() {
            let peer = format!("storage-{index}");
            let operator = format!("operator-{index}");
            let region = if index % 2 == 0 { "iad" } else { "sfo" };
            swarm
                .add_peer(InMemoryPeer::new(
                    storage_descriptor(&peer, &operator, region).unwrap(),
                ))
                .unwrap();
        }

        let descriptors = swarm.descriptors();
        let (_plan, providers) = plan_and_distribute_shards(
            &mut swarm,
            &client,
            descriptors,
            &placement_policy,
            &shards,
            1,
            1_000,
        )
        .unwrap();
        swarm
            .corrupt_shard(&providers[1].peer_id, providers[1].shard_cid)
            .unwrap();
        swarm
            .remove_shard(&providers[3].peer_id, providers[3].shard_cid)
            .unwrap();

        let report =
            challenge_shards_via_transport(&mut swarm, &client, encrypted.cid, &providers, 100, 1)
                .unwrap();

        assert_eq!(report.checked_providers, storage_policy.total_shards());
        assert_eq!(report.verified_shards, vec![0, 2, 4]);
        assert_eq!(report.failed_shards, vec![1, 3]);
    }

    #[test]
    fn repairs_transport_providers_and_republishes_refreshed_leases() {
        let plaintext = b"transport repair should rebuild missing and corrupt shards";
        let storage_policy = StoragePolicy {
            data_shards: 3,
            parity_shards: 2,
        };
        let placement_policy =
            PlacementPolicy::new(storage_policy.total_shards(), 5, 2, true).unwrap();
        let encrypted = encrypt_segment(plaintext).unwrap();
        let shards = erasure_encode(&encrypted, &storage_policy).unwrap();
        let client = PeerId::new("client-a").unwrap();
        let directory = PeerId::new("directory-a").unwrap();
        let mut swarm = InMemorySwarm::default();
        swarm
            .add_peer(InMemoryPeer::new(client_descriptor("client-a").unwrap()))
            .unwrap();
        swarm
            .add_peer(InMemoryPeer::new(directory_descriptor("directory-a")))
            .unwrap();
        for index in 0..storage_policy.total_shards() {
            let peer = format!("storage-{index}");
            let operator = format!("operator-{index}");
            let region = if index % 2 == 0 { "iad" } else { "sfo" };
            swarm
                .add_peer(InMemoryPeer::new(
                    storage_descriptor(&peer, &operator, region).unwrap(),
                ))
                .unwrap();
        }

        let descriptors = swarm.descriptors();
        let (_plan, providers) = plan_and_distribute_shards(
            &mut swarm,
            &client,
            descriptors,
            &placement_policy,
            &shards,
            1,
            1_000,
        )
        .unwrap();
        publish_providers_via_transport(&mut swarm, &client, &directory, &providers).unwrap();
        swarm
            .corrupt_shard(&providers[1].peer_id, providers[1].shard_cid)
            .unwrap();
        swarm
            .remove_shard(&providers[3].peer_id, providers[3].shard_cid)
            .unwrap();

        let outcome = repair_shards_via_transport(
            &mut swarm,
            TransportRepairRequest {
                client_peer: &client,
                directory_peer: Some(&directory),
                segment: &encrypted,
                policy: &storage_policy,
                providers: &providers,
                replacement_descriptors: &[],
                now_unix: 100,
                lease_epoch: 2,
                expires_at_unix: 2_000,
            },
        )
        .unwrap();
        let discovered =
            discover_providers_via_transport(&mut swarm, &client, &directory, encrypted.cid, 1_500)
                .unwrap();
        let fetched = fetch_shards_via_transport(
            &mut swarm,
            &client,
            &discovered[..storage_policy.data_shards],
            1_500,
        )
        .unwrap();
        let ciphertext = reconstruct_ciphertext(&encrypted, &storage_policy, &fetched).unwrap();
        let recovered = decrypt_segment(&encrypted, &ciphertext).unwrap();

        assert_eq!(outcome.repaired_shards, vec![1, 3]);
        assert_eq!(outcome.audit_before.corrupt_shards, vec![1]);
        assert_eq!(outcome.audit_before.missing_shards, vec![3]);
        assert_eq!(
            outcome.audit_after.verified_shards.len(),
            storage_policy.total_shards()
        );
        assert!(!outcome.audit_after.repair_needed);
        assert!(outcome.durability_satisfied);
        assert_eq!(recovered, plaintext);
    }

    #[test]
    fn transport_repair_uses_replacement_peer_when_provider_disappears() {
        let plaintext = b"transport repair should move a shard when its provider disappears";
        let storage_policy = StoragePolicy {
            data_shards: 3,
            parity_shards: 2,
        };
        let placement_policy =
            PlacementPolicy::new(storage_policy.total_shards(), 5, 2, true).unwrap();
        let encrypted = encrypt_segment(plaintext).unwrap();
        let shards = erasure_encode(&encrypted, &storage_policy).unwrap();
        let client = PeerId::new("client-a").unwrap();
        let directory = PeerId::new("directory-a").unwrap();
        let mut swarm = InMemorySwarm::default();
        swarm
            .add_peer(InMemoryPeer::new(client_descriptor("client-a").unwrap()))
            .unwrap();
        swarm
            .add_peer(InMemoryPeer::new(directory_descriptor("directory-a")))
            .unwrap();
        for index in 0..=storage_policy.total_shards() {
            let peer = format!("storage-{index}");
            let operator = format!("operator-{index}");
            let region = if index % 2 == 0 { "iad" } else { "sfo" };
            swarm
                .add_peer(InMemoryPeer::new(
                    storage_descriptor(&peer, &operator, region).unwrap(),
                ))
                .unwrap();
        }

        let descriptors = swarm.descriptors();
        let (_plan, providers) = plan_and_distribute_shards(
            &mut swarm,
            &client,
            descriptors.clone(),
            &placement_policy,
            &shards,
            1,
            1_000,
        )
        .unwrap();
        publish_providers_via_transport(&mut swarm, &client, &directory, &providers).unwrap();
        let vanished_provider = providers[3].clone();
        swarm.remove_peer(&vanished_provider.peer_id).unwrap();

        let outcome = repair_shards_via_transport(
            &mut swarm,
            TransportRepairRequest {
                client_peer: &client,
                directory_peer: Some(&directory),
                segment: &encrypted,
                policy: &storage_policy,
                providers: &providers,
                replacement_descriptors: &descriptors,
                now_unix: 100,
                lease_epoch: 2,
                expires_at_unix: 2_000,
            },
        )
        .unwrap();
        let replacement_provider = outcome
            .providers_after_repair
            .iter()
            .find(|provider| provider.shard_index == vanished_provider.shard_index)
            .unwrap();
        let discovered =
            discover_providers_via_transport(&mut swarm, &client, &directory, encrypted.cid, 1_500)
                .unwrap();
        let fetched = fetch_shards_via_transport(
            &mut swarm,
            &client,
            &discovered[..storage_policy.data_shards],
            1_500,
        )
        .unwrap();
        let ciphertext = reconstruct_ciphertext(&encrypted, &storage_policy, &fetched).unwrap();
        let recovered = decrypt_segment(&encrypted, &ciphertext).unwrap();

        assert_eq!(outcome.repaired_shards, vec![3]);
        assert_eq!(outcome.audit_before.missing_shards, vec![3]);
        assert_ne!(replacement_provider.peer_id, vanished_provider.peer_id);
        assert_eq!(
            replacement_provider.peer_id,
            PeerId::new("storage-5").unwrap()
        );
        assert_eq!(recovered, plaintext);
    }

    #[test]
    fn transport_repair_refuses_below_reconstruction_threshold() {
        let storage_policy = StoragePolicy {
            data_shards: 3,
            parity_shards: 2,
        };
        let placement_policy =
            PlacementPolicy::new(storage_policy.total_shards(), 5, 2, true).unwrap();
        let encrypted = encrypt_segment(b"too many remote shards are gone").unwrap();
        let shards = erasure_encode(&encrypted, &storage_policy).unwrap();
        let client = PeerId::new("client-a").unwrap();
        let mut swarm = InMemorySwarm::default();
        swarm
            .add_peer(InMemoryPeer::new(client_descriptor("client-a").unwrap()))
            .unwrap();
        for index in 0..storage_policy.total_shards() {
            let peer = format!("storage-{index}");
            let operator = format!("operator-{index}");
            let region = if index % 2 == 0 { "iad" } else { "sfo" };
            swarm
                .add_peer(InMemoryPeer::new(
                    storage_descriptor(&peer, &operator, region).unwrap(),
                ))
                .unwrap();
        }

        let descriptors = swarm.descriptors();
        let (_plan, providers) = plan_and_distribute_shards(
            &mut swarm,
            &client,
            descriptors,
            &placement_policy,
            &shards,
            1,
            1_000,
        )
        .unwrap();
        for provider in providers.iter().take(3) {
            swarm
                .remove_shard(&provider.peer_id, provider.shard_cid)
                .unwrap();
        }

        let err = repair_shards_via_transport(
            &mut swarm,
            TransportRepairRequest {
                client_peer: &client,
                directory_peer: None,
                segment: &encrypted,
                policy: &storage_policy,
                providers: &providers,
                replacement_descriptors: &[],
                now_unix: 100,
                lease_epoch: 2,
                expires_at_unix: 2_000,
            },
        )
        .unwrap_err();

        assert!(matches!(
            err,
            StorageError::NotEnoughShards {
                available: 2,
                required: 3
            }
        ));
    }
}
