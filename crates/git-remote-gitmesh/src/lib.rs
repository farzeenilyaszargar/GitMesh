//! Git remote-helper protocol skeleton for GitMesh.
//!
//! Git invokes helpers as `git-remote-<transport> <repository> [<url>]` and
//! talks to them over stdin/stdout. This crate implements the Gen 1 fetch path
//! by installing daemon-exported packs into Git's local object database.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{fs, str::FromStr};

use gitmesh_coordination::{RefName, RefUpdate, RepoId, TransactionId};
use gitmesh_core::hex;
use gitmesh_git::GitSha1Oid;
use gitmesh_identity::{AccountRootKey, DeviceKey};
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HelperConfig {
    pub repository: String,
    pub url: Option<String>,
    pub refs_advertisement: Option<String>,
    pub pack_advertisement: Option<String>,
    pub git_dir: Option<PathBuf>,
    pub daemon_socket: Option<PathBuf>,
    pub identity_enabled: bool,
}

impl HelperConfig {
    pub fn new(repository: impl Into<String>, url: Option<String>) -> Self {
        Self {
            repository: repository.into(),
            url,
            refs_advertisement: None,
            pack_advertisement: None,
            git_dir: None,
            daemon_socket: None,
            identity_enabled: true,
        }
    }

    pub fn with_refs_advertisement(mut self, refs_advertisement: Option<String>) -> Self {
        self.refs_advertisement = refs_advertisement;
        self
    }

    pub fn with_pack_advertisement(mut self, pack_advertisement: Option<String>) -> Self {
        self.pack_advertisement = pack_advertisement;
        self
    }

    pub fn with_git_dir(mut self, git_dir: Option<PathBuf>) -> Self {
        self.git_dir = git_dir;
        self
    }

    pub fn with_daemon_socket(mut self, daemon_socket: Option<PathBuf>) -> Self {
        self.daemon_socket = daemon_socket;
        self
    }

    pub fn with_identity_enabled(mut self, identity_enabled: bool) -> Self {
        self.identity_enabled = identity_enabled;
        self
    }
}

