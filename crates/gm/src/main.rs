use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

use gitmesh_collaboration::{
    CollaborationEvent, CollaborationEventKind, CollaborationPayload, sample_issues,
    sample_pull_requests,
};
use gitmesh_coordination::{RefName, RefUpdate, RepoId, TransactionId};
use gitmesh_core::{Cid, CidKind, HashAlgorithm, hex};
use gitmesh_crypto::RepoContentKey;
use gitmesh_git::{GitObjectKind, GitSha1Oid, parse_loose_object, parse_packfile};
use gitmesh_identity::{AccountRootKey, DevIdentity, DeviceKey, short_id};
use gitmeshd::{default_socket_path, request_unix_socket};
use thiserror::Error;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("gm: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), GmError> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        None | Some("help") | Some("--help") | Some("-h") => {
            print_help();
            Ok(())
        }
        Some("auth") => auth(&args[1..]),
        Some("repo") => repo(&args[1..]),
        Some("issue") => issue(&args[1..]),
        Some("pr") => pr(&args[1..]),
        Some("daemon") => daemon(&args[1..]),
        Some("policy") => policy(&args[1..]),
        Some("ref") => refs(&args[1..]),
        Some("object") => object(&args[1..]),
        Some("key") => key(&args[1..]),
        Some("account") => account(&args[1..]),
        Some("session") => session(&args[1..]),
        Some("proof") => proof(&args[1..]),
        Some(command) => Err(GmError::UnknownCommand(command.to_string())),
    }
}

fn auth(args: &[String]) -> Result<(), GmError> {
    match args.first().map(String::as_str) {
        Some("init") => {
            let label = args.get(1).map_or("gm-local-device", String::as_str);
            validate_state_field(label)?;
            let identity = LocalIdentity::create(label.to_string());
            identity.save()?;
            print_identity_status(&identity, "configured");
            Ok(())
        }
        Some("status") | None => {
            if let Some(identity) = LocalIdentity::load_optional()? {
                print_identity_status(&identity, "configured");
            } else {
                let identity = DevIdentity::generate("gm-dev-device");
                identity.certificate.verify()?;
                println!("GitMesh authentication: local development mode");
                println!(
                    "Ephemeral account: {}",
                    short_id(identity.account_id.as_cid())
                );
                println!(
                    "Ephemeral device: {}",
                    short_id(identity.device_id.as_cid())
                );
                println!("Device certificate: verified");
                println!("Persistence: not configured yet");
                println!("Run: gm auth init [device-label]");
            }
            Ok(())
        }
        Some(command) => Err(GmError::UnknownCommand(format!("auth {command}"))),
    }
}

#[derive(Clone)]
struct LocalIdentity {
    label: String,
    account: AccountRootKey,
    device: DeviceKey,
    certificate: gitmesh_identity::DeviceCertificate,
}

impl LocalIdentity {
    fn create(label: String) -> Self {
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let certificate = account.certify_device(&device, label.clone());
        Self {
            label,
            account,
            device,
            certificate,
        }
    }

    fn load_optional() -> Result<Option<Self>, GmError> {
        let path = identity_path()?;
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(Self::load_from_path(&path)?))
    }

    fn load_or_create_default() -> Result<Self, GmError> {
        if let Some(identity) = Self::load_optional()? {
            return Ok(identity);
        }
        let identity = Self::create("gm-local-device".to_string());
        identity.save()?;
        Ok(identity)
    }

    fn load_from_path(path: &Path) -> Result<Self, GmError> {
        let text = fs::read_to_string(path)?;
        let mut lines = text.lines();
        if lines.next() != Some("gitmesh-local-identity-v0") {
            return Err(GmError::IdentityStoreCorrupt);
        }
        let label = read_identity_field(&mut lines, "label")?;
        let account_seed =
            decode_fixed_hex::<32>(&read_identity_field(&mut lines, "account_seed")?)?;
        let device_seed = decode_fixed_hex::<32>(&read_identity_field(&mut lines, "device_seed")?)?;
        if lines.next().is_some() {
            return Err(GmError::IdentityStoreCorrupt);
        }
        let account = AccountRootKey::from_seed(account_seed);
        let device = DeviceKey::from_seed(device_seed);
        let certificate = account.certify_device(&device, label.clone());
        certificate.verify()?;
        Ok(Self {
            label,
            account,
            device,
            certificate,
        })
    }

    fn save(&self) -> Result<(), GmError> {
        let path = identity_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let contents = format!(
            "gitmesh-local-identity-v0\nlabel\t{}\naccount_seed\t{}\ndevice_seed\t{}\n",
            self.label,
            hex(&self.account.seed_bytes()),
            hex(&self.device.seed_bytes())
        );
        let tmp_path = path.with_extension("tmp");
        fs::write(&tmp_path, contents)?;
        set_owner_only_permissions(&tmp_path)?;
        fs::rename(&tmp_path, &path)?;
        Ok(())
    }

    fn signed_ref_update_command(&self, args: &[String]) -> Result<String, GmError> {
        let update = RefUpdate {
            repo_id: RepoId::new(b"gitmeshd-v0-repo"),
            ref_name: RefName::new(&args[1])?,
            expected_old_oid: parse_optional_oid(&args[2])?,
            new_oid: if args[3] == "delete" {
                None
            } else {
                Some(GitSha1Oid::from_str(&args[3])?)
            },
            force: false,
            policy_epoch: 0,
            transaction_id: TransactionId::new(&args[0])?,
            signer: self.certificate.device_id.as_cid().to_string(),
        };
        let update_signature = self.device.sign(&update.signing_transcript());
        Ok(format!(
            "REF_UPDATE_SIGNED {} {} {} {} {} {} {} {} {}",
            args[0],
            args[1],
            args[2],
            args[3],
            hex(self.certificate.label.as_bytes()),
            hex(&self.certificate.account_verifying_key),
            hex(&self.certificate.device_verifying_key),
            hex(&self.certificate.signature),
            hex(&update_signature)
        ))
    }

    fn signed_issue_open_command(
        &self,
        repo: &str,
        title: &str,
        body: &str,
        labels: Vec<String>,
    ) -> Result<String, GmError> {
        let timestamp = now_unix()?;
        let event = CollaborationEvent::new(
            repo,
            CollaborationEventKind::IssueOpened,
            self.certificate.device_id.as_cid().as_hex(),
            Vec::new(),
            timestamp,
            CollaborationPayload::issue(title, body, labels),
        )?;
        let event_signature = self.device.sign(&event.signing_transcript());
        Ok(format!(
            "ISSUE_OPEN_SIGNED {} {} {} {} {} {} {} {} {} {}",
            repo,
            timestamp,
            encode_text_arg(title),
            encode_text_arg(body),
            encode_label_list_arg(&event.payload.labels),
            hex(self.certificate.label.as_bytes()),
            hex(&self.certificate.account_verifying_key),
            hex(&self.certificate.device_verifying_key),
            hex(&self.certificate.signature),
            hex(&event_signature)
        ))
    }

    fn signed_pr_open_command(
        &self,
        repo: &str,
        source_ref: &str,
        target_ref: &str,
        title: &str,
        body: &str,
        labels: Vec<String>,
    ) -> Result<String, GmError> {
        let timestamp = now_unix()?;
        let event = CollaborationEvent::new(
            repo,
            CollaborationEventKind::PullRequestOpened,
            self.certificate.device_id.as_cid().as_hex(),
            Vec::new(),
            timestamp,
            CollaborationPayload::pull_request(title, body, labels, source_ref, target_ref),
        )?;
        let event_signature = self.device.sign(&event.signing_transcript());
        Ok(format!(
            "PR_OPEN_SIGNED {} {} {} {} {} {} {} {} {} {} {} {}",
            repo,
            timestamp,
            source_ref,
            target_ref,
            encode_text_arg(title),
            encode_text_arg(body),
            encode_label_list_arg(&event.payload.labels),
            hex(self.certificate.label.as_bytes()),
            hex(&self.certificate.account_verifying_key),
            hex(&self.certificate.device_verifying_key),
            hex(&self.certificate.signature),
            hex(&event_signature)
        ))
    }
}

fn print_identity_status(identity: &LocalIdentity, mode: &str) {
    println!("GitMesh authentication: {mode}");
    println!(
        "Account: {}",
        short_id(identity.certificate.account_id.as_cid())
    );
    println!("Account CID: {}", identity.certificate.account_id.as_cid());
    println!(
        "Device: {}",
        short_id(identity.certificate.device_id.as_cid())
    );
    println!("Device CID: {}", identity.certificate.device_id.as_cid());
    println!("Device label: {}", identity.label);
    println!("Device certificate: verified");
    println!(
        "Identity file: {}",
        identity_path().unwrap_or_default().display()
    );
}

