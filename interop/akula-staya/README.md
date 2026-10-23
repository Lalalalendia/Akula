# AK-P2P-REAL-STAYA-01 — executable Akula-to-Staya interop

**Status: isolated opt-in integration, not a user-facing production transfer.**

This directory contains a **real compiled adapter** between Akula Core
`OfferV1 / DecisionV1 / OfferSession` and the separately maintained
`staya-transfer::queue::QueueSelection`. It intentionally does not mirror or
duplicate Staya's queue implementation. The dependency is pinned to Staya's
immutable commit `ccfe82dc6125e2810e95043a6a4ac55954b710e0`.

The CI workflow checks out this exact Staya revision, builds its existing
`staya-direct-preview` executable with its locked release compatibility
manifest, then runs an actual Iroh QUIC transfer between a sender and receiver
in **two independent operating-system processes**, using a real Akula
receiver-approved subset and a Staya queue commitment.

## What the roundtrip must prove

1. Create three **disposable synthetic files**, with UTF-8 Akula display labels.
2. Initialize a real Staya sender and receiver, and parse their actual Iroh
   EndpointIds from the receiver's public-key announcement and sender's
   serialized endpoint ticket.
3. Create a valid Akula Offer, then receive explicit consent for files #0 and
   #2. A forged commitment must fail before any receiver GET is started.
4. Materialize the exact approved roots and sizes into the **actual Staya**
   `QueueSelection` using restricted ASCII transport aliases
   (`object_00.bin`, `object_02.bin`).
5. Call the reusable `verify_staya_invitation` bridge: decode the real
   serialized Iroh endpoint ticket and compare expected endpoint, every root,
   size, order, alias, and Staya commitment against the approved queue.
   Mutated tickets and unselected root #1 fail **before GET**.
6. An outsider receiver with a different real QUIC key must be refused.
7. Send the invitation only after Akula consent. Receive actual Bao ranges,
   then independently read the private staged files, checking byte identity,
   exact BLAKE3 and queue progress. No Akula vault is opened.
8. Do not claim `File Gate`, `APPLIED` or a durable ACK. All CI steps must
   run on the exact head. Pinned Staya builds on Linux and Windows runners.

## Not a production authorization

This demo necessarily trusts a **local test operator** to supply two temporary
Akula DeviceIds and identify the initial Staya endpoint ticket. The gated
provider now denies real QUIC GET before the controller explicitly arms the
sender after Akula consent, but the test controller is trusted local input.
This **does not** establish a production entitlement against hostile local clients.

The next product step is a real Broker-enrolled
`Akula DeviceId <-> authenticated Iroh EndpointId` mapping and durable
single-root, time-bounded policy grant checked by the Staya provider before
every new GET. On-disk revocation and crash-stable replay checks remain
required. User file source descriptors require a race-free open and pinned
root, and publication requires Akula File Gate. None of these security
conditions can be inferred from this end-to-end test.

### Limitations and build rules

- This package is outside the shipping `aquilum-app` workspace; feature
  `p2p-offer-lab` stays disabled in released Akula binaries.
- No modification of the shipping app's `Cargo.lock`, UI, File Gate,
  network permissions or installed updater.
- The standalone bridge manifest currently resolves its own lockfile in
  CI. **Do not release** before pinning and reviewing this resolved lockfile.
- For a local run, build the exact Staya pinned
  `interop/release/Cargo.toml` binary and set `STAYA_PREVIEW_BIN`.
  The workflow is the primary supported Linux/Windows reproduction.

## Provider-side Akula consent gate (STY-G1n)

The interop test now runs real Staya `serve-batch-gated`, not an already
permitted `serve-batch`. The original receiver EndpointId must first fail a
real QUIC connection/GET. The test controller validates Akula OfferSession
consent and the actual Staya ticket/selected-root commitment, then submits a
sender-local `AUTHORIZE <queue-commitment> <receiver-EndpointId>` command.
Only after Staya confirms `PROVIDER_ARMED` may the same receiver complete
Bao staging. An unrelated real peer is denied **after** arming. Provider
permission expires after 180 seconds and closes on sender EOF.

This is a **synthetic two-process integration test**, with a trusted local
controller and ephemeral device identities. It is not a real Akula installed
UI, trusted device enrollment, remote-signed user decision, authenticated
host IPC, File Gate publish permission, or durable application receipt.
Do not enable shipping P2P from this result.

## Receiver create-new inbox (Akula P2P INBOX-01)

