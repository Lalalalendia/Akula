# YC-S3-T01d E3e — receiver CSPRNG one-shot Vault join session

**Research-only / opt-in, not an installed Akula feature.** `signed-vault-receiver-session-lab` depends on E3c. No S3 PUT/CAS/DELETE, File Gate, Sync, note writes, key custody or new database/table.

## E3a/E3c limitation addressed in this experiment

In E3a, `IndependentlyApprovedJoin.recipient_challenge_hex` is externally supplied. E3a validates its format and inclusion in the owner's signature but does not guarantee an unpredictable freshly issued challenge or one-time consumption. E3c enrolls an approved signed root+policy atomically but does not itself mint/consume a receiver nonce.

E3e introduces a host-scoped ephemeral `ReceiverJoinSession`:

1. Only the *receiving device* calls `ReceiverJoinSession::issue(scope)`. It requests **32 bytes from `ring::rand::SystemRandom`** (OS CSPRNG), rejects an all-zero result and exposes **64 lowercase hex** for the authenticated pairing channel.
2. The session holds the exact local profile ID, scope binding fingerprint and canonical current-head key. A changed bucket, account binding, VaultId, keyspace or profile cannot reuse that challenge.
3. A monotonic `Instant` gives the session a fixed **five-minute** lifetime. No wall-clock timestamp is trusted, and a restart discards the pending session entirely.
4. `enroll_once(&mut session, ...)` spends its state **before** checking the signed offer or any database operation. A malformed, forged, expired or foreign-scope attempt does not leave a reusable challenge. Safe Rust does not clone this session.
5. On the single allowed attempt, the wrapper delegates to the *existing* E3c signed owner quorum and independent genesis-root/policy BLAKE3 pins, E3b complete V2 pinned read, and atomic SQLite root+policy insert.

## Linux/Windows laboratory acceptance

- CSPRNG produces nonzero 32-byte challenges, lowercase hex and distinct independent sessions.
- One valid signed invitation with a complete synthetic pinned read inserts exact root+policy once; replay through the same session fails.
- A forged Ed25519 owner signature spends the session and cannot produce a partial SQLite root/policy.
- An expired session, changed independently approved challenge or foreign local scope fails without changing the V2 journal.
- Existing E3c/E2b/E3a tests and default Core compile remain intact.
- Exact new tests must appear in `cargo test -- --list` before the tests run; this avoids false-green `--exact` zero-test CI.

## Security limits — do not misstate as production pairing

- **This does not establish that owner keys or pins came from an independently authenticated, human-approved channel.** The caller must provide and authenticate them outside S3. No UI, camera/QR exchange or key custody is implemented.
- **The existing E3c lab method remains available** to its research callers without the new session wrapper. Production must expose *only* a reviewed host-controlled session admission path before claiming mandatory anti-replay.
- One-shot means *within this process-local session*. There is no persistent challenge/replay ledger; a process restart always requires a newly generated challenge. Whole-DB rollback and host compromise remain out of scope.
- Live Yandex S3 HEAD/GET/ETag and Windows CredUI acceptance are not performed by these tests. The synthetic V2 pinned proof is not a live provider observation.
- No installed Akula build enables the feature; no S3 writer, File Gate, Sync scheduler, or user Markdown mutations.

Source: [Notion YC-S3-T01d RFC](https://app.notion.com/p/3ff32a84beec813ebfdff9b96bf103d1). Dependent Draft: [E3c PR #493](https://github.com/Lalalalendia/Akula/pull/493).
