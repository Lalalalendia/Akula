#!/usr/bin/env python3
"""Generate a synthetic, no-patient-data DICOM FileSet with extensionless images."""
from pathlib import Path
import argparse
import numpy as np
import pydicom
import zipfile
from pydicom.dataset import Dataset, FileMetaDataset
from pydicom.fileset import FileSet
from pydicom.uid import ExplicitVRLittleEndian, SecondaryCaptureImageStorage

def sample(i: int) -> pydicom.FileDataset:
    ds = pydicom.dataset.FileDataset(
        None, {}, file_meta=FileMetaDataset(), preamble=bytes(128))
    ds.file_meta.MediaStorageSOPClassUID = SecondaryCaptureImageStorage
    ds.file_meta.MediaStorageSOPInstanceUID = pydicom.uid.generate_uid(prefix="2.25.")
    ds.file_meta.TransferSyntaxUID = ExplicitVRLittleEndian
    ds.file_meta.ImplementationClassUID = pydicom.uid.PYDICOM_IMPLEMENTATION_UID
    ds.SOPClassUID = SecondaryCaptureImageStorage
    ds.SOPInstanceUID = ds.file_meta.MediaStorageSOPInstanceUID
    ds.PatientID = "SYNTHETIC_NO_PATIENT"
    ds.PatientName = "SYNTHETIC^DATA"
    ds.StudyInstanceUID = "2.25.456789000000001"
    ds.SeriesInstanceUID = "2.25.456789000000002"
    ds.StudyID = "LAB"
    ds.SeriesNumber = 1
    ds.InstanceNumber = i + 1
    ds.Modality = "OT"
    ds.StudyDate = "20261026"
    ds.StudyTime = "120000"
    ds.ContentDate = "20261026"
    ds.ContentTime = "120000"
    ds.Rows = 8
    ds.Columns = 8
    ds.SamplesPerPixel = 1
    ds.PhotometricInterpretation = "MONOCHROME2"
    ds.BitsAllocated = 8
    ds.BitsStored = 8
    ds.HighBit = 7
    ds.PixelRepresentation = 0
    ds.PixelData = (np.arange(64, dtype=np.uint8) + i).tobytes()
    ds.is_little_endian = True
    ds.is_implicit_VR = False
    return ds


def generate_medical_contrasts(directory: Path) -> None:
    """Generate 16-bit signed multi-frame + grayscale-inversion controls."""
    signed = sample(0)
    signed.file_meta.MediaStorageSOPInstanceUID = pydicom.uid.generate_uid(prefix="2.25.")
    signed.SOPInstanceUID = signed.file_meta.MediaStorageSOPInstanceUID
    signed.SeriesInstanceUID = "2.25.456789000000099"
    signed.Rows = 16
    signed.Columns = 16
    signed.NumberOfFrames = "3"
    signed.BitsAllocated = 16
    signed.BitsStored = 16
    signed.HighBit = 15
    signed.PixelRepresentation = 1
    signed.RescaleSlope = "1"
    signed.RescaleIntercept = "-100"
    signed.WindowCenter = "-30"
    signed.WindowWidth = "300"
    base = np.arange(256, dtype=np.int16).reshape(16, 16) - 160
    pixels = np.stack([base + 80 * frame for frame in range(3)])
    signed.PixelData = pixels.astype("<i2").tobytes(order="C")
    signed.save_as(directory / "SIGNED16.dcm", enforce_file_format=True)
    check = pydicom.dcmread(directory / "SIGNED16.dcm").pixel_array
    assert check.shape == (3, 16, 16) and check.dtype.kind == "i"
    assert np.array_equal(check, pixels)

    # Keep the pixels and all rendering metadata identical except inversion.
    for photometric, filename in [("MONOCHROME1", "MONO1.dcm"),
                                  ("MONOCHROME2", "MONO2.dcm")]:
        image = sample(0)
        image.PhotometricInterpretation = photometric
        image.save_as(directory / filename, enforce_file_format=True)
    print("SYNTHETIC_CONTRAST_OK signed16_frames=3 invert_controls=2")

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("directory", type=Path)
    a = parser.parse_args()
    a.directory.mkdir(parents=True, exist_ok=True)
    fs = FileSet()
    fs.ID = "AKULA_SMOKE"
    for i in range(3):
        fs.add(sample(i))
    fs.write(str(a.directory))
    idx = a.directory / "DICOMDIR"
    assert idx.is_file()
    restored = FileSet(str(idx))
    assert len(restored) == 3
    assert all(Path(i.path).suffix == "" for i in restored)
    print("SYNTHETIC_FILESET_OK images=3, extensionless=3, root=", a.directory)
    generate_medical_contrasts(a.directory)
    # FileSet ZIP is opened directly; nothing is extracted into the vault.
    archive = a.directory / 'study.zip'
    with zipfile.ZipFile(archive, 'w', compression=zipfile.ZIP_DEFLATED) as z:
        for p in sorted(a.directory.rglob('*')):
            if p.is_file() and p != archive and p.name not in {'MONO1.dcm', 'MONO2.dcm', 'SIGNED16.dcm'}:
                z.write(p, arcname=p.relative_to(a.directory).as_posix())
    loose = a.directory / 'loose_folder'
    loose.mkdir(exist_ok=True)
    with zipfile.ZipFile(archive) as z:
        names = sorted(n for n in z.namelist() if n.rsplit('/', 1)[-1].startswith('IM'))
        assert len(names) == 3
        for i, n in enumerate(names):
            destination = f'FRAME{i:04d}' if i == 0 else f'{i}.dcm'
            (loose / destination).write_bytes(z.read(n))
    with zipfile.ZipFile(a.directory / 'unsafe.zip', 'w') as z:
        z.writestr('../escape.dcm', b'REJECT')
    print('SYNTHETIC_INPLACE_OK zip=1 loose_folder=1 traversal_zip=1')
if __name__ == "__main__":
    main()