fn identity_path() -> Result<PathBuf, GmError> {
    if let Some(path) = std::env::var_os("GITMESH_IDENTITY") {
        return Ok(path.into());
    }
    let home = std::env::var_os("HOME")
        .ok_or_else(|| GmError::InvalidArguments("HOME is not set".to_string()))?;
    Ok(PathBuf::from(home).join(".gitmesh").join("identity-v0.tsv"))
}

fn read_identity_field<'a>(
    lines: &mut impl Iterator<Item = &'a str>,
    expected_name: &str,
) -> Result<String, GmError> {
    let line = lines.next().ok_or(GmError::IdentityStoreCorrupt)?;
    let (name, value) = line.split_once('\t').ok_or(GmError::IdentityStoreCorrupt)?;
    if name != expected_name || value.is_empty() {
        return Err(GmError::IdentityStoreCorrupt);
    }
    Ok(value.to_string())
}

fn decode_fixed_hex<const N: usize>(value: &str) -> Result<[u8; N], GmError> {
    let bytes = decode_hex(value)?;
    bytes
        .try_into()
        .map_err(|_| GmError::InvalidArguments(format!("expected {N} bytes encoded as hex")))
}

#[cfg(unix)]
fn set_owner_only_permissions(path: &Path) -> Result<(), GmError> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_owner_only_permissions(_path: &Path) -> Result<(), GmError> {
    Ok(())
}

