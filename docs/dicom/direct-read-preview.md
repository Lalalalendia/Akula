# Akula DICOM: direct read-only preview without import

**Status:** experimental feature branch `feat/dicom-readonly-preview-20261026`; not part of the default application, approved Windows release, or a clinical viewer. Do not publish this build through the signed Akula release channel.

## User path

1. Start a **locally built** Akula with the `dicom-preview` feature enabled. Opening a normal Akula workspace is not required.
2. On the no-workspace screen select **DICOM: открыть папку без импорта** for an optical disc or directory, or **DICOM: открыть ZIP или файл** for a ZIP, `.dcm`, extensionless image, or DICOMDIR file.
3. Navigate images using Left/Right or the mouse wheel. PageUp/PageDown changes series. A media-read error is reported on the page; the last successfully rendered image remains visible, and navigation can be retried after the medium reconnects.
4. Select a new source with the same buttons. Source content is read in place; no import, no extraction of ZIP entries to the filesystem, and no copy into an Akula vault.

## Reproducible developer launch (Windows PowerShell)

From the checkout of this **feature branch**:

```powershell
cargo build --release --locked --manifest-path aquilum-app/Cargo.toml -p aquilum-native --features installed,dicom-preview
$exe = "aquilum-app/target/release/aquilum-native.exe"
# Replace $exe with the binary produced by CARGO_TARGET_DIR if your build overrides it.
& $exe --data-dir "$env:TEMP\akula-dicom-test" --open-dicom "F:\"
# Or: --open-dicom "F:\DICOMDIR"
# Or: --open-dicom "D:\studies\case.zip"
```

`--open-dicom` is an explicit opt-in for a particular external source. It does **not** create a workspace or turn on bulk import. Passing both `--open` and `--open-dicom` together is rejected. The normal build (without `dicom-preview`) does not accept this experimental switch.

For automated synthetic fixtures only:

```powershell
python -m pip install pydicom==3.0.1 numpy==2.2.6
python scripts/dicom-native-smoke/generate.py scripts/dicom-native-smoke/fixture
```

These tests use generated data, not actual patient records. Do **not** upload medical discs, DICOM metadata or screenshots containing patient names into CI, Issues, or public artifacts.

## Supported and bounded

- Native DICOMDIR file sets on a CD/DVD or hard disk, including extensionless DICOM images; ordinary directories without DICOMDIR; individual Part 10 DICOM files; ZIP archives (STORE/DEFLATE) read without extraction. RAR/7z and password-protected archives are **not supported**.
- Up to 512 image references, 4,096 frames, 4,194,304 pixels per frame, 128 MiB per image, 2 GiB ZIP file, 512 MiB total indexed DICOM data in ZIP, and 10,000 directory entries. Larger real-world studies can produce an explicit limit error rather than being partially imported.
- Read-only preview. This is **not for medical diagnosis**, clinical decisions, annotations, or patient information management.
- No automatic repair of physically damaged discs. Some USB/DVD drivers may block inside the operating system; a worker prevents most decoding from running on the UI thread, but cannot guarantee prompt cancellation of a stuck kernel read. Unplugging the reader during a write by another application may damage data even though Akula does not write.

## Validation evidence

- [Linux source-model + native CI](https://github.com/Lalalalendia/Akula/actions/runs/40393286674) — SUCCESS on the direct reader and protected context menu.
- [Windows native CI](https://github.com/Lalalalendia/Akula/actions/runs/40393286476) — SUCCESS.
- [Windows no-vault GUI launch](https://github.com/Lalalalendia/Akula/actions/runs/40393556511) — real release executable launched without a workspace; both DICOM source buttons found via UI Automation.
- [Windows source-loss simulation](https://github.com/Lalalalendia/Akula/actions/runs/40392477029) — ZIP/folder removed and restored during navigation, then resumed successfully.
- [End-to-end selected source GUI](https://github.com/Lalalalendia/Akula/actions/runs/40395011135) — added to CI; check **run conclusion** before claiming a successful image preview.
- **Not verified:** a physical, intermittently disconnecting optical drive in a real Windows desktop session, some compressed transfer syntaxes, 7z/RAR, clinical conformance.

Use temporary local data directories for preview tests. Never replace the signed installed release with the unsigned development binary.

## Validation: Windows GUI end-to-end (2026-10-27)

- [Windows GUI run 40410083550](https://github.com/Lalalalendia/Akula/actions/runs/40410083550) — **PASS** on commit `5783120`, running the real `aquilum-native.exe` without an Akula workspace. Both open-source buttons are accessible in Windows.
- **For both** a synthetic ZIP and an ordinary folder: first slice rendered; actual focused Windows key events changed slices `1/3 -> 2/3 -> 3/3 -> 2/3`; temporarily moving the source away produced a read error while preserving the last valid slice, and restoring it allowed navigation to `3/3` again. The ZIP source's SHA-256 remained unchanged. Logs contain `AKULA_DICOM_GUI_NAVIGATION_PASS`, `AKULA_DICOM_GUI_MEDIA_LOSS_PASS`, and `AKULA_DICOM_GUI_MEDIA_RECOVERY_PASS` for each source.
- [Linux DICOM run 40409944086](https://github.com/Lalalalendia/Akula/actions/runs/40409944086) — **PASS**, source model, DICOMDIR, ZIP and native integration.
- [Application checks 40410089831](https://github.com/Lalalalendia/Akula/actions/runs/40410089831) — the required Rust/policy job passed, including the no-growth check (**0 violations**). Check the whole workflow conclusion before declaring the entire PR green.
- Portable unsigned Windows test archive: [GitHub Actions artifacts](https://github.com/Lalalalendia/Akula/actions/runs/40410083550#artifacts), `akula-dicom-unsigned-test-40410083550`. Verified EXE SHA-256: `02B980A6927B684953FAB2B0DC555BB405D6AD3370616C810C098DE350B5306B`. Artifacts are temporary; build from the branch after expiration.

### Physical optical disc acceptance — local Windows only

This step is **not covered by GitHub Actions**. Test only on a local PC with an inserted CD/DVD and the unsigned experimental preview; never substitute it for a signed installed Akula application.

1. Extract the Windows archive, confirm that the EXE SHA-256 matches the supplied `SHA256.txt`, and start `aquilum-native-dicom-preview.exe`. On the no-workspace page choose **DICOM: open folder without import**, then select the CD/DVD root (for example, `F:\\`). Alternatively, from PowerShell in the extracted folder run: `& .\\aquilum-native-dicom-preview.exe --data-dir \"$env:TEMP\\akula-dicom-optical-test\" --open-dicom \"F:\\\"` with the correct drive letter.
2. Confirm the first image appears and the page reports the current slice and series. If there are multiple slices, use the left/right arrow keys; use PageUp/PageDown only when multiple series are present. A one-image disc does not have to advance.
3. Check that the app remains responsive while the optical drive spins, and that an inaccessible/unsupported medium shows an error instead of silently importing anything.
4. Close the viewer before physically ejecting the disc, reinsert the same disc, and reopen it. Do not force-eject a busy disc and do not unplug an active drive merely to reproduce the CI simulation.
5. Record **only** non-medical results: opening PASS/FAIL, reported series/slice counts, navigation PASS/FAIL or single-slice, behavior on reopen, and generic failure category. **Do not** upload DICOM files, patient identifiers, screenshots, disc metadata, or real medical filenames to GitHub, Notion, chat, or CI.

The real-drive test remains pending until performed on a physical Windows machine; synthetic ZIP/folder success is not clinical validation.
