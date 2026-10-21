# AKULA-SERVER-BOOTSTRAP B0b — real pinned OpenSSH key exchange (LAB)

**Status: experimental Linux-only B0b staging.** Not an installed Akula feature, not a public SSH wizard, and not a server installer. Builds on [B0a PR #430](https://github.com/Lalalalendia/Akula/pull/430) and [Notion AKULA-SERVER-BOOTSTRAP-02](https://app.notion.com/p/3fe32a84beec814e8342f69fdd518a34).

## Narrow security design

- The operator supplies an **independently obtained** ed25519 SSH HOST PUBLIC KEY (OpenSSH two-field format) and the pinned SHA-256 digest of the underlying SSH wire bytes. Never fetch the key from the host being connected to and silently trust it. There is **no ssh-keyscan TOFU**, DNS SSHFP auto-acceptance, or key auto-rotation.
- B0b checks that the given public key decodes to a canonical 51-byte SSH ed25519 wire key and matches the independently pinned SHA-256 digest **before network access**.
- It creates a fresh Linux /tmp directory (mode 0700) with its own known_hosts (0600), one exact ed25519 key under an InstallId-derived, fixed, safe HostKeyAlias. It never edits the user's permanent known_hosts.
- An absolute **/usr/bin/ssh** OpenSSH subprocess, with `-F /dev/null`, `StrictHostKeyChecking=yes`, `HostKeyAlgorithms=ssh-ed25519`, the exact private known_hosts and `GlobalKnownHostsFile=/dev/null`, verifies the server's host-key signature in the *real SSH key exchange* **before public-key authentication**. A different signed key fails closed, even if DNS or the network path changes. UpdateHostKeys, VerifyHostKeyDNS, proxy/jump, GSSAPI/password/keyboard-interactive auth, default keys, agent, agent forwarding and connection multiplexing are disabled.
- Private **client identity key** is supplied as a local absolute path with 0600-style permissions, rather than read or logged by Rust. No password/SSH private bytes in preview data, logs or plan digest. The path cannot be a symlink. Client public-key authentication remains after OpenSSH's KEX trust check.
- The real B0b transport first executes a hardcoded remote **true** no-op under an authenticated, host-key-verified SSH session. **Only after success** can its fixed second SSH session read host diagnostics; the second session again verifies the same pinned host key. The B0a PreviewTransport return value is a *previously trusted reference key*, returned only after successful KEX; it is **not** a key extracted from the peer by Rust.
- Fixed diagnostic commands observe /etc/os-release, uname, /run/systemd/system, a non-interactive `sudo -n -l` **rights list**, getconf CPUs, /proc/meminfo and `df -Pk /`. No user-supplied shell string is ever interpolated. `sudo -n -l` does not acquire an elevated shell or run a privileged installer; it is only a permission query, after SSH verification.
- Process lifetime bounded by a 12-second local wall-clock deadline, 8-KiB stdout ceiling, no stdin/TTY, no command stderr reflected, no forwarding. Host diagnostic lines are strictly parsed with unique, expected labels and numeric bounds. Unsupported Linux/arch/systemd/privileges/resources fail closed through the inherited B0a plan.
- Separate experimental Cargo feature `server-bootstrap-b0-openssh-lab`; core built normally or on Windows is unchanged. No release binary, GUI flow, Node daemon changes, File Gate mutation or Sync changes.

## Validation

`scripts/ci/akula-b0b-local-sshd.sh` starts a **disposable SSH server only on 127.0.0.1** inside a GitHub-hosted Ubuntu runner. It generates two **ephemeral** ed25519 server identities, one authorized client identity and an unauthorized client identity. The 5 tests in `core/tests/server_bootstrap_b0b.rs` verify:

1. Correct pinned signed KEX and real read-only Linux fact parsing; B0a produces a plan only if OS, systemd, sudo, CPU, memory and disk meet the test profile.
2. **Wrong server host key**: the untrusted listener A is never accepted under independently pinned host key B; OpenSSH refuses before public-key authentication.
3. Wrong advertised/pinned SHA-256, incorrect key type, malformed public key rejected *locally* before SSH.
4. Different client identity cannot authenticate after a valid host-key KEX.
5. Symlinked/private-key-with-world-readable-permissions rejected before network.

The workflow additionally re-runs B0a 8/8 mock tests and the default Core feature build. It does **not** run against any user's VPS, account, permanent ~/.ssh configuration, or installed Akula. A green CI must explicitly demonstrate real KEX, not only mocks.

## Explicit nonclaims and next steps

- B0b relies on the locally installed OpenSSH executable, its signed KEX implementation and system configuration. It has **not** proven resistance to a compromised local executable, same-UID attacker, compromised provider control plane or stolen server/client private key. CI uses a permissive disposable sshd to simulate a trusted, unprivileged user; it does not prove production deployment policy.
- Only ed25519 host keys are admitted in this first lab. ARM64, Android, Windows OpenSSH paths and other host-key algorithms are separate compatibility gates.
- A successful login/diagnostic does **not** install Node, provide device pairing, access production file vaults, or authorize privilege changes. The preview digest is neither an approval nor a package signature.
- The SSH diagnostics read actual host facts; a host under an attacker's administrative control can **lie** about these facts even if its key is trusted. The user must separately approve B1's exact proposed change list and trust its release signing key.
- There is no persistent InstallId journal, progress resume, signed package verification, systemd unit installer, rollback, SSH passphrase handling, server self-updating, remote writer, TLS/WAN, Docker, RouterOS, Android client or relay networking.
- **Next gate:** once B0a #430 exact-head general CI and B0b actual local SSH tests pass, review the observed key-exchange trust sequence and run read-only B0b against a disposable Debian/Ubuntu Linux VM (separate machine instance). Only then design B1 signed native package/systemd installation as a separately authorized transaction.

Referenced upstream OpenSSH options: https://man.openbsd.org/ssh_config and https://manpages.debian.org/trixie/openssh-client/ssh_config.5.en.html.
