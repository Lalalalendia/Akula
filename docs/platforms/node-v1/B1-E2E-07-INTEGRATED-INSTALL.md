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


The staging implementation also reopens and rehashes the *root-owned published*
`helper.staged` inode before writing its durable HOLD marker. Owner UID0,
a single link, mode 0400, unchanged inode and complete SHA-256 must all agree.
A disposable test executes the complete fixed root entrypoint as **actual UID0**
with a temporary CI-only signer and root directory override; production path
and real VPS are not used. No signed release key is distributed by this test.


### Opt-in Windows pinned SSH sender: signed data HOLD, not node install

The experimental Core feature `server-root-stage-ssh-lab` now introduces a real Windows OpenSSH stdin-based sender for the previously verified root-stage frame. It **reuses the same System32 ssh.exe, independent host-key pinned known_hosts and strict noninteractive options** as the read-only B0c probe. The one-shot `InertUserConsent` is consumed, and the target InstallId and pinned KEX host digest must match before any root-stage SSH process starts. Source is compiled into Core, bounded and POSIX-shell-quoted; no remote package path, shell command or private SSH key can be configured by the user. The outgoing binary frame is verified B1a bytes. I/O and remote reply are bounded; timeouts and unsuccessful exits refuse. A successful remote message is only `RemoteReportedSignedDataHold`, not an attested installation or a systemd health check.

**Critical sudo limitation:** allowing `sudo /usr/bin/python3 -I -c` in sudoers is equivalent to granting arbitrary Python execution as root. It is **not** a safe restricted sudoers permission. The first-root command is only appropriate on a VPS where the account already has independently authorized unrestricted administrative power; it cannot create a new privilege grant or be marketed as least-privilege escalation. Hosts without such authorization remain `NeedsAdminAuthorization / HOLD`. The native Installed Akula does not enable this feature or invoke the sender. A **separately reviewed bootstrap-helper SHA in the owner-visible plan, root-authenticated approval and real OS-signed receipt** are missing before any production rollout. No production release anchor is provisioned.

### B0 v3 owner review pins the actual privileged bootstrap helper

The previous B0 v2 Node plan described the privileged bootstrap action but
did not contain the exact **bootstrap-helper ELF SHA-256**. This is insufficient
for an owner click before privileged helper staging. Version 3 adds a distinct
`bootstrap_helper_sha256` field and
`VerifySignedBootstrapHelperBeforeRootExecution` to the fixed 13-action
plan under a separate BLAKE3 hash domain. Original v1/v2 JSON and digests are
unchanged; v1/v2 plans cannot carry a helper pin. The new Windows
`prepare_signed_node_review_with_bootstrap` consumes a previously B1a-verified
`VerifiedRootStageBundle` to create a v3 plan. The one-use local consent keeps
the exact helper SHA and passes it to the opt-in pinned-SSH transport. A v2
consent, altered helper hash, missing hash or mismatched host refuses
**before** any root SSH subprocess is launched. This is not yet a root-signed
CAS receipt or live owner-facing UI.

The existing root-stage Linux test failed in CI #40104002068 because its
nonroot variant tried to publish a root-only inode. The production UID0 check
was **not** weakened: `publish_hold` now refuses before creating any files
when not root; only the isolated root test exercises successful root-owned
publication and nonroot tests explicitly require failure without state.
Window's opt-in sender compile/tests from this prior run passed.

**Remaining P0-HOLD**: production helper publisher key/ELF, independent root
trust enrollment, actual Windows owner-signature and native review wizard,
the B1d0 fixed commit, systemd and reboot, and two-client real VPS E2E.
No normal Akula installer or `main` is changed. Allowlisting
`sudo python -c` in sudoers remains unsafe; a genuinely unprivileged VPS
cannot be bootstrapped by this code without independently granted admin
authority.

### Root-stage read-only reinspection across restarts

The fixed root-stage source now supports a separate `inspect_published_hold`
routine. Only an already privileged root process holding a trusted root-owned
0700 parent directory handle can re-open the exact `bootstrap-v1` directory.
Inspection is **read-only**: it checks owner UID0 and modes, rejects unexpected
or missing files, symlinks, changed or multiply-linked files, rechecks the
canonical B1a signer and validity window, compares the stored Ed25519 key with
the independently supplied pinned key, rehashes the full helper, verifies
Linux x86-64 ELF identity, re-verifies the detached Ed25519 signature using
OS OpenSSL and checks the exact `HOLD` marker. The published public PEM is
now persisted as a 0400 root-owned file for verification without modifications.
No staged ELF is executed; a consistent reinspection does NOT imply permission
to install Node, start systemd or trust an unsigned SSH success message.

Positive reinspection and modified-helper refusal are tested using isolated
UID0 test directories, while the nonroot test refuses before creating files.
The Windows SSH remote command also calls `/usr/bin/sudo` by an explicit
OS path rather than relying on the remote account's PATH to resolve `sudo`.
This does NOT turn arbitrary `sudo python -c` into restricted privilege:
an operator must already possess independently authorized root access.

### Production publisher trust anchor: no caller-supplied root authority

