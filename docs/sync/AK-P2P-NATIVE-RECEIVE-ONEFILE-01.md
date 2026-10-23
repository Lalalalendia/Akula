# AK-P2P-NATIVE-RECEIVE-ONEFILE-01

Experimental, default-off Native receiving stack on the one-file QUIC beta.

The recipient selects an OS-pinned, non-revoked sender in Akula. A numeric
private LAN IP:UDP port in the system clipboard provides connectivity only,
not identity or file permission. The transport independently rechecks the OS
pin and authenticated Iroh TLS peer at connection and before persistence.

Received data is bounded to one file, 8 MiB, and validated with BLAKE3. The
new receiver mode stores it only inside the independent permanent SQLite
Received Inbox and does not create a user destination or Vault import.
Duplicate replays from the same authenticated peer map to the same receipt.
Successful completion requests a Native Inbox refresh.

The receiver starts only after a user click, runs in a separate cancellable
Tokio runtime, and is cancelled when the transfer modal closes. It does not
start a listener at application launch. The legacy operator-only CLI remains
separate and retains its explicit destination mode.

Known release HOLD: physical Windows-to-Windows GUI acceptance, pairing and
LAN address auto-discovery, signed installer, trusted Windows ancestor custody,
and durable authenticated sender acknowledgement. A successful QUIC ACK is not
proof of power-loss durability. Normal installed Akula and main remain off.
