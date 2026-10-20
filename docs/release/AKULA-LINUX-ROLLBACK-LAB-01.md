# RELEASE-LINUX-ROLLBACK-LAB-01 — crash-safe AppImage transaction experiment

Status: **isolated Draft / no runtime integration / no update authorization**. Canonical plan: [AKULA-MASTER-INDEX-01](https://app.notion.com/p/3fc32a84beec813e869fdf83a61e626b).

## Current production defect and narrow lab scope

In `aquilum-app/native/src/updater/apply.rs`, the Linux updater stages a download into `.update`, then renames it directly over the installed AppImage and starts it without confirming executable health. The previous executable is lost at publication and `spawn()` success does not establish a working Akula GUI.

`scripts/linux-appimage-transaction-lab.rs` is a standalone, std-only Rust executable tested with `rustc --test`. It does **not** compile into the shipped Akula binary. All test operations use freshly created, disposable directories with fake `akula-app_0.3.9_x86_64.AppImage` and an unrelated `Aquilum.AppImage` sentinel.

## Filesystem transaction and recovery contract

1. Open the stable sibling `.akula-appimage-update.lock` and acquire a nonblocking OS `flock` before examining any generations. Never rely on a PID lock or remove this lock file.
2. Accept only an explicit Akula AppImage filename, a real directory and real nonlinked regular files. One 32-lowercase-hex generation directory maximum; any competing or malformed generation enters HOLD.
3. Write candidate bytes into a same-parent staging directory with `create_new`, bounded payload length and `sync_all` of the candidate, then a generation-specific receipt, then synchronize stage and parent directories. This receipt is **not** publisher authentication or a content hash.
4. Move old active image to `previous.AppImage` and synchronize directories. Only then move candidate into the active path and synchronize again. The two renames are **not atomic together**; the gap is intentionally covered by cold recovery.
5. If process death leaves the installed path missing and a single intact previous backup, a distinct process under the same OS lock restores it. If both installed new and backed-up old exist, report `PendingUnacknowledged`; NEVER equate process launch with acceptance.
6. Under an explicit cold-recovery decision with no GUI generation ACK, move new to `failed.AppImage`, synchronize, restore the previous generation and synchronize. Keep the failed bytes for inspection. An interruption halfway through rollback is again recoverable from the previous backup. Do not silently delete stages.

## Acceptance evidence

The isolated CI [Akula Linux AppImage rollback lab](https://github.com/Lalalalendia/Akula/actions/workflows/linux-appimage-rollback-lab.yml) must compile the actual Rust module with `-D warnings` and execute the complete native tests. Its three actual `SIGKILL` child tests require a new OS process to recover after: (a) old backup; (b) new publication; and (c) the first rename of an interrupted rollback. Two-process lock refusal, symlink refusal, ambiguous stage HOLD, staged-only HOLD and byte-for-byte Aquilum sentinel continuity are also required.

The child sets a durable marker only after the target filesystem boundary; its parent sends SIGKILL and waits, then reopens the original directory under an independently acquired OS lock. The host test must demand all three exact named results and ten successful tests, rather than trusting an empty child run.

## Non-goals and required future gates

- `fsync` + `SIGKILL` is not a power-cut or filesystem-corruption test, especially for devices and storage stacks with different durability behavior.
- Cooperative same-user directory model, **not** adversarial same-UID path security. Symlink refusal does not remove every TOCTOU or mount/rename attack.
- No hashing or publisher signature admission here. The separately signed release trust root must authenticate every exact candidate before any production transaction.
- No generation-bound acknowledgment from actual Linux first rendered GUI frame, no foreground OS process lifetime gate and no independent installer verification. Until implemented and tested, `PendingUnacknowledged` means HOLD, never delete backup.
- No `mod linux_transaction` runtime wiring, `channel_ready()` activation, private key, `gh release`, Aquilum data mutation, Sync apply, production S3 or macOS changes.

Next gate is binding a *verified* package digest + protected directory authority to this state machine, then real Linux GUI-start ACK, crash/reboot testing, and only then dry-run Akula beta-v1 -> beta-v2 upgrade/recovery.
