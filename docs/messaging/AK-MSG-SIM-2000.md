# AK-MSG-SIM-2000 — synthetic 36-family building contact test

**Status:** experimental, inside Akula Core. Not real Wi-Fi/BLE measurements,
not a released messaging feature, and not W4 cryptographic receipt validation.
This fixture compiles **only for tests** with opt-in
\`messaging-sim-lab\`; no daemon, sockets, GUI, file writes, or external server.

## Scope

The test models a single five-floor building with 70 two-sided rooms per floor,
five stairwell shafts, 2,000 logical Akula devices and 60 time-staggered
messages per scenario.
It invokes the *same* \`OutboundMessage\` state transitions introduced in
[PR #530](https://github.com/Lalalalendia/Akula/pull/530).

Four communication profiles:

- Shared Wi-Fi LAN, with locally reachable clients even without WAN.
- Mixed Wi-Fi+BLE and BLE-only devices, including voluntary dual-radio relays.
- BLE-only devices with synthetic short-range physical contacts.
- Exclusive one-of Wi-Fi/cellular IP/BLE per device: **BLE cannot cross to IP**
  without an explicit dual-radio gateway (none exists in this profile).

Each message is created during a reproducible 40-slot send window and has a
40-minute synthetic TTL. This ensures the simulated disconnection interval can
affect *new* messages; starting every message at time zero would wrongly make
some outage scenarios indistinguishable from no outage.

For each profile: 0%, 25% or 100% mobility, and none / one offline
window / periodic outages. This is **4 × 3 × 3 = 36 distinct families**.
All scenarios use the same controlled seed and starting assignments.

The position graph is constrained to rooms, corridor positions, and five
staircases. A vertical move crosses at most one floor and only at a stair
location; a horizontal move advances one corridor position per tick.
No user teleports through a wall. For BLE, same-room/same-floor/cross-floor
contact probabilities are deliberately **illustrative, uncalibrated**
parameters; one contact means enough hypothetical capacity for one
1-KiB opaque message in a time slot, not a demonstrated BLE packet throughput.
No packet-level collision model, phone operating-system background restrictions,
battery calibration, PHY/MAC, AP congestion, or changing Wi-Fi infrastructure is
claimed. A separate real-device / physical-propagation validation is required.

The simulator uses greedy limited-copy **store-carry-forward** with a fixed
copy bound of six. Relay observations are never final delivery. A
test-only recipient witness drives the existing state machine to
\`RecipientAccepted\` when a valid contact is assumed. This witness is **NOT**
an Ed25519 signature and cannot be compiled outside tests. It must never be
used in production. Production receipt verification belongs to WOW W4,
as explored independently in [PR #537](https://github.com/Lalalalendia/Akula/pull/537).

\`SharedWifi\` assumes all local access points bridge clients. The exclusive
Wi-Fi/cellular profile assumes the available clients can use a common
authorized IP path. Neither proves NAT traversal or IP availability
in a physical building.

## Reproduction

From \`aquilum-app/\`:

\`\`\`bash
cargo test --locked -p aquilum-core --features messaging-sim-lab building_sim:: -- --nocapture
cargo check --locked -p aquilum-core --no-default-features
\`\`\`

The tests report per-family final recipient acceptances, expiry, authority
revocations, limited relay copies, assumed transferred bytes, disallowed
cross-technology pairs, relative delivery slots, and p50/p95 delay **among
accepted test messages**. Expired and revoked messages remain in the denominator
of the separate overall acceptance fraction. Repeating the same inputs must
return the identical report. Do **not** interpret these assumed-link numbers
as measured reliability. The 36/36 test is a proof of **scenario execution**,
not proof of real-world performance or Akula messaging release-readiness.

## Follow-up acceptance blockers

1. Replace synthetic contact probabilities with measured wall/stairwell/AP
   propagation, interference, discovery time, BLE transfer capacity and power.
2. Compare algorithms on **identical** generated contact/event schedules,
   rather than picking the best result from incomparable random traces.
3. Model phone background process suspension, OS Bluetooth quotas, independent
   WAN vs WLAN states, bursty packet loss, and persistent mailbox quotas.
4. Link real WOW W4 durable sender and recipient journals, authenticated
   DeviceId–key mappings, and final recipient receipts; no test witness in UI.
5. Validate paired Akula devices on Roha/noq, then actual BLE devices and
   representative staircases. Only after that draw deployment conclusions.

Specs: [AK-MSG-SIM-2000](https://app.notion.com/p/40032a84beec8113a9e0ddd204081f50)
and [AK-MSG-CORE-01](https://app.notion.com/p/40032a84beec81919ec8df2e31056543).
