# VIDEO-18b-12 — refuse legacy Windows NSIS updates in the five-EXE video experiment

**Status: NON-RELEASE / updater transaction incomplete.**

## Why this is necessary

The existing **shipping two-EXE** Akula Windows updater accepts a publisher-signed `SignedCandidate`, verifies its bytes, stages a temporary NSIS setup, and executes `/S /R /UPDATE /ARGS`. It is a real beta updater, but it has **no durable five-component generation journal, first-GUI-frame acknowledgement or automatic crash rollback**.

Our VIDEO-16–18 lab builds have `--features installed,video-rust-bundle,video-ui-e2e`. Previously, that combination inherited `channel_ready() == true`, meaning an experimental video-enabled build could discover and hand off the regular two-EXE signed installer — an inconsistent and potentially destructive change of component set.

## Protected boundary

- `updater/release.rs::channel_ready()` now accepts only `cfg!(all(target_os="windows", feature="installed", not(feature="video-rust-bundle")))`.
- This intentionally leaves the real ordinary **two-EXE signed Windows** update unchanged, while the five-EXE opt-in lab has no update discovery, no UI offer, and cannot reach `updater/apply.rs` (which checks `channel_ready` before writing or invoking setup).
- Explicit native unit tests assert no video-enabled update lookup or install action; release-HOLD Node tests assert the source guard, its order before side effects, and negative mutations; CI source triggers cover both updater files and tests.
- The signed publisher workflow, public two-EXE NSIS manifest, Linux/macOS updates and experimental video download/playback code remain unchanged.

## What this does not solve

- It is **not** a transactional protected Windows video updater. It only makes current video experiments safe from crossing into the legacy update path.
- The next stage must use the existing signed-candidate authority and a separately reviewed protected machine-wide installation identity: OS lock, exact five-EXE source/sha list, an immutable previous generation, durable write-ahead receipt and power-loss state recovery, bounded generation-bound first rendered frame ACK, rollback on failures, and no version downgrade.
- Real protected installed GUI and user-data persistence tests are independent release gates. No public five-EXE installer, no package signing, no production migration.

## Acceptance

- [ ] Exact-head Node release HOLD and negative tests SUCCESS.
- [ ] Application checks Rust tests and source policy SUCCESS.
- [ ] Verify the normal two-EXE signed-update channel remains available only on Windows installed builds without video features.
- [ ] Keep protected five-EXE VIDEO-18 release HOLD until separate update/rollback proof accepted.

**Nothing here publishes binaries, signs installers or alters end-user machines.**
