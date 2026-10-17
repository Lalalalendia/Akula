# PLUGIN-02 — Deny-by-default in-memory Permission Broker

Status: **implementation in PR; in-memory only, no guest runtime**. Cross-reference [PLUGIN-00–22 program](https://app.notion.com/p/3fc32a84beec8132bd9cd127ba517080), [ADR-0001](adr-0001-trust-boundaries.md) and [manifest protocol](manifest-v1.md).

## Goal and boundary

PLUGIN-02 implements the smallest useful **typed Rust authorizer** atop the existing manifest vocabulary, without Cedar, Biscuit, WASM, a public plugin manager, a new database or a network library.

The trusted host owns the broker. A host installation step first validates a candidate package and artifact digest, registers a plugin instance scoped to **one vault**, and only after explicit consent creates actual grants. A manifest listing permissions **does not** create grants. An installed instance starts with an empty grant set, so **all host operations deny by default**.

**Important stage limitation:** \`register_host_package\` accepts a digest supplied by the trusted host; it does not cryptographically authenticate that digest, verify a publisher, or install a package. **Host code must never call this function directly based on guest input**. The GUI/installer verification and consent UX are PLUGIN-11/12/19.

## Implemented conceptual types and their meaning

- \`VaultId\`, \`NoteId\`: strongly typed host-provided identities, not serialized to guests. The temporary plugin vault ID is distinct from the sync-engine's distributed identity; actual integration must map these safely.
- \`PluginInstanceHandle\`: random in-memory unexported handle minted by the broker, bound to one registered package/digest and vault. \`PluginIdentity\` also retains the host-bound plugin ID, publisher namespace and package digest for future consent/audit; a manifest's publisher claim is not proof of identity. A guess of an unknown instance cannot authorize access.
- \`HostContext\`: current vault and current note supplied by the host, not by plugin JSON.
- \`NoteResource\`: logical note ID/path from the host index; **not** an OS file descriptor or proof of path confinement.
- \`GrantScope\`: Action, CurrentNote, SelectedNotes, Folder, WholeVault, NetworkHost. Scope classes must match the operation; no network grant authorizes notes, and no notes grant authorizes a network destination.
- \`Resource\`: Ui, Note, Network, HostAction. A \`NotesSearch\` permission is evaluated for **each proposed hit** (including snippets/paths/tags). PLUGIN-07 must ensure the search adapter never leaks unfiltered intermediate results.
- \`AuthorizationLease\`: a snapshot of instance, context, requested operation, resource, grant revision and package digest. Revalidation rejects a stale lease after revocation, new grants, context switch or unloading.

## Mandatory host-side flow

1. Read the valid \`PluginManifest\`; **do not grant requested permissions**.
2. A trusted package-verification step calls \`broker.register_host_package(manifest, digest, vault)\`. There is no guest-exposed registration operation.
3. A trusted consent handler calls \`grant_host_approved(handle, capability, scope)\`. Broker rejects undeclared capability, invalid resource scope, or network host missing from the package's declared list.
4. A host façade resolves the actual vault/document/network identity, then calls \`broker.authorize(handle, context, capability, resource)\`.
5. After asynchronous operations and before commit/network side effects, call \`broker.revalidate_lease(lease, live_host_context)\` and use an atomic downstream operation. The in-memory broker alone cannot prevent TOCTOU at filesystem or network layer.
6. A host revoke/disable removes all grants or unregisters the instance, invalidating old leases.

## Safeguards and explicit non-goals

- **No requestable system.exec, FFI, raw filesystem, env, arbitrary TCP/UDP**, or implicit AI access to document text; the manifest enum remains closed.
- Per-resource matching: folder \`A\` does not include \`AB\`; CurrentNote requires host current note ID; SelectedNotes uses IDs, not filename prefixes.
- Relative path lexical validation rejects absolute paths, \`..\`, \`.\`, backslashes, drive colon, control characters and empty segments. **Not yet protection against symlink/junction/hardlink races, Windows path identity or actual filesystem access**; that requires PLUGIN-04.
- Network host grants are exact lower-case DNS names declared by the package. **No actual sockets**, SSRF protection, DNS resolution or redirect checks exist here; PLUGIN-15 owns network safety.
- Grant state is **in memory only** and does not survive restart. PLUGIN-03 adds SQLite, consent audit and monotonic revocation persistence.
- No filesystem, search, event or UI code path is wired to the broker. PLUGIN-04/07/09/10/15/16 explicitly own those façades and their negative tests.
- \`revalidate_lease\` is a **pre-commit authorization check**, not a transaction lock. The eventual mutation must pair it with the DocumentHub/File Gate atomicity and safety interlocks.
- Treat compromised trusted host code as outside this broker's sandbox; the plugin runtime must never have the ability to invoke host administration methods.

## Tests and acceptance

From \`aquilum-app/\`:

\`\`\`text
cargo test -p aquilum-core --locked plugins::broker_tests
cargo test --workspace --locked
cargo test -p aquilum-core --features sync-engine --locked
\`\`\`

Thirteen negative/matrix tests cover default deny, wrong vault and instance, CurrentNote, SelectedNotes and search hit filtering expectation, segment-aware Folder, WholeVault, invalid action-scope pairs, undeclared capability, per-host network grants, invalid paths, revocation and stale leases, duplicate grant, cross-vault table matrix and independent package instances. No external test crate added.

Required \`Main PR gate\` remains the single protected main check; no new workflow or dependency. Production sandbox tests must be added later before external plugin code is runnable.

## Product contribution

Builds a small, reviewable reusable permission decision layer that can be shared by the future native GUI, File Gate adapter, read/search API, agent bridge, and network integrations. It prevents the harmful assumption that a plugin is trusted just because it declares a capability or is installed.
