# MUSIC-02b: SQLite storage candidate (not accepted)

The standalone branch adds SQLite history using the same direct rusqlite
0.41.1 release as Akula Core. The provider/account ID is an opaque local
identifier, never a secret. Per-session intervals represent confirmed audio
at normal speed, mapped by the host to UTC. The query clips to a half-open
range and merges overlap across tracks (crossfade). SQL schema is versioned.

This is not yet a runtime integration. The host must fence generation and
account events, guarantee unique session IDs, establish exclusive recovery
ownership, use monotonic audio timing and handle UTC clock discontinuities.
Do not infer time from seeks, wall-clock stalls, or unobserved audio.
Only an explicitly identified dead session may be finished as Interrupted.
Deleting history requires fencing producers and no active session; SQL
deletion is not secure erase of WAL, backups or storage media.

Run after resolving and reviewing the standalone Cargo.lock:
cargo test --manifest-path modules/music/Cargo.toml --locked

Current MUSIC-01 CI is intentionally unchanged on its own branch. This
candidate must not be admitted until MUSIC-01 is accepted, the exact
dependency lock is generated/reviewed and scanned, the existing required
CI route is adjusted for locked builds (without an offline cache assumption),
and all Rust tests, crash-process tests and clock/fencing witnesses pass.
The SQL experiment is not Rust acceptance.
