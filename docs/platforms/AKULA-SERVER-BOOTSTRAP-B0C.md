# AKULA B0c — Windows signed OpenSSH KEX and read-only server preflight

## Opt-in behavior

Windows-only Rust module under existing non-shipping server-bootstrap-b0-openssh-lab. Reuses B0Input, ServerTarget, PreviewTransport and immutable DryRunPlan. No installer opcode.

- User provides independently obtained ssh-ed25519 host public key and the SHA-256 of its canonical 51-byte SSH wire. Both checked offline. No ssh-keyscan, TOFU, DNS auto-accept, key update or observed-key fallback.
- Uses Windows System32 OpenSSH ssh.exe from GetSystemDirectoryW, not PATH. Creates a unique temporary known_hosts with fixed HostKeyAlias, holds a Windows read-sharing-only file handle and non-reparse pinned directory handle, preventing replacement during signed KEX.
- Strict known_hosts KEX with no user/system SSH config, global known_hosts, proxies, password, keyboard auth, GSSAPI, SSH forwarding or user-selected remote command. Uses only existing Windows OpenSSH ssh-agent for client public-key auth. No password/private key is requested or serialized in Akula.
- Permits only a constant true command and the existing fixed Linux read-only B0 host facts command. A hard 12-second process deadline, 8-KiB output bound and strict fields parser. This is a preview, NOT a permission to install.
- Windows 2025 CI runs real ssh.exe against test-only ephemeral Paramiko SSH server on 127.0.0.1 with host key A and independent B. This CI-only server permits none-auth for synthetic ubuntu and the two fixed commands; Windows signed KEX to A must pass, forged pin to B must fail, and typed B0 plan must be immutable. Paramiko is solely a CI dependency.

## Limitations

The existing native Windows Verify SSH and Install Node buttons in B1d0 PR #518 REMAIN DISABLED. No native UX or owner consent, production agent identity selection, full Windows TEMP DACL custody proof, or physical VPS probe. Same-user and administrator compromise is not defended. No write, sudo execution or arbitrary command is available. Missing agent/private login fails closed. This is NOT a completed Windows installer, and no user data or external host is touched.

Next gate: integrate a consented preflight in Akula, then connect B1d0 direct /opt signed Node commit with durable CAS and separate authorized systemd activation. See https://app.notion.com/p/3fe32a84beec814e8342f69fdd518a34.


## B0c verified KEX positive-test repair — exact HEAD follow-up

Initial Windows profile #39607921043 built successfully, but real OpenSSH returned `No ED25519 host key is known for akula-b0c-...` even for the independently pinned A key. **2/3 tests passing was insufficient** because the B-key negative also failed for that generic reason.

The existing implementation now:
- Sends an unquoted, strictly ASCII safe absolute `UserKnownHostsFile` path as one OpenSSH `-o` argv element instead of embedding a second layer of literal quotes; unsupported paths (spaces, Unicode) **fail closed**, not trusted implicitly.
- Flushes initial `known_hosts`, closes the writer, reopens it READ-ONLY with Windows `FILE_SHARE_READ`, verifies exact contents, and retains this locked reader handle through signed KEX. This removes a Windows share-mode conflict caused by holding a write-capable pin while SSH also tries opening the known-hosts file. A same-user attacker is not considered defeated by this mechanism; TEMP ACL-hardening remains a release gate.
- Changes the isolated Paramiko **test-only** server from `none` auth to exact ephemeral `publickey` authentication. Windows CI now registers that one synthetic client key in a local Windows OpenSSH ssh-agent; the shipping Rust transport does not gain any auth bypass, password, private key import or arbitrary SSH command.

**Acceptance is still pending** until one exact-head Windows Actions run independently proves A=authenticated read-only facts and B=wrong host-key denial, not merely two generic refusals. Native UI Verify/Install remains disabled.


## B1-E2E-01 native UI: opted-in read-only Verify SSH

On the same baseline that includes accepted B1d0 Windows `ServerBootstrapDraft`,
the `aquilum-native` optional feature `server-readonly-preflight` makes the
existing **Verify SSH** button actionable only when the independent
`ssh-ed25519` host public key and SHA-256 fingerprint match offline.
This feature remains absent from the default and signed installers until
it passes native acceptance and explicit policy review. **Install Node**
is *always disabled*, in either build flavor.

The button executes a typed, nonprivileged `ServerTarget` read-only path:
`VerifyOfflineInput → signed Windows OpenSSH KEX → fixed OS facts →
observed/unsupported/refused UI status`. It creates **no synthetic
PackagePin**, signed installer plan, owner approval, remote write, privileged
helper request, activation or saved private key. It uses the previously
tested OS protected Windows OpenSSH client and pinned temporary known_hosts.
Only normalized resource facts enter the UI. An SSH success is explicitly
labelled **Node not installed**.

The native application uses a background worker and the existing wake/poll
mechanism, so the Masonry UI thread does not block on OpenSSH. Results are
session-only and bound to an exact public draft: if address, user, port, key
or fingerprint change while the worker is running, its reply is discarded.
A second Verify click is disabled during an active probe. A failed or
unsupported server remains visibly blocked; no SSH stderr is shown.

The Windows-2025 workflow now requires both the original actual loopback
signed A=PASS/B=DENY tests and a targeted `aquilum-native` test/compile with
`server-readonly-preflight`. The UI test rejects private-key input, checks
verification state/disablement and ensures no `Install Node` button becomes
enabled. This is the **first connected read-only UI slice**, not B1-E2E-01
release PASS until the installed default Windows Akula and a user-authorized
real VPS are tested. B1-E2E-02…07 remain unaccepted.

Reference: [canonical B1 end-to-end matrix](https://app.notion.com/p/40032a84beec81d0a25dd178456910bb).
