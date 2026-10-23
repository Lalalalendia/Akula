# AK-DATA-CTRL-01D: real Linux Node read gate with scoped signed anti-memes

Status: developer laboratory. No released runtime, no default behavior changes.

Source: AK-DATA-CTRL-01B (https://app.notion.com/p/3ff32a84beec815597a7e096ed532663); two-locks overview (https://app.notion.com/p/3ff32a84beec8168bf21fd7e177e5e91); STAYA (https://app.notion.com/p/3fe32a84beec8133acd0e76dc9eeffd0).

## Reuses existing Core

The existing ReadCatalog, with a Linux pinned root fd and exact top-level Markdown allowlist, is the only source of document reads. An opt-in additional lab binary, akula-node-policy-lab, binds only to 127.0.0.1, takes a fixed appdata directory disjoint from the vault, and verifies a 32-byte lab bearer on every request. The only allowed peer operations are READ, PROOF and BAN. The protocol is short, length-bounded, one request per connection, without arbitrary paths, URL fetching or executing anything.

One scope administrator's pinned Ed25519 public key verifies a signed anti-meme for a precise document ID, VaultId and monotone sequence. An accepted BAN suppresses later reads in that scope and saves the original signed proof to an appdata-only SQLite table. Another authenticated peer can retrieve that exact proof and pass it on; the forwarding peer has no signing or file-delete authority. Stale lower-sequence signed proofs receive the newer original proof as a correction. An unrecognized document ID is not admitted to the revocation store.

## Actual acceptance coverage

Three real Linux subprocesses expose the same scope/file via separate temporary vaults. They all initially read the allowed fixture; the owner/scope admin signs an anti-meme for A; B and C remain outdated until the coordinator forwards that same proof A -> B -> C over real loopback TCP sockets. All cease serving the scoped read; after C is killed and restarted, SQLite still suppresses its read. A fourth process representing another VaultId rejects an otherwise authentic but wrong-scope anti-meme. The test verifies every original file remains byte-for-byte unchanged.

Additional tests validate rejection of outside signatures, wrong scope, wrong bearer, duplicate and stale messages, and refusal to bind when a persisted anti-meme signature is corrupted. A real persisted proof is reconstructed from SQLite and verified against a pinned scope key BEFORE the listener binds.

Run in aquilum-app:

    cargo test --locked -p aquilum-core --features node-scope-revocation-lab --test node_scope_revocation_lab -- --nocapture

## Explicit limits

This is only the administrator denial half of the two-lock idea. The owner grant, dual approval, delegated role rotation, reviews, content similarity, scope freshness leases and File Gate mutation admission are NOT implemented. The trusted operator's read catalog and bearer are required for existing read permission, not a general cryptographic owner grant. A recipient cannot make an offline node instantly hear a revocation.

The network here is real loopback TCP with a simple fixed test wire protocol, NOT Roha/noq, WAN, BLE or STAYA auto-discovery. The test coordinator explicitly forwards signed proofs; the node does not discover or broadcast neighbors.

Critical: SQLite state is verified for signature validity but there is no externally pinned high-watermark or independent anti-rollback witness. A malicious host restoring a fully valid older SQLite snapshot could resurrect the read. Such a witness is being modeled in separate Draft PR #492. The two pieces must not be represented as a production-capable unified protocol before an independent custody and crash-recovery design is tested.

No original user files may be changed by this experiment. The shipping D2 node-lab read-only code and File Gate writer stay unchanged; this is a completely feature-gated alternative lab target.

## Central-file CI ratchet compatibility

Core `lib.rs` remains unchanged: this experiment imports its policy implementation through an explicit `#[path]` from only the opt-in binary and integration tests. The manifest now builds on Core's extracted `akula-node-catalog` feature in current main. No extra default Core dependency, module registration or CI fanout is added to the common `lib.rs` entrypoint. Avoid treating this test-only import as a public runtime API.
