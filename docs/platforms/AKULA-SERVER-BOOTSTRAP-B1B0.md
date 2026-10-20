# AKULA-SERVER-BOOTSTRAP B1b0 — inert private staging laboratory

**Status:** Draft experiment, not an installer. Based on B1a PR #448. Parent: https://app.notion.com/p/3fe32a84beec814e8342f69fdd518a34.

Scope: A Linux feature-gated Core library accepts B1a-authenticated candidate bytes and an opaque operator-plan digest, streams bytes into a private 0700 directory, verifies the signed size/hash, synchronizes and marks staged bytes read-only, rechecks byte identity, then commits an inert InstallId journal. Exact repeated InstallId with identical manifest and approval is idempotent; mismatched approval, incomplete state or corruption refuses. Feature flag is opt-in and does not alter installed Akula.

**Deliberate limitations:** No trusted operator-approval CAS, real release signing key, trusted clock, independent release floor persistence, root-owned directory or hostile-root fencing, file seals, Node binary execution, SSH, systemd service, installation, rollback, power-loss durability proof or real VPS. flock coordinates only cooperating writers. An inert approval digest does not itself prove consent. This test cannot be treated as B1b/B1 release acceptance.

**Acceptance before any systemd work:** exact-head Linux CI, race/negative tests, security review of directory ownership, symlinks, atomicity and fsync; B1a dependency accepted. Production installation requires a separate OS-owned helper plus approval authority and real disposable-VM verification.
