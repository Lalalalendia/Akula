# STY-DEVICE-02 — two independent OS processes on local QUIC

Scope: experimental Roha/noq transport crate; no desktop/installer integration.

- Real separate processes bind real loopback UDP sockets and authenticate each other through Iroh 1.3 QUIC.
- Temporary 32-byte test key files recreate the same EndpointId after process restart. They are NOT an OS-protected production key store.
- AllowedPeers clone now shares local revocation. Each new logical operation must re-check the actual connection remote_id. The receiver denies a second stream without reading its bytes after revocation.
- Outbound connect rechecks policy after the TLS handshake, blocking a concurrent removal.
- Existing tests already cover unknown peers and corrupt frame rejection. Linux and Windows test jobs were added to the existing transport workflow.
- Network admission is not File Gate access. There are no customer file reads/writes, no durable ACK/APPLIED, no owner signing or pairing UI, and no real 2-PC WAN/NAT acceptance.
- Required next: OS custody, user confirmed pairing/QR, Core-owned policy epoch and binding to Vault/object File Gate, real multi-device acceptance.

Do not merge until exact-HEAD process test and full transport CI are successful.

## STY-DEVICE-02 live host admission seam

The existing `AllowedPeers` can now take `with_live_check`, a host-owned predicate
over **the authenticated Iroh `Connection::remote_id()`**. This predicate is
evaluated on connect, accepted connection, and every call to
`authorize_new_operation`; no state cached at handshake becomes a permanent
grant. The two-process test keeps the real QUIC connection open, revokes a
separate dynamic policy after the first acknowledged stream, and refuses to
read the next operation. Original in-process `revoke` remains functional.

For Windows Akula the required closure is based on
`aquilum_core::p2p_device_identity::peer_pin::require_pinned_remote`
with a user-selected full EndpointId and `remote_id().as_bytes()` from the
connection itself. No wire-provided key is accepted as the authenticated input.
This change adds **only the transport hook and a real-network falsifier**:
it does **not** yet wire that closure from installed Native, persist a Staya
endpoint, grant file reads, integrate File Gate or prove transfer between two
installed PCs. No new dependency, transport, startup job or default feature.
