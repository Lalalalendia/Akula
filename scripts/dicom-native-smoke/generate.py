#!/usr/bin/env python3
"""Generate a synthetic, no-patient-data DICOM FileSet with extensionless images."""
from pathlib import Path
import argparse
import numpy as np
import pydicom
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
if __name__ == "__main__":
    main()
