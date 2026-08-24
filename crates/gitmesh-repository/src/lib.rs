//! Repository object-store spine for GitMesh.
//!
//! This crate is intentionally small but real: it maps canonical Git objects
//! into GitMesh encrypted segment storage and can reconstruct those objects
//! through verified shards. Later networking, leases, and persistence should
//! replace the in-memory backing without changing the object record semantics.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::str::FromStr;

use gitmesh_core::{Cid, encrypted_segment_cid, hex, shard_cid};
use gitmesh_crypto::AeadAlgorithm;
use gitmesh_git::{
    GitError, GitObject, GitObjectKind, GitSha1Oid, GitTreeEntryTarget, parse_canonical_object,
    parse_commit_links, parse_tree_entries, write_packfile,
};
use gitmesh_network::{
    AvailabilityReport, AvailabilityRequirement, InMemoryAvailabilityDirectory, InMemoryPeer,
    InMemorySwarm, NodeRole, OperatorId, PeerId, ProviderLease, ShardProviderRecord, ShardRef,
    client_descriptor, storage_descriptor,
};
use gitmesh_storage::{
    EncryptedSegment, RepairOutcome, Shard, ShardAuditReport, SimulatedNetwork, StoragePolicy,
    StoredShard, TransportRepairRequest, audit_segment_shards, decrypt_segment,
    discover_providers_via_transport, encrypt_segment, erasure_encode, fetch_shards_via_transport,
    plan_and_distribute_shards, publish_providers_via_transport, reconstruct_ciphertext,
    repair_segment_shards, repair_shards_via_transport,
};
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitObjectRecord {
    pub oid: GitSha1Oid,
    pub kind: GitObjectKind,
    pub canonical_len: usize,
    pub segment_cid: Cid,
    pub shard_cids: Vec<Cid>,
    pub available_shards: usize,
    pub durability_satisfied: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryObjectAudit {
    pub oid: GitSha1Oid,
    pub kind: GitObjectKind,
    pub report: ShardAuditReport,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryRepairReport {
    pub oid: GitSha1Oid,
    pub kind: GitObjectKind,
    pub outcome: RepairOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryTransportRepairProof {
    pub oid: GitSha1Oid,
    pub recovered_exactly: bool,
    pub repaired_shards: Vec<usize>,
    pub original_peer: PeerId,
    pub replacement_peer: PeerId,
    pub provider_count: usize,
    pub verified_after_repair: usize,
    pub durability_satisfied: bool,
}

#[derive(Clone, Debug)]
struct StoredObject {
    record: GitObjectRecord,
    segment: EncryptedSegment,
    shards: Vec<Shard>,
}

#[derive(Clone, Debug)]
pub struct RepositoryObjectStore {
    policy: StoragePolicy,
    objects: BTreeMap<GitSha1Oid, StoredObject>,
}

impl RepositoryObjectStore {
    pub fn new(policy: StoragePolicy) -> Self {
        Self {
            policy,
            objects: BTreeMap::new(),
        }
    }

    pub fn policy(&self) -> &StoragePolicy {
        &self.policy
    }

    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    pub fn put_git_object(&mut self, object: GitObject) -> Result<GitObjectRecord> {
        let canonical = object.canonical_bytes();
        parse_canonical_object(&canonical)?;
        let oid = object.sha1_oid();
        if let Some(stored) = self.objects.get(&oid) {
            return Ok(stored.record.clone());
        }

        let segment = encrypt_segment(&canonical)?;
        let shards = erasure_encode(&segment, &self.policy)?;
        let record = GitObjectRecord {
            oid,
            kind: object.kind,
            canonical_len: canonical.len(),
            segment_cid: segment.cid,
            shard_cids: shards.iter().map(|shard| shard.cid).collect(),
            available_shards: shards.len(),
            durability_satisfied: shards.len() >= self.policy.data_shards,
        };
        self.objects.insert(
            oid,
            StoredObject {
                record: record.clone(),
                segment,
                shards,
            },
        );
        Ok(record)
    }

    pub fn get_git_object(&self, oid: GitSha1Oid) -> Result<GitObject> {
        let stored = self
            .objects
            .get(&oid)
            .ok_or(RepositoryError::MissingObject(oid))?;
        let network = self.network_for(stored, &[])?;
        let canonical = self.reconstruct_canonical(stored, &network.available_shards())?;
        Ok(parse_canonical_object(&canonical)?)
    }

    pub fn get_record(&self, oid: GitSha1Oid) -> Option<&GitObjectRecord> {
        self.objects.get(&oid).map(|stored| &stored.record)
    }

    pub fn has_durable_object(&self, oid: GitSha1Oid) -> bool {
        self.get_record(oid)
            .is_some_and(|record| record.durability_satisfied)
    }

    pub fn object_availability_report(
        &self,
        oid: GitSha1Oid,
        providers: &[ShardProviderRecord],
        requirement: AvailabilityRequirement,
        now_unix: u64,
    ) -> Result<AvailabilityReport> {
        let stored = self
            .objects
            .get(&oid)
            .ok_or(RepositoryError::MissingObject(oid))?;
        let known_shards = stored
            .record
            .shard_cids
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let mut directory = InMemoryAvailabilityDirectory::default();
        for provider in providers {
            if provider.segment_cid != stored.record.segment_cid
                || !known_shards.contains(&provider.shard_cid)
                || provider.shard_index >= self.policy.total_shards()
            {
                return Err(RepositoryError::InvalidAvailabilityEvidence);
            }
            directory
                .publish(provider.clone())
                .map_err(|err| RepositoryError::Network(err.to_string()))?;
        }
        Ok(directory.availability_report(stored.record.segment_cid, now_unix, requirement))
    }

    pub fn local_provider_records_for_object(
        &self,
        oid: GitSha1Oid,
        lease_epoch: u64,
        expires_at_unix: u64,
    ) -> Result<Vec<ShardProviderRecord>> {
        let stored = self
            .objects
            .get(&oid)
            .ok_or(RepositoryError::MissingObject(oid))?;
        stored
            .shards
            .iter()
            .map(|shard| {
                let region = if shard.shard_index % 2 == 0 {
                    "local-a"
                } else {
                    "local-b"
                };
                ShardProviderRecord::new(
                    ShardRef {
                        segment_cid: shard.segment_cid,
                        shard_cid: shard.cid,
                        shard_index: shard.shard_index,
                    },
                    PeerId::new(format!("v0-node-{}", shard.shard_index))
                        .map_err(|err| RepositoryError::Network(err.to_string()))?,
                    OperatorId::new(format!("v0-operator-{}", shard.shard_index))
                        .map_err(|err| RepositoryError::Network(err.to_string()))?,
                    region,
                    [NodeRole::Storage],
                    ProviderLease::new(lease_epoch, expires_at_unix)
                        .map_err(|err| RepositoryError::Network(err.to_string()))?,
                )
                .map_err(|err| RepositoryError::Network(err.to_string()))
            })
            .collect()
    }

    pub fn has_qualified_durable_object(
        &self,
        oid: GitSha1Oid,
        providers: &[ShardProviderRecord],
        requirement: AvailabilityRequirement,
        now_unix: u64,
    ) -> Result<bool> {
        Ok(self
            .object_availability_report(oid, providers, requirement, now_unix)?
            .satisfies_requirement())
    }

    pub fn validate_ref_update(
        &self,
        ref_name: &str,
        expected_old_oid: Option<GitSha1Oid>,
        new_oid: Option<GitSha1Oid>,
        force: bool,
    ) -> Result<()> {
        let Some(new_oid) = new_oid else {
            return Ok(());
        };
        self.validate_ref_target(ref_name, new_oid)?;
        if ref_name.starts_with("refs/heads/")
            && !force
            && let Some(old_oid) = expected_old_oid
            && !self.is_commit_ancestor(old_oid, new_oid)?
        {
            return Err(RepositoryError::NonFastForward { old_oid, new_oid });
        }
        Ok(())
    }

    pub fn validate_ref_target(&self, ref_name: &str, oid: GitSha1Oid) -> Result<()> {
        let record = self
            .get_record(oid)
            .ok_or(RepositoryError::MissingObject(oid))?;
        if !record.durability_satisfied {
            return Err(RepositoryError::ObjectNotDurable(oid));
        }
        if ref_name.starts_with("refs/heads/") {
            self.validate_commit_graph(oid)?;
        }
        Ok(())
    }

    pub fn records(&self) -> impl Iterator<Item = &GitObjectRecord> {
        self.objects.values().map(|stored| &stored.record)
    }

    pub fn audit_object(&self, oid: GitSha1Oid) -> Result<RepositoryObjectAudit> {
        let stored = self
            .objects
            .get(&oid)
            .ok_or(RepositoryError::MissingObject(oid))?;
        let network = self.network_for(stored, &[])?;
        Ok(RepositoryObjectAudit {
            oid,
            kind: stored.record.kind,
            report: audit_segment_shards(&stored.segment, &self.policy, &network)?,
        })
    }

    pub fn audit_all(&self) -> Result<Vec<RepositoryObjectAudit>> {
        self.objects
            .keys()
            .copied()
            .map(|oid| self.audit_object(oid))
            .collect()
    }

    pub fn repair_object(&mut self, oid: GitSha1Oid) -> Result<RepositoryRepairReport> {
        let stored = self
            .objects
            .get_mut(&oid)
            .ok_or(RepositoryError::MissingObject(oid))?;
        let mut network = SimulatedNetwork::with_node_count(self.policy.total_shards());
        network.store_shards(stored.shards.clone())?;
        let outcome = repair_segment_shards(&stored.segment, &self.policy, &mut network)?;
        stored.shards = network
            .available_shards()
            .into_iter()
            .map(|stored| stored.shard)
            .collect();
        stored.shards.sort_by_key(|shard| shard.shard_index);
        stored.record.shard_cids = stored.shards.iter().map(|shard| shard.cid).collect();
        stored.record.available_shards = stored.shards.len();
        stored.record.durability_satisfied = stored.shards.len() >= self.policy.data_shards;
        Ok(RepositoryRepairReport {
            oid,
            kind: stored.record.kind,
            outcome,
        })
    }

    pub fn repair_all(&mut self) -> Result<Vec<RepositoryRepairReport>> {
        let oids = self.objects.keys().copied().collect::<Vec<_>>();
        oids.into_iter()
            .map(|oid| self.repair_object(oid))
            .collect()
    }

    pub fn simulate_object_shard_loss(
        &mut self,
        oid: GitSha1Oid,
        shard_indexes: &[usize],
    ) -> Result<()> {
        let stored = self
            .objects
            .get_mut(&oid)
            .ok_or(RepositoryError::MissingObject(oid))?;
        let lost = shard_indexes.iter().copied().collect::<BTreeSet<_>>();
        stored
            .shards
            .retain(|shard| !lost.contains(&shard.shard_index));
        stored.record.shard_cids = stored.shards.iter().map(|shard| shard.cid).collect();
        stored.record.available_shards = stored.shards.len();
        stored.record.durability_satisfied = stored.shards.len() >= self.policy.data_shards;
        Ok(())
    }

    pub fn export_pack_all(&self) -> Result<Vec<u8>> {
        let objects = self
            .objects
            .keys()
            .copied()
            .map(|oid| self.get_git_object(oid))
            .collect::<Result<Vec<_>>>()?;
        Ok(write_packfile(&objects)?)
    }

    pub fn export_pack_reachable_from(&self, tips: &[GitSha1Oid]) -> Result<Vec<u8>> {
        let mut reachable = BTreeSet::new();
        for tip in tips {
            self.collect_reachable_object_graph(*tip, &mut reachable)?;
        }
        let objects = reachable
            .into_iter()
            .map(|oid| self.get_git_object(oid))
            .collect::<Result<Vec<_>>>()?;
        Ok(write_packfile(&objects)?)
    }

    pub fn save_to_path(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let mut snapshot = String::new();
        snapshot.push_str("gitmesh-repository-store-v0\n");
        snapshot.push_str(&format!(
            "policy\t{}\t{}\t{}\t{}\n",
            self.policy.data_shards,
            self.policy.parity_shards,
            self.policy.min_distinct_operators,
            self.policy.min_distinct_regions
        ));
        for stored in self.objects.values() {
            snapshot.push_str(&format!(
                "object\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                stored.record.oid,
                encode_git_object_kind(stored.record.kind),
                stored.segment.plaintext_len,
                stored.segment.ciphertext_len,
                encode_aead_algorithm(stored.segment.algorithm),
                encode_hex(&stored.segment.nonce),
                encode_hex(&stored.segment.key),
                encode_hex(&stored.segment.ciphertext)
            ));
            for shard in &stored.shards {
                snapshot.push_str(&format!(
                    "shard\t{}\t{}\t{}\t{}\t{}\n",
                    stored.record.oid,
                    shard.shard_index,
                    shard.shard_count,
                    shard.data_shards,
                    encode_hex(&shard.bytes)
                ));
            }
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp_path = path.with_extension("tmp");
        fs::write(&tmp_path, snapshot)?;
        fs::rename(tmp_path, path)?;
        Ok(())
    }

    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(path)?;
        Self::from_snapshot(&text)
    }

    pub fn from_snapshot(text: &str) -> Result<Self> {
        let mut lines = text.lines();
        if lines.next() != Some("gitmesh-repository-store-v0") {
            return Err(RepositoryError::InvalidStore("missing store header"));
        }
        let policy_line = lines
            .next()
            .ok_or(RepositoryError::InvalidStore("missing storage policy"))?;
        let policy_parts = policy_line.split('\t').collect::<Vec<_>>();
        if !matches!(policy_parts.len(), 3 | 5) || policy_parts[0] != "policy" {
            return Err(RepositoryError::InvalidStore("invalid storage policy"));
        }
        let policy = StoragePolicy {
            data_shards: parse_usize(policy_parts[1])?,
            parity_shards: parse_usize(policy_parts[2])?,
            min_distinct_operators: policy_parts
                .get(3)
                .map(|value| parse_usize(value))
                .transpose()?
                .unwrap_or(3),
            min_distinct_regions: policy_parts
                .get(4)
                .map(|value| parse_usize(value))
                .transpose()?
                .unwrap_or(2),
        };
        let mut builders = BTreeMap::<GitSha1Oid, StoredObjectBuilder>::new();

        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            let parts = line.split('\t').collect::<Vec<_>>();
            match parts.first().copied() {
                Some("object") => {
                    if parts.len() != 9 {
                        return Err(RepositoryError::InvalidStore("invalid object line"));
                    }
                    let oid = parse_oid(parts[1])?;
                    let kind = parse_git_object_kind(parts[2])?;
                    let plaintext_len = parse_usize(parts[3])?;
                    let ciphertext_len = parse_usize(parts[4])?;
                    let algorithm = parse_aead_algorithm(parts[5])?;
                    let nonce = fixed_bytes::<24>(&decode_hex(parts[6])?)?;
                    let key = fixed_bytes::<32>(&decode_hex(parts[7])?)?;
                    let ciphertext = decode_hex(parts[8])?;
                    if ciphertext.len() != ciphertext_len {
                        return Err(RepositoryError::InvalidStore("ciphertext length mismatch"));
                    }
                    let segment = EncryptedSegment {
                        cid: encrypted_segment_cid(&ciphertext),
                        algorithm,
                        plaintext_len,
                        ciphertext_len,
                        nonce,
                        key,
                        ciphertext,
                    };
                    builders.insert(
                        oid,
                        StoredObjectBuilder {
                            oid,
                            kind,
                            segment,
                            shards: Vec::new(),
                        },
                    );
                }
                Some("shard") => {
                    if parts.len() != 6 {
                        return Err(RepositoryError::InvalidStore("invalid shard line"));
                    }
                    let oid = parse_oid(parts[1])?;
                    let shard_index = parse_usize(parts[2])?;
                    let shard_count = parse_usize(parts[3])?;
                    let data_shards = parse_usize(parts[4])?;
                    let bytes = decode_hex(parts[5])?;
                    let builder = builders
                        .get_mut(&oid)
                        .ok_or(RepositoryError::InvalidStore("shard before object"))?;
                    let cid = shard_cid(builder.segment.cid, shard_index, &bytes);
                    builder.shards.push(Shard {
                        segment_cid: builder.segment.cid,
                        shard_index,
                        shard_count,
                        data_shards,
                        bytes,
                        cid,
                    });
                }
                _ => return Err(RepositoryError::InvalidStore("unknown snapshot line")),
            }
        }

        let mut store = Self::new(policy.clone());
        for builder in builders.into_values() {
            let canonical = decrypt_segment(&builder.segment, &builder.segment.ciphertext)?;
            let object = parse_canonical_object(&canonical)?;
            if object.kind != builder.kind || object.sha1_oid() != builder.oid {
                return Err(RepositoryError::InvalidStore(
                    "stored Git object identity mismatch",
                ));
            }
            let record = GitObjectRecord {
                oid: builder.oid,
                kind: builder.kind,
                canonical_len: canonical.len(),
                segment_cid: builder.segment.cid,
                shard_cids: builder.shards.iter().map(|shard| shard.cid).collect(),
                available_shards: builder.shards.len(),
                durability_satisfied: builder.shards.len() >= policy.data_shards,
            };
            store.objects.insert(
                builder.oid,
                StoredObject {
                    record,
                    segment: builder.segment,
                    shards: builder.shards,
                },
            );
        }
        Ok(store)
    }

    pub fn read_after_losing_nodes(
        &self,
        oid: GitSha1Oid,
        destroyed_nodes: &[usize],
    ) -> Result<GitObject> {
        let stored = self
            .objects
            .get(&oid)
            .ok_or(RepositoryError::MissingObject(oid))?;
        let network = self.network_for(stored, destroyed_nodes)?;
        let canonical = self.reconstruct_canonical(stored, &network.available_shards())?;
        Ok(parse_canonical_object(&canonical)?)
    }

    fn network_for(
        &self,
        stored: &StoredObject,
        destroyed_nodes: &[usize],
    ) -> Result<SimulatedNetwork> {
        let mut network = SimulatedNetwork::with_node_count(self.policy.total_shards());
        network.store_shards(stored.shards.clone())?;
        network.destroy_nodes(destroyed_nodes)?;
        Ok(network)
    }

    fn reconstruct_canonical(
        &self,
        stored: &StoredObject,
        available: &[StoredShard],
    ) -> Result<Vec<u8>> {
        let ciphertext = reconstruct_ciphertext(&stored.segment, &self.policy, available)?;
        let canonical = decrypt_segment(&stored.segment, &ciphertext)?;
        if canonical.len() != stored.record.canonical_len {
            return Err(RepositoryError::CanonicalLengthMismatch {
                expected: stored.record.canonical_len,
                actual: canonical.len(),
            });
        }
        Ok(canonical)
    }

    fn validate_commit_graph(&self, oid: GitSha1Oid) -> Result<()> {
        let object = self.get_git_object(oid)?;
        if object.kind != GitObjectKind::Commit {
            return Err(RepositoryError::InvalidRefTarget {
                oid,
                expected: GitObjectKind::Commit,
                actual: object.kind,
            });
        }
        let links = parse_commit_links(&object.payload)?;
        self.validate_tree_graph(links.tree, &mut BTreeSet::new())?;
        for parent in links.parents {
            self.require_kind(parent, GitObjectKind::Commit)?;
        }
        Ok(())
    }

    fn collect_reachable_object_graph(
        &self,
        oid: GitSha1Oid,
        seen: &mut BTreeSet<GitSha1Oid>,
    ) -> Result<()> {
        if !seen.insert(oid) {
            return Ok(());
        }
        let object = self.get_git_object(oid)?;
        match object.kind {
            GitObjectKind::Commit => {
                let links = parse_commit_links(&object.payload)?;
                self.collect_reachable_object_graph(links.tree, seen)?;
                for parent in links.parents {
                    self.collect_reachable_object_graph(parent, seen)?;
                }
            }
            GitObjectKind::Tree => {
                for entry in parse_tree_entries(&object.payload)? {
                    self.collect_reachable_object_graph(entry.oid, seen)?;
                }
            }
            GitObjectKind::Blob | GitObjectKind::Tag => {}
        }
        Ok(())
    }

    fn validate_tree_graph(
        &self,
        tree_oid: GitSha1Oid,
        seen: &mut BTreeSet<GitSha1Oid>,
    ) -> Result<()> {
        if !seen.insert(tree_oid) {
            return Ok(());
        }
        self.require_kind(tree_oid, GitObjectKind::Tree)?;
        let tree = self.get_git_object(tree_oid)?;
        let entries = parse_tree_entries(&tree.payload)?;
        for entry in entries {
            match entry.target {
                GitTreeEntryTarget::Blob => self.require_kind(entry.oid, GitObjectKind::Blob)?,
                GitTreeEntryTarget::Tree => self.validate_tree_graph(entry.oid, seen)?,
                GitTreeEntryTarget::Commit => {
                    self.require_kind(entry.oid, GitObjectKind::Commit)?;
                }
            }
        }
        Ok(())
    }

    fn require_kind(&self, oid: GitSha1Oid, expected: GitObjectKind) -> Result<()> {
        let record = self
            .get_record(oid)
            .ok_or(RepositoryError::MissingObject(oid))?;
        if !record.durability_satisfied {
            return Err(RepositoryError::ObjectNotDurable(oid));
        }
        if record.kind != expected {
            return Err(RepositoryError::InvalidRefTarget {
                oid,
                expected,
                actual: record.kind,
            });
        }
        Ok(())
    }

    fn is_commit_ancestor(&self, ancestor: GitSha1Oid, descendant: GitSha1Oid) -> Result<bool> {
        if ancestor == descendant {
            return Ok(true);
        }
        self.require_kind(ancestor, GitObjectKind::Commit)?;
        self.require_kind(descendant, GitObjectKind::Commit)?;

        let mut stack = vec![descendant];
        let mut seen = BTreeSet::new();
        while let Some(oid) = stack.pop() {
            if !seen.insert(oid) {
                continue;
            }
            if oid == ancestor {
                return Ok(true);
            }
            let object = self.get_git_object(oid)?;
            let links = parse_commit_links(&object.payload)?;
            stack.extend(links.parents);
        }
        Ok(false)
    }
}

impl Default for RepositoryObjectStore {
    fn default() -> Self {
        Self::new(StoragePolicy::default())
    }
}

pub fn run_repository_transport_repair_proof(
    payload: &[u8],
) -> Result<RepositoryTransportRepairProof> {
    let policy = StoragePolicy {
        data_shards: 3,
        parity_shards: 2,
        min_distinct_operators: 5,
        min_distinct_regions: 2,
    };
    let placement_policy = policy
        .placement_policy()
        .map_err(|err| RepositoryError::Network(err.to_string()))?;
    let object = GitObject::new(GitObjectKind::Blob, payload);
    let oid = object.sha1_oid();
    let mut store = RepositoryObjectStore::new(policy.clone());
    store.put_git_object(object.clone())?;
    let stored = store
        .objects
        .get(&oid)
        .ok_or(RepositoryError::MissingObject(oid))?
        .clone();

    let client =
        PeerId::new("repo-client").map_err(|err| RepositoryError::Network(err.to_string()))?;
    let directory =
        PeerId::new("repo-directory").map_err(|err| RepositoryError::Network(err.to_string()))?;
    let mut swarm = InMemorySwarm::default();
    swarm
        .add_peer(InMemoryPeer::new(
            client_descriptor("repo-client")
                .map_err(|err| RepositoryError::Network(err.to_string()))?,
        ))
        .map_err(|err| RepositoryError::Network(err.to_string()))?;
    swarm
        .add_peer(InMemoryPeer::new(
            gitmesh_network::NodeDescriptor::new(
                directory.clone(),
                gitmesh_network::OperatorId::new("repo-directory-operator")
                    .map_err(|err| RepositoryError::Network(err.to_string()))?,
                [gitmesh_network::NodeRole::Dht],
                "iad",
                [
                    gitmesh_network::ProtocolId::PingV0,
                    gitmesh_network::ProtocolId::AvailabilityV0,
                ],
            )
            .map_err(|err| RepositoryError::Network(err.to_string()))?,
        ))
        .map_err(|err| RepositoryError::Network(err.to_string()))?;
    for index in 0..=policy.total_shards() {
        let peer = format!("repo-storage-{index}");
        let operator = format!("repo-operator-{index}");
        let region = if index % 2 == 0 { "iad" } else { "sfo" };
        swarm
            .add_peer(InMemoryPeer::new(
                storage_descriptor(&peer, &operator, region)
                    .map_err(|err| RepositoryError::Network(err.to_string()))?,
            ))
            .map_err(|err| RepositoryError::Network(err.to_string()))?;
    }

    let descriptors = swarm.descriptors();
    let (_plan, providers) = plan_and_distribute_shards(
        &mut swarm,
        &client,
        descriptors.clone(),
        &placement_policy,
        &stored.shards,
        1,
        1_000,
    )?;
    publish_providers_via_transport(&mut swarm, &client, &directory, &providers)?;
    let vanished_provider = providers[policy.data_shards].clone();
    swarm
        .remove_peer(&vanished_provider.peer_id)
        .map_err(|err| RepositoryError::Network(err.to_string()))?;

    let repair = repair_shards_via_transport(
        &mut swarm,
        TransportRepairRequest {
            client_peer: &client,
            directory_peer: Some(&directory),
            segment: &stored.segment,
            policy: &policy,
            providers: &providers,
            replacement_descriptors: &descriptors,
            now_unix: 100,
            lease_epoch: 2,
            expires_at_unix: 2_000,
        },
    )?;
    let replacement_provider = repair
        .providers_after_repair
        .iter()
        .find(|provider| provider.shard_index == vanished_provider.shard_index)
        .ok_or(RepositoryError::InvalidStore(
            "missing replacement provider",
        ))?;
    let discovered = discover_providers_via_transport(
        &mut swarm,
        &client,
        &directory,
        stored.segment.cid,
        1_500,
    )?;
    let fetched = fetch_shards_via_transport(
        &mut swarm,
        &client,
        &discovered[..policy.data_shards],
        1_500,
    )?;
    let ciphertext = reconstruct_ciphertext(&stored.segment, &policy, &fetched)?;
    let canonical = decrypt_segment(&stored.segment, &ciphertext)?;
    let recovered = parse_canonical_object(&canonical)?;

    Ok(RepositoryTransportRepairProof {
        oid,
        recovered_exactly: recovered == object,
        repaired_shards: repair.repaired_shards,
        original_peer: vanished_provider.peer_id,
        replacement_peer: replacement_provider.peer_id.clone(),
        provider_count: discovered.len(),
        verified_after_repair: repair.audit_after.verified_shards.len(),
        durability_satisfied: repair.durability_satisfied,
    })
}

struct StoredObjectBuilder {
    oid: GitSha1Oid,
    kind: GitObjectKind,
    segment: EncryptedSegment,
    shards: Vec<Shard>,
}

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("storage failed: {0}")]
    Storage(#[from] gitmesh_storage::StorageError),
    #[error("Git object failed: {0}")]
    Git(#[from] GitError),
    #[error("object {0} is not in the repository object store")]
    MissingObject(GitSha1Oid),
    #[error("object {0} is not durably stored")]
    ObjectNotDurable(GitSha1Oid),
    #[error(
        "object {oid} has invalid kind for ref target: expected {expected:?}, actual {actual:?}"
    )]
    InvalidRefTarget {
        oid: GitSha1Oid,
        expected: GitObjectKind,
        actual: GitObjectKind,
    },
    #[error("non-fast-forward branch update rejected: {old_oid} is not an ancestor of {new_oid}")]
    NonFastForward {
        old_oid: GitSha1Oid,
        new_oid: GitSha1Oid,
    },
    #[error("canonical object length mismatch: expected {expected}, actual {actual}")]
    CanonicalLengthMismatch { expected: usize, actual: usize },
    #[error("invalid hex payload")]
    InvalidHex,
    #[error("unknown object kind '{0}'")]
    UnknownKind(String),
    #[error("repository store is invalid: {0}")]
    InvalidStore(&'static str),
    #[error("availability evidence does not match stored object shards")]
    InvalidAvailabilityEvidence,
    #[error("network failed: {0}")]
    Network(String),
    #[error("I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, RepositoryError>;

pub fn parse_git_object_kind(value: &str) -> Result<GitObjectKind> {
    match value {
        "blob" => Ok(GitObjectKind::Blob),
        "tree" => Ok(GitObjectKind::Tree),
        "commit" => Ok(GitObjectKind::Commit),
        "tag" => Ok(GitObjectKind::Tag),
        _ => Err(RepositoryError::UnknownKind(value.to_string())),
    }
}

pub fn encode_git_object_kind(kind: GitObjectKind) -> &'static str {
    match kind {
        GitObjectKind::Blob => "blob",
        GitObjectKind::Tree => "tree",
        GitObjectKind::Commit => "commit",
        GitObjectKind::Tag => "tag",
    }
}

pub fn decode_hex(value: &str) -> Result<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return Err(RepositoryError::InvalidHex);
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|chunk| {
            let high = hex_nibble(chunk[0]).ok_or(RepositoryError::InvalidHex)?;
            let low = hex_nibble(chunk[1]).ok_or(RepositoryError::InvalidHex)?;
            Ok((high << 4) | low)
        })
        .collect()
}

