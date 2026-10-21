# AKULA B1d0 — first direct-to-/opt signed Node file commit with persistent local HOLD

**Scope:** one *reusable installer transaction primitive* plus root-owned tests on a disposable Ubuntu runner, not a new CAS protocol. It factors the already accepted B1c0 verified-FD/owner-signature/CAS/rename transaction instead of duplicating it.

## Functional improvement over B1c0 and B1c1

1. A new feature-gated Linux-only Core module opens **only** the fixed preprovisioned root-owned `/opt/akula-node` directory (owner UID0, mode 0755) by first pinning root-owned `/opt`. A separately provisioned root-owned 0600 enrollment marker is mandatory; symlinks, hardlinks, foreign files, wrong modes and unexpected existing data refuse before any CAS. The production function remains `pub(crate)` and has **no caller-visible SSH/IPC route**. It cannot silently self-enroll a VPS.
2. With the caller-held exclusive destination lock, B1c0's exact root-pinned signed package FD, detached owner signature, host/plan binding, OS-owned B1b1c CAS and signed witness are reverified. A new CAS revision consumes the one-use owner nonce. The file is copied **directly into /opt**, not via a runnable /run staging inode: O_EXCL `node.pending`, SHA-256 comparison, 0555 mode, file fsync, NOREPLACE rename to `node`, directory fsync and readback.
3. The commit then creates one fixed 0600 `node.receipt.pending` with bounded, checksummed revision, record tip, release fingerprint, host/plan digest, InstallId, owner signature digest and original package SHA-256. After fsync, NOREPLACE rename to `node.receipt` and directory fsync, it independently reads the receipt and exact installed Node bytes.
4. On another process opening the root-owned CAS journal, `inspect_ci_hold()` confirms SHA-256, receipt checksum and the CAS revision/record tip/floor. It returns **`LocallyConsistentHold`, never `ReadyToExecute`**. A pending file, interrupted two-object commit, mismatch, corrupt receipt, changed inode or consumed approval without published bytes is `NeedsRepair`, not silent retry.
5. The same fixed transaction core supports deterministic fault injection (only in opt-in code) at the boundaries *after signed CAS, pending-file fsync, final Node rename+directory fsync, and pending-receipt fsync*. Reboot recovery still requires an independent live machine test. An authentic file after restart is **not** authority to activate systemd.

## Proof
A separate CI feature `server-bootstrap-b1d0-durable-node` and actual root UID0 tests run **only** under a CI-guarded, otherwise inaccessible root path `/opt/akula-b1d0-ci-node`. The test creates its own enrollment, tests a real fsynced signed owner/CAS and byte-accurate installed Node, actor/process reopen, replay refusal, four crash points, foreign file, symlink, corrupt receipt, modified Node and wrong enrollment/owner. It never creates `/opt/akula-node` or touches installed Akula. The fixture remains synthetic bytes, with B1c1 QEMU providing the independent real-ELF execution proof.

## Production gates still missing
- No fixed **production release-key/owner-key loader** exists in the root service yet. The crate-private function must remain unreachable from an untrusted socket or GUI until independent trusted OS key provisioning, one-use challenge and revocation policy are implemented.
- This is a first-install *file* transaction and local HOLD, **not an atomic file+service activation** or unattended crash repair. The CAS ledger supplied by the host must be durable separately (future root-owned `/var/lib/akula` provisioning); this module cannot accept an ephemeral journal for real deployment.
- No real Windows SSH ProvisionCoordinator, installer progress UI, signed package distribution, real owner authorization UI or VPS operation is connected. No automatic boot service, WAN bind, user vault mutation, arbitrary path/command or port is provided. Root on the user's own VPS is trusted; hostile UID0/complete snapshot rollback remain out of scope.

**Next shared B1 gate:** prove B1d0 with the *real* signed ELF inside existing B1c1 QEMU, persistent witness journal and crash reboots; only then connect a platform-neutral typed install coordinator, Windows OpenSSH pinned transport and a native confirmation wizard. No new standalone CAS laboratory is needed.

## Windows setup draft (same PR; no SSH action yet)