Security audit found that the first root-stage bundle factory previously
accepted a `TrustedReleaseSigner` argument without comparing its Ed25519
public key to a separately provisioned publisher anchor. A self-signed
candidate could therefore supply both its key and its signature. The
production `prepare_root_stage_bundle` now has an explicit, immutable
`PRODUCTION_BOOTSTRAP_PUBKEY` gate. It is intentionally **None** until a
reviewed Akula publishing ceremony pins a real public key in signed source.
The public factory always returns `UntrustedBootstrapSigner` while that
publisher authority is unprovisioned, even if a caller supplies a matching
candidate signature. A private test factory accepts only an independently
fixed test key to exercise signed B1a frames and negative cases in CI.

No runtime SSH value, environment variable, package contents, test key or
caller argument may provision the production root trust anchor. The signed
helper distribution, key ceremony and rotation policy still require a
separate audited release decision. Staging and UI installation remain HOLD.

### Opt-in post-reboot root readback (still not an installation receipt)

The fixed Python root-stage code has a second exact command,
`--inspect-hold`, which opens only the preexisting root-owned staging
directory without creating missing directories or writing any files. It
revalidates the pinned publisher key, canonical B1a signature, full staged
helper SHA-256, ELF type, ownership, file modes and the original HOLD marker.
Other arguments are rejected. The command never executes the helper,
installs Node, or starts systemd. It is unprovisioned until the independent
publisher root key is distributed in an audited signed release.

The experimental Windows SSH adapter reuses the pinned OpenSSH identity and
the fixed compiled Python source to request read-only reinspection. Its
`RemoteReportedReinspectedHold` result requires an exact response
containing the previously verified helper SHA-256. It is **not** a signed root
witness, B1d0 CAS receipt, or authorization to start a service. Both readback
and read-only modes are tested with valid, modified, wrong-key and
unsupported-command fixtures. This feature remains outside ordinary builds.

### Integrated B0 v3 plan → real signed Node-V1 ELF → B1d0 `/opt` CAS proof

The branch CI now exercises the pre-existing **actual signed Node-V1 service
ELF** through its already isolated B1d0 root-owned `/opt` test, rather
than relying only on synthetic staging tests. A new root-only test builds an
immutable B0 v3 preview against a pinned *simulated* SSH KEX using the exact
same-HEAD real Node ELF SHA-256, the separately disclosed helper SHA and
fixed public UDP/43871 action list. Its **actual BLAKE3 v3 plan digest** and
pinned host hash enter the B1d0 fixture's detached owner signature, staged
package, witness and fsynced CAS receipt.

Before the valid commit, the test generates another mathematically valid
owner signature over a different plan digest. The independently pinned
scope must reject it before any `/opt` write or CAS revision. The valid
approval then commits the exact ELF once, verifies the receipt and bytes,
refuses replay, and reopens the durable witness into an explicit HOLD.
All writes happen in an isolated GitHub Linux mount namespace with test
keys; the production `/opt/akula-node` and witness directories are
asserted absent and never created. No helper is executed or service started.

This closes a **test integration gap**, not the deployment acceptance gate.
The root installer still needs a production parser to verify the submitted
B0 v3 plan against independently trusted helper state, real publisher and
owner key enrollment, service/systemd activation and authorized VPS E2E.

### Root-side B0 v3 canonical plan / owner correspondence (isolated gate)

The existing B1d0 CAS test now has a **separate** Linux root-side, non-shipping
`server-root-b0v3-gate-lab` validation module. It parses a size-bounded
canonical JSON `DryRunPlan` as an exact B0 v3 schema (unknown fields and
noncanonical encodings are refused), recomputes the distinct BLAKE3 v3
digest and exact fixed-action list, and cross-checks an Ed25519 owner
signature over that same plan against the independently pinned host,
helper, Node B1a package/manifest identity and expected witness revision.
A B0 v2 plan, extra JSON fields, changed helper, wrong signer, forged signed
plan digest, stale CAS revision, wrong release size, expired claim or
unexpected OS profile is rejected. The previously used v1/v2 serialization
remains unchanged; the added Deserialize implementation is strictly for
root-side parsing.

**Important trust boundary:** the expected key/host/release values in this
pure verifier must eventually be loaded from *verified OS-root-owned*
enrollment and verified signed release FDs. They are never trusted when
supplied by an SSH peer. This new module returns a strictly **inert**
`CheckedRootV3Scope`, has no root helper entrypoint or ability to invoke
B1d0, and is not enabled in the installed client. Its tests are not proof
that production has any enrollment, authenticated host, owner signature
or activation authority. Until an independent root state loader and real
root-verified transaction are implemented, the E2E gate remains HOLD.

The isolated real-ELF B1d0 test also now runs this root-side parser on the
actual submitted B0 v3 JSON and the **same detached Ed25519 owner claim**
used by B1d0. A second valid owner signature on a changed plan digest is
rejected by the root validator before any witness CAS or /opt mutation.
Only an exactly bound, independently pinned owner claim is passed to the
existing durable B1d0 one-shot commit, after which its receipt is reopened
as HOLD. This integration remains inside a disposable mount namespace with
CI-only keys, not a VPS product pathway.

### Read-only root-owned B0 v3 trust enrollment loader

New optional Linux module `server_root_v3_trust` reads only the fixed
`/var/lib/akula/server-bootstrap/trust` hierarchy using OS UID0,
`O_NOFOLLOW`, expected owner and exact modes on each ancestor, an independent
B1d0 enrollment marker and a **separate** B0 v3 policy-enrollment marker.
The bounded, canonical `b0v3.policy` record stores owner identity and
Ed25519 public key, pinned SSH host digest, independently pinned helper
digest, publisher identity and publisher Ed25519 key. These values can no
longer be substituted by the SSH-supplied plan. The loader is read-only;
no policy files are created or enrolled by it.