fn repo(args: &[String]) -> Result<(), GmError> {
    match args.first().map(String::as_str) {
        Some("view") | None => {
            let repo_name = args.get(1).map_or("farzeen/gitmesh", String::as_str);
            let store = LocalRepoStore::load()?;
            let repo = store
                .find(repo_name)
                .cloned()
                .unwrap_or_else(|| LocalRepo::sample(repo_name));
            print_repo(&repo);
            Ok(())
        }
        Some("clone") => {
            let url = args.get(1).ok_or_else(|| {
                GmError::InvalidArguments("repo clone requires a URL".to_string())
            })?;
            let checkout_dir = args
                .get(2)
                .map(PathBuf::from)
                .unwrap_or_else(|| default_clone_dir_from_url(url));
            clone_from_daemon(default_socket_path(), url, checkout_dir)?;
            Ok(())
        }
        Some("materialize") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let bare_dir = args.get(2).ok_or_else(|| {
                GmError::InvalidArguments(
                    "repo materialize requires an output bare repository path".to_string(),
                )
            })?;
            materialize_bare_repository(socket_path, PathBuf::from(bare_dir))?;
            Ok(())
        }
        Some("create") => {
            let options = RepoCreateOptions::parse(&args[1..])?;
            let mut store = LocalRepoStore::load()?;
            if store.find(&options.name).is_some() {
                return Err(GmError::AlreadyExists(options.name));
            }
            let repo = LocalRepo {
                name: options.name,
                visibility: options.visibility,
                default_branch: "main".to_string(),
                description: options.description,
            };
            store.insert(repo.clone());
            store.save()?;
            println!("Created repository {}", repo.name);
            println!("Visibility: {}", repo.visibility.as_str());
            println!("Remote: gitmesh://{}", repo.name);
            match register_repo_with_daemon(default_socket_path(), &repo) {
                Ok(response) => println!("Daemon registration: {response}"),
                Err(err) => println!("Daemon registration: pending ({err})"),
            }
            Ok(())
        }
        Some(command) => Err(GmError::UnknownCommand(format!("repo {command}"))),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LocalRepo {
    name: String,
    visibility: RepoVisibility,
    default_branch: String,
    description: String,
}

impl LocalRepo {
    fn sample(repo_name: &str) -> Self {
        Self {
            name: repo_name.to_string(),
            visibility: RepoVisibility::Private,
            default_branch: "main".to_string(),
            description: "decentralized Git hosting prototype".to_string(),
        }
    }

    fn encode(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}",
            self.name,
            self.visibility.as_str(),
            self.default_branch,
            self.description
        )
    }

    fn decode(line: &str) -> Result<Self, GmError> {
        let parts = line.split('\t').collect::<Vec<_>>();
        if parts.len() != 4 {
            return Err(GmError::StateCorrupt);
        }
        Ok(Self {
            name: parts[0].to_string(),
            visibility: RepoVisibility::parse(parts[1])?,
            default_branch: parts[2].to_string(),
            description: parts[3].to_string(),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RepoVisibility {
    Public,
    Private,
}

impl RepoVisibility {
    fn parse(value: &str) -> Result<Self, GmError> {
        match value {
            "public" => Ok(Self::Public),
            "private" => Ok(Self::Private),
            _ => Err(GmError::InvalidArguments(format!(
                "unknown visibility '{value}'"
            ))),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RepoCreateOptions {
    name: String,
    visibility: RepoVisibility,
    description: String,
}

impl RepoCreateOptions {
    fn parse(args: &[String]) -> Result<Self, GmError> {
        let mut name = None;
        let mut visibility = RepoVisibility::Private;
        let mut description = "GitMesh repository".to_string();
        let mut index = 0;

        while index < args.len() {
            match args[index].as_str() {
                "--public" => visibility = RepoVisibility::Public,
                "--private" => visibility = RepoVisibility::Private,
                "--description" | "-d" => {
                    index += 1;
                    description = args.get(index).cloned().ok_or_else(|| {
                        GmError::InvalidArguments("--description requires a value".to_string())
                    })?;
                }
                value if value.starts_with('-') => {
                    return Err(GmError::InvalidArguments(format!(
                        "unknown repo create option '{value}'"
                    )));
                }
                value => {
                    if name.replace(value.to_string()).is_some() {
                        return Err(GmError::InvalidArguments(
                            "repo create accepts one repository name".to_string(),
                        ));
                    }
                }
            }
            index += 1;
        }

        let name = name.unwrap_or_else(|| "new-repository".to_string());
        validate_repo_name(&name)?;
        validate_state_field(&description)?;

        Ok(Self {
            name,
            visibility,
            description,
        })
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct LocalRepoStore {
    repos: Vec<LocalRepo>,
}

impl LocalRepoStore {
    fn load() -> Result<Self, GmError> {
        let path = state_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let bytes = fs::read_to_string(path)?;
        let mut lines = bytes.lines();
        if lines.next() != Some("gitmesh-gm-state-v0") {
            return Err(GmError::StateCorrupt);
        }
        let repos = lines
            .filter(|line| !line.trim().is_empty())
            .map(LocalRepo::decode)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { repos })
    }

    fn find(&self, name: &str) -> Option<&LocalRepo> {
        self.repos.iter().find(|repo| repo.name == name)
    }

    fn insert(&mut self, repo: LocalRepo) {
        self.repos.push(repo);
        self.repos.sort_by(|left, right| left.name.cmp(&right.name));
    }

    fn save(&self) -> Result<(), GmError> {
        let path = state_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out = String::from("gitmesh-gm-state-v0\n");
        for repo in &self.repos {
            out.push_str(&repo.encode());
            out.push('\n');
        }
        fs::write(path, out)?;
        Ok(())
    }
}

fn print_repo(repo: &LocalRepo) {
    println!("{}", repo.name);
    println!("Description: {}", repo.description);
    println!("Visibility: {}", repo.visibility.as_str());
    println!("Default branch: {}", repo.default_branch);
    println!("Remote: gitmesh://{}", repo.name);
    println!("Local manifest: persisted");
    println!("Daemon registration: available through gitmeshd account repository registry");
}

fn register_repo_with_daemon(socket_path: PathBuf, repo: &LocalRepo) -> Result<String, GmError> {
    let (owner, _) = split_repo_name(&repo.name)?;
    let identity = LocalIdentity::load_or_create_default()?;
    let account_command = format!(
        "ACCOUNT_CREATE {owner} {} {} - -",
        identity.certificate.account_id.as_cid(),
        encode_text_arg(owner)
    );
    let account_response = request_unix_socket(&socket_path, &account_command)?;
    accept_daemon_response(
        &account_response,
        &["username already exists", "account already exists"],
    )?;
    let response = request_unix_socket(socket_path, &repo_register_command(repo)?)?;
    accept_daemon_response(&response, &[])
}

fn accept_daemon_response(response: &str, tolerated_errors: &[&str]) -> Result<String, GmError> {
    if let Some(message) = response.strip_prefix("OK ") {
        Ok(message.to_string())
    } else if let Some(message) = response.strip_prefix("ERR ") {
        if tolerated_errors
            .iter()
            .any(|tolerated| message.contains(tolerated))
        {
            Ok(message.to_string())
        } else {
            Err(GmError::DaemonResponse(message.to_string()))
        }
    } else {
        Err(GmError::DaemonResponse(response.to_string()))
    }
}

fn repo_register_command(repo: &LocalRepo) -> Result<String, GmError> {
    let (owner, name) = split_repo_name(&repo.name)?;
    Ok(format!(
        "REPO_REGISTER {owner} {name} {} {}",
        repo_id_for_name(&repo.name)?,
        repo.visibility.as_str()
    ))
}

fn split_repo_name(value: &str) -> Result<(&str, &str), GmError> {
    validate_repo_name(value)?;
    if let Some((owner, name)) = value.split_once('/') {
        Ok((owner, name))
    } else {
        Ok(("local", value))
    }
}

fn repo_id_for_name(value: &str) -> Result<String, GmError> {
    validate_repo_name(value)?;
    Ok(format!("repo:{value}"))
}

fn state_path() -> Result<PathBuf, GmError> {
    if let Some(path) = std::env::var_os("GITMESH_GM_STATE") {
        return Ok(path.into());
    }
    let home = std::env::var_os("HOME")
        .ok_or_else(|| GmError::InvalidArguments("HOME is not set".to_string()))?;
    Ok(PathBuf::from(home).join(".gitmesh").join("gm-state.tsv"))
}

fn validate_repo_name(value: &str) -> Result<(), GmError> {
    let parts = value.split('/').collect::<Vec<_>>();
    if parts.is_empty()
        || parts.len() > 2
        || parts
            .iter()
            .any(|part| part.is_empty() || part.starts_with('.') || part.ends_with('.'))
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
    {
        return Err(GmError::InvalidArguments(
            "repository name must be owner/repo or repo using ASCII letters, digits, '.', '-', '_'"
                .to_string(),
        ));
    }
    validate_state_field(value)
}

fn validate_state_field(value: &str) -> Result<(), GmError> {
    if value.contains('\n') || value.contains('\r') || value.contains('\t') {
        return Err(GmError::InvalidArguments(
            "values cannot contain tabs or newlines".to_string(),
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ListedRef {
    name: String,
    oid: GitSha1Oid,
}

fn materialize_bare_repository(socket_path: PathBuf, bare_dir: PathBuf) -> Result<(), GmError> {
    ensure_bare_repository(&bare_dir)?;

    let pack_response = request_unix_socket(&socket_path, "PACK_GET all")?;
    if !pack_response.starts_with("OK ") {
        return Err(GmError::DaemonResponse(pack_response));
    }
    let object_count = response_field(&pack_response, "objects")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    if object_count > 0 {
        let pack_hex = response_field(&pack_response, "pack_hex")
            .ok_or_else(|| GmError::DaemonResponse(pack_response.clone()))?;
        let pack = decode_hex(pack_hex)?;
        let pack_dir = bare_dir.join("objects").join("pack");
        fs::create_dir_all(&pack_dir)?;
        let pack_path = pack_dir.join("gitmesh-materialized.pack");
        fs::write(&pack_path, &pack)?;
        run_git(
            ["--git-dir"],
            &bare_dir,
            ["index-pack", "--strict"],
            &pack_path,
        )?;
    }

    let refs_response = request_unix_socket(&socket_path, "REF_LIST")?;
    let refs = parse_ref_list_response(&refs_response)?;
    for listed_ref in &refs {
        run_git_update_ref(&bare_dir, &listed_ref.name, listed_ref.oid)?;
    }
    if refs
        .iter()
        .any(|listed_ref| listed_ref.name == "refs/heads/main")
    {
        run_git_symbolic_ref(&bare_dir, "HEAD", "refs/heads/main")?;
    }

    println!(
        "Materialized {} objects and {} refs into {}",
        object_count,
        refs.len(),
        bare_dir.display()
    );
    Ok(())
}

fn clone_from_daemon(
    socket_path: PathBuf,
    url: &str,
    checkout_dir: PathBuf,
) -> Result<(), GmError> {
    validate_gitmesh_url(url)?;
    let bare_dir = cached_bare_repo_path(url)?;
    materialize_bare_repository(socket_path, bare_dir.clone())?;
    run_git_clone(&bare_dir, &checkout_dir)?;
    println!("Cloned {url} into {}", checkout_dir.display());
    Ok(())
}

fn validate_gitmesh_url(url: &str) -> Result<(), GmError> {
    let Some(name) = url.strip_prefix("gitmesh://") else {
        return Err(GmError::InvalidArguments(
            "repo clone requires a gitmesh:// URL".to_string(),
        ));
    };
    validate_repo_name(name)
}

fn default_clone_dir_from_url(url: &str) -> PathBuf {
    let name = url
        .strip_prefix("gitmesh://")
        .unwrap_or(url)
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("gitmesh-repository");
    PathBuf::from(name)
}

fn cached_bare_repo_path(url: &str) -> Result<PathBuf, GmError> {
    let repo = url.strip_prefix("gitmesh://").unwrap_or(url);
    let safe_repo = repo
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
                byte as char
            } else {
                '-'
            }
        })
        .collect::<String>();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| GmError::InvalidArguments("system time is before UNIX_EPOCH".to_string()))?
        .as_nanos();
    Ok(std::env::temp_dir().join(format!(
        "gitmesh-{safe_repo}-{}-{now}.git",
        std::process::id()
    )))
}

fn ensure_bare_repository(bare_dir: &Path) -> Result<(), GmError> {
    if !bare_dir.join("HEAD").exists() {
        run_git_init_bare(bare_dir)?;
        return Ok(());
    }
    let output = Command::new("git")
        .arg("--git-dir")
        .arg(bare_dir)
        .arg("rev-parse")
        .arg("--is-bare-repository")
        .output()?;
    if output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true" {
        Ok(())
    } else {
        Err(GmError::InvalidArguments(format!(
            "{} is not a bare Git repository",
            bare_dir.display()
        )))
    }
}

fn run_git_clone(bare_dir: &Path, checkout_dir: &Path) -> Result<(), GmError> {
    let output = Command::new("git")
        .arg("clone")
        .arg(bare_dir)
        .arg(checkout_dir)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(GmError::GitCommandFailed(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

fn run_git_init_bare(bare_dir: &Path) -> Result<(), GmError> {
    let output = Command::new("git")
        .arg("init")
        .arg("--bare")
        .arg(bare_dir)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(GmError::GitCommandFailed(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

fn run_git<const P: usize, const A: usize>(
    prefix: [&str; P],
    git_dir: &Path,
    args: [&str; A],
    pack_path: &Path,
) -> Result<(), GmError> {
    let output = Command::new("git")
        .args(prefix)
        .arg(git_dir)
        .args(args)
        .arg(pack_path)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(GmError::GitCommandFailed(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

fn run_git_update_ref(bare_dir: &Path, ref_name: &str, oid: GitSha1Oid) -> Result<(), GmError> {
    let output = Command::new("git")
        .arg("--git-dir")
        .arg(bare_dir)
        .arg("update-ref")
        .arg(ref_name)
        .arg(oid.to_string())
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(GmError::GitCommandFailed(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

fn run_git_symbolic_ref(bare_dir: &Path, name: &str, target: &str) -> Result<(), GmError> {
    let output = Command::new("git")
        .arg("--git-dir")
        .arg(bare_dir)
        .arg("symbolic-ref")
        .arg(name)
        .arg(target)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(GmError::GitCommandFailed(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

fn parse_ref_list_response(response: &str) -> Result<Vec<ListedRef>, GmError> {
    if !response.starts_with("OK ") {
        return Err(GmError::DaemonResponse(response.to_string()));
    }
    let refs = response
        .strip_prefix("OK refs=")
        .ok_or_else(|| GmError::DaemonResponse(response.to_string()))?;
    if refs == "none" {
        return Ok(Vec::new());
    }
    refs.split(',')
        .map(|entry| {
            let (name, oid) = entry
                .split_once(':')
                .ok_or_else(|| GmError::DaemonResponse(response.to_string()))?;
            Ok(ListedRef {
                name: name.to_string(),
                oid: GitSha1Oid::from_str(oid)?,
            })
        })
        .collect()
}

fn response_field<'a>(response: &'a str, name: &str) -> Option<&'a str> {
    response
        .split_whitespace()
        .find_map(|part| part.strip_prefix(&format!("{name}=")))
}

fn load_issue_summaries() -> Vec<gitmesh_collaboration::IssueSummary> {
    request_unix_socket(default_socket_path(), "ISSUE_LIST farzeen/gitmesh")
        .ok()
        .and_then(|response| parse_issue_list_response(&response).ok())
        .filter(|issues| !issues.is_empty())
        .unwrap_or_else(sample_issues)
}

fn load_pull_request_summaries() -> Vec<gitmesh_collaboration::PullRequestSummary> {
    request_unix_socket(default_socket_path(), "PR_LIST farzeen/gitmesh")
        .ok()
        .and_then(|response| parse_pr_list_response(&response).ok())
        .filter(|pull_requests| !pull_requests.is_empty())
        .unwrap_or_else(sample_pull_requests)
}

fn parse_issue_list_response(
    response: &str,
) -> Result<Vec<gitmesh_collaboration::IssueSummary>, GmError> {
    let value = response
        .strip_prefix("OK ")
        .and_then(|_| response_field(response, "issues"))
        .ok_or_else(|| GmError::DaemonResponse(response.to_string()))?;
    if value == "none" {
        return Ok(Vec::new());
    }
    value
        .split('|')
        .map(|entry| {
            let parts = entry.split(';').collect::<Vec<_>>();
            if parts.len() != 5 {
                return Err(GmError::DaemonResponse(response.to_string()));
            }
            Ok(gitmesh_collaboration::IssueSummary {
                number: parse_number(parts[0], "issue")?,
                title: decode_hex_text(parts[1])?,
                actor: parts[2].to_string(),
                labels: decode_hex_text_list(parts[3])?,
                event_id: parse_protocol_cid_digest(parts[4])?,
            })
        })
        .collect()
}

fn parse_pr_list_response(
    response: &str,
) -> Result<Vec<gitmesh_collaboration::PullRequestSummary>, GmError> {
    let value = response
        .strip_prefix("OK ")
        .and_then(|_| response_field(response, "prs"))
        .ok_or_else(|| GmError::DaemonResponse(response.to_string()))?;
    if value == "none" {
        return Ok(Vec::new());
    }
    value
        .split('|')
        .map(|entry| {
            let parts = entry.split(';').collect::<Vec<_>>();
            if parts.len() != 7 {
                return Err(GmError::DaemonResponse(response.to_string()));
            }
            Ok(gitmesh_collaboration::PullRequestSummary {
                number: parse_number(parts[0], "pull request")?,
                title: decode_hex_text(parts[1])?,
                actor: parts[2].to_string(),
                source_ref: parts[3].to_string(),
                target_ref: parts[4].to_string(),
                labels: decode_hex_text_list(parts[5])?,
                event_id: parse_protocol_cid_digest(parts[6])?,
            })
        })
        .collect()
}

fn parse_protocol_cid_digest(value: &str) -> Result<Cid, GmError> {
    Ok(Cid::from_digest(
        CidKind::ProtocolObject,
        HashAlgorithm::Blake3_256,
        decode_fixed_hex::<32>(value)?,
    ))
}

fn decode_hex_text(value: &str) -> Result<String, GmError> {
    String::from_utf8(decode_hex(value)?)
        .map_err(|_| GmError::InvalidArguments("invalid UTF-8 text field".to_string()))
}

fn decode_hex_text_list(value: &str) -> Result<Vec<String>, GmError> {
    if value == "-" {
        return Ok(Vec::new());
    }
    value.split(',').map(decode_hex_text).collect()
}

fn encode_label_arg(value: Option<&String>) -> Result<String, GmError> {
    let Some(value) = value else {
        return Ok("-".to_string());
    };
    if value.trim().is_empty() {
        return Ok("-".to_string());
    }
    value
        .split(',')
        .map(|label| {
            validate_state_field(label.trim())?;
            Ok(encode_text_arg(label.trim()))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|labels| labels.join(","))
}

fn parse_label_values(value: Option<&String>) -> Result<Vec<String>, GmError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }
    value
        .split(',')
        .map(|label| {
            let label = label.trim();
            validate_state_field(label)?;
            Ok(label.to_string())
        })
        .collect()
}

fn encode_label_list_arg(labels: &[String]) -> String {
    if labels.is_empty() {
        return "-".to_string();
    }
    labels
        .iter()
        .map(|label| encode_text_arg(label))
        .collect::<Vec<_>>()
        .join(",")
}

fn now_unix() -> Result<u64, GmError> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| GmError::InvalidArguments("system time is before UNIX_EPOCH".to_string()))?
        .as_secs())
}

fn issue(args: &[String]) -> Result<(), GmError> {
    match args.first().map(String::as_str) {
        Some("list") | None => {
            let issues = load_issue_summaries();
            println!("Showing {} open issues in farzeen/gitmesh", issues.len());
            for issue in issues {
                println!(
                    "#{:<3} {:<48} {}",
                    issue.number,
                    issue.title,
                    issue.labels.join(", ")
                );
                println!(
                    "      opened by {}  event {}",
                    issue.actor,
                    short_cid(issue.event_id)
                );
            }
            Ok(())
        }
        Some("view") => {
            let id = args.get(1).ok_or_else(|| {
                GmError::InvalidArguments("issue view requires an id".to_string())
            })?;
            let number = parse_number(id, "issue")?;
            let issue = load_issue_summaries()
                .into_iter()
                .find(|issue| issue.number == number)
                .ok_or(GmError::NotFound {
                    resource: "issue",
                    number,
                })?;
            println!("#{} {}", issue.number, issue.title);
            println!("State: open");
            println!("Author: {}", issue.actor);
            println!("Labels: {}", issue.labels.join(", "));
            println!("Event: {}", issue.event_id);
            Ok(())
        }
        Some("create") => {
            let signed = args.iter().any(|arg| arg == "--signed");
            let create_args = args
                .iter()
                .skip(1)
                .filter(|arg| arg.as_str() != "--signed")
                .collect::<Vec<_>>();
            let title = create_args.first().ok_or_else(|| {
                GmError::InvalidArguments("issue create requires a title".to_string())
            })?;
            let body = create_args.get(1).map_or("-", |value| value.as_str());
            validate_state_field(title)?;
            validate_state_field(body)?;
            let command = if signed {
                LocalIdentity::load_or_create_default()?.signed_issue_open_command(
                    "farzeen/gitmesh",
                    title,
                    body,
                    parse_label_values(create_args.get(2).copied())?,
                )?
            } else {
                format!(
                    "ISSUE_OPEN farzeen/gitmesh farzeen {} {} {}",
                    encode_text_arg(title),
                    encode_text_arg(body),
                    encode_label_arg(create_args.get(2).copied())?
                )
            };
            println!("{}", request_unix_socket(default_socket_path(), &command)?);
            Ok(())
        }
        Some(command) => Err(GmError::UnknownCommand(format!("issue {command}"))),
    }
}

fn pr(args: &[String]) -> Result<(), GmError> {
    match args.first().map(String::as_str) {
        Some("list") | None => {
            let pull_requests = load_pull_request_summaries();
            println!(
                "Showing {} open pull requests in farzeen/gitmesh",
                pull_requests.len()
            );
            for pr in pull_requests {
                println!(
                    "#{:<3} {:<48} {} -> {}",
                    pr.number, pr.title, pr.source_ref, pr.target_ref
                );
                println!(
                    "      opened by {}  labels {}  event {}",
                    pr.actor,
                    pr.labels.join(", "),
                    short_cid(pr.event_id)
                );
            }
            Ok(())
        }
        Some("status") => {
            let pull_requests = load_pull_request_summaries();
            println!("Relevant pull requests in farzeen/gitmesh");
            println!("Current branch: refs/heads/collaboration-cli");
            if let Some(pr) = pull_requests
                .iter()
                .find(|pr| pr.source_ref == "refs/heads/collaboration-cli")
            {
                println!("#{} {}", pr.number, pr.title);
                println!("Target: {}", pr.target_ref);
                println!("Checks: pending local protocol integration");
            } else {
                println!("No pull request found for the current branch.");
            }
            Ok(())
        }
        Some("view") => {
            let id = args
                .get(1)
                .ok_or_else(|| GmError::InvalidArguments("pr view requires an id".to_string()))?;
            let number = parse_number(id, "pull request")?;
            let pr = load_pull_request_summaries()
                .into_iter()
                .find(|pr| pr.number == number)
                .ok_or(GmError::NotFound {
                    resource: "pull request",
                    number,
                })?;
            println!("#{} {}", pr.number, pr.title);
            println!("State: open");
            println!("Author: {}", pr.actor);
            println!("Refs: {} -> {}", pr.source_ref, pr.target_ref);
            println!("Labels: {}", pr.labels.join(", "));
            println!("Event: {}", pr.event_id);
            Ok(())
        }
        Some("create") => {
            let signed = args.iter().any(|arg| arg == "--signed");
            let create_args = args
                .iter()
                .skip(1)
                .filter(|arg| arg.as_str() != "--signed")
                .collect::<Vec<_>>();
            let title = create_args.first().ok_or_else(|| {
                GmError::InvalidArguments("pr create requires a title".to_string())
            })?;
            let source = create_args.get(1).ok_or_else(|| {
                GmError::InvalidArguments("pr create requires a source ref".to_string())
            })?;
            let target = create_args
                .get(2)
                .map_or("refs/heads/main", |value| value.as_str());
            let body = create_args.get(3).map_or("-", |value| value.as_str());
            validate_state_field(title)?;
            validate_state_field(source)?;
            validate_state_field(target)?;
            validate_state_field(body)?;
            let command = if signed {
                LocalIdentity::load_or_create_default()?.signed_pr_open_command(
                    "farzeen/gitmesh",
                    source,
                    target,
                    title,
                    body,
                    parse_label_values(create_args.get(4).copied())?,
                )?
            } else {
                format!(
                    "PR_OPEN farzeen/gitmesh farzeen {} {} {} {} {}",
                    source,
                    target,
                    encode_text_arg(title),
                    encode_text_arg(body),
                    encode_label_arg(create_args.get(4).copied())?
                )
            };
            println!("{}", request_unix_socket(default_socket_path(), &command)?);
            Ok(())
        }
        Some(command) => Err(GmError::UnknownCommand(format!("pr {command}"))),
    }
}

fn daemon(args: &[String]) -> Result<(), GmError> {
    match args.first().map(String::as_str) {
        Some("ping") | None => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            println!("{}", request_unix_socket(socket_path, "PING")?);
            Ok(())
        }
        Some("proof") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let payload = args.iter().skip(2).cloned().collect::<Vec<_>>().join(" ");
            let command = if payload.is_empty() {
                "V0_PROOF".to_string()
            } else {
                format!("V0_PROOF {payload}")
            };
            println!("{}", request_unix_socket(socket_path, &command)?);
            Ok(())
        }
        Some("network-proof") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let payload = args.iter().skip(2).cloned().collect::<Vec<_>>().join(" ");
            let command = if payload.is_empty() {
                "NETWORK_REPAIR_PROOF".to_string()
            } else {
                format!("NETWORK_REPAIR_PROOF {payload}")
            };
            println!("{}", request_unix_socket(socket_path, &command)?);
            Ok(())
        }
        Some("network-status") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            println!("{}", request_unix_socket(socket_path, "NETWORK_STATUS")?);
            Ok(())
        }
        Some("network-listen") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let address = args.get(2).ok_or_else(|| {
                GmError::InvalidArguments("daemon network-listen requires a multiaddr".to_string())
            })?;
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("NETWORK_LISTEN {address}"))?
            );
            Ok(())
        }
        Some("network-bootstrap") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let rest = args.iter().skip(2).cloned().collect::<Vec<_>>();
            if rest.len() != 4 {
                return Err(GmError::InvalidArguments(
                    "daemon network-bootstrap requires <peer-id> <operator-id> <region> <multiaddr>"
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
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let rest = args.iter().skip(2).cloned().collect::<Vec<_>>();
            if rest.len() != 6 {
                return Err(GmError::InvalidArguments(
                    "daemon network-peer-add requires <peer-id> <operator-id> <roles-csv> <region> <protocols-csv> <addresses-csv|->"
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
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            println!("{}", request_unix_socket(socket_path, "NETWORK_PEER_LIST")?);
            Ok(())
        }
        Some(command) => Err(GmError::UnknownCommand(format!("daemon {command}"))),
    }
}

fn policy(args: &[String]) -> Result<(), GmError> {
    match args.first().map(String::as_str) {
        Some("show") | None => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            println!("{}", request_unix_socket(socket_path, "POLICY_SHOW")?);
            Ok(())
        }
        Some("require-signed") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let value = args.get(2).ok_or_else(|| {
                GmError::InvalidArguments(
                    "policy require-signed requires true or false".to_string(),
                )
            })?;
            if !matches!(value.as_str(), "true" | "false") {
                return Err(GmError::InvalidArguments(
                    "policy require-signed requires true or false".to_string(),
                ));
            }
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("POLICY_SET_REQUIRE_SIGNED {value}"))?
            );
            Ok(())
        }
        Some("grant-writer") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let account_id = args.get(2).ok_or_else(|| {
                GmError::InvalidArguments("policy grant-writer requires an account CID".to_string())
            })?;
            println!(
                "{}",
                request_unix_socket(
                    socket_path,
                    &format!("POLICY_GRANT_WRITER_ACCOUNT {account_id}")
                )?
            );
            Ok(())
        }
        Some("grant-force") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let account_id = args.get(2).ok_or_else(|| {
                GmError::InvalidArguments("policy grant-force requires an account CID".to_string())
            })?;
            println!(
                "{}",
                request_unix_socket(
                    socket_path,
                    &format!("POLICY_GRANT_FORCE_ACCOUNT {account_id}")
                )?
            );
            Ok(())
        }
        Some("protect") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let ref_name = args.get(2).ok_or_else(|| {
                GmError::InvalidArguments("policy protect requires a ref name".to_string())
            })?;
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("POLICY_PROTECT_REF {ref_name}"))?
            );
            Ok(())
        }
        Some(command) => Err(GmError::UnknownCommand(format!("policy {command}"))),
    }
}

fn refs(args: &[String]) -> Result<(), GmError> {
    match args.first().map(String::as_str) {
        Some("list") | None => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            println!("{}", request_unix_socket(socket_path, "REF_LIST")?);
            Ok(())
        }
        Some("get") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let ref_name = args
                .get(2)
                .ok_or_else(|| GmError::InvalidArguments("ref get requires a ref".to_string()))?;
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("REF_GET {ref_name}"))?
            );
            Ok(())
        }
        Some("update") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let rest = args.iter().skip(2).cloned().collect::<Vec<_>>();
            if rest.len() != 5 {
                return Err(GmError::InvalidArguments(
                    "ref update requires <tx> <ref> <expected|none> <new|delete> <signer>"
                        .to_string(),
                ));
            }
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("REF_UPDATE {}", rest.join(" ")))?
            );
            Ok(())
        }
        Some("checkpoint") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            println!("{}", request_unix_socket(socket_path, "REF_CHECKPOINT")?);
            Ok(())
        }
        Some("signed-update") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let rest = args.iter().skip(2).cloned().collect::<Vec<_>>();
            if rest.len() != 4 {
                return Err(GmError::InvalidArguments(
                    "ref signed-update requires <tx> <ref> <expected|none> <new|delete>"
                        .to_string(),
                ));
            }
            let identity = LocalIdentity::load_or_create_default()?;
            println!(
                "{}",
                request_unix_socket(socket_path, &identity.signed_ref_update_command(&rest)?)?
            );
            Ok(())
        }
        Some("signed-update-dev") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let rest = args.iter().skip(2).cloned().collect::<Vec<_>>();
            if rest.len() != 4 {
                return Err(GmError::InvalidArguments(
                    "ref signed-update-dev requires <tx> <ref> <expected|none> <new|delete>"
                        .to_string(),
                ));
            }
            let identity = LocalIdentity::create("gm-dev-device".to_string());
            println!(
                "{}",
                request_unix_socket(socket_path, &identity.signed_ref_update_command(&rest)?)?
            );
            Ok(())
        }
        Some(command) => Err(GmError::UnknownCommand(format!("ref {command}"))),
    }
}

