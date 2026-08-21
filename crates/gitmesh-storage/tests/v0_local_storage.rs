use gitmesh_storage::{
    SimulatedNetwork, StorageError, StoragePolicy, decrypt_segment, encrypt_segment,
    erasure_encode, reconstruct_ciphertext, run_v0_local_storage_proof,
};

use gitmesh_network::InMemoryAvailabilityDirectory;

#[test]
fn v0_acceptance_round_trips_after_six_node_losses() {
    let payload = include_bytes!("../../../docs/ARCHITECTURE.md");
    let policy = StoragePolicy::default();
    let result = run_v0_local_storage_proof(payload, policy, vec![0, 2, 4, 6, 8, 10]).unwrap();

    assert_eq!(result.recovered, payload);
}

#[test]
fn corrupted_shards_are_ignored_before_reconstruction() {
    let payload = b"corrupted shards should not be trusted";
    let policy = StoragePolicy::default();
    let encrypted = encrypt_segment(payload).unwrap();
    let shards = erasure_encode(&encrypted, &policy).unwrap();
    let mut network = SimulatedNetwork::with_node_count(policy.total_shards());
    network.store_shards(shards).unwrap();
    network.destroy_nodes(&[0, 1, 2, 3, 4]).unwrap();

    let mut available = network.available_shards();
    available[0].shard.bytes[0] ^= 0xff;

    let ciphertext = reconstruct_ciphertext(&encrypted, &policy, &available).unwrap();
    let recovered = decrypt_segment(&encrypted, &ciphertext).unwrap();

    assert_eq!(recovered, payload);
}

#[test]
fn too_many_missing_shards_fails_cleanly() {
    let payload = b"losing seven of sixteen shards should fail with k=10";
    let policy = StoragePolicy::default();
    let encrypted = encrypt_segment(payload).unwrap();
    let shards = erasure_encode(&encrypted, &policy).unwrap();
    let mut network = SimulatedNetwork::with_node_count(policy.total_shards());
    network.store_shards(shards).unwrap();
    network.destroy_nodes(&[0, 1, 2, 3, 4, 5, 6]).unwrap();

    let err = reconstruct_ciphertext(&encrypted, &policy, &network.available_shards()).unwrap_err();

    assert!(matches!(
        err,
        StorageError::NotEnoughShards {
            available: 9,
            required: 10
        }
    ));
}

#[test]
fn simulated_network_publishes_active_storage_records() {
    let payload = b"availability records should mirror available shards";
    let policy = StoragePolicy::default();
    let encrypted = encrypt_segment(payload).unwrap();
    let shards = erasure_encode(&encrypted, &policy).unwrap();
    let mut network = SimulatedNetwork::with_node_count(policy.total_shards());
    network.store_shards(shards).unwrap();
    network.destroy_nodes(&[0, 1, 2]).unwrap();

    let mut directory = InMemoryAvailabilityDirectory::default();
    network
        .publish_availability(&mut directory, 7, 1_000)
        .unwrap();

    assert_eq!(
        directory.durable_shard_count(encrypted.cid, 100),
        policy.total_shards() - 3
    );
    assert_eq!(directory.durable_shard_count(encrypted.cid, 1_001), 0);
}
