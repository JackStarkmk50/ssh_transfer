# Shelf

**A small, local-first SFTP client for moving files with the SSH connection you already use.**

Shelf replaces repetitive `scp` commands with a focused desktop workspace: your computer on one side, the remote SFTP server on the other. Connect, browse, and move files without a cloud account, a separate service, or a terminal open beside you.

## What it does

- Connects to an existing SSH/SFTP server with a hostname or IP address, port, user name, and password.
- Verifies SSH host keys using your local `known_hosts` file.
- Lets you explicitly trust and save a new host key on its first verified connection.
- Browses local folders, mounted drives, and remote SFTP folders in a two-pane file browser.
- Uploads a selected local file to the current remote folder.
- Downloads a selected remote file to the current local folder.
- Builds native installers for Windows and Linux from one codebase.

Shelf never saves your password. It is used only for the active connection.

## Connect to a server

If you normally connect in a terminal with:

```bash
ssh rajendran@192.168.1.50
```

use these values in Shelf:

| Shelf field | Value |
| --- | --- |
| Host | `192.168.1.50` |
| Port | `22` |
| User name | `rajendran` |
| Password | Your normal SSH password |

`22` is the standard SSH port. If you use `ssh -p 2222 rajendran@192.168.1.50`, enter `2222` instead.

For a new server, verify that the address is correct before choosing **Trust and save an unknown host key**. Shelf will reject a changed host key on later connections.

## Browse and transfer

- Use the local Home button to open the available drives on your computer.
- Double-click a folder to open it; use the up-arrow to return to its parent.
- Connect to a server to enable the remote pane.
- Choose **Upload to server** to select a local file and send it to the open remote folder.
- Select a remote file, then choose **Download** to copy it into the open local folder.

## Install

Download the Windows setup executable or MSI from the project’s GitHub Releases page once a release has been published.

For Linux, each release includes AppImage, Debian (`.deb`), and RPM (`.rpm`) packages.

## Develop locally

### Requirements

- Node.js 20 or later
- Rust stable toolchain via [Rustup](https://rustup.rs/)
- Windows: Microsoft C++ Build Tools

### Run in development

```powershell
npm install
npm run tauri dev
```

### Create production installers

```powershell
npm run tauri build
```

Windows installers are created under:

```text
src-tauri/target/release/bundle/
```

## Release a version

The GitHub Actions workflow runs when a version tag beginning with `v` is pushed. It builds Windows x64 and Linux x64 packages and attaches them to the same GitHub Release.

Before creating a release, update the version consistently in:

- `package.json`
- `src-tauri/Cargo.toml`
- `src-tauri/tauri.conf.json`

Then commit and publish the tag:

```powershell
git add .
git commit -m "Release v0.1.1"
git push origin main
git tag v0.1.1
git push origin v0.1.1
```

GitHub Actions publishes Windows MSI and setup installers, plus Linux AppImage, DEB, and RPM packages.

## Current scope

Shelf currently supports password-based SFTP connections and single-file uploads and downloads. SSH key authentication, recursive folder transfers, transfer queues, resume support, and saved connection profiles are intentionally not represented as finished features yet.

## Technology

- [Tauri 2](https://tauri.app/) desktop shell
- Rust and Tokio for the native SFTP core
- `russh` and `russh-sftp` for SSH/SFTP
- React and TypeScript for the interface
