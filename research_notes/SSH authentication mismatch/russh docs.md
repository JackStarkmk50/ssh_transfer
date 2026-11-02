# Russh authentication diagnosis

## What Russh can use on Windows and Linux

### Takeaway
Russh has direct APIs for a Windows named-pipe SSH agent, Pageant, and Unix `SSH_AUTH_SOCK`, but Shelf currently tries only one Windows agent endpoint and does not surface which step fails. The direct `ssh root@IP` result therefore does not prove that Shelf can reach the same credential provider.

### Cited Findings
- Russh exposes `connect_named_pipe` for a caller-supplied Windows SSH-agent named pipe and retries only the busy-pipe case; other pipe errors are returned to the caller. [Russh agent client source](https://docs.rs/russh/latest/src/russh/keys/agent/client.rs.html)
- Russh separately exposes `connect_pageant` on Windows, so a Pageant-backed key is a distinct compatibility path from a Windows OpenSSH named pipe. [Russh agent client source](https://docs.rs/russh/latest/src/russh/keys/agent/client.rs.html)
- On Unix, `connect_env` obtains the agent socket exclusively from `SSH_AUTH_SOCK`, returning an error when the variable or socket is unavailable. [Russh agent client source](https://docs.rs/russh/latest/src/russh/keys/agent/client.rs.html)
- The project README documents public-key authentication, OpenSSH agent support, Pageant support, RSA SHA-2, Ed25519, ECDSA, certificates, and later PuTTY PPK support as distinct capabilities. [Russh README](https://github.com/Eugeny/russh)

### Inferences
- A no-prompt OpenSSH login can be supplied by Windows OpenSSH Agent, Pageant, a third-party agent, a hardware-backed signer, or a configured private key. Shelf's fixed named-pipe attempt cannot establish that all of those sources are available.
- Shelf should expose the selected authentication source and its specific failure rather than collapse agent, key-file, and server rejection into one message.

### Gaps
- The user has not supplied `ssh -vvv root@IP` output or the resolved `ssh -G root@IP` values, so the actual provider of the successful command-line signature cannot be identified from the screenshot.

## Why the current Shelf flow can fail after OpenSSH succeeds

### Takeaway
The current Shelf code authenticates through a Rust SSH client that is independent from `ssh.exe`; its key/agent handling can differ from OpenSSH. The reported generic failure means every Shelf strategy failed, not that the server lacks the provider's public key.

### Cited Findings
- Russh is a low-level SSH client library that gives applications direct control of authentication and transport rather than invoking the system OpenSSH client. [Russh README](https://github.com/Eugeny/russh)
- Agent identities are enumerated by `request_identities`, and agent signatures are separately requested for each identity; a client must select and attempt identities itself. [Russh agent client source](https://docs.rs/russh/latest/src/russh/keys/agent/client.rs.html)
- The project code authenticates selected files with `load_secret_key`, and the project currently uses Russh 0.63 with only the `ring` backend. `load_secret_key` is an application-side parse operation, not a call to `ssh.exe`.
- Russh documents RSA SHA-2, Ed25519, ECDSA, SSH-RSA, and OpenSSH certificates as public-key algorithms; signature algorithm and key format matter independently of whether the server has the public key. [Russh README](https://github.com/Eugeny/russh)

### Inferences
- The most likely categories are: the working CLI uses an agent type Shelf does not reach; the configured identity is encrypted, hardware-backed, or a format unsupported by the pinned library; or the application offers different identities/order from OpenSSH and the server rejects them.
- Browsing to a key is a fallback only when it is the matching private key in a format Russh can parse. Selecting a `.pub` file cannot authenticate, and adding a new "Rust key" to the server would not repair access because it would not match the already-working local credential.
- A decisive read-only diagnostic is to capture OpenSSH verbose authentication output and resolved configuration, then compare the exact offered key type/fingerprint and agent path with Shelf's attempts.

### Gaps
- Russh's current online README describes features in the latest release; it does not prove every format listed is present in the project's pinned 0.63.3 build. Version-specific compatibility must be verified against the actual selected key type.
