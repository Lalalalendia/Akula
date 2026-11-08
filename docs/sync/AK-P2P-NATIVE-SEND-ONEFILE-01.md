# AK-P2P-NATIVE-SEND-ONEFILE-01

Dependent on Draft #677 → #672 → #642. **Feature default OFF.**

The native File transfers dialog can now explicitly initiate a **real
one-file QUIC send** for exactly one non-paused draft and one OS-pinned,
non-revoked recipient. It calls the SAME Iroh implementation as the isolated
one-file beta, imported as a library: **no external CLI or script**.

An isolated background Tokio runtime binds a temporary direct-LAN endpoint
only after user consent. Its public UDP port is displayed inside the dialog;
wildcard 0.0.0.0 and :: binds are not claimed to be reachable LAN IPs.
The network path rechecks the persistent peer pin before opening the local
source file, verifies TLS remote_id, and repeats pin checking before serving.

The receive side still runs through the existing explicit one-file beta path
and must be integrated with Native later. This is **not** an installed or
signed released feature. Known unfinished gates: native receive, compact
address exchange, native enrollment, cancellation, signed installer flavor,
Windows ACL/reparse custody, dual-physical-PC acceptance, and end-to-end GUI
pointer tests. Do not turn on P2P in ordinary installer/update.
