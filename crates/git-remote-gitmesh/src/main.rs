use std::io::{self, BufReader};
use std::path::PathBuf;
use std::process::ExitCode;

use git_remote_gitmesh::{HelperConfig, run_helper};
use gitmeshd::{default_socket_path, request_unix_socket};
use thiserror::Error;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("git-remote-gitmesh: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), MainError> {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|arg| arg == "--v0-proof") {
        args.remove(0);
        return run_v0_proof(args);
    }
    if args
        .first()
        .is_some_and(|arg| matches!(arg.as_str(), "--help" | "-h"))
    {
        print_help();
        return Ok(());
    }

    let repository = args
        .first()
        .cloned()
        .unwrap_or_else(|| "gitmesh://unknown".to_string());
    let url = args.get(1).cloned();
    let stdin = io::stdin();
    let stdout = io::stdout();
    let daemon_socket = default_socket_path();
    let refs_advertisement = request_unix_socket(&daemon_socket, "REF_LIST").ok();
    let pack_advertisement = request_unix_socket(&daemon_socket, "PACK_GET all").ok();
    let git_dir = std::env::var_os("GIT_DIR").map(PathBuf::from);
    run_helper(
        HelperConfig::new(repository, url)
            .with_refs_advertisement(refs_advertisement)
            .with_pack_advertisement(pack_advertisement)
            .with_git_dir(git_dir)
            .with_daemon_socket(Some(daemon_socket)),
        BufReader::new(stdin.lock()),
        stdout.lock(),
    )?;
    Ok(())
}

fn run_v0_proof(words: Vec<String>) -> Result<(), MainError> {
    let payload = if words.is_empty() {
        "git-remote-gitmesh V0 proof".to_string()
    } else {
        words.join(" ")
    };
    let response = request_unix_socket(default_socket_path(), &format!("V0_PROOF {payload}"))?;

    println!("{response}");
    Ok(())
}

fn print_help() {
    println!("git-remote-gitmesh <repository> [url]");
    println!();
    println!("Development commands:");
    println!("  --v0-proof [payload...]   run the local storage proof");
}

#[derive(Debug, Error)]
enum MainError {
    #[error(transparent)]
    Helper(#[from] git_remote_gitmesh::HelperError),
    #[error(transparent)]
    Daemon(#[from] gitmeshd::DaemonError),
}