fn object(args: &[String]) -> Result<(), GmError> {
    match args.first().map(String::as_str) {
        Some("put") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let kind = args.get(2).ok_or_else(|| {
                GmError::InvalidArguments("object put requires a kind".to_string())
            })?;
            let payload_hex = args.get(3).ok_or_else(|| {
                GmError::InvalidArguments("object put requires a hex payload".to_string())
            })?;
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("OBJECT_PUT {kind} {payload_hex}"))?
            );
            Ok(())
        }
        Some("get") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let oid = args.get(2).ok_or_else(|| {
                GmError::InvalidArguments("object get requires an oid".to_string())
            })?;
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("OBJECT_GET {oid}"))?
            );
            Ok(())
        }
        Some("list") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            println!("{}", request_unix_socket(socket_path, "OBJECT_LIST")?);
            Ok(())
        }
        Some("audit") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let target = args.get(2).map_or("all", String::as_str);
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("OBJECT_AUDIT {target}"))?
            );
            Ok(())
        }
        Some("repair") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let target = args.get(2).map_or("all", String::as_str);
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("OBJECT_REPAIR {target}"))?
            );
            Ok(())
        }
        Some("import-loose") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let git_dir = args.get(2).ok_or_else(|| {
                GmError::InvalidArguments(
                    "object import-loose requires a .git directory".to_string(),
                )
            })?;
            import_loose_objects(socket_path, PathBuf::from(git_dir))?;
            Ok(())
        }
        Some("import-pack") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let pack_path = args.get(2).ok_or_else(|| {
                GmError::InvalidArguments(
                    "object import-pack requires a .pack file path".to_string(),
                )
            })?;
            import_pack_objects(socket_path, PathBuf::from(pack_path))?;
            Ok(())
        }
        Some("export-pack") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let pack_path = args.get(2).ok_or_else(|| {
                GmError::InvalidArguments(
                    "object export-pack requires an output .pack file path".to_string(),
                )
            })?;
            export_pack_objects(socket_path, PathBuf::from(pack_path))?;
            Ok(())
        }
        Some("status") | None => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            println!("{}", request_unix_socket(socket_path, "REPO_STATUS")?);
            Ok(())
        }
        Some(command) => Err(GmError::UnknownCommand(format!("object {command}"))),
    }
}

