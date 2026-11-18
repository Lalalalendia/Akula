# VIDEO-18c-02b-03 — real rar2 activation, signed first-frame ACK, confirmation and rollback

**Scope: disposable GitHub-hosted Windows `pull_request` lab only. VIDEO-18 RELEASE HOLD.**

## Goal and accepted proof

The accepted [#1013](https://github.com/Lalalalendia/Akula/pull/1013) independently compiled historical and new Akula and installed two SHA-distinct five-PE sets, but did not authorize a real `rar2` commit. This stage extends the protected lab:

1. Source pin: historical `ec6d6d48`, candidate exact PR HEAD; pinned `Cargo.lock`; four pinned helper PEs. Distinct *real source-built* Akula executable. No synthetic overlay, no downloaded replacement EXE.
2. Existing protected flat NSIS and current/ NSIS fixture, ephemeral Minisign 0.12 key and two independently verified manifests. Only the new signed manifest can authenticate the proposed candidate.
3. Copy *verified* five-file candidate into disposable `RUNNER_TEMP` stage. Restore **old** protected current/ from flat protected generation. Verify all five old and flat hashes; verify candidate signed SHA-256 and reject signing the old tree as new.
4. Under real pinned `rar2` `InstallLock`: begin journal, retain previous, activate signed candidate, check journal phase and exact five. No second lock owner.
5. On actual new GUI: private inherited Win32 handle, OS PID/creation time, visible window, bounded post-first-frame receipt and exact SignedFive manifest digest. The **two negative launches** (bad nonce and bad generation) must be observed and rolled back with old five-file current/ restored. A failed GUI launch does not count as successful negative test.
6. Stage one final candidate, run the positive actual GUI ACK, *then and only then* `UpdateEngine::confirm_candidate()`. Confirm clears active journal; signed new current/ remains, protected flat old generation unchanged, no orphan files block NSIS uninstall.
7. Dual NSIS uninstall must preserve independent private note and not leave protected root. No signed binaries, publisher keys or installers are uploaded as artifacts.

**Limitation:** both Rust GUI products still declare **0.3.20**. Signed candidate manifest label **0.3.21** is not a product version upgrade. Both signed manifests use ephemeral testing keys and do not prove an owner/publisher trust channel, production install, production updater/relay, crash injection across separate processes, or release readiness.

**RELEASE HOLD** stays in force. No release admission, installed update to users or publisher signing.
