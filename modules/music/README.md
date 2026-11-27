# AKULA-MUSIC-01

Standalone deterministic listening contract. No shipping UI, player backend,
Yandex access, credentials, network or database. No copied upstream source.
Yandex Music is the first target provider; the domain uses provider-scoped IDs.

The eventual backend must report actual cumulative rendered audio time for a
specific playback session. Seek positions, paused time and buffering are not
listening time. Finish consumes the session; a repeat gets a new ID. Identical
completed records deduplicate and conflicting IDs fail closed. The initial local
qualification threshold is 30 seconds or the full duration of a shorter track.
Natural end is a separate outcome, not proof of complete coverage.
Crossfade overlap is included in per-track audio time; distinct wall-clock
listening time, UTC timestamps, artist/album metadata, SQLite durability,
checkpoints and crash recovery are separate follow-up contracts.
The host must fence stale generation/account/session events before delivery.

Run: `cargo test --manifest-path modules/music/Cargo.toml --offline`.
The existing mandatory Application checks runner owns this crate. No separate
PR workflow, workspace dependency, network or live account test is introduced.