fn key(args: &[String]) -> Result<(), GmError> {
    match args.first().map(String::as_str) {
        Some("grant-self") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let repo_id = args.get(2).ok_or_else(|| {
                GmError::InvalidArguments("key grant-self requires a repo id".to_string())
            })?;
            let epoch = args.get(3).ok_or_else(|| {
                GmError::InvalidArguments("key grant-self requires an epoch".to_string())
            })?;
            let epoch = parse_epoch(epoch)?;
            validate_protocol_token(repo_id, "repo id")?;
            let identity = LocalIdentity::load_or_create_default()?;
            let repo_key = RepoContentKey::generate();
            let grant = identity.account.grant_repo_key_to_device(
                repo_id,
                epoch,
                repo_key,
                &identity.device,
                &identity.certificate,
            )?;
            let grant_id = grant.grant_id();
            let device_id = grant.recipient_device_id.as_cid();
            let response = request_unix_socket(
                socket_path,
                &format!("KEY_GRANT_PUT {}", grant.to_wire_string()?),
            )?;
            if !response.starts_with("OK ") {
                return Err(GmError::DaemonResponse(response));
            }
            println!("{response}");
            println!("Grant: {grant_id}");
            println!("Repo: {repo_id}");
            println!("Epoch: {epoch}");
            println!("Device: {device_id}");
            println!(
                "Note: generated a V0 local repo content key; production should replace this wrapping path with HPKE/X25519 and a real backup flow."
            );
            Ok(())
        }
        Some("list") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let repo_id = args.get(2).ok_or_else(|| {
                GmError::InvalidArguments("key list requires a repo id".to_string())
            })?;
            validate_protocol_token(repo_id, "repo id")?;
            let selector = args.get(3).map_or("latest", String::as_str);
            validate_protocol_token(selector, "epoch selector")?;
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("KEY_GRANT_LIST {repo_id} {selector}"))?
            );
            Ok(())
        }
        Some("revoke-device") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let device_id = args.get(2).ok_or_else(|| {
                GmError::InvalidArguments("key revoke-device requires a device CID".to_string())
            })?;
            let effective_epoch = args.get(3).ok_or_else(|| {
                GmError::InvalidArguments(
                    "key revoke-device requires an effective epoch".to_string(),
                )
            })?;
            validate_protocol_token(device_id, "device CID")?;
            let effective_epoch = parse_epoch(effective_epoch)?;
            println!(
                "{}",
                request_unix_socket(
                    socket_path,
                    &format!("KEY_GRANT_REVOKE_DEVICE {device_id} {effective_epoch}")
                )?
            );
            Ok(())
        }
        Some("status") | None => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let repo_id = args.get(2).map_or("repo:farzeen/gitmesh", String::as_str);
            validate_protocol_token(repo_id, "repo id")?;
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("KEY_GRANT_STATUS {repo_id}"))?
            );
            Ok(())
        }
        Some(command) => Err(GmError::UnknownCommand(format!("key {command}"))),
    }
}

