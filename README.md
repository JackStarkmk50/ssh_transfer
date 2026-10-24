# Shelf

A small, local-first desktop file-transfer client for SFTP over SSH.

## First release scope

- Connect to an existing SSH/SFTP server with password or private-key authentication.
- Browse local and remote directories in two panes.
- Upload and download files and folders, with progress and cancellation.
- Verify host keys and preserve known-host decisions.

The current UI is an interactive prototype. The next implementation phase connects its controls to the Rust SFTP service and replaces sample files with real directory listings.

## Development setup

The project requires Node.js 20+ and the stable Rust toolchain with Microsoft C++ Build Tools on Windows. Then run:

```powershell
npm install
npm run tauri dev
```

For the web-only interface preview:

```powershell
npm run dev
```
