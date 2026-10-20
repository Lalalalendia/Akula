# Akula Host Broker — independent denial-only research crate

This is **not part of the Akula workspace**, not installed as a Windows Service, and not started by the desktop app. `Cargo.toml` declares its own empty workspace so that Cargo cannot silently add a new production member.

On Windows, its standalone binary checks the real process token for **LocalSystem** before creating a single local named pipe instance. It applies a dedicated explicit pipe ACL, refuses remote clients, reads one bounded message, obtains the named-pipe client's Windows impersonation token, and only returns a negative status. There is no `grant`/ `enroll`/ `approve`/ `write` operation.

Run tests without any elevated setup:

```sh
cargo test --locked --manifest-path aquilum-app/host-broker-lab/Cargo.toml --all-targets
```

Do **not** run the binary as LocalSystem from a product installer. A separate, audited SCM/service lifecycle, server identity verification on every client pipe, protected policy store, durable revocation epochs, true peer identity from the owner of the QUIC connection, cleanup and crash tests are missing. The application intentionally has no client connector.

Detailed threat model, protocol and future gates: [RN-4E1b-03b1a](../../docs/sync/staya-rn4e1b03b1a-broker-ipc.md).
