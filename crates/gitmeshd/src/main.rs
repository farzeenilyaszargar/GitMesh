use std::{fs, path::PathBuf, process::ExitCode};

use gitmesh_repository::{RepositoryError, encode_hex, run_repository_transport_repair_proof};
use gitmesh_storage::{StoragePolicy, run_v0_local_storage_proof};
use gitmeshd::{
    DaemonAuth, DaemonStorePaths, default_socket_path, request_unix_socket,
    request_unix_socket_frame, serve_unix_socket_with_stores_and_auth,
};
use thiserror::Error;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("gitmeshd: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), GitMeshdError> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("v0-proof") => run_v0_proof(args.collect()),
        Some("serve") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            let object_store_path: Option<PathBuf> = args.next().map(Into::into);
            let ref_store_path: Option<PathBuf> = args.next().map(Into::into);
            let policy_store_path: Option<PathBuf> = args.next().map(Into::into);
            let key_grant_store_path: Option<PathBuf> = args.next().map(Into::into);
            let account_store_path: Option<PathBuf> = args.next().map(Into::into);
            let collaboration_store_path: Option<PathBuf> = args.next().map(Into::into);
            let network_store_path: Option<PathBuf> = args.next().map(Into::into);
            println!("gitmeshd listening on {}", socket_path.display());
            if let Some(path) = &object_store_path {
                println!("gitmeshd object store {}", path.display());
            }
            if let Some(path) = &ref_store_path {
                println!("gitmeshd ref store {}", path.display());
            }
            if let Some(path) = &policy_store_path {
                println!("gitmeshd policy store {}", path.display());
            }
            if let Some(path) = &key_grant_store_path {
                println!("gitmeshd key grant store {}", path.display());
            }
            if let Some(path) = &account_store_path {
                println!("gitmeshd account store {}", path.display());
            }
            if let Some(path) = &collaboration_store_path {
                println!("gitmeshd collaboration store {}", path.display());
            }
            if let Some(path) = &network_store_path {
                println!("gitmeshd network store {}", path.display());
            }
            let auth = DaemonAuth::from_env()?;
            if auth.is_enabled() {
                println!("gitmeshd admin auth enabled");
            }
            serve_unix_socket_with_stores_and_auth(
                socket_path,
                DaemonStorePaths {
                    object_store_path,
                    ref_store_path,
                    policy_store_path,
                    key_grant_store_path,
                    account_store_path,
                    collaboration_store_path,
                    network_store_path,
                },
                auth,
            )?;
            Ok(())
        }
        Some("ping") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            println!("{}", request_unix_socket(socket_path, "PING")?);
            Ok(())
        }
        Some("frame-ping") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            let request_id = args.next().unwrap_or_else(|| "frame-ping".to_string());
            let response = request_unix_socket_frame(socket_path, &request_id, "PING")?;
            println!(
                "id={} status={} payload={}",
                response.request_id,
                if response.is_error { "error" } else { "ok" },
                String::from_utf8_lossy(&response.payload)
            );
            Ok(())
        }
        Some("socket-v0-proof") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            let payload = args.collect::<Vec<_>>().join(" ");
            let command = if payload.is_empty() {
                "V0_PROOF".to_string()
            } else {
                format!("V0_PROOF {payload}")
            };
            println!("{}", request_unix_socket(socket_path, &command)?);
            Ok(())
        }
        Some("network-repair-proof") => run_network_repair_proof(args.collect()),
        Some("socket-network-repair-proof") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            let payload = args.collect::<Vec<_>>().join(" ");
            let command = if payload.is_empty() {
                "NETWORK_REPAIR_PROOF".to_string()
            } else {
                format!("NETWORK_REPAIR_PROOF {payload}")
            };
            println!("{}", request_unix_socket(socket_path, &command)?);
            Ok(())
        }
        Some("ref-get") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            let ref_name = args
                .next()
                .ok_or_else(|| GitMeshdError::InvalidArguments("missing ref name".to_string()))?;
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("REF_GET {ref_name}"))?
            );
            Ok(())
        }
        Some("ref-list") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            println!("{}", request_unix_socket(socket_path, "REF_LIST")?);
            Ok(())
        }
        Some("ref-update") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            let rest = args.collect::<Vec<_>>();
            if rest.len() != 5 {
                return Err(GitMeshdError::InvalidArguments(
                    "ref-update requires <tx> <ref> <expected|none> <new|delete> <signer>"
                        .to_string(),
                ));
            }
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("REF_UPDATE {}", rest.join(" ")))?
            );
            Ok(())
        }
        Some("ref-checkpoint") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            println!("{}", request_unix_socket(socket_path, "REF_CHECKPOINT")?);
            Ok(())
        }
        Some("policy-show") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            println!("{}", request_unix_socket(socket_path, "POLICY_SHOW")?);
            Ok(())
        }
        Some("policy-require-signed") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            let value = args.next().ok_or_else(|| {
                GitMeshdError::InvalidArguments("missing true|false value".to_string())
            })?;
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("POLICY_SET_REQUIRE_SIGNED {value}"))?
            );
            Ok(())
        }
        Some("policy-grant-writer-account") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            let account_id = args
                .next()
                .ok_or_else(|| GitMeshdError::InvalidArguments("missing account id".to_string()))?;
            println!(
                "{}",
                request_unix_socket(
                    socket_path,
                    &format!("POLICY_GRANT_WRITER_ACCOUNT {account_id}")
                )?
            );
            Ok(())
        }
        Some("policy-grant-force-account") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            let account_id = args
                .next()
                .ok_or_else(|| GitMeshdError::InvalidArguments("missing account id".to_string()))?;
            println!(
                "{}",
                request_unix_socket(
                    socket_path,
                    &format!("POLICY_GRANT_FORCE_ACCOUNT {account_id}")
                )?
            );
            Ok(())
        }
        Some("policy-protect-ref") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            let ref_name = args
                .next()
                .ok_or_else(|| GitMeshdError::InvalidArguments("missing ref".to_string()))?;
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("POLICY_PROTECT_REF {ref_name}"))?
            );
            Ok(())
        }
        Some("object-put") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            let kind = args.next().ok_or_else(|| {
                GitMeshdError::InvalidArguments("missing object kind".to_string())
            })?;
            let payload_hex = args.next().ok_or_else(|| {
                GitMeshdError::InvalidArguments("missing hex payload".to_string())
            })?;
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("OBJECT_PUT {kind} {payload_hex}"))?
            );
            Ok(())
        }
        Some("object-availability") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            let oid = args
                .next()
                .ok_or_else(|| GitMeshdError::InvalidArguments("missing object oid".to_string()))?;
            let min_shards = args.next().unwrap_or_else(|| "10".to_string());
            let min_operators = args.next().unwrap_or_else(|| "3".to_string());
            let min_regions = args.next().unwrap_or_else(|| "2".to_string());
            println!(
                "{}",
                request_unix_socket(
                    socket_path,
                    &format!(
                        "OBJECT_AVAILABILITY {oid} {min_shards} {min_operators} {min_regions}"
                    )
                )?
            );
            Ok(())
        }
        Some("pack-put") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            let pack_hex = args
                .next()
                .ok_or_else(|| GitMeshdError::InvalidArguments("missing pack hex".to_string()))?;
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("PACK_PUT {pack_hex}"))?
            );
            Ok(())
        }
        Some("pack-import") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            let pack_path = args
                .next()
                .ok_or_else(|| GitMeshdError::InvalidArguments("missing pack path".to_string()))?;
            let pack_hex = encode_hex(&fs::read(pack_path)?);
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("PACK_PUT {pack_hex}"))?
            );
            Ok(())
        }
        Some("pack-get") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            println!("{}", request_unix_socket(socket_path, "PACK_GET all")?);
            Ok(())
        }
        Some("pack-export") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            let pack_path = args
                .next()
                .ok_or_else(|| GitMeshdError::InvalidArguments("missing pack path".to_string()))?;
            let response = request_unix_socket(socket_path, "PACK_GET all")?;
            let pack_hex = response
                .split_whitespace()
                .find_map(|part| part.strip_prefix("pack_hex="))
                .ok_or_else(|| GitMeshdError::InvalidDaemonResponse(response.clone()))?;
            fs::write(pack_path, decode_hex(pack_hex)?)?;
            println!("{response}");
            Ok(())
        }
        Some("object-get") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            let oid = args
                .next()
                .ok_or_else(|| GitMeshdError::InvalidArguments("missing object oid".to_string()))?;
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("OBJECT_GET {oid}"))?
            );
            Ok(())
        }
        Some("object-list") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            println!("{}", request_unix_socket(socket_path, "OBJECT_LIST")?);
            Ok(())
        }
        Some("repo-status") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            println!("{}", request_unix_socket(socket_path, "REPO_STATUS")?);
            Ok(())
        }
        Some("network-status") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            println!("{}", request_unix_socket(socket_path, "NETWORK_STATUS")?);
            Ok(())
        }
        Some("network-listen") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            let address = args.next().ok_or_else(|| {
                GitMeshdError::InvalidArguments("missing listen address".to_string())
            })?;
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("NETWORK_LISTEN {address}"))?
            );
            Ok(())
        }
        Some("network-bootstrap") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            let rest = args.collect::<Vec<_>>();
            if rest.len() != 4 {
                return Err(GitMeshdError::InvalidArguments(
                    "network-bootstrap requires <peer-id> <operator-id> <region> <address>"
                        .to_string(),
                ));
            }
            println!(
                "{}",
                request_unix_socket(
                    socket_path,
                    &format!("NETWORK_BOOTSTRAP {}", rest.join(" "))
                )?
            );
            Ok(())
        }
        Some("network-peer-add") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            let rest = args.collect::<Vec<_>>();
            if rest.len() != 6 {
                return Err(GitMeshdError::InvalidArguments(
                    "network-peer-add requires <peer-id> <operator-id> <roles-csv> <region> <protocols-csv> <addresses-csv|->"
                        .to_string(),
                ));
            }
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("NETWORK_PEER_ADD {}", rest.join(" ")))?
            );
            Ok(())
        }
        Some("network-peer-list") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            println!("{}", request_unix_socket(socket_path, "NETWORK_PEER_LIST")?);
            Ok(())
        }
        Some("collab-seed-samples") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            println!(
                "{}",
                request_unix_socket(socket_path, "COLLAB_SEED_SAMPLES")?
            );
            Ok(())
        }
        Some("issue-list") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            let repo = args
                .next()
                .ok_or_else(|| GitMeshdError::InvalidArguments("missing repo".to_string()))?;
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("ISSUE_LIST {repo}"))?
            );
            Ok(())
        }
        Some("issue-open") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            let rest = args.collect::<Vec<_>>();
            if rest.len() != 5 {
                return Err(GitMeshdError::InvalidArguments(
                    "issue-open requires <repo> <actor> <title-hex> <body-hex|-> <labels-hex-list|->"
                        .to_string(),
                ));
            }
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("ISSUE_OPEN {}", rest.join(" ")))?
            );
            Ok(())
        }
        Some("pr-list") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            let repo = args
                .next()
                .ok_or_else(|| GitMeshdError::InvalidArguments("missing repo".to_string()))?;
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("PR_LIST {repo}"))?
            );
            Ok(())
        }
        Some("pr-open") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            let rest = args.collect::<Vec<_>>();
            if rest.len() != 7 {
                return Err(GitMeshdError::InvalidArguments(
                    "pr-open requires <repo> <actor> <source-ref> <target-ref> <title-hex> <body-hex|-> <labels-hex-list|->"
                        .to_string(),
                ));
            }
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("PR_OPEN {}", rest.join(" ")))?
            );
            Ok(())
        }
        Some("key-grant-list") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            let repo_id = args
                .next()
                .ok_or_else(|| GitMeshdError::InvalidArguments("missing repo id".to_string()))?;
            let selector = args.next().unwrap_or_else(|| "latest".to_string());
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("KEY_GRANT_LIST {repo_id} {selector}"))?
            );
            Ok(())
        }
        Some("key-grant-revoke-device") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            let device_id = args
                .next()
                .ok_or_else(|| GitMeshdError::InvalidArguments("missing device id".to_string()))?;
            let effective_epoch = args.next().ok_or_else(|| {
                GitMeshdError::InvalidArguments("missing effective epoch".to_string())
            })?;
            println!(
                "{}",
                request_unix_socket(
                    socket_path,
                    &format!("KEY_GRANT_REVOKE_DEVICE {device_id} {effective_epoch}")
                )?
            );
            Ok(())
        }
        Some("key-grant-status") => {
            let socket_path = args.next().map_or_else(default_socket_path, Into::into);
            let repo_id = args
                .next()
                .ok_or_else(|| GitMeshdError::InvalidArguments("missing repo id".to_string()))?;
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("KEY_GRANT_STATUS {repo_id}"))?
            );
            Ok(())
        }
        Some("help") | Some("--help") | Some("-h") | None => {
            print_help();
            Ok(())
        }
        Some(command) => Err(GitMeshdError::UnknownCommand(command.to_string())),
    }
}

