# VIDEO-TOOLS-01m — real signed read-only status on Windows (non-release)

## Purpose
Prove that Akula's **existing source-implemented** diagnostic classification
distinguishes a genuine Minisign-authenticated, protected optional package
at the supported `0.1.0` version (`ReadOnlyAttested`) from the **same
three real PE binaries** under a newly signed `0.2.0` manifest
(`IncompatibleVersion`). Both states are diagnostics only, never permission
to enqueue/download, execute a Video Tool, sign a public package or update
production installations.

## Real acceptance
This milestone extends VIDEO-TOOLS-01j's existing disposable `windows-2025`
PR-only CI acceptance. It does **not** create another Windows package installer.

1. An exact-HEAD source-built Rust mux and pinned yt-dlp/QuickJS PE are staged.
   Ephemeral Minisign `0.12` is verified by its immutable archive SHA-256.
2. The separate unsigned CI NSIS installer installs **six exact files** under
   the OS-known protected `Program Files\Akula Video Tools` root.
   Native OS ownership/ACL, exact file count and deny-write/delete held handles
   are verified before trust classification.
3. The native Akula-source Rust witness verifies a real ephemeral Minisign
   signature and all three held PE SHA-256 digests. With version `0.1.0`,
   it proves `ReadOnlyAttested`, while the ordinary product-side
   `probe_installed_video_tools()` still returns `PublisherNotApproved`.
4. Inside the same owned disposable fixture, a fresh `0.2.0` manifest
   is independently signed and verified by Minisign. The identical protected
   package is rechecked and classified `IncompatibleVersion` through the
   **same** source function used by the product after authenticated admission.
   The production probe must still return `PublisherNotApproved`.
5. A `finally` block restores the exact original manifest and signature bytes.
   The `ReadOnlyAttested` positive case is rechecked, followed by VIDEO-TOOLS-01j's
   uninstaller, cleanup and unrelated-user-file preservation tests.
6. Source mutation tests fail when the signer, signature validation, OS ACL,
   actual held file checks, exact status classification, restore or CI
   acceptance marker is bypassed.

The Windows job must produce
`VIDEO_TOOLS_01M_REAL_SIGNED_STATES_PASS` only after both native positive
states and the original publisher-denial checks have succeeded.

## Not proved / release boundary
- A **temporary CI key** is not an approved publisher signing key. The
  `APPROVED_VIDEO_TOOLS_KEY` constant remains the empty string; the
  `shipping_admission_enabled()` function remains `false`.
- The `0.2.0` version is a fresh **test manifest label** over the *same*
  three real PE binaries, not evidence of a new product executable or a
  product update. No release version of Video Tools has been authorized.
- This does **not** claim genuine positive warnings in the ordinary Akula
  GUI: the production gate intentionally keeps those warnings unreachable
  until owner approval. The supported English/Russian message mapping remains
  separately covered by native tests.
- The stock Akula installer stays two-EXE, the installed download queue stays
  closed, and no Video Tool child process may be launched by this test.
- No production owner key, signing workflow, update channel, user Windows 10/11,
  AV/locked-file recovery, power-cut/reboot, or public distribution is accepted.

**VIDEO-18 RELEASE HOLD** and `VIDEO_FIVE_CHANNEL_READY=false` remain in force.
The next independent owner-approval milestone must establish the real publisher
root, a protected actual versioned publisher-signed installer and a genuine
update/recovery process before any executable admission or release.
