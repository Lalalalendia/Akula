# AK-DATA-CTRL-01C — Synthetic two-locks / anti-meme lab

**Status:** inert Rust experiment, not an application feature. No filesystem access, vault reads, deletion, network, or production signing keys.

Source: [AK-DATA-CTRL-01B](https://app.notion.com/p/3ff32a84beec815597a7e096ed532663), [two-locks](https://app.notion.com/p/3ff32a84beec8168bf21fd7e177e5e91).

The test uses three simulated DeviceIds, two ScopeIds, fixed test-only Ed25519 keys, owner + scope administrator signed grants, scoped bans, revision- and exact-content-specific targets, replay/equivocation checks, and in-memory return of the original signed anti-meme.

A policy decision only gates a synthetic publication. It cannot delete owner bytes. Object-level deny applies to modified revisions of the *same* DocumentId; exact-content deny applies to identical plaintext across a new DocumentId **in the same authorized scope**. A changed unrelated ID without verified provenance only produces a review signal if an explicit similarity hint exists; no similarity matching algorithm is claimed. Offline devices do not learn bans until synchronization.

Reproduce via `cargo test --locked -p aquilum-core --test scope_publication_policy_lab` from `aquilum-app`.

**Missing before production:** durable negative evidence across restart, actual scope membership and delegation, policy/key epochs, File Gate integration, freshness lease, real-device transfer, safe content extraction, appeal workflow, and adversarial replay/DoS. Do not ship this fixture as a moderation engine.
