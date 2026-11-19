# VIDEO-TOOLS-01n — manual installer replacement refusal; owner-signing hold

**State:** non-release Windows CI only. `RELEASE_READY=false`.
**Scope:** close a real manual-installation safety gap while a separate Video Tools
publisher key and legal evidence are still not approved.

## What the real protected Windows test proves

The existing VIDEO-TOOLS-01j six-file separate NSIS laboratory is reused.
No second updater, new runner, public signing service, online installer or
production path is introduced.

1. Source-built mux plus SHA-256 pinned yt-dlp/QuickJS PE and the real
   ephemeral Minisign-signed manifest are installed by an elevated test NSIS
   under the OS-known `Program Files\Akula Video Tools` directory.
2. While those six protected files exist, a *second silent invocation of the
   same installer* must exit with explicit error level **77** before writing.
   Native Akula verifies ownership/ACL, exact file identity and the genuine
   signed manifest; all **six SHA-256 hashes must remain unchanged**.
3. After all existing 01j negative cases, the real uninstaller must remove
   only its six package files and preserve an unrelated note. A deliberate
   **same-version** manual reinstall then passes the same genuine native
   cryptographic, PE hash and OS ACL checks. Its second uninstaller removes
   the package entirely; native discovery returns `Missing`.
4. A final exact-HEAD CI proof marker is written *only after* both refusal
   and clean reinstall, the existing 01j/01m signing cases and safe cleanup.

The source-mutation test refuses to accept a removed installer abort, a
changed errorlevel, a bypassed SHA comparison, a skipped verification,
an added release privilege or a production trust-key shortcut.

## Not proven, and why VIDEO-18 RELEASE HOLD remains

- There is **no approved publisher key** for Video Tools. A valid CI
  ephemeral Minisign signature does not establish owner approval.
  The stock Akula update signing key MUST NOT silently become this key.
- There is no real signed Windows release installer or independently reviewed
  owner custody/rotation/revocation process. A publisher key can only be
  introduced by a separately reviewed owner decision with out-of-band
  fingerprint comparison, rollback/revocation rules and a new acceptance PR.
- This is an explicit **same-version** reinstall, not a successful upgrade
  with two independently built different versions. A signed manifest label
  without different PE bytes cannot establish a legitimate version floor.
- Windows 10/11 user-mode installation and real publisher-signed uninstall/
  replacement, antivirus/locked-file cases, reboot and power failure remain
  unverified. No production automatic rollback is promised.
- External legal and bundled dependency notices are still marked `REVIEW_`
  in the source allowlist; approval requires an independent human review,
  lockfile/SBOM evidence and a releasable package.
- Production `APPROVED_VIDEO_TOOLS_KEY=""`, `shipping_admission_enabled()`
  returns false, the installed queue refuses download and the standard two-EXE
  Akula installer remains unchanged.

**VIDEO-18 RELEASE HOLD** and `VIDEO_FIVE_CHANNEL_READY=false` stay in
force. This does not approve or perform a release.

## Owner approval handoff — required, not simulated

- [ ] Independently choose and approve a Video Tools publisher public key
      and securely custody its private counterpart outside PR runners.
- [ ] Confirm independent owner decision and exact fingerprint, plus a
      documented key rotation/revocation procedure.
- [ ] Verify legally distributable source, transitive notices and pinned
      `Cargo.lock`/dependency versions for mux, yt-dlp and QuickJS.
- [ ] Build a genuinely versioned package, sign both exact manifest and
      protected installer, independently verify each installed PE and
      Windows Authenticode signer.
- [ ] Test no-overwrite/manual update with two *distinct real* package
      versions on Windows 10 and Windows 11 as an ordinary user, including
      downgrade refusal, locked files, incomplete install and recovery.
- [ ] Only then consider an explicit approval PR and a separate go/no-go
      release decision. No automatic channel for Video Tools V1.
