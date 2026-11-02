use russh::{client, keys, ChannelMsg};
use russh::keys::PublicKeyOrCertificate;
use russh_sftp::client::SftpSession;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::{Path, PathBuf}, process::{Command, Stdio}, sync::{atomic::{AtomicU64, Ordering}, Arc}, time::Duration};
use tauri::{ipc::Channel, State};
use tokio::{io::{AsyncWriteExt}, sync::Mutex};

struct ShelfState {
    sftp: Mutex<Option<SftpSession>>,
    ssh: Mutex<Option<client::Handle<HostVerifier>>>,
    terminals: Mutex<HashMap<u64, russh::ChannelWriteHalf<client::Msg>>>,
    next_terminal_id: AtomicU64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionInput {
    host: String,
    port: u16,
    username: String,
    password: String,
    #[serde(default)]
    private_key_path: String,
    #[serde(default)]
    key_passphrase: String,
    trust_new_host: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct FileEntry {
    name: String,
    path: String,
    kind: String,
    size: u64,
    modified: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DirectoryListing {
    path: String,
    entries: Vec<FileEntry>,
}

const DRIVES_ROOT: &str = "shelf://drives";

struct HostVerifier {
    host: String,
    port: u16,
    trust_new_host: bool,
}

impl client::Handler for HostVerifier {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let public_key = key.public_key();
        match keys::check_known_hosts(&self.host, self.port, &public_key) {
            Ok(true) => Ok(true),
            Ok(false) if self.trust_new_host => {
                Ok(keys::known_hosts::learn_known_hosts(&self.host, self.port, &public_key).is_ok())
            }
            _ => Ok(false),
        }
    }
}

fn app_home() -> PathBuf {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME");
    home.map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

fn key_hashes(algorithm: keys::Algorithm) -> Vec<Option<keys::HashAlg>> {
    if algorithm.is_rsa() { vec![Some(keys::HashAlg::Sha512), Some(keys::HashAlg::Sha256), None] } else { vec![None] }
}

async fn authenticate_with_agent<S: keys::agent::client::AgentStream + Send + Unpin>(session: &mut client::Handle<HostVerifier>, username: &str, agent: &mut keys::agent::client::AgentClient<S>) -> bool {
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
async fn authenticate_with_system_agent(session: &mut client::Handle<HostVerifier>, username: &str) -> bool {
    let mut agent = match keys::agent::client::AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await { Ok(agent) => agent, Err(_) => return false };
    authenticate_with_agent(session, username, &mut agent).await
}

#[cfg(unix)]
async fn authenticate_with_system_agent(session: &mut client::Handle<HostVerifier>, username: &str) -> bool {
    let mut agent = match keys::agent::client::AgentClient::connect_env().await { Ok(agent) => agent, Err(_) => return false };
    authenticate_with_agent(session, username, &mut agent).await
}

#[cfg(not(any(windows, unix)))]
async fn authenticate_with_system_agent(_: &mut client::Handle<HostVerifier>, _: &str) -> bool { false }

fn openssh_identity_files(host: &str, port: u16, username: &str) -> Vec<PathBuf> {
    let target = format!("{username}@{host}");
    let output = match Command::new("ssh").args(["-G", "-p", &port.to_string(), &target]).output() { Ok(output) if output.status.success() => output, _ => return Vec::new() };
    String::from_utf8_lossy(&output.stdout).lines().filter_map(|line| {
        let path = line.strip_prefix("identityfile ")?.trim();
        if path == "none" { return None; }
        Some(if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) { app_home().join(rest) } else { PathBuf::from(path) })
    }).collect()
}

fn candidate_key_paths(input: &ConnectionInput) -> Vec<PathBuf> {
    if input.private_key_path.trim().is_empty() {
        let mut paths = openssh_identity_files(input.host.trim(), input.port, input.username.trim());
        paths.extend(["id_ed25519", "id_ecdsa", "id_rsa"].into_iter().map(|name| app_home().join(".ssh").join(name)));
        paths
    } else { vec![PathBuf::from(input.private_key_path.trim())] }
}

async fn open_session(input: &ConnectionInput) -> Result<client::Handle<HostVerifier>, String> {
    let host = input.host.trim().to_owned();
    let config = Arc::new(client::Config::default());
    let verifier = HostVerifier { host: host.clone(), port: input.port, trust_new_host: input.trust_new_host };
    client::connect(config, (host.as_str(), input.port), verifier).await.map_err(|e| format!("SSH connection failed: {e}"))
}

async fn authenticate_key(session: &mut client::Handle<HostVerifier>, username: &str, key: keys::PrivateKey) -> bool {
    for hash in key_hashes(key.algorithm()) {
        let result = session.authenticate_publickey(username, keys::PrivateKeyWithHashAlg::new(Arc::new(key.clone()), hash)).await;
        if result.map(|auth| auth.success()).unwrap_or(false) { return true; }
    }
    false
}

async fn connect_authenticated(input: &ConnectionInput) -> Result<client::Handle<HostVerifier>, String> {
    if !input.password.trim().is_empty() && input.private_key_path.trim().is_empty() {
        let mut session = open_session(input).await?;
        let result = session.authenticate_password(input.username.trim(), &input.password).await.map_err(|e| format!("Authentication failed: {e}"))?;
        return result.success().then_some(session).ok_or_else(|| "Authentication was rejected by the server.".into());
    }

    let passphrase = if input.key_passphrase.is_empty() { None } else { Some(input.key_passphrase.as_str()) };
    let mut found_key = false;
    for path in candidate_key_paths(input) {
        if !path.is_file() { continue; }
        found_key = true;
        let key = match keys::load_secret_key(&path, passphrase) { Ok(key) => key, Err(_) => continue };
        let mut session = open_session(input).await?;
        if authenticate_key(&mut session, input.username.trim(), key).await { return Ok(session); }
    }

    if input.private_key_path.trim().is_empty() {
        let mut session = open_session(input).await?;
        if authenticate_with_system_agent(&mut session, input.username.trim()).await { return Ok(session); }
    }
    if !input.private_key_path.trim().is_empty() { return Err("The selected private key could not authenticate with this server. Check the key file and passphrase.".into()); }
    if found_key { Err("The SSH agent and configured key files were not accepted. Enter a password or select the correct private key.".into()) } else { Err("No password was entered and no usable SSH agent or configured key was found. Enter a password or select a private key.".into()) }
}

fn display_path(path: &Path) -> String {
    let raw = path.to_string_lossy().into_owned();
    raw.strip_prefix(r"\\?\").unwrap_or(&raw).to_owned()
}

fn drives_listing() -> DirectoryListing {
    #[cfg(windows)]
    let entries = (b'A'..=b'Z').filter_map(|letter| {
        let root = PathBuf::from(format!("{}:\\", letter as char));
        root.exists().then(|| FileEntry { name: format!("{}: drive", letter as char), path: root.to_string_lossy().into_owned(), kind: "folder".into(), size: 0, modified: None })
    }).collect();
    #[cfg(not(windows))]
    let entries = vec![FileEntry { name: "Root filesystem".into(), path: "/".into(), kind: "folder".into(), size: 0, modified: None }];
    DirectoryListing { path: DRIVES_ROOT.into(), entries }
}

fn local_listing(path: PathBuf) -> Result<DirectoryListing, String> {
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

async fn remote_listing(sftp: &SftpSession, path: &str) -> Result<DirectoryListing, String> {
    let canonical = sftp.canonicalize(path).await.map_err(|e| format!("Cannot open remote folder: {e}"))?;
    let mut entries = sftp.read_dir(canonical.clone()).await.map_err(|e| format!("Cannot read remote folder: {e}"))?
        .map(|entry| {
            let metadata = entry.metadata();
            FileEntry { name: entry.file_name(), path: entry.path(), kind: if entry.file_type().is_dir() { "folder" } else { "file" }.to_owned(), size: metadata.len(), modified: metadata.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|t| t.as_secs()) }
        }).collect::<Vec<_>>();
    entries.sort_by(|a, b| (a.kind != "folder", a.name.to_lowercase()).cmp(&(b.kind != "folder", b.name.to_lowercase())));
    Ok(DirectoryListing { path: canonical, entries })
}

#[tauri::command]
async fn list_local(path: Option<String>) -> Result<DirectoryListing, String> {
    match path.as_deref() {
        Some(DRIVES_ROOT) => Ok(drives_listing()),
        _ => local_listing(path.map(PathBuf::from).unwrap_or_else(app_home)),
    }
}

#[tauri::command]
async fn connect_sftp(input: ConnectionInput, state: State<'_, ShelfState>) -> Result<DirectoryListing, String> {
    if input.host.trim().is_empty() || input.username.trim().is_empty() { return Err("Host and user name are required.".into()); }
    let session = connect_authenticated(&input).await?;
    let channel = session.channel_open_session().await.map_err(|e| format!("Could not open SSH channel: {e}"))?;
    channel.request_subsystem(true, "sftp").await.map_err(|e| format!("Server does not provide SFTP: {e}"))?;
    let sftp = SftpSession::new(channel.into_stream()).await.map_err(|e| format!("Could not start SFTP: {e}"))?;
    let listing = remote_listing(&sftp, ".").await?;
    *state.sftp.lock().await = Some(sftp);
    *state.ssh.lock().await = Some(session);
    Ok(listing)
}

#[tauri::command]
async fn authenticate_system_ssh(input: ConnectionInput) -> Result<(), String> {
    if input.host.trim().is_empty() || input.username.trim().is_empty() { return Err("Host and user name are required.".into()); }
    let destination = format!("{}@{}", input.username.trim(), input.host.trim());
    let mut child = tokio::process::Command::new("ssh")
        .args(["-N", "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes", "-o", "ConnectTimeout=8", "-p", &input.port.to_string(), &destination])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "System OpenSSH was not found. Install the OpenSSH client or select a private key.".to_string())?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    match child.try_wait().map_err(|e| format!("Could not check system SSH: {e}"))? {
        None => { child.kill().await.map_err(|e| format!("Could not close SSH verification: {e}"))?; Ok(()) }
        Some(_) => Err("System OpenSSH did not establish an authenticated session. Check the host key and your SSH configuration.".into()),
    }
}

#[tauri::command]
async fn open_terminal(columns: u32, rows: u32, output: Channel<String>, state: State<'_, ShelfState>) -> Result<u64, String> {
    let channel = {
        let guard = state.ssh.lock().await;
        let ssh = guard.as_ref().ok_or("Connect to a server first.")?;
        ssh.channel_open_session().await.map_err(|e| format!("Could not open terminal channel: {e}"))?
    };
    let columns = columns.clamp(1, 500);
    let rows = rows.clamp(1, 200);
    channel.request_pty(false, "xterm-256color", columns, rows, 0, 0, &[]).await.map_err(|e| format!("Could not request terminal: {e}"))?;
    channel.request_shell(false).await.map_err(|e| format!("Could not start remote shell: {e}"))?;
    let (mut reader, writer) = channel.split();
    let terminal_id = state.next_terminal_id.fetch_add(1, Ordering::Relaxed);
    state.terminals.lock().await.insert(terminal_id, writer);
    tauri::async_runtime::spawn(async move {
        while let Some(message) = reader.wait().await {
            match message {
                ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => { let _ = output.send(String::from_utf8_lossy(&data).into_owned()); }
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => {}
            }
        }
    });
    Ok(terminal_id)
}

#[tauri::command]
async fn write_terminal(terminal_id: u64, data: String, state: State<'_, ShelfState>) -> Result<(), String> {
    if data.len() > 65_536 { return Err("Terminal input is too large.".into()); }
    let guard = state.terminals.lock().await;
    let terminal = guard.get(&terminal_id).ok_or("Terminal is no longer open.")?;
    terminal.data_bytes(data).await.map_err(|e| format!("Could not send terminal input: {e}"))
}

#[tauri::command]
async fn resize_terminal(terminal_id: u64, columns: u32, rows: u32, state: State<'_, ShelfState>) -> Result<(), String> {
    let guard = state.terminals.lock().await;
    let terminal = guard.get(&terminal_id).ok_or("Terminal is no longer open.")?;
    terminal.window_change(columns.clamp(1, 500), rows.clamp(1, 200), 0, 0).await.map_err(|e| format!("Could not resize terminal: {e}"))
}

#[tauri::command]
async fn close_terminal(terminal_id: u64, state: State<'_, ShelfState>) -> Result<(), String> {
    let terminal = state.terminals.lock().await.remove(&terminal_id).ok_or("Terminal is no longer open.")?;
    terminal.close().await.map_err(|e| format!("Could not close terminal: {e}"))
}

#[tauri::command]
async fn list_remote(path: String, state: State<'_, ShelfState>) -> Result<DirectoryListing, String> {
    let guard = state.sftp.lock().await;
    let sftp = guard.as_ref().ok_or("Connect to a server first.")?;
    remote_listing(sftp, &path).await
}

#[tauri::command]
async fn upload_file(local_path: String, remote_directory: String, state: State<'_, ShelfState>) -> Result<(), String> {
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

#[tauri::command]
async fn download_file(remote_path: String, local_directory: String, state: State<'_, ShelfState>) -> Result<(), String> {
    let name = Path::new(&remote_path).file_name().ok_or("Invalid remote file path.")?.to_string_lossy();
    let local_path = PathBuf::from(local_directory).join(name.as_ref());
    let guard = state.sftp.lock().await;
    let sftp = guard.as_ref().ok_or("Connect to a server first.")?;
    let mut remote = sftp.open(remote_path).await.map_err(|e| format!("Cannot read remote file: {e}"))?;
    let mut local = tokio::fs::File::create(&local_path).await.map_err(|e| format!("Cannot create local file: {e}"))?;
    tokio::io::copy(&mut remote, &mut local).await.map_err(|e| format!("Download failed: {e}"))?;
    local.flush().await.map_err(|e| format!("Download did not finish cleanly: {e}"))
}

#[tauri::command]
async fn delete_local_file(path: String) -> Result<(), String> {
    let metadata = tokio::fs::metadata(&path).await.map_err(|e| format!("Cannot inspect local file: {e}"))?;
    if !metadata.is_file() { return Err("Only files can be deleted here.".into()); }
    tokio::fs::remove_file(path).await.map_err(|e| format!("Could not delete local file: {e}"))
}

#[tauri::command]
async fn delete_remote_file(path: String, state: State<'_, ShelfState>) -> Result<(), String> {
    let guard = state.sftp.lock().await;
    let sftp = guard.as_ref().ok_or("Connect to a server first.")?;
    sftp.remove_file(path).await.map_err(|e| format!("Could not delete remote file: {e}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(ShelfState { sftp: Mutex::new(None), ssh: Mutex::new(None), terminals: Mutex::new(HashMap::new()), next_terminal_id: AtomicU64::new(1) })
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![list_local, connect_sftp, authenticate_system_ssh, open_terminal, write_terminal, resize_terminal, close_terminal, list_remote, upload_file, download_file, delete_local_file, delete_remote_file])
        .run(tauri::generate_context!())
        .expect("error while running Shelf");
}