fn account(args: &[String]) -> Result<(), GmError> {
    match args.first().map(String::as_str) {
        Some("create") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let username = args.get(2).ok_or_else(|| {
                GmError::InvalidArguments("account create requires a username".to_string())
            })?;
            validate_account_token(username, "username")?;
            let display_name = args.get(3).map_or(username.as_str(), String::as_str);
            let bio = args.get(4).map_or("", String::as_str);
            let avatar = args.get(5).map_or("", String::as_str);
            validate_state_field(display_name)?;
            validate_state_field(bio)?;
            validate_state_field(avatar)?;
            let identity = LocalIdentity::load_or_create_default()?;
            println!(
                "{}",
                request_unix_socket(
                    socket_path,
                    &format!(
                        "ACCOUNT_CREATE {username} {} {} {} {}",
                        identity.certificate.account_id.as_cid(),
                        encode_text_arg(display_name),
                        encode_text_arg(bio),
                        encode_text_arg(avatar)
                    )
                )?
            );
            Ok(())
        }
        Some("profile") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let username = args.get(2).ok_or_else(|| {
                GmError::InvalidArguments("account profile requires a username".to_string())
            })?;
            validate_account_token(username, "username")?;
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("ACCOUNT_PROFILE {username}"))?
            );
            Ok(())
        }
        Some("update") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let username = args.get(2).ok_or_else(|| {
                GmError::InvalidArguments("account update requires a username".to_string())
            })?;
            validate_account_token(username, "username")?;
            let display_name = args
                .get(3)
                .map_or("keep".to_string(), |value| encode_text_arg_or_keep(value));
            let bio = args
                .get(4)
                .map_or("keep".to_string(), |value| encode_text_arg_or_keep(value));
            let avatar = args
                .get(5)
                .map_or("keep".to_string(), |value| encode_text_arg_or_keep(value));
            println!(
                "{}",
                request_unix_socket(
                    socket_path,
                    &format!("ACCOUNT_UPDATE_PROFILE {username} {display_name} {bio} {avatar}")
                )?
            );
            Ok(())
        }
        Some("register-repo") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let owner = args.get(2).ok_or_else(|| {
                GmError::InvalidArguments("account register-repo requires an owner".to_string())
            })?;
            let name = args.get(3).ok_or_else(|| {
                GmError::InvalidArguments("account register-repo requires a repo name".to_string())
            })?;
            let repo_id = args.get(4).ok_or_else(|| {
                GmError::InvalidArguments("account register-repo requires a repo id".to_string())
            })?;
            let visibility = args.get(5).map_or("private", String::as_str);
            validate_account_token(owner, "owner")?;
            validate_account_token(name, "repo name")?;
            validate_protocol_token(repo_id, "repo id")?;
            if !matches!(visibility, "public" | "private") {
                return Err(GmError::InvalidArguments(
                    "visibility must be public or private".to_string(),
                ));
            }
            println!(
                "{}",
                request_unix_socket(
                    socket_path,
                    &format!("REPO_REGISTER {owner} {name} {repo_id} {visibility}")
                )?
            );
            Ok(())
        }
        Some("repos") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let owner = args.get(2).ok_or_else(|| {
                GmError::InvalidArguments("account repos requires an owner".to_string())
            })?;
            validate_account_token(owner, "owner")?;
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("REPO_LIST {owner}"))?
            );
            Ok(())
        }
        Some("status") | None => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            println!("{}", request_unix_socket(socket_path, "ACCOUNT_STATUS")?);
            Ok(())
        }
        Some(command) => Err(GmError::UnknownCommand(format!("account {command}"))),
    }
}

fn session(args: &[String]) -> Result<(), GmError> {
    match args.first().map(String::as_str) {
        Some("issue") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let username = args.get(2).ok_or_else(|| {
                GmError::InvalidArguments("session issue requires a username".to_string())
            })?;
            let ttl = args.get(3).map_or("86400", String::as_str);
            let device_id = args.get(4).map_or("none", String::as_str);
            validate_account_token(username, "username")?;
            validate_protocol_token(ttl, "ttl")?;
            validate_protocol_token(device_id, "device id")?;
            println!(
                "{}",
                request_unix_socket(
                    socket_path,
                    &format!("SESSION_ISSUE {username} {ttl} {device_id}")
                )?
            );
            Ok(())
        }
        Some("auth") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let token = args.get(2).ok_or_else(|| {
                GmError::InvalidArguments("session auth requires a token".to_string())
            })?;
            validate_protocol_token(token, "session token")?;
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("SESSION_AUTH {token}"))?
            );
            Ok(())
        }
        Some("revoke") => {
            let socket_path = args.get(1).map_or_else(default_socket_path, Into::into);
            let session_id = args.get(2).ok_or_else(|| {
                GmError::InvalidArguments("session revoke requires a session id".to_string())
            })?;
            validate_protocol_token(session_id, "session id")?;
            println!(
                "{}",
                request_unix_socket(socket_path, &format!("SESSION_REVOKE {session_id}"))?
            );
            Ok(())
        }
        Some(command) => Err(GmError::UnknownCommand(format!("session {command}"))),
        None => Err(GmError::InvalidArguments(
            "session requires issue, auth, or revoke".to_string(),
        )),
    }
}

