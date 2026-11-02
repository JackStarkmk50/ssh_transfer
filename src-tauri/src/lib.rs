mod ssh;

use ssh::{ConnectionInput, DirectoryListing, ShelfState, TerminalOutput};
use tauri::{ipc::Channel, State};

#[tauri::command]
async fn list_local(path: Option<String>) -> Result<DirectoryListing, String> {
    ssh::list_local(path).await
}

#[tauri::command]
async fn connect_sftp(input: ConnectionInput, state: State<'_, ShelfState>) -> Result<DirectoryListing, String> {
    ssh::connect_sftp(&input, state.inner()).await
}

#[tauri::command]
async fn open_terminal(columns: u32, rows: u32, output: Channel<String>, state: State<'_, ShelfState>) -> Result<u64, String> {
    let sink: TerminalOutput = std::sync::Arc::new(move |data: String| { let _ = output.send(data); });
    ssh::open_terminal(state.inner(), columns, rows, sink).await
}

#[tauri::command]
async fn write_terminal(terminal_id: u64, data: String, state: State<'_, ShelfState>) -> Result<(), String> {
    ssh::write_terminal(state.inner(), terminal_id, data).await
}

#[tauri::command]
async fn resize_terminal(terminal_id: u64, columns: u32, rows: u32, state: State<'_, ShelfState>) -> Result<(), String> {
    ssh::resize_terminal(state.inner(), terminal_id, columns, rows).await
}

#[tauri::command]
async fn close_terminal(terminal_id: u64, state: State<'_, ShelfState>) -> Result<(), String> {
    ssh::close_terminal(state.inner(), terminal_id).await
}

#[tauri::command]
async fn list_remote(path: String, state: State<'_, ShelfState>) -> Result<DirectoryListing, String> {
    ssh::list_remote(state.inner(), path).await
}

#[tauri::command]
async fn upload_file(local_path: String, remote_directory: String, state: State<'_, ShelfState>) -> Result<(), String> {
    ssh::upload_file(state.inner(), local_path, remote_directory).await
}

#[tauri::command]
async fn download_file(remote_path: String, local_directory: String, state: State<'_, ShelfState>) -> Result<(), String> {
    ssh::download_file(state.inner(), remote_path, local_directory).await
}

#[tauri::command]
async fn delete_local_file(path: String) -> Result<(), String> {
    ssh::delete_local_file(path).await
}

#[tauri::command]
async fn delete_remote_file(path: String, state: State<'_, ShelfState>) -> Result<(), String> {
    ssh::delete_remote_file(state.inner(), path).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(ShelfState::new())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![list_local, connect_sftp, open_terminal, write_terminal, resize_terminal, close_terminal, list_remote, upload_file, download_file, delete_local_file, delete_remote_file])
        .run(tauri::generate_context!())
        .expect("error while running Shelf");
}
