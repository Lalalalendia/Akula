# VIDEO-18b-13 — five-file Windows generation journal and abrupt-exit recovery lab

**NON-RELEASE. This is a Rust transaction experiment, not a replacement for Akula's signed updater or installer.**

## Test objective

Use one *five-component directory generation* for Akula, Shiva, Rust mux, yt-dlp and QuickJS, with an exclusive OS-backed file lock and a non-overwriting, flushed journal, so an interrupted upgrade does not silently leave a mixed old/new executable set.

The executable fixture data are synthetic and never launched or distributed. This lab does **not** sign anything, inspect real PE dependencies, alter the existing shipping NSIS, touch the installed application or test a real application GUI.

## Sequence under the OS lock

1. Confirm the root is a throwaway direct child of a real GitHub-hosted Windows runner's `RUNNER_TEMP` directory. Reject caller-supplied installation roots, non-PR execution, directory aliases and an existing interrupted generation.
2. Require exactly five regular, non-symlink executable-named fixtures in `active/ OLD` and `candidate/ NEW`.
3. Write a unique immutable `journal.v1` receipt with `create_new`, `write_all`, `sync_all` **before** any rename.
4. Rename `active -> previous`, then `candidate -> active` on the same filesystem, retaining the entire five-file previous generation. Never delete previous before a generation-bound ACK.
5. A separate *test-only* first-frame ACK is permitted only when the exact five new files and previous five old files are present with the matching receipt. It is not a real GUI ACK or cryptographic source verification.
6. Without ACK, restart under the re-acquired file lock restores the complete previous generation. If the new one was active, keep it under `failed/` for inspection. Invalid/mismatched receipts and damaged backups fail closed rather than deleting evidence.
7. With valid ACK, mark the generation committed before cleaning the backup; leave a committed receipt so interrupted cleanup is recognized on the next invocation. Refuse a second update while any unresolved journal/backup exists.

## Windows acceptance

- [ ] Standalone `rustc --test` compiles under Windows 2022 with `-Dwarnings`.
- [ ] A real child process exits with code **73** after each of the journal, backup and promotion phases; parent re-acquires the OS lock and restores old five files, or confirms no promotion occurred.
- [ ] A second simultaneous updater cannot acquire the exclusive file lock.
- [ ] No ACK means no acceptance of the new generation.
- [ ] Exact ACK commits the five-file generation together; subsequent recovery recognizes the durable committed receipt.
- [ ] Corrupt ACK and corrupted previous-file fixtures fail closed, preserving the backup.
- [ ] Existing signed/published two-EXE Akula and experimental five-EXE releases unchanged; the lab creates no release asset.

## Limits — still mandatory before shipping

This proves **process-exit** recovery and rename discipline with synthetic files, not power-loss durability of directory entries, real protected Program Files permissions, open-file/antivirus behavior, Windows NSIS signing, privileged installer migration, cryptographic five-PE source attestation or actual rendered-GUI proof. Tests are deliberately not wired into `updater/apply.rs`; the ordinary Windows beta updater remains separate and experimental video updates remain disabled (VIDEO-18b-12).

Next: integrate this generation model with a properly authorized, protected, signed Windows updater using one OS lock and a real first-render receipt; test forced machine restart and complete rollback with genuine five-EXE fixtures before allowing a video release.

**No public release, unsigned executable artifact upload, user migration, VPS action or production change.**
