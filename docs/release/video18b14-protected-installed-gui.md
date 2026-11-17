# VIDEO-18b-14 — physical GUI from a genuinely protected five-EXE Windows NSIS

Status: **NON-RELEASE / TEST ONLY**.

VIDEO-17 verified the physical Windows GUI click, locally downloaded MP4, restart, decoded frame and uninstall, but installed the application into per-user `LocalAppData`. VIDEO-18b-11 verified five real PE binaries in a protected `Program Files` NSIS and denied 11 unauthorized standard-user mutations, but did not open the GUI from there.

This independent lab connects **those two acceptance boundaries** without touching the public two-EXE signed installer or the updater.

## Windows PR-only protocol

1. Check out exact PR head, compile pinned Shiva, tested Rust mux and experimental video-enabled Akula, download size+SHA-pinned upstream yt-dlp and QuickJS; reuse the existing five-EXE custody witness.
2. Create only an ephemeral GUID-named five-PE staging set under the hosted runner temp, checking real PE/MZ signatures, exact hashes and source provenance. Compile and install the isolated elevated unsigned `scripts/video18b-protected-five-real.nsi` into `Program Files\AkulaVideo18bRealNsisLab-<GUID>`; no product uninstall registry or signed release identity.
3. Check **all five installed hashes**, unaliased protected root, real uninstaller.
4. Start the actual protected installed Akula via physical Windows `SendInput` GUI pointer, not a simulated widget call. In the isolated video feature only, serve a locally generated synthetic H.264/AAC DASH manifest on `127.0.0.1`; require hover, Download click, local MP4, restart, Play, decoded frame and a local-only persisted note.
5. Rehash all installed executables after GUI use, reject any MP4 or segments copied to the note vault. Uninstall the protected app; verify private local MP4/note remains byte-for-byte unchanged.
6. Upload only a synthetic MP4 output (not the installer or EXEs), independently decode both tracks with FFmpeg on Linux.

## Required evidence

- [ ] Exact-head source and mutation tests, including refusal of arbitrary/per-user paths and missing fifth PE, pass.
- [ ] Windows 2022 actually builds and installs the isolated real five-PE NSIS under `Program Files` and verifies exact hashes.
- [ ] Actual GUI pointer Download, local media, restart, Play and a decoded frame from the protected executable pass.
- [ ] A separate Linux job decodes the saved H.264 and AAC frames; uninstaller preserves local data.
- [ ] Application CI and no-release boundaries pass; PR merged only after all required checks.

This does not prove the real signed product installer, privilege separation of GUI execution from elevated setup, the safe atomic five-EXE updater, crash/power-loss recovery or a real Windows 10/11 user's desktop. Standard-user ACL tamper protection is separately tested by VIDEO-18b-11, not replaced by this GUI proof. The VIDEO-18 release stays blocked.

Relevant: [VIDEO-18b-11 protected NSIS PR #993](https://github.com/Lalalalendia/Akula/pull/993), [VIDEO-18b-12 updater HOLD PR #994](https://github.com/Lalalalendia/Akula/pull/994), [VIDEO-18b-13 crash model PR #995](https://github.com/Lalalalendia/Akula/pull/995).