After an actual Staya QUIC/Bao transfer produces a private staged file,
`aquilum_core::p2p_inbox_lab::save_verified_stage_as_new` accepts its
**already opened File handle** plus the root/length from independently
approved Akula OfferSession. It copies through a 64 KiB buffer into a fresh
sibling candidate, cold checks source size and BLAKE3, flushes the candidate
and uses a **same-directory no-clobber hard link** to publish the
user-selected NEW destination. Existing files/symlinks, changed stage sizes,
wrong roots, and unsupported hard-link filesystems fail closed; no rename
fallback. A wrong digest never makes partial destination bytes visible.

This is now exercised with the actual Staya staged bytes in the Windows/Linux
two-process E2E and also through portable Core unit tests. The verified stage
remains separate from the user destination; a second attempted save to that
destination is rejected without mutation.

**Important limits:** this is a Core opt-in feature / test path, not yet a
button in installed Akula; neither the receiver's selected destination nor
the sender's QUIC identity is currently driven by the real File transfers
window. The API cannot prove the provenance of caller-supplied expected
BLAKE3 roots; the caller MUST use an authenticated accepted OfferSession.
It does not protect all ancestor path races or guarantee directory metadata
durability after power loss. A successful copy is not File Gate APPLIED,
Vault synchronization, or a durable revision ACK.

## AK-P2P-LIVE-ID-01 — live Iroh key and fresh protected peer pin (opt-in)

The standalone `p2p-live-identity` feature derives the actual Iroh
`SecretKey` from Akula's existing OS-backed `DeviceIdentity`, checking
the resulting public ID. It exposes `require_pinned_live_peer(connection,
selected_id)`: this reads cryptographically authenticated Iroh
`Connection::remote_id()` and delegates to Core's OS-persisted
`peer_pin::require_pinned_remote` on **every new operation**. No claimed
network ID or invitation-derived key is accepted as proof.

Linux/Windows tests establish a genuine loopback QUIC connection and prove
wrong selected IDs and a fresh unconfirmed peer fail closed. Test code does
not initialize, replace, delete, or leak any user's canonical credentials.

**Not installed:** neither native File transfers nor the Staya provider calls
this seam yet. No DeviceId/Vault/object authorization, sender consent, local
IPC control, user-file GET, File Gate apply, durable ACK or two-PC acceptance.
The existing receiver create-new and OS identity code stay opt-in. This work
cannot enable network file access.

## AK-P2P-SENDER-ARM-LAB-01 — bounded local consent handoff (default off)

The opt-in `sender-arm-lab` feature adds `LabSenderFence`: the parent test
controller must revalidate the **same** Akula OfferV1 / DecisionV1, exact Staya
queue contents and actual sender EndpointAddr ticket before it is permitted to
emit the existing provider's `AUTHORIZE <queue-commitment> <receiver-ID>`
line. An altered queue, wrong receiver, expired offer, second issuance or
repeat consumption denies without arming. A maximum of 128 local issuances
prevents indefinite process-memory growth. This code is covered by unit
tests and the **same** existing Linux/Windows real two-process QUIC/Bao test.

**Security boundary:** this is a test-controller fence, *NOT* trusted Core
authorization. The Staya preview still uses disposable private keys and
its private parent-owned stdin channel, not production host IPC. Its current
caller-supplied DeviceIds and timestamps have no independent OS/Vault proof.
This per-process fence is NOT durable, cross-process replay-proof, or a peer
revocation witness. No File Gate/Vault grant, shipping feature, new network
library, long-running server, or background Sync is enabled. A future host
integration MUST source real device identity from OS Core, bind Vault/file
rights and current epoch to the selected root, and provide authenticated
local process control before any real user-file GET.

## OS-pinned two-Windows-PC direct QUIC — manual bounded LAN probe

The opt-in executable `akula-p2p-lan` uses the **same** #625
`with_protected_iroh_secret` and `require_pinned_live_peer` APIs.
It never creates credentials implicitly; it never prints secrets, reads
user files, offers a Staya root, issues a Vault/File Gate grant or starts Sync.

On EACH Windows computer, run the **existing** `akula-p2p-identity`
operator utility explicitly (`--init` only once if missing, then `--id`).
Independently compare the 64-character public IDs out of band.
On both computers, confirm the OTHER complete ID with
`--peer-confirm OTHER_ENDPOINT_ID` and type the same independently verified
ID when prompted. Once both pins are confirmed, start the listener:

```powershell
cargo run --manifest-path interop/akula-staya/Cargo.toml --features os-pinned-lan-probe --bin akula-p2p-lan -- listen OTHER_ENDPOINT_ID
```

Use the actual LAN IPv4 of the listener and its printed UDP port:

