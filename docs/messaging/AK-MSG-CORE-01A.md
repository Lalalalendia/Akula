# AK-MSG-CORE-01A: opt-in pure messaging state model

**Status: experimental, no user-facing messaging.** Canonical Notion contract:
[AK-MSG-CORE-01](https://app.notion.com/p/40032a84beec81919ec8df2e31056543).

The state transition model is part of `aquilum-core::messaging` and only compiles
with the opt-in `messaging-core-lab` Cargo feature. It starts no worker and
does not add a UI, store, socket, account, identity registry, transport, or
plaintext serialization. The ordinary Akula configuration is unchanged.

## Integration boundary

1. The caller first **durably stores the original E2E-encrypted WOW bytes** in
   the future authoritative WOW W4 queue, then creates
   `OutboundMessage::after_durable_enqueue` with its message ID, recipient
   `sync::DeviceId`, immutable ciphertext digest, trust epoch and expiry.
2. `RouteAvailable/RouteLost` are transport observations only. `SendAttempt`
   is a planned send, not a claim that an IP, BLE or mobile link succeeded.
   `TransportFailed` schedules explicit retry; no background retries occur.
3. A `RelayStored` observation is never `RecipientAccepted`. The application
   must display different statuses for queue, relay, recipient and UI visibility.
4. Only `accept_receipt` may set `RecipientAccepted`; it matches
   message ID, immutable byte digest, typed device identity and current policy
   epoch, then calls a caller-provided `ReceiptVerifier`. There is **no**
   production cryptographic verifier in this PR: tests inject an explicitly
   fake witness. No remote packet may call this method without cryptographic
   verification and authenticated DeviceId-to-WOW-signing-key binding.
5. A higher authoritative trust epoch revokes *pending* delivery; a delayed
   receipt for an older epoch cannot mark it delivered. Already accepted
   history is not retroactively falsified. Time is an explicit monotonic
   parameter, usable with LesHa or a future 2,000-device Akula simulator.
6. `Expired` blocks a late receipt. A duplicate correctly verified receipt
   returns a distinct idempotent outcome. The model DOES NOT implement durable
   deduplication across process restarts; that remains a WOW W4 journal duty.

## Trust and security boundaries

- `RelayStored` is a **trusted local simulation observation**, not a
  validated relay signature or end-to-end proof. A production relay adapter
  must authenticate its own storage receipt separately.
- Do not confuse `core::sync::DeviceId` (UUID) with Roha transport identity
  (cryptographic EndpointId). Mapping must be validated by the later
  CORE-01B identity-policy port. A live QUIC link grants no file rights.
- The state machine is an **in-memory projection**. It does not assert a durable
  journal write, does not process inbound encrypted envelopes, and does not
  create the recipient-signed receipt. Marking acceptance via a fake verifier
  would be unsafe outside tests. No user messaging until CORE-01B/C/D.
- Large attachments are handled by the existing guarded File Gate / Sync Core,
  not copied into experimental mesh payloads. No new file-authority boundary.
- Delivery after recipient absence, relay outage and revoked trust are
  model-level test scenarios, not evidence of physical Bluetooth/CGNAT paths.

## Contract-only scale test

The `messaging::scale_tests` fixture instantiates **2,000 logical Akula clients**
across five floors and 70 rooms per floor, then exercises a fixed mix of
nonreachable recipients, relay loss, failed transport and revoked trust.
The expected result is 1,500 authorized recipient acknowledgements and 500
revoked senders that never advance. This is an **in-memory state-machine
smoke test**; it does NOT simulate BLE propagation, walls, routes, actual
movement, or the 36 physical/network scenarios in Notion. Those require
separate calibrated transport and mobility models.

## Validation

From `aquilum-app`:

```sh
cargo test --locked -p aquilum-core --features messaging-core-lab messaging:: -- --nocapture
cargo check --locked -p aquilum-core --no-default-features
```

The targeted GitHub workflow `messaging-core-contract.yml` runs both plus
standalone Rust formatting checks. Do not merge to main until tests and
ordinary application regression checks pass and reviewers confirm that
feature-off compilation has no runtime changes.

**Next:** bind identity/receipt verification to the real audited WOW W4
interfaces and develop a durable journal port, then feed the same transitions
to the 4 × 3 × 3 network scenario matrix of
[AK-MSG-SIM-2000](https://app.notion.com/p/40032a84beec8113a9e0ddd204081f50).
