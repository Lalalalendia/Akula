# AK-P2P-NATIVE-RECEIVE-ONEFILE-01

Native P2P receiving stack on the one-file QUIC beta. The signed Windows
installed feature is configured to include P2P send, receive and pairing,
subject to PR integration, exact-head testing and signed release approval.
The user must explicitly start each network operation; no P2P listener runs
automatically at application launch or during an updater check.

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

### Windows pinned stage and owner/DACL gate (experimental)

For Native receiving on Windows, the existing Akula Core NT directory-handle
primitive now creates the private stage relative to a pinned parent HANDLE
(never by resolving a separate global stage pathname). The parent is first
audited using the existing read-only owner/DACL guard, and separately opened
read/write parent handles must have identical Windows file identities.
The parent and stage handles remain alive during fsync and cold verification.

The Windows CI additionally tests rejection of an Inbox root with explicit
Everyone Full Control. This is a negative gate, not proof against a malicious
same-user process or privileged actor. The **objects directory and SQLite
database still reopen by global paths**, and the whole ancestor chain is not
pinned for the duration of publication. Release HOLD remains in force.

### Local OS-protected peer enrollment integrated from PR #609 (opt-in)

The Native send/receive beta now enables the existing `p2p-device-ui` feature,
rather than only listing previously pinned contacts. The transfer dialog
offers the existing user-mediated two-field manual pin interface from
[PR #609](https://github.com/Lalalalendia/Akula/pull/609), using the same
first-party Windows Credential Manager `peer_pin::confirm_peer`,
`peer_state`, and `revoke_peer` APIs. The source implementation and
18 RU/EN pairing strings are reused from #609, not an additional identity
store. Peer controls appear only when the protected local identity has
been explicitly loaded or initialized by the user. No private key is
displayed, exported, stored in app files or sent across the network.

The user must manually enter and independently compare the full 64-character
key twice. Confirmation does not create a network session or authorize
remote Vault/file reads. Separate review and confirmation are required to
revoke, a changed key invalidates revocation, and stale background results
cannot silently change the selected peer. A successful confirm/revoke
refreshes OS-backed contacts so Native send/receive reflects revocation.
The actual QUIC `remote_id` and fresh protected pin are still checked by
the transport before reading a file or writing the private Inbox.

The existing scoped Windows workflow additionally builds the complete
Native feature and runs both peer-pair UI/state test modules, local-key UI
tests and native send/receive action tests. **Code and test wiring only**:
no confirmed exact-head CI, physical Windows two-device acceptance, complete
Windows Inbox handle custody or shipping Windows installer. PR #609 remains
an independent upstream implementation until reconciled; this branch
does not merge or close it.

## 24 October: standard auto-update integration decision

The normal signed Windows installer and updater use Cargo --features installed.
This profile now includes p2p-native-receive-beta transitively, so a future
main-based, publisher-signed, version-increased release will contain P2P
without a second application or a separate P2P installer. The standard
signed candidate workflow inspects the installed feature relationship
before packaging. Scoped Windows CI compiles/tests this installed graph.

This source change alone is NOT a published update. Public v0.3.12 is
unchanged. The stacked PR chain must enter protected main, CI must pass,
and the normal owner-approved signed release must publish a newer version.
No bypass of signature, Windows path-custody or updater integrity checks.
Peer confirmation is local-only, not a durable delivery ACK.

## 24 October: pinned Windows Inbox object + SQLite ACL negative gate

The Native receiver now re-audits the **same NT root directory identity**
before opening the path-based Inbox ledger and again before reporting a
verified receipt. An existing `objects` directory, `received-inbox.sqlite3`
and optional `-wal`/`-shm` files are individually opened using NT
RootDirectory-relative one-component opens with explicit READ_CONTROL.
Their owner and DACL must satisfy Akula's conservative Windows File Gate
audit; reparse points and multiply hard-linked SQLite files fail closed.
After publication, both the objects directory and SQLite file must exist
and pass the same check. A failing postflight never becomes a sender ACK.

New Windows regressions grant Everyone Full Control to an existing Inbox
objects directory or SQLite database and verify that a second received
file is refused with no additional ledger record. The tests are part of
the existing single exact-HEAD P2P Windows workflow, not a new CI matrix.

**Scope limitation:** `rusqlite` and object creation/cold-open remain
path-based between these audits, so a malicious same-user actor can still
race ancestor or child replacement. This does NOT establish full
handle-relative custody, hostile process isolation, or a durable signed
delivery receipt. Do not merge/publish P2P until those release gates and
two installed-Windows-PC Internet E2E have been accepted.

## 24 October: exact-blob NT custody readback before receiver OK

The previous [exact-head Windows CI #40086269028](https://github.com/Lalalalendia/Akula/actions/runs/40086269028)
fully passed, including both Windows negative ACL regressions, external n0
EndpointId-only file transfer, cold Inbox and the `installed` feature graph.
This is not an installed two-physical-PC acceptance.

For an actual Windows received file, after the SQLite receipt and existing
root/objects/SQLite pre- and postflight, the receiver now independently
opens `objects/<locally derived UUID>.blob` relative to the previously pinned
root directory and pinned `objects` directory. On the exact opened NT HANDLE
it requires owner/DACL audit, a single hard link, the authenticated byte count
and streamed BLAKE3. It also rechecks the `objects` directory's NT identity.
A mismatch refuses the receiver's success/OK. This applies both to newly
received bytes and duplicate receipts that SQLite would otherwise call
`VerifiedLocal`.

Additional Windows CI regressions exercise an existing verified blob granted
`Everyone:F` and one with a second hard-link name. The previously verified
SQLite record may remain for investigation, but no subsequent successful
network receipt is issued for that compromised object.

**Remaining release HOLD:** Object and SQLite **writes** still go through
path-based `rusqlite` and file creation. Final handle-relative readback
narrows detection but cannot close a malicious same-user rename/swap race
or guarantee power-loss durability. Before beta release require complete
Windows file/directory custody, physical installed two-network P2P, and
ordinary signed application update. The new exact-head CI must pass before
marking this slice accepted.

## 24 October: original NT-HANDLE deletion of temporary received stages

The Windows receiver's temporary `.stage` file is created under the pinned
Inbox root by the existing NT `NtCreateFile(FILE_CREATE, DELETE_ACCESS)`
boundary. It previously removed that temporary file in a `Drop` guard using
`std::fs::remove_file(original_path)`, which could delete an unrelated file
if a same-user actor renamed/replaced the stage's original name.

For Windows only, the receiver now keeps a duplicate of the newly created
stage HANDLE as a `PinnedStageCleanup` guard. After successful blob/SQLite
verification, it uses the existing `NtSetInformationFile` function with
`FileDispositionInformation (13)` to mark the **opened file object**, not a
pathname, for deletion on final close; an unsuccessful operation attempts
the same operation in its drop guard. A successful network OK requires that
this handle-only delete disposition succeed. After applying disposition
the handle is only closed and is never accessed again.

The new Windows test `windows_pinned_stage_cleanup_never_deletes_replaced_name`
renames a live stage, places an unrelated sentinel under its old name and
checks both success/error cleanup modes. Only the original stage may be
removed. This test runs in the existing exact-head Windows P2P job, not a
new CI workflow. Per Microsoft's NT disposition contract, the original
stage was opened with DELETE access and deletion occurs at handle close.

**Release remains HOLD**: This removes one cleanup pathname race, but
`rusqlite` and `save_verified_stage_as_new` still create/open their
persistent targets via global paths. Same-user namespace custody and
power-loss signed delivery are not established. No merge, independent
installer or auto-update activation before installed physical WAN tests.
