# VIDEO-18b-09 — Windows 10/11 video dependency license triage (NON-RELEASE)

**Status: EVIDENCE ONLY / VIDEO-18 RELEASE HOLD.** Follows accepted #989/#990 and does not touch the signed installer, in-app updater, application feature defaults, installed software, release assets or user libraries.

## Why the previous count cannot be used as a release bill of materials

VIDEO-18b-07/#989 conservatively indexed Rust dependency graphs with `--all-features` and no platform filter. The exact accepted VIDEO-18b-08/#990 artifact had **1,161 distinct packages**, of which **975** had a package-local LICENSE/COPYING/NOTICE candidate and **186** had no verified package-local license file. Those numbers span Linux/macOS, optional Akula features and lab-only crates. They are **not** the exact set linked into the five Windows video executables; `TEXT_NOT_VERIFIED` is not proof of copyright infringement.

## Narrower repeatable measurement

- Build the same conservative `--locked --all-features` graphs for Akula native, pure Rust mux and pinned Shiva CLI `cb542d30d2c18a18b1ee457f5158d5b0f551757e` without compiling software.
- Separately use Cargo `metadata --locked --filter-platform x86_64-pc-windows-msvc` for these **explicit roots**. Enable `installed,video-rust-bundle` on the Akula native root; keep mux/Shiva CLI default feature selection.
- Walk only the target-filtered `resolve.nodes` via the same proven #989 helper; **never** count unrelated items remaining in Cargo's `packages` array. Reject any target package that is absent from the pinned all-features reference or lacks #990's package license-file evidence.
- Write a JSON comparison: selected Windows dependency closure by root, deduplicated total, excluded conservative-only packages, and first-priority missing-text review queue. Prioritization orders manual review but does **not** auto-classify licenses as safe/unsafe. A Cargo package with `source = null` is only known to be in the local checkout; it may be upstream/vendor code. It is categorized as `LOCAL_SOURCE_OR_VENDOR_REVIEW`, not proof of first-party copyright.
- Keep five exact binary names, source HEAD, fixed Shiva pin and `release_admitted=false`, `legal_approval=false`, `license_text_review_complete=false`; source-only CI uploads **only** one-day JSON, no license documents, binaries or installers.

## Remaining gaps (mandatory before VIDEO release)

1. Review **actual license texts**, copyright notices, source/package bundles, exceptions and compatibility for selected dependencies, including dependencies that have an SPDX expression but no package-local file; record the decision owner and provenance.
2. Investigate actual embedded yt-dlp and QuickJS binary constituents, dynamic/native libraries and non-Cargo assets; the Rust closure does not cover these.
3. A Cargo graph **is not a linked PE/SBOM**: target-specific metadata is a narrower planning signal, but build scripts, native code, runtime dynamic loading, feature unification and final packed binary contents need an independent report.
4. Protected signed five-EXE installer, signed identity/migration, generation-atomic update/rollback, runtime tamper tests and real installed Windows 10/11 GUI acceptance remain separate P0 release gates.
5. Keep older Windows 7/8 fallback on the stock two-executable build, without bundling the pinned Windows-10+ yt-dlp runtime.

**No release admission, signing, production deployment, migrations, hidden downloader or cloud media sync.**
