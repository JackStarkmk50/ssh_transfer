# Trace the working SSH identity

Shelf is failing because a successful `ssh root@IP` command does not prove that Shelf can access the same authentication mechanism. The server is already configured correctly: it accepted the provider public key and the command-line client authenticated. The evidence instead points to a client-side mismatch: Windows OpenSSH can select a configured identity, a dedicated agent, a hardware-backed security key, or a PKCS11 provider, while Shelf currently attempts ordinary key files and one Windows OpenSSH agent pipe. The screenshot is therefore not evidence that another server key, firewall rule, or “Rust key” is needed. The decisive next step is to identify precisely which identity and provider `ssh.exe` uses, without changing the server or Shelf.

## OpenSSH resolves more than one private-key file

**Windows OpenSSH evaluates client configuration before authenticating.** Its configuration can come from a command-line file, the user configuration at `%USERPROFILE%\.ssh\config`, and the system configuration at `%PROGRAMDATA%\ssh\ssh_config`. The read-only command `ssh -G root@IP` prints the effective configuration after `Host` and `Match` rules have been applied, so it is the authoritative view for the exact command that succeeds. ([Microsoft OpenSSH configuration](https://learn.microsoft.com/th-th/windows-server/administration/OpenSSH/openssh-server-configuration)) ([OpenSSH ssh manual](https://man.openbsd.org/ssh))

OpenSSH can authenticate through an `IdentityFile`, `ssh-agent`, a PKCS11 provider, or a security-key provider. The default identity set also includes security-key identities such as `id_ecdsa_sk` and `id_ed25519_sk`, whose private half is held by an authenticator rather than exposed as a normal file. A provider label such as `OGPC07Windows` identifies the server-side public key record; it does not reveal which local signing method OpenSSH selected. ([OpenSSH ssh_config](https://man.openbsd.org/OpenBSD-current/man/ssh_config))

## Shelf may be reaching a different agent

**Russh can use several agent transports, but Shelf currently reaches only one Windows named pipe.** Russh has separate support for the Windows OpenSSH named-pipe agent, Pageant, and on Unix the socket named by `SSH_AUTH_SOCK`. A successful command-line login may instead use Pageant, an `IdentityAgent` setting, a hardware-backed signer, or a provider chosen by OpenSSH configuration. Consequently, the current Shelf error means its file and agent strategies were not accepted; it does not mean the server has lost the public key. ([Russh agent client source](https://docs.rs/russh/latest/src/russh/keys/agent/client.rs.html)) ([Russh project README](https://github.com/Eugeny/russh))

OpenSSH also provides `IdentitiesOnly` specifically to limit which keys are offered when an agent contains several identities, and `IdentityAgent` can choose or disable an agent socket. Shelf does not yet reproduce those evaluated options. That difference can make Shelf offer the wrong identity or a different order even when `ssh.exe` connects silently. ([OpenSSH ssh_config](https://man.openbsd.org/OpenBSD-current/man/ssh_config))

## Two read-only commands identify the gap

Run the following in PowerShell, replacing the placeholder with the real server IP, and share only the authentication-related output—never private-key contents, passwords, or passphrases:

```powershell
ssh -G root@<ip> | Select-String 'identityfile|identityagent|identitiesonly|pkcs11provider|securitykeyprovider'
ssh -vvv root@<ip>
```

The first command identifies OpenSSH’s resolved configuration without connecting. The second prints which public key or agent identity is offered and which one the server accepts. The decisive line is normally similar to `Server accepts key:` or `Offering public key:`. If it identifies a private-key path, Shelf can use that exact private key through the existing picker or pasted path. If it identifies an agent, Pageant, PKCS11 provider, or a security key, Shelf needs support for that specific provider rather than a copied key file.

## The appropriate fix depends on the trace

If the verbose trace names a normal OpenSSH private-key file, the fix is to make Shelf select that file and handle its passphrase if encrypted. If it names `IdentityAgent`, Shelf should honor that configured agent endpoint. If it uses Pageant, Shelf needs the Russh Pageant connector. If it uses `PKCS11Provider`, `SecurityKeyProvider`, or an `*_sk` identity, Shelf needs a hardware/provider signing path; exporting or inventing a “Rust key” is neither necessary nor appropriate. Russh is a direct SSH library rather than a wrapper around `ssh.exe`, so compatibility with a particular OpenSSH provider must be implemented explicitly. ([Russh README](https://github.com/Eugeny/russh))

The most durable product approach is to add an authentication diagnostics screen that reports the selected mode, the resolved identity source, and a precise failure reason. It should not silently attempt every possible identity because SSH servers can limit authentication attempts. The immediate decision, however, should wait for the two read-only command outputs: they identify the exact supported integration to add and avoid changing the server’s working key setup.

## Conclusion

The server is not the failing side: port 22 is open and the existing OpenSSH client already authenticates. **Shelf and `ssh.exe` are currently using different—or differently selected—credential paths.** Capture the effective configuration and verbose identity offer first, then choose the smallest integration: exact private file, configured agent endpoint, Pageant, or hardware/provider support. No server key change is warranted before that evidence is available.