pub fn encode_hex(bytes: &[u8]) -> String {
    hex(bytes)
}

pub fn parse_oid(value: &str) -> Result<GitSha1Oid> {
    Ok(GitSha1Oid::from_str(value)?)
}

fn parse_usize(value: &str) -> Result<usize> {
    value
        .parse::<usize>()
        .map_err(|_| RepositoryError::InvalidStore("invalid integer"))
}

fn encode_aead_algorithm(algorithm: AeadAlgorithm) -> &'static str {
    match algorithm {
        AeadAlgorithm::XChaCha20Poly1305 => "xchacha20poly1305",
    }
}

fn parse_aead_algorithm(value: &str) -> Result<AeadAlgorithm> {
    match value {
        "xchacha20poly1305" => Ok(AeadAlgorithm::XChaCha20Poly1305),
        _ => Err(RepositoryError::InvalidStore("unknown AEAD algorithm")),
    }
}

fn fixed_bytes<const N: usize>(bytes: &[u8]) -> Result<[u8; N]> {
    bytes
        .try_into()
        .map_err(|_| RepositoryError::InvalidStore("fixed byte field has invalid length"))
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

    fn put_root_commit(store: &mut RepositoryObjectStore, message: &str) -> GitSha1Oid {
        let tree = GitObject::new(GitObjectKind::Tree, Vec::new());
        let tree_oid = tree.sha1_oid();
        store.put_git_object(tree).unwrap();
        let payload = format!(
            "tree {tree_oid}\nauthor A <a@example.com> 0 +0000\ncommitter A <a@example.com> 0 +0000\n\n{message}\n"
        );
        let commit = GitObject::new(GitObjectKind::Commit, payload.into_bytes());
        let commit_oid = commit.sha1_oid();
        store.put_git_object(commit).unwrap();
        commit_oid
    }

    fn put_commit_with_tree(
        store: &mut RepositoryObjectStore,
        tree_oid: GitSha1Oid,
        message: &str,
    ) -> GitSha1Oid {
        let payload = format!(
            "tree {tree_oid}\nauthor A <a@example.com> 0 +0000\ncommitter A <a@example.com> 0 +0000\n\n{message}\n"
        );
        let commit = GitObject::new(GitObjectKind::Commit, payload.into_bytes());
        let commit_oid = commit.sha1_oid();
        store.put_git_object(commit).unwrap();
        commit_oid
    }

    fn tree_with_blob(name: &[u8], blob_oid: GitSha1Oid) -> Vec<u8> {
        let mut tree = Vec::new();
        tree.extend_from_slice(b"100644 ");
        tree.extend_from_slice(name);
        tree.push(0);
        tree.extend_from_slice(&blob_oid.digest());
        tree
    }

    fn put_child_commit(
        store: &mut RepositoryObjectStore,
        parent_oid: GitSha1Oid,
        message: &str,
    ) -> GitSha1Oid {
        let tree_oid = GitSha1Oid::from_str("4b825dc642cb6eb9a060e54bf8d69288fbee4904").unwrap();
        let payload = format!(
            "tree {tree_oid}\nparent {parent_oid}\nauthor A <a@example.com> 0 +0000\ncommitter A <a@example.com> 0 +0000\n\n{message}\n"
        );
        let commit = GitObject::new(GitObjectKind::Commit, payload.into_bytes());
        let commit_oid = commit.sha1_oid();
        store.put_git_object(commit).unwrap();
        commit_oid
    }

    fn providers_for_record(record: &GitObjectRecord, count: usize) -> Vec<ShardProviderRecord> {
        record
            .shard_cids
            .iter()
            .take(count)
            .enumerate()
            .map(|(index, shard_cid)| {
                let region = if index % 2 == 0 { "iad" } else { "sfo" };
                ShardProviderRecord::new(
                    ShardRef {
                        segment_cid: record.segment_cid,
                        shard_cid: *shard_cid,
                        shard_index: index,
                    },
                    PeerId::new(format!("peer-{index}")).unwrap(),
                    OperatorId::new(format!("operator-{index}")).unwrap(),
                    region,
                    [NodeRole::Storage],
                    gitmesh_network::ProviderLease::new(1, 500).unwrap(),
                )
                .unwrap()
            })
            .collect()
    }

    #[test]
    fn stores_and_reads_git_object() {
        let mut store = RepositoryObjectStore::default();
        let object = GitObject::new(GitObjectKind::Blob, b"hello backend\n");
        let oid = object.sha1_oid();

        let record = store.put_git_object(object.clone()).unwrap();
        let recovered = store.get_git_object(oid).unwrap();

        assert_eq!(record.oid, oid);
        assert_eq!(record.kind, GitObjectKind::Blob);
        assert!(record.durability_satisfied);
        assert_eq!(recovered, object);
    }

    #[test]
    fn read_survives_parity_count_node_loss() {
        let mut store = RepositoryObjectStore::default();
        let object = GitObject::new(GitObjectKind::Blob, b"survive shard loss");
        let oid = object.sha1_oid();
        store.put_git_object(object.clone()).unwrap();

        let recovered = store
            .read_after_losing_nodes(oid, &[0, 3, 6, 9, 12, 15])
            .unwrap();

        assert_eq!(recovered, object);
    }

    #[test]
    fn duplicate_put_is_idempotent() {
        let mut store = RepositoryObjectStore::default();
        let object = GitObject::new(GitObjectKind::Blob, b"same object");

        let first = store.put_git_object(object.clone()).unwrap();
        let second = store.put_git_object(object).unwrap();

        assert_eq!(first, second);
        assert_eq!(store.object_count(), 1);
        assert!(store.has_durable_object(first.oid));
    }

    #[test]
    fn object_availability_report_uses_qualified_provider_evidence() {
        let mut store = RepositoryObjectStore::default();
        let object = GitObject::new(GitObjectKind::Blob, b"qualified availability");
        let record = store.put_git_object(object).unwrap();
        let providers = store
            .local_provider_records_for_object(record.oid, 1, 500)
            .unwrap();
        let requirement = store.policy().availability_requirement().unwrap();

        let report = store
            .object_availability_report(record.oid, &providers, requirement, 100)
            .unwrap();
        let satisfied = store
            .has_qualified_durable_object(record.oid, &providers, requirement, 100)
            .unwrap();

        assert_eq!(report.distinct_shard_count, store.policy().total_shards());
        assert_eq!(
            report.distinct_operator_count,
            store.policy().total_shards()
        );
        assert_eq!(report.distinct_region_count, 2);
        assert!(report.satisfies_requirement());
        assert!(satisfied);
    }

    #[test]
    fn object_availability_report_rejects_foreign_shard_evidence() {
        let mut store = RepositoryObjectStore::default();
        let object = GitObject::new(GitObjectKind::Blob, b"availability evidence");
        let record = store.put_git_object(object).unwrap();
        let mut providers = providers_for_record(&record, store.policy().data_shards);
        providers[0].shard_cid = shard_cid(record.segment_cid, 0, b"forged");
        let requirement = store.policy().availability_requirement().unwrap();

        let err = store
            .object_availability_report(record.oid, &providers, requirement, 100)
            .unwrap_err();

        assert!(matches!(err, RepositoryError::InvalidAvailabilityEvidence));
    }

    #[test]
    fn audits_and_repairs_object_shard_loss() {
        let mut store = RepositoryObjectStore::default();
        let object = GitObject::new(GitObjectKind::Blob, b"repairable object");
        let oid = object.sha1_oid();
        store.put_git_object(object.clone()).unwrap();
        store.simulate_object_shard_loss(oid, &[0, 3, 6]).unwrap();

        let audit = store.audit_object(oid).unwrap();
        let repair = store.repair_object(oid).unwrap();
        let post_audit = store.audit_object(oid).unwrap();
        let recovered = store.get_git_object(oid).unwrap();

        assert_eq!(audit.report.missing_shards, vec![0, 3, 6]);
        assert!(audit.report.repair_needed);
        assert_eq!(repair.outcome.repaired_shards, vec![0, 3, 6]);
        assert_eq!(
            repair.outcome.verified_after_repair,
            store.policy().total_shards()
        );
        assert!(!post_audit.report.repair_needed);
        assert_eq!(recovered, object);
    }

    #[test]
    fn repair_refuses_object_below_threshold() {
        let mut store = RepositoryObjectStore::default();
        let object = GitObject::new(GitObjectKind::Blob, b"too sparse");
        let oid = object.sha1_oid();
        store.put_git_object(object).unwrap();
        store
            .simulate_object_shard_loss(oid, &[0, 1, 2, 3, 4, 5, 6])
            .unwrap();

        let err = store.repair_object(oid).unwrap_err();

        assert!(matches!(
            err,
            RepositoryError::Storage(gitmesh_storage::StorageError::NotEnoughShards {
                available: 9,
                required: 10
            })
        ));
    }

    #[test]
    fn transport_repair_proof_recovers_git_object_after_provider_loss() {
        let proof = run_repository_transport_repair_proof(b"repository transport repair").unwrap();

        assert!(proof.recovered_exactly);
        assert_eq!(proof.repaired_shards, vec![3]);
        assert_ne!(proof.original_peer, proof.replacement_peer);
        assert_eq!(proof.provider_count, 5);
        assert_eq!(proof.verified_after_repair, 5);
        assert!(proof.durability_satisfied);
    }

    #[test]
    fn exports_all_objects_as_full_object_pack() {
        let mut store = RepositoryObjectStore::default();
        let blob = GitObject::new(GitObjectKind::Blob, b"hello");
        let tree = GitObject::new(GitObjectKind::Tree, Vec::new());
        store.put_git_object(blob.clone()).unwrap();
        store.put_git_object(tree.clone()).unwrap();

        let pack = store.export_pack_all().unwrap();
        let parsed = gitmesh_git::parse_packfile(&pack).unwrap();

        assert_eq!(parsed.objects.len(), 2);
        assert!(parsed.objects.contains(&blob));
        assert!(parsed.objects.contains(&tree));
    }

    #[test]
    fn exports_reachable_closure_from_requested_tip() {
        let mut store = RepositoryObjectStore::default();
        let included_blob = GitObject::new(GitObjectKind::Blob, b"included");
        let included_blob_oid = included_blob.sha1_oid();
        store.put_git_object(included_blob.clone()).unwrap();
        let included_tree = GitObject::new(
            GitObjectKind::Tree,
            tree_with_blob(b"included.txt", included_blob_oid),
        );
        let included_tree_oid = included_tree.sha1_oid();
        store.put_git_object(included_tree.clone()).unwrap();
        let included_commit = GitObject::new(
            GitObjectKind::Commit,
            format!(
                "tree {included_tree_oid}\nauthor A <a@example.com> 0 +0000\ncommitter A <a@example.com> 0 +0000\n\nincluded\n"
            )
            .into_bytes(),
        );
        let included_commit_oid = included_commit.sha1_oid();
        store.put_git_object(included_commit.clone()).unwrap();
        let unrelated = GitObject::new(GitObjectKind::Blob, b"unrelated");
        store.put_git_object(unrelated.clone()).unwrap();

        let pack = store
            .export_pack_reachable_from(&[included_commit_oid])
            .unwrap();
        let parsed = gitmesh_git::parse_packfile(&pack).unwrap();

        assert_eq!(parsed.objects.len(), 3);
        assert!(parsed.objects.contains(&included_blob));
        assert!(parsed.objects.contains(&included_tree));
        assert!(parsed.objects.contains(&included_commit));
        assert!(!parsed.objects.contains(&unrelated));
    }

    #[test]
    fn branch_ref_target_requires_durable_commit_graph() {
        let mut store = RepositoryObjectStore::default();
        let commit_oid = put_root_commit(&mut store, "initial");

        store
            .validate_ref_target("refs/heads/main", commit_oid)
            .unwrap();
    }

    #[test]
    fn branch_ref_target_accepts_tree_with_blob_entry() {
        let mut store = RepositoryObjectStore::default();
        let blob = GitObject::new(GitObjectKind::Blob, b"readme");
        let blob_oid = blob.sha1_oid();
        store.put_git_object(blob).unwrap();
        let tree = GitObject::new(GitObjectKind::Tree, tree_with_blob(b"README.md", blob_oid));
        let tree_oid = tree.sha1_oid();
        store.put_git_object(tree).unwrap();
        let commit_oid = put_commit_with_tree(&mut store, tree_oid, "with readme");

        store
            .validate_ref_target("refs/heads/main", commit_oid)
            .unwrap();
    }

    #[test]
    fn branch_ref_target_rejects_tree_with_missing_blob_entry() {
        let mut store = RepositoryObjectStore::default();
        let missing_blob =
            GitSha1Oid::from_str("3b18e512dba79e4c8300dd08aeb37f8e728b8dad").unwrap();
        let tree = GitObject::new(
            GitObjectKind::Tree,
            tree_with_blob(b"README.md", missing_blob),
        );
        let tree_oid = tree.sha1_oid();
        store.put_git_object(tree).unwrap();
        let commit_oid = put_commit_with_tree(&mut store, tree_oid, "missing readme");

        let err = store
            .validate_ref_target("refs/heads/main", commit_oid)
            .unwrap_err();

        assert!(matches!(err, RepositoryError::MissingObject(_)));
    }

    #[test]
    fn branch_update_accepts_fast_forward_commit() {
        let mut store = RepositoryObjectStore::default();
        let root = put_root_commit(&mut store, "root");
        let child = put_child_commit(&mut store, root, "child");

        store
            .validate_ref_update("refs/heads/main", Some(root), Some(child), false)
            .unwrap();
    }

    #[test]
    fn branch_update_rejects_non_fast_forward_commit() {
        let mut store = RepositoryObjectStore::default();
        let root = put_root_commit(&mut store, "root");
        let unrelated = put_root_commit(&mut store, "unrelated");

        let err = store
            .validate_ref_update("refs/heads/main", Some(root), Some(unrelated), false)
            .unwrap_err();

        assert!(matches!(err, RepositoryError::NonFastForward { .. }));
    }

    #[test]
    fn forced_branch_update_skips_fast_forward_check() {
        let mut store = RepositoryObjectStore::default();
        let root = put_root_commit(&mut store, "root");
        let unrelated = put_root_commit(&mut store, "unrelated");

        store
            .validate_ref_update("refs/heads/main", Some(root), Some(unrelated), true)
            .unwrap();
    }

    #[test]
    fn branch_ref_target_rejects_blob() {
        let mut store = RepositoryObjectStore::default();
        let blob = GitObject::new(GitObjectKind::Blob, b"not a commit");
        let blob_oid = blob.sha1_oid();
        store.put_git_object(blob).unwrap();

        let err = store
            .validate_ref_target("refs/heads/main", blob_oid)
            .unwrap_err();

        assert!(matches!(
            err,
            RepositoryError::InvalidRefTarget {
                expected: GitObjectKind::Commit,
                actual: GitObjectKind::Blob,
                ..
            }
        ));
    }

    #[test]
    fn branch_ref_target_rejects_missing_tree() {
        let mut store = RepositoryObjectStore::default();
        let commit = GitObject::new(
            GitObjectKind::Commit,
            b"tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\nauthor A <a@example.com> 0 +0000\ncommitter A <a@example.com> 0 +0000\n\ninitial\n".to_vec(),
        );
        let commit_oid = commit.sha1_oid();
        store.put_git_object(commit).unwrap();

        let err = store
            .validate_ref_target("refs/heads/main", commit_oid)
            .unwrap_err();

        assert!(matches!(err, RepositoryError::MissingObject(_)));
    }

    #[test]
    fn hex_payload_round_trips() {
        let bytes = b"abc123";

        assert_eq!(decode_hex(&encode_hex(bytes)).unwrap(), bytes);
        assert!(decode_hex("abc").is_err());
    }

    #[test]
    fn snapshot_round_trips() {
        let mut store = RepositoryObjectStore::default();
        let object = GitObject::new(GitObjectKind::Blob, b"persist me");
        let oid = object.sha1_oid();
        store.put_git_object(object.clone()).unwrap();

        let mut snapshot = String::new();
        snapshot.push_str("gitmesh-repository-store-v0\n");
        snapshot.push_str("policy\t10\t6\n");
        for stored in store.objects.values() {
            snapshot.push_str(&format!(
                "object\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                stored.record.oid,
                encode_git_object_kind(stored.record.kind),
                stored.segment.plaintext_len,
                stored.segment.ciphertext_len,
                encode_aead_algorithm(stored.segment.algorithm),
                encode_hex(&stored.segment.nonce),
                encode_hex(&stored.segment.key),
                encode_hex(&stored.segment.ciphertext)
            ));
            for shard in &stored.shards {
                snapshot.push_str(&format!(
                    "shard\t{}\t{}\t{}\t{}\t{}\n",
                    stored.record.oid,
                    shard.shard_index,
                    shard.shard_count,
                    shard.data_shards,
                    encode_hex(&shard.bytes)
                ));
            }
        }

        let restored = RepositoryObjectStore::from_snapshot(&snapshot).unwrap();

        assert_eq!(restored.object_count(), 1);
        assert_eq!(restored.get_git_object(oid).unwrap(), object);
        assert_eq!(restored.policy().min_distinct_operators, 3);
        assert_eq!(restored.policy().min_distinct_regions, 2);
    }

    #[test]
    fn snapshot_round_trips_storage_policy_diversity_thresholds() {
        let policy = StoragePolicy {
            data_shards: 4,
            parity_shards: 2,
            min_distinct_operators: 4,
            min_distinct_regions: 3,
        };
        let store = RepositoryObjectStore::new(policy.clone());
        let path = std::env::temp_dir().join(format!(
            "gitmesh-repository-policy-{}.snapshot",
            std::process::id()
        ));

        store.save_to_path(&path).unwrap();
        let restored = RepositoryObjectStore::load_from_path(&path).unwrap();
        let _ = std::fs::remove_file(path);

        assert_eq!(restored.policy(), &policy);
    }

    #[test]
    fn invalid_snapshot_is_rejected() {
        let err = RepositoryObjectStore::from_snapshot("bad\n").unwrap_err();

        assert!(matches!(err, RepositoryError::InvalidStore(_)));
    }
}