```powershell
cargo run --manifest-path interop/akula-staya/Cargo.toml --features os-pinned-lan-probe --bin akula-p2p-lan -- connect OTHER_ENDPOINT_ID 192.168.1.10:UDP_PORT
```

The listener rejects an unconfirmed, wrong or newly revoked live TLS peer,
and the initiator verifies the authenticated remote key too. Only bounded
`AKULA_HELLO_V1` and `AKULA_OK_V1` are sent. Routing is **direct LAN
only**, without a relay, discovery, NAT traversal, UPnP or automatic firewall
configuration. An IP/UDP port from another source is a route hint, never a
trust grant. This CLI is not shipped with Akula. Profile CI **only compiles**
this Windows-oriented tool on Linux/Windows; no physical two-PC acceptance
can be claimed from CI. Linux refuses missing protected OS credentials at
runtime.

## Two Windows PCs on DIFFERENT internet connections (explicit opt-in)

The **same** default-off `akula-p2p-lan` operator utility also supports
`internet-listen` and `internet-connect`. This is NOT an installed Akula
file-transfer feature. It only proves that the actual OS-persisted private
Iroh key and the independently confirmed peer pin work over a real internet
QUIC session.

These subcommands **explicitly opt into external n0-operated DNS/Pkarr
discovery and encrypted relay fallback**. An n0 relay may observe addresses,
connection timing and packet sizes; it cannot decrypt the QUIC content.
Iroh first tries a direct UDP path (including hole punching) and, if that
fails, can continue over the encrypted relay. We do not silently enable this
on ordinary LAN modes, import user files, or configure routers/firewalls.
Availability depends on n0 infrastructure and local outbound access.

You do NOT need two devices on the same LAN. To begin, download the
**test-only, unsigned Windows diagnostic bundle** from the successful
`Akula P2P to real Staya QUIC two-process interoperability` GitHub Actions
run for your exact selected PR commit. The Windows artifact includes
`akula-p2p-identity.exe`, `akula-p2p-lan.exe`,
`SHA256SUMS.txt`, this README and an explicit NOT-RELEASE note.
This is not the signed Akula installer. Only use the artifact from our
own GitHub repository, check the SHA-256 hashes and isolate test accounts.

On Windows computer A and computer B, each in its own terminal:

```powershell
.\akula-p2p-identity.exe --init
.\akula-p2p-identity.exe --id
```

Only run `--init` if that Windows account has not enrolled. If it
already has an identity, use only `--id`. Exchange ONLY the full 64-hex
public EndpointIds; compare them independently (for example by voice).
Then both computers run:

```powershell
.\akula-p2p-identity.exe --peer-confirm OTHER_ENDPOINT_ID
```

Each person must **retype** the complete other EndpointId after checking
it independently. The command never creates any file-reading permission.

On computer A (start this first and leave it running):

```powershell
.\akula-p2p-lan.exe internet-listen OTHER_ENDPOINT_ID
```

On computer B, from a completely different network, cellular tethering
or another household:

```powershell
.\akula-p2p-lan.exe internet-connect OTHER_ENDPOINT_ID
```

No LAN IP, public IP or manually opened UDP port is required for this
internet mode, but outbound DNS/HTTPS/UDP access and reachable n0
infrastructure are required; restrictive networks may still fail. The
result `OS_PINNED_QUIC_OK=true ROUTE_MODE=INTERNET_N0` proves only
a mutual authenticated QUIC **HELLO/OK**, not direct-vs-relay selection,
file delivery, safe file storage, File Gate or automatic Sync. Full root
authorization, a trusted Core→Staya host channel and two-PC file acceptance
remain future gates.

If you cannot or do not want to use n0 infrastructure, use direct
`listen`/`connect` with a reachable numeric IP:port or a separately
configured private overlay/VPN. In that case the normal direct-only CLI
will not use any n0 discovery/relay.

## Native one-file P2P on the standard installed Akula profile

The protected-main integration branch enables `installed -> p2p-native-receive-beta`
for the regular signed Windows installer and its existing auto-updater. P2P
networking starts only on explicit user send/receive actions; peer confirmation
is not a Vault or File Gate grant. LAN uses numeric private UDP addresses,
Internet mode explicitly opts into n0 rendezvous and relay, and verified bytes
remain in a separate local Inbox. See
`docs/sync/AK-P2P-NATIVE-RECEIVE-ONEFILE-01.md` for limits and acceptance gates.
This integration is not a published release until merged and signed, and still
requires installed two-machine GUI and Windows directory/SQLite custody review.
