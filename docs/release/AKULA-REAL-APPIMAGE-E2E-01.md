# RELEASE-APPIMAGE-REAL-E2E-01 — two installed AppImages on one CI runner

Status: **proposed executable acceptance, pending exact-head CI**. This lab is not a production release or an updater enablement.

## Two actual packaged binaries, not two renamed copies

The existing `Akula AppImage beta candidate` workflow first builds and packages the actual unmodified `0.3.9` candidate from its pinned PR merge HEAD. **Only inside that disposable GitHub-hosted runner**, a second native release binary is compiled after changing the local `aquilum-native` crate's version from `0.3.9` to `0.3.10` in `Cargo.toml` and the matching workspace `Cargo.lock` package record. The independent Akula packager manifest also identifies `0.3.10`. The workflow packages this distinct executable into `dist/e2e-v2/akula-app_0.3.10_x86_64.AppImage`, records SHA-256 for **both complete AppImages**, and restores all three modified tracked files.

**Important provenance qualification:** v2 is a **CI-only version variant compiled from the same Git commit**. It is genuinely rebuilt and carries different compiled/version metadata, but it is *not* a separately reviewed source commit, signed public beta, or evidence that a real vendor release pipeline is ready. Its separate unsigned CI artifact explicitly says `NOT-RELEASE` and expires after three days.

## Real OS-level experiment

The existing Ubuntu 22.04 Docker/Xvfb/Openbox AppImage acceptance also receives the two packaged binaries and `scripts/appimage-real-upgrade-helper.rs`, compiled with `rustc -D warnings` from the **same native `linux_transaction.rs` source** as Akula. It refuses paths outside fresh top-level `/tmp/akula-release-e2e-*` directories and requires CI-only environment guards. It never mutates the user's installed files.

The new `scripts/appimage-real-upgrade-accept.py` proves:

1. `v1` and `v2` have correct 0.3.9/0.3.10 package names and **different actual SHA-256 digests**; both are regular accessible AppImages.
2. Generate an **ephemeral test** Minisign publisher keypair. Sign a manifest binding the exact v2 file SHA-256/size/version, then reject another public key or an edited manifest. Verify the v2 file bytes match the manifest again. This does *not* substitute for the native `SignedCandidate` test, which is separately required by Application CI.
3. Under the actual Rust transaction module's OS lock, copy v1 into a disposable installation and atomically run its two-stage rename protocol to publish v2. The retained `previous.AppImage` must exactly match v1 and the active candidate must exactly match v2.
4. Launch the **real installed v2 GUI** under a deliberately wrong supervisor challenge (never a valid ACK). Wait for a visible Akula window, then use **SIGKILL on the isolated AppImage session** to terminate v2. Demand an empty ACK stream and log evidence of the Linux sender's wrong-supervisor refusal.
5. From another OS process, reopen the actual installation under the original transaction module's OS lock, execute cold rollback, restore the precise v1 byte hash, retain v2 at `failed.AppImage`, and prove the **restored v1 AppImage opens a real GUI and exits cleanly**.
6. Use a *second independent* temporary installation to publish v2 again; this time a real parent process creates an inherited Unix socket challenge. Verify v2's GUI first-frame ACK against process-ancestry/session identity, fresh nonce and SHA-256, and close the visible GUI. Require that the previous v1 backup remains preserved even after successful launch.
7. Verify the original Markdown vault and three Aquilum witness files retain their SHA-256 hashes throughout both scenarios. The precise completion marker is `AKULA_REAL_APPIMAGE_V1_V2_E2E_PASS real_packages=2 visible_gui=3 restored_v1=1 ack_v2=1 aquilum_unchanged=1`.

Each experiment fails closed on process exit, missing visible window, wrong ACK, timeout, file damage, missing backup or mutated Aquilum witnesses. CI must prove the marker was emitted after the real Docker execution; a skipped script or an exit code of zero alone cannot count as success.

## Remaining release blockers

This is a **CI-only supervisor**, not the production updater. Its detached signature uses an ephemeral key generated during the same test, and the helper bypasses the product's `SignedCandidate` constructor; the product-level test separately proves native signed bytes → locked stage → synthetic rollback. It does not attest the running executable's inode with pidfd, persist a monotonic version floor, handle power-loss durability, test package-manager upgrades, or secure real publisher signing keys. The shipped `apply()` still returns HOLD, `PUBLIC_KEY` remains empty, `channel_ready()` remains false, and both Aquilum data and public release remain protected.

**The next gate** is unifying this end-to-end OS supervisor with the native type-sealed signing/admission path and adding real powered-down restart and signed-vendor root checks. Do not declare production-ready from passing this lab.

Evidence source: [Akula PR #505](https://github.com/Lalalalendia/Akula/pull/505); [AKULA-MASTER-INDEX-01](https://app.notion.com/p/3fc32a84beec813e869fdf83a61e626b).
