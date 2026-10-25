# NODE-V1-WOW-MAIL-01 — isolated, default-off mailbox bridge

Status: **laboratory-only / HOLD**. This is not a deployable Node role.

Source: [WOW #95](https://github.com/calex794/wow/pull/95); pinned Git
revision `29bd044cf11c60687716560c15abef69f5581ebc` (W12, signed
one-slot invitation, private List/Fetch, signed HPKE W4 return, all
baseline CI green on predecessor `f7cf1357`). This new pin includes
the actual B LIST/FETCH E2E test and `rusqlite = 0.40.1` required by the
Akula Core SQLite ABI. W12 regression is confirmed green on the new pin;
full Rust/QUIC results must be confirmed before promotion. No floating
branch or mutable `main` dependency is permitted.

## What is implemented

The independent `interop/akula-staya` package has the optional feature
`node-wow-mailbox-lab`. It is **not enabled by default**, it does not
modify the installed `akula-node-v1-service` binary, its two CLI modes,
Iroh ALPN, root policy v3, public port or signed installer.

The bridge checks the current existing Akula Node-V1 root-pinned Iroh
EndpointId before opening an existing pre-provisioned WOW mailbox, and
again before **every** authorized local request. Its custodial Ed25519
key must independently match a pinned lab TrustRegistry key+epoch.

It rejects a missing, symlinked, hardlinked, publicly readable, wrongly
owned or replaced SQLite mailbox. The mailbox directory must be private
0700 and the pre-existing `wow-mailbox.sqlite` private 0600, with no
implicit mailbox or identity creation. The W3/W4 journal is separate
from the Akula NodeStore. Only the already hardened **127.0.0.1**
WOW STORE/LIST/FETCH/return-message functions are available; the Node
File Gate is not callable from this bridge.

The isolated test uses an actual NodeHost fixture under root-policy v3
(with independently pinned endpoint identity) alongside a different
WOW SQLite and signed sender/recipient/custodian keys. It verifies
private recipient discovery/fetch, state after bridge restart, refusal
when Node policy changes, refusal of public bind and unsafe SQLite,
and refusal of a mismatched custodian key.

Test workflow: [node-wow-mailbox-lab.yml](../../../.github/workflows/node-wow-mailbox-lab.yml)
(actual path from repo root is `.github/workflows/node-wow-mailbox-lab.yml`).
Run: `cargo test --manifest-path interop/akula-staya/Cargo.toml
--features node-wow-mailbox-lab --test node_wow_mailbox_lab`.

## Critical authority gap — never bypass

**Node-V1 root owner-policy v3 grants only the existing file replica
Put/Get.** Checking its EndpointId is a transport-identity prerequisite,
**not mail-role authorization**. The WOW TrustRegistry currently comes
from explicitly provided independent lab fixtures; it is NOT loaded
from a production root-protected owner policy, and a nonroot process
could otherwise forge that registry.

To permit any actual Node/WOW service activation, a *separate*
independently approved owner-signed mailbox role policy must bind:
- exact Node-V1 EndpointId **and** Node receipt identity;
- WOW custodian signing key, policy epoch, enrolled owner/recipient/
  permitted sender keys, private mailbox tags and separate STORE/FETCH/
  RETURN scopes;
- quota, TTL, storage path, reversible/revoked epochs and health
  obligations, anchored to immutable root-owned configuration;
- installed signed Node ELF, root witness/CAS, service UID and
  explicit operator consent.

The new mail policy must be read from pinned root-owned paths and checked
for freshness at the same durable SQLite transaction boundaries. The
existing `RootTrust v3` cannot be silently interpreted as this policy.
Neither an arbitrary `TrustRegistry` nor the public W3 route token is
mailbox authorization.

## Still not accepted

No Akula installer activation, no server lifecycle on the user's VPS,
no network ALPN for WOW, no persistent production clock or root mail
policy, no OS-attested mail key enrollment, no two-physical-host E2E,
no B->C->A delivery receipt in the installed client, no Windows UX.
No user data is sent or VPS changed. Keep the bridge and WOW #95 Draft
until these separate production gates pass.
