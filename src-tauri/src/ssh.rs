use russh::{client, keys, ChannelMsg};
use russh::keys::PublicKeyOrCertificate;
use russh_sftp::client::SftpSession;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::{Path, PathBuf}, process::Command, sync::{atomic::{AtomicU64, Ordering}, Arc}};
use tokio::{io::{AsyncWriteExt}, sync::Mutex};

pub type TerminalOutput = std::sync::Arc<dyn Fn(String) + Send + Sync>;

pub struct ShelfState {
    pub sftp: Mutex<Option<SftpSession>>,
    pub ssh: Mutex<Option<client::Handle<HostVerifier>>>,
    pub terminals: Arc<Mutex<HashMap<u64, russh::ChannelWriteHalf<client::Msg>>>>,
    pub next_terminal_id: AtomicU64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInput {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub private_key_path: String,
    #[serde(default)]
    pub key_passphrase: String,
    pub trust_new_host: bool,
    #[serde(default)]
    pub confirmed_fingerprint: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub kind: String,
    pub size: u64,
    pub modified: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryListing {
    pub path: String,
    pub entries: Vec<FileEntry>,
}

const DRIVES_ROOT: &str = "shelf://drives";

#[derive(Clone, Default)]
pub struct HostKeyVerdict(std::sync::Arc<std::sync::Mutex<Option<String>>>);

impl HostKeyVerdict {
    fn set(&self, verdict: String) {
        *self.0.lock().unwrap() = Some(verdict);
    }

    fn message(&self) -> Option<String> {
        self.0.lock().unwrap().clone()
    }
}

pub fn host_key_fingerprint_string(public_key: &keys::PublicKey) -> String {
    format!("{} {}", public_key.algorithm().as_str(), public_key.fingerprint(keys::HashAlg::Sha256))
}

pub struct HostVerifier {
    host: String,
    port: u16,
    trust_new_host: bool,
    confirmed_fingerprint: String,
    verdict: HostKeyVerdict,
}

impl client::Handler for HostVerifier {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let public_key = key.public_key();
        match keys::check_known_hosts(&self.host, self.port, &public_key) {
            Ok(true) => Ok(true),
            Ok(false) => {
                // Unknown host: never learn on the checkbox alone. The key is saved only when the
                // fingerprint the user confirmed matches the key presented at THIS handshake.
                let presented = host_key_fingerprint_string(&public_key);
                let presented_hash = public_key.fingerprint(keys::HashAlg::Sha256).to_string();
                let confirmed = self.confirmed_fingerprint.trim();
                if self.trust_new_host && !confirmed.is_empty() {
                    if confirmed == presented || confirmed == presented_hash {
                        let learned = keys::known_hosts::learn_known_hosts(&self.host, self.port, &public_key).is_ok();
                        if !learned { self.verdict.set("Could not save the server's host key to ~/.ssh/known_hosts. Check that your home folder is writable.".into()); }
                        Ok(learned)
                    } else {
                        self.verdict.set(format!("The server presented a different host key ({presented}) than the one you confirmed. Nothing was saved; verify the new fingerprint before trusting it."));
                        Ok(false)
                    }
                } else {
                    self.verdict.set(format!("This server's host key is not saved yet. Server fingerprint: {presented}. Compare it with your provider's record, then tick 'Trust and save an unknown host key on first connection' and connect again."));
                    Ok(false)
                }
            }
            Err(keys::Error::KeyChanged { .. }) => {
                self.verdict.set("The server's host key has changed since it was saved. This can indicate a man-in-the-middle attack or a rebuilt server. Shelf will not replace the saved key automatically; remove the old entry from ~/.ssh/known_hosts only if you expected the change.".into());
                Ok(false)
            }
            Err(_) => {
                self.verdict.set("Could not check the server's host key against ~/.ssh/known_hosts.".into());
                Ok(false)
            }
        }
    }
}

pub fn app_home() -> PathBuf {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME");
    home.map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

pub fn key_hashes(algorithm: keys::Algorithm) -> Vec<Option<keys::HashAlg>> {
    // RSA uses SHA-2 signatures only; never request legacy ssh-rsa (SHA-1), which modern
    // servers reject and no agent should be asked to produce.
    if algorithm.is_rsa() { vec![Some(keys::HashAlg::Sha512), Some(keys::HashAlg::Sha256)] } else { vec![None] }
}

pub async fn authenticate_with_agent<S: keys::agent::client::AgentStream + Send + Unpin>(session: &mut client::Handle<HostVerifier>, username: &str, agent: &mut keys::agent::client::AgentClient<S>) -> bool {
    let identities = match agent.request_identities().await { Ok(identities) => identities, Err(_) => return false };
    for identity in identities {
        let public_key = identity.public_key().into_owned();
        for hash in key_hashes(public_key.algorithm()) {
            let result = session.authenticate_publickey_with(username, public_key.clone(), hash, agent).await;
            if result.map(|auth| auth.success()).unwrap_or(false) { return true; }
        }
    }
    false
}

#[cfg(windows)]
pub async fn authenticate_with_system_agent(session: &mut client::Handle<HostVerifier>, username: &str) -> bool {
    let mut agent = match keys::agent::client::AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await { Ok(agent) => agent, Err(_) => return false };
    authenticate_with_agent(session, username, &mut agent).await
}

#[cfg(unix)]
pub async fn authenticate_with_system_agent(session: &mut client::Handle<HostVerifier>, username: &str) -> bool {
    let mut agent = match keys::agent::client::AgentClient::connect_env().await { Ok(agent) => agent, Err(_) => return false };
    authenticate_with_agent(session, username, &mut agent).await
}

#[cfg(not(any(windows, unix)))]
pub async fn authenticate_with_system_agent(_: &mut client::Handle<HostVerifier>, _: &str) -> bool { false }

pub fn openssh_identity_files(host: &str, port: u16, username: &str) -> Vec<PathBuf> {
    let target = format!("{username}@{host}");
    let output = match Command::new("ssh").args(["-G", "-p", &port.to_string(), &target]).output() { Ok(output) if output.status.success() => output, _ => return Vec::new() };
    String::from_utf8_lossy(&output.stdout).lines().filter_map(|line| {
        let path = line.strip_prefix("identityfile ")?.trim();
        if path == "none" { return None; }
        Some(if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) { app_home().join(rest) } else { PathBuf::from(path) })
    }).collect()
}

pub fn candidate_key_paths(input: &ConnectionInput) -> Vec<PathBuf> {
    if input.private_key_path.trim().is_empty() {
        let mut paths = openssh_identity_files(input.host.trim(), input.port, input.username.trim());
        paths.extend(["id_ed25519", "id_ecdsa", "id_rsa"].into_iter().map(|name| app_home().join(".ssh").join(name)));
        paths
    } else { vec![PathBuf::from(input.private_key_path.trim())] }
}

pub async fn open_session(input: &ConnectionInput) -> Result<client::Handle<HostVerifier>, String> {
    let host = input.host.trim().to_owned();
    let config = Arc::new(client::Config::default());
    let verdict = HostKeyVerdict::default();
    let verifier = HostVerifier { host: host.clone(), port: input.port, trust_new_host: input.trust_new_host, confirmed_fingerprint: input.confirmed_fingerprint.clone(), verdict: verdict.clone() };
    client::connect(config, (host.as_str(), input.port), verifier).await.map_err(|e| verdict.message().unwrap_or_else(|| format!("SSH connection failed: {e}")))
}

pub async fn authenticate_key(session: &mut client::Handle<HostVerifier>, username: &str, key: keys::PrivateKey) -> bool {
    for hash in key_hashes(key.algorithm()) {
        let result = session.authenticate_publickey(username, keys::PrivateKeyWithHashAlg::new(Arc::new(key.clone()), hash)).await;
        if result.map(|auth| auth.success()).unwrap_or(false) { return true; }
    }
    false
}

pub async fn connect_authenticated(input: &ConnectionInput) -> Result<client::Handle<HostVerifier>, String> {
    if !input.password.trim().is_empty() && input.private_key_path.trim().is_empty() {
        let mut session = open_session(input).await?;
        let result = session.authenticate_password(input.username.trim(), &input.password).await.map_err(|e| format!("Authentication failed: {e}"))?;
        return result.success().then_some(session).ok_or_else(|| "Authentication was rejected by the server.".into());
    }

    let passphrase = if input.key_passphrase.is_empty() { None } else { Some(input.key_passphrase.as_str()) };
    let mut found_key = false;
    let mut rsa_key_path: Option<PathBuf> = None;
    for path in candidate_key_paths(input) {
        if !path.is_file() { continue; }
        found_key = true;
        let key = match keys::load_secret_key(&path, passphrase) { Ok(key) => key, Err(_) => continue };
        if key.algorithm().is_rsa() {
            // Shelf is built without the advisory-flagged RustCrypto rsa crate, so it never signs
            // with an RSA private key in-process. RSA authentication goes through the system SSH
            // agent instead; remember the key's path so the final error can name it specifically.
            // Note: an RSA key that fails to load (e.g. passphrase missing) falls through to the
            // generic message below - a known limitation, since we only learn the type by parsing.
            if rsa_key_path.is_none() { rsa_key_path = Some(path); }
            continue;
        }
        let mut session = open_session(input).await?;
        if authenticate_key(&mut session, input.username.trim(), key).await { return Ok(session); }
    }

    if input.private_key_path.trim().is_empty() {
        let mut session = open_session(input).await?;
        if authenticate_with_system_agent(&mut session, input.username.trim()).await { return Ok(session); }
    }
    if !input.private_key_path.trim().is_empty() {
        return Err(if let Some(path) = &rsa_key_path {
            format!("The selected key is an RSA key; Shelf does not sign with RSA key files directly. {} Then connect again with the key field left empty.", rsa_agent_steps(path))
        } else {
            "The selected private key could not authenticate with this server. Check the key file and passphrase. If it is an encrypted or older-format RSA key, Shelf cannot read it directly - load it into your SSH agent with ssh-add (the agent asks for its passphrase itself), then connect with the key field left empty.".into()
        });
    }
    if let Some(path) = &rsa_key_path {
        return Err(format!("Your RSA key ({}) is handled by your SSH agent, not by Shelf directly. {} Then connect again. You can also enter the account password instead.", display_path(path), rsa_agent_steps(path)));
    }
    if found_key { Err("The SSH agent and configured key files were not accepted. Enter a password or select the correct private key. If a key is an encrypted or older-format RSA key, Shelf cannot read it directly - load it into your SSH agent with ssh-add (the agent asks for its passphrase itself), then connect again.".into()) } else { Err("No password was entered and no usable SSH agent or configured key was found. Enter a password or select a private key.".into()) }
}

// OS-specific agent setup steps naming the actual key file. The Linux form warns that a
// terminal-started agent is invisible to an already-running GUI app, so Shelf must be
// launched from a shell that has the agent (or the desktop session's agent must be used).
#[cfg(windows)]
fn rsa_agent_steps(path: &Path) -> String {
    format!("Start the ssh-agent service if it is stopped (elevated PowerShell: Start-Service ssh-agent), then in PowerShell run: ssh-add \"{}\".", display_path(path))
}

#[cfg(not(windows))]
fn rsa_agent_steps(path: &Path) -> String {
    format!("In a terminal run: eval \"$(ssh-agent)\" && ssh-add \"{}\", then start Shelf from that same terminal so it can reach the agent (or add the key to your desktop session's agent if you already have one).", display_path(path))
}

pub fn display_path(path: &Path) -> String {
    let raw = path.to_string_lossy().into_owned();
    raw.strip_prefix(r"\\?\").unwrap_or(&raw).to_owned()
}

pub fn drives_listing() -> DirectoryListing {
    #[cfg(windows)]
    let entries = (b'A'..=b'Z').filter_map(|letter| {
        let root = PathBuf::from(format!("{}:\\", letter as char));
        root.exists().then(|| FileEntry { name: format!("{}: drive", letter as char), path: root.to_string_lossy().into_owned(), kind: "folder".into(), size: 0, modified: None })
    }).collect();
    #[cfg(not(windows))]
    let entries = vec![FileEntry { name: "Root filesystem".into(), path: "/".into(), kind: "folder".into(), size: 0, modified: None }];
    DirectoryListing { path: DRIVES_ROOT.into(), entries }
}

pub fn local_listing(path: PathBuf) -> Result<DirectoryListing, String> {
    let canonical = path.canonicalize().map_err(|e| format!("Cannot open folder: {e}"))?;
    let mut entries = std::fs::read_dir(&canonical)
        .map_err(|e| format!("Cannot read folder: {e}"))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            let kind = if metadata.is_dir() { "folder" } else { "file" }.to_owned();
            let modified = metadata.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|t| t.as_secs());
            Some(FileEntry { name: entry.file_name().to_string_lossy().into_owned(), path: display_path(&entry.path()), kind, size: metadata.len(), modified })
        }).collect::<Vec<_>>();
    entries.sort_by(|a, b| (a.kind != "folder", a.name.to_lowercase()).cmp(&(b.kind != "folder", b.name.to_lowercase())));
    Ok(DirectoryListing { path: display_path(&canonical), entries })
}

