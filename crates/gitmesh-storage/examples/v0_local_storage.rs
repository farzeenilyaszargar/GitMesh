use gitmesh_storage::{StoragePolicy, run_v0_local_storage_proof};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let plaintext = if input.is_empty() {
        "GitMesh V0 local storage proof: encrypt, erasure-code, lose shards, recover exactly."
            .as_bytes()
            .to_vec()
    } else {
        input.into_bytes()
    };

    let policy = StoragePolicy::default();
    let destroyed_nodes = vec![0, 3, 6, 9, 12, 15];
    let result = run_v0_local_storage_proof(&plaintext, policy.clone(), destroyed_nodes)?;

    println!("GitMesh V0 local storage proof");
    println!("data shards: {}", policy.data_shards);
    println!("parity shards: {}", policy.parity_shards);
    println!("plaintext bytes: {}", result.plaintext_len);
    println!("ciphertext bytes: {}", result.ciphertext_len);
    println!("destroyed nodes: {:?}", result.destroyed_nodes);
    println!("available shards: {}", result.available_shards);
    println!("segment cid: {}", result.segment_cid);
    println!("recovered exactly: {}", result.recovered == plaintext);

    Ok(())
}
