# MAC-HOLD-01 — macOS platform freeze

**Decision:** 2026-10-19. **Status:** Frozen / P2, not supported for current Akula release acceptance. **Active platform focus:** Windows desktop and Linux CI/desktop. Android/mobile networking is a separate future milestone, not a macOS substitute.

## What is frozen

- No new macOS-specific UI, updater, packaging, signing/notarization, installed-host or S3 synchronization development.
- No macOS installer or updater packages in new Akula release manifests; do not publish a new `darwin-*` entry, reuse an older macOS artifact, or describe macOS as supported.
- Do not dispatch or rerun macOS-only workflows while frozen. Existing in-flight macOS CI outcomes remain historical evidence, not an active acceptance dependency.
- In touched automatic cross-platform workflows, accept **Windows and Linux**, not macOS. Preserve their full security tests, including installed-host custody, SQLite V7 claim, File Gate, HistoryService and postflight. Do not turn red Windows/Linux checks green with `continue-on-error`, `|| true`, or a skipped-required status.

**The macOS source code stays in the repository.** This is a support and engineering-priority decision, not a blanket statement that the code is broken or a request to remove macOS-specific files.

## Consequences for existing sync work

- #358: exact-head platform and Main PR checks actually passed before merge. It provides only the in-File-Gate old-inode check and does **not** authorize a shipping S3 overwrite.
- #363: history-version evidence is a bounded point-in-time observation, not a durable archive. Requalify it on supported Windows and Linux after integrating #358; macOS checks remain **Deferred**, never treated as passed by policy.
- **Production S3 existing-file replacement stays HOLD on every platform** until an independently reviewed single-publication native receipt, durable SQLite V7 evidence, recovery proof, and remote-DAG/baseline admission exist. No `StageDownload::mark_applied` or `ConfirmedBaseline` promotion just because a CI matrix was reduced.
- GitHub ruleset `Protect main` continues to require `Main PR gate` on every PR. Linux/Windows acceptance steps remain real executed tests. Future macOS reactivation must not relabel earlier cancelled jobs as successes.

## CI and packaging changes

The active auto-run sync/host/file-identity and selected native desktop regression matrices omit macOS runners while frozen. macOS-only updater/smoke jobs are skipped rather than deleting the workflows, preserving exact test definitions for reactivation.

The manual multi-platform release workflow skips its macOS build and constructs a manifest **only for Windows and Linux artifacts**; it must not require or list old Apple packages. Existing public releases are outside the scope of this source change: if they contain older macOS assets, they are not newly certified.

## Reopening the macOS track

Reopen explicitly in a separate `MAC-RESUME-01` change, not as an incidental side effect of editing shared CI:

1. Confirm an actual macOS product owner/test machine and a stable eligible GitHub runner.
2. Reestablish native compile, UI smoke, update/install, File Gate/history, crash recovery and S3 custody tests on supported macOS versions.
3. Review platform-specific filesystem durability, symlink/host identity rules, signing/notarization and release provenance; restore new macOS artifacts only after independent acceptance.
4. Restore automatic macOS matrices and explicit required checks **before** marking the platform supported.

Until then, focus delivery, QA and roadmap capacity on Windows/Linux. Never use this freeze as a justification to weaken common-path data-safety behavior.
