# VIDEO-TOOLS-01s — embedded PyInstaller third-party notices vs upstream source

**NONRELEASE evidence. VIDEO-18 RELEASE HOLD.** Extends the accepted
VIDEO-TOOLS-01r actual yt-dlp.exe CArchive/PYZ metadata without a new
downloader, executable, product change or installer.

## Verified inputs

- Exact public Windows PyInstaller asset: yt-dlp.exe 2026.10.26,
  SHA-256 f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d.
- Actual 01r archive: **154 CArchive entries and 1,574 PYZ module names**.
- Pinned source tag commit: f264a6cbd1f630894da0f368df00f8aae50140e0.
- Pinned source THIRD_PARTY_LICENSES.txt Git blob:
  88f9872c66b456703e91ca32db4041e1ec47258d.

The actual EXE contains a compressed THIRD_PARTY_LICENSES.txt member with
**248,243** recorded unpacked bytes. The Git-tagged source file contains
**243,550** UTF-8 bytes: a difference of **4,693** bytes.

That size delta proves the two cannot be byte-identical, but it does not
identify which notice statements changed. The 01s CI reads **only** that
embedded notice text in memory (no DLL or Python code extraction or execution),
checks source commit and Git blob identity, and compares the two
full notices. CI retains only one-day JSON with hashes, lengths,
section titles, label differences and source-review queue; **no EXE and no
license-text bodies are uploaded**.

## Family-level review candidates, not audited packages

From the 01r real PyInstaller archive, top-level module roots give likely
families, not exact dependency licensing or ownership. For example:
- yt_dlp (1,041 modules), Cryptodome (68), websockets (52), mutagen (48),
  urllib3 (36), curl_cffi (20), requests (19), cffi (16), plus other
  roots and stdlib. Both Python modules and native libraries must be reviewed.
- Two embedded .dist-info paths establish exact **names in package metadata**
  for curl_cffi **0.17.0** and websockets **17.3**, but they do not prove
  every source, DLL or transitive dependency version.
- Upstream THIRD_PARTY_LICENSES lists sections including PyCryptodome,
  mutagen (GPL-2.0-or-later), websockets, curl_cffi, OpenSSL 3.0+ and Python.
  A section being in upstream text is not evidence it is included in this
  Windows EXE; a matching name is only a review candidate.
- Module roots not mapped to a section, including potential Python
  standard-library modules, remain an explicit **review queue**.
  No unknown is silently marked compliant or infringing.

The report must retain:
release_admitted=false, legal_approval=false,
license_compatibility_review_complete=false,
complete_corresponding_source_available=false,
notices_packaging_approved=false and publisher_signing_authorized=false.

## P0 before any package distribution

1. Read the actual byte-level notice differences identified by 01s and
   reconcile every material addition/removal or changed license label.
   A 4,693-byte size difference alone does not establish a violation.
2. Establish exact PyInstaller bundled versions, native DLL provenance,
   Python extension modules and JavaScript solver source. Versioned
   .dist-info directory names alone are insufficient.
3. Obtain or assemble full GPLv3+ **corresponding source** for this exact
   Windows binary and verify license/attribution requirements independently.
   GPLv3+ applies to the PyInstaller combined EXE, while upstream
   yt-dlp source has Unlicense. Review whether tagged upstream source alone
   is sufficient before choosing a source-distribution mechanism.
4. Reconcile MIT QuickJS notices and AGPL Rust mux source/third-party
   notices separately; do not infer their status from yt-dlp inventory.
5. Only after complete source-and-notices review and owner approval,
   design the publisher-signed optional Windows 10/11 installer, independent
   distribution and manual update flow. This stage grants **no legal approval**
   or release admission.

**Explicit non-goals:** no new updater, no change to the existing six-file
signed-manifest NSIS lab, no public release, no end-user downloads,
no activation of APPROVED_VIDEO_TOOLS_KEY, no VIDEO-18 release-HOLD removal.
This is **not a complete SBOM** and is never a legal determination.
