# VIDEO-18b-08 — real license-file evidence, not distribution approval

Status: **NON-RELEASE / SECURITY HOLD**. This closes a stale manifest discrepancy and provides a reproducible queue of license-file evidence for manual legal and packaging review.

## Changes

- The five-EXE allowlist now reflects the first-party Rust mux's already merged `license = "AGPL-3.0-only"` declaration (#988), rather than claiming its license is absent. Its distribution status remains `REVIEW_TRANSITIVE_DEPENDENCY_NOTICES`, not APPROVED. The existing #985/#987 custody tests check the revised exact value.
- The new PR-only workflow checks out exact Akula PR HEAD and pinned Shiva `cb542d30d2c18a18b1ee457f5158d5b0f551757e`, runs `cargo metadata --locked --all-features` for Akula, Rust mux, Shiva, and reconstructs the already reviewed VIDEO-18b-07 dependency inventory.
- For each unique source/registry package, the follow-on evidence pass reads explicitly declared package-local license file or bounded conventional license candidates; records relative filenames, file lengths, SHA-256 and missing/unsafe/unreadable results. It never uploads license texts, executables or archives.
- Strict path checks reject traversal and absolute references, symlinks at the file, out-of-package resolved targets, non-regular files, empty/oversize files, missing files and unreadable files. Each file is capped at 256 KiB. A package with no inspected license text stays `TEXT_NOT_VERIFIED`.
- The JSON retains `release_admitted=false`, `legal_approval=false`, `license_text_review_complete=false`, and binary-notices `REVIEW_REQUIRED`. Merely seeing a file is not a legal decision.

## Boundaries and open tasks

- Explicit Cargo `license-file` paths outside a package root are recorded as blocked, not followed. Manually reconcile legitimate workspace-wide root documents.
- Some dependencies intentionally publish only SPDX declarations, not a local LICENSE file. That is a **review queue item**, not proof of violation.
- Package-wide metadata `--all-features` is not an exact runtime executable SBOM; it does not cover bundled resources, non-Rust library components, QuickJS and yt-dlp's included payloads or any AV/Windows-specific assets.
- A SHA digest proves which bytes were inspected in the workflow, not whether those bytes satisfy copyright, attribution, notices, source-distribution or patent conditions.
- Manual legal/SBOM/NOTICE review, real protected signed five-EXE installer, update journal/recovery/rollback and supported Windows 10/11 installed E2E are separate release P0 gates.

**No branch merge, public release, package signing, feature-default changes, production migration, website deployment or cloud video sync is authorized by this task.**
