# AK-P2P-LAN-PING-01 — two Windows Akula keys over live LAN QUIC

Scope: **separate operator diagnostic**, not an installed File transfers feature. Reuse #625; do not resurrect #620's duplicated OS store or network admission wrapper.

## Preconditions

Both Windows accounts explicitly initialize their existing Akula Iroh identity and manually confirm each other's independent full 64-character EndpointId using the opt-in File transfers pairing UI (#609) or existing secure CLI. Never auto-enroll, copy from an untrusted invitation, or replace a credential. Each machine must be reachable over direct UDP on a trusted local network. Relay is deliberately disabled.

Build the `akula-p2p-ping` binary from the isolated `interop/akula-staya` workspace using the `p2p-live-identity` feature. It is **not packaged** in the installed app and has no file capabilities.

On computer B, run `akula-p2p-ping listen <A_FULL_ENDPOINT_ID>`. Read `BOUND_UDP` and use B's reachable local LAN IP with that port. On computer A, run `akula-p2p-ping connect <B_FULL_ENDPOINT_ID> <B_LAN_IP>:<B_UDP_PORT>`. Both processes print their actual local endpoint public key and finish only when the full selected peer matches the authenticated Iroh `remote_id()`, the protected OS peer pin is still confirmed and the bounded HELLO/OK exchange succeeds. The final output is `PINNED_QUIC_OK=true FILE_TRANSFER=false FILE_GATE=false`.

## Safety and falsifiers

No network action is attempted without a confirmed OS pin. Invalid lowercase IDs, changed/partial IDs, unknown/revoked peers and missing/corrupt OS identity deny. The actual TLS-authenticated remote key is rechecked after handshake and again before each new logical message operation. The test accepts only bounded marker bytes; no file paths, user content, Staya GET, Bao, Vault permission or durable ACK. Wrong IP/port or blocked Windows Firewall causes bounded failure without a fallback to relay or plaintext identity.

Profile CI checks this command on Linux and Windows with Rustfmt, argument parsing tests, strict Clippy and the preexisting isolated real QUIC/OS pin test. **Physical A/B acceptance is still required**; CI on one runner does not prove independent installed-machine connectivity. This diagnostic must not be mistaken for a production file-transfer runtime.

Future next step: expose this exact protected identity and verified connection via the existing Native transfer worker and sender-consent boundary, not a parallel network implementation.
