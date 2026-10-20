# RELEASE-LINUX-FIRST-GUI-ACK-01 — actual installed first-GUI-frame signal

Status: **implementation candidate for non-authorizing startup evidence**, not an Akula update transaction or public release.

## Where the real GUI signal originates

In `aquilum-app/native/src/host/frame.rs` (Linux installed build), the updater's first-frame hook is called only after the native renderer returns **nonempty** damage, `output.present(&drawn)` is called and `self.reveal()` completes. No ACK comes from process spawning, creating a hidden window, an empty frame, an untrusted marker path or startup argument parsing. The existing macOS ACK lifecycle is not changed.

Linux's `updater/linux_first_gui_ack.rs` sends a fixed line only once, asynchronously, and only when all four expected inherited challenge fields are present. It accepts an inherited **Unix stream socket FD** created by the spawning supervisor, plus a 32-lowercase-hex generation nonce, a 64-lowercase-hex package hash and expected supervisor PID. It checks Linux `SO_TYPE`, `SO_PEERCRED`, peer UID and peer GID, and a bounded `/proc/<pid>/status` ancestor chain back to the exact socket-creating supervisor PID. This ancestry exception is necessary because `APPIMAGE_EXTRACT_AND_RUN=1` may fork the GUI payload and keep its runtime parent alive until cleanup. Only the parent supervisor can decide whether the actual GUI PID belongs to the launched package. There are **no file writes, network sockets/listeners or updater release switches** in the emitter.

Protocol:
`AKULA-LINUX-FIRST-GUI-V1<TAB>nonce<TAB>gui_pid<TAB>candidate_sha256<LF>`

The package SHA-256 in the message is **only an echo of the supervisor-supplied challenge**. It is not itself a secure proof that the process executed that package. A future actual updater must verify the process launch identity, executable inode, signed candidate, OS process lifetime and persisted version floor *independently*. An attacker can run a separate process with self-selected environment/challenge; the emitter by itself is **never a trust root**.

## Real acceptance

The existing [Akula AppImage beta candidate](https://github.com/Lalalalendia/Akula/actions/workflows/appimage-beta.yml) workflow performs two independent checks without adding a new runner:

- Compile the exact native source with `rustc --test -D warnings` and run five unit/process tests. A distinct OS subprocess inherits a Unix stream through `pre_exec(dup2)`. The parent checks the exact PID reported by the GUI against its launched process tree, OS process group, nonce and signed-package-hash challenge; a wrong supervisor or a regular-file FD must not generate ACK.
- Package and run the **actual installed AppImage** in the existing Ubuntu 22.04 Docker/Xvfb/Openbox GUI acceptance. In a second launch, an isolated Python supervisor creates the socketpair and nonce, launches the AppImage with the socket inherited, and refuses an ACK from any other PID, digest, nonce or protocol. The first visible window must exist, and the proof must arrive before GUI close. An exit code or omitted subprocess is not accepted as success.

The GUI test must preserve the synthetic Markdown vault and all pre-existing Aquilum witness hashes. It does not make a signed application update or replace any executable, and does not require two physical computers.

## Limits and separate P0 requirements

- #483 (native Linux AppImage transaction integration) is still blocked on macOS installer CI and remains Draft. This PR makes no attempt to bypass that requirement.
- #488 (native signed manifest) has been merged but still has no independently controlled publisher signing root and no linkage to installed transaction authority.
- A real supervisor must **mint** the challenge only after verification of the signed candidate, custody of the exact installed generation and an OS-verified process launch; bind IPC to that process and reject replay/timeout/wrong child. The stdout marker in the Docker test is only an acceptance artifact.
- `PUBLIC_KEY = ""`, `channel_ready() = false`, existing public release HOLD, protected Aquilum data, sync policies, Windows and macOS installed updaters all remain unchanged.
- Only after real Akula beta-v1→v2 installed upgrade, absence-of-ACK rollback, crash/power failure recovery and release security review can this signal be used as evidence for accepting a generation.

Source plan: [AKULA-MASTER-INDEX-01](https://app.notion.com/p/3fc32a84beec813e869fdf83a61e626b).
