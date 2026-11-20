# VIDEO-TOOLS-01q — Windows V1 executable license evidence (NON-APPROVAL)

**Status: REVIEW REQUIRED. VIDEO-18 RELEASE HOLD remains active.** This
corrects the first-order mismatch between the license of the yt-dlp
*source repository* and the license of the **actual pinned Windows EXE**.
It does **not** accept a distributable package or provide legal advice.

## Verified at immutable upstream tags (2026-11-20)

| V1 runtime | Exact source / upstream asset | Verified upstream licensing evidence | Unresolved |
| --- | --- | --- | --- |
| `akula-video-mux.exe` | Akula exact PR HEAD, `labs/video-lite/Cargo.lock` | Local crate declares `AGPL-3.0-only`; existing VIDEO-18b-09 enumerates Windows Cargo dependency closure, **not** a linked PE SBOM | Actual linked/native constituents, source distribution, transitive notices, AGPL obligations |
| `akula-yt-dlp.exe` | `yt-dlp/yt-dlp@f264a6cbd1f630894da0f368df00f8aae50140e0`, tag `2026.10.26`; release asset SHA-256 `f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d`, size 23,575,744 bytes | Git source LICENSE = **Unlicense**. But the [tagged README Licensing section](https://github.com/yt-dlp/yt-dlp/blob/2026.10.26/README.md#licensing) explicitly states that the **PyInstaller-bundled executables** include GPLv3+ code and their combined work is **GPLv3+**. The [tag-specific THIRD_PARTY_LICENSES.txt](https://github.com/yt-dlp/yt-dlp/blob/2026.10.26/THIRD_PARTY_LICENSES.txt) enumerates additional license texts | Confirm actual bundled PyInstaller constituents, GPLv3+ obligations, correspondence and availability of complete source, third-party notices, distribution procedure |
| `akula-quickjs.exe` | `quickjs-ng/quickjs@d7215b0ad3fc2c562e8317fd9bca058fd4863804`, tag `v0.18.0`; asset SHA-256 `ff03398d74ea6e6bd3a8e58058f7003225abcac229d7d57775b7aaae812a49c7`, size 1,873,152 bytes | [Tag-specific MIT LICENSE](https://github.com/quickjs-ng/quickjs/blob/v0.18.0/LICENSE) contains copyright attribution and notice-preservation requirement | Include correct copyright and permission text in releasable notices; check PE actual contents |

**Correction:** `docs/release/video18b04-five-exe-allowlist.json`
previously assigned `license="Unlicense"` to `akula-yt-dlp.exe`.
This was the license of the project's own source, **not** its Windows
PyInstaller distribution. It is now recorded as
`license="GPL-3.0-or-later"` for the combined Windows EXE, with
`source_license="Unlicense"` separately. `license_status` remains
`REVIEW_BUNDLED_DEPENDENCIES`; this correction is **not** a finding
of compliance or incompatibility.

## Evidence pipeline (no new multi-platform dependency inventory)

The existing VIDEO-18b-09 **PR-only** metadata workflow reuses the
already accepted Cargo license triage, and now also checks out the two
immutable upstream commit SHAs, verifies the Git blob SHA-1 of four
tagged license/README documents, requires the actual wording regarding
GPLv3+ PyInstaller bundles and MIT notices, and emits a one-day
`video-tools-01q-license-HOLD.json`. No license text, EXE, artifact
bundle or private signing key is uploaded. A metadata match does not
prove which Python/C/native modules are embedded in the shipped EXE.

The source-only report retains:
`release_admitted=false`, `publisher_key_approved=false`,
`legal_approval=false`, `license_text_review_complete=false`,
`binary_contents_sbom_verified=false`, and
`third_party_notices_packaged=false`.

## Owner / independent legal review — not automated by CI

1. Review the exact **GPLv3+** obligations applicable to distributing the
   pinned yt-dlp PyInstaller EXE alongside separately packaged Akula and
   QuickJS, including actual covered modules, appropriate corresponding
   source and delivery/notice method. Do not infer that Unlicense of the
   yt-dlp Git tree covers the frozen Windows binary.
2. Review `THIRD_PARTY_LICENSES.txt` at the immutable tag, record
   copyright/attribution requirements and decide where notices and
   necessary source information can be distributed. The current six-file
   lab installer must **not** be silently declared releasable just because
   the license file exists upstream. A separate compliant notice/source
   delivery design is required before release.
3. Include the QuickJS MIT copyright and permission notice with any
   approved distributable package. Independently check third-party
   linked/static/native modules in its Windows EXE, if any.
4. Review Rust mux `AGPL-3.0-only`, selected Cargo dependency license
   texts, actual native/linked binary composition and source distribution
   obligations. Reuse VIDEO-18b-09; it is a conservative metadata
   inventory, **not** a release-ready linked binary SBOM.
5. Keep the Video Tools public publisher key unapproved until exact
   source, bundled binaries, legal notices and reproducible/signable
   packaging have an explicit owner decision. Then separately test
   the actual signed installer and user-mode Windows 10/11 flows.

**Non-goals:** no public release, no NSIS/updater changes, no production
runtime enablement, no live YouTube download, no licensing self-approval.
`APPROVED_VIDEO_TOOLS_KEY=""`,
`VIDEO_FIVE_CHANNEL_READY=false`, and **VIDEO-18 RELEASE HOLD** stay on.
