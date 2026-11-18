# VIDEO-18c-02b-06 — genuine source-version transition through signed-five rar2 (non-release)

**VIDEO-18 RELEASE HOLD remains in force.** This is an isolated, disposable, GitHub-hosted Windows pull-request lab. No production updater, deployment, publisher keys, published installer, or user installation is authorized.

## Why this stage exists

The earlier #1018 acceptance signed and activated two genuinely different five-PE generations, but both native Cargo package versions were `0.3.20`. Its second signed manifest was labelled `0.3.21` only as an ephemeral lab generation. The accepted #1025 now raises the **tracked** native/core manifests and lock package records to `0.3.21`, and embeds the actual Windows PE version directly from Cargo.

The old stage mistakenly required both generations' `Cargo.toml` and `Cargo.lock` to be identical. Keeping that rule would reject a real upgrade and encourage a forged version label.

## New exact-head Windows acceptance

1. Compile the historical native Akula `0.3.20` from pinned commit `ec6d6d48a78e60fca7a6f810b825418500bad64b` and current native Akula `0.3.21` from exact PR HEAD independently with `cargo build --release --locked`.
2. Verify each checkout's tracked `Cargo.toml` and `Cargo.lock` against **its own** Git blob; demand separate source/lock blobs and correct native/core version records, rather than incorrectly requiring identical files.
3. Before any ephemeral signing, use `cargo metadata --locked` and the independent Windows `GetFileVersionInfoW/VerQueryValueW` reader on the actual newly built PE. Demand `new source == new PE == signed candidate 0.3.21` and `0.3.21 > old 0.3.20`. Reject three forged or nonincreasing variants.
4. In the protected NSIS lab, construct five genuine PE files for each version. Sign each generation's SHA-256 manifest using a **different historical/current Cargo.lock hash**, the correct source commit and the matching version. Check both ephemeral Minisign signatures and reject cross-generation signature substitution.
5. Run the existing *real* rar2 OS `InstallLock` candidate activation; require physical first-frame GUI ACK bound to the signed five-component digest. In bad-nonce and bad-generation cases, roll back five files to the exact historical set. Only after positive ACK call `confirm_candidate()`.
6. Require complete uninstall and byte-preserved independent note. Upload only the existing non-executable custody report; do not publish a binary.

## Failure and trust boundary

The old native source predates the embedded Windows RT_VERSION resource; its version is attested from the pinned native Cargo source/lock and isolated source build, **not** by inventing a PE-version resource on that historical executable. The new native executable has a separately verified real PE resource.

**Ephemeral Minisign is not publisher signing or an owner-trusted key.** The test key is generated within the runner and does not establish a trusted release channel. This acceptance does **not** prove production authorization, trusted public manifest distribution, separate-process crash/reboot recovery, security of a production update service, or release readiness. Those remain separate release blockers.

Markers: `VIDEO18C02B06_REAL_NATIVE_FLOOR_PASS`, `VIDEO18C02B06_SIGNED_REAL_VERSION_PAIR_PASS`, and the existing `VIDEO18C02B03_RAR2_REAL_POSITIVE_PASS`. A green Windows CI is required before the stage is accepted.

**VIDEO-18 RELEASE HOLD and VIDEO_FIVE_CHANNEL_READY=false stay unchanged.**
