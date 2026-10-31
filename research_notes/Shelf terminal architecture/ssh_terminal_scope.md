# Shelf terminal architecture

## Should Shelf provide a local terminal, a remote SSH terminal, or both?

### Takeaway
Release 1 should provide one optional **remote SSH terminal** associated with the connected Shelf profile. It should not run local commands, and it should not add forwarding, agent forwarding, X11, saved terminal transcripts, command history persistence, or an arbitrary remote-command API. This keeps the feature aligned with Shelf's SFTP purpose and avoids giving WebView-originated input an additional local-code-execution path.

### Cited Findings
- SSH defines a session as a remote shell, application, command, or subsystem; SFTP is therefore one session use, while an interactive shell is another. Only one shell, command, or subsystem request can succeed on a single session channel. [RFC 4254, sections 6 and 6.5](https://datatracker.ietf.org/doc/html/rfc4254#section-6)
- An interactive terminal needs a `pty-req` with a terminal name, dimensions, and modes before the `shell` request; the protocol also defines a `window-change` request when dimensions change. [RFC 4254, sections 6.2 and 6.7](https://datatracker.ietf.org/doc/html/rfc4254#section-6.2)
- `russh` exposes the exact client-side operations needed for a remote terminal: open a session, request a PTY, request a shell, write data, receive channel messages, resize, and close. [russh Channel API](https://docs.rs/russh/0.63.3/russh/struct.Channel.html)
- Tauri separates the WebView from Rust application code; data passed over IPC is a trust boundary, and core code has full system-resource access. [Tauri security documentation](https://v2.tauri.app/security/)
- OpenSSH treats host-key verification as protection against man-in-the-middle attacks; its strict mode never automatically accepts changed keys. [OpenSSH `ssh_config(5)`](https://man.openbsd.org/ssh_config#StrictHostKeyChecking)

### Inferences
- A local shell would be materially different from a remote SSH terminal: it would expose process execution on the user workstation, including its files, credentials, and network identity. It is out of scope for a small SFTP client and should be a separately approved product feature with a distinct permission model.
- The first release should present the feature as “Open remote terminal” only after a profile is connected and its host key has been verified. The session header should identify `user@host:port`, connection state, and a clear Disconnect/Close control.
- Use a maintained terminal emulator in the frontend rather than interpreting shell output as HTML. Send keystrokes as opaque UTF-8 bytes through narrow Rust commands and render output as terminal data, never as DOM markup. Do not log input or output by default because either can contain secrets.
- Professional baseline UX: open/close terminal, terminal focus, selectable/copyable output, paste with an optional multi-line warning, connection/exit messages, resize propagation, Ctrl+C sent to the remote PTY, and an obvious explanation if the server rejects PTY or shell access. Do not imply command success merely because input was accepted.
- The existing unknown-host-key flow is not sufficient for a terminal release: show the SHA-256 fingerprint and require explicit confirmation before writing a new key; always reject a changed key unless the user deliberately removes/replaces the saved key through a reviewed workflow.

### Gaps
- Server policy controls the availability of a PTY and shell. `sshd` may disable PTYs, force a command, or limit sessions, so no client can promise an interactive terminal for every SFTP profile. [OpenSSH `sshd_config(5)`](https://man.openbsd.org/sshd_config#PermitTTY)
- This research did not select a terminal-emulator dependency or license. That selection needs a frontend compatibility, licensing, accessibility, and maintenance review before implementation.

## Can the already connected SSH session be reused safely while SFTP remains active?

### Takeaway
Yes at the SSH-protocol level: use a second `session` channel on the authenticated SSH connection—one channel remains the SFTP subsystem and the other receives PTY + shell. However, Shelf currently retains only `SftpSession`; it drops the `russh::client::Handle` after opening the SFTP channel, so it cannot open a terminal channel later. Preserve a single authenticated handle in backend state and protect only handle operations with a short async mutex; do not hold that mutex while streaming terminal or SFTP data.

### Cited Findings
- RFC 4254 specifies that terminal sessions and forwarded connections are channels and that multiple channels are multiplexed into a single SSH connection; it also explicitly allows multiple active sessions. [RFC 4254, sections 5 and 6](https://datatracker.ietf.org/doc/html/rfc4254#section-5)
- The OpenSSH client documents connection sharing for multiple sessions over a single network connection, including shell, command execution, and SFTP. [OpenSSH `ssh_config(5)`](https://man.openbsd.org/ssh_config#ControlMaster)
- A server can cap open shell/login/subsystem sessions per network connection with `MaxSessions`; OpenSSH documents a default of 10 and notes that `1` effectively disables session multiplexing. [OpenSSH `sshd_config(5)`](https://man.openbsd.org/sshd_config#MaxSessions)
- `russh::client::Handle` is the object returned by `connect` and is documented as the handle used to create channels; the project currently locks and stores only `russh_sftp::client::SftpSession`. [russh client source](https://docs.rs/russh/0.63.3/src/russh/client/mod.rs.html#299-304); [Shelf `lib.rs`](../../src-tauri/src/lib.rs)
- A `russh::Channel` can be converted into an asynchronous bidirectional stream; channel traffic is flow-controlled by SSH. [russh Channel API](https://docs.rs/russh/0.63.3/russh/struct.Channel.html#method.into_stream); [RFC 4254, section 5.2](https://datatracker.ietf.org/doc/html/rfc4254#section-5.2)
- The current resolved Shelf dependency is russh 0.63.3. A client-side PTY panic reported against versions through 0.62.2 is fixed in 0.62.5, so the resolved version is beyond that affected range. [RustSec/GitHub advisory GHSA-cqjc-rmpq-xprq](https://github.com/Eugeny/russh/security/advisories/GHSA-cqjc-rmpq-xprq); [Shelf lockfile](../../src-tauri/Cargo.lock)

### Inferences
- The correct reuse model is one TCP/SSH connection, then two independent client-opened `session` channels: `SftpSession` on the first and `TerminalChannel` on the second. Never attempt `request_shell` on the channel already committed to the `sftp` subsystem; RFC 4254 permits only one of shell/command/subsystem on it.
- Refactor state around a connection owner that retains the authenticated `russh::client::Handle`, the SFTP session, connection identity/generation, and an optional terminal channel/task. Opening a terminal then acquires the handle briefly to create one channel, requests `xterm-256color` (or an explicitly supported TERM), requests a PTY with current columns/rows, starts the shell, and releases the handle lock.
- Do not multiplex frontend calls directly into a single long-held `Mutex`: the present SFTP methods hold a mutex for the entire directory or file operation. Retaining the same pattern for terminal read/write would make typing or streaming wait behind a transfer. Channel-specific tasks/queues should own terminal I/O and emit bounded output events; SFTP operations can retain their own synchronization as a later performance improvement.
- A second authenticated TCP connection is a reasonable fallback only when the server rejects the second session (for example, `MaxSessions=1`) and the product explicitly chooses that behaviour. It costs a second handshake/authentication and creates a separate server session; do not silently fall back because it changes server-side auditing and may surprise the user.
- Remote terminal channel lifecycle: wait for PTY/shell confirmation, read stdout and extended stderr, relay resize events, send EOF/close on terminal close, and mark terminal unavailable if the common SSH transport disconnects. EOF is half-close, while SSH channel close requires the close handshake. [RFC 4254, sections 5.3 and 6.6](https://datatracker.ietf.org/doc/html/rfc4254#section-5.3)

### Gaps
- The actual target servers and their `MaxSessions`, `PermitTTY`, forced-command, and idle-timeout policies are not known. Validate the preferred one-connection design against the intended Raspberry Pi/production targets before committing to a fallback policy.
- The russh documentation verifies supported API surfaces but does not itself guarantee performance under concurrent, large SFTP transfers and high-rate terminal output. Load testing is required.

## What controls, failure handling, and acceptance tests are required for a first release?

### Takeaway
Treat the terminal as a privileged remote-control surface. Keep its IPC small and stateful, keep permissions/CSP restrictive, place limits on buffers and event rate, disclose host identity, and prove that terminal failure is contained to its channel rather than corrupting or blocking the active SFTP workflow.

### Cited Findings
- SSH channel data is flow-controlled; stderr is represented by `SSH_MSG_CHANNEL_EXTENDED_DATA`, and an exit status may be sent before channel close. [RFC 4254, sections 5.2, 5.3, 6.6, and 6.10](https://datatracker.ietf.org/doc/html/rfc4254#section-6.6)
- `russh` exposes channel `wait`, `data_bytes`, `eof`, `close`, `signal`, and `window_change`, which are the primitives for input, output, shutdown, Ctrl+C support, and resize handling. [russh Channel API](https://docs.rs/russh/0.63.3/russh/struct.Channel.html)
- Tauri capabilities grant or deny frontend access to core APIs, and overlapping capabilities merge their permissions. [Tauri capabilities documentation](https://tauri.app/security/capabilities/)
- Tauri recommends a restrictive CSP, warns that remote scripts are an attack vector, and says CSP is only active when configured. [Tauri CSP documentation](https://v2.tauri.app/security/csp/)
- The SSH standard recommends that clients reject server-initiated session-channel opens to make a corrupt server harder to use to attack the client. [RFC 4254, section 6.1](https://datatracker.ietf.org/doc/html/rfc4254#section-6.1)

### Inferences
- IPC controls: expose only `open_terminal`, `write_terminal(bytes)`, `resize_terminal(cols, rows)`, and `close_terminal`, all bound to the active connection generation and terminal ID. Reject input before terminal-ready state, after close, from a stale generation, or above a small per-message size. Rust—not the frontend—must enforce these conditions.
- Security controls: no local command API; no shell auto-start on connect; no port/X11/agent forwarding; no environment-variable forwarding by default; no password/keystroke/transcript logging; do not expose arbitrary Rust command invocation; set an explicit restrictive CSP; and give the terminal window/webview only the minimum Tauri capability permissions it needs.
- Resource controls: cap terminal scrollback and per-channel buffered output; apply backpressure or safely drop only oldest already-rendered scrollback when the UI falls behind; coalesce resize events; use a bounded outbound input queue; and surface a readable “output limit reached”/“connection lost” state instead of exhausting memory.
- Failure modes to handle without crash: PTY request rejected, shell request rejected, SFTP remains usable after terminal channel rejection; server disconnect/rekey/network loss marks both features disconnected; terminal EOF/close does not close SFTP; SFTP transfer failure does not terminate terminal; remote output contains invalid UTF-8; rapid resize; accidental duplicate Open Terminal; stale events after disconnect/reconnect; and server session limit reached.
- Acceptance tests: (1) connect to a test SSH server and perform SFTP listing while opening a remote shell on the same transport; (2) run `printf`, receive output, and preserve terminal ordering; (3) resize and verify remote `stty size`; (4) send Ctrl+C to a long-running command; (5) close terminal and confirm SFTP listing/upload/download still work; (6) reject PTY/shell and confirm a clear error plus healthy SFTP; (7) configure `MaxSessions 1` and verify the defined fallback/no-fallback behaviour; (8) test changed host key rejection and first-use fingerprint confirmation; (9) paste multi-line and Unicode/invalid-byte edge cases; (10) sustained SFTP transfer plus high-output command without UI freeze or unbounded memory; (11) disconnect/reconnect and ensure old terminal IDs cannot accept input; (12) run the normal Tauri build plus dependency/security audit after adding the terminal dependencies.

### Gaps
- Exact buffer sizes, output-rate thresholds, reconnect semantics, and transcript retention policy are product decisions that require usability and memory-budget measurements; they should not be guessed in a first implementation.
- A terminal emulator can support a broad ANSI/VT feature set, but the specific emulation level and its accessibility behaviour must be tested with the selected dependency and target server shells.
