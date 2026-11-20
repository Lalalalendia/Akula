# VIDEO-TOOLS-01o — distinct source-built PE generations and signed floor (NONRELEASE)

## What changes

The separate optional V1 manual Video Tools package remains a **nonshipping
laboratory**. This stage builds a real Rust H264/AAC mux twice from reviewed
source and the same pinned `Cargo.lock`, using a source-controlled compile-time
feature for the later candidate. The old PE's `--video-tools-lab-build-id`
returns `0.1.0`; the new, independently compiled PE returns `0.2.0`. Their
actual SHA-256 hashes must be distinct. This is **distinct compiled PE byte
identity**, not merely a rewritten signed version label. The other two real
upstream PEs retain their separately SHA-256-pinned exact bytes.

In GitHub-hosted Windows 2025 PR CI:

1. The existing exact-HEAD pinned mux and native signed-package verifier
   are checked, with the production publisher approval still disabled.
2. A second mux is actually compiled with `--locked --features lab_mux_020`;
   both built PEs answer an exact identity request and have different SHA-256.
3. The exact old and new three-PE staging sets each produce a deterministic
   unsigned custody manifest, then distinct manifests are independently
   signed and verified using the same **ephemeral CI-only** Minisign key.
4. Two different NSIS test installers are compiled. The old installer
   installs all six files under OS-known protected Program Files; the native
   Akula-source verifier authenticates it as `0.1.0`. The standalone
   **signed version-floor** preflight allows the authenticated newer candidate
   only if its compiled mux digest differs and both upstream PE digests match.
5. Following an explicit manual uninstall, the new installer installs its
   distinct PE. The native witness authenticates actual signed `0.2.0`
   bytes as `IncompatibleVersion`, not as an approved shipping package.
   The same signed preflight rejects the older candidate, and direct old NSIS
   invocation also fails with code `77` while the new package is present.
   All six installed file hashes remain unchanged after attempted downgrade.
6. Final native checks, real uninstall and cleanup must pass before the
   exact proof marker `VIDEO_TOOLS_01O_REAL_MUX_FLOOR_PASS` is emitted.
   Source mutation and numeric version-order tests reject incorrect ordering,
   fake PE upgrades and missing publisher HOLD.

## Strict limits

- The source feature produces **lab-only** `0.2.0`; this is not a
  public Video Tools product release, approved version floor or an approved
  Windows executable. Both builds are real PEs, but the other two PEs remain
  byte-identical; no independent full three-component version upgrade exists.
- The version-floor policy here is only a candidate-admission comparison
  **while an installed authenticated package exists**. This is NOT persistent
  anti-rollback **after uninstall**. Windows protected persistent trust/floor
  state, actual owner key rotation, revocation and user-mode checks on
  Windows 10/11 still require independent implementation and acceptance.
- NSIS itself refuses **any** installation over an occupied root; the
  signed floor verifier is a nonshipping, pre-install CI witness, not a
  product updater or an enforceable user-facing downgrade rule.
- The public publisher key is absent: `APPROVED_VIDEO_TOOLS_KEY=""` and
  `shipping_admission_enabled()==false`. The ordinary installed Akula
  downloader remains closed. The standard two-EXE Akula updater is untouched.
- The signing key is **ephemeral**, entirely confined to the disposable
  Windows runner. Authentic release signing, package Authenticode, legal and
  bundled notice review, ordinary user acceptance, AV/locked-file/power-loss
  recovery and actual CDN delivery are still outside this test.

**VIDEO-18 RELEASE HOLD** and `VIDEO_FIVE_CHANNEL_READY=false` remain in
force. Manual V1 and Windows 10/11 owner-signed deployment require a separate
release approval. No public artifact is uploaded from this CI.
