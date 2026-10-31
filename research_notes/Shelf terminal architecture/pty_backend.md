# Shelf PTY backend

## Can a real Bash terminal work on both Windows and Linux? What should run on each OS by default?

### Takeaway
A PTY-backed terminal can present the same VT/UTF-8 terminal UI on Windows and Linux, but Bash is not a Windows system component. Default to the user's native shell: `%ComSpec%`/`cmd.exe` on Windows and the login shell (`$SHELL`, then the password database, then `/bin/sh`) on Linux. Offer Bash on Windows only as an explicit, detected profile (for example, installed Git Bash or WSL), with its correct executable path and launch arguments.

### Cited Findings
- `portable-pty`'s `CommandBuilder::new_default_prog` chooses `$SHELL`, then the password database, and falls back to `/bin/sh` on Unix; on Windows it resolves `ComSpec` and otherwise uses `cmd.exe`. [portable-pty source](https://docs.rs/portable-pty/latest/src/portable_pty/cmdbuilder.rs.html)
- The crate's `CommandBuilder` can also launch an explicitly selected program with a separate argument vector and working directory; it does not require command-line string construction. [CommandBuilder API](https://docs.rs/portable-pty/latest/portable_pty/cmdbuilder/struct.CommandBuilder.html)
- ConPTY transports UTF-8 plain text interleaved with virtual-terminal sequences. Input uses text for ordinary keys and VT sequences for control keys and mouse input, so its byte stream is suitable for a terminal emulator front end. [Microsoft CreatePseudoConsole](https://learn.microsoft.com/en-us/windows/console/createpseudoconsole)
- Linux describes the master/slave PTY relationship as asynchronous; writing Ctrl-C to the master can produce `SIGINT` for the foreground process group on the slave. New Linux code should use UNIX 98 PTYs rather than legacy BSD PTYs. [Linux pty(7)](https://man7.org/linux/man-pages/man7/pty.7.html)

### Inferences
- A local Shelf terminal should be described as a *native shell terminal*, not a cross-platform Bash terminal. This preserves user expectations and avoids silently depending on WSL, Git Bash, Cygwin, or MSYS2.
- A Windows Bash profile should never be implemented by sending a user-provided command string to `cmd.exe /c`. Store profile `program` and `args` separately, validate that the program exists, and launch it directly.
- Sensible built-in profiles are: Windows Command Prompt (default), optional PowerShell/Pwsh, and optional discovered Git Bash or WSL; Linux login shell (default), with explicit Bash only when `/bin/bash` exists.

### Gaps
- The intended Windows Bash provider is not specified. Git Bash, WSL, MSYS2, and Cygwin have different executable paths, environment semantics, and working-directory conversion rules; select and test each as a separate optional profile.

## Which Rust libraries and operating-system APIs are mature enough for a Tauri app?

### Takeaway
Use `portable-pty` as Shelf's first implementation: it is a stable cross-platform abstraction extracted from WezTerm and exposes the operations Shelf needs (open, spawn, read, write, resize, wait, kill). Keep direct ConPTY code out of the application unless a tested feature gap requires it. `conpty` is Windows-only and narrower; `rust-pty` has a Tokio-native interface but is a much newer alternative that needs its own compatibility testing before adoption.

### Cited Findings
- `portable-pty` 0.9.0 provides a cross-platform system PTY API and runtime-selectable implementations; it is part of WezTerm. Its documented example opens a sized PTY, spawns a shell, reads the master, and writes input to the master. [portable-pty crate docs](https://docs.rs/portable-pty/latest/portable_pty/index.html)
- Its `MasterPty` trait has `resize`, `try_clone_reader`, and `take_writer`; `resize` updates kernel size information and signals the child to update. [MasterPty API](https://docs.rs/portable-pty/latest/portable_pty/trait.MasterPty.html)
- `portable-pty`'s native factory returns a `Send` runtime-selected `PtySystem`; opening it returns a master/slave pair. [native factory](https://docs.rs/portable-pty/latest/portable_pty/fn.native_pty_system.html) [PtySystem API](https://docs.rs/portable-pty/latest/portable_pty/trait.PtySystem.html)
- Windows ConPTY is the native PTY API for Windows 10 version 1809 and later. It requires a size, synchronous input/output handles, a pseudoconsole handle, and a child process attached through `STARTUPINFOEX`/`PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`. [Microsoft CreatePseudoConsole](https://learn.microsoft.com/en-us/windows/console/createpseudoconsole) [Microsoft session walkthrough](https://learn.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session)
- Microsoft warns that ConPTY input and output channels must be serviced on separate threads, otherwise full blocking buffers can deadlock the host. [Microsoft session walkthrough](https://learn.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session)
- The standalone `conpty` crate 0.7.0 exposes a Windows ConPTY interface but its latest listed release is from 2024-09-23 and its stated origin is an `expectrl` Windows backend. [conpty crate](https://docs.rs/crate/conpty/latest)
- `rust-pty` 0.6.1 advertises one async API over Unix `rustix` and Windows ConPTY (Windows 10 1809+), with Tokio read/write support. [rust-pty crate](https://docs.rs/rust-pty/latest/rust_pty/)
- Tauri 2 executes async commands on a separate async task and recommends them for heavy work. Its IPC channels are the documented streaming mechanism for chunked frontend data. [Tauri command and channel docs](https://v2.tauri.app/develop/calling-rust/)

### Inferences
- `portable-pty` is the lowest-risk choice for current Shelf because its synchronous `Read`/`Write` handles can run in dedicated `std::thread` workers, while Shelf already has Tokio/Tauri for state and IPC. This also follows ConPTY's separate-worker requirement.
- Do not pick direct Win32/ConPTY plus Linux `openpty` unless Shelf needs a feature unavailable from `portable-pty`. That duplicates platform lifecycle, handle inheritance, process-group, and resize work.
- Do not use `std::process::Command` pipes as a substitute: pipes do not give the child a terminal, so many programs change their behavior (line buffering, no colors, no full-screen UI).

### Gaps
- This review did not run the candidate crates on Shelf's supported Windows and Linux matrix. Before adoption, validate spawn, resize, Ctrl-C, Unicode, full-screen TUI, shell exit, and app shutdown on every supported OS version.

## What backend architecture best fits Shelf's existing Rust and Tauri stack?

### Takeaway
Create a local-terminal service with one opaque session ID per tab, backed by `portable-pty` and bounded worker channels. The frontend sends input and dimensions through short Tauri commands; Rust streams raw output chunks and lifecycle events to only the owning webview through a Tauri `Channel` or targeted event. Never hold Shelf's SFTP mutex while starting or operating a terminal.

### Cited Findings
- Shelf currently uses Tauri 2, Tokio, `serde`, and `tokio::sync::Mutex`; its `ShelfState` contains only an optional `russh_sftp::client::SftpSession`. [Shelf Rust backend](../../src-tauri/src/lib.rs)
- Tauri documents async commands for non-blocking work and recommends channels for streaming chunked data; window-specific events can be emitted to a designated webview. [Tauri command and channel docs](https://v2.tauri.app/develop/calling-rust/)
- `portable-pty` provides the reader, writer, resize, child wait, and child termination responsibilities needed by a session manager. [portable-pty crate docs](https://docs.rs/portable-pty/latest/portable_pty/index.html) [MasterPty API](https://docs.rs/portable-pty/latest/portable_pty/trait.MasterPty.html)
- On ConPTY, the host is responsible for collecting input, presenting output, closing the pseudoconsole handle, and using synchronous I/O. [Microsoft CreatePseudoConsole](https://learn.microsoft.com/en-us/windows/console/createpseudoconsole)

### Inferences
- Recommended Rust shape:
  1. `TerminalManager` managed separately from `ShelfState`, with `HashMap<TerminalId, TerminalSession>` behind a mutex.
  2. `terminal_open(profile, cwd, rows, cols, output_channel)` validates inputs, opens `native_pty_system()`, builds `CommandBuilder` from an allowlisted profile, and returns an opaque ID.
  3. One blocking reader worker owns a cloned PTY reader and forwards fixed-size byte chunks through a bounded channel. One writer worker owns the PTY writer and receives input chunks from another bounded channel. A third task waits for the child and emits one exit result.
  4. `terminal_write(id, bytes)` and `terminal_resize(id, rows, cols)` only find the session and enqueue/perform work; `terminal_close(id)` closes input, asks the child to end, and force-kills after a bounded grace period.
  5. Output chunks must remain bytes until the terminal emulator consumes them. UTF-8 decoding happens at the renderer boundary only; splitting multibyte characters or VT control sequences per backend chunk is valid and expected.
- Use bounded queues and an output size/scrollback limit. Otherwise a stalled webview or a command that emits unlimited output can exhaust application memory. Preserve input order; apply the newest valid resize, with rows/columns clamped to a conservative positive range.
- On window close, app exit, or SFTP disconnect, close only the related terminal session. A local shell is independent of the existing SFTP session; it should not be represented as remote SSH access unless a separate SSH-shell feature is deliberately designed.
- For a later remote terminal feature, reuse the frontend terminal protocol but create an SSH channel with a remote PTY and shell request. Do not tunnel local PTY traffic through SFTP, which is a different SSH subsystem.

### Gaps
- The frontend terminal renderer is not yet selected. It must correctly consume VT streams, report resize dimensions, send keyboard/paste bytes without browser shortcut conflicts, and cap scrollback. Verify its license, bundle size, accessibility, and input behavior before implementation.

## What security trade-offs apply to an embedded terminal?

### Takeaway
An embedded local terminal intentionally gives the user a process running with Shelf's privileges. Treat terminal creation as a privileged local capability: explicit user action, fixed launch profiles, validated working directories, session ownership checks, bounded resource use, and no hidden command execution. A terminal is not a sandbox.

### Cited Findings
- Windows creates a child process in the security context of the calling process. [Microsoft CreateProcess](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessa)
- The ConPTY API requires the terminal host to collect input and render output, so all input sent by the frontend reaches the child session through the host-controlled stream. [Microsoft CreatePseudoConsole](https://learn.microsoft.com/en-us/windows/console/createpseudoconsole)
- `portable-pty` accepts a program and argument vector through `CommandBuilder`, enabling direct process launch instead of passing an assembled shell command line. [CommandBuilder API](https://docs.rs/portable-pty/latest/portable_pty/cmdbuilder/struct.CommandBuilder.html)
- Tauri events are asynchronous and global events are delivered to all listeners; Tauri also supports targeting a specific webview. [Tauri events](https://v2.tauri.app/develop/calling-rust/)

### Inferences
- Keep only explicit, local profiles in the first release. Do not expose an unrestricted `program`, arbitrary environment override, or raw command-line field to frontend code or persisted connection data.
- Canonicalize and authorize the working directory before spawn. Do not rely on a user-supplied shell-quoted string; program paths, arguments, and environment entries are separate typed fields.
- Attach each session to its creating webview label and use a per-session unguessable ID. Send output only to that window; reject cross-window read, write, resize, and close requests. This reduces accidental leakage of terminal output, which can contain credentials or host data.
- Do not log keystrokes, raw terminal output, passwords, or shell command history. Redact structured error messages and avoid returning executable paths or environment values unless the user requested diagnostics.
- Prevent terminal output from becoming HTML. Render VT output in a terminal emulator/text canvas; never inject it with `innerHTML`. Treat clipboard paste as user input and show a multi-line paste warning at the UI level.
- Clearly label local versus future remote sessions. Remote shell use would require its own host-key verification and authorization policy; Shelf's present `trust_new_host` switch is insufficient for a secure terminal trust UX without fingerprint review.

### Gaps
- No threat model or supported multi-window behavior was supplied. Decide whether terminal profiles and working directories are trusted local preferences, and whether remote-shell support is in scope, before persisting any terminal configuration.