pub async fn remote_listing(sftp: &SftpSession, path: &str) -> Result<DirectoryListing, String> {
    let canonical = sftp.canonicalize(path).await.map_err(|e| format!("Cannot open remote folder: {e}"))?;
    let mut entries = sftp.read_dir(canonical.clone()).await.map_err(|e| format!("Cannot read remote folder: {e}"))?
        .map(|entry| {
            let metadata = entry.metadata();
            FileEntry { name: entry.file_name(), path: entry.path(), kind: if entry.file_type().is_dir() { "folder" } else { "file" }.to_owned(), size: metadata.len(), modified: metadata.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|t| t.as_secs()) }
        }).collect::<Vec<_>>();
    entries.sort_by(|a, b| (a.kind != "folder", a.name.to_lowercase()).cmp(&(b.kind != "folder", b.name.to_lowercase())));
    Ok(DirectoryListing { path: canonical, entries })
}


impl ShelfState {
    pub fn new() -> Self {
        ShelfState { sftp: Mutex::new(None), ssh: Mutex::new(None), terminals: Arc::new(Mutex::new(HashMap::new())), next_terminal_id: AtomicU64::new(1) }
    }
}

pub async fn list_local(path: Option<String>) -> Result<DirectoryListing, String> {
    match path.as_deref() {
        Some(DRIVES_ROOT) => Ok(drives_listing()),
        _ => local_listing(path.map(PathBuf::from).unwrap_or_else(app_home)),
    }
}