`RootEnrolledV3::verify_review` also requires an already verified, stable
B1a root-owned `RootVerifiedFd` and matches the signed release publisher
against the independently enrolled publisher key. The actual Node digest,
byte count and manifest identity are obtained from that verified FD;
the full B0 v3 JSON and owner approval then pass through
`verify_root_b0_v3_scope`. Incorrect owner, modified input, wrong publisher
or stale witness remains a HOLD. All of these checks only produce inert
scope evidence; no IPC, root commit, command execution or UI activation
has been introduced.

The test uses a temporary UID0-owned directory for positive and hostile
symlink/mode/corrupt-marker proofs; a nonroot test must reject before
reading protected trust. Production key enrollment, local real host
discovery, witness locked-transaction admission, signed root receipt,
systemd and VPS installation are still unimplemented. The production
loader can only work after a separate trusted administrator action has
created the exact enrollment; this commit does not perform that action.

### Root policy must bind the actual B1a verifier identity, not a second signer argument

A security review of the first `RootEnrolledV3::verify_review` API found a
mismatch risk: it compared its independently root-enrolled publisher key with
a *separate* `TrustedReleaseSigner` argument rather than the signer actually
used when `RootVerifiedFd` authenticated the staged B1a package. The new
`RootVerifiedFd::verified_publisher` exposes ONLY the immutable publisher
identity captured from the signing trust used in its original verification.
The root-owned policy compares that saved identity directly to its enrolled
publisher key before any owner plan/nonce is admitted. The redundant
caller-supplied publisher parameter has been removed.

A disposable UID0 integration test now creates a truly root-owned signed B1a
package with a test-only Ed25519 issuer, a separate OS-owned B0 v3 policy,
an independently signed owner claim and a complete canonical B0 v3 plan.
It proves read-only acceptance of an exactly matching verified FD and
refusal of a changed enrolled publisher or a changed owner signature.
Production keys are still unprovisioned; this does not create a root install
opcode, trusted bootstrap ceremony, service launcher or live VPS acceptance.


### B1-E2E-07 — production B1d0 writer now gates on root-owned v3 review

The previous `commit_first_provisioned_opt` accepted a caller-provided
`TrustedOwnerApprover` and `ExternalApprovalScope` without loading the
new root-owned B0 v3 policy. Therefore the separately tested root-policy
validator could be bypassed. This is corrected by **removing the legacy
production wrapper** from the B1d0-only build. The only provisioned writer
is compiled under the opt-in `server-root-b0v3-gate-lab` feature in a
separate, size-limited `provisioned_gate.rs` Rust module.

Before touching the fixed `/opt` directory or advancing CAS, this entry:
(1) opens the independently enrolled root-owned B1d0 witness and checks
the exact checkpoint; (2) reads the fixed root-owned B0 v3 policy rather
than accepting an SSH-provided owner key; (3) observes the local OS from
`/etc/os-release`, compiled architecture and live systemd directory,
refusing unsupported or container-only hosts; (4) derives a single portable
owner wire from the **same signed B1d0 claim**, checks the complete
canonical B0 v3 plan, Ed25519 owner signature, independently enrolled
publisher key, pinned helper and SSH host digests, B1a signed FD size and
manifest; (5) compares the verified scope to B1d0's exact transaction
binding and only then enters the existing destination-locked B1d0
`commit_in`, which independently rechecks owner signature, floor, CAS
revision and one-use nonce. The trusted owner supplied to B1d0 is now
derived exclusively from OS enrollment, never from caller data.

The new module has a pure negative test for install ID, plan digest,
package SHA, owner nonce and SSH host divergence, and a canonical legacy
owner-wire encoding test. The previous CI proves root-policy reading and
real Node ELF CAS in isolated test-only environments. **No production
publisher key, admin root enrollment ceremony, remote privileged install
command, systemd activation or VPS run has been introduced**. This is a
code-level fail-closed authorization improvement; B1-E2E-07 stays HOLD
until a separately trusted enrollment and complete live E2E succeed.

### One-time root administrative B0 v3 enrollment (experimental, no VPS writes)

A separate opt-in, fixed-command binary
`akula-root-b0v3-enroll-lab` supports only a **manual root-admin
bootstrap ceremony**. It takes no SSH-supplied input, command arguments,
paths, keys, or environment configuration. Its only candidate is the
preexisting root-owned 0600
`/root/akula-b0v3-policy.candidate`, which must have been independently
reviewed and written by an authorized administrator after verifying the
signed enrollment binary and publisher/owner/host/helper public identities
out of band. The candidate is the strict 512-byte-bounded B0 v3 policy
record already used by the read-only root-policy loader.

Before any mutation it requires real effective UID0, exact trusted
root-owned / and /root path ownership and modes, an already enrolled
B1d0 OS witness with its own persistent root-owned journal and key, and
the same fixed `/var/lib/akula/server-bootstrap/trust` hierarchy that
the production read-only verifier later re-opens. The candidate policy
is validated before publication, the old B1d0 enrollment marker must
exist, and **every** new final or pending B0 v3 policy/marker name must
be absent. The tool creates only the exact root-owned 0600 files
`b0v3.policy` and `b0v3.enrollment` using O_EXCL, fsync, and NOREPLACE
renames; publication of the enrollment marker occurs last. A crash,
collision, symlink or replay leaves HOLD and requires independent manual
root repair; no automatic overwrite, bootstrapping, sudo privilege gain
or retry exists. No test keys may be used for real enrollment.