fn run_v0_proof(words: Vec<String>) -> Result<(), GitMeshdError> {
    let payload = if words.is_empty() {
        b"gitmeshd V0 local storage proof".to_vec()
    } else {
        words.join(" ").into_bytes()
    };
    let policy = StoragePolicy::default();
    let destroyed_nodes = vec![0, 3, 6, 9, 12, 15];
    let result = run_v0_local_storage_proof(&payload, policy.clone(), destroyed_nodes)?;

    println!("gitmeshd V0 proof");
    println!("data_shards={}", policy.data_shards);
    println!("parity_shards={}", policy.parity_shards);
    println!("plaintext_bytes={}", result.plaintext_len);
    println!("ciphertext_bytes={}", result.ciphertext_len);
    println!("destroyed_nodes={:?}", result.destroyed_nodes);
    println!("available_shards={}", result.available_shards);
    println!("segment_cid={}", result.segment_cid);
    println!("recovered_exactly={}", result.recovered == payload);
    Ok(())
}

fn run_network_repair_proof(words: Vec<String>) -> Result<(), GitMeshdError> {
    let payload = if words.is_empty() {
        b"gitmeshd repository transport repair proof".to_vec()
    } else {
        words.join(" ").into_bytes()
    };
    let proof = run_repository_transport_repair_proof(&payload)?;

    println!("gitmeshd repository transport repair proof");
    println!("oid={}", proof.oid);
    println!("recovered_exactly={}", proof.recovered_exactly);
    println!("repaired_shards={:?}", proof.repaired_shards);
    println!("original_peer={}", proof.original_peer);
    println!("replacement_peer={}", proof.replacement_peer);
    println!("providers={}", proof.provider_count);
    println!("verified_after_repair={}", proof.verified_after_repair);
    println!("durability_satisfied={}", proof.durability_satisfied);
    Ok(())
}

