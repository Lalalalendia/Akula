# RELEASE-LINUX-ROLLBACK-LAB-01 — crash-safe AppImage transaction experiment

Historical acceptance: **PR #480 merged as an isolated lab**. The follow-up RELEASE-LINUX-RUNTIME-GATE-01 transfers the same crash-tested engine into the native application while retaining a mandatory production HOLD. No update authorization has been granted. Canonical plan: [AKULA-MASTER-INDEX-01](https://app.notion.com/p/3fc32a84beec813e869fdf83a61e626b).

## Current production defect and narrow lab scope

**Before the runtime-gate follow-up**, the Linux updater wrote into `.update`, replaced the installed AppImage and exited without proof of a healthy GUI. That direct-overwrite path has now been removed. This paragraph records the historical defect, not the current implementation.

The original standalone `scripts/linux-appimage-transaction-lab.rs` was moved to `aquilum-app/native/src/updater/linux_transaction.rs`. It remains independently testable with `rustc --test`, and now **also compiles into Akula** behind a mandatory non-publishing runtime admission HOLD. All test operations use freshly created, disposable directories with fake `akula-app_0.3.9_x86_64.AppImage` and an unrelated `Aquilum.AppImage` sentinel.

## Filesystem transaction and recovery contract

1. Open the stable sibling `.akula-appimage-update.lock` and acquire a nonblocking OS `flock` before examining any generations. Never rely on a PID lock or remove this lock file.
2. Accept only an explicit Akula AppImage filename, a real directory and real nonlinked regular files. One 32-lowercase-hex generation directory maximum; any competing or malformed generation enters HOLD.
3. Write candidate bytes into a same-parent staging directory with `create_new`, bounded payload length and `sync_all` of the candidate, then a generation-specific receipt, then synchronize stage and parent directories. This receipt is **not** publisher authentication or a content hash.
4. Move old active image to `previous.AppImage` and synchronize directories. Only then move candidate into the active path and synchronize again. The two renames are **not atomic together**; the gap is intentionally covered by cold recovery.
5. If process death leaves the installed path missing and a single intact previous backup, a distinct process under the same OS lock restores it. If both installed new and backed-up old exist, report `PendingUnacknowledged`; NEVER equate process launch with acceptance.
6. Under an explicit cold-recovery decision with no GUI generation ACK, move new to `failed.AppImage`, synchronize, restore the previous generation and synchronize. Keep the failed bytes for inspection. An interruption halfway through rollback is again recoverable from the previous backup. Do not silently delete stages.

## Acceptance evidence

The CI [Akula Linux AppImage rollback lab](https://github.com/Lalalalendia/Akula/actions/workflows/linux-appimage-rollback-lab.yml) compiles `native/src/updater/linux_transaction.rs` with `-D warnings` and executes the full suite, including the additional production HOLD entrypoint test. Its three actual `SIGKILL` child tests require a new OS process to recover after: (a) old backup; (b) new publication; and (c) the first rename of an interrupted rollback. Two-process lock refusal, symlink refusal, ambiguous stage HOLD, staged-only HOLD and byte-for-byte Aquilum sentinel continuity are also required.

The child sets a durable marker only after the target filesystem boundary; its parent sends SIGKILL and waits, then reopens the original directory under an independently acquired OS lock. The host test demands all three exact named results and **eleven** successful tests, rather than trusting an empty child run.

## Non-goals and required future gates

- `fsync` + `SIGKILL` is not a power-cut or filesystem-corruption test, especially for devices and storage stacks with different durability behavior.
- Cooperative same-user directory model, **not** adversarial same-UID path security. Symlink refusal does not remove every TOCTOU or mount/rename attack.
- No hashing or publisher signature admission here. The separately signed release trust root must authenticate every exact candidate before any production transaction.
- No generation-bound acknowledgment from actual Linux first rendered GUI frame, no foreground OS process lifetime gate and no independent installer verification. Until implemented and tested, `PendingUnacknowledged` means HOLD, never delete backup.
- Runtime module is linked, but `guarded_runtime_apply()` always refuses before writes or application shutdown. There is no publisher-root enablement, startup ACK, `gh release`, Aquilum data mutation, Sync apply, production S3 or macOS unfreeze.

Next gate is binding a *verified* package digest + protected directory authority to this state machine, then real Linux GUI-start ACK, crash/reboot testing, and only then dry-run Akula beta-v1 -> beta-v2 upgrade/recovery.
