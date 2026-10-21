# YC-S3-T01d E3c — atomic signed-and-pinned genesis enrollment laboratory

**Research-only; not shipping.** Feature `signed-vault-atomic-join-lab` depends on E1/E2a/E2b/E3a/E3b. Standard installed Akula does not enable it. No secret keys, Yandex resources, users' Markdown, auto Sync or writer capability are created.

## Why

The pre-existing `S3V2TrustedAnchorJournal::enroll_verified_genesis` and E2b `enroll_externally_approved_genesis_policy` write the root and policy in **two separate transactions**. A crash in between can leave a root-only row, requiring separate recovery handling. E3c is a narrowly scoped experiment to write the *verified matching root and signed owner policy together*, in **one BEGIN IMMEDIATE SQLite transaction** in the same existing `s3_v2_trusted_roots` table.

## Explicit admission protocol

Caller must separately establish ALL of these before attempting enrollment:

- A human-reviewed, independently authenticated out-of-S3 channel carrying exact gen-1 `VaultId`, Yandex bucket/namespace, genesis root BLAKE3, genesis policy BLAKE3, authorized Ed25519 owner keys/quorum and a fresh receiver challenge.
- An E3a owner-quorum DSSE signature over exact approved genesis identity, epoch=1 policy digest and recipient challenge.
- An E3b **fully pinned bounded S3 V2 receipt** for the exact same root, ETag and protected `S3RemoteAnchorScope`. The existing reader validates complete manifest/index/packs and re-fetches root to detect an unexpected change. ETag is a concurrency token, never evidence of owner identity.
- A registered protected local workspace and private app-data-only SQLite journal; the local host must separately guarantee the preceding approval came from a trustworthy channel, not S3.
- The production binding of an S3 account to a principal is not proven merely by an app-supplied scope string.

Only then `S3V2TrustedAnchorJournal::enroll_signed_pinned_genesis_atomically`:

1. Rechecks E3a signed owner invitation, independently approved pins, full transport provider/bucket/Vault identity and complete E3b pinned read.
2. Canonicalizes the approved `PublisherPolicyV1` bytes using the *existing E2b lab-only serialization*, checks the exact digest against the signed and independently approved value.
3. Opens **one SQLite BEGIN IMMEDIATE transaction**, refuses foreign scope, conflicting root/ETag, different policy or a legacy root-only entry. Same root and policy with same ETag is an idempotent no-op.
4. Writes the root JSON+ETag+receipt and policy JSON+digest+receipt **in the very same row and transaction**. Only after the insert does `COMMIT` occur.
5. Returns typed `AtomicJoinOutcome::{Inserted,Unchanged}`, which is a **private trust receipt only**, not cloud/local file-write authority.

No second database, no new policy table, no source of truth in user Markdown. Existing E2b root load validates all policy columns; malformed/partial rows reject old and new APIs.

## Tests

Under `--features signed-vault-atomic-join-lab`:

- Authorized exact root and policy are inserted together and survive restart, preserving one SQLite row.
- Same signed/pinned invitation is idempotent; no additional generation is created.
- Forged owner signature cannot create half a join.
- Missing matching ETag/full pinned receipt cannot create a root-only policy entry.
- Wrong independently approved policy digest cannot create any row.
- Existing root-only admission cannot silently become a signed join.
- Two concurrent handles serialize through SQLite; second matching offer observes `Unchanged`, conflicting root+policy cannot overwrite the first one.
- Once owner/publisher policy is enrolled, the **legacy root-only** `record_verified_successor` refuses any distinct V2 generation even with a complete pinned read: `UnverifiedRead`. A signed publisher-authorized successor path is not yet implemented and must remain HOLD. Root-only research journals without approved publisher policy retain their previous behavior.
- E2b original 12 tests and E3b/E3a/E2a regression run in same feature matrix on Windows and Linux.

The tests use **synthetic Ed25519 seeds, a test-only minted pinned V2 receipt and disposable app-data**. They do not contact real Yandex Object Storage. Exact-head CI must pass before any claim of automated acceptance.

## Limits / production hold

- Cryptographic trust-source provenance remains a **caller precondition**, not an actual human-pairing UI; E3a cannot validate that S3 metadata was not used as the alleged independent key and pin.
- No real receiver CSPRNG challenge generation, session expiry or one-time consumption ledger; invitation replay between independent sessions needs separate host logic.
- No independently witnessed anti-rollback after restoring a complete malicious local SQLite snapshot.
- Canonical JSON policy wire format is still lab-specific and not stable across languages.
- This method **does not create S3 authorization or File Gate permissions**, publish a new root, upload/download notes into the user vault, promote `ConfirmedBaseline`, acknowledge Sync, or enable any automatic background work.
- Actual Windows CredUI and real private Yandex read-only QA remain separate blocked gates.

Parent PR stack: [E1 #472](https://github.com/Lalalalendia/Akula/pull/472) → [E2a #475](https://github.com/Lalalalendia/Akula/pull/475) → [E2b #481](https://github.com/Lalalalendia/Akula/pull/481) → [E3a #487](https://github.com/Lalalalendia/Akula/pull/487) → [E3b #491](https://github.com/Lalalalendia/Akula/pull/491).

[Notion technical RFC](https://app.notion.com/p/3ff32a84beec813ebfdff9b96bf103d1)