pub async fn connect_sftp(input: &ConnectionInput, state: &ShelfState) -> Result<DirectoryListing, String> {
    if input.host.trim().is_empty() || input.username.trim().is_empty() { return Err("Host and user name are required.".into()); }
    let session = connect_authenticated(input).await?;
    let channel = session.channel_open_session().await.map_err(|e| format!("Could not open SSH channel: {e}"))?;
    channel.request_subsystem(true, "sftp").await.map_err(|e| format!("Server does not provide SFTP: {e}"))?;
    let sftp = SftpSession::new(channel.into_stream()).await.map_err(|e| format!("Could not start SFTP: {e}"))?;
    let listing = remote_listing(&sftp, ".").await?;
    // A new connection replaces any previous session: drop stale terminals, SFTP and SSH handles.
    state.terminals.lock().await.clear();
    *state.sftp.lock().await = Some(sftp);
    *state.ssh.lock().await = Some(session);
    Ok(listing)
}

pub async fn open_terminal(state: &ShelfState, columns: u32, rows: u32, output: TerminalOutput) -> Result<u64, String> {
    let channel = {
        let guard = state.ssh.lock().await;
        let ssh = guard.as_ref().ok_or("Connect to a server first.")?;
        ssh.channel_open_session().await.map_err(|e| format!("Could not open terminal channel: {e}"))?
    };
    let columns = columns.clamp(1, 500);
    let rows = rows.clamp(1, 200);
    channel.request_pty(true, "xterm-256color", columns, rows, 0, 0, &[]).await.map_err(|e| format!("Could not request terminal: {e}"))?;
    channel.request_shell(true).await.map_err(|e| format!("Could not start remote shell: {e}"))?;
    let (mut reader, writer) = channel.split();
    let terminal_id = state.next_terminal_id.fetch_add(1, Ordering::Relaxed);
    state.terminals.lock().await.insert(terminal_id, writer);
    let terminals = Arc::clone(&state.terminals);
    tokio::spawn(async move {
        while let Some(message) = reader.wait().await {
            match message {
                ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => { output(String::from_utf8_lossy(&data).into_owned()); }
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => {}
            }
        }
        terminals.lock().await.remove(&terminal_id);
        output("\r\n[Remote shell closed]\r\n".into());
    });
    Ok(terminal_id)
}

