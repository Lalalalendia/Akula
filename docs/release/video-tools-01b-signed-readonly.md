# VIDEO-TOOLS-01b — read-only Minisign and component hash verification

**Status: non-release security substrate.** The separately signed Akula Video Tools package remains unapproved. The installed stock Akula continues to use the existing two-EXE installer; the integrated VIDEO-18 five-EXE auto-updater remains **HOLD**.

## Accepted shape for a future package

A fixed protected Windows 10/11 Video Tools installation contains `manifest.json`, `manifest.minisig`, precisely three executables (`akula-video-mux.exe`, `akula-yt-dlp.exe`, `akula-quickjs.exe`), and optionally its own `uninstall.exe`. It must **never** contain `akula-app.exe` or `shiva-pdf-preview.exe`. The Video Tools installer must have a separate identity and must not change or uninstall stock Akula.

The exact manifest bytes have a separately publisher-signed Minisign signature (no reused stock Akula key), with these fields and **no unknown fields**:

```json
{
  "schema": 1,
  "product": "Akula Video Tools",
  "version": "0.1.0",
  "platform": "windows-x86_64",
  "source_commit": "<40 lower-case hex digits>",
  "cargo_lock_sha256": "<64 lower-case hex digits>",
  "binaries": [
    {"name": "akula-video-mux.exe", "sha256": "<64 lower-case hex digits>", "size": 123456},
    {"name": "akula-yt-dlp.exe", "sha256": "<64 lower-case hex digits>", "size": 23575744},
    {"name": "akula-quickjs.exe", "sha256": "<64 lower-case hex digits>", "size": 1873152}
  ]
}
```

Values above are illustrative, **not** actual signed distributable data. The first package version floor is `0.1.0`; downgrade, leading zeros, overflow, duplicate/unknown fields, filenames and path traversal are rejected. The three individual sizes and SHA-256 digests must match the actual files. Only canonical names are read, so there is no signed arbitrary executable path or PATH/environment fallback.

## Security ordering

1. Require a nonempty **owner-approved Video Tools publisher public key**. The source explicitly keeps `APPROVED_VIDEO_TOOLS_KEY=""`; no CI-generated temporary key may activate release trust.
2. Authenticate the exact, bounded manifest bytes with Minisign before trusting product, version, source, file names or hashes.
3. Require that the exact on-disk manifest and signature match the authenticated bytes; reject links/reparse points.
4. Validate the exact file list and bounded lengths, then stream SHA-256 of each of the three component files, rejecting truncated or mutated bytes, extra EXEs and non-regular files.
5. Return **content attestation only**. This is not permission to execute a file: no executable path is exposed and `shipping_admission_enabled()` remains `false`.

The real-crypto acceptance uses newly generated ephemeral Minisign signing and incorrect keys in GitHub CI; the three fixture files are **synthetic MZ-like bytes** for checking the signature and hash protocol. Their success does not prove that Windows would load the binaries or that their signatures are publisher trusted.

## Remaining VIDEO-TOOLS-01b gates before launch

- Confirm publisher signing trust root and out-of-band key custody; bind to the distinct package product rather than inheriting the stock updater's authority.
- Use Windows OS-known Program Files path with verified ACL, no junction/reparse parents and installed package identity; test with a non-admin token and a real signed package.
- Bind source/lock provenance, actual PE architecture/version and installer receipts into the activation decision; reverify handles under any execution boundary to avoid TOCTOU.
- Wire the verified three-tool set into the native download path without arbitrary environment/PATH fallback. Add UI feedback for absent/untrusted/outdated tools; do not promise in-box downloading until this passes.
- Full Windows 10 and 11 acceptance, offline playback and owner distribution approval; automatic updater remains out of scope.

**No release, no deployment, no signed installation, no key rotation and no VIDEO-18 HOLD removal in this PR.**
