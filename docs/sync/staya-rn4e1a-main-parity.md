# RN-4E1a — validate disabled P2P grant and File Gate research on current main

**Original main source:** `da7cbfeaa14313a23afb02dc31c78476285cb176`; **updated with current File Gate/History changes** at `6865adddad2cf1d8aa048c080edba3bfaa186dbb` through a verified two-parent merge, preserving the eight scoped lab files.  **Prior passing lab:** [Akula #411](https://github.com/Lalalalendia/Akula/pull/411) at exact `5f4d1b61b8570ca7bca76ede3e9d81720c6c322c`; [CI Linux + Windows #39325530650](https://github.com/Lalalalendia/Akula/actions/runs/39325530650).

## Reason for a fresh branch from main

The stacked research chain (#335 → #391 → #398 → #404 → #411) diverged from the live Akula product code. The original lab base lacks newer main File Gate, S3/HistoryService, journal, host-custody and installed-host checks. Green lab CI does **not** imply compatibility with updated Core or product write authority.

This integration makes the **minimum compatible replay**: transplant the RN-4C ledger and RN-4D/RN-4E0 tests verbatim into a branch based on the current `main`; add `staya-core-grant-lab` as a **default-off** Core-only feature; run the original cross-process and real existing File Gate tests against current Core using `--locked` Linux/Windows. No modifications to the existing File Gate, sync journal, OS locks, installed runtime, Cargo.lock, transport, S3 or editor.

**No public P2P connection is accepted; no network payload is published.** All fixture bytes are locally defined inside tests. The lab exposes an arbitrary callback `with_fenced_pull_preflight`, which Rust does not make read-only; that is a research weakness, not a production permit. The entire module is excluded from native/installed builds.

## Evidence and release gate

- Scope: exactly two insertion-only changes in the Core feature/Sync module registration, four lab Rust files, one workflow and this document.
- Provenance: imported byte-identical original `peer_grant_lab.rs`, `rn4c_peer_grant_lab.rs`, `rn4d_revocation_fence.rs`, `rn4e_filegate_boundary.rs` from source `5f4d1b61b8570ca7bca76ede3e9d81720c6c322c`. No mass merge of experiment or code from the older main base.
- Required CI: **same exact head** on Ubuntu and Windows; rustfmt, scope guard, `Cargo.lock` unchanged, RN4C 4/4, RN4D 3/3, RN4E0 4/4. PR stays Draft and is not permission to merge into main even after PASS.
- Failure interpretation: breakage on main is a real API/lock interaction requiring investigation, **not** permission to disable host custody or weaken the File Gate.
- Evidence limit: deterministic local fixture / two cooperative SQLite processes ≠ hostile same-user protection, real Bao staging, server/device enrollment, signed durable receipts, Android/Windows persistence, power-loss recovery, installed-host custody or actual Roha/noq transfer.

## RN-4E1b: the required new code, not included in this parity experiment

Before enabling actual P2P file writes, design a non-cloneable, Core-owned admission interface without arbitrary callbacks; bind the *authenticated* QUIC endpoint to a local enrolled DeviceId and canonical shared VaultId (never `VaultId::from_uuid(workspace_id)`). Use independently verified pinned Bao stage outside user vault, a durable intent, the **existing** handle-relative `local_create::create_with_hook`/File Gate custody route, no-replace and post-publish readback. Agree global lock-order across OS custody, DocumentHub reservation, Core File Gate, grant SQLite and journal; **never hold a SQLite write transaction during network retrieval**. Define crash cuts and NeedsReview/Partial on ambiguity before signed/replay-safe receipt or ACK.

**STY-G2-06 remains the upstream hard gate for production Staya storage.** Do not merge this research slice or deploy an auto-sync worker based on an isolated success.

## Main compatibility merge

On October 20 the upstream main introduced two additional commits touching `files/gate/write.rs`, `files/gate/mod.rs`, `sync/guarded_note_write.rs`, strict HistoryService tests, and headless smoke. Our research files have zero path overlap with these modifications. The explicit merge preserves these newer product changes, and CI compares the 8 research paths against `git merge-base HEAD origin/main` rather than pretending the older main SHA is current. The merge proves provenance and zero overlap by path only; it does **not** prove semantic compatibility until exact merged HEAD Linux/Windows success.