pub async fn write_terminal(state: &ShelfState, terminal_id: u64, data: String) -> Result<(), String> {
    if data.len() > 65_536 { return Err("Terminal input is too large.".into()); }
    let guard = state.terminals.lock().await;
    let terminal = guard.get(&terminal_id).ok_or("Terminal is no longer open.")?;
    terminal.data_bytes(data).await.map_err(|e| format!("Could not send terminal input: {e}"))
}

pub async fn resize_terminal(state: &ShelfState, terminal_id: u64, columns: u32, rows: u32) -> Result<(), String> {
    let guard = state.terminals.lock().await;
    let terminal = guard.get(&terminal_id).ok_or("Terminal is no longer open.")?;
    terminal.window_change(columns.clamp(1, 500), rows.clamp(1, 200), 0, 0).await.map_err(|e| format!("Could not resize terminal: {e}"))
}

pub async fn close_terminal(state: &ShelfState, terminal_id: u64) -> Result<(), String> {
    // Idempotent: the reader task may already have removed the id after the remote shell exited.
    let terminal = match state.terminals.lock().await.remove(&terminal_id) {
        Some(terminal) => terminal,
        None => return Ok(()),
    };
    terminal.close().await.map_err(|e| format!("Could not close terminal: {e}"))
}

pub async fn list_remote(state: &ShelfState, path: String) -> Result<DirectoryListing, String> {
    let guard = state.sftp.lock().await;
    let sftp = guard.as_ref().ok_or("Connect to a server first.")?;
    remote_listing(sftp, &path).await
}

