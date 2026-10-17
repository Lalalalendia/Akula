# Plugin security threat model — initial 16 falsifiers

Scope: untrusted plugin package + guest runtime + host APIs + imported/synced vault content + network/registry. Status: **proposed test plan; no tests below are claimed to pass in production**. The IDs are mirrored in `architecture-inventory.json` and validated in the existing Main PR gate.

### T01 — Direct cross-vault read/write
Guest with current/selected-note authority asks for a second vault, guesses file IDs or invokes a write. **Required:** host rejects foreign handles and any action outside a grant. **Future tests:** PLUGIN-04/07/09; DENY before IO, invariant across multiple vaults.

### T02 — Path escape and time-of-check/time-of-use race
`../`, symlink, Windows junction/reparse/UNC, casing/normalization or concurrent rename changes target after permission check. **Required:** capability-bound open/commit, fail closed on identity drift. **Future tests:** PLUGIN-04, Linux/Windows/macOS.

### T03 — Search/metadata/event side channels
Hidden note names, snippets, backlinks, tags or event payloads leak outside granted folder. **Required:** same read ACL at every derived-data source. **Future tests:** PLUGIN-07/08.

### T04 — Read + network exfiltration
Plugin reads vault then sends content to a permitted endpoint. **Required:** show composed risk, narrow grant, clear user intent and audit; network allowlist alone cannot prevent deliberate exfiltration once permitted. **Future tests:** PLUGIN-11/15.

### T05 — Rich-output/render exfiltration
Rendered Markdown, HTML, remote image, iframe or embedded asset loads a network resource without calling `network.fetch`. **Required:** sanitization and separate render-resource network authorization. **Future tests:** PLUGIN-10/16.

### T06 — Delayed indirect execution by a write
Plugin writes a script, automation config or executable file later run by other software. **Required:** do not auto-execute vault content; distinguish content edits from privilege-changing executable edits. **Future tests:** PLUGIN-09/16.

### T07 — Process/FFI privilege escape
Guest obtains shell, native library, unbounded Node/Deno or direct process capabilities. **Required:** no system exec/FFI in normal community runtime, no ambient WASI imports; dangerous trusted integration is a distinct profile. **Future tests:** PLUGIN-05/06.

### T08 — SSRF, redirect or DNS rebinding
Allowed URL redirects or resolves to localhost/metadata service/private subnet, including proxies and IP literal forms. **Required:** network broker enforces destination throughout connection chain and rejects unsafe targets by default. **Future tests:** PLUGIN-15.

### T09 — Forged plugin identity or stolen resource handle
Guest claims another publisher/plugin, reuses revoked or foreign vault handle. **Required:** host-bound identity and unforgeable scoped handle with instance and vault binding. **Future tests:** PLUGIN-02/03/06.

### T10 — Revocation during in-flight work
User revokes access while pending async operation continues to write/send. **Required:** revalidate at side effect, cancel pending operations, invalidate handles; cannot retract already transmitted information. **Future tests:** PLUGIN-03/09/15.

### T11 — Dirty editor buffer/history corruption
Background plugin updates disk from stale snapshot while native editor holds unsaved changes. **Required:** DocumentHub interlock, expected hash/revision and File Gate; conflict preserves both versions. **Future tests:** PLUGIN-09.

### T12 — Resource exhaustion
Infinite loop, huge message/UI tree, rapid RPC/events, hanging host function, memory growth or plugin crash stalls editor. **Required:** resource quotas, backpressure, interrupts, host timeouts and quarantine. **Future tests:** PLUGIN-06/08/10.

### T13 — Silent permission escalation via update
Previously approved plugin expands read-current to read-all, adds domains or changes publisher. **Required:** signed package identity + manifest/grant diff and fresh consent before new side effects. **Future tests:** PLUGIN-11/19.

### T14 — Compromised publisher, rollback or freeze
Stolen release credential, malicious signed new version, stale metadata or replayed package. **Required:** verified provenance, rollback/revocation safeguards, publisher recovery and no blind auto-update. **Future tests:** PLUGIN-19.

### T15 — Imported vault auto-install or auto-run
Adversarial synced Markdown or plugin folder becomes active code or grants on another device. **Required:** keep installed plugin artifacts and grants outside vault sync; never auto-enable from file copies. **Future tests:** PLUGIN-12.

### T16 — Confused deputy through MCP, IPC or clipboard
Plugin invokes less-restricted MCP/local API, plugin-to-plugin messaging or UI/open-url instead of authorized host call. **Required:** consistent identity and broker policy across entry points; separate grants for each channel. **Future tests:** PLUGIN-16.

## Test execution policy

This document specifies **negative acceptance tests**, not a list of known defects. Each PLUGIN stage implements its mapped tests and provides failing-before/fixed-after evidence. For permissions/sandbox guarantees use real untrusted fixtures on Windows, Linux and macOS before public release. Documentation lint/architecture script does **not** substitute for runtime sandbox penetration tests.

Sources: [WASI](https://wasi.dev/security), [OWASP SSRF Prevention](https://cheatsheetseries.owasp.org/cheatsheets/Server_Side_Request_Forgery_Prevention_Cheat_Sheet.html), [Zed extension capabilities](https://zed.dev/docs/extensions/capabilities), [Obsidian plugin security](https://obsidian.md/help/plugin-security), [TUF](https://theupdateframework.io/docs/security/).
