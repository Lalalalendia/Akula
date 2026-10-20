# Akula

**Akula is a native, local-first workspace for our own files, documents, and knowledge.** It is developed in Rust, with a desktop interface built on Masonry/winit and no WebView dependency.

**Project direction:** we are building Akula primarily for our own workflows. Feedback from other users is useful research, not a feature checklist or a commitment to ship everything other note-taking apps provide.

**Status (19 October 2026): active development, not a published desktop or sync release.** There are currently no downloadable Akula builds on [this repository's Releases page](https://github.com/Lalalalendia/Akula/releases). Do not mistake [Aquilum's releases](https://github.com/Freaction/Aquilum/releases) for Akula releases.

[Русский](README.ru.md) · [Source repository](https://github.com/Lalalalendia/Akula) · [Development notes](DEVELOPMENT.md) · [Original Aquilum](https://github.com/Freaction/Aquilum)

## Principles

- **Our files remain primary.** Markdown notes and attachments live in regular folders and must remain usable without Akula, an account, or a proprietary cloud service.
- **Local work comes first.** Opening, editing, searching, and reading local documents should not depend on a running sync service or a plugin runtime.
- **Do not lose data silently.** Conflicts, failed writes, interrupted operations, and external changes must be explicit and recoverable rather than silently overwritten.
- **Keep the normal editing path simple.** Advanced tools should not get in the way of an ordinary Markdown document.
- **Choose features based on our actual tasks.** Other projects and user comments are sources of ideas and tests, not roadmaps to copy.

## What is in the repository

| Area | Current boundary |
| --- | --- |
| Desktop and Markdown | Native editor, local files, tabs, search, links, history, and document navigation exist in code. Installed-product acceptance is still separate. |
| Books and reference material | Local reading and note-taking components are present; format support and the complete UX must be verified on real builds. |
| File safety | File Gate, revision/history, conflict and recovery mechanisms exist and continue to receive cross-process, crash, and OS-specific hardening. |
| S3/MinIO synchronization | Transport, manifests, journaling, and guarded operations are under development. **A complete, user-ready two-device sync flow is not yet established.** |
| Extensions | Initial permission and host contracts exist. A general-purpose, user-ready plugin runtime is not available. |
| Canvas and Bases | Initial structured-file parsing and read-only integration exist; a full editing experience is not available. |
| Web, mobile, collaboration, Nextcloud | Research or future options, **not shipped features**. P2P experiments are likewise not a supported sync product. |

These are engineering boundaries, not a claim that every component is production-ready. For current progress, inspect [pull requests](https://github.com/Lalalalendia/Akula/pulls), CI, and source code rather than inferring readiness from a merged PR.

## Build from source

Install a current stable Rust toolchain and the platform build dependencies. Some detailed instructions in [DEVELOPMENT.md](DEVELOPMENT.md) and [aquilum-app/README.md](aquilum-app/README.md) are inherited from upstream and may still describe Aquilum packaging; **their release/download instructions are not Akula release instructions**.

```sh
cd aquilum-app
cargo build -p aquilum-native
cargo test --workspace
cargo run -p aquilum-native
```

On Windows, the inherited `aquilum-app/native/run.bat` provides a release-build-and-run workflow. For active development, use a separate test vault and keep a backup of important files.

## Platform policy

**Active:** Windows and Linux. **macOS:** frozen / P2 (not a supported Akula release target). We have paused macOS development, automatic platform QA, packaging and updater work while retaining its source for a separately approved resumption. A deferred macOS result is **not** a successful test; existing Windows/Linux and data-safety gates remain mandatory. See [MAC-HOLD-01](docs/platforms/MAC-HOLD-01.md).

## Releases, installation, and updates

Akula has **not yet published its own installer, signed update channel, or public release**. The source tree includes packaging and installation work in progress, but that does not make an installable Akula release available. In particular, do not follow an old README link to install upstream Aquilum by accident.

Some internal identifiers still use the upstream names `aquilum-core`, `aquilum-native`, `Aquilum.app`, and Aquilum package/application IDs. They are transitional implementation details, **not evidence that a released Akula installer or safe migration path exists**. Avoid installing experimental builds over an existing Aquilum installation.

Before declaring a release, the project must pass installed-app checks for file protection and recovery, update/rollback, and the relevant end-to-end workflows on supported operating systems. Two-client synchronization has its own separate acceptance gate.

## Source layout

- `aquilum-app/core/` — local files, history, search, sync building blocks, and shared logic.
- `aquilum-app/native/` — native desktop interface and renderer.
- `docs/` — technical decisions and test notes.
- `scripts/` — development, packaging, and installation scripts; review their assumptions before running them.

## Origin and license

Akula is an independent fork of [Freaction/Aquilum](https://github.com/Freaction/Aquilum), originally developed by Dmitriy Chaplinskiy. We retain attribution to the original project and its contributors. The source code is distributed under [GNU AGPL-3.0-only](LICENSE); original copyright and license notices remain applicable.

The name and logo **Aquilum** identify the upstream project. They are not Akula branding. Historical filenames and screenshots inherited from upstream may still contain that name while migration is in progress.
