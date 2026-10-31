# Shelf terminal frontend and UI

## Is xterm.js suitable for Tauri on Windows and Linux?

### Takeaway
xterm.js is the recommended frontend emulator for Shelf: it runs in Tauri's webview, is not tied to an operating system, and has maintained fit, search, WebGL, Unicode, link, and accessibility facilities. It is an emulator only, so Windows/Linux support and SSH behavior belong to the Rust backend rather than xterm.js.

### Cited Findings
- xterm.js exposes a browser terminal API and a maintained add-on model; `@xterm/addon-fit` sizes terminal rows and columns to its host element. [xterm.js add-on guide](https://xtermjs.org/docs/guides/using-addons/)
- The xterm.js API provides `Terminal.resize`; its `windowsPty` option documents ConPTY-specific resize/reflow behavior, including `backend: 'conpty'` and a Windows build number. [xterm.js terminal options](https://xtermjs.org/docs/api/terminal/interfaces/iterminaloptions/)
- `screenReaderMode` exposes supporting DOM elements for NVDA on Windows and VoiceOver on macOS; `minimumContrastRatio` supports 4.5 (WCAG AA) and 7 (WCAG AAA). [xterm.js terminal options](https://xtermjs.org/docs/api/terminal/interfaces/iterminaloptions/)
- xterm.js has an optional WebGL renderer; its current release notes say the legacy canvas renderer was removed and recommend the DOM renderer or WebGL. [xterm.js releases](https://github.com/xtermjs/xterm.js/releases)
- hterm is also a browser terminal renderer and explicitly says it is neither an SSH client nor a shell. Its documented embedding path is a repository build that produces `hterm_all.js`, which is a less natural fit for Shelf's Vite/React dependency model. [hterm README](https://chromium.googlesource.com/apps/libapps/%2B/refs/heads/chromeos-R86/hterm/); [hterm embedding guide](https://chromium.googlesource.com/apps/libapps/%2B/HEAD/hterm/docs/embed.md)
- `node-pty` is cross-platform and uses Windows ConPTY, but it is a Node native module. Shelf has a Rust/Tauri backend, so it would introduce a separate Node process/runtime path instead of using the existing Rust SSH stack. [node-pty README](https://github.com/Microsoft/node-pty/blob/main/README.md)

### Inferences
- Use `@xterm/xterm` plus `@xterm/addon-fit`; add `@xterm/addon-search` only if terminal search is a requirement. Start with DOM rendering for the broadest accessibility path, then make WebGL an opt-in/fallback-tested performance enhancement. This inference follows xterm.js's renderer and accessibility options. [xterm.js terminal options](https://xtermjs.org/docs/api/terminal/interfaces/iterminaloptions/)
- Do not add a local PTY or `node-pty` for remote SSH. Shelf already uses Rust and needs a remote SSH session channel, not a locally spawned shell. Rust-only keeps authentication, host verification, and connection lifecycle in one trust boundary. [Russh client module](https://docs.rs/russh/latest/russh/client/)
- Configure `windowsPty` only when Shelf knows the remote endpoint is a Windows ConPTY host; the user's desktop OS is not enough to establish that. This follows the option's stated purpose: compatibility with the PTY that hosts the terminal. [xterm.js terminal options](https://xtermjs.org/docs/api/terminal/interfaces/iterminaloptions/)

### Gaps
- Official documentation does not publish a supported Tauri-specific xterm.js sample or a Tauri WebView compatibility matrix. Validate on the project's minimum supported Windows WebView2 and Linux WebKitGTK versions with actual SSH output, Unicode, copy/paste, and resize tests.

## What frontend and backend message protocol is needed for real PTY interaction?

### Takeaway
Create one independent SSH shell channel per visible Shelf terminal. Open a PTY with the measured `cols`/`rows`, request a shell, stream raw SSH output through a Tauri `Channel`, and send user input and debounced dimensions through narrowly scoped commands. Do not stream terminal bytes through Tauri events because Tauri documents channels as the ordered, fast streaming mechanism.

### Cited Findings
- Russh's client API says its connection handle can create channels that request a PTY or perform other SSH work, and specifically links an interactive PTY client example. [Russh client module](https://docs.rs/russh/latest/russh/client/)
- Russh exposes PTY parameters including terminal type, character columns, rows, pixel dimensions, and modes; it also exposes shell requests and window changes. [Russh `Session`](https://docs.rs/russh/latest/russh/client/struct.Session.html); [Russh `ChannelMsg`](https://docs.rs/russh/latest/russh/enum.ChannelMsg.html)
- Tauri says events are for small streamed amounts and are not designed for low-latency or high-throughput delivery; they use JSON payloads and asynchronous listeners can process rapid events out of order. [Tauri: calling the frontend](https://v2.tauri.app/develop/calling-frontend/)
- Tauri describes `Channel` as fast and ordered, uses it internally for child-process output and WebSocket messages, and recommends it for streaming data. [Tauri: calling the frontend](https://v2.tauri.app/develop/calling-frontend/); [Tauri: calling Rust](https://v2.tauri.app/develop/calling-rust/)
- xterm.js buffers `write` calls asynchronously. Its flow-control guide says an overwhelmed terminal may become unresponsive, can buffer up to 50 MB, and should have transport backpressure/acknowledgement under fast output. [xterm.js flow control guide](https://xtermjs.org/docs/guides/flowcontrol/)
- Tauri commands are typed, accept serializable inputs, can be asynchronous, and command return data is JSON unless an optimized raw response is used. [Tauri: calling Rust](https://v2.tauri.app/develop/calling-rust/)

### Inferences
- Recommended API contract, all scoped to an opaque `terminalId`:
  - `terminal_open({ terminalId, cols, rows, term: 'xterm-256color' }, output: Channel<TerminalOutput>)` opens a new Russh session channel, requests the PTY, requests the shell, and starts the SSH-read task. `TerminalOutput` needs `data` (UTF-8 text or explicitly encoded bytes), `stderr`, `exit`, and `error` variants.
  - `terminal_write({ terminalId, data })` sends exact `term.onData` content to the SSH channel. It must not normalize Enter, control keys, escape sequences, or paste content.
  - `terminal_resize({ terminalId, cols, rows, pixelWidth, pixelHeight })` calls Russh window-change after `FitAddon.fit()` and a `ResizeObserver`; debounce so layout animation does not flood the SSH channel.
  - `terminal_close({ terminalId })` sends EOF/close, cancels the reader task, removes state, and rejects later writes. Connection loss must produce a terminal `error` then `exit` state, not leave a live-looking input surface.
- Make stream records ordered per terminal and never share terminal output with the file-listing channel. Tauri provides ordering for the channel, while Shell state needs its own lifecycle and backpressure. [Tauri: calling the frontend](https://v2.tauri.app/develop/calling-frontend/); [xterm.js flow control guide](https://xtermjs.org/docs/guides/flowcontrol/)
- Use bounded backend queues and a frontend acknowledgement/watermark only if testing finds output can exceed the renderer's drain speed. xterm.js documents the need for such control but does not prescribe a Tauri transport implementation. [xterm.js flow control guide](https://xtermjs.org/docs/guides/flowcontrol/)
- Put only the four terminal commands in the main-window capability. Tauri capabilities are the framework's fine-grained access boundary; do not grant a broad shell plugin permission merely to implement an SSH terminal. [Tauri security](https://v2.tauri.app/security/); [Tauri shell plugin](https://v2.tauri.app/reference/javascript/shell/)

### Gaps
- The exact public Russh ergonomic calls should be confirmed against the pinned `0.63.x` API during implementation; its official API confirms the required primitives but the report does not compile a prototype.
- Tauri documentation recommends channels but does not specify terminal-byte encoding, queue sizes, or terminal session retention rules. Those are application protocol decisions and require load testing.

## What design keeps this useful without turning Shelf into a full SSH client?

### Takeaway
Make terminal a collapsible, single-session "Remote command" drawer attached to the selected SFTP connection—not a tabbed terminal workspace. It should help inspect remote paths and run short commands while preserving Shelf's two-pane transfer workflow as its primary surface.

### Cited Findings
- The current Tauri clipboard plugin supports Windows and Linux, but no clipboard feature is enabled by default because clipboard access is application-specific and potentially dangerous. [Tauri clipboard plugin](https://v2.tauri.app/plugin/clipboard/)
- xterm.js supports selection behavior, screen-reader mode, contrast control, scrollback, and a configurable `disableStdin` option. [xterm.js terminal options](https://xtermjs.org/docs/api/terminal/interfaces/iterminaloptions/)
- xterm.js advises validating link activation and warns that a terminal can output untrusted `javascript:` links; its default OSC 8 behavior prompts, while a custom handler must preserve an explicit safe scheme policy. [xterm.js terminal options](https://xtermjs.org/docs/api/terminal/interfaces/iterminaloptions/)

### Inferences
- Placement: add a bottom drawer below both file panes, collapsed by default. Its header should show connection name, a clear status (`Connected`, `Disconnected`, or `Shell exited`), "Clear", "Copy", and "Close". Expand from a single toolbar action and size it with a drag handle; store only drawer height, not terminal contents.
- Scope boundary: one shell per active SFTP connection, no terminal tabs, port forwarding, local shell, serial console, SSH config editor, or saved command library. Closing the connection closes its shell. This yields a useful escape hatch without competing with a dedicated SSH application.
- Context: the drawer can offer explicit user-triggered actions such as "Copy remote path" and "Open shell here". The latter may send a quoted `cd` command only after showing the exact command and respecting the remote shell's quoting constraints; do not silently run commands when a directory is selected.
- Keyboard: when the terminal has focus, reserve terminal-native `Ctrl+C`, `Ctrl+Z`, arrows, Tab, Page Up/Down, and common shell chords for the remote host. Keep application shortcuts active only when focus is outside; use an explicit application chord such as `Ctrl+Shift+T` to open/focus the drawer and `Escape` to return focus to the file pane. Never make `Ctrl+C` copy by default inside the terminal; provide a visible Copy button and a documented selection shortcut.
- Clipboard: copy selected terminal text only on an explicit UI action or keyboard shortcut. Paste must be user initiated, should show a confirmation for multiline paste, and needs only `clipboard-manager:allow-write-text` for copy plus `allow-read-text` when paste is enabled. [Tauri clipboard plugin](https://v2.tauri.app/plugin/clipboard/)
- Accessibility: use a labelled drawer, a real labelled status region, visible focus ring, predictable focus return on close, 4.5+ contrast, and a settings toggle for `screenReaderMode`. Keep DOM rendering available for assistive-technology testing; do not require WebGL. [xterm.js terminal options](https://xtermjs.org/docs/api/terminal/interfaces/iterminaloptions/)
- Security: show a one-time warning that this runs arbitrary commands on the remote host; do not render OSC 8 or detected links as silently clickable. Link activation must remain allowlisted to `https:`/`http:` and use the existing explicit external-open confirmation. [xterm.js terminal options](https://xtermjs.org/docs/api/terminal/interfaces/iterminaloptions/)

### Gaps
- No user research was available to validate whether Shelf users need a terminal at all, or whether the default drawer height and proposed shortcuts match their workflows. Test a prototype with transfer users before expanding scope.
