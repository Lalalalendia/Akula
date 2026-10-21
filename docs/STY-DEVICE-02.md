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