This command is NOT included in default releases, does not grant the
desktop application root permissions, and cannot be called by client
SSH/IPC. CI only compiles the opt-in command and tests the root-only
transaction in disposable owner-controlled directories, plus negatives
for wrong marker, malformed public policy, symlinks and interrupted
writes. There is still no independently reviewed production publisher
key or packaged verified administrator binary, no signed owner
enrollment proposal on Windows and no real VPS install. The B1-E2E-07
release gate remains HOLD.

### Fixed root-administrator B1d0 witness genesis on a genuinely empty VPS

The B0 v3 enrollment CLI previously required an already enrolled
`OsWitnessLab`, which made the clean-VPS initial state unreachable without
an unimplemented external step. A separate opt-in
`akula-root-witness-init-lab` binary now supplies that *first*
**administratively authorized** step. It accepts no arguments, paths,
SSH, IPC, public keys, candidate artifacts or environment-selected
locations and must be invoked by an operator who already holds UID0
privileges and independently verified the exact signed binary.

The sole production destination is the OS-owned `/var/lib` tree.
The command opens `/`, `/var` and `/var/lib` by fixed
`O_NOFOLLOW` directory-FD traversal, validates UID0 and mode 0755,
takes an exclusive root-owned parent lock, then exclusively creates
`/var/lib/akula/server-bootstrap/trust` and `journal` with mode 0700.
It uses the Linux OS CSPRNG to generate a fresh Ed25519 signing seed
locally and publishes `witness.seed` (0600), matching `witness.pub`
(0600), and finally the distinct B1d0 production `enrollment` marker
(0600). All file writes use an O_EXCL pending name, fsync, NOREPLACE
rename and parent directory fsync. Public witness identity is derived
from the same newly generated private key. A missing/corrupt partial
state cannot be interpreted as enrollment because the marker is last.

A pre-existing `/var/lib/akula` directory, unsafe parent, symlink,
interrupted attempt or retry is an irreversible **HOLD pending human
root review**; never silently repair, overwrite or restart a key
generation. Root-owned test directories verify proper 32-byte private
seed custody, matching Ed25519 public key, pristine genesis ledger,
single-use semantics and simulated crashes after every major step.
Production `/var/lib` is never mutated by CI tests; the CLI is
compiled only, not executed. The separate existing B0 v3 admin
enrollment may run only afterward on the same fixed protected paths.

**Unresolved production prerequisites:** out-of-band authentication
of the root-admin executable, signature/publisher key and safe
distribution, independent admin consent, real VPS E2E, actual root
commit/activation authority, systemd and post-reboot two-client proof.
This remains a non-shipping experiment and **B1-E2E-07 stays HOLD**.

### Fixed administrator-only empty /opt destination enrollment

A further clean-VPS blocker was the preexisting requirement that
`B1d0::open_root(ProvisionedOpt)` finds an already enrolled root-owned
`/opt/akula-node` directory, mode 0755, with a precise root-only 0600
`enrollment` file. The opt-in `akula-root-opt-enroll-lab` now supplies
a separate **root-administrator** creation step, strictly after
successful protected witness genesis and separately pinned B0 v3 owner/
publisher policy enrollment.

Before any filesystem mutation, it requires UID0, a root-owned
registered B1d0 witness and durable ledger, an independently enrolled
B0 v3 policy with pinned keys, and locally confirmed supported Linux
architecture/systemd. It opens only the fixed absolute `/opt` with
`O_NOFOLLOW`, requires a root-owned 0755 directory, takes an exclusive
parent lock and creates exactly `/opt/akula-node` (0755) and its root
0600 `enrollment` marker. The marker is written via O_EXCL to a
pending name, fsynced, NOREPLACE-renamed and directory-fsynced. Any
preexisting target, symlink or interrupted publication is a permanent
HOLD pending independent root inspection, not a silent overwrite or
repair. Root-only tests exercise exact mode, byte-for-byte B1d0 marker,
single-use refusal, preexisting partial Node, symlink and untrusted /opt.
Production /opt is never mutated by CI.

This is an EMPTY INSTALL DESTINATION only: it does not copy or execute
a Node ELF, advance the witness CAS, create a systemd service, open a port
or change the Windows client. The executable is compile-only in CI and
requires an externally authenticated signed release and independent
administrator consent before it may be used on a real VPS. The normal
Akula install button and B1-E2E-07 status stay HOLD.

### One CI acceptance for the three first-root administrator ceremonies

After independent root-owned witness genesis, B0 v3 policy enrollment and
empty `/opt/akula-node` enrollment were separately implemented, add an
actual **single disposable UID0 acceptance** for their composition. It
uses only temporary root-owned `var/lib` and `opt` directory FDs,
while retaining the real production B1d0 witness enrollment marker and
the same production `OsWitnessLab::open_enrolled` implementation through
a test-only path adapter. The run starts with no files, generates a fresh
OS CSPRNG Ed25519 witness, reads the genesis checkpoint with the
production marker, refuses B0 v3 review until the independent policy
is registered, publishes exact B0 v3 owner/publisher/helper/host pins,
creates the empty protected Node destination, reopens all state after
dropping the initial FDs, and verifies that all three administrator
commands refuse any replay. The witness checkpoint remains genesis and
the Node directory contains only its enrollment marker; no release
package, privileged RPC, network transfer, service or activation is used.

