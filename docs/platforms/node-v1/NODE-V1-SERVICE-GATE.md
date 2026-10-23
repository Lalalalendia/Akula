# NODE-V1-SERVICE-01 — staged non-root Iroh daemon

Status: **HOLD / candidate only**. This service is not installed by Akula.
A passing Linux CI confirms code and tests, not installed-VPS operation.

## Implemented boundaries

- The opt-in \`akula-node-v1-service\` is a separate binary from the original
  GUI-free local-health \`aquilum-core\` \`akula-node\` lab binary.
- Exactly one CLI operation is accepted: \`--serve-iroh\`. Root refuses to run.
- The immutable owner policy must already be independently enrolled under
  root-owned \`/etc/akula-node/owner-policy.json\` and match the Node Ed25519
  key under the dedicated private StateDirectory before any public listener.
- Iroh uses a **separate**, persistent 256-bit Ed25519 seed stored as
  `/var/lib/akula-node/node-iroh-seed.bin`. Only the approved installer may
  create it once; the service never generates or rotates this identity.
  Same service UID, 0700 directory, 0600 file, one link and NOFOLLOW apply.
  Missing, corrupt or replaced keys cause HOLD.
- The Node receipt-signing identity is also never created by the running
  daemon; only the explicitly approved enrollment phase can initialize it.
- Root policy schema **v3** pins the exact Iroh EndpointId alongside the
  Node receipt key and exact owner-signed read/write authority. The daemon
  checks this independent pin BEFORE opening the public UDP socket.
- Binds **only IPv4 UDP port 43871** using Iroh 1.3, relay disabled, with
  the fixed \`akula-node-v1/quic/v1\` ALPN. The caller must independently
  pin Iroh EndpointId AND the separate Node Ed25519 receipt/public key.
- Maximum eight concurrent in-flight handshakes, ten-second handshake and
  forty-second operation deadline. SQLite, owner policy and grant checks
  continue to be performed inside \`serve_one_authenticated_connection\`.
  A slow or unauthorized caller cannot create unlimited resources.
- Systemd template is in
  \`docs/platforms/node-v1/akula-node-v1.service\`, not deployed.
  It assumes a separately created **static dedicated** \`akula-node\` account,
  a signed root-owned ELF at \`/opt/akula-node/node\`, private state and
  runtime directories, and exact root policy under \`/etc/akula-node\`.

## One-shot non-root identity enrollment (install prerequisite)

A separate opt-in `akula-node-v1-enroll --enroll-identities` executable
has a **fixed operation and fixed StateDirectory**. It never accepts a
path, private key, account name, command, or network address from the UI.
A signed, owner-approved root installer must first create the dedicated
non-root service account and private StateDirectory, then invoke this one-shot
binary under that account. The binary checks that the protected owner policy
and **both** service keys are absent before writing anything.

The Node receipt key and separate Iroh transport seed are created with
exclusive creation, NOFOLLOW, 0600 mode and durable file/directory sync.
Only after independent readback does the command emit their public
identifiers. A partial crash is HOLD: no silent generation of whichever
key is missing. The root installer must separately bind both keys and the
owner's scoped grants into root-owned policy **v3** before systemd can start.

This does **not** authorize enrollment by itself. The signed/root-witnessed
package approval, pinned host identity, durable CAS and actual installer
invocation are still unconnected; this binary is not yet installed on a VPS.

The owner-policy file is root-owned and group-readable by the dedicated
`akula-node` account (`root:akula-node 0640`); it is never writable by
the service. A root-only `0600` policy would prevent the non-root daemon
from starting and is an invalid service configuration.

## Disposable Ubuntu systemd proof (not production installation)

The branch-only smoke workflow builds both opt-in service/enrollment ELFs
and runs `scripts/ci/akula-node-v1-systemd-host.sh` on a disposable GitHub
Actions Ubuntu runner. The test proves real non-root static UID, isolated
StateDirectory, one-shot keys, synthetic root-pinned policy v3, immutable
root-owned service binary, systemd start, fixed UDP listener, SIGKILL restart
and preservation of private identities. The CI policy is synthetic and has
no matching owner signing key or authorized grants; a systemd localhost
network restriction prevents use as a public server.

The test does not prove a signed root-helper installation, independent owner
approval, real VPS reboot, Windows setup or receiver File Gate import.

## Installation blockers — do not bypass

1. The existing signed/root-witnessed installer prototype must provision the
   actual service ELF with immutable release digest, package provenance,
   external owner approval, fixed executable path and atomic CAS receipt;
   it must pre-provision the Iroh key and independently pin its public ID.
2. The trusted installer must create/verify the isolated service account and
   root-owned policy **before** enabling systemd; it must refuse existing
   conflicting units, service users, symlinks or unsafe state and log HOLD.
3. Windows Akula must verify SSH host identity, show a short owner-approved
   plan, verify the signed package and require separate installation consent.
   A successful SSH probe is not installation consent.
4. Port 43871/UDP must be explicitly authorized at the firewall and included
   in an exportable configuration with independently verified endpoint IDs.
5. The service must be tested on a real Ubuntu VPS through restart/reboot,
   revocation, disk and network failure, including A→Node→B while A offline.
   Receiver writes must go through the existing no-overwrite File Gate.

The service MUST NOT be declared READY based on a listening process or an
Iroh handshake alone. Full installed end-to-end acceptance remains HOLD.
