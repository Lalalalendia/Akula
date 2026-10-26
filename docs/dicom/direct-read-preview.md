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