Only this dedicated root-only test is added to the existing
`signed-review-contract` job; no new workflow or broad fanout. The
test-only adapter is not available in a production Core build. All
signers/hosts in this test are disposable CI identities, not
production authorization. This is **evidence of composable initial
root state**, NOT a genuine live VPS E2E or permission to enable the
Windows Install Node control. B1-E2E-07 remains HOLD.

### First B1a release — approved Node identity derived only from root enrollment

The previously existing `verify_root_staged` correctly authenticated a
Linux root-owned signed B1a package FD, but its `RootPinnedReleaseTrust`
(expected publisher key, package digest, signed manifest fingerprint and
release sequence) was an argument supplied by the caller. A production
root installer must not be allowed to substitute that expected authority.

The new non-shipping `server_root_v3_trust::release_pins` read-only
entrypoint instead opens the SAME fixed root-owned trust directory as B0 v3.
It requires both independent owner/publisher B0 v3 enrollment and the
separate root-owned `b1a.first.enrollment` marker plus a strict versioned
`b1a.first.approval` record. The approval pins a positive release sequence,
exact signed-manifest fingerprint, Node SHA-256 and byte count. The only
accepted candidate directory is the root-owned 0700
`/var/lib/akula/server-bootstrap/trust/b1a-first-stage`, opened
`O_NOFOLLOW` relative to the already verified root-owned directory FD.
The B1a publisher ID and Ed25519 key come ONLY from the independently
registered B0 v3 policy. The existing B1a signature/manifest/FD verifier
has an additional FD-based entrypoint to avoid allowing client-supplied
ancestor paths, and returns an inert `RootVerifiedFd` only when the
approved SHA256/fingerprint/sequence, signed issuer, package size, inode,
owner and file modes all agree.

There is NO root writer for these release-approval files, and no path,
candidate, expected hash, trusted key or time parameter in the production
public entrypoint: current Unix time is obtained on the root host.
Until out-of-band trustworthy publication of approved release files
and real production signing material exists this loader will fail closed.
Root-owned disposable CI fixtures prove acceptance of an actually
Ed25519-signed B1a package and rejection of a changed enrolled signer,
wrong fingerprint/hash/size/sequence, missing/altered approval marker,
tampered file, expired signed release, unsafe file mode and symlink stage.
The tests do NOT execute a Node ELF or commit CAS and remain HOLD.

### B1d0 cannot accept caller-provided release authority or a request clock

The opt-in fixed-root `commit_first_provisioned_opt` previously received
a `RootVerifiedFd` from a caller and a caller-selected `now_unix`.
Even though it compared the verified FD's signing key to the enrolled
publisher, the caller could choose a different genuinely signed release,
and a remote request time could be used to manipulate owner-expiry checks.

That interface has been tightened: the provisioned B1d0 entrypoint no
longer accepts any `RootVerifiedFd`, `RootPinnedReleaseTrust`,
publisher public key, approval fingerprint, package digest or timestamp.
It obtains time directly from the host OS, opens the fixed root-owned B1a
first-release approval and pinned stage via the new release-pins loader,
reconstructs the same owner wire and B0 v3 scope, then passes the exact
same verified FD and local clock to the existing destination-locked B1d0
CAS and fixed Node writer. Any missing approved release, changed signed
manifest, package hash, publisher identity, stale approval or invalid
owner signature fails *before* that write transaction.

This change does not introduce an SSH command, production helper
distribution, root activation authority, systemd or owner consent by
itself. The out-of-band operator procedure to safely publish
`b1a.first.approval` and the actual root staging files does not yet
exist, so default behaviour remains fail-closed and **HOLD**.


### 25 October 2026 — first B1a signed-release root enrollment (experimental)