fn import_loose_objects(socket_path: PathBuf, git_dir: PathBuf) -> Result<(), GmError> {
    let objects_dir = git_dir.join("objects");
    let mut imported = 0_u64;
    for fanout in fs::read_dir(&objects_dir)? {
        let fanout = fanout?;
        if !fanout.file_type()?.is_dir() {
            continue;
        }
        let fanout_name = fanout.file_name();
        let fanout_name = fanout_name.to_string_lossy();
        if fanout_name.len() != 2 || !fanout_name.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            continue;
        }
        for entry in fs::read_dir(fanout.path())? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.len() != 38 || !name.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                continue;
            }
            let bytes = fs::read(entry.path())?;
            let object = parse_loose_object(&bytes)?;
            let kind = encode_git_kind(object.kind);
            let payload_hex = if object.payload.is_empty() {
                "-".to_string()
            } else {
                hex(&object.payload)
            };
            let response =
                request_unix_socket(&socket_path, &format!("OBJECT_PUT {kind} {payload_hex}"))?;
            if !response.starts_with("OK ") {
                return Err(GmError::DaemonResponse(response));
            }
            imported += 1;
        }
    }
    println!("Imported {imported} loose Git objects");
    println!("{}", request_unix_socket(socket_path, "REPO_STATUS")?);
    Ok(())
}

fn import_pack_objects(socket_path: PathBuf, pack_path: PathBuf) -> Result<(), GmError> {
    let bytes = fs::read(pack_path)?;
    let pack = parse_packfile(&bytes)?;
    let version = pack.version;
    let mut imported = 0_u64;

    for object in pack.objects {
        let kind = encode_git_kind(object.kind);
        let payload_hex = if object.payload.is_empty() {
            "-".to_string()
        } else {
            hex(&object.payload)
        };
        let response =
            request_unix_socket(&socket_path, &format!("OBJECT_PUT {kind} {payload_hex}"))?;
        if !response.starts_with("OK ") {
            return Err(GmError::DaemonResponse(response));
        }
        imported += 1;
    }

    println!("Imported {imported} Git pack objects from pack v{version}");
    println!("{}", request_unix_socket(socket_path, "REPO_STATUS")?);
    Ok(())
}

fn export_pack_objects(socket_path: PathBuf, pack_path: PathBuf) -> Result<(), GmError> {
    let response = request_unix_socket(socket_path, "PACK_GET all")?;
    if !response.starts_with("OK ") {
        return Err(GmError::DaemonResponse(response));
    }
    let pack_hex = response
        .split_whitespace()
        .find_map(|part| part.strip_prefix("pack_hex="))
        .ok_or_else(|| GmError::DaemonResponse(response.clone()))?;
    let pack = decode_hex(pack_hex)?;
    if let Some(parent) = pack_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&pack_path, &pack)?;
    let objects = response
        .split_whitespace()
        .find_map(|part| part.strip_prefix("objects="))
        .unwrap_or("unknown");
    println!(
        "Exported {objects} Git objects to pack {}",
        pack_path.display()
    );
    Ok(())
}

fn encode_git_kind(kind: GitObjectKind) -> &'static str {
    match kind {
        GitObjectKind::Blob => "blob",
        GitObjectKind::Tree => "tree",
        GitObjectKind::Commit => "commit",
        GitObjectKind::Tag => "tag",
    }
}

fn proof(args: &[String]) -> Result<(), GmError> {
    let payload = args.join(" ");
    let command = if payload.is_empty() {
        "V0_PROOF".to_string()
    } else {
        format!("V0_PROOF {payload}")
    };
    println!("{}", request_unix_socket(default_socket_path(), &command)?);
    Ok(())
}

fn parse_optional_oid(value: &str) -> Result<Option<GitSha1Oid>, GmError> {
    if value == "none" {
        Ok(None)
    } else {
        Ok(Some(GitSha1Oid::from_str(value)?))
    }
}

fn parse_number(value: &str, resource: &'static str) -> Result<u64, GmError> {
    value
        .trim_start_matches('#')
        .parse::<u64>()
        .map_err(|_| GmError::InvalidArguments(format!("{resource} id must be a number")))
}

fn parse_epoch(value: &str) -> Result<u64, GmError> {
    let epoch = value
        .parse::<u64>()
        .map_err(|_| GmError::InvalidArguments("epoch must be a positive integer".to_string()))?;
    if epoch == 0 {
        return Err(GmError::InvalidArguments(
            "epoch must be a positive integer".to_string(),
        ));
    }
    Ok(epoch)
}

fn validate_protocol_token(value: &str, name: &str) -> Result<(), GmError> {
    if value.is_empty() || value.contains(char::is_whitespace) {
        return Err(GmError::InvalidArguments(format!(
            "{name} cannot be empty or contain whitespace"
        )));
    }
    Ok(())
}

fn validate_account_token(value: &str, name: &str) -> Result<(), GmError> {
    if value.is_empty()
        || value.contains(char::is_whitespace)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(GmError::InvalidArguments(format!(
            "{name} must be a non-empty ASCII token"
        )));
    }
    Ok(())
}

fn encode_text_arg(value: &str) -> String {
    if value.is_empty() {
        "-".to_string()
    } else {
        hex(value.as_bytes())
    }
}

fn encode_text_arg_or_keep(value: &str) -> String {
    if value == "keep" {
        "keep".to_string()
    } else {
        encode_text_arg(value)
    }
}

fn short_cid(cid: gitmesh_core::Cid) -> String {
    let hex = cid.as_hex();
    hex.chars().take(12).collect()
}

fn print_help() {
    println!("gm - GitMesh CLI");
    println!();
    println!("Commands:");
    println!("  auth init [device-label]");
    println!("  auth status");
    println!("  repo view [owner/repo]");
    println!("  repo clone <gitmesh-url> [directory]");
    println!("  repo create [owner/repo] [--public|--private] [-d description]");
    println!("  repo materialize [socket] <bare-dir>");
    println!(
        "  issue list | issue view <id> | issue create <title> [body] [label,label] [--signed]"
    );
    println!(
        "  pr list | pr status | pr view <id> | pr create <title> <source-ref> [target-ref] [body] [label,label] [--signed]"
    );
    println!("  daemon ping [socket]");
    println!("  daemon proof [socket] [payload...]");
    println!("  daemon network-proof [socket] [payload...]");
    println!("  daemon network-status [socket]");
    println!("  daemon network-listen [socket] <multiaddr>");
    println!("  daemon network-bootstrap [socket] <peer-id> <operator-id> <region> <multiaddr>");
    println!(
        "  daemon network-peer-add [socket] <peer-id> <operator-id> <roles-csv> <region> <protocols-csv> <addresses-csv|->"
    );
    println!("  daemon network-peer-list [socket]");
    println!("  policy show [socket]");
    println!("  policy require-signed [socket] <true|false>");
    println!("  policy grant-writer [socket] <account-cid>");
    println!("  policy grant-force [socket] <account-cid>");
    println!("  policy protect [socket] <ref>");
    println!("  ref get [socket] <ref>");
    println!("  ref list [socket]");
    println!("  ref update [socket] <tx> <ref> <expected|none> <new|delete> <signer>");
    println!("  ref checkpoint [socket]");
    println!("  ref signed-update [socket] <tx> <ref> <expected|none> <new|delete>");
    println!("  ref signed-update-dev [socket] <tx> <ref> <expected|none> <new|delete>");
    println!("  object put [socket] <blob|tree|commit|tag> <hex-payload>");
    println!("  object get [socket] <oid>");
    println!("  object list [socket]");
    println!("  object audit [socket] [all|oid]");
    println!("  object repair [socket] [all|oid]");
    println!("  object import-loose [socket] <git-dir>");
    println!("  object import-pack [socket] <pack-file>");
    println!("  object export-pack [socket] <pack-file>");
    println!("  object status [socket]");
    println!("  key grant-self [socket] <repo-id> <epoch>");
    println!("  key list [socket] <repo-id> [latest|all|epoch]");
    println!("  key revoke-device [socket] <device-cid> <effective-epoch>");
    println!("  key status [socket] [repo-id]");
    println!("  account create [socket] <username> [display-name] [bio] [avatar-uri]");
    println!("  account profile [socket] <username>");
    println!(
        "  account update [socket] <username> [display-name|keep] [bio|keep] [avatar-uri|keep]"
    );
    println!("  account register-repo [socket] <owner> <name> <repo-id> [public|private]");
    println!("  account repos [socket] <owner>");
    println!("  account status [socket]");
    println!("  session issue [socket] <username> [ttl-seconds] [device-id|none]");
    println!("  session auth [socket] <token>");
    println!("  session revoke [socket] <session-id>");
    println!("  proof [payload...]");
}

