# OpenSSH authentication mismatch

## Why command line SSH can work while Shelf fails

### Takeaway
OpenSSH can combine evaluated client configuration, agent identities, PKCS11 providers, and security-key providers. A third-party SSH client that only reads files or only contacts one agent endpoint does not necessarily reproduce `ssh root@IP` authentication.

### Cited Findings
- On Windows, `ssh.exe` evaluates client configuration from a command-line `-F` file, then `%userprofile%\.ssh\config`, then `%programdata%\ssh\ssh_config`. [Source](https://learn.microsoft.com/th-th/windows-server/administration/OpenSSH/openssh-server-configuration)
- `ssh -G` prints the configuration after evaluating `Host` and `Match` blocks, so it is the correct read-only command to inspect the effective settings for the exact connection target. [Source](https://man.openbsd.org/ssh)
- OpenSSH can use keys from an `IdentityFile`, from `ssh-agent`, from a PKCS11 provider, or from a security-key provider; the configuration may also refer to a public key whose private half is only held by the agent. [Source](https://man.openbsd.org/OpenBSD-current/man/ssh_config)
- The standard OpenSSH identity list includes more than Shelf's original three filenames, including `id_ecdsa_sk` and `id_ed25519_sk`; those are authenticator-hosted security-key identities rather than ordinary file-only keys. [Source](https://man.openbsd.org/OpenBSD-current/man/ssh_config)

### Inferences
- The successful `ssh root@IP` command does not prove that an ordinary, unencrypted private-key file is available for Shelf to load. It may be using an agent-held, hardware-backed, PKCS11, or configuration-selected identity.
- If the key label shown by the provider is a public key associated with a security-key or external provider identity, browsing to an unrelated private-key file will not work.

### Gaps
- The supplied screenshots do not show `ssh -G root@IP` output, `ssh -vvv root@IP` output, or the actual identity provider, so the exact source used by the Windows OpenSSH client cannot be identified from them.

## Agent selection and rejected identities

### Takeaway
OpenSSH has controls to select a specific agent and limit which identities are offered. Shelf's current implementation does not yet honor those resolved options, so it can send different identities than the command-line client.

### Cited Findings
- `IdentityAgent` selects an authentication-agent socket and can override `SSH_AUTH_SOCK`; it can also disable agent use. [Source](https://man.openbsd.org/OpenBSD-current/man/ssh_config)
- `IdentitiesOnly yes` limits OpenSSH to configured/default identity and certificate files even when an agent or hardware provider has more identities; the manual specifically says this is useful when an agent offers many identities. [Source](https://man.openbsd.org/OpenBSD-current/man/ssh_config)
- `IdentityFile` can be paired with `IdentitiesOnly` to select an agent identity, and OpenSSH tries multiple configured identity files in sequence. [Source](https://man.openbsd.org/OpenBSD-current/man/ssh_config)
- Microsoft documents that the Windows OpenSSH Authentication Agent stores private keys for public-key authentication, that the service is disabled by default in its example, and that `ssh-add` loads a key into it. [Source](https://learn.microsoft.com/th-th/windows-server/administration/openssh/openssh_keymanagement)

### Inferences
- The screenshot's generic Shelf rejection is consistent with Shelf not reproducing the exact OpenSSH identity selection, especially if OpenSSH uses `IdentityAgent`, `IdentitiesOnly`, a security key, or a PKCS11 provider.
- The next diagnostic should obtain the effective OpenSSH configuration and verbose authentication offers before changing server keys or the firewall.

### Gaps
- Official documentation confirms these mechanisms but cannot identify which one applies without the effective configuration and verbose local-client trace for this user.
