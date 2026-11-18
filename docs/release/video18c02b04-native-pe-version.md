# VIDEO-18c-02b-04 — native Windows executable version identity (non-release)

**Purpose:** make it impossible to confuse a signed laboratory manifest label with the version of the actual Akula executable. **VIDEO-18 RELEASE HOLD** unchanged.

Previously accepted #1010/#1013/#1018 proved independently built SHA-distinct native Akula, real signed five-PE generations, actual GUI first-frame ACK, two complete rar2 rollbacks, and genuine successful rar2 confirm. However both native Cargo package sources declare **0.3.20**, while the ephemeral signed test generation says **0.3.21**. This is **not a version upgrade**.

This stage embeds `VS_FIXEDFILEINFO` (Windows `RT_VERSION`) from the **native Cargo package `CARGO_PKG_VERSION`** into the linked actual `aquilum-native.exe` alongside its existing icon resource. No version is supplied by environment at verification time, and the original native Rust package version is not changed by this PR.

A separate Windows-only, read-only Rust witness calls **`GetFileVersionInfoW` / `VerQueryValueW`** on the exact `.artifacts/cargo-native/release/aquilum-native.exe` produced by an exact-HEAD `cargo build --release --locked`. It verifies the canonical version from `cargo metadata --locked --no-deps` against the native PE and additionally enforces `PE == source == signed` and `PE > installed` for admission. Malformed, forged, absent or mismatched version resources fail closed. Hosted PR only, no publisher signing, no updater activation.

**This first acceptance deliberately expects DENIAL** for the current `0.3.20` executable being falsely promoted as signed `0.3.21`. It proves the release gate refuses phantom upgrades. Separate future work will require a tracked, genuinely newer native version, rebuilt PE with the matching resource, signed owner manifest and successful protected transition with those same bytes. Do not label this first stage release-ready.

The native RT_VERSION resource has no owner signature by itself; release trust still needs a publisher-signed manifest pinned to the exact executable SHA and Windows trust policy. Do not use ephemeral Minisign keys from the labs as publisher credentials. No binary or installer publication; `VIDEO_FIVE_CHANNEL_READY=false`.
