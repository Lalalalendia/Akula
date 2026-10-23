# AK-P2P-NATIVE-RECEIVE-ONEFILE-01

Experimental, default-off Native receiving stack on the one-file QUIC beta.

## 24 October: explicit Internet/N0 route on the same authenticated file protocol

Native sender and receiver now have **separate LAN and Internet (n0)** buttons.
The Internet button opts into Iroh 1.3 `presets::N0`, n0 DNS/Pkarr
discovery and encrypted relay fallback; the LAN button retains
`presets::Minimal` and `RelayMode::Disabled`, using the exact numeric private
IP:UDP port from the receiver's clipboard. Internet mode needs no LAN address.
The same bounded one-file QUIC stream, TLS `remote_id()`, fresh Windows
Credential Manager pin/revoke checks, BLAKE3 and persistent private Inbox are
used on both routes, without a second transfer engine or Vault writes.

No external service starts at application launch. On user opt-in, an N0
endpoint must report `online()` within 60 seconds; the identified receiver
uses `EndpointAddr::new(confirmed_peer)` with a 90-second connect bound.
The sender awaits a manually triggered receiver; the third-party n0 service
can see IP addresses, timing, and traffic metadata but cannot read the QUIC
file content. This reliance is disclosed in the RU/EN transfer interface.
A relay only forwards packets; it **does not** store data for offline devices.
A successful receiver ACK is **not** power-loss durable or File Gate APPLIED.

The existing standalone CLI remains LAN-only. This change is on #691's
stacked default-off research branch, not in `main` and not in signed Akula.
Real WAN two-physical-Windows-PC proof, cross-network n0 availability,
receiver identity confirmation, negative/revoke tests, handle-relative
Windows Inbox custody and signed installer are still release HOLD.
Test-only invalid-key rejection and strict separation of the two route
choices are checked in the scoped Windows workflow.

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

### Windows receiver path hardening slice

The Native receiver keeps the **same newly created, read/write OS stage handle**
through fsync, seek and cold BLAKE3 publication; it no longer reopens the stage
by a mutable pathname after writing.

On Windows, receiver root creation and pre-publication inspect every existing
directory component for `FILE_ATTRIBUTE_REPARSE_POINT`, rejecting stable
junctions/symlinks before write and refusing a missing chain at publication.
A Windows junction regression and ordinary-directory regression are run in
the scoped receiver workflow.

**Still HOLD:** these checks are path-based and subject to ancestor substitution.
There is no claimed Windows handle-relative root/objects/SQLite custody, owner-DACL
proof, same-user attacker isolation, or power-loss durable signed delivery receipt.
