# B1-E2E-07 — one installed Akula → signed VPS Node → second installed Akula

**Status: implementation branch, NOT ACCEPTED.** Canonical [Windows/VPS acceptance matrix](https://app.notion.com/p/40032a84beec81d0a25dd178456910bb) and [Node V1 service boundary](NODE-V1-SERVICE-GATE.md).

The Node-V1 Core/service integration was merged to main as `cfb2b328` (PR #697). That merge included real QUIC A→Node→B, B1d0 root-signed file/receipt on disposable Linux, systemd restart tests and Windows SSH *read-only* verification. It did **not** install Node on a real VPS or enable the installed Windows **Install Node** button.

## Required single product acceptance

A clean installed Windows Akula obtains independently pinned signed SSH KEX and Linux facts from the user's own supported Debian/Ubuntu x86-64 VPS. It verifies exact release-signed Node bytes with an independent release trust anchor; shows the fixed list of privileged changes, pinned host, release digest, service identity, UDP/43871 implications; receives **separate one-use owner approval**; verifies release/owner signatures again inside a fixed privileged Linux authority; commits the exact ELF and root-owned CAS to /opt via the existing B1d0 code; provisions immutable policy v3 and non-root service identity; starts systemd and proves actual health after reboot. An authorized second installed Windows Akula retrieves the approved object with source A offline, via File Gate create-new without overwriting existing Vault data. Wrong host, signature, owner, revoked permission, stale nonce, changed plan, interrupted commit, reboot, corrupted bytes and repeated requests must HOLD or DENY rather than install twice.

No human has authorized a real VPS target in this task. Do not infer VPS addresses, private credentials, signing keys or deployment consent from Notion or GitHub notes.

## Integrated work package / source-to-user mapping

| Gate | Evidence required |
| --- | --- |
| B1-E2E-00 | One signed Linux release and independently pinned public signer, one stable Windows client/release manifest, immutable package digest |
| B1-E2E-01 | Actual OS Windows signed SSH KEX (already PASS); reverify immediately before any remote operation |
| B1-E2E-02 | Immutable plan, verified package bytes, distinct explicit user click, owner-signed limited approval and immutable digest/nonce |
| B1-E2E-03 | Existing B1d0 verified-FD + one-use detached owner signature + root-owned durable witness CAS + fixed /opt |
| B1-E2E-04 | Provision non-root static service UID, one-shot identity enrollment, root policy v3 then systemd health |
| B1-E2E-05 | Reboot and crash/reconnect into real health or explicit Needs Review without silent regenerated keys |
| B1-E2E-06 | Negative-path and failpoint tests at each root authorization and fsync boundary |
| B1-E2E-07 | Installed Windows A → real owner-authorized VPS → installed Windows B, source offline, no overwrite |

### Concrete implementation recorded on the working branch

The opt-in `aquilum-core/server-install-review` feature introduces `server_release_lab::review`:

- Reuses the **existing** B1a signature verifier and hashes *the actual bounded package stream*. No new signing format or signing key.
- Validates the existing B0 digest-bound plan and that its public version, package digest and signer ID match the **independently verified signed manifest**.
- Binds a 10-minute, randomly generated single-use UI review nonce to exactly one InstallId, immutable plan digest, host SHA-256 and package SHA-256.
- Refuses changed host, user, fingerprint, package, echoed plan, replay, clock rollback, expired review, forged release signature, modified/oversized package or unsupported B0 facts. Rejections consume the local review attempt.
- Returns **InertUserConsent only**. This is not B1b1b `SignedOwnerApproval`, not a CAS commit, not signed package provisioning, not SSH install authority and not systemd activation. An app calling `expected_echo` without a genuine user click cannot obtain owner privileges.

The ordinary installed Windows Akula is intentionally **unchanged** by this module; `server-install-review` is an opt-in Core feature. Do **not** enable the existing Install Node button by adding this feature alone. An untrusted SSH server can never supply the pinned B1a signer or owner approval.

### Remaining implementation (not accepted)

- Production release trust anchor/floor and published signed Node artifact; no test signing key may serve as production identity.
- Actual Windows immutable plan/consent UI tied to verified SSH and signed package.
- Fixed pinned SSH install transport and root-only helper backed by B1d0, separately trusted owner signature and durable CAS.
- Policy v3/identity enrollment/systemd service activation, reboot and recovery transaction.
- Exportable **public-only** Node pairing config and two installed Windows/File Gate recipient acceptance.

Do not create per-module public release previews or treat laboratory markers as user acceptance. Continue on the same integrated feature branch until the full slice is wired, then run the exact-head CI and actual owner-authorized VPS acceptance. Nothing in this document grants access to a server.

### B1-E2E-02 transport + owner-signing compatibility (working branch)

- Windows Core now exposes `WindowsOpenSshPinnedProbe::prepare_signed_node_review` only behind `server-install-review`: it uses **actual System32 OpenSSH signed KEX**, the B0 fixed read-only OS facts, and B1a detached Ed25519 package signature/size/digest. Fingerprint-only input first probes candidate public key, but still demands the independently supplied SHA-256 pin and a real signed OpenSSH handshake. This is **not connected to a production Install button**.
- `server_owner_approval_wire::OwnerApprovalWireV1` is the single portable B1b1b canonical signing encoder used by Linux's existing `SignedApprovalClaim::canonical_bytes()` and available for future Windows owner signing. Same original signature domain and field order; legacy parity test guards against an incompatible second protocol.
- Local `InertUserConsent::matches_owner_claim` compares install ID, plan, SSH host, package, manifest and nonce with a future owner-signed B1b1b claim. **It does not authenticate the prior floor or witness**. Those remain independent root-owned Linux checks before CAS and /opt.
- Branch gate runs the Linux B0/B1a review and owner-signature parity tests, a default Core build, and a Windows pinned SSH + review compile. Formatting is scoped to newly authored Rust code rather than the entire vendor workspace.

**Remaining release HOLD**: No owner signing key provisioned on Windows, no production release key/floor, no fixed SSH mutation/privileged root helper, no systemd activation transaction, no installed Windows UI confirmation, no real VPS reboot or Windows B File Gate end-to-end. Neither an owner-signature test fixture nor a signed review is permission to deploy to an actual server.

### Separate Windows owner-key custody (opt-in, unreleased)

The independent `server-owner-windows-signer` feature contains a distinct owner Ed25519 identity, never the Iroh/P2P device key or release-signing key. It is explicitly enrolled once into the Windows Credential Manager under a fixed dedicated target; normal loading refuses missing/malformed records and never silently rotates keys. The private seed is zeroized in memory and never appears in app settings, logs, exported configuration or a remote installer request. Existing same-user malware and Windows account compromise are outside this software store's resistance: this is **not a non-exportable TPM-backed key**.

Signing requires the in-memory one-use `InertUserConsent` and the exact B1b1b owner claim (InstallId, host fingerprint, plan, package, manifest, review nonce, expiry). It still **does not install**; production root must independently trust/pin the owner public key, accept the real CAS floor, verify signed release, execute a fixed root-only helper and attest the installed service. The normal installed Akula UI does not initialize this owner key or enable `InstallServer`. Production root trust enrollment and a genuine per-user UI click remain missing.

### B1-E2E-07: reject misleading B0 v1 preview; privilege bootstrapping HOLD

A correctness/security gap was found in the historical B0 v1 template: it described `EnableLocalLoopbackOnly`, while the accepted Node-V1 daemon actually listens on IPv4 UDP **43871**. The UI **must never sign that old plan** as an authorization to provision the public Node service.

The work branch adds `preview_node_v2` under a new BLAKE3 domain and a fixed 12-action scope that explicitly discloses root trust bootstrap, independent owner approval/CAS, signed `/opt` ELF, dedicated non-root account, one-time durable key enrollment, root policy v3, restricted systemd, public UDP 43871 **without automatically modifying firewall**, service health and reboot/signed-receipt validation. Original B0 v1 template/digest are retained only for historical read-only uses. The B1 signed-package review refuses v1 even if internally rehashed correctly; it refuses missing or modified v2 actions.

The existing B0 Windows SSH probe's `sudo -n -l` establishes at most that *some sudo permissions are listed*. It does **not** prove that this SSH user can install the fixed helper, or that a root-owned helper is already enrolled. Root readiness is a **separate unimplemented gate**:
- If a separately root-provisioned, pinned helper exists, validate it independently and admit only its fixed install transaction.
- For a truly clean VPS, a tightly scoped, independently signed *initial* bootstrap must first obtain explicitly authorized noninteractive privilege. No password prompt, arbitrary SSH shell text, unsigned binary execution, accepting a VPS-provided signer key, or broad sudo capability may be silently inferred from a B0 success.
- If neither prerequisite is demonstrated, result **Needs admin authorization / HOLD**. Do not enable the Install button just because `has_sudo_rights=true`.

This versioned disclosure is a necessary prerequisite, not proof of deployment authorization. The owner signature, protected CAS witness, actual /opt and non-root systemd state remain authoritative only after a separate root transaction on an explicitly authorized host.

### Pre-enrolled root-helper presence proof (working branch; still HOLD)

The opt-in Core module `server_install_root_gate` now makes the *absence* of
privileged installer authority explicit instead of interpreting `sudo -n -l`
as an installation grant. It accepts only a short-lived Ed25519 attestation
under the domain `Akula B1-E2E-07 preenrolled root helper evidence v1`,
verified against an **independently pinned** root-attestor key and bound to
the specific InstallId, B0 v2 plan digest, independently authenticated SSH
host fingerprint, signed package digest, UI review nonce, root-helper ELF
digest, owner public key and root policy digest. The root attestor must be
enrolled through an independent OS-admin-authorized root procedure: none of
these trust pins may be learned from an untrusted VPS response.

The only positive output is `AuthenticatedPresenceOnly`: evidence that
the pre-enrolled signer answered this review, **not** verified privilege to
install, actual OS ownership, a root CAS operation, or consent to start a
service. Missing proof, invalid pins, mismatched scope, modified signatures,
expired/future or overlong validity all yield `NeedsAdminAuthorization`.
No command, chosen executable path, remote source file, root execution or
socket is introduced. The normal installed Akula UI and its disabled
`Install Node` control are not changed.

**Still unimplemented:** initial helper enrollment on a truly clean VPS,
actual independent OS-root owner/mode and fixed helper-identity verification,
approved noninteractive admin escalation, production helper invocation,
release-authority pinning, B1d0 commit and systemd activation. The pre-enrolled
presence proof must not be used to bypass any of those gates. B1-E2E-07
remains HOLD until the whole signed Windows→VPS→Windows workflow is tested.

### B0 v2: честный read-only preflight для чистого VPS (follow-up)

В Node v2 убрана ошибочная зависимость **чтения** совместимости сервера от `sudo -n -l`: подходящий Debian/Ubuntu x86-64 VPS без указанного sudo теперь получает неизменяемый B0 v2 план с `has_sudo_rights=false` вместо полного отказа в исследовании хоста. Старая B0 v1 семантика сохранена. При этом `prepare_signed_review` по-прежнему отказывает `InvalidPreview` без sudo, **никакого согласия на install** и root-выполнения не возникает. Для B1d0 нужна независимая, отдельно авторизованная root-процедура, а не вывод о разрешении из прочитанной host-факты.

Ограничение: изменение ещё не вводит привилегированную установку, авторизованный initial bootstrap или проверку на личном VPS. Полный B1-E2E-07 остаётся HOLD.

### Windows UI: отличать отсутствие прав администратора от несовместимой ОС

Нативная read-only SSH-проверка теперь показывает отдельный `NeedsAdminAuthorization` для совместимой Ubuntu/Debian x86-64 без sudo-listing. Такой сервер не следует ошибочно помечать как неподдерживаемый по ОС. `sudo -n -l` явно остаётся только списком разрешений и никогда не трактуется как полномочие установить root-helper. Добавлены RU/EN строки, отрицательные и UI-тесты, профильная Windows-проверка на той же ветке; кнопка `Install Node` по-прежнему отключена. Это UX-диагностика, **не первичная sudo-установка**; P0-HOLD остаётся.

### First-root signed helper staging: fixed interpreter, no candidate execution

The integrated branch now includes `scripts/server-bootstrap/root_stage_v1.py`, a self-contained, fixed Python source intended to be compiled into a future signed Windows installer. **It must never be fetched from a user-writable VPS file or executed from a staged candidate.** After a separate, explicit owner/admin-approved noninteractive sudo operation, an OS-owned `/usr/bin/python3 -I` may receive this fixed source over independently pinned SSH and accept a bounded binary frame on stdin. No actual Windows install transport invokes it yet.

The sender-side opt-in Core module `server_first_root_stage` reuses the existing B1a verifier and verifies the complete helper ELF before creating the frame. Only signer role `akula-root-bootstrap-v1` is accepted. The fixed Python source is compiled into Core and receives only the independently pinned public Ed25519 key as a substituted 64-character lowercase hex literal; neither remote server responses nor a user-supplied path choose this key.

On the server the fixed root operation rechecks the B1a canonical manifest with **OS-owned OpenSSL Ed25519**, exact signed package length and SHA-256, supported Linux x86-64 ELF identity, expiry and strict input framing. It reads the candidate from SSH stdin, not from a user directory; refuses trailing bytes, corrupt/foreign signatures, malformed releases and unexpected path/symlink ownership. Verified bytes are persisted only as a non-executable root-owned 0400 `helper.staged` under the fixed 0700 `/var/lib/akula/server-bootstrap/bootstrap-v1`. Every new file is create-exclusive and fsynced; existing or interrupted state refuses rather than overwriting. The HOLD marker explicitly prevents interpreting signed staging as permission to execute.

**Production remains locked:** The script deliberately contains **no release trust key** until a legitimate independently reviewed signing ceremony and a signed helper artifact exist. There is no production Windows sudo invocation, real owner approval, registered root helper, trusted-key distribution, actual B1d0 /opt CAS commit, systemd activation or VPS acceptance. The current code adds bounded, root-side signed-data custody and tests, not permission to deploy. B1-E2E-07 remains HOLD.

The branch gate runs the root-stage fixture with ordinary and disposable CI root identity, plus a Core B1a frame producer test in the same existing workflow; no separate PR or general CI fanout is created.
