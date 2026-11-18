# VIDEO-18c-02b-05 — real native candidate version 0.3.21 (non-release)

After accepted #1021, the tracked native/core Cargo.toml, both Cargo.lock package records, and Windows packager version change consistently from **0.3.20** to **0.3.21**. The repository's established scripts/akula-version.cjs contract enforces those five mirror fields; the Linux packager remains separately held. This is a real source version change; the existing Windows build.rs places it into RT_VERSION of the actual built aquilum-native.exe.

The exact-head hosted Windows PR job builds genuine native Akula with cargo build --release --locked. It verifies Cargo metadata, embedded PE version and a monotonic floor over installed baseline 0.3.20. It expects the legitimate version claim 0.3.21 to succeed, and three forged/nonincreasing claims to fail closed.

**Limitations:** This is a version *identity* test, not proof of a real installed cross-version transition. The candidate-version argument is only an unauthenticated lab claim: it is **not owner signing**, not a verified publisher manifest, and not a production updater. Previous #1018 authenticated an ephemeral Minisign lab generation, not production release signing. Repeating its real rar2 ACK→confirm flow using a new genuinely-versioned installed five-PE set, enforcing publisher trust, and testing crash/reboot recovery remain required.

**VIDEO-18 RELEASE HOLD** and VIDEO_FIVE_CHANNEL_READY=false. No users, installers, deployments, owner keys or releases are touched.