pub async fn upload_file(state: &ShelfState, local_path: String, remote_directory: String) -> Result<(), String> {
    let source = PathBuf::from(&local_path);
    let name = source.file_name().ok_or("Select a file, not a folder.")?.to_string_lossy();
    let remote_path = format!("{}/{}", remote_directory.trim_end_matches('/'), name);
    let guard = state.sftp.lock().await;
    let sftp = guard.as_ref().ok_or("Connect to a server first.")?;
    let mut local = tokio::fs::File::open(&source).await.map_err(|e| format!("Cannot read local file: {e}"))?;
    let mut remote = sftp.create(remote_path).await.map_err(|e| format!("Cannot create remote file: {e}"))?;
    tokio::io::copy(&mut local, &mut remote).await.map_err(|e| format!("Upload failed: {e}"))?;
    remote.shutdown().await.map_err(|e| format!("Upload did not close cleanly: {e}"))
}

pub async fn download_file(state: &ShelfState, remote_path: String, local_directory: String) -> Result<(), String> {
    let name = Path::new(&remote_path).file_name().ok_or("Invalid remote file path.")?.to_string_lossy();
    let local_path = PathBuf::from(local_directory).join(name.as_ref());
    let guard = state.sftp.lock().await;
    let sftp = guard.as_ref().ok_or("Connect to a server first.")?;
    let mut remote = sftp.open(remote_path).await.map_err(|e| format!("Cannot read remote file: {e}"))?;
    let mut local = tokio::fs::File::create(&local_path).await.map_err(|e| format!("Cannot create local file: {e}"))?;
    tokio::io::copy(&mut remote, &mut local).await.map_err(|e| format!("Download failed: {e}"))?;
    local.flush().await.map_err(|e| format!("Download did not finish cleanly: {e}"))
}

pub async fn delete_local_file(path: String) -> Result<(), String> {
    let metadata = tokio::fs::metadata(&path).await.map_err(|e| format!("Cannot inspect local file: {e}"))?;
    if !metadata.is_file() { return Err("Only files can be deleted here.".into()); }
    tokio::fs::remove_file(path).await.map_err(|e| format!("Could not delete local file: {e}"))
}

pub async fn delete_remote_file(state: &ShelfState, path: String) -> Result<(), String> {
    let guard = state.sftp.lock().await;
    let sftp = guard.as_ref().ok_or("Connect to a server first.")?;
    sftp.remove_file(path).await.map_err(|e| format!("Could not delete remote file: {e}"))
}
