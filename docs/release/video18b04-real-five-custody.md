# VIDEO-18b-04 — custody of the real five-video-EXE set (NON-RELEASE)

**Release admission: HOLD.** This evidence-gathering PR builds the exact five
real Windows executables but does not install, sign, distribute or publish them.

## Five-member custody

| Name | Origin and verification | License/notice review |
| --- | --- | --- |
| `akula-app.exe` | Current exact PR HEAD; native Cargo.lock; experimental `installed,video-rust-bundle,video-ui-e2e` build | Akula AGPL-3.0-only — distribution notices review required |
| `shiva-pdf-preview.exe` | Lalalalendia/shiva `cb542d30d2c18a18b1ee457f5158d5b0f551757e`, committed Cargo.lock SHA-256 `62b83af03e4079f6603eef7f7170edc4138a8233ad57829adf13baacd3f2b1d4`, built with `--locked` | CLI metadata fixed to `MIT OR Apache-2.0`; upstream LICENSE-MIT and LICENSE-APACHE exist; **transitive distribution notices still require review** |
| `akula-video-mux.exe` | Repository's `labs/video-lite/Cargo.lock`, `cargo test --release --locked` and source-built dash.exe; recorded build hash passed to Akula compilation | **REVIEW REQUIRED:** `labs/video-lite/Cargo.toml` has no license metadata; first-party ownership does not waive dependency notice audits |
| `akula-yt-dlp.exe` | Official yt-dlp `2026.10.26`, SHA-256 `f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d`, 23,575,744 bytes | Upstream Unlicense; bundled dependencies/notice obligations not fully reviewed |
| `akula-quickjs.exe` | QuickJS-NG `v0.18.0`, SHA-256 `ff03398d74ea6e6bd3a8e58058f7003225abcac229d7d57775b7aaae812a49c7`, 1,873,152 bytes | Upstream MIT; ensure full copyright/license notice included in any distribution |

Sources were inspected at their **exact tags/revisions**, not guessed from
project names. Static allowlist: `video18b04-five-exe-allowlist.json`.

## What the Windows CI establishes

A GitHub-hosted Windows PR runner checks out exact HEAD and exact Shiva commit,
builds Shiva, Rust mux and Akula, retrieves and independently hashes the two
upstream runtimes, then:

1. Checks the two reviewed local Cargo.lock digests and the upstream
   **source-tracked** Shiva Cargo.lock against 62b83af03e4079f6603eef7f7170edc4138a8233ad57829adf13baacd3f2b1d4; Shiva is built
   with `cargo build --locked`. This does **not** approve transitive licenses.
2. Copies exactly five actual PE/MZ executables to a disposable, non-release
   stage and verifies their sizes, SHA-256, reparse-point restrictions and
   source/copy equality.
3. Requires the Rust mux bytes to match the hash embedded during app build;
   requires both upstream files to match their allowlist sizes and hashes.
4. Deliberately replaces a *temporary copy* of yt-dlp with malicious dummy
   bytes and requires the hash check to reject it; restores the verified copy.
5. Captures five SHA-256 digests, sizes, Authenticode **status** and unresolved
   license labels in a JSON report. The unsigned report is retained for one
   day; **no executable bytes are uploaded**.

The test never executes outside a PR's hosted Windows runner. A green CI
means only that the exact PR source and fetched binaries have a verified,
documented laboratory custody record. It is **not an attestation signed by
the Akula publisher**; Authenticode status is measured, not an approval.

## Remaining mandatory gates

- Audit Shiva's transitive dependency licenses and package all mandatory
  notices; the previously missing CLI license metadata and Cargo.lock are fixed.
- Independently audit all transitive licenses, include the required
  notices/SBOM, and distinguish package signatures from per-PE Authenticode.
- Move real five-file custody into an owner-controlled, protected signed
  machine-wide package without changing the public two-file installer first.
- Test authenticated single-generation updates, reparse/TOCTOU resistance,
  crash journal/first-render acknowledgement, rollback and old installer
  identity migration. Verify permitted real sites on supported Windows.
- Keep VIDEO-18a HOLD until all security gates and separate owner release
  approval have completed. **No public release or production migration.**
