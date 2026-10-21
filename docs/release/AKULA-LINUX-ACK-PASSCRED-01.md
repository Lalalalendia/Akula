# RELEASE-LINUX-ACK-PASSCRED-01 — kernel-attributed first-GUI ACK receiver

Status: **compiled native proof, installed publication path HOLD**. Scope intentionally independent of [PR #505](https://github.com/Lalalalendia/Akula/pull/505), whose signed-staging/full AppImage E2E is undergoing required CI.

## Why it is needed

The existing [#498](https://github.com/Lalalalendia/Akula/pull/498) installed GUI sends an inherited Unix-stream message only after a nonempty frame is presented and the native window is revealed. It checks the receiving supervisor's `SO_PEERCRED`, but that is a **snapshot of socketpair creation, not authentication of the sender**. The receiving side must **not trust the PID embedded in ACK text**.

## Implementation

`aquilum-app/native/src/updater/linux_ack_receiver.rs` is compiled by the real Akula native Linux crate, with no call site in `apply()`, installer or `Updater::download()`. It performs:

- One-time 128-bit generation nonce from the OS random source `/dev/urandom`, rejecting unavailable entropy and malformed SHA-256 digests; a nonce is consumed even on malformed data or timeout.
- `UnixStream::pair` with `SO_PASSCRED` set on the receiving socket. A single bounded `recvmsg` reads both the ACK and the **kernel-provided SCM_CREDENTIALS** associated with the data. The control buffer is naturally aligned and `MSG_CTRUNC` is denied. If credentials are absent, multiple, oversized or malformed, the ACK is rejected.
- Strict wire grammar for `AKULA-LINUX-FIRST-GUI-V1<TAB>nonce<TAB>pid<TAB>sha256<LF>`, no trailing second message, missing newline or alternate version.
- Verify text PID equals the actual `SCM_CREDENTIALS.pid`, and sender UID/GID equal the supervisor's own. Pin the live GUI PID using Linux `pidfd_open` while validating a bounded `/proc/PID/status` ancestry chain and matching process group.
- The expected wrapper is an OS-owned live `std::process::Child` with its own session, not a client-controlled PID argument. No ACK is accepted if the wrapper has exited or if the sender belongs to another wrapper/session. An ordinary AppImage payload may be a child/grandchild of the AppImage runtime.
- Return `GuiAckWitness`, **not** an update-commit or filesystem-publish token.

## Executable acceptance

Existing [AppImage beta CI](https://github.com/Lalalalendia/Akula/actions/workflows/appimage-beta.yml) compiles the exact native source with `rustc --edition=2024 --test -D warnings` and requires six tests. These create **real independent OS processes** using `dup2` for the inherited Unix socket and `setsid` for the launched wrapper, including:

1. Real positive ACK attributed by the kernel to the expected PID and accepted exactly once.
2. Wrong generation nonce, wrong digest, forged textual PID and additional wire message all denied.
3. No ACK before timeout consumes the nonce; retry/replay denied.
4. Another same-UID process in an **unrelated session** fails even after receiving the nonce and FD.
5. OS random nonce changes per instance; malformed digest denied.
6. A standalone subprocess helper proves the test truly uses kernel processes.

The test workflow asserts exact test names/count plus an execution marker, not merely the script's presence.

The same AppImage workflow also compiles a CI-only binary `scripts/appimage-kernel-ack-supervisor.rs` (linked to **the exact native receiver source**), then starts the **actual packaged Akula AppImage** a third time in the disposable Ubuntu 22.04 Docker/Xvfb GUI environment. The real installed GUI's inherited Unix FD is read via Linux `recvmsg(SCM_CREDENTIALS)`, the sender is pinned with `pidfd_open`, and the supervisor verifies nonce/digest, live process ancestry/session and package wrapper lifetime. The host script waits for the real visible GUI and an exact `AKULA_LINUX_KERNEL_REAL_GUI_ACK_RECEIVED` marker, closes the Akula window, waits for clean exit and requires the final `AKULA_LINUX_KERNEL_REAL_GUI_ACK_PASS passcred=1 pidfd=1 os_tree=1 nonce=1 digest=1` execution marker. Markdown and Aquilum SHA-256 witness files must be unchanged. This is an **actual GUI execution** and not merely a mock child, although the package hash is only a CI witness, not a pinned publisher signature.

## Still HOLD

This module is deliberately an **unconnected witness reader**, not a production supervisor. It **does not** prove executable package hash, authority over the installed destination, publisher identity, monotonic release floor, cold power-loss recovery or safe backup removal. The future supervisor must bind a **sealed SignedCandidate**, verified installed file inode, launch authority and persisted version floor to this receiver. Only then can a generation become ACK-eligible, with a separate explicit release security review. The local two-AppImage E2E #505 is independent evidence, not such approval.

`PUBLIC_KEY=""`, `channel_ready()=false`, and installed `apply()` rejection remain unchanged. No release, automatic package replacement, macOS unfreeze, S3/Sync mutation or Aquilum file writes.

Specification: [AKULA-MASTER-INDEX-01](https://app.notion.com/p/3fc32a84beec813e869fdf83a61e626b).
