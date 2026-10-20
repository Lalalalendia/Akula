# AK-DATA-CTRL-01C — Synthetic two-locks / anti-meme lab

**Status:** inert Rust experiment, not an application feature. No filesystem access, vault reads, deletion, network, or production signing keys.

Source: [AK-DATA-CTRL-01B](https://app.notion.com/p/3ff32a84beec815597a7e096ed532663), [two-locks](https://app.notion.com/p/3ff32a84beec8168bf21fd7e177e5e91).

The test uses three simulated DeviceIds, two ScopeIds, fixed test-only Ed25519 keys, owner + scope administrator signed grants, scoped bans, revision- and exact-content-specific targets, replay/equivocation checks, and in-memory return of the original signed anti-meme.

A policy decision only gates a synthetic publication. It cannot delete owner bytes. Object-level deny applies to modified revisions of the *same* DocumentId; exact-content deny applies to identical plaintext across a new DocumentId **in the same authorized scope**. A changed unrelated ID without verified provenance only produces a review signal if an explicit similarity hint exists; no similarity matching algorithm is claimed. Offline devices do not learn bans until synchronization.

Reproduce via `cargo test --locked -p aquilum-core --test scope_publication_policy_lab` from `aquilum-app`.

**Missing before production:** durable negative evidence across restart, actual scope membership and delegation, policy/key epochs, File Gate integration, freshness lease, real-device transfer, safe content extraction, appeal workflow, and adversarial replay/DoS. Do not ship this fixture as a moderation engine.


## Durable negative witness extension (additional tests, still only a lab)

The same integration test contains a synthetic SQLite witness for origin-signed policy decisions. It records one latest signed proof per (scope, role, subject), in a temporary SQLite database using WAL and FULL synchronous writes. An accepted decision is not visible in process until its SQL transaction commits.

Important separation:

- The signing keys and trusted administrative anchors are supplied independently; the SQLite DB cannot authorize its own replacement signer.
- On initial creation, an explicit synthetic Genesis pin is required. An existing DB refuses Genesis; absent DB refuses a non-Genesis pin.
- Every reopen requires a separately supplied **exact checkpoint**, computed from deterministic canonical signed proof bytes in sorted order. The checkpoint is NOT kept in the SQLite DB itself. A signed snapshot rolled back to earlier records is rejected when the caller still retains a newer independently pinned checkpoint.
- When storage fails after validation, the in-memory lab poisons publication decisions to Pending until a new externally anchored open. These tests DO NOT prove host crash-safety or atomic commitment across *two independent stores*. A real host must engineer a transactional witness and recovery protocol for the anchor, resist symlink/snapshot replacement, and never trust an unprotected checkpoint.
- Invalid signatures, altered signed proof bytes, wrong owner keys, equal-sequence equivocation, and old replay remain inadmissible. **No foreign file is deleted**.

New tests: signed deny persists through reopen and rejects stale scope grants; replacing SQLite DB with an older copy fails the newer external checkpoint; corrupted stored proof stops admission; injected SQL trigger failure does not expose uncommitted policy state; missing witness rejects an exact pin. These are controlled local, synthetic operations with no Akula user files.

The first tests and new witness tests run from aquilum-app with:

    cargo test --locked -p aquilum-core --test scope_publication_policy_lab -- --nocapture

**Remaining gaps:** no durable trust-root management for an installed machine, no TTL or policy refresh, no actual networking/File Gate integration, no OS-level power loss, no key rotation or ACL delegation, and no verified two-store crash transaction. Passing a test is not proof of those capabilities.