The installed Native application's Settings now includes a **Your server / Свой сервер** section with public-only, persisted fields: server host, SSH port, SSH account name, canonical `ssh-ed25519` host public key and the **independently obtained** OpenSSH `SHA256:` fingerprint. Core validates a strict 51-byte SSH public-wire encoding and SHA-256 consistency entirely offline. No private SSH key, passphrase, sudo password, provider token or secret appears in `AppConfig`. A legacy configuration without this section loads with empty fields and port 22.

The interface explicitly says **Not connected / Install locked** and keeps both SSH-probe and Node-install buttons disabled. Matching manually entered public fields is **not** a successful SSH key exchange or trusted server, and cannot grant execution authority. The Windows profile CI tests serialization, hostile/secret inputs, offline host-pin mismatch, presence of disabled actions and native widget compilation.

This is an actual native UI entry and a reusable public setup state, not an end-to-end install wizard yet. The next missing implementation is a Windows OpenSSH subprocess transport with independent host key pin, ACL-protected temporary known_hosts, non-secret error handling, bounded I/O and a root-owned fixed install RPC after real operator approval. Do **not** enable the buttons by merely toggling a feature.

## B1d0 exact real executable acceptance (additional)

The Linux UID0 workflow also compiles the actual repository `akula-node-lab` Linux ELF, passes its exact bytes to a test-only Ed25519 B1a signer (not a production release key), pins the resulting signed package FD, consumes a detached test owner approval through the real CAS witness, publishes exactly those bytes directly under the fixed root-owned `/opt/akula-b1d0-ci-node/node`, persists a checksummed receipt, reopens the actor and verifies `LocallyConsistentHold`. A second attempt is refused. This closes the signed *real ELF file* gap in B1d0 but **does not execute or activate** that file; independent QEMU tests B1c1/B1c2 demonstrate runtime and boot durability separately. CI must show `B1D0_SIGNED_REAL_AKULA_NODE_ELF_DIRECT_OPT_HOLD_PASS` in the root test job log. No real release signer, user VPS or private data are used.

## B1d0 production witness provenance boundary

The production-only, crate-private `commit_first_provisioned_opt` no longer
accepts an arbitrary caller-supplied `OsWitnessLab` actor. It must reopen
its **own** independently pre-enrolled root-owned witness at exactly
`/var/lib/akula/server-bootstrap/trust` and
`/var/lib/akula/server-bootstrap/journal`, with root-owned, symlink-free
ancestry `/var/lib/akula/server-bootstrap` mode 0700 and a **distinct**
24-byte `AKULA-B1D0-KEY-ENROLLED` marker. A B1b4b CI witness marker is
not valid for a production /opt file commit. No actor directories,
signing seed, private owner key, manifest or OS enrollment are created by
this function. The new read-only `inspect_provisioned_opt_hold` also uses
this fixed OS witness and returns local consistency **HOLD only**, never an
authorization to start. CI explicitly proves that a completely unprovisioned
production witness/destination is not manufactured by inspection, even if
the disposable root tests have their own separate witness actor.

An independently owned real release-signing trust anchor, owner approval,
persistent operating-system key provisioning, verified time/revocation and
power-loss recovery are still prerequisites to exposing any privileged
installation action to Windows, SSH or a local service.

## Hosted Ubuntu runner /opt ownership refusal (exact-head CI repair)

On commit b9658636, the first Linux UID0 run (#39601844241) **compiled both Rust Core test binaries and real akula-node-lab ELF successfully**, but 8 of 9 root tests failed with **DurableError::UntrustedRoot** before reaching signed file CAS. This indicates the runner's /opt metadata did not satisfy the deliberately fixed root:root 0755 trust boundary; the initial error did NOT prove a signed package/CAS defect. Do NOT loosen production owner/mode requirements for compatibility with a managed GitHub runner.

The profile workflow now logs actual /opt UID/mode, creates an empty root-owned 0755 **temporary filesystem directory**, and invokes each real UID0 test in a separate Linux mount namespace with that directory bind-mounted over /opt. This leaves the host runner's /opt untouched and keeps identical production trust rules. The test directory must be empty after test cleanup and is then removed. All Linux root tests and the real ELF marker remain mandatory. This proves only first-install file+receipt and process re-open, not power-loss of the same virtual disk or service activation.