#[derive(Debug, Error)]
pub enum HelperError {
    #[error("I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported remote-helper command '{0}'")]
    UnsupportedCommand(String),
    #[error("invalid daemon ref advertisement '{0}'")]
    InvalidRefsAdvertisement(String),
    #[error("invalid daemon pack advertisement")]
    InvalidPackAdvertisement,
    #[error("invalid hex payload")]
    InvalidHex,
    #[error("fetch requires GIT_DIR")]
    MissingGitDir,
    #[error("push requires gitmeshd socket")]
    MissingDaemonSocket,
    #[error("git command failed: {0}")]
    GitCommandFailed(String),
    #[error("daemon command failed: {0}")]
    Daemon(#[from] gitmeshd::DaemonError),
    #[error("daemon returned an error response: {0}")]
    DaemonResponse(String),
    #[error("unsupported push command '{0}'")]
    UnsupportedPush(String),
    #[error("local identity file is corrupt")]
    IdentityStoreCorrupt,
    #[error("invalid local identity hex payload")]
    InvalidIdentityHex,
    #[error("coordination failed: {0}")]
    Coordination(#[from] gitmesh_coordination::CoordinationError),
    #[error("Git object id failed: {0}")]
    Git(#[from] gitmesh_git::GitError),
    #[error("identity failed: {0}")]
    Identity(#[from] gitmesh_identity::IdentityError),
}

pub type Result<T> = std::result::Result<T, HelperError>;

pub fn run_helper<R, W>(config: HelperConfig, input: R, mut output: W) -> Result<()>
where
    R: BufRead,
    W: Write,
{
    let mut state = HelperState::new(config);
    for line in input.lines() {
        let line = line?;
        if line.is_empty() {
            if state.handle_blank(&mut output)? == HelperFlow::Stop {
                break;
            }
            output.flush()?;
            continue;
        }
        state.handle_command(&line, &mut output)?;
        output.flush()?;
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct HelperState {
    config: HelperConfig,
    verbosity: Option<i32>,
    progress: Option<bool>,
    check_connectivity: bool,
    fetch_requested: bool,
    fetch_oids: Vec<String>,
    pack_installed: bool,
    pending_pushes: Vec<PushCommand>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HelperFlow {
    Continue,
    Stop,
}

impl HelperState {
    fn new(config: HelperConfig) -> Self {
        Self {
            config,
            verbosity: None,
            progress: None,
            check_connectivity: false,
            fetch_requested: false,
            fetch_oids: Vec::new(),
            pack_installed: false,
            pending_pushes: Vec::new(),
        }
    }

    fn handle_command<W: Write>(&mut self, line: &str, output: &mut W) -> Result<()> {
        if line == "capabilities" {
            return self.capabilities(output);
        }
        if line == "list" || line == "list for-push" {
            return self.list_refs(output);
        }
        if line.starts_with("option ") {
            return self.option(line, output);
        }
        if line.starts_with("fetch ") {
            self.fetch(line)?;
            return Ok(());
        }
        if line.starts_with("push ") {
            self.push(line)?;
            return Ok(());
        }
        Err(HelperError::UnsupportedCommand(line.to_string()))
    }

    fn handle_blank<W: Write>(&mut self, output: &mut W) -> Result<HelperFlow> {
        if !self.pending_pushes.is_empty() {
            let pushes = std::mem::take(&mut self.pending_pushes);
            for push in pushes {
                match self.apply_push(&push) {
                    Ok(()) => writeln!(output, "ok {}", push.dst)?,
                    Err(err) => writeln!(
                        output,
                        "error {} {}",
                        push.dst,
                        sanitize_status(&err.to_string())
                    )?,
                }
            }
            writeln!(output)?;
            return Ok(HelperFlow::Continue);
        }
        if self.fetch_requested {
            self.install_fetch_pack()?;
            if self.check_connectivity {
                let git_dir = self
                    .config
                    .git_dir
                    .as_ref()
                    .ok_or(HelperError::MissingGitDir)?;
                verify_git_connectivity(git_dir)?;
                writeln!(output, "connectivity-ok")?;
            }
            writeln!(output)?;
            self.fetch_requested = false;
            return Ok(HelperFlow::Continue);
        }
        Ok(HelperFlow::Stop)
    }

    fn capabilities<W: Write>(&self, output: &mut W) -> Result<()> {
        writeln!(output, "fetch")?;
        writeln!(output, "push")?;
        writeln!(output, "option")?;
        writeln!(output, "check-connectivity")?;
        writeln!(output)?;
        Ok(())
    }

    fn list_refs<W: Write>(&self, output: &mut W) -> Result<()> {
        if let Some(refs) = &self.config.refs_advertisement {
            for reference in parse_refs_advertisement(refs)? {
                writeln!(output, "{} {}", reference.oid, reference.name)?;
            }
        }
        writeln!(output)?;
        Ok(())
    }

    fn option<W: Write>(&mut self, line: &str, output: &mut W) -> Result<()> {
        let mut parts = line.splitn(3, ' ');
        let _command = parts.next();
        let name = parts.next().unwrap_or_default();
        let value = parts.next().unwrap_or_default();

        match name {
            "verbosity" => match value.parse::<i32>() {
                Ok(value) => {
                    self.verbosity = Some(value);
                    writeln!(output, "ok")?;
                }
                Err(_) => {
                    writeln!(output, "error invalid verbosity")?;
                }
            },
            "progress" => {
                self.progress = Some(value != "false");
                writeln!(output, "ok")?;
            }
            "check-connectivity" => {
                self.check_connectivity = value != "false";
                writeln!(output, "ok")?;
            }
            _ => {
                writeln!(output, "unsupported")?;
            }
        }
        Ok(())
    }

    fn fetch(&mut self, line: &str) -> Result<()> {
        let parts = line.split_whitespace().collect::<Vec<_>>();
        if parts.len() != 3 {
            return Err(HelperError::UnsupportedCommand(line.to_string()));
        }
        let oid = parts[1];
        let name = parts[2];
        let advertised = self
            .config
            .refs_advertisement
            .as_deref()
            .map(parse_refs_advertisement)
            .transpose()?
            .unwrap_or_default();
        if !advertised
            .iter()
            .any(|reference| reference.oid == oid && reference.name == name)
        {
            return Err(HelperError::UnsupportedCommand(line.to_string()));
        }
        self.fetch_requested = true;
        if !self.fetch_oids.iter().any(|existing| existing == oid) {
            self.fetch_oids.push(oid.to_string());
        }
        Ok(())
    }

    fn install_fetch_pack(&mut self) -> Result<()> {
        if self.pack_installed {
            return Ok(());
        }
        let git_dir = self
            .config
            .git_dir
            .as_ref()
            .ok_or(HelperError::MissingGitDir)?;
        let pack_response = self.fetch_pack_response()?;
        let pack_hex = response_field(&pack_response, "pack_hex")
            .ok_or(HelperError::InvalidPackAdvertisement)?;
        let pack = decode_hex(pack_hex)?;
        let pack_dir = git_dir.join("objects").join("pack");
        fs::create_dir_all(&pack_dir)?;
        let pack_path = pack_dir.join("gitmesh-fetch.pack");
        fs::write(&pack_path, pack)?;
        let output = Command::new("git")
            .arg("--git-dir")
            .arg(git_dir)
            .arg("index-pack")
            .arg("--strict")
            .arg(&pack_path)
            .output()?;
        if !output.status.success() {
            return Err(HelperError::GitCommandFailed(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }
        self.pack_installed = true;
        Ok(())
    }

    fn fetch_pack_response(&self) -> Result<String> {
        if let Some(daemon_socket) = &self.config.daemon_socket
            && !self.fetch_oids.is_empty()
        {
            let response = gitmeshd::request_unix_socket(
                daemon_socket,
                &format!("PACK_GET_REACHABLE {}", self.fetch_oids.join(",")),
            )?;
            if response.starts_with("OK ") {
                return Ok(response);
            }
        }
        self.config
            .pack_advertisement
            .clone()
            .ok_or(HelperError::InvalidPackAdvertisement)
    }

    fn push(&mut self, line: &str) -> Result<()> {
        let spec = line
            .strip_prefix("push ")
            .ok_or_else(|| HelperError::UnsupportedPush(line.to_string()))?;
        let push = PushCommand::parse(spec)?;
        self.pending_pushes.push(push);
        Ok(())
    }

    fn apply_push(&self, push: &PushCommand) -> Result<()> {
        let git_dir = self
            .config
            .git_dir
            .as_ref()
            .ok_or(HelperError::MissingGitDir)?;
        let daemon_socket = self
            .config
            .daemon_socket
            .as_ref()
            .ok_or(HelperError::MissingDaemonSocket)?;
        let expected = current_remote_ref(daemon_socket, &push.dst)?;
        let new_target = if push.delete {
            "delete".to_string()
        } else {
            let new_oid = resolve_git_oid(git_dir, &push.src)?;
            import_reachable_objects(git_dir, &push.src, daemon_socket)?;
            new_oid
        };
        let expected_text = expected.unwrap_or_else(|| "none".to_string());
        let transaction_id = format!(
            "push-{}-{}-{}",
            sanitize_tx_component(&push.dst),
            expected_text,
            new_target
        );
        let response = if self.config.identity_enabled
            && let Some(identity) = LocalIdentity::load_optional()?
        {
            let command = identity.signed_ref_update_command(
                &transaction_id,
                &push.dst,
                &expected_text,
                &new_target,
                push.force,
            )?;
            gitmeshd::request_unix_socket(daemon_socket, &command)?
        } else {
            let command = if push.force {
                "REF_UPDATE_FORCE"
            } else {
                "REF_UPDATE"
            };
            gitmeshd::request_unix_socket(
                daemon_socket,
                &format!(
                    "{command} {transaction_id} {} {} {new_target} git-remote-gitmesh",
                    push.dst, expected_text
                ),
            )?
        };
        if !response.starts_with("OK status=committed") {
            return Err(HelperError::DaemonResponse(response));
        }
        Ok(())
    }
}

#[derive(Clone)]
struct LocalIdentity {
    label: String,
    device: DeviceKey,
    certificate: gitmesh_identity::DeviceCertificate,
}

impl LocalIdentity {
    fn load_optional() -> Result<Option<Self>> {
        let path = identity_path()?;
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(Self::load_from_path(&path)?))
    }

    fn load_from_path(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)?;
        let mut lines = text.lines();
        if lines.next() != Some("gitmesh-local-identity-v0") {
            return Err(HelperError::IdentityStoreCorrupt);
        }
        let label = read_identity_field(&mut lines, "label")?;
        let account_seed =
            decode_fixed_hex::<32>(&read_identity_field(&mut lines, "account_seed")?)?;
        let device_seed = decode_fixed_hex::<32>(&read_identity_field(&mut lines, "device_seed")?)?;
        if lines.next().is_some() {
            return Err(HelperError::IdentityStoreCorrupt);
        }
        let account = AccountRootKey::from_seed(account_seed);
        let device = DeviceKey::from_seed(device_seed);
        let certificate = account.certify_device(&device, label.clone());
        certificate.verify()?;
        Ok(Self {
            label,
            device,
            certificate,
        })
    }

    fn signed_ref_update_command(
        &self,
        transaction_id: &str,
        ref_name: &str,
        expected_old_oid: &str,
        new_oid: &str,
        force: bool,
    ) -> Result<String> {
        let update = RefUpdate {
            repo_id: RepoId::new(b"gitmeshd-v0-repo"),
            ref_name: RefName::new(ref_name)?,
            expected_old_oid: parse_optional_oid(expected_old_oid)?,
            new_oid: parse_optional_new_oid(new_oid)?,
            force,
            policy_epoch: 0,
            transaction_id: TransactionId::new(transaction_id)?,
            signer: self.certificate.device_id.as_cid().to_string(),
        };
        let update_signature = self.device.sign(&update.signing_transcript());
        let command = if force {
            "REF_UPDATE_SIGNED_FORCE"
        } else {
            "REF_UPDATE_SIGNED"
        };
        Ok(format!(
            "{command} {transaction_id} {ref_name} {expected_old_oid} {new_oid} {} {} {} {} {}",
            hex(self.label.as_bytes()),
            hex(&self.certificate.account_verifying_key),
            hex(&self.certificate.device_verifying_key),
            hex(&self.certificate.signature),
            hex(&update_signature)
        ))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PushCommand {
    force: bool,
    delete: bool,
    src: String,
    dst: String,
}

impl PushCommand {
    fn parse(spec: &str) -> Result<Self> {
        let (force, spec) = spec
            .strip_prefix('+')
            .map_or((false, spec), |spec| (true, spec));
        let (src, dst) = spec
            .split_once(':')
            .ok_or_else(|| HelperError::UnsupportedPush(spec.to_string()))?;
        if dst.is_empty() || !dst.starts_with("refs/") {
            return Err(HelperError::UnsupportedPush(spec.to_string()));
        }
        Ok(Self {
            force,
            delete: src.is_empty(),
            src: src.to_string(),
            dst: dst.to_string(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdvertisedRef {
    pub name: String,
    pub oid: String,
}

fn resolve_git_oid(git_dir: &Path, rev: &str) -> Result<String> {
    let output = Command::new("git")
        .arg("--git-dir")
        .arg(git_dir)
        .arg("rev-parse")
        .arg("--verify")
        .arg(rev)
        .output()?;
    if !output.status.success() {
        return Err(HelperError::GitCommandFailed(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn verify_git_connectivity(git_dir: &Path) -> Result<()> {
    let output = Command::new("git")
        .arg("--git-dir")
        .arg(git_dir)
        .arg("fsck")
        .arg("--connectivity-only")
        .output()?;
    if !output.status.success() {
        return Err(HelperError::GitCommandFailed(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(())
}

fn import_reachable_objects(git_dir: &Path, rev: &str, daemon_socket: &Path) -> Result<()> {
    let output = Command::new("git")
        .arg("--git-dir")
        .arg(git_dir)
        .arg("rev-list")
        .arg("--objects")
        .arg("--no-object-names")
        .arg(rev)
        .output()?;
    if !output.status.success() {
        return Err(HelperError::GitCommandFailed(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    for oid in String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        import_git_object(git_dir, oid, daemon_socket)?;
    }
    Ok(())
}

fn import_git_object(git_dir: &Path, oid: &str, daemon_socket: &Path) -> Result<()> {
    let kind_output = Command::new("git")
        .arg("--git-dir")
        .arg(git_dir)
        .arg("cat-file")
        .arg("-t")
        .arg(oid)
        .output()?;
    if !kind_output.status.success() {
        return Err(HelperError::GitCommandFailed(
            String::from_utf8_lossy(&kind_output.stderr)
                .trim()
                .to_string(),
        ));
    }
    let kind = String::from_utf8_lossy(&kind_output.stdout)
        .trim()
        .to_string();
    if !matches!(kind.as_str(), "blob" | "tree" | "commit" | "tag") {
        return Err(HelperError::UnsupportedPush(format!(
            "unsupported Git object kind {kind}"
        )));
    }
    let payload_output = Command::new("git")
        .arg("--git-dir")
        .arg(git_dir)
        .arg("cat-file")
        .arg(&kind)
        .arg(oid)
        .output()?;
    if !payload_output.status.success() {
        return Err(HelperError::GitCommandFailed(
            String::from_utf8_lossy(&payload_output.stderr)
                .trim()
                .to_string(),
        ));
    }
    let payload_hex = if payload_output.stdout.is_empty() {
        "-".to_string()
    } else {
        encode_hex(&payload_output.stdout)
    };
    let response =
        gitmeshd::request_unix_socket(daemon_socket, &format!("OBJECT_PUT {kind} {payload_hex}"))?;
    if !response.starts_with("OK ") {
        return Err(HelperError::DaemonResponse(response));
    }
    Ok(())
}

fn current_remote_ref(daemon_socket: &Path, ref_name: &str) -> Result<Option<String>> {
    let response = gitmeshd::request_unix_socket(daemon_socket, &format!("REF_GET {ref_name}"))?;
    if !response.starts_with("OK ") {
        return Err(HelperError::DaemonResponse(response));
    }
    let oid = response_field(&response, "oid").ok_or_else(|| {
        HelperError::DaemonResponse(format!("missing oid field in response: {response}"))
    })?;
    if oid == "none" {
        Ok(None)
    } else {
        Ok(Some(oid.to_string()))
    }
}

fn parse_optional_oid(value: &str) -> Result<Option<GitSha1Oid>> {
    if value == "none" {
        Ok(None)
    } else {
        Ok(Some(GitSha1Oid::from_str(value)?))
    }
}

fn parse_optional_new_oid(value: &str) -> Result<Option<GitSha1Oid>> {
    if value == "delete" {
        Ok(None)
    } else {
        Ok(Some(GitSha1Oid::from_str(value)?))
    }
}

fn identity_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("GITMESH_IDENTITY") {
        return Ok(path.into());
    }
    let home = std::env::var_os("HOME").ok_or(HelperError::IdentityStoreCorrupt)?;
    Ok(PathBuf::from(home).join(".gitmesh").join("identity-v0.tsv"))
}

fn read_identity_field<'a>(
    lines: &mut impl Iterator<Item = &'a str>,
    expected_name: &str,
) -> Result<String> {
    let line = lines.next().ok_or(HelperError::IdentityStoreCorrupt)?;
    let (name, value) = line
        .split_once('\t')
        .ok_or(HelperError::IdentityStoreCorrupt)?;
    if name != expected_name || value.is_empty() {
        return Err(HelperError::IdentityStoreCorrupt);
    }
    Ok(value.to_string())
}

fn decode_fixed_hex<const N: usize>(value: &str) -> Result<[u8; N]> {
    let bytes = decode_hex(value).map_err(|_| HelperError::InvalidIdentityHex)?;
    bytes
        .try_into()
        .map_err(|_| HelperError::InvalidIdentityHex)
}

pub fn parse_refs_advertisement(value: &str) -> Result<Vec<AdvertisedRef>> {
    let value = value
        .strip_prefix("OK ")
        .unwrap_or(value)
        .strip_prefix("refs=")
        .ok_or_else(|| HelperError::InvalidRefsAdvertisement(value.to_string()))?;
    if value == "none" {
        return Ok(Vec::new());
    }
    value
        .split(',')
        .map(|entry| {
            let (name, oid) = entry
                .split_once(':')
                .ok_or_else(|| HelperError::InvalidRefsAdvertisement(entry.to_string()))?;
            if name.is_empty()
                || oid.len() != 40
                || !oid.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(HelperError::InvalidRefsAdvertisement(entry.to_string()));
            }
            Ok(AdvertisedRef {
                name: name.to_string(),
                oid: oid.to_string(),
            })
        })
        .collect()
}

fn response_field<'a>(response: &'a str, name: &str) -> Option<&'a str> {
    response
        .split_whitespace()
        .find_map(|part| part.strip_prefix(&format!("{name}=")))
}

fn decode_hex(value: &str) -> Result<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return Err(HelperError::InvalidHex);
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|chunk| {
            let high = hex_nibble(chunk[0]).ok_or(HelperError::InvalidHex)?;
            let low = hex_nibble(chunk[1]).ok_or(HelperError::InvalidHex)?;
            Ok((high << 4) | low)
        })
        .collect()
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sanitize_tx_component(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_') {
                byte as char
            } else {
                '-'
            }
        })
        .collect()
}

fn sanitize_status(value: &str) -> String {
    value.replace(['\n', '\r'], " ")
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
    use std::io::Cursor;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::*;

    const CAPABILITIES: &str = "fetch\npush\noption\ncheck-connectivity\n\n";

    #[test]
    fn reports_capabilities_with_blank_terminator() {
        let mut output = Vec::new();
        run_helper(
            HelperConfig::new("origin", Some("gitmesh://farzeen/gitmesh".to_string())),
            Cursor::new("capabilities\n\n"),
            &mut output,
        )
        .unwrap();

        assert_eq!(String::from_utf8(output).unwrap(), CAPABILITIES);
    }

    #[test]
    fn accepts_basic_options() {
        let mut output = Vec::new();
        run_helper(
            HelperConfig::new("origin", None),
            Cursor::new("capabilities\noption verbosity 2\noption progress true\n\n"),
            &mut output,
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            format!("{CAPABILITIES}ok\nok\n")
        );
    }

    #[test]
    fn unsupported_options_do_not_abort_protocol() {
        let mut output = Vec::new();
        run_helper(
            HelperConfig::new("origin", None),
            Cursor::new("capabilities\noption depth 1\n\n"),
            &mut output,
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            format!("{CAPABILITIES}unsupported\n")
        );
    }

    #[test]
    fn list_returns_empty_ref_list() {
        let mut output = Vec::new();
        run_helper(
            HelperConfig::new("origin", None),
            Cursor::new("capabilities\nlist\n\n"),
            &mut output,
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            format!("{CAPABILITIES}\n")
        );
    }

    #[test]
    fn list_advertises_daemon_refs() {
        let mut output = Vec::new();
        run_helper(
            HelperConfig::new("origin", None).with_refs_advertisement(Some(
                "OK refs=refs/heads/main:3b18e512dba79e4c8300dd08aeb37f8e728b8dad".to_string(),
            )),
            Cursor::new("capabilities\nlist\n\n"),
            &mut output,
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            format!("{CAPABILITIES}3b18e512dba79e4c8300dd08aeb37f8e728b8dad refs/heads/main\n\n")
        );
    }

    #[test]
    fn parses_push_specs() {
        let push = PushCommand::parse("refs/heads/main:refs/heads/main").unwrap();
        let forced = PushCommand::parse("+HEAD:refs/heads/main").unwrap();
        let deleted = PushCommand::parse(":refs/heads/main").unwrap();

        assert_eq!(
            push,
            PushCommand {
                force: false,
                delete: false,
                src: "refs/heads/main".to_string(),
                dst: "refs/heads/main".to_string()
            }
        );
        assert!(forced.force);
        assert!(!forced.delete);
        assert!(deleted.delete);
        assert_eq!(deleted.dst, "refs/heads/main");
    }

    #[test]
    fn parses_empty_ref_advertisement() {
        assert_eq!(
            parse_refs_advertisement("OK refs=none").unwrap(),
            Vec::new()
        );
    }

    #[test]
    fn rejects_unadvertised_fetch() {
        let mut output = Vec::new();
        let err = run_helper(
            HelperConfig::new("origin", None),
            Cursor::new("fetch 3b18e512dba79e4c8300dd08aeb37f8e728b8dad refs/heads/main\n\n"),
            &mut output,
        )
        .unwrap_err();

        assert!(matches!(err, HelperError::UnsupportedCommand(_)));
    }

    #[test]
    fn fetch_records_requested_tip_oids_once() {
        let mut state = HelperState::new(
            HelperConfig::new("origin", None).with_refs_advertisement(Some(
                "OK refs=refs/heads/main:3b18e512dba79e4c8300dd08aeb37f8e728b8dad".to_string(),
            )),
        );

        state
            .fetch("fetch 3b18e512dba79e4c8300dd08aeb37f8e728b8dad refs/heads/main")
            .unwrap();
        state
            .fetch("fetch 3b18e512dba79e4c8300dd08aeb37f8e728b8dad refs/heads/main")
            .unwrap();

        assert_eq!(
            state.fetch_oids,
            vec!["3b18e512dba79e4c8300dd08aeb37f8e728b8dad"]
        );
    }

    #[test]
    fn fetch_installs_advertised_pack_and_checks_connectivity() {
        let git_dir =
            std::env::temp_dir().join(format!("gitmesh-helper-fetch-{}.git", std::process::id()));
        let init = Command::new("git")
            .arg("init")
            .arg("--bare")
            .arg(&git_dir)
            .output()
            .unwrap();
        assert!(
            init.status.success(),
            "{}",
            String::from_utf8_lossy(&init.stderr)
        );
        let object = gitmesh_git::GitObject::new(gitmesh_git::GitObjectKind::Blob, b"fetched");
        let oid = object.sha1_oid().to_string();
        let pack = gitmesh_git::write_packfile(&[object]).unwrap();
        let refs_advertisement = format!("OK refs=refs/heads/main:{oid}");
        let pack_advertisement =
            format!("OK pack_version=2 objects=1 pack_hex={}", encode_hex(&pack));
        let mut output = Vec::new();

        run_helper(
            HelperConfig::new("origin", None)
                .with_refs_advertisement(Some(refs_advertisement))
                .with_pack_advertisement(Some(pack_advertisement))
                .with_git_dir(Some(git_dir.clone())),
            Cursor::new(format!(
                "capabilities\noption check-connectivity true\nfetch {oid} refs/heads/main\n\n"
            )),
            &mut output,
        )
        .unwrap();
        let cat = Command::new("git")
            .arg("--git-dir")
            .arg(&git_dir)
            .arg("cat-file")
            .arg("-p")
            .arg(&oid)
            .output()
            .unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            format!("{CAPABILITIES}ok\nconnectivity-ok\n\n")
        );
        assert!(cat.status.success());
        assert_eq!(cat.stdout, b"fetched");

        let _ = fs::remove_dir_all(git_dir);
    }

    #[test]
    fn push_imports_commit_graph_into_live_daemon_and_publishes_ref() {
        let daemon = LiveDaemon::start("gitmesh-helper-push");
        let worktree = daemon.root.join("worktree");
        run_git_command(&daemon.root, ["init", worktree.to_str().unwrap()]);
        run_git_command(
            &worktree,
            ["config", "user.email", "gitmesh@example.invalid"],
        );
        run_git_command(&worktree, ["config", "user.name", "GitMesh Test"]);
        fs::write(worktree.join("README.md"), "hello from remote helper\n").unwrap();
        run_git_command(&worktree, ["add", "README.md"]);
        run_git_command(&worktree, ["commit", "-m", "initial"]);
        let oid_output = Command::new("git")
            .arg("-C")
            .arg(&worktree)
            .arg("rev-parse")
            .arg("HEAD")
            .output()
            .unwrap();
        assert!(oid_output.status.success());
        let oid = String::from_utf8_lossy(&oid_output.stdout)
            .trim()
            .to_string();
        let mut output = Vec::new();

        run_helper(
            HelperConfig::new("origin", Some("gitmesh://farzeen/gitmesh".to_string()))
                .with_git_dir(Some(worktree.join(".git")))
                .with_daemon_socket(Some(daemon.socket.clone()))
                .with_identity_enabled(false),
            Cursor::new("capabilities\npush HEAD:refs/heads/main\n\n"),
            &mut output,
        )
        .unwrap();
        let ref_response =
            gitmeshd::request_unix_socket(&daemon.socket, "REF_GET refs/heads/main").unwrap();
        let pack_response = gitmeshd::request_unix_socket(&daemon.socket, "PACK_GET all").unwrap();
        let pack_hex = response_field(&pack_response, "pack_hex").unwrap();
        let pack = decode_hex(pack_hex).unwrap();
        let parsed = gitmesh_git::parse_packfile(&pack).unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            format!("{CAPABILITIES}ok refs/heads/main\n\n")
        );
        assert_eq!(ref_response, format!("OK ref=refs/heads/main oid={oid}"));
        assert!(
            parsed
                .objects
                .iter()
                .any(|object| object.sha1_oid().to_string() == oid)
        );

        let _ = fs::remove_dir_all(daemon.root);
    }

    #[test]
    fn push_delete_removes_live_daemon_ref() {
        let daemon = LiveDaemon::start("gitmesh-helper-delete");
        let worktree = daemon.root.join("worktree");
        run_git_command(&daemon.root, ["init", worktree.to_str().unwrap()]);
        run_git_command(
            &worktree,
            ["config", "user.email", "gitmesh@example.invalid"],
        );
        run_git_command(&worktree, ["config", "user.name", "GitMesh Test"]);
        fs::write(worktree.join("README.md"), "delete me\n").unwrap();
        run_git_command(&worktree, ["add", "README.md"]);
        run_git_command(&worktree, ["commit", "-m", "initial"]);
        let config = HelperConfig::new("origin", Some("gitmesh://farzeen/gitmesh".to_string()))
            .with_git_dir(Some(worktree.join(".git")))
            .with_daemon_socket(Some(daemon.socket.clone()))
            .with_identity_enabled(false);
        let mut create_output = Vec::new();
        let mut delete_output = Vec::new();

        run_helper(
            config.clone(),
            Cursor::new("capabilities\npush HEAD:refs/heads/main\n\n"),
            &mut create_output,
        )
        .unwrap();
        run_helper(
            config,
            Cursor::new("capabilities\npush :refs/heads/main\n\n"),
            &mut delete_output,
        )
        .unwrap();
        let ref_response =
            gitmeshd::request_unix_socket(&daemon.socket, "REF_GET refs/heads/main").unwrap();

        assert_eq!(
            String::from_utf8(delete_output).unwrap(),
            format!("{CAPABILITIES}ok refs/heads/main\n\n")
        );
        assert_eq!(ref_response, "OK ref=refs/heads/main oid=none");

        let _ = fs::remove_dir_all(daemon.root);
    }

    #[test]
    fn push_reports_error_when_live_daemon_ref_would_move_backwards() {
        let daemon = LiveDaemon::start("gitmesh-helper-conflict");
        let worktree = daemon.root.join("worktree");
        run_git_command(&daemon.root, ["init", worktree.to_str().unwrap()]);
        configure_git_author(&worktree);
        fs::write(worktree.join("README.md"), "first\n").unwrap();
        run_git_command(&worktree, ["add", "README.md"]);
        run_git_command(&worktree, ["commit", "-m", "first"]);
        let first_oid = git_rev_parse(&worktree, "HEAD");
        let config = HelperConfig::new("origin", Some("gitmesh://farzeen/gitmesh".to_string()))
            .with_git_dir(Some(worktree.join(".git")))
            .with_daemon_socket(Some(daemon.socket.clone()))
            .with_identity_enabled(false);
        let mut first_output = Vec::new();
        run_helper(
            config.clone(),
            Cursor::new("capabilities\npush HEAD:refs/heads/main\n\n"),
            &mut first_output,
        )
        .unwrap();
        fs::write(worktree.join("README.md"), "second\n").unwrap();
        run_git_command(&worktree, ["add", "README.md"]);
        run_git_command(&worktree, ["commit", "-m", "second"]);
        let second_oid = git_rev_parse(&worktree, "HEAD");
        let mut second_output = Vec::new();
        run_helper(
            config.clone(),
            Cursor::new("capabilities\npush HEAD:refs/heads/main\n\n"),
            &mut second_output,
        )
        .unwrap();
        let mut stale_output = Vec::new();

        run_helper(
            config,
            Cursor::new(format!(
                "capabilities\npush {first_oid}:refs/heads/main\n\n"
            )),
            &mut stale_output,
        )
        .unwrap();
        let ref_response =
            gitmeshd::request_unix_socket(&daemon.socket, "REF_GET refs/heads/main").unwrap();

        assert_eq!(
            String::from_utf8(second_output).unwrap(),
            format!("{CAPABILITIES}ok refs/heads/main\n\n")
        );
        let stale_text = String::from_utf8(stale_output).unwrap();
        assert!(stale_text.contains("error refs/heads/main"));
        assert!(stale_text.contains("non-fast-forward"));
        assert_eq!(
            ref_response,
            format!("OK ref=refs/heads/main oid={second_oid}")
        );

        let _ = fs::remove_dir_all(daemon.root);
    }

    #[test]
    fn push_reports_non_fast_forward_without_force() {
        let daemon = LiveDaemon::start("gitmesh-helper-non-ff");
        let first_worktree = daemon.root.join("first");
        let second_worktree = daemon.root.join("second");
        run_git_command(&daemon.root, ["init", first_worktree.to_str().unwrap()]);
        run_git_command(&daemon.root, ["init", second_worktree.to_str().unwrap()]);
        configure_git_author(&first_worktree);
        configure_git_author(&second_worktree);
        fs::write(first_worktree.join("README.md"), "first root\n").unwrap();
        run_git_command(&first_worktree, ["add", "README.md"]);
        run_git_command(&first_worktree, ["commit", "-m", "first"]);
        fs::write(second_worktree.join("README.md"), "second root\n").unwrap();
        run_git_command(&second_worktree, ["add", "README.md"]);
        run_git_command(&second_worktree, ["commit", "-m", "second"]);
        let first_oid = git_rev_parse(&first_worktree, "HEAD");
        let second_oid = git_rev_parse(&second_worktree, "HEAD");
        let first_config =
            HelperConfig::new("origin", Some("gitmesh://farzeen/gitmesh".to_string()))
                .with_git_dir(Some(first_worktree.join(".git")))
                .with_daemon_socket(Some(daemon.socket.clone()))
                .with_identity_enabled(false);
        let second_config =
            HelperConfig::new("origin", Some("gitmesh://farzeen/gitmesh".to_string()))
                .with_git_dir(Some(second_worktree.join(".git")))
                .with_daemon_socket(Some(daemon.socket.clone()))
                .with_identity_enabled(false);
        let mut first_output = Vec::new();
        let mut rejected_output = Vec::new();

        run_helper(
            first_config,
            Cursor::new("capabilities\npush HEAD:refs/heads/main\n\n"),
            &mut first_output,
        )
        .unwrap();
        run_helper(
            second_config,
            Cursor::new("capabilities\npush HEAD:refs/heads/main\n\n"),
            &mut rejected_output,
        )
        .unwrap();
        let ref_response =
            gitmeshd::request_unix_socket(&daemon.socket, "REF_GET refs/heads/main").unwrap();
        let rejected_text = String::from_utf8(rejected_output).unwrap();

        assert!(rejected_text.contains("error refs/heads/main"));
        assert!(rejected_text.contains("non-fast-forward"));
        assert_eq!(
            ref_response,
            format!("OK ref=refs/heads/main oid={first_oid}")
        );
        assert_ne!(first_oid, second_oid);

        let _ = fs::remove_dir_all(daemon.root);
    }

    #[test]
    fn decodes_pack_hex_payloads() {
        assert_eq!(decode_hex("5041434b").unwrap(), b"PACK");
        assert!(decode_hex("xyz").is_err());
    }

    #[test]
    fn verifies_connectivity_for_empty_bare_repository() {
        let git_dir = std::env::temp_dir().join(format!(
            "gitmesh-helper-connectivity-{}.git",
            std::process::id()
        ));
        let output = Command::new("git")
            .arg("init")
            .arg("--bare")
            .arg(&git_dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );

        verify_git_connectivity(&git_dir).unwrap();

        let _ = fs::remove_dir_all(git_dir);
    }

    #[test]
    fn local_identity_builds_signed_force_ref_update_command() {
        let account = AccountRootKey::generate();
        let device = DeviceKey::generate();
        let path = std::env::temp_dir().join(format!(
            "gitmesh-helper-identity-{}.tsv",
            std::process::id()
        ));
        fs::write(
            &path,
            format!(
                "gitmesh-local-identity-v0\nlabel\thelper-device\naccount_seed\t{}\ndevice_seed\t{}\n",
                hex(&account.seed_bytes()),
                hex(&device.seed_bytes())
            ),
        )
        .unwrap();

        let identity = LocalIdentity::load_from_path(&path).unwrap();
        let command = identity
            .signed_ref_update_command(
                "tx1",
                "refs/heads/main",
                "3b18e512dba79e4c8300dd08aeb37f8e728b8dad",
                "6b18e512dba79e4c8300dd08aeb37f8e728b8dad",
                true,
            )
            .unwrap();

        assert!(command.starts_with(
            "REF_UPDATE_SIGNED_FORCE tx1 refs/heads/main 3b18e512dba79e4c8300dd08aeb37f8e728b8dad 6b18e512dba79e4c8300dd08aeb37f8e728b8dad"
        ));

        let _ = fs::remove_file(path);
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        PathBuf::from("/tmp").join(format!("{prefix}-{}-{nanos}", std::process::id()))
    }

    struct LiveDaemon {
        root: PathBuf,
        socket: PathBuf,
    }

    impl LiveDaemon {
        fn start(prefix: &str) -> Self {
            let root = unique_temp_dir(prefix);
            let socket = root.join("gitmeshd.sock");
            fs::create_dir_all(&root).unwrap();
            let daemon_socket = socket.clone();
            let object_store = root.join("objects.tsv");
            let ref_store = root.join("refs.tsv");
            let policy_store = root.join("policy.tsv");
            let key_store = root.join("keys.tsv");
            let account_store = root.join("accounts.tsv");
            let (ready_tx, ready_rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let result = gitmeshd::serve_unix_socket_with_stores_and_auth(
                    daemon_socket,
                    gitmeshd::DaemonStorePaths {
                        object_store_path: Some(object_store),
                        ref_store_path: Some(ref_store),
                        policy_store_path: Some(policy_store),
                        key_grant_store_path: Some(key_store),
                        account_store_path: Some(account_store),
                        collaboration_store_path: None,
                        network_store_path: None,
                        ..gitmeshd::DaemonStorePaths::default()
                    },
                    gitmeshd::DaemonAuth::disabled(),
                );
                let _ = ready_tx.send(result.map_err(|err| err.to_string()));
            });
            wait_for_socket(&socket, &ready_rx);
            Self { root, socket }
        }
    }

    fn wait_for_socket(
        socket: &Path,
        server_result: &std::sync::mpsc::Receiver<std::result::Result<(), String>>,
    ) {
        for _ in 0..100 {
            if socket.exists() {
                return;
            }
            if let Ok(result) = server_result.try_recv() {
                panic!("daemon exited before creating socket: {result:?}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("daemon socket was not created: {}", socket.display());
    }

    fn run_git_command<const N: usize>(cwd: &Path, args: [&str; N]) {
        let output = Command::new("git")
            .current_dir(cwd)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn configure_git_author(worktree: &Path) {
        run_git_command(
            worktree,
            ["config", "user.email", "gitmesh@example.invalid"],
        );
        run_git_command(worktree, ["config", "user.name", "GitMesh Test"]);
    }

    fn git_rev_parse(worktree: &Path, rev: &str) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(worktree)
            .arg("rev-parse")
            .arg(rev)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git rev-parse failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }
}
