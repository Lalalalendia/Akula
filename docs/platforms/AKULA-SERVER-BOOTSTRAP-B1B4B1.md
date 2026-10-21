# AKULA B1b4b1 — isolated read-only witness process and kernel-authenticated Unix IPC

**Depends on B1b4b0 [PR #499](https://github.com/Lalalalendia/Akula/pull/499).** This is a GitHub Actions disposable Linux laboratory, not an installed service.

## Trust and execution boundaries
A separate UID0 binary starts under a real transient systemd unit with fixed paths in /run/akula-b1b4b1-lab and /opt/akula-b1b4b1-lab. The 0711 root-owned socket parent contains only a root-owned 0600 allowed-peer-uid file. Witness signing seed, independently derived public key and explicit enrollment marker live under root-owned 0700 trust. CAS journal is a separate root-owned 0700 directory. No public networking or arbitrary path arguments.

The binary opens and verifies the existing B1b4b0 OsWitnessLab before binding a single AF_UNIX socket. A 0600 socket is granted *transport* to an approved CI group (0660), but every request must satisfy Linux kernel SO_PEERCRED against a *single* independently OS-provisioned nonroot UID. Root caller, wrong or trailing frame, truncated frame and unrecognized opcode must fail. The entire IPC vocabulary is ONE exact fixed-frame READ_CHECKPOINT request. The response contains exact typed revision, chain tip and release floor. It cannot sign a new witness, grant an install, run a command, load an arbitrary path, modify a service or write a user vault.

The actual Ubuntu CI verifies real root process and systemd sandbox, nonroot positive and root/invalid/trailing/truncated negative requests, journald-backed evidence, root trust non-writability, fail-closed missing marker and mismatched pinned public key in separately started failed units, and clean service restart. Parent runner tests separately prove a new B1b4b0 signed CAS record with actual UID0, not a mock.

## Security limits
This process boundary and SO_PEERCRED are real, but it does NOT isolate against compromised root, same-disk snapshot rollback, stolen signing seed or manipulated system time. The process is deliberately bounded, not persistent. The current IPC cannot accept authorization or sign a CAS transition; owner consent, revocation, one-use challenge issuance and fresh commit-time atomic readback remain future gates. No real VPS, Akula Desktop, production installer or user data changed. Root is explicitly trusted for the first single-owner self-hosted VPS milestone.
The integration test also creates a genuine owner-signed, fsynced nonzero CAS revision 1 with signed release sequence 17 before starting systemd. The separate witness process must read that revision and both nonzero hash-chain tips, and retain the same read-only result after restart; an empty genesis-only response does not count as acceptance.