#[derive(Debug, Error)]
enum GmError {
    #[error("unknown command '{0}'")]
    UnknownCommand(String),
    #[error("invalid arguments: {0}")]
    InvalidArguments(String),
    #[error("repository '{0}' already exists")]
    AlreadyExists(String),
    #[error("local gm state file is corrupt")]
    StateCorrupt,
    #[error("local identity file is corrupt")]
    IdentityStoreCorrupt,
    #[error("{resource} #{number} was not found")]
    NotFound { resource: &'static str, number: u64 },
    #[error(transparent)]
    Daemon(#[from] gitmeshd::DaemonError),
    #[error(transparent)]
    Identity(#[from] gitmesh_identity::IdentityError),
    #[error(transparent)]
    Coordination(#[from] gitmesh_coordination::CoordinationError),
    #[error(transparent)]
    Collaboration(#[from] gitmesh_collaboration::CollaborationError),
    #[error(transparent)]
    Git(#[from] gitmesh_git::GitError),
    #[error("daemon returned an error response: {0}")]
    DaemonResponse(String),
    #[error("git command failed: {0}")]
    GitCommandFailed(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

fn decode_hex(value: &str) -> Result<Vec<u8>, GmError> {
    if !value.len().is_multiple_of(2) {
        return Err(GmError::InvalidArguments("invalid hex payload".to_string()));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|chunk| {
            let high = hex_nibble(chunk[0])
                .ok_or_else(|| GmError::InvalidArguments("invalid hex payload".to_string()))?;
            let low = hex_nibble(chunk[1])
                .ok_or_else(|| GmError::InvalidArguments("invalid hex payload".to_string()))?;
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

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn parses_repo_create_options() {
        let options = RepoCreateOptions::parse(&args(&[
            "farzeen/gitmesh",
            "--public",
            "-d",
            "premium decentralized git",
        ]))
        .unwrap();

        assert_eq!(options.name, "farzeen/gitmesh");
        assert_eq!(options.visibility, RepoVisibility::Public);
        assert_eq!(options.description, "premium decentralized git");
    }

    #[test]
    fn rejects_unsafe_repo_names() {
        let err = RepoCreateOptions::parse(&args(&["bad\tname"])).unwrap_err();

        assert!(matches!(err, GmError::InvalidArguments(_)));
    }

    #[test]
    fn local_repo_round_trips() {
        let repo = LocalRepo {
            name: "farzeen/gitmesh".to_string(),
            visibility: RepoVisibility::Private,
            default_branch: "main".to_string(),
            description: "GitMesh repository".to_string(),
        };

        assert_eq!(LocalRepo::decode(&repo.encode()).unwrap(), repo);
    }

    #[test]
    fn splits_repo_names_for_daemon_registration() {
        assert_eq!(
            split_repo_name("farzeen/gitmesh").unwrap(),
            ("farzeen", "gitmesh")
        );
        assert_eq!(
            split_repo_name("gitmesh-local").unwrap(),
            ("local", "gitmesh-local")
        );
        assert!(split_repo_name("bad/name/extra").is_err());
    }

    #[test]
    fn builds_repo_register_command_for_daemon_registry() {
        let repo = LocalRepo {
            name: "farzeen/gitmesh".to_string(),
            visibility: RepoVisibility::Private,
            default_branch: "main".to_string(),
            description: "GitMesh repository".to_string(),
        };

        assert_eq!(
            repo_register_command(&repo).unwrap(),
            "REPO_REGISTER farzeen gitmesh repo:farzeen/gitmesh private"
        );
        assert_eq!(
            repo_id_for_name("farzeen/gitmesh").unwrap(),
            "repo:farzeen/gitmesh"
        );
    }

    #[test]
    fn accepts_daemon_ok_and_tolerated_idempotent_errors() {
        assert_eq!(
            accept_daemon_response("OK owner=farzeen", &[]).unwrap(),
            "owner=farzeen"
        );
        assert_eq!(
            accept_daemon_response("ERR username already exists", &["username already exists"])
                .unwrap(),
            "username already exists"
        );
        assert!(accept_daemon_response("ERR account not found", &[]).is_err());
        assert!(accept_daemon_response("not protocol", &[]).is_err());
    }

    #[test]
    fn local_identity_round_trips_and_signs_ref_updates() {
        let temp_dir =
            std::env::temp_dir().join(format!("gitmesh-gm-identity-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let path = temp_dir.join("identity.tsv");
        let identity = LocalIdentity::create("test-device".to_string());
        let account_id = identity.certificate.account_id.clone();
        let device_id = identity.certificate.device_id.clone();

        let contents = format!(
            "gitmesh-local-identity-v0\nlabel\ttest-device\naccount_seed\t{}\ndevice_seed\t{}\n",
            hex(&identity.account.seed_bytes()),
            hex(&identity.device.seed_bytes())
        );
        fs::write(&path, contents).unwrap();

        let loaded = LocalIdentity::load_from_path(&path).unwrap();
        assert_eq!(loaded.certificate.account_id, account_id);
        assert_eq!(loaded.certificate.device_id, device_id);
        assert_eq!(loaded.label, "test-device");
        assert!(
            loaded
                .signed_ref_update_command(&args(&["tx1", "refs/heads/main", "none", "delete"]))
                .unwrap()
                .starts_with("REF_UPDATE_SIGNED tx1 refs/heads/main none delete")
        );

        let _ = fs::remove_file(path);
        let _ = fs::remove_dir(temp_dir);
    }

    #[test]
    fn derives_checkout_directory_from_gitmesh_url() {
        assert_eq!(
            default_clone_dir_from_url("gitmesh://farzeen/gitmesh"),
            PathBuf::from("gitmesh")
        );
        assert_eq!(
            default_clone_dir_from_url("gitmesh://gitmesh"),
            PathBuf::from("gitmesh")
        );
    }

    #[test]
    fn validates_gitmesh_clone_urls() {
        assert!(validate_gitmesh_url("gitmesh://farzeen/gitmesh").is_ok());
        assert!(validate_gitmesh_url("https://example.com/repo").is_err());
    }

    #[test]
    fn parses_ref_list_response() {
        let refs = parse_ref_list_response(
            "OK refs=refs/heads/main:3b18e512dba79e4c8300dd08aeb37f8e728b8dad",
        )
        .unwrap();

        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "refs/heads/main");
        assert_eq!(
            refs[0].oid,
            GitSha1Oid::from_str("3b18e512dba79e4c8300dd08aeb37f8e728b8dad").unwrap()
        );
        assert_eq!(parse_ref_list_response("OK refs=none").unwrap(), Vec::new());
    }

    #[test]
    fn parses_daemon_issue_and_pr_lists() {
        let issue_response = "OK repo=farzeen/gitmesh issues=1;5065727369737420636f6c6c61626f726174696f6e206576656e74206c6f6773;farzeen;70726f746f636f6c,636f6c6c61626f726174696f6e;aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa count=1";
        let pr_response = "OK repo=farzeen/gitmesh prs=1;5769726520676d20636f6c6c61626f726174696f6e20636f6d6d616e6473;farzeen;refs/heads/collaboration-cli;refs/heads/main;636c69;bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb count=1";

        let issues = parse_issue_list_response(issue_response).unwrap();
        let prs = parse_pr_list_response(pr_response).unwrap();

        assert_eq!(issues[0].number, 1);
        assert_eq!(issues[0].title, "Persist collaboration event logs");
        assert_eq!(issues[0].labels, ["protocol", "collaboration"]);
        assert_eq!(prs[0].title, "Wire gm collaboration commands");
        assert_eq!(prs[0].source_ref, "refs/heads/collaboration-cli");
        assert_eq!(prs[0].labels, ["cli"]);
        assert!(
            parse_issue_list_response("OK repo=farzeen/gitmesh issues=none count=0")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn encodes_collaboration_label_arguments() {
        assert_eq!(encode_label_arg(None).unwrap(), "-");
        assert_eq!(
            encode_label_arg(Some(&"storage, correctness".to_string())).unwrap(),
            "73746f72616765,636f72726563746e657373"
        );
        assert!(encode_label_arg(Some(&"bad\nlabel".to_string())).is_err());
    }

    #[test]
    fn encodes_git_object_kinds_for_daemon_commands() {
        assert_eq!(encode_git_kind(GitObjectKind::Blob), "blob");
        assert_eq!(encode_git_kind(GitObjectKind::Tree), "tree");
        assert_eq!(encode_git_kind(GitObjectKind::Commit), "commit");
        assert_eq!(encode_git_kind(GitObjectKind::Tag), "tag");
    }

    #[test]
    fn decodes_pack_hex_payloads() {
        assert_eq!(decode_hex("5041434b").unwrap(), b"PACK");
        assert!(decode_hex("xyz").is_err());
    }
}