fn print_help() {
    println!("gitmeshd");
    println!();
    println!("Commands:");
    println!("  v0-proof [payload...]   run the local encrypt/erasure-code/recover proof");
    println!("  network-repair-proof [payload...]   run Git-object transport repair proof");
    println!(
        "  serve [socket] [object-store] [ref-store] [policy-store] [key-grant-store] [account-store] [collaboration-store] [network-store]   run the local daemon socket server"
    );
    println!("  ping [socket]           ping a running local daemon");
    println!("  frame-ping [socket] [request-id]   ping using the binary frame protocol");
    println!("  socket-v0-proof [socket] [payload...]");
    println!("  socket-network-repair-proof [socket] [payload...]");
    println!("  ref-get [socket] <ref>");
    println!("  ref-list [socket]");
    println!("  ref-update [socket] <tx> <ref> <expected|none> <new|delete> <signer>");
    println!("  ref-checkpoint [socket]");
    println!("  policy-show [socket]");
    println!("  policy-require-signed [socket] <true|false>");
    println!("  policy-grant-writer-account [socket] <account-cid>");
    println!("  policy-grant-force-account [socket] <account-cid>");
    println!("  policy-protect-ref [socket] <ref>");
    println!("  object-put [socket] <blob|tree|commit|tag> <hex-payload>");
    println!("  pack-put [socket] <pack-hex>");
    println!("  pack-import [socket] <pack-file>");
    println!("  pack-get [socket]");
    println!("  pack-export [socket] <pack-file>");
    println!("  object-get [socket] <oid>");
    println!("  object-list [socket]");
    println!("  repo-status [socket]");
    println!("  network-status [socket]");
    println!("  network-listen [socket] <multiaddr>");
    println!("  network-bootstrap [socket] <peer-id> <operator-id> <region> <multiaddr>");
    println!(
        "  network-peer-add [socket] <peer-id> <operator-id> <roles-csv> <region> <protocols-csv> <addresses-csv|->"
    );
    println!("  network-peer-list [socket]");
    println!("  key-grant-list [socket] <repo-id> [latest|all|epoch]");
    println!("  key-grant-revoke-device [socket] <device-cid> <effective-epoch>");
    println!("  key-grant-status [socket] <repo-id>");
    println!("  collab-seed-samples [socket]");
    println!(
        "  issue-open [socket] <owner/repo> <actor> <title-hex> <body-hex|-> <labels-hex-list|->"
    );
    println!("  issue-list [socket] <owner/repo>");
    println!(
        "  pr-open [socket] <owner/repo> <actor> <source-ref> <target-ref> <title-hex> <body-hex|-> <labels-hex-list|->"
    );
    println!("  pr-list [socket] <owner/repo>");
}

#[derive(Debug, Error)]
enum GitMeshdError {
    #[error("unknown command '{0}'")]
    UnknownCommand(String),
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    #[error(transparent)]
    Storage(#[from] gitmesh_storage::StorageError),
    #[error(transparent)]
    Daemon(#[from] gitmeshd::DaemonError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("invalid daemon response: {0}")]
    InvalidDaemonResponse(String),
    #[error("invalid hex payload")]
    InvalidHex,
    #[error("invalid arguments: {0}")]
    InvalidArguments(String),
}

fn decode_hex(value: &str) -> Result<Vec<u8>, GitMeshdError> {
    if !value.len().is_multiple_of(2) {
        return Err(GitMeshdError::InvalidHex);
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|chunk| {
            let high = hex_nibble(chunk[0]).ok_or(GitMeshdError::InvalidHex)?;
            let low = hex_nibble(chunk[1]).ok_or(GitMeshdError::InvalidHex)?;
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
