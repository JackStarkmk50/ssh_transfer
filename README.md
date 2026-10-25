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

## Publishing a GitHub release

The GitHub Actions workflow builds and publishes the Windows x64 installers whenever a tag beginning with `v` is pushed. Before tagging, update the version consistently in `package.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json`.

```powershell
git tag v0.1.0
git push origin v0.1.0
```

GitHub Actions creates the release and uploads the MSI and NSIS setup executable.