A new **non-shipping** one-shot UID0 CLI \`akula-root-b1a-enroll-lab\` reads
only a previously independently reviewed root-owned 0600
\`/root/akula-b1a-first.approval.candidate\`. A separately authorized
administrator must already have populated the fixed root-owned 0700
\`/var/lib/akula/server-bootstrap/trust/b1a-first-stage\` with the B1a
\`manifest\`, \`signature\` and \`node\` files (root-owned 0400).

Before publishing any authority, it checks the existing enrolled witness and
B0 v3 policy, exact candidate format, the policy's independently pinned release
publisher Ed25519 key, valid signed manifest, Node SHA256/size, manifest
fingerprint, sequence and root-host clock. It refuses symlinks and interrupted,
partial or replayed enrollment. The root trust directory is locked; exclusive
0600 approval and fsync precede publication of the enrollment marker LAST.
A fresh read through the original production B1a verifier closes the
transaction. Only disposable UID0 test fixtures and CLI compile-only are run
in CI. Neither normal installed Akula nor any real VPS is changed.

This does NOT provide independent production signing key/artifact publication,
safe staging of release files on a real VPS, a reviewed human-admin ceremony,
protected installation transport, root IPC, systemd activation, reboot recovery
or a two-installed-Windows acceptance. Install Node / B1-E2E-07 remain HOLD.


### 25 October 2026 — root-only first-release staging, distinct from approval

A separate opt-in non-shipping administrator CLI \`akula-root-b1a-stage-lab\`
is a prerequisite to the existing \`akula-root-b1a-enroll-lab\`. It consumes
only a root-owned 0700 fixed source directory
\`/root/akula-b1a-first-source\` (with root 0400 manifest/signature/node)
and an independently reviewed 0600 fixed candidate
\`/root/akula-b1a-first.approval.candidate\`. An authorized administrator
must populate and authenticate these inputs independently; Akula Windows
never stages them via an SSH-controlled path.

After independently enrolled root B0v3/witness checks, the CLI uses that
policy's publisher Ed25519 key, a canonical B1a manifest, SHA256/length,
approval fingerprint/sequence and Linux time to verify the real source.
Under the exclusive root-trust lock, it creates the **single-use**
\`b1a-first-stage.pending\` directory, copies bounded manifest/signature
and streams the already verified Node FD with rehash, writes root-owned
0400 files, fsyncs each and the directory, independently verifies the
**copied** bytes, then publishes via NOREPLACE rename with parent fsync.
Existing, interrupted, symlinked, corrupt, expired or mismatched state HOLD.
The separate \`b1a.first.enrollment\` marker is NOT created by staging.
UID0 tests use only disposable roots and synthetic signing keys. The
fixed production-root CLI is compile-only in CI and is not run on a VPS.

This is not actual release publication by a production issuer: no signing
ceremony, independently authenticated copy to real /root, installed helper
provenance, Windows root install transport, systemd, reboot or second-device
File Gate acceptance. Default installed Akula and \`main\` remain unchanged.


### 25 October 2026 — one canonical owner-signed B1b1b frame decoder

The existing Windows/Linux B1b1b owner-approval wire has gained a
strict, bounded, canonical binary decoder. It is not a second signing
protocol. One frame is the exact original claim followed by 64 detached
Ed25519 signature bytes; unknown domains or versions, malformed
identities, truncated or trailing data, invalid shape and altered
signature are rejected.

\`SignedOwnerWireFrame::verify_pinned_owner\` requires the independently
enrolled owner identity and key (never a public key from a remote
request). An optional Linux-only bridge returns an inert
\`SignedOwnerApproval\` with byte-for-byte unchanged canonical payload,
for the existing B1b1b/CAS checks. Tests cover canonical parity on
Windows/Linux, forged scope, changed identity, replay time limits,
truncation, UTF-8 and oversized input. No SSH installation dispatch,
privileged permission, systemd or actual VPS is enabled; release HOLD.


### 25 October 2026 — root-enrolled owner-frame admission to EXISTING B1d0

A narrow non-shipping root-only adapter connects the canonical
Windows owner-signed B1b1b frame to the existing B0 v3/B1d0 transaction.
The fixed root-owned trust directory supplies the publisher-independent
owner ID and Ed25519 public key and root host supplies the clock.
A bounded parser refuses altered or malformed frames. Only an exact,
signature-verified immutable \`SignedOwnerApproval\` reaches the
**existing** \`commit_first_provisioned_opt\` root gate; it still validates
the host's B0 v3 plan, independently approved B1a release and real
Node FD, witness checkpoint, signed owner scope and durable CAS.
No new RPC or SSH command, helper privilege, systemd unit activation or
Node installer is exposed by this change.

Disposable UID0 tests verify acceptance of a root-enrolled signing key,
bad key/identity, missing witness/policy marker, wrong file type or
symlink, tampered or truncated frames, expiry and nonroot refusal.
Additional pure B1d0 correspondence tests reject modified package,
plan, proposed floor and witness revision. The default Windows Akula
and actual VPS remain unchanged. Full installed end-to-end = HOLD.


### 25 October 2026 — protected one-shot AF_UNIX owner preview (no root installer)

The existing B1b3a lab already provides Linux kernel SO_PEERCRED
authentication but its general Preview accepts a policy passed by the
caller; that is NOT independent root install authority. Instead, a
new opt-in, **non-shipping** \`server_root_v3_trust::owner_ipc\` adapter
opens only the previously enrolled root-owned B0 v3 policy and a
separately provisioned root-owned 0600 \`b1d0.peer-uid\` record. The
record has a fixed domain and a nonzero, non-root Linux UID; callers
cannot provide or change it. There is deliberately no production writer
for this record and no installed listener.

A real accepted AF_UNIX stream is gated on Linux SO_PEERCRED matching
that independently pinned UID before reading an exact, capped, domain
separated B1b1b preview frame. The peer must end its write half after
one complete frame; partial, oversized, appended, wrong-domain or
hanging requests HOLD. Only the independently pinned root owner ID/key
can authenticate the nested detached Ed25519 owner signature. A
freshly signed request for a *different* SSH host is also refused by
the root policy's separate host pin. Expired, tampered or malformed
requests never yield an inert preview.

The Linux UID0 CI uses actual AF_UNIX sockets and a separately
unprivileged (UID 65534) Python sender in disposable directories
to prove kernel peer identity, correct signature, root-owned policy,
replay being *preview-only*, bad pins/permissions/symlinks, different
host, trailing/truncated frames and nonroot service refusal.
No sudo command, interpreter, listener, socket file or user creation
is installed onto a real VPS. In particular this admission DOES NOT
call B1d0 CAS, modify /opt, activate systemd, give SSH/root privileges
or enable Install Node. Root-helper signer \`PRODUCTION_BOOTSTRAP_PUBKEY\`
remains unprovisioned and the authorized privileged transport is HOLD.


### 25 October 2026 — one-shot privileged registration of local IPC peer identity

The non-shipping administrator-only \`akula-root-b1d0-peer-enroll-lab\`
command requires UID0 and accepts **no** args, arbitrary paths, SSH
payloads, account names or stdin. Before any mutation, an independently
authorized administrator must provide a protected 0600
\`/root/akula-b1d0-peer-uid.candidate\` record containing exactly the
canonical root IPC peer domain and four-byte big-endian UID. The
installed helper, signer and real root witness must already be
independently authenticated; this command neither provisions them nor
performs privilege escalation.

Unlike a bare UID pin, production admission also verifies a fixed
*local* account \`akula-provision\` from root-owned, regular 0644
\`/etc/passwd\`. It must have exactly the enrolled UID, nonzero GID,
locked \`x\` password field, \`/nonexistent\` home and a non-login shell.
The UID cannot be 0, 65534, MAX or shared by a second local username.
No NSS/LDAP fallback, useradd, SSH-login or environment-supplied user
is accepted. A production PREVIEW rechecks this mapping on every
request: deletion or UID recycling HOLD, rather than silently binding
a different account to the same number.

Under the exclusive root trust directory flock, the existing enrolled
B0 v3 + witness authority is revalidated; a root-owned 0600 file is
created with O_EXCL at \`b1d0.peer-uid.pending\`, fsynced, renamed
NOREPLACE to \`b1d0.peer-uid\`, then parent-fsynced and read back.
Existing, interrupted, foreign, symlinked and replayed enrollment HOLD.
No automatic repair or overwrite. UID0 disposable tests cover exact
publication/replay, wrong mode, missing policy/marker, corrupt bytes,
pending state, symlink and OS-account aliasing. The fixed production
CLI is COMPILE-ONLY in CI. It is not executed against the runner's or
the user's real root state.

**Still HOLD:** no production publisher/root helper trust key is
pinned, no authorized root IPC install command exists, no installed
socket listener, systemd activation, reboot/readback or Windows
A-to-VPS-to-Windows B acceptance. Normal Akula, \`main\` and real VPS
remain untouched; B1-E2E-07 is not user-installable.


### 25 October 2026 — shadow-locked local IPC identity (experimental)

The previously added read-only local account check deliberately did not
claim that \`x\` in \`/etc/passwd\` proves an unusable password.
The next non-shipping gate now also reopens the root-owned
\`/etc/shadow\` via the previously validated \`/etc\` directory
descriptor on every fixed-root IPC Preview and before one-use UID
enrollment. No remote/account path is taken from SSH, argv, a frame
or environment.

It requires a regular single-link UID0 shadow inode with only a
conservative 0000/0400/0600/0640 permission profile, no symlink,
bounded size and stable inode/metadata across the read. The fixed
\`akula-provision\` entry must exist **exactly once** with a password
field beginning with \`!\` or \`*\` (Linux locked/non-crypt marker).
Active crypt hashes, empty fields, duplicate/missing/malformed entries,
unexpected file permissions and changed/symlinked inodes fail HOLD.
All shadow bytes remain confined to the root process and are never
printed, returned to an IPC peer or persisted in the trust directory.

Disposable CI tests cover ordinary locked markers, active and empty
passwords, duplicate and malformed entries, exact metadata permissions,
hardlinks and an attacker-selected symlink. The tests NEVER modify
the runner's real /etc/shadow. There is still no privileged IPC
Install/Commit opcode, no root helper publisher key, systemd service,
or live VPS deployment. A locked password does not independently prove
SSH/PAM policy, systemd service restrictions or helper provenance;
these remain separate production blockers.


### 25 October 2026 — root staging helper digest bound to independent B0v3 policy (HOLD)

The first-root Python stage at root-controlled
\`/var/lib/akula/server-bootstrap/bootstrap-v1\` publishes
\`manifest\`, \`signature\`, \`helper.staged\`, \`release.pub\`,
\`release.ed25519\` and \`HOLD\`. This non-shipping increment introduces
an **independent Rust read-only root check** that opens the same fixed
staged directory through the already authenticated root \`trust\` FD
and rechecks exact UID0-owned 0400 file inodes, size, metadata,
canonical signed B1a manifest, Ed25519 signature, exact public-key PEM
and the original on-disk HOLD marker. It streams the *real* ELF inode,
checks Linux x86-64 magic and hashes its full bytes.

The stage-provided signing key alone is **not** a trust anchor. The
verified signed helper digest must also match the independent, fixed,
previously root-enrolled \`helper_sha256\` in B0v3 policy, otherwise
even a newly self-signed helper cannot pass. The existing
\`commit_first_provisioned_opt\` B1d0 path now demands this read-only
admission before it can reach B1a Node/witness/owner CAS validation.
Its only result is a metadata-only \`BootstrapHelperHold\`; no
executable descriptor, process spawn or authority escapes.

Disposable root-owned CI tests use synthetic signed bytes to prove
matching staged-ELF/hash/identity/marker acceptance, and refusal on
modified helper bytes, wrong independent B0 helper digest, changed
signer/signature, malicious PEM, interrupted/missing stage, symlinks,
hardlinks, unsafe permissions, truncation and expiry. No production
signer key has been provisioned: Core still has
\`PRODUCTION_BOOTSTRAP_PUBKEY=None\`. This verifies *staged bytes* only;
it does not authenticate the binary currently running as root, issue
a privileged IPC install opcode, enable systemd or constitute VPS
acceptance. Default installed Akula and main remain unchanged.


### 25 October 2026 — actual running privileged helper inode must match signed B0v3 bytes (HOLD)

A staged signed 0400 helper in \`bootstrap-v1/helper.staged\` proves data
custody but **not** which process presently runs as root. This
non-shipping increment makes an additional read-only check mandatory
on the existing root B1d0 production CAS admission path.

Root must open fixed directories \`/usr\` (0755),
\`/usr/libexec\` (0755), \`/usr/libexec/akula\` (0700) with
NOFOLLOW and exact UID0 ownership, then open the exact installed
\`/usr/libexec/akula/akula-root-b1d0-helper\` (root-owned regular
single-link 0500). It obtains the kernel's \`/proc/self/exe\` target
FD **of the actually running privileged process**, requires identical
device/inode identity to the fixed file, stable metadata, exact
signed helper size/hash, Linux x86-64 ELF and no trailing bytes.
A signed but unrelated executable, deleted/replaced inode, hardlink,
wrong mode, wrong digest, fake process identity or missing fixed path
returns HOLD. The staged helper still must independently pass B1a
signature and B0v3 digest verification; that stage remains 0400,
not executable, and cannot itself launch the helper.

No binary is installed or executed by these checks. No signing key,
systemd socket, elevated SSH command or privileged IPC opcode is
introduced. A real future independent release/install ceremony MUST
disclose and authorize this fixed executable path in the B0 v3 plan
and separately install signed root-owned 0500 bytes; the current plan
does not implicitly authorize writes under \`/usr/libexec\`.
The existing service installation and both Windows clients remain
unchanged; user-facing Install Node remains disabled.

Root-only disposable tests inspect two independently opened FDs
pointing to the same fixture ELF, reject another identically signed
inode, wrong permission/digest/size/links and symlink path; production
path also requires the trusted fixed /proc/self/exe and cannot accept
an injected test FD. This is a **process provenance gate**, not
a root-helper release, deployment, B1d0 activation or production acceptance.


### 25 October 2026 — B0 v3 owner approval explicitly binds root-helper install destination

The installed-executable provenance gate previously hard-coded
\`/usr/libexec/akula/akula-root-b1d0-helper\`, but the reviewed B0 v3
plan did **not** contain that path. That was a real authorization gap:
a user could sign the plan without seeing the separately required
root-owned helper binary destination.

The opt-in, unreleased B0 v3 contract now carries the exact fixed
\`bootstrap_helper_install_path\`, requires
\`InstallRootOwnedBootstrapHelperAtFixedPath\` as a separate
privileged proposed action and includes BOTH in the existing
domain-separated canonical BLAKE3 owner-review digest. A wrong
path, traversal variant, omitted path, missing action or replay
of a former B0 v3 plan with the old digest fails closed, even
if the attacker recomputes an internally consistent digest.
The Linux B0v3 verifier checks the same fixed constant against
the root process image implementation, not against a request-supplied
path. B0 v1/v2 omit the new field and retain identical canonical
hashes and action lists. Any previously signed **experimental**
B0 v3 plan requires a fresh review and owner signature; there is
no silent migration. The new field describes an intended privileged
operation, **not** permission to execute it.

The service installed binaries remain untouched and no helper
installation is implemented. Exact OS-root test of staged and
running helper, independently pinned publisher key, IPC authorization
and user approval remain necessary before any root write. Production
helper signer is not enrolled; no VPS was accessed.


### 25 October 2026 — executable root-helper candidate (unsigned, HOLD)

The opt-in \`akula-root-b1d0-helper\` now compiles as a REAL Linux ELF,
not a mocked helper. It is deliberately non-shipping and UNSIGNED.
It accepts zero CLI arguments, no user-provided paths, commands,
keys, SSH text or configuration. A future root-owned socket-activated
service may supply only one already connected AF_UNIX fd 0; no
listener, socket, service or account is installed by this change.
The helper first verifies that its own running executable inode
matches the independently B0v3-pinned signed staged ELF and fixed
root-owned /usr/libexec path. Only then does it reuse the existing
SO_PEERCRED + independently enrolled peer UID + locked local account
+ strict B1b1b owner signed-frame preview. Even a valid preview ALWAYS
exits 2 and explicitly reports installation HOLD. There is no B1d0
CAS, /opt writer, sudo call, systemd activation or new privileged
protocol opcode. Nonroot, arguments and absent independent root
authority are denied.

The branch gate compiles this actual binary and checks rejection with
\`--install\` and stdin; a separate GitHub Actions candidate workflow
builds a locked release ELF, checks architecture and 64 MiB limit,
computes SHA256SUMS and uploads a short-lived artifact explicitly named
\`UNSIGNED-NOT-FOR-INSTALL\`. CI has no publisher private key and
performs NO signing. The artifact is not a production release and
must not be installed to any VPS. The Windows release still pins
\`PRODUCTION_BOOTSTRAP_PUBKEY=None\`.

Remaining production blockers: independent publisher key and signed
release provenance; administrator-authorized fixed-path ELF install
on an explicitly selected VPS; protected systemd socket, limited
B1d0 write opcode with durable one-use CAS, service health and
reboot; full installed Windows A -> authorized VPS -> Windows B.
