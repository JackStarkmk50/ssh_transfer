use russh::{client, keys};
use russh::keys::PublicKeyOrCertificate;
use russh_sftp::client::SftpSession;
use serde::{Deserialize, Serialize};
use std::{path::{Path, PathBuf}, sync::Arc};
use tauri::State;
use tokio::{io::{AsyncWriteExt}, sync::Mutex};

struct ShelfState {
    sftp: Mutex<Option<SftpSession>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionInput {
    host: String,
    port: u16,
    username: String,
    password: String,
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
    std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
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
    let host = input.host.trim().to_owned();
    let config = Arc::new(client::Config::default());
    let verifier = HostVerifier { host: host.clone(), port: input.port, trust_new_host: input.trust_new_host };
    let mut session = client::connect(config, (host.as_str(), input.port), verifier).await.map_err(|e| format!("SSH connection failed: {e}"))?;
    let authenticated = session.authenticate_password(input.username.trim(), &input.password).await.map_err(|e| format!("Authentication failed: {e}"))?;
    if !authenticated.success() { return Err("Authentication was rejected by the server.".into()); }
    let channel = session.channel_open_session().await.map_err(|e| format!("Could not open SSH channel: {e}"))?;
    channel.request_subsystem(true, "sftp").await.map_err(|e| format!("Server does not provide SFTP: {e}"))?;
    let sftp = SftpSession::new(channel.into_stream()).await.map_err(|e| format!("Could not start SFTP: {e}"))?;
    let listing = remote_listing(&sftp, ".").await?;
    *state.sftp.lock().await = Some(sftp);
    Ok(listing)
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(ShelfState { sftp: Mutex::new(None) })
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![list_local, connect_sftp, list_remote, upload_file, download_file])
        .run(tauri::generate_context!())
        .expect("error while running Shelf");
}
