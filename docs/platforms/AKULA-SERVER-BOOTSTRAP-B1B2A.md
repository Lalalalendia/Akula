# AKULA-SERVER-BOOTSTRAP B1b2a — actual systemd process on disposable GitHub runner

**Status:** isolated host lab only. A real process runs under transient **systemd** on an ephemeral GitHub Actions Ubuntu 24.04 runner and is deleted at job exit. This is neither signed package installation nor a released Akula Node service. The existing `akula-node-lab` is deliberately bounded and read-only; it is not a long-lived server daemon or P2P Sync Node.

This experiment runs **independently** of B1b1b #465 and B1b1c0 #468. It does not import their inert approval as authority or weaken their release/witness gates. The running binary is built from the checked-out repository source in CI; it is NOT a B1a-approved distributable artifact.

## Test architecture

- CI exclusively: script refuses execution unless `GITHUB_ACTIONS=true`, Linux runner and a numeric per-run identity. Names are derived from that identity. Existing paths are refused without removing them; a cleanup trap removes **only** paths and the unit the test created.
- Compile the existing opt-in `akula-node-lab` with Cargo; copy to uniquely named root-owned `/opt/` path. Create a synthetic top-level `demo.md` in a root-owned `0555` fixture directory. No real vaults, credentials, customer VPS or installed desktop.
- Create an unpredictable, single-run 256-bit *test token* and give it to systemd via `LoadCredential=node-token:<root-owned file>`. A fixed root-owned wrapper exposes the token to the **lab process environment only**, because the current D1 lab binary requires `AKULA_NODE_D1_TOKEN`. This is not production-grade token custody or a real UI/owner approval scheme.
- Launch a real transient service via `systemd-run` with `DynamicUser=yes`, `StateDirectory`, `ProtectSystem=strict`, `ProtectHome=yes`, `PrivateTmp`, `PrivateDevices`, `NoNewPrivileges`, bounded address families, `IPAddressDeny=any` with `IPAddressAllow=localhost`, and a read-only synthetic vault. Check the applied critical systemd properties.
- Verify the Node binds only to `127.0.0.1`; GET without bearer is denied (401); bearer-authenticated `/health` and `/v1/note` succeed with the exact synthetic content; test the original fixture SHA-256 before and after.
- Send a real `SIGKILL` to the child service via systemd, restart it, and recheck authenticated loopback access. No reliance on in-process mocks for this step. Fail the CI if process, sandbox, API or restart evidence is missing.
- All paths, credentials and the unit are torn down in a trap. The test runner is disposable. There is no SSH and no install on a real Ubuntu VPS.

## Claims excluded

- **NO operator-approved installation, OS-owned witness, external monotonicity or B1a-distributed signed package.** Root/adversarial privileged writes, compromised runner, image rollback and powercut durability remain outside the proof.
- The current D1 Node terminates after its bounded request count (`--max-requests`); therefore a one-time successful systemd launch/restart is *not* evidence of an always-on or production-ready Node service.
- `ProtectSystem=strict` by itself does **not** prevent connecting to AF_UNIX sockets; unit and IPC authorization must be enforced separately. `DynamicUser` requires careful `StateDirectory` and filesystem custody design before production.
- `SO_PEERCRED` identifies a **local** Unix-domain socket peer, not a remote Windows operator. A future broker must verify the independently signed owner claim and bind it to a root/OS-owned release floor.
- No firmware/TPM monotonic counter or external witness. We should explicitly scope the **initial** installer to a non-compromised host administrator and network adversaries, while providing a separate threat-gate for hostile-root/VM snapshot rollback; an ordinary filesystem journal cannot solve the latter.

## Next acceptance gates

1. Exact-HEAD specialized Ubuntu CI green, mandatory Application checks green, security review of named paths, state cleanup and transient systemd properties. Keep Draft until proved.
2. B1b1c0 signed-witness experiment accepted independently, then a separate OS-owned helper/ACL and authenticated local IPC with process-isolated negative tests; no write API in the installed desktop yet.
3. Future B1b2b: real B1a-signed Node package, immutable inode, systemd service install and rollback **inside a disposable VM**, not just the hosted CI runner.
4. Future B2: explicit Windows UI approval with independently pinned SSH key, nonroot remote user and clear refusal/rollback states. No live-server operation until separate admission.

References: Linux UNIX `SO_PEERCRED` manual (https://man7.org/linux/man-pages/man7/unix.7.html), `renameat2` NOREPLACE manual (https://man7.org/linux/man-pages/man2/renameat2.2.html), systemd.exec filesystem and credential namespace rules (https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html).

Parent: [AKULA-SERVER-BOOTSTRAP-02](https://app.notion.com/p/3fe32a84beec814e8342f69fdd518a34).
