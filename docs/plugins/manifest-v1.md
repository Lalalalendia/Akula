# PLUGIN-01 — Akula plugin manifest and protocol v1

**Status: proposed public contract, implemented only as inert Rust data parsing.** This document is the semantic contract for the serializer in \`aquilum-app/core/src/plugins/manifest.rs\`. It does **not** install, load, execute or authorize plugins. The first trusted installation manager and sandbox belong to later PLUGIN stages.

## Manifest v1

See [valid-v1.json](fixtures/valid-v1.json) and negative golden fixtures in the same directory. Public Rust entrypoint: \`aquilum_core::plugins::PluginManifest::parse_json(input)\`. JSON unknown fields are rejected to avoid accidentally ignoring security-relevant declarations.

- \`schema: 1\` — required exact package-schema version.
- \`id\` — lower-case ASCII reverse-DNS identifier (three or more segments), prefixed by \`publisher + "."\`; stable plugin key.
- \`publisher\` — lower-case ASCII reverse-DNS namespace. **A claim from untrusted JSON, not verified identity.** Trust-bound publisher binding is the installer's responsibility.
- \`display_name\` — human-friendly text, max 120 bytes; control characters rejected.
- \`version\` — strict unsigned \`MAJOR.MINOR.PATCH\` with no leading zeroes; pre-release/build-metadata grammar explicitly deferred (requires version-policy ADR before public SDK).
- \`api: {family:"akula.plugin",min:1,max:1}\` — inclusive supported host API range. The advertised host ABI name is \`akula.plugin/1\`, represented by family and integer level; no runtime-specific linkage.
- \`runtime\` — \`wasm-component\` or \`wasm-module\` **as a package declaration only**. Actual runtime choice/ABI remains PLUGIN-05.
- \`entry\` — canonical relative lower-case ASCII path ending \`.wasm\` within package; never filesystem, UNC, network URL or traversal. Package extraction safety is not provided by this parser.
- \`platforms\` — nonempty unique subset of \`windows\`, \`linux\`, \`macos\`.
- \`dependencies\` — optional list of reverse-DNS plugin IDs and minimum numeric versions. Cycle/resolution checking belongs to installer PLUGIN-12; self/duplicate dependencies are rejected now.
- \`permissions.required\` / \`permissions.optional\` — closed capability enum (UI, notes, attachments, events, network, clipboard, AI). No unknown/duplicated values; no \`system.exec\`, raw \`std::fs\`, FFI, environment or arbitrary sockets.
- \`network_domains\` — unique lower-case ASCII hostnames without wildcards, IP literals or localhost/intranet suffixes. A declared \`network.fetch\` requires at least one declared domain, and domains without the capability are rejected. This is **declaration validation**, not runtime SSRF protection.

## Rejection and forward-compatibility policy

- Manifests over 64 KiB fail before deserialization.
- Unknown JSON fields, unknown permissions, malformed paths/identities/versions, duplicate declarations and unsupported host API levels fail closed.
- A manifest serialized back to JSON must parse to the same typed structure (round-trip).
- Future API levels and manifests are not silently accepted and must never grant new permissions through an older host.
- A host may read/inspect a rejected manifest only in a separate inert inspection mode later; it may not execute or silently rewrite it.

## Distinction between declared and granted privileges

This code has **no API to turn permissions into grants**. Actual rights will be bound to verified publisher/bundle, plugin instance, vault, scope and user approval in PLUGIN-02/03. Network domains alone are not an allowlist against DNS rebinding or SSRF: PLUGIN-15 enforces destinations and redirects.

## Test and gate

- \`cargo test -p aquilum-core --locked plugins::tests\` verifies the golden/negative fixtures.
- \`cargo test --workspace --locked\` and existing \`cargo test -p aquilum-core --features sync-engine --locked\` remain required through the single existing Main PR gate.
- No Cargo dependencies are added (uses existing serde/serde_json and std only).
- Only documentation, typed inert parsing and unit tests change: **no user-visible plugin loading or marketplace**.

Source of truth for trust boundaries: [ADR-0001](adr-0001-trust-boundaries.md) after PLUGIN-00 is merged; master plan [Notion](https://app.notion.com/p/3fc32a84beec8132bd9cd127ba517080).